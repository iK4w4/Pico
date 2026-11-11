//! Download manager core (0.32): the pure engine behind
//! Pico's own download manager — segment planning with
//! work-stealing, filename / cookie safety, the resume
//! decision, persisted task state, and the speed / ETA
//! helpers the panel renders with. No network, no window:
//! the WinHTTP transport (`src/winhttp.rs`) and the manager
//! (`src/downloads.rs`) wire this to the OS; every decision
//! that can be made without either lives here and is pinned
//! by `tests/download_tests.rs`.
//!
//! SEGMENTS. A task's byte range is cut into [`Segment`]s
//! (end EXCLUSIVE). A segment's `done` counts bytes actually
//! written at `start + done` — progress only ever advances
//! after bytes land (see the brief's pause/persist law), so a
//! persisted segment map can under-claim but never splice
//! bytes that are not on disk.
//!
//! UNKNOWN TOTAL. A task whose size is unknown carries
//! `total: None` and exactly one segment `{start: 0, end: 0,
//! done: N}` — `end == start` is the open-ended marker. Such
//! a task is single-stream by construction (completion is
//! the stream's EOF, not the planner's `all_finished`) and
//! it CANNOT resume: with no total there is nothing to
//! validate a partial file against, so
//! [`resume_decision`] restarts it.
//!
//! CONNECTION COUNT (the implemented semantics, verbatim
//! from the brief): a task's connection count may be changed
//! at any time. For a queued task it is the plan used at
//! start. For a running or paused task it re-splits only the
//! not-yet-started byte ranges ([`Planner::replan_pending`]);
//! in-flight segments are never resized except by stealing.
//! If every remaining byte is already in flight, the new
//! count takes effect at the next steal / resume.
//!
//! WORK STEALING. When no fresh (unassigned, unfinished)
//! segment remains, [`Planner::claim`] splits the assigned
//! segment with the largest remaining range — but only when
//! that remainder exceeds [`STEAL_MIN_REMAINING`], so a
//! nearly-done segment is never churned for a sliver.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Fewest connections a task may use.
pub const MIN_CONNECTIONS: u32 = 1;
/// Most connections a task may use.
pub const MAX_CONNECTIONS: u32 = 32;
/// The connection count a fresh task (and a corrupt `0` in a
/// persisted task) falls back to.
pub const DEFAULT_CONNECTIONS: u32 = 8;
/// At most this many tasks download at once (manager law).
pub const MAX_ACTIVE_TASKS: usize = 3;
/// A segment is stolen from only when its remaining range
/// exceeds this (256 KiB) — below it, splitting costs more
/// (a new connection, a new range request) than it saves.
pub const STEAL_MIN_REMAINING: u64 = 256 * 1024;
/// On-disk version of a persisted task file.
pub const TASK_FILE_VERSION: u32 = 1;
/// Most automatic retries a failed task gets (0.33).
pub const AUTO_RETRY_MAX: u32 = 3;
/// Backoff before automatic retry #1/#2/#3, in ms (0.33).
pub const AUTO_RETRY_BACKOFF_MS: [u64; 3] = [5_000, 15_000, 45_000];

/// Largest task file `load_tasks` will even read (1 MiB) —
/// the real thing is a few hundred bytes of JSON.
const MAX_TASK_FILE_BYTES: u64 = 1024 * 1024;
/// A persisted task with more segments than this is corrupt,
/// not ambitious (32 connections, split by stealing, never
/// honestly reach it).
const MAX_SEGMENTS: usize = 4096;
/// Persisted cookies per task (a page's jar slice, host
/// scoped — dozens is generous).
const MAX_COOKIES: usize = 64;
/// Byte cap on one persisted cookie name / value.
const MAX_COOKIE_PART_BYTES: usize = 8192;
/// Character cap on a persisted error string (it renders in
/// one panel line; the log keeps the same discipline).
const MAX_ERROR_CHARS: usize = 300;
/// Character cap on a sanitized file name.
const MAX_FILE_NAME_CHARS: usize = 200;
/// Trailing window the speed meter rates over.
const SPEED_WINDOW_MS: u64 = 5_000;
/// Intervals shorter than this say nothing about a rate.
const SPEED_MIN_DT_MS: u64 = 250;

// ---------------------------------------------------------------------------
// Task model
// ---------------------------------------------------------------------------

/// Where a task is in its life. Serialized snake_case; an
/// unknown string in a task file fails the parse, and
/// [`load_tasks`] skips that file (see the sanitizer rules
/// in the brief).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Downloading,
    Paused,
    Complete,
    Failed,
}

impl TaskStatus {
    /// The short word the panel shows ("done", not
    /// "complete" — the label vocabulary is the brief's).
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Queued => "queued",
            TaskStatus::Downloading => "downloading",
            TaskStatus::Paused => "paused",
            TaskStatus::Complete => "done",
            TaskStatus::Failed => "failed",
        }
    }
}

/// One byte range of a task: `[start, end)` with `done`
/// bytes of it already on disk. See the module header for
/// the open-ended marker (`end == start`, unknown total).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub done: u64,
}

impl Segment {
    /// Length of the range (`end` is exclusive).
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Bytes of this segment not yet on disk.
    pub fn remaining(&self) -> u64 {
        self.len().saturating_sub(self.done)
    }

    /// Absolute file offset the next byte belongs at.
    pub fn position(&self) -> u64 {
        self.start.saturating_add(self.done)
    }

    /// True when every byte of the range is on disk. For the
    /// open-ended marker this follows `remaining()` (the
    /// marker's length is 0); single-stream completion is
    /// EOF-driven and never consults this — see the module
    /// header.
    pub fn is_finished(&self) -> bool {
        self.remaining() == 0
    }

    /// True for an empty range: the open-ended marker (or a
    /// zero-byte task's only segment).
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// One cookie captured from a tab's WebView2 cookie manager,
/// already normalized by the capture side: `domain` is
/// lowercase with no leading dot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookiePair {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub secure: bool,
    pub path: String,
    /// Absolute expiry (unix ms) when the cookie has one;
    /// `None` for session cookies. Serde-defaulted so task
    /// files written before 0.32.1 (which have no such field)
    /// still load. AUDIT 0.32.1 (P2-4): through 0.32 the
    /// capture never read the expiry, so a cookie the browser
    /// later expired stayed valid in the task file forever.
    #[serde(default)]
    pub expires_unix_ms: Option<u64>,
}

/// Wall-clock unix milliseconds (the core's only clock use:
/// cookie expiry judgments in [`sanitize_cookie`] and
/// [`cookie_header`]).
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Convert an `ICoreWebView2Cookie::Expires` value (an OLE
/// Automation DATE: days since 1899-12-30, fractional) to
/// unix milliseconds. Session cookies read as ≤ 0 (and a NaN
/// is meaningless): both become `None` — a session cookie —
/// rather than a 19th-century expiry. AUDIT 0.32.1 (P2-4).
pub fn cookie_expiry_unix_ms(webview_date: f64) -> Option<u64> {
    if !webview_date.is_finite() || webview_date <= 0.0 {
        return None;
    }
    // Days between 1899-12-30 and 1970-01-01.
    const EPOCH_OFFSET_DAYS: f64 = 25_569.0;
    let ms = (webview_date - EPOCH_OFFSET_DAYS) * 86_400_000.0;
    if ms <= 0.0 {
        None
    } else {
        Some(ms as u64)
    }
}

/// A download task as persisted (one `<id>.json` file per
/// task) and as the manager keeps it in memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskState {
    pub version: u32,
    pub id: u64,
    pub url: String,
    pub file_name: String,
    pub total: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub connections: u32,
    pub status: TaskStatus,
    pub added_ms: u64,
    pub segments: Vec<Segment>,
    pub cookies: Vec<CookiePair>,
    pub error: Option<String>,
    /// The page that referred this download (0.33), sent as
    /// the transport's Referer on probe/fetch. Serde-defaulted
    /// so task files written before 0.33 still load.
    #[serde(default)]
    pub referer: Option<String>,
    /// Expected SHA-256 of the finished file (0.33), 64
    /// lowercase hex chars when present — the manager
    /// verifies the completed file against it.
    #[serde(default)]
    pub sha256: Option<String>,
    /// How many automatic retries this task has already
    /// spent (0.33); clamped to [`AUTO_RETRY_MAX`].
    #[serde(default)]
    pub auto_retries: u32,
    /// The plain-language hint for the task's failure
    /// (0.33), from [`failure_hint`]; rendered under the
    /// error line on the downloads page.
    #[serde(default)]
    pub error_hint: Option<String>,
}

impl TaskState {
    /// Bytes on disk across all segments.
    pub fn downloaded(&self) -> u64 {
        self.segments
            .iter()
            .fold(0u64, |acc, s| acc.saturating_add(s.done))
    }

    /// Sanitize a task that came from disk (or anywhere
    /// untrusted). `None` is unrecoverable — the URL is not
    /// downloadable at all, so there is no task to salvage.
    ///
    /// The segment map gets the honest treatment: it must be
    /// sorted by start, gapless, non-overlapping, `done <=
    /// len` per segment and within the total. ANY violation
    /// rebuilds a fresh zero-progress plan from the total —
    /// a corrupt progress map must never splice wrong bytes,
    /// and re-downloading is always cheaper than a silently
    /// corrupt file. (For an unknown total the only honest
    /// map is the single open-ended segment; anything else
    /// rebuilds to it, zeroed.)
    pub fn sanitized(mut self) -> Option<TaskState> {
        if !is_downloadable_url(&self.url) {
            return None;
        }
        // AUDIT 0.32.1 (P2-4): embedded credentials never
        // persist — the transport strips userinfo for the
        // connect anyway, so the stored URL drops it too.
        self.url = strip_url_userinfo(&self.url);
        // AUDIT 0.32.1 (P1-5): an empty validator is NO
        // validator. Left as Some(""), it could never equal a
        // real probe value, yet vs a probe None it silently
        // fell through to total-only resume.
        self.etag = self.etag.filter(|s| !s.trim().is_empty());
        self.last_modified = self.last_modified.filter(|s| !s.trim().is_empty());
        self.version = TASK_FILE_VERSION;
        self.connections = if self.connections == 0 {
            DEFAULT_CONNECTIONS
        } else {
            self.connections.clamp(MIN_CONNECTIONS, MAX_CONNECTIONS)
        };
        self.file_name = sanitize_file_name(&self.file_name);
        match self.total {
            Some(total) => {
                if !segment_map_is_honest(&self.segments, total) {
                    self.segments = plan_segments(total, self.connections);
                }
            }
            None => {
                let honest = self.segments.len() == 1
                    && self
                        .segments
                        .first()
                        .is_some_and(|s| s.start == 0 && s.end == 0);
                if !honest {
                    self.segments = vec![Segment {
                        start: 0,
                        end: 0,
                        done: 0,
                    }];
                }
            }
        }
        // AUDIT 0.34.2 (G3): a Complete record's total is
        // the finished file's size — live completion forces
        // it (`finalize_complete` in downloads.rs), but a
        // legacy record can carry `total: null`, which the
        // renderers draw as an unfinished bar on a done row.
        // Repair it from the segment map's progress sum
        // (the map was made honest just above).
        if matches!(self.status, TaskStatus::Complete) && self.total.is_none() {
            self.total = Some(self.downloaded());
        }
        // AUDIT 0.34.2 (G5): `added_ms` is a wall-clock unix
        // stamp; a crafted record can carry a value outside
        // the JS Date range, which the downloads page renders
        // as the literal text "Invalid Date". Anything past
        // the year-9999 stamp is not a date — zero it
        // (unknown; the renderers skip a zero stamp).
        if self.added_ms > 253_402_300_799_999 {
            self.added_ms = 0;
        }
        self.cookies = sanitize_cookies(self.cookies);
        if let Some(error) = &self.error {
            self.error = Some(error.chars().take(MAX_ERROR_CHARS).collect());
        }
        // AUDIT 0.33: the 0.33 fields get the same untrusted-
        // input treatment. A referer is a URL the transport
        // will send: userinfo never persists (same rule as
        // the task URL), http/https only, capped at 2048
        // chars — anything else is no referer at all.
        self.referer = self.referer.take().and_then(|r| {
            let stripped = strip_url_userinfo(&r);
            if stripped.chars().count() <= 2048 && is_downloadable_url(&stripped) {
                Some(stripped)
            } else {
                None
            }
        });
        // A checksum is kept only in its exact shape: 64
        // hex chars, stored lowercased. Anything else was
        // never a checksum.
        self.sha256 = self.sha256.take().and_then(|s| {
            if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
                Some(s.to_ascii_lowercase())
            } else {
                None
            }
        });
        self.auto_retries = self.auto_retries.min(AUTO_RETRY_MAX);
        if let Some(hint) = &self.error_hint {
            self.error_hint = Some(hint.chars().take(200).collect());
        }
        Some(self)
    }
}

/// The segment-map honesty check for a known total: sorted,
/// gapless from 0 to `total`, every `done` within its
/// segment, and no more than [`MAX_SEGMENTS`] entries.
fn segment_map_is_honest(segments: &[Segment], total: u64) -> bool {
    if segments.is_empty() || segments.len() > MAX_SEGMENTS {
        return false;
    }
    let mut cursor = 0u64;
    for s in segments {
        if s.start != cursor || s.end < s.start || s.done > s.len() {
            return false;
        }
        cursor = s.end;
    }
    cursor == total
}

/// One persisted cookie, cleaned: control characters are
/// stripped from name / value / domain, the domain is
/// normalized (lowercase, no leading dot), name and value
/// are capped at [`MAX_COOKIE_PART_BYTES`]. A cookie whose
/// name is empty after stripping carried nothing — dropped.
/// AUDIT 0.32.1 (P1-19): a `;` in the value (or a `;` / `=`
/// in the name) is ALSO dropped — `cookie_header` joins
/// pairs as `name=value` with `"; "`, so those characters
/// would mint extra, attacker-shaped pairs inside a header
/// the cookie legitimately belongs to. And (P2-4) a cookie
/// already past its expiry is dropped, not stored.
fn sanitize_cookie(mut c: CookiePair) -> Option<CookiePair> {
    c.name = strip_controls(&c.name);
    c.value = strip_controls(&c.value);
    c.domain = strip_controls(&c.domain).to_lowercase();
    if let Some(stripped) = c.domain.strip_prefix('.') {
        c.domain = stripped.to_string();
    }
    if c.name.is_empty() || c.name.contains([';', '=']) || c.value.contains(';') {
        return None;
    }
    if let Some(exp) = c.expires_unix_ms {
        if exp <= unix_now_ms() {
            return None;
        }
    }
    c.name = truncate_bytes(&c.name, MAX_COOKIE_PART_BYTES);
    c.value = truncate_bytes(&c.value, MAX_COOKIE_PART_BYTES);
    Some(c)
}

/// Clean a whole captured cookie list (the sanitizer above,
/// plus the per-task count cap). The manager runs every
/// capture through this at add time (AUDIT 0.32.1, P1-19:
/// through 0.32 only the disk-load path sanitized, so the
/// live path stored cookies verbatim).
pub fn sanitize_cookies(cookies: Vec<CookiePair>) -> Vec<CookiePair> {
    cookies
        .into_iter()
        .filter_map(sanitize_cookie)
        .take(MAX_COOKIES)
        .collect()
}

fn strip_controls(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Truncate to at most `max` BYTES on a char boundary.
fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = 0usize;
    for (i, c) in s.char_indices() {
        let next = i + c.len_utf8();
        if next > max {
            break;
        }
        end = next;
    }
    s[..end].to_string()
}

// ---------------------------------------------------------------------------
// Segment planning
// ---------------------------------------------------------------------------

/// Cut `total` bytes into a fresh segment plan: connections
/// clamped to `1..=32`; `total == 0` is a single empty
/// segment; otherwise `n = min(connections, total)` near-even
/// segments (the first `total % n` are one byte longer). No
/// empty segments, no gaps.
pub fn plan_segments(total: u64, connections: u32) -> Vec<Segment> {
    let connections = connections.clamp(MIN_CONNECTIONS, MAX_CONNECTIONS);
    if total == 0 {
        return vec![Segment {
            start: 0,
            end: 0,
            done: 0,
        }];
    }
    let n = u64::from(connections).min(total);
    let base = total / n;
    let extra = total % n;
    let mut out = Vec::with_capacity(n as usize);
    let mut cursor = 0u64;
    for i in 0..n {
        let size = base + u64::from(i < extra);
        out.push(Segment {
            start: cursor,
            end: cursor + size,
            done: 0,
        });
        cursor += size;
    }
    out
}

/// What [`Planner::claim`] hands a worker. Segments are
/// addressed by their START offset everywhere in the planner
/// (AUDIT 0.32.1, P1-10/P4-A8): a start can never be moved by
/// a split, a steal or a replan, while a vector index could
/// silently come to name a different byte range when the
/// list was rebuilt mid-flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// An existing segment (identified by its start) that
    /// nobody was working on.
    Fresh(u64),
    /// A new segment split off the segment starting at
    /// `from`, already assigned to the claimant; the new
    /// segment starts at `start`.
    Steal { from: u64, start: u64 },
}

/// The segment list plus who is working on what. The planner
/// is the single writer of a running task's progress: workers
/// report through it, and [`Planner::sync_into`] is how a
/// task's persisted map is refreshed.
///
/// INVARIANT: `segments` is always sorted by `start` (the
/// persisted map's honesty check demands exactly that, and
/// AUDIT 0.32.1 P4-A2 found steal/replan breaking it, which
/// made the loader discard ALL progress on the next launch).
/// Every mutation below — steal insertion, replan merge —
/// preserves the sort.
#[derive(Debug, Clone)]
pub struct Planner {
    segments: Vec<Segment>,
    assigned: Vec<bool>,
    /// Unknown-total mode: the single segment is the
    /// open-ended marker, claimable exactly once (its
    /// `is_finished` is vacuous — length 0 — so without this
    /// flag no worker would ever fetch the stream; AUDIT
    /// 0.32.1, P4-A1).
    open_ended: bool,
}

impl Planner {
    pub fn new(segments: Vec<Segment>) -> Planner {
        let assigned = vec![false; segments.len()];
        Planner {
            segments,
            assigned,
            open_ended: false,
        }
    }

    /// A planner for an unknown-total (single-stream) task:
    /// its open-ended marker segment is claimable.
    pub fn new_open_ended(segments: Vec<Segment>) -> Planner {
        let assigned = vec![false; segments.len()];
        Planner {
            segments,
            assigned,
            open_ended: true,
        }
    }

    pub fn from_task(t: &TaskState) -> Planner {
        Planner::new(t.segments.clone())
    }

    fn index_of(&self, start: u64) -> Option<usize> {
        // The list is short (≤ MAX_SEGMENTS, in practice ≤ a
        // few hundred) and lookups happen per chunk, not per
        // byte: a linear scan is the honest cost.
        self.segments.iter().position(|s| s.start == start)
    }

    /// Claim the next piece of work:
    ///
    /// 1. The lowest unassigned unfinished segment →
    ///    [`Claim::Fresh`]. (In an open-ended planner the
    ///    marker counts: it is claimable once, and the
    ///    stream's EOF — not the planner — ends it.)
    /// 2. Else, the assigned unfinished segment with the
    ///    largest remaining range, when that remainder is
    ///    `> STEAL_MIN_REMAINING`: split it at
    ///    `position + remaining / 2` — the original keeps
    ///    `[start, split)` with its progress, the new segment
    ///    `[split, old_end)` is inserted in sorted position,
    ///    created already assigned, and returned as
    ///    [`Claim::Steal`].
    /// 3. Else `None`. Finished segments are never
    ///    candidates for either step.
    pub fn claim(&mut self) -> Option<Claim> {
        for (i, s) in self.segments.iter().enumerate() {
            let claimable = !s.is_finished() || (self.open_ended && s.is_empty());
            if !self.assigned.get(i).copied().unwrap_or(false) && claimable {
                if let Some(slot) = self.assigned.get_mut(i) {
                    *slot = true;
                }
                return Some(Claim::Fresh(s.start));
            }
        }
        let mut best: Option<usize> = None;
        let mut best_remaining = 0u64;
        for (i, s) in self.segments.iter().enumerate() {
            if self.assigned.get(i).copied().unwrap_or(false) && !s.is_finished() {
                let remaining = s.remaining();
                if remaining > best_remaining {
                    best = Some(i);
                    best_remaining = remaining;
                }
            }
        }
        let from = best?;
        if best_remaining <= STEAL_MIN_REMAINING {
            return None;
        }
        let original = self.segments.get(from).copied()?;
        let split = original.position().saturating_add(original.remaining() / 2);
        if let Some(slot) = self.segments.get_mut(from) {
            slot.end = split;
        }
        // Sorted insertion: split lies strictly inside the
        // original's range, so the new segment belongs
        // immediately after it (the next segment, if any,
        // starts at the original's old end).
        self.segments.insert(
            from + 1,
            Segment {
                start: split,
                end: original.end,
                done: 0,
            },
        );
        self.assigned.insert(from + 1, true);
        Some(Claim::Steal {
            from: original.start,
            start: split,
        })
    }

    /// Give a segment back (its `done` is kept — the bytes
    /// are on disk whoever picks it up next).
    pub fn release(&mut self, start: u64) {
        if let Some(i) = self.index_of(start) {
            if let Some(slot) = self.assigned.get_mut(i) {
                *slot = false;
            }
        }
    }

    /// Record a segment's progress: monotonic (a lower value
    /// is ignored) and clamped to the segment's length. The
    /// open-ended marker has no length to clamp to, so its
    /// progress is taken as reported (still monotonic).
    pub fn progress(&mut self, start: u64, done: u64) {
        let Some(i) = self.index_of(start) else {
            return;
        };
        if let Some(s) = self.segments.get_mut(i) {
            let cap = if s.end > s.start { s.len() } else { u64::MAX };
            let clamped = done.min(cap);
            if clamped > s.done {
                s.done = clamped;
            }
        }
    }

    pub fn segment(&self, start: u64) -> Option<Segment> {
        self.index_of(start)
            .and_then(|i| self.segments.get(i).copied())
    }

    pub fn downloaded(&self) -> u64 {
        self.segments
            .iter()
            .fold(0u64, |acc, s| acc.saturating_add(s.done))
    }

    pub fn all_finished(&self) -> bool {
        self.segments.iter().all(|s| s.is_finished())
    }

    /// Re-split only the not-yet-started ranges for a new
    /// connection count (the brief's connection rule).
    /// Segments with `done == 0` that are unassigned and
    /// unfinished are removed; their ranges are merged and
    /// re-split into `k` fresh segments, where
    /// `k = connections - started` and `started` counts only
    /// the kept segments that still NEED a connection
    /// (in-flight or partially downloaded — a finished
    /// segment is inert and must not eat the budget; AUDIT
    /// 0.32.1, P4-A7), at least 1 when merged bytes exist and
    /// at most the merged byte count. In-flight, finished and
    /// partial segments are never touched, and the rebuilt
    /// list stays sorted by start (P4-A2). Open-ended
    /// (unknown-total) planners: no-op — there is nothing to
    /// re-split.
    pub fn replan_pending(&mut self, connections: u32) {
        if self.segments.iter().any(|s| s.is_empty()) {
            return;
        }
        let connections = connections.clamp(MIN_CONNECTIONS, MAX_CONNECTIONS);
        let mut kept_segments: Vec<Segment> = Vec::new();
        let mut kept_assigned: Vec<bool> = Vec::new();
        let mut removed: Vec<(u64, u64)> = Vec::new();
        for (i, s) in self.segments.iter().enumerate() {
            let is_assigned = self.assigned.get(i).copied().unwrap_or(false);
            if s.done == 0 && !is_assigned && !s.is_finished() {
                removed.push((s.start, s.end));
            } else {
                kept_segments.push(*s);
                kept_assigned.push(is_assigned);
            }
        }
        if removed.is_empty() {
            return;
        }
        // Merge the removed ranges (sorted; adjacent or
        // overlapping ranges coalesce).
        removed.sort_unstable();
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for (start, end) in removed {
            match ranges.last_mut() {
                Some((_, last_end)) if start <= *last_end => {
                    *last_end = (*last_end).max(end);
                }
                _ => ranges.push((start, end)),
            }
        }
        let started = kept_segments.iter().filter(|s| !s.is_finished()).count() as u32;
        let mut k = connections.saturating_sub(started);
        if k == 0 {
            k = 1;
        }
        // Start from one fresh segment per merged range, then
        // split the largest piece until the target count is
        // reached (splitting stops on its own at one byte per
        // segment, which is the merged-byte-count cap).
        let mut fresh: Vec<Segment> = ranges
            .iter()
            .map(|(start, end)| Segment {
                start: *start,
                end: *end,
                done: 0,
            })
            .collect();
        while (fresh.len() as u32) < k {
            let mut widest: Option<usize> = None;
            let mut widest_len = 0u64;
            for (i, s) in fresh.iter().enumerate() {
                if s.len() > widest_len {
                    widest = Some(i);
                    widest_len = s.len();
                }
            }
            let Some(i) = widest else { break };
            if widest_len < 2 {
                break;
            }
            let s = fresh[i];
            let mid = s.start.saturating_add(s.len() / 2);
            if let Some(slot) = fresh.get_mut(i) {
                slot.end = mid;
            }
            fresh.insert(
                i + 1,
                Segment {
                    start: mid,
                    end: s.end,
                    done: 0,
                },
            );
        }
        // Merge kept + fresh back into ONE sorted list: a
        // kept partial segment can sit between two removed
        // ranges, so appending the fresh pieces after the
        // kept ones would leave the map unsorted (P4-A2).
        let mut all: Vec<(Segment, bool)> = kept_segments
            .into_iter()
            .zip(kept_assigned)
            .chain(fresh.into_iter().map(|s| (s, false)))
            .collect();
        all.sort_by_key(|(s, _)| s.start);
        self.segments = all.iter().map(|(s, _)| *s).collect();
        self.assigned = all.iter().map(|(_, a)| *a).collect();
    }

    /// Write the planner's segment map back into a task (the
    /// persist point for pause / progress saves).
    pub fn sync_into(&self, t: &mut TaskState) {
        t.segments = self.segments.clone();
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
}

// ---------------------------------------------------------------------------
// File names
// ---------------------------------------------------------------------------

/// The extension split shared by the sanitizer and the
/// deduper: the last `.` suffix, when it is at most 10 chars
/// and not the name's first char. Returns `(stem, Some(ext))`
/// or `(name, None)`.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => {
            let ext = &name[dot + 1..];
            if !ext.is_empty() && ext.chars().count() <= 10 {
                return (&name[..dot], Some(ext));
            }
            (name, None)
        }
        _ => (name, None),
    }
}

const RESERVED_STEMS: [&str; 24] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

/// Truncate to at most `max_chars` characters AND at most
/// `max_units` UTF-16 code units, on a char boundary.
/// AUDIT 0.32.1 (P1-17): the name cap used to count chars
/// only — 200 astral chars are 400 UTF-16 units, over
/// NTFS's 255-unit component limit.
fn truncate_to(s: &str, max_chars: usize, max_units: usize) -> String {
    let mut out = String::new();
    let mut units = 0usize;
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars || units + c.len_utf16() > max_units {
            break;
        }
        units += c.len_utf16();
        out.push(c);
    }
    out
}

/// True when `s` fits the file-name cap on both measures.
fn within_name_cap(s: &str) -> bool {
    s.chars().count() <= MAX_FILE_NAME_CHARS && s.encode_utf16().count() <= MAX_FILE_NAME_CHARS
}

/// Make an untrusted name safe to create on disk (and on
/// Windows in particular): keep only the last path component
/// (split on `/` and `\`), remove control characters and
/// `<>:"|?*`, trim trailing dots and spaces and leading
/// spaces, prefix Windows reserved device names with `_`,
/// and cap the result at 200 chars preserving the extension.
/// An empty result becomes `"download"`.
/// Unicode format characters (category Cf) stripped from
/// file names (AUDIT 0.40.1): the bidi controls can render a
/// name's extension reversed in Explorer and other UIs
/// (`invoice\u{202e}fdp.exe` is the classic spoof), and the
/// zero-width marks / BOM / soft hyphen serve no legitimate
/// purpose in a downloaded file's name. `char::is_control`
/// covers only category Cc, so these are named explicitly.
fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{00ad}'
        | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206f}'
        | '\u{feff}')
}

pub fn sanitize_file_name(raw: &str) -> String {
    let last = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| {
            !c.is_control()
                && !is_format_char(*c)
                && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        .collect::<String>();
    let trimmed = last
        .trim_start_matches(' ')
        .trim_end_matches(['.', ' '])
        .to_string();
    if trimmed.is_empty() {
        return "download".to_string();
    }
    // Reserved device names: the stem before the FIRST dot,
    // case-insensitive, with or without an extension.
    // AUDIT 0.32.1 (P1-17): Win32 trims trailing spaces and
    // dots from a component before resolving it, so
    // "CON .txt" IS the CON device — the comparison stem
    // gets the same trim, or the check is bypassed by one
    // space.
    let stem = match trimmed.find('.') {
        Some(dot) => &trimmed[..dot],
        None => &trimmed[..],
    };
    let stem_key = stem.trim_end_matches([' ', '.']).to_uppercase();
    let prefixed = if RESERVED_STEMS.contains(&stem_key.as_str()) {
        format!("_{trimmed}")
    } else {
        trimmed
    };
    if within_name_cap(&prefixed) {
        return prefixed;
    }
    match split_extension(&prefixed) {
        (stem, Some(ext)) => {
            let keep_chars = MAX_FILE_NAME_CHARS
                .saturating_sub(1)
                .saturating_sub(ext.chars().count());
            let keep_units = MAX_FILE_NAME_CHARS
                .saturating_sub(1)
                .saturating_sub(ext.encode_utf16().count());
            let stem_part = truncate_to(stem, keep_chars, keep_units);
            format!("{stem_part}.{ext}")
        }
        _ => truncate_to(&prefixed, MAX_FILE_NAME_CHARS, MAX_FILE_NAME_CHARS),
    }
}

/// Percent-decode exactly once: valid `%XX` escapes become
/// their byte, invalid escapes stay literal. The byte stream
/// is then read as UTF-8 (lossy — a hostile name degrades to
/// replacement chars, never to a panic).
fn percent_decode_once(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// The file name a URL suggests: its last non-empty path
/// segment, percent-decoded exactly once, sanitized. Query
/// and fragment never contribute; no usable segment →
/// `"download"`.
pub fn file_name_from_url(url: &str) -> String {
    let after_scheme = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let path = match after_scheme.find('/') {
        Some(i) => {
            let p = &after_scheme[i..];
            let end = p.find(['?', '#']).unwrap_or(p.len());
            &p[..end]
        }
        None => "",
    };
    let segment = path.split('/').rfind(|s| !s.is_empty());
    match segment {
        Some(seg) => sanitize_file_name(&percent_decode_once(seg)),
        None => "download".to_string(),
    }
}

/// Split a header parameter list on `;` without splitting
/// inside a quoted string.
fn split_params(header: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in header.chars() {
        if escaped {
            current.push(c);
            escaped = false;
        } else if in_quotes && c == '\\' {
            current.push(c);
            escaped = true;
        } else if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
        } else if c == ';' && !in_quotes {
            out.push(current);
            current = String::new();
        } else {
            current.push(c);
        }
    }
    out.push(current);
    out
}

/// Unquote a `filename="..."` value (backslash escapes are
/// resolved); a bare token passes through trimmed.
fn unquote_value(value: &str) -> String {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('"') {
        let mut out = String::new();
        let mut escaped = false;
        for c in rest.chars() {
            if escaped {
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                break;
            } else {
                out.push(c);
            }
        }
        out
    } else {
        value.to_string()
    }
}

/// Extract the file name from a `Content-Disposition`
/// header: the RFC 5987 `filename*` form (percent-decoded)
/// is preferred, then a quoted or token `filename`. Returns
/// the RAW extracted name — sanitizing it is the caller's
/// step. `None` when absent or empty.
pub fn file_name_from_disposition(header: &str) -> Option<String> {
    let mut star: Option<String> = None;
    let mut plain: Option<String> = None;
    for (i, part) in split_params(header).into_iter().enumerate() {
        if i == 0 {
            continue; // the disposition type itself
        }
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();
        if key == "filename*" {
            // charset'lang'percent-encoded — the charset is
            // taken on trust (the brief's probes are UTF-8).
            let decoded = match (value.find('\''), value.rfind('\'')) {
                (Some(first), Some(last)) if last > first => {
                    percent_decode_once(&value[last + 1..])
                }
                _ => percent_decode_once(value),
            };
            if !decoded.trim().is_empty() {
                star = Some(decoded);
            }
        } else if key == "filename" {
            let name = unquote_value(value);
            if !name.trim().is_empty() {
                plain = Some(name);
            }
        }
    }
    star.or(plain)
}

/// If `exists(name)`, try `base (1).ext`, `base (2).ext`, …
/// returning the first free name. The counter runs to
/// 99_999 before giving up (and returns the last tried name
/// then): AUDIT 0.32.1 (P1-23) — the old 999 cap returned a
/// name that EXISTED in exactly the exhausted case, and the
/// caller's rename on Windows replaces, silently breaking
/// the never-overwrite promise. Candidates are also fitted
/// to the name cap (P1-17): the suffix never pushes a
/// capped name past it.
pub fn dedupe_file_name(name: &str, exists: impl Fn(&str) -> bool) -> String {
    if !exists(name) {
        return name.to_string();
    }
    let (stem, ext) = split_extension(name);
    let mut last_tried = name.to_string();
    for i in 1..=99_999u32 {
        let suffix = format!(" ({i})");
        let tail = match ext {
            Some(ext) => format!("{suffix}.{ext}"),
            None => suffix,
        };
        let stem_part = truncate_to(
            stem,
            MAX_FILE_NAME_CHARS.saturating_sub(tail.chars().count()),
            MAX_FILE_NAME_CHARS.saturating_sub(tail.encode_utf16().count()),
        );
        let candidate = format!("{stem_part}{tail}");
        if !exists(&candidate) {
            return candidate;
        }
        last_tried = candidate;
    }
    last_tried
}

// ---------------------------------------------------------------------------
// URLs and cookies
// ---------------------------------------------------------------------------

/// The host of an http(s)-style URL, lowercased: userinfo and
/// port stripped, a bracketed IPv6 literal unbracketed.
/// `None` when there is no authority to speak of.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

/// Downloads ride plain HTTP(S) only — no file:, no ftp:,
/// no script schemes, and a host must be present.
pub fn is_downloadable_url(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    url_host(url).is_some()
}

/// The verdict of [`triage_download_uri`]: accept the URL
/// for download, or reject it with a PLAIN reason that
/// renders verbatim in the panel's error line (AUDIT
/// 0.32.2, Fix 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadTriage {
    Accept,
    Reject(&'static str),
}

/// Triage one download URI the content view tried to hand
/// Pico (AUDIT 0.32.2, Fix 1): through 0.32.1 a refusal at
/// this boundary logged one line and showed the user
/// NOTHING — the click simply vanished. Acceptance is
/// exactly [`is_downloadable_url`]; every rejection carries
/// the reason, so the caller can record a Failed task the
/// user can see. The scheme (the text before the first
/// `:`) is extracted BEFORE the "has ://" question, so a
/// `blob:https://…` URI — which contains "://" inside
/// itself — lands on the scheme branch, never on the
/// no-scheme branch.
pub fn triage_download_uri(uri: &str) -> DownloadTriage {
    if uri.trim().is_empty() {
        return DownloadTriage::Reject("empty link");
    }
    if is_downloadable_url(uri) {
        return DownloadTriage::Accept;
    }
    // Not acceptable: name the reason. A scheme is the text
    // before the first `:` when that colon precedes any
    // path/query/fragment delimiter (`data:…`, `file:…`,
    // `blob:…`); a colon later in the string is path text,
    // not a scheme.
    let scheme = match uri.find(':') {
        Some(colon) if !uri[..colon].contains(['/', '?', '#']) => Some(&uri[..colon]),
        _ => None,
    };
    match scheme {
        // An http/https spelling that is not downloadable
        // failed the one remaining check: it has no host.
        Some(s) if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https") => {
            DownloadTriage::Reject("the link has no host")
        }
        Some(_) => DownloadTriage::Reject("Pico can only download http/https links"),
        None => DownloadTriage::Reject("not a web link"),
    }
}

/// How the tab came to exist (0.32.4): a tab the SITE
/// opened (new-window/popup adoption) vs one the USER
/// opened (new-tab gesture, startup, typed navigation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnwindOrigin {
    Adopted,
    User,
}

/// What the shell should do with a tab whose main-frame
/// navigation was just captured as a download (AUDIT
/// 0.32.3, Fix 1; extended AUDIT 0.32.4).
///
/// The 0.32.4 extension: a tab the site opened only to
/// carry this download (an adopted new-window/popup tab
/// with no page of its own) closes instead of converting
/// to the start page — it was never the user's tab to
/// keep. A user's own tab, and the window's only tab in
/// every case, never closes this way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnwindAction {
    /// Leave the tab alone: the capture was not this tab's
    /// in-flight main-frame navigation (a subframe or
    /// download-attribute capture, or the user has already
    /// moved the tab elsewhere), or a traversal landing is
    /// already in flight and will reconcile the tab itself.
    None,
    /// The captured URL itself is the tab's current history
    /// entry (it somehow committed): traverse back to the
    /// previous entry through the history machinery, the
    /// same landing a user's own Back produces.
    Back,
    /// Reload the tab's last committed page: the captured
    /// navigation never committed (the standard case), so
    /// the committed page is where the user still is.
    NavigateTo(String),
    /// The tab has no committed page to return to (a fresh
    /// tab or popup whose first navigation was the file):
    /// convert it to the start page rather than leaving it
    /// parked on the file URL.
    StartPage,
    /// The tab exists only to carry this download (site-
    /// opened, no page of its own, not the window's only
    /// tab): close it through the ordinary tab-close path.
    CloseTab,
}

/// Decide the unwind for one download capture (AUDIT
/// 0.32.3, Fix 1; extended AUDIT 0.32.4). The mechanism
/// this answers: a main-frame navigation to a file URL is
/// adopted by the tab's URL state at load start (the
/// address bar shows the file URL, the tab shows Loading),
/// then the download handler cancels WebView2's download
/// — and no load-finish event ever reconciles the tab, so
/// without an unwind it sits on the file URL forever. The
/// decision is pure over the tab state the shell already
/// tracks:
///
/// - `live_url` — the tab's live URL (the load-start
///   adoption included);
/// - `committed_url` — the history machine's current
///   entry: the last page that actually committed;
/// - `prev_committed_url` — the entry before the current
///   one, when there is one;
/// - `captured_url` — the URL the download was captured
///   from (the POST-redirect `DownloadOperation.Uri`);
/// - `chain` — every URL the captured navigation has
///   occupied: its initial target plus each redirect hop,
///   ending at `captured_url` (0.35, WS2). Through 0.34
///   the capture carried only `captured_url` and the
///   first gate compared it against `live_url` by exact
///   string: a navigation that redirected (the soft98
///   shape — the tab's live URL is the PRE-redirect hop
///   while the handler hands over the POST-redirect URI)
///   could never match, so its fate silently decided
///   None and the site-opened download tab never closed;
/// - `traversal_pending` — a back/forward intent is
///   already in flight for this tab.
/// - `origin` — how the tab came to exist (0.32.4): a
///   tab the site opened (`UnwindOrigin::Adopted`, a
///   new-window/popup adoption) vs one the user opened
///   (`UnwindOrigin::User`);
/// - `only_tab` — whether this is the window's only tab
///   (0.32.4): a window never ends with zero tabs, so an
///   only tab never closes.
///
/// Rules, in order: a capture whose tab's live URL is
/// not IN the chain is not this tab's navigation (None)
/// — the chain form of the old exact-match gate, and
/// the invariant that still refuses a genuine user
/// navigation, whose URL is by construction outside the
/// captured navigation's chain; a pending traversal
/// owns the tab's next landing (None); those two gates
/// stay first and dominate `origin`. Committed and
/// previous entries are then judged as before, except
/// that an entry which is itself a chain member is
/// dropped before judging: a redirect hop is not a page
/// the user read (the 0.33 `drop_top_entry` idea,
/// generalised from the single file URL to the whole
/// chain). So: if a chain member committed as the
/// current entry, go Back to the previous entry when a
/// real (non-chain) one exists — the tab had a life
/// before the file — unless there is none or it is also
/// a chain member, which would loop or land on a hop:
/// then CloseTab when the tab is adopted and not the
/// only tab, else StartPage. If a different, real page
/// is committed, reload it (NavigateTo) — unchanged and
/// origin-independent. If nothing (or only a chain
/// member) ever committed, the tab was opened for this
/// download: CloseTab when it is adopted and not the
/// only tab, else StartPage.
// The fact list is the decision's whole input, kept flat
// like the coordinator's `CaptureFacts` it mirrors (the
// downloads manager's own precedent for this allow is in
// src/downloads.rs).
#[allow(clippy::too_many_arguments)]
pub fn unwind_action(
    live_url: &str,
    committed_url: Option<&str>,
    prev_committed_url: Option<&str>,
    captured_url: &str,
    chain: &[String],
    traversal_pending: bool,
    origin: UnwindOrigin,
    only_tab: bool,
) -> UnwindAction {
    if captured_url.is_empty() {
        return UnwindAction::None;
    }
    // Chain membership. The chain includes `captured_url`
    // by construction; an empty chain (a caller that has
    // no navigation record) falls back to the pre-0.35
    // exact match against the captured URL alone, so the
    // gate never widens on missing data.
    let in_chain = |u: &str| -> bool {
        if chain.is_empty() {
            u == captured_url
        } else {
            chain.iter().any(|c| c == u)
        }
    };
    if !in_chain(live_url) {
        return UnwindAction::None;
    }
    if traversal_pending {
        return UnwindAction::None;
    }
    // A site-opened tab with no page of its own exists
    // only to carry this download: close it (0.32.4). A
    // user's own tab, and the window's only tab, fall
    // back to the start page instead.
    let close_or_start = if origin == UnwindOrigin::Adopted && !only_tab {
        UnwindAction::CloseTab
    } else {
        UnwindAction::StartPage
    };
    // A previous entry that is itself a chain member is
    // a redirect hop, not a page: dropped before judging.
    let real_prev = prev_committed_url.filter(|prev| !in_chain(prev));
    match committed_url {
        Some(committed) if in_chain(committed) => match real_prev {
            Some(_) => UnwindAction::Back,
            _ => close_or_start,
        },
        Some(committed) => UnwindAction::NavigateTo(committed.to_string()),
        None => close_or_start,
    }
}

/// A run-generation gate (AUDIT 0.32.2, Fix 2): the pure
/// decision behind instant pause/cancel. Every supervisor
/// run of a task carries a token minted by
/// [`GenerationGate::begin`]; a manager action that must
/// retire the run (pause, cancel, shutdown) calls
/// [`GenerationGate::invalidate`], after which the old
/// token is rejected everywhere it is checked — the
/// supervisor's state writes and the workers' sink — so a
/// stale run can never write over the manager's own
/// snapshot. Token 0 is never valid: a fresh gate accepts
/// nothing, and `begin` saturates upward from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationGate {
    current: u64,
}

impl GenerationGate {
    pub fn new() -> Self {
        GenerationGate { current: 0 }
    }

    /// Mint the token for a new run: advances the current
    /// generation (saturating) and returns it.
    pub fn begin(&mut self) -> u64 {
        self.current = self.current.saturating_add(1);
        self.current
    }

    /// Retire the live run without minting one: advances
    /// the current generation (saturating) and returns it,
    /// so every previously minted token is now rejected.
    pub fn invalidate(&mut self) -> u64 {
        self.current = self.current.saturating_add(1);
        self.current
    }

    /// True only for the live token: never 0, and only the
    /// current generation.
    pub fn accepts(&self, token: u64) -> bool {
        token != 0 && token == self.current
    }

    pub fn current(&self) -> u64 {
        self.current
    }
}

impl Default for GenerationGate {
    fn default() -> Self {
        Self::new()
    }
}

/// The request path of a URL (everything from the first
/// `/` after the authority, query/fragment excluded); `/`
/// when the URL has no path.
fn url_path(url: &str) -> &str {
    let rest = match url.split_once("://") {
        Some((_, r)) => r,
        None => url,
    };
    match rest.find('/') {
        Some(i) => {
            let p = &rest[i..];
            let end = p.find(['?', '#']).unwrap_or(p.len());
            &p[..end]
        }
        None => "/",
    }
}

/// RFC 6265 §5.1.4 path-match: the request path equals the
/// cookie path, or the cookie path is a prefix ending in
/// `/`, or it is a prefix and the next request-path char is
/// `/`. An empty or non-`/` cookie path means `/`.
fn cookie_path_matches(cookie_path: &str, request_path: &str) -> bool {
    let cp = if cookie_path.is_empty() || !cookie_path.starts_with('/') {
        "/"
    } else {
        cookie_path
    };
    if request_path == cp {
        return true;
    }
    request_path.starts_with(cp)
        && (cp.ends_with('/') || request_path.as_bytes().get(cp.len()) == Some(&b'/'))
}

/// The `Cookie` header for one request: which of the task's
/// captured cookies apply to `url`. A cookie applies when
/// `(host_only && host == domain) || (!host_only && (host ==
/// domain || host.ends_with("." + domain)))`; secure cookies
/// ride https only; the cookie's PATH scope is honored
/// (RFC 6265 §5.1.4) and an expired cookie never applies.
/// AUDIT 0.32.1 (P2-3/P2-4): through 0.32 the path was
/// ignored — harmless on the first request (WebView2 had
/// path-matched at capture) but a leak on redirect hops,
/// where this function is recomputed for a new path — and
/// expiry was never checked at all. Pairs join as
/// `name=value` with `"; "`; `None` when no cookie applies.
pub fn cookie_header(cookies: &[CookiePair], url: &str) -> Option<String> {
    let host = url_host(url)?;
    let is_https = url
        .split_once("://")
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("https"));
    let request_path = url_path(url);
    let now = unix_now_ms();
    let mut pairs: Vec<String> = Vec::new();
    for c in cookies {
        if c.secure && !is_https {
            continue;
        }
        if let Some(exp) = c.expires_unix_ms {
            if exp <= now {
                continue;
            }
        }
        if !cookie_path_matches(&c.path, request_path) {
            continue;
        }
        let domain = c.domain.to_lowercase();
        let domain = domain.strip_prefix('.').unwrap_or(&domain);
        if domain.is_empty() {
            continue;
        }
        let applies = if c.host_only {
            host == domain
        } else {
            host == domain || host.ends_with(&format!(".{domain}"))
        };
        if applies {
            pairs.push(format!("{}={}", c.name, c.value));
        }
    }
    if pairs.is_empty() {
        None
    } else {
        Some(pairs.join("; "))
    }
}

/// May the cookies of the page at `page_url` be captured for
/// a download of `download_url`? Only when the two hosts are
/// the same site, pragmatically: equal, or either is a
/// subdomain of the other (`example.com` ↔
/// `cdn.example.com`, `www.example.com` ↔ `example.com`).
/// AUDIT 0.32.1 (P2-1): capture used to be scoped only to
/// the DOWNLOAD's host, so any page could make Pico fire an
/// authenticated GET at an arbitrary host with that host's
/// profile cookies — a blind CSRF-by-download (the body
/// lands in a file, but side-effecting GETs still fire).
/// Cross-site downloads still work; they just carry no
/// cookies.
pub fn cookie_capture_allowed(page_url: &str, download_url: &str) -> bool {
    let (Some(page_host), Some(dl_host)) = (url_host(page_url), url_host(download_url)) else {
        return false;
    };
    page_host == dl_host
        || dl_host.ends_with(&format!(".{page_host}"))
        || page_host.ends_with(&format!(".{dl_host}"))
}

/// Remove any `user[:pass]@` userinfo from a URL's
/// authority. An `@` in the path/query is not userinfo and
/// is left alone. AUDIT 0.32.1 (P2-4).
pub fn strip_url_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let auth_end = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    match authority.rfind('@') {
        Some(at) => format!("{scheme}://{}{}", &authority[at + 1..], &rest[auth_end..]),
        None => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Probe / resume
// ---------------------------------------------------------------------------

/// What a probe (HEAD, or the ranged-GET fallback) learned
/// about the resource a task points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeFacts {
    pub status: u16,
    pub total: Option<u64>,
    pub accept_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// The supervisor's call at (re)start: download fresh,
/// resume the partial file, or throw it away and restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecision {
    Fresh,
    Resume,
    Restart,
}

/// Decide from the saved state and a fresh probe:
///
/// - Nothing downloaded yet → [`ResumeDecision::Fresh`].
/// - Else [`ResumeDecision::Restart`] when: the totals
///   differ (both known, unequal); the saved total is known
///   but the probe's is not; the saved task's total is
///   unknown (an open-ended partial can never resume);
///   etag / last-modified are present on both sides and
///   differ; or the probe lost range support.
/// - Else [`ResumeDecision::Resume`]. A validator absent on
///   one side only falls back to the remaining checks —
///   total equality is the floor.
pub fn resume_decision(saved: &TaskState, probe: &ProbeFacts) -> ResumeDecision {
    if saved.downloaded() == 0 {
        return ResumeDecision::Fresh;
    }
    if saved.total.is_none() || probe.total.is_none() {
        return ResumeDecision::Restart;
    }
    if saved.total != probe.total {
        return ResumeDecision::Restart;
    }
    if let (Some(a), Some(b)) = (&saved.etag, &probe.etag) {
        if a != b {
            return ResumeDecision::Restart;
        }
    }
    if let (Some(a), Some(b)) = (&saved.last_modified, &probe.last_modified) {
        if a != b {
            return ResumeDecision::Restart;
        }
    }
    if !probe.accept_ranges {
        return ResumeDecision::Restart;
    }
    ResumeDecision::Resume
}

// ---------------------------------------------------------------------------
// Failure taxonomy, retry policy and link refresh (0.33)
// ---------------------------------------------------------------------------

/// What KIND of failure stopped a task (0.33). The class —
/// not the raw message — drives the retry policy and the
/// hint the downloads page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Network,
    Server,
    RateLimited,
    NotFound,
    Auth,
    Disk,
    RangeLost,
    Changed,
    Other,
}

impl FailureClass {
    /// Stable log / wire spelling.
    pub fn label(self) -> &'static str {
        match self {
            FailureClass::Network => "network",
            FailureClass::Server => "server",
            FailureClass::RateLimited => "rate-limited",
            FailureClass::NotFound => "not-found",
            FailureClass::Auth => "auth",
            FailureClass::Disk => "disk",
            FailureClass::RangeLost => "range-lost",
            FailureClass::Changed => "changed",
            FailureClass::Other => "other",
        }
    }
}

/// Find the HTTP status a failure message carries: the
/// first `HTTP <3-digit>` token (the manager's strings are
/// "HTTP {status}", "unfollowable redirect (HTTP
/// {status})", "HTTP 416", "short response (HTTP 206)", …).
/// A digit run longer than three is not a status token.
pub fn http_status_of(message: &str) -> Option<u16> {
    let bytes = message.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = message[from..].find("HTTP ") {
        let start = from + rel + 5;
        let digits = &bytes[start.min(bytes.len())..];
        if digits.len() >= 3
            && digits[..3].iter().all(|b| b.is_ascii_digit())
            && digits.get(3).map(|b| !b.is_ascii_digit()).unwrap_or(true)
        {
            // Three ASCII digits always parse into a u16.
            let value = digits[..3]
                .iter()
                .fold(0u16, |acc, b| acc * 10 + u16::from(b - b'0'));
            return Some(value);
        }
        from = from + rel + 5;
    }
    None
}

/// Classify one of the manager's real failure strings
/// (0.33). A carried HTTP status decides first; without
/// one, the known message shapes decide, and anything
/// unrecognized is a network failure (the transport's
/// ordinary way to die).
pub fn classify_failure(message: &str) -> FailureClass {
    if let Some(status) = http_status_of(message) {
        return match status {
            404 => FailureClass::NotFound,
            401 | 403 => FailureClass::Auth,
            429 => FailureClass::RateLimited,
            416 => FailureClass::RangeLost,
            500..=599 => FailureClass::Server,
            _ => FailureClass::Other,
        };
    }
    if message.contains("not enough disk space")
        || message.contains("disk nearly full")
        || message.contains("cannot prepare file")
        || message.contains("cannot open file")
        || message.contains("cannot move finished file")
        || message.contains("write failed")
        || message.contains("seek failed")
    {
        return FailureClass::Disk;
    }
    if message.contains("server stopped honoring ranges") {
        return FailureClass::RangeLost;
    }
    if message.contains("size mismatch") {
        return FailureClass::Changed;
    }
    if message.contains("unsupported URL") || message.contains("transport unavailable") {
        return FailureClass::Other;
    }
    FailureClass::Network
}

/// What the manager should do with a failed task (0.33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// Retry in place after this many milliseconds.
    Retry { after_ms: u64 },
    /// Restart the task from zero, once.
    RestartOnce,
    /// Do not retry automatically.
    FailNow,
}

/// The retry policy for a failure class and the number of
/// automatic retries already spent (0.33): transient
/// classes retry on the [`AUTO_RETRY_BACKOFF_MS`] ladder
/// until [`AUTO_RETRY_MAX`]; a lost range or a changed
/// file restarts once from zero; everything else fails.
pub fn retry_policy(class: FailureClass, auto_retries: u32) -> RetryPolicy {
    match class {
        FailureClass::Network | FailureClass::Server | FailureClass::RateLimited => {
            if auto_retries < AUTO_RETRY_MAX {
                let idx = (auto_retries as usize).min(AUTO_RETRY_BACKOFF_MS.len() - 1);
                match AUTO_RETRY_BACKOFF_MS.get(idx) {
                    Some(ms) => RetryPolicy::Retry { after_ms: *ms },
                    None => RetryPolicy::FailNow,
                }
            } else {
                RetryPolicy::FailNow
            }
        }
        FailureClass::RangeLost | FailureClass::Changed => {
            if auto_retries == 0 {
                RetryPolicy::RestartOnce
            } else {
                RetryPolicy::FailNow
            }
        }
        FailureClass::NotFound | FailureClass::Auth | FailureClass::Disk | FailureClass::Other => {
            RetryPolicy::FailNow
        }
    }
}

/// The plain-language hint for a failure class (0.33) —
/// shown under the error on the downloads page. Only the
/// classes where a hint tells the user something the raw
/// message does not have one.
pub fn failure_hint(class: FailureClass) -> Option<&'static str> {
    match class {
        FailureClass::Auth => Some("the link may have expired — use Refresh link"),
        FailureClass::NotFound => Some("the file is gone from the server (404)"),
        FailureClass::Disk => {
            Some("a disk problem stopped this download — free space, then Resume")
        }
        FailureClass::RangeLost => Some("the server stopped honoring ranges"),
        FailureClass::Changed => Some("the file changed on the server"),
        _ => None,
    }
}

/// The link-refresh decision (0.33): after the user pastes
/// a fresh URL for a task, may the partial file resume
/// against it, or must the task restart?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDecision {
    RebindResume,
    RebindRestart,
}

/// Decide from the saved state and a probe OF THE NEW URL
/// (0.33). Resume only when every fact agrees: some bytes
/// are on disk, the probe answered in 200..400, both
/// totals are known and equal, neither validator is
/// contradicted (present on both sides and different),
/// and the new endpoint still honors ranges.
pub fn refresh_decision(saved: &TaskState, probe: &ProbeFacts) -> RefreshDecision {
    if saved.downloaded() == 0 {
        return RefreshDecision::RebindRestart;
    }
    if !(200..400).contains(&probe.status) {
        return RefreshDecision::RebindRestart;
    }
    match (saved.total, probe.total) {
        (Some(a), Some(b)) if a == b => {}
        _ => return RefreshDecision::RebindRestart,
    }
    if let (Some(a), Some(b)) = (&saved.etag, &probe.etag) {
        if a != b {
            return RefreshDecision::RebindRestart;
        }
    }
    if let (Some(a), Some(b)) = (&saved.last_modified, &probe.last_modified) {
        if a != b {
            return RefreshDecision::RebindRestart;
        }
    }
    if !probe.accept_ranges {
        return RefreshDecision::RebindRestart;
    }
    RefreshDecision::RebindResume
}

// ---------------------------------------------------------------------------
// Speed limiting (0.33)
// ---------------------------------------------------------------------------

/// A token bucket for the global speed limit (0.33):
/// capacity is ONE SECOND of rate (a one-second burst),
/// refilled continuously. Rate 0 is unlimited/off: every
/// take succeeds and no wait is ever owed. Tokens are
/// tracked in milli-tokens (1 token = 1000 milli) so many
/// small refills accumulate exactly — a refill of 1 ms at
/// 3 tokens/s adds 3 milli-tokens instead of rounding to
/// zero and drifting forever (the no-drift law).
#[derive(Debug, Clone)]
pub struct TokenBucket {
    rate_per_sec: u64,
    tokens_milli: u128,
    last_ms: u64,
}

impl TokenBucket {
    /// A bucket that starts FULL at `rate_per_sec`
    /// (tokens = rate; rate 0 = unlimited/off).
    pub fn new(rate_per_sec: u64, now_ms: u64) -> Self {
        TokenBucket {
            rate_per_sec,
            tokens_milli: u128::from(rate_per_sec) * 1000,
            last_ms: now_ms,
        }
    }

    pub fn rate(&self) -> u64 {
        self.rate_per_sec
    }

    fn capacity_milli(&self) -> u128 {
        u128::from(self.rate_per_sec) * 1000
    }

    fn refill(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.last_ms);
        self.last_ms = self.last_ms.max(now_ms);
        if elapsed == 0 || self.rate_per_sec == 0 {
            return;
        }
        // Milli-tokens accrue at `rate` per millisecond:
        // exact integer math, no fractional loss.
        let add = u128::from(elapsed) * u128::from(self.rate_per_sec);
        self.tokens_milli = (self.tokens_milli + add).min(self.capacity_milli());
    }

    /// Change the rate: refill at the OLD rate first, then
    /// clamp the balance to the NEW capacity (= new rate).
    pub fn set_rate(&mut self, rate_per_sec: u64, now_ms: u64) {
        self.refill(now_ms);
        self.rate_per_sec = rate_per_sec;
        let cap = self.capacity_milli();
        if self.tokens_milli > cap {
            self.tokens_milli = cap;
        }
        // Note: coming OFF unlimited (old rate 0) starts
        // empty — refill() added nothing at rate 0, so the
        // balance was 0 and the clamp keeps it there. No
        // free refill is granted on a rate change.
    }

    /// Whole tokens available right now (refills first).
    /// Unlimited (rate 0) reports `u64::MAX`.
    pub fn available(&mut self, now_ms: u64) -> u64 {
        self.refill(now_ms);
        if self.rate_per_sec == 0 {
            return u64::MAX;
        }
        u64::try_from(self.tokens_milli / 1000).unwrap_or(u64::MAX)
    }

    /// Take exactly `want` tokens: succeeds only when the
    /// FULL want is available. Rate 0 always succeeds.
    pub fn try_take(&mut self, want: u64, now_ms: u64) -> bool {
        self.refill(now_ms);
        if self.rate_per_sec == 0 {
            return true;
        }
        let cost = u128::from(want) * 1000;
        if self.tokens_milli >= cost {
            self.tokens_milli -= cost;
            true
        } else {
            false
        }
    }

    /// Milliseconds until `want` tokens are available: 0
    /// when a take would succeed now, when the rate is 0,
    /// or when `want` is 0; otherwise
    /// `ceil((want - tokens) * 1000 / rate)` in u128 math.
    pub fn wait_ms_for(&mut self, want: u64, now_ms: u64) -> u64 {
        self.refill(now_ms);
        if self.rate_per_sec == 0 || want == 0 {
            return 0;
        }
        let cost = u128::from(want) * 1000;
        if self.tokens_milli >= cost {
            return 0;
        }
        let deficit_milli = cost - self.tokens_milli;
        // Milli-tokens accrue at `rate` per ms, so the wait
        // is ceil(deficit_milli / rate) — the same value as
        // ceil((want - tokens) * 1000 / rate) over exact
        // (fractional) tokens.
        let rate = u128::from(self.rate_per_sec);
        let wait = (deficit_milli + rate - 1) / rate;
        u64::try_from(wait).unwrap_or(u64::MAX)
    }
}

/// Parse a `Content-Range` header (`bytes 0-1023/2048`) into
/// `(start, end INCLUSIVE, total)`. An unknown total (`*`),
/// a unit-range form, or numbers that do not cohere
/// (start past end, end at or past the total) are `None`.
pub fn parse_content_range(header: &str) -> Option<(u64, u64, u64)> {
    let header = header.trim();
    let rest = header
        .get(..5)
        .filter(|unit| unit.eq_ignore_ascii_case("bytes"))
        .map(|_| &header[5..])?
        .trim_start();
    let (range, total) = rest.split_once('/')?;
    let total: u64 = total.trim().parse().ok()?;
    let (start, end) = range.trim().split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().ok()?;
    if start > end || total == 0 || end >= total {
        return None;
    }
    Some((start, end, total))
}

// ---------------------------------------------------------------------------
// Speed / ETA
// ---------------------------------------------------------------------------

/// A trailing-window transfer rate: `(ms, total_bytes)`
/// samples, rated over the last [`SPEED_WINDOW_MS`] of the
/// newest sample. Bytes going backwards reset the baseline
/// (a restart must not print a negative or absurd rate).
#[derive(Debug, Clone, Default)]
pub struct SpeedMeter {
    samples: Vec<(u64, u64)>,
}

impl SpeedMeter {
    pub fn new() -> SpeedMeter {
        SpeedMeter {
            samples: Vec::new(),
        }
    }

    /// Record the task's total downloaded bytes at `now_ms`.
    /// Samples older than the window (relative to `now_ms`)
    /// are dropped; a total below the previous sample resets
    /// the baseline to this sample alone.
    pub fn record(&mut self, now_ms: u64, total_bytes: u64) {
        if let Some((_, last_total)) = self.samples.last() {
            if total_bytes < *last_total {
                self.samples.clear();
            }
        }
        self.samples.push((now_ms, total_bytes));
        let floor = now_ms.saturating_sub(SPEED_WINDOW_MS);
        self.samples.retain(|(t, _)| *t >= floor);
    }

    /// Bytes per second over the window ending at `now_ms`:
    /// `(bytes_newest - bytes_oldest_in_window) / dt`. `0.0`
    /// with fewer than two samples in the window, a span
    /// under [`SPEED_MIN_DT_MS`], or a backwards step.
    pub fn rate(&self, now_ms: u64) -> f64 {
        let floor = now_ms.saturating_sub(SPEED_WINDOW_MS);
        let window: Vec<(u64, u64)> = self
            .samples
            .iter()
            .filter(|(t, _)| *t >= floor)
            .copied()
            .collect();
        if window.len() < 2 {
            return 0.0;
        }
        let (Some((t0, b0)), Some((t1, b1))) = (window.first().copied(), window.last().copied())
        else {
            return 0.0;
        };
        let dt = t1.saturating_sub(t0);
        if dt < SPEED_MIN_DT_MS || b1 < b0 {
            return 0.0;
        }
        (b1 - b0) as f64 / (dt as f64 / 1000.0)
    }

    pub fn reset(&mut self) {
        self.samples.clear();
    }
}

/// Seconds left at `rate` bytes/sec, when the total is
/// known and the rate is usable. A finished (or
/// over-finished) task has 0 seconds left.
pub fn eta_seconds(total: Option<u64>, downloaded: u64, rate: f64) -> Option<u64> {
    let total = total?;
    if downloaded >= total {
        return Some(0);
    }
    if !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    let remaining = (total - downloaded) as f64;
    Some((remaining / rate).ceil() as u64)
}

/// Completion percentage in `0.0..=100.0`; `None` for an
/// unknown total. A zero-byte total is trivially complete.
pub fn percent(downloaded: u64, total: Option<u64>) -> Option<f64> {
    let total = total?;
    if total == 0 {
        return Some(100.0);
    }
    Some((downloaded as f64 / total as f64 * 100.0).min(100.0))
}

/// `humanize_bytes(rate) + "/s"` — the one byte humanizer
/// (`core::stats`) does the formatting, as everywhere else.
pub fn speed_text(rate: f64) -> String {
    let bytes = if rate.is_finite() && rate > 0.0 {
        rate.round() as u64
    } else {
        0
    };
    format!("{}/s", crate::core::stats::humanize_bytes(bytes))
}

/// `"—"` for an unknown ETA, else the shared duration
/// humanizer over the seconds.
pub fn eta_text(eta: Option<u64>) -> String {
    match eta {
        None => "—".to_string(),
        Some(seconds) => crate::core::stats::humanize_duration(seconds.saturating_mul(1000)),
    }
}

// ---------------------------------------------------------------------------
// Persisted task files
// ---------------------------------------------------------------------------

/// Load every task file in `dir`: `*.json` files up to
/// 1 MiB, parsed and sanitized; failures are skipped
/// silently (core never logs — the manager logs what it
/// dropped). A task that was `Downloading` on disk loads as
/// `Paused` — nothing is running after a restart until the
/// user (or the scheduler) resumes it. Sorted by `added_ms`,
/// then id. A missing directory is an empty list.
///
/// AUDIT 0.32.1 hardening of the load (P1-20 / P4-A3 /
/// P4-13): a task persisted as `Queued` ALSO loads as
/// `Paused` (a queued task has no supervisor after a
/// restart and the panel offers it no resume — parked is
/// the actionable state, exactly like Downloading); a
/// second file carrying an id already loaded is dropped
/// (two rows aliasing one task); and a non-Complete task
/// whose file name is already claimed by an earlier task is
/// deduped (two live tasks must never share one `.part`).
pub fn load_tasks(dir: &Path) -> Vec<TaskState> {
    let mut out: Vec<TaskState> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut seen_ids: HashSet<u64> = HashSet::new();
    let mut claimed_names: HashSet<String> = HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        match path.extension() {
            Some(ext) if ext == "json" => {}
            _ => continue,
        }
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() <= MAX_TASK_FILE_BYTES => {}
            _ => continue,
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(task) = serde_json::from_str::<TaskState>(&text) else {
            continue;
        };
        if let Some(mut task) = task.sanitized() {
            if matches!(task.status, TaskStatus::Downloading | TaskStatus::Queued) {
                task.status = TaskStatus::Paused;
            }
            if !seen_ids.insert(task.id) {
                continue;
            }
            if task.status != TaskStatus::Complete && claimed_names.contains(&task.file_name) {
                task.file_name = dedupe_file_name(&task.file_name, |c| claimed_names.contains(c));
            }
            claimed_names.insert(task.file_name.clone());
            out.push(task);
        }
    }
    out.sort_by(|a, b| a.added_ms.cmp(&b.added_ms).then(a.id.cmp(&b.id)));
    out
}

/// Persist one task atomically: serialize to
/// `<id>.json.tmp-<pid>-<seq>`, fsync, rename over
/// `<id>.json` — the same tmp+rename pattern as
/// `core::state`'s save, so a crash mid-write never tears
/// the previous file. The `<seq>` is a process-wide atomic
/// counter (AUDIT 0.32.1, P1-8): the pid alone made every
/// concurrent save of the same task — supervisor, UI action,
/// shutdown — collide on ONE temp path, losing updates and
/// occasionally renaming a torn write into place.
pub fn save_task(dir: &Path, t: &TaskState) -> std::io::Result<()> {
    static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir)?;
    let name = format!("{}.json", t.id);
    let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!("{name}.tmp-{}-{seq}", std::process::id()));
    let text = serde_json::to_string_pretty(t)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dir.join(name))
}

/// Remove a task's state file (best-effort: cancel paths
/// call this whether or not the file is still there).
pub fn delete_task_file(dir: &Path, id: u64) {
    let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
}

/// Where downloads land: the exe folder's `Downloads` when
/// the exe folder is writable; else the user's own Downloads
/// folder when known; else the exe folder's `Downloads`
/// anyway (last resort — the app logs the choice and why).
pub fn resolve_downloads_dir(
    exe_dir: &Path,
    user_downloads: Option<&Path>,
    exe_writable: bool,
) -> PathBuf {
    if exe_writable {
        return exe_dir.join("Downloads");
    }
    if let Some(user) = user_downloads {
        return user.to_path_buf();
    }
    exe_dir.join("Downloads")
}
