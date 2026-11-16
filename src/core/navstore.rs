//! Bookmarks + history store (0.41.0, Workstream B) — the
//! user's own navigation record: pages they bookmarked and
//! pages they visited. Pure core: no GUI deps, no clock of
//! its own (timestamps are injected by the caller as
//! milliseconds since the Unix epoch), no I/O beyond the
//! one file this module owns.
//!
//! STORAGE CHOICE. A dedicated file, `nav.json`, in the
//! Pico home (`%LOCALAPPDATA%\Pico\`, resolved by
//! `core::state` exactly like `state.json`). NOT a field
//! inside `state.json`: that file is documented as < 4 KiB
//! and capped at 1 MiB, and 2,000 history entries do not
//! belong in it — a hand-grown or corrupt nav file must
//! never take the settings/theme/profile bookkeeping down
//! with it, and vice versa. The WRITE discipline is the
//! state machinery's, replicated exactly: serialize, write
//! to a per-process temp sibling, fsync, clear a read-only
//! destination, rename over it; a failed save removes its
//! temp so the loader can never promote it. Loads mirror
//! `core::state::load_from_checked`: a size cap checked
//! from metadata BEFORE reading, a missing file is
//! `Missing` (defaults), an unusable one is `Corrupt`
//! (defaults) — and, since 0.41.1, a corrupt file is
//! quarantined to `nav.json.corrupt` at load (so the
//! session's first save cannot silently overwrite the
//! only copy), a complete save temp is promoted over a
//! missing/corrupt main file, parsing is per-entry
//! tolerant, and every save merges the on-disk store
//! under a lock (two running Picos must not erase each
//! other's records). Every loaded value passes through
//! [`NavStore::sanitized`] before use.
//!
//! RECORDING RULES (the privacy scope, pinned by tests):
//! a visit is recorded only for a real http/https URL
//! (localhost pages are http, so they ride the same
//! rule) or a `file://`
//! URL naming a local path. (Pico's own file-server
//! pages pass this module's predicate too — they are
//! http — but the shell never offers them for recording
//! since 0.41.1: their port + root token are per-session,
//! so a stored entry would be a dead link.) `pico://` internal pages are
//! NEVER recorded — the check is explicit and runs first,
//! before the scheme rules — and neither is anything else
//! (`about:`, `data:`, credentialed URLs are refused too:
//! a userinfo URL's password must never land in a
//! persisted, searchable record). Re-visiting a URL
//! updates its entry in place: title (when the new one is
//! non-empty), `last_visited`, and `visits += 1` — never a
//! duplicate row.
//!
//! CAPS. History is capped at [`MAX_HISTORY`] entries;
//! when a new entry would exceed the cap the
//! least-recently-visited entries are pruned (the store is
//! kept most-recent-first, so pruning is a truncate from
//! the tail). Bookmarks are capped at [`MAX_BOOKMARKS`]
//! the same way (oldest-added pruned). Titles are capped
//! at 512 chars and stripped of control characters; URLs
//! longer than [`MAX_NAV_URL_CHARS`] are not recorded.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Schema version of the nav file (independent of the
/// state file's schema — the two files evolve separately).
pub const NAV_SCHEMA_VERSION: u32 = 1;
/// Hard cap on remembered history entries.
pub const MAX_HISTORY: usize = 2000;
/// Hard cap on bookmarks.
pub const MAX_BOOKMARKS: usize = 1000;
/// Hard cap on a recorded URL's length, in chars.
pub const MAX_NAV_URL_CHARS: usize = 2048;
/// Hard cap on a recorded title's length, in chars.
pub const MAX_NAV_TITLE_CHARS: usize = 512;
/// Largest nav file we will even read. 2,000 entries at
/// the field caps above fit comfortably under this; a
/// bigger file is a hand-grown anomaly, not data.
pub const MAX_NAV_BYTES: u64 = 8 * 1024 * 1024;

/// One bookmark: the URL, the title the page reported
/// when it was bookmarked (or the host fallback), and
/// when it was added (ms since the Unix epoch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub added_at: u64,
}

/// One history entry: a visited URL, its latest known
/// title, when it was last visited (ms since the Unix
/// epoch), and how many times it has been visited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub last_visited: u64,
    pub visits: u32,
}

/// Which kind of record a suggestion came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SuggestionKind {
    Bookmark,
    History,
}

/// One address-bar suggestion row's data (the "go" row for
/// the raw typed input is the chrome's own — it is not
/// data and never appears here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub url: String,
    pub title: String,
    pub kind: SuggestionKind,
}

/// One deletion tombstone (AUDIT 0.41.3, Front A F3):
/// the canonical key of an entry the user deleted, and
/// when (ms since the Unix epoch). Tombstones are what
/// make a deletion survive [`merge_nav_stores`]: the
/// merge is a union, so without them a second process
/// holding the pre-deletion snapshot resurrects every
/// cleared/removed entry on its next save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub key: String,
    pub at_ms: u64,
}

/// Hard cap on persisted tombstones; past it the
/// oldest (by `at_ms`) are pruned.
pub const MAX_TOMBSTONES: usize = 512;

/// The whole store. `bookmarks` is kept most-recently-
/// added first; `history` most-recently-visited first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavStore {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// The clear-history watermark (AUDIT 0.41.3,
    /// Front A F3): a history entry survives a merge /
    /// sanitize only when it was visited AFTER this
    /// instant. Serde-defaulted (0 = never cleared), so
    /// every pre-0.41.3 nav.json loads unchanged.
    #[serde(default)]
    pub history_cleared_at: u64,
    /// Per-entry deletion tombstones (AUDIT 0.41.3,
    /// Front A F3). Serde-defaulted, same as above.
    #[serde(default)]
    pub tombstones: Vec<Tombstone>,
}

impl Default for NavStore {
    fn default() -> Self {
        NavStore {
            version: NAV_SCHEMA_VERSION,
            bookmarks: Vec::new(),
            history: Vec::new(),
            history_cleared_at: 0,
            tombstones: Vec::new(),
        }
    }
}

/// Normalize a tombstone list in place: dedupe by key
/// keeping the max `at_ms`, sort by (`at_ms`, key) so
/// two runs agree, and cap at [`MAX_TOMBSTONES`]
/// pruning the oldest by `at_ms`.
fn prune_tombstones(tombstones: &mut Vec<Tombstone>) {
    let mut deduped: Vec<Tombstone> = Vec::with_capacity(tombstones.len());
    for t in tombstones.drain(..) {
        match deduped.iter_mut().find(|x| x.key == t.key) {
            Some(existing) => existing.at_ms = existing.at_ms.max(t.at_ms),
            None => deduped.push(t),
        }
    }
    deduped.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.key.cmp(&b.key)));
    if deduped.len() > MAX_TOMBSTONES {
        deduped.drain(..deduped.len() - MAX_TOMBSTONES);
    }
    *tombstones = deduped;
}

/// The host a URL belongs to, for fallback titles and
/// display: the web origin's host, `localhost`-style
/// hosts included; for a `file://` URL the file name.
pub fn host_of(url: &str) -> String {
    if let Some((_, host, _)) = crate::core::addr::web_origin(url) {
        return host;
    }
    if url.starts_with("file://") {
        if let Some(path) = crate::core::addr::file_url_to_path(url) {
            if let Some(name) = path.file_name() {
                return name.to_string_lossy().into_owned();
            }
        }
    }
    String::new()
}

/// The title to store when a page reported none (or an
/// empty one): its host, or the URL itself when even that
/// fails (a bounded, control-free string by the time the
/// recordability gate has passed).
pub fn fallback_title(url: &str) -> String {
    let host = host_of(url);
    if host.is_empty() {
        url.chars().take(MAX_NAV_TITLE_CHARS).collect()
    } else {
        host
    }
}

fn clean_title(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAV_TITLE_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The canonical identity of a recorded URL (AUDIT
/// 0.41.1, Front B F5). Through 0.41.0 a record's
/// identity was its exact string, so one page split
/// into several entries across spellings that name the
/// same resource: fragment variants, a trailing-dot
/// host, an explicit default port, host case. The key
/// lowercases scheme+host (via `web_origin`, which also
/// fills the default port — dropped here), drops the
/// default port and one trailing host dot, and drops
/// the fragment; path and query stay byte-exact (they
/// are case-sensitive data). The STORED string is
/// untouched — the first-seen spelling is what the UI
/// shows and opens; the key only decides sameness.
/// Non-web URLs (file://) key on the string minus its
/// fragment.
pub fn canonical_nav_key(url: &str) -> String {
    if let Some((scheme, host, port)) = crate::core::addr::web_origin(url) {
        let host = host.strip_suffix('.').unwrap_or(&host).to_string();
        let default = if scheme == "http" { 80 } else { 443 };
        let mut key = if host.contains(':') {
            format!("{scheme}://[{host}]")
        } else {
            format!("{scheme}://{host}")
        };
        if port != default {
            key.push(':');
            key.push_str(&port.to_string());
        }
        // Path + query from the original spelling,
        // fragment dropped.
        if let Some((_, rest)) = url.split_once("://") {
            let end = rest
                .find(|c| ['/', '?', '#'].contains(&c))
                .unwrap_or(rest.len());
            let tail = &rest[end..];
            let tail = tail.split('#').next().unwrap_or(tail);
            // AUDIT 0.41.3 (Front A F7): an empty path
            // tail normalizes to "/" — `https://example.com`
            // and `https://example.com/` are one page and
            // now share one key (a query-only tail gets
            // the "/" in front of its `?` for the same
            // reason).
            if tail.is_empty() {
                key.push('/');
            } else if let Some(query) = tail.strip_prefix('?') {
                key.push_str("/?");
                key.push_str(query);
            } else {
                key.push_str(tail);
            }
        }
        return key;
    }
    url.split('#').next().unwrap_or(url).to_string()
}

/// Is this URL one of Pico's internal pages in wry's
/// folded spelling, tolerant of the case/port/
/// trailing-dot variation WebView2 can hand us (AUDIT
/// 0.41.1, Front B F4)? `normalize_pico_url` folds
/// case-SENSITIVELY, so `https://PICO.start/`,
/// `https://pico.start:443/` and `https://pico.start./`
/// all escaped the recordability gate's internal test
/// through 0.41.0. This check is deliberately LOCAL to
/// the nav store — the shared fold feeds navigation
/// gates whose exact-spelling discipline is a security
/// property and must not change.
fn is_folded_internal(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://pico.")
        .or_else(|| lower.strip_prefix("http://pico."));
    let Some(rest) = rest else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => authority,
    };
    let host = host.strip_suffix('.').unwrap_or(host);
    matches!(host, "start" | "downloads" | "history")
}

/// May this URL be recorded (history) or bookmarked at
/// all? The recording rules from the module header, as one
/// pure predicate — the single gate every write path and
/// the sanitizer run.
pub fn is_recordable_url(url: &str) -> bool {
    // NEVER internal pages — first, explicitly, so no
    // future scheme rule can accidentally admit them.
    // Both spellings: the canonical pico:// form and
    // wry's folded https://pico.* form.
    let folded = crate::core::addr::normalize_pico_url(url);
    if folded.starts_with("pico://") || is_folded_internal(url) {
        return false;
    }
    if url.is_empty() || url.chars().count() > MAX_NAV_URL_CHARS {
        return false;
    }
    if url
        .chars()
        .any(|c: char| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    // Credentialed URLs are never recorded: the userinfo
    // (a password, classically) must not persist.
    if crate::core::ipc::url_has_userinfo(url) {
        return false;
    }
    if crate::core::addr::web_origin(url).is_some() {
        return true;
    }
    // A file:// URL naming a local path (file_url_to_path
    // refuses remote authorities and UNC shapes).
    if url.to_ascii_lowercase().starts_with("file://") {
        return crate::core::addr::file_url_to_path(url).is_some();
    }
    false
}

/// Does a completed navigation's verdict mean a real
/// page was DISPLAYED at its URL (0.41.2)? The verdict
/// is the navigation's own `NavigationCompleted` args:
/// `success` (IsSuccess) and `http_status` (the HTTP
/// status the navigation received, 0 when no response
/// arrived — both read from the SAME navigation's args
/// by the shell's raw registration).
///
/// A navigation counts as a visit when it succeeded,
/// OR when it received any HTTP response at all: an
/// HTTP error page (404/500) is a real page the user
/// saw — WebView2 reports it with IsSuccess=FALSE
/// (Microsoft Learn, CoreWebView2NavigationCompleted-
/// EventArgs), and browsers record it. A failure with
/// NO response (status 0 — DNS failure, refused,
/// reset, timeout, cert error) displayed the browser's
/// own error page, not a page at the URL: not a visit.
///
/// This replaces the 0.41.1 gate (Front B F1), which
/// compared the committed URL against the tab's "last
/// failed source" — a mark written by a SEPARATE event
/// stream, keyed by the core's LIVE Source property
/// read at dispatch time (the completed args carry no
/// URL). When a new navigation started before the
/// previous page finished loading — the normal pattern
/// on a slow connection, where a page is clickable
/// seconds before NavigationCompleted fires — the
/// aborted navigation's failure verdict was stamped
/// with the NEW page's URL (Source advances before
/// ContentLoading), and the new page's own record was
/// then refused. First navigations in a fresh tab
/// survived (empty mark); later ones were refused
/// whenever a matching mark existed. See BUILD_NOTES
/// (0.41.2). Per-tab session history never consults
/// this: Back must keep working to and from error
/// pages.
pub fn verdict_records_visit(success: bool, http_status: i32) -> bool {
    success || http_status != 0
}

/// One tab's pending history visits (0.41.2; the
/// id-keyed candidate queue is AUDIT 0.41.3, Front A
/// F2). A navigation has committed (its page is
/// displayed), but its completion verdict has not been
/// processed yet. The commit STASHES the URL here
/// instead of recording at once; the verdict decides
/// it via [`PendingVisit::note_verdict`].
///
/// Through 0.41.2 this was a SINGLE slot: the verdict
/// carries no URL, so it decided whatever candidate
/// happened to be current — and the pairing argument
/// ("a verdict is always processed before the next
/// commit") was a queue-order assumption about the
/// shell, not a property of the data. Candidates and
/// verdicts are now paired by the navigation's own id
/// (`nav_id`, supplied by the shell): a verdict finds
/// ITS candidate by id, wherever it sits in the queue,
/// so interleaved commits can never stamp one
/// navigation's verdict onto another's URL. A verdict
/// for an id that is not queued (already flushed or
/// evicted) decides NOTHING ([`VerdictOutcome::NoMatch`]).
/// A candidate still pending when the tab goes away
/// is flushed as a record by [`PendingVisit::flush_all`]
/// — it committed and displayed. A lost verdict
/// therefore degrades to recording (the pre-0.41.1
/// behavior), never to silence; a candidate pushed out
/// of the bounded queue is returned by
/// [`PendingVisit::note_candidate`] for the same
/// immediate recording (the lost-verdict doctrine).
#[derive(Debug, Default, Clone)]
pub struct PendingVisit {
    queue: Vec<(u64, String)>,
}

/// How many candidates one tab may hold undecided.
/// Verdicts normally land within one navigation of
/// their commit, so 8 is generous headroom; past it the
/// oldest candidate is evicted for recording (see
/// [`PendingVisit::note_candidate`]).
pub const MAX_PENDING_VISITS: usize = 8;

/// What one verdict did to its candidate (0.41.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictOutcome {
    /// The verdict proves a page was displayed: record
    /// this URL.
    Record(String),
    /// The verdict proves NO page was displayed (a
    /// transport failure): drop this URL.
    Drop(String),
    /// No candidate carries the verdict's nav id — it
    /// was already flushed or evicted, and this verdict
    /// must decide nothing.
    NoMatch,
}

impl PendingVisit {
    pub fn new() -> Self {
        Self { queue: Vec::new() }
    }

    /// A navigation committed: `(nav_id, url)` becomes
    /// a pending candidate. A repeat `nav_id` replaces
    /// its URL in place (same queue position). When the
    /// queue exceeds [`MAX_PENDING_VISITS`], the OLDEST
    /// candidate is evicted and its URL returned for
    /// immediate recording (the lost-verdict doctrine);
    /// otherwise `None`.
    pub fn note_candidate(&mut self, nav_id: u64, url: &str) -> Option<String> {
        if let Some(slot) = self.queue.iter_mut().find(|(id, _)| *id == nav_id) {
            slot.1 = url.to_string();
            return None;
        }
        self.queue.push((nav_id, url.to_string()));
        if self.queue.len() > MAX_PENDING_VISITS {
            return Some(self.queue.remove(0).1);
        }
        None
    }

    /// The completion verdict for navigation `nav_id`
    /// arrived: find that candidate BY ID (never by
    /// position), remove it, and apply
    /// [`verdict_records_visit`] to it. An absent id is
    /// [`VerdictOutcome::NoMatch`].
    pub fn note_verdict(&mut self, nav_id: u64, success: bool, http_status: i32) -> VerdictOutcome {
        let Some(pos) = self.queue.iter().position(|(id, _)| *id == nav_id) else {
            return VerdictOutcome::NoMatch;
        };
        let (_, url) = self.queue.remove(pos);
        if verdict_records_visit(success, http_status) {
            VerdictOutcome::Record(url)
        } else {
            VerdictOutcome::Drop(url)
        }
    }

    /// Flush without verdicts (the tab is closing, the
    /// app is closing): every pending candidate
    /// committed and displayed, so all of them record,
    /// in FIFO (commit) order.
    pub fn flush_all(&mut self) -> Vec<String> {
        self.queue.drain(..).map(|(_, url)| url).collect()
    }
}

/// Case-insensitive substring match of an ALREADY-LOWERCASED
/// query (`q`, produced once per search by the caller) against
/// a record's title, URL, and host. An empty query matches
/// everything (the dropdown's "just focused" shape and the
/// history page's unfiltered list both rely on it).
///
/// PERF (0.41.2): through 0.41.1 this function lowercased the
/// QUERY on every call — one allocation per store entry per
/// keystroke — on top of the per-entry title/URL lowercases.
/// The query lowercase is now hoisted to the two call sites
/// (computed once per search, not once per entry); the three
/// disjuncts below are the original expressions verbatim
/// (`str::to_lowercase` is kept rather than a hand-rolled
/// lowering: it alone applies the Greek final-sigma context
/// rule). An allocation-free ASCII byte scan was tried and
/// REVERTED on measurement: it replaced memchr-backed
/// `str::contains` with a naive window scan and made
/// miss-heavy searches slower. Equivalence is pinned by the
/// differential tests in `tests/perf_equiv.rs`.
fn matches_query(q: &str, title: &str, url: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    title.to_lowercase().contains(q) || url.to_lowercase().contains(q) || host_of(url).contains(q)
}

/// The frecency score of one history entry at `now_ms`
/// (0.41.0): visits weighted by recency, as ONE
/// deterministic integer —
///
/// ```text
/// score = visits * recency_weight(age)
/// age <= 1 hour  -> weight 100
/// age <= 1 day   -> weight  70
/// age <= 7 days  -> weight  40
/// age <= 30 days -> weight  20
/// older          -> weight  10
/// ```
///
/// A page visited 10 times today (1000) outranks one
/// visited 50 times a year ago (500); a page visited
/// twice in the last hour (200) outranks one visited
/// 4 times last month (80). Ordering ties break on
/// `last_visited` (newer first), then URL ascending —
/// the full ordering is total, so two runs over the
/// same store always agree (pinned by test).
pub fn frecency_score(entry: &HistoryEntry, now_ms: u64) -> u64 {
    const HOUR: u64 = 3_600_000;
    const DAY: u64 = 24 * HOUR;
    let age = now_ms.saturating_sub(entry.last_visited);
    let weight: u64 = if age <= HOUR {
        100
    } else if age <= DAY {
        70
    } else if age <= 7 * DAY {
        40
    } else if age <= 30 * DAY {
        20
    } else {
        10
    };
    (entry.visits as u64).saturating_mul(weight)
}

impl NavStore {
    /// Sanitize a store that came from disk (or anywhere
    /// untrusted): drop unrecordable URLs, clean titles,
    /// fill empty titles with the fallback, dedupe by
    /// canonical key (bookmarks keep the first/newest;
    /// history merges duplicates by summing visits and
    /// taking the latest visit/title), re-sort both
    /// lists newest-first, and enforce both caps.
    /// `now_ms` is the caller's clock (this module keeps
    /// none of its own): timestamps from the future are
    /// clamped to it (AUDIT 0.41.1, Front B S2 — through
    /// 0.41.0 a hand-grown future `last_visited` pinned
    /// an entry to the top of the recency sort and the
    /// frecency table indefinitely).
    pub fn sanitized(mut self, now_ms: u64) -> Self {
        self.version = NAV_SCHEMA_VERSION;
        // AUDIT 0.41.3 (Front A F3): a loaded store
        // self-cleans against its own deletion record —
        // the same survival filters `merge_nav_stores`
        // applies (see there for the rules and the
        // same-millisecond caveat). Tombstones are
        // normalized first so the lookups below see one
        // entry per key at its max `at_ms`.
        prune_tombstones(&mut self.tombstones);
        let tombstone_at: std::collections::HashMap<&str, u64> = self
            .tombstones
            .iter()
            .map(|t| (t.key.as_str(), t.at_ms))
            .collect();
        // PERF (0.41.2): both dedupe passes below used to
        // recompute the canonical key of every already-kept
        // entry for every candidate — O(N²) key computations
        // (a key is a URL parse plus allocations), measured
        // at ~0.3–8.7 s for a full store on the bench host.
        // Keys are now computed once per entry and carried in
        // a set/index map; the kept set never holds two
        // entries with the same key, so set membership /
        // first-index lookup is exactly the old linear scan's
        // verdict. Pinned by the differential tests in
        // `tests/perf_equiv.rs`.
        let mut bookmarks: Vec<Bookmark> = Vec::new();
        let mut bookmark_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
        for mut b in self.bookmarks {
            if !is_recordable_url(&b.url) {
                continue;
            }
            let key = canonical_nav_key(&b.url);
            // AUDIT 0.41.3 (F3): a bookmark survives only
            // when it was added after its tombstone.
            if let Some(&at) = tombstone_at.get(key.as_str()) {
                if b.added_at.min(now_ms) <= at {
                    continue;
                }
            }
            if !bookmark_keys.insert(key) {
                continue;
            }
            b.title = clean_title(&b.title);
            if b.title.is_empty() {
                b.title = fallback_title(&b.url);
            }
            b.added_at = b.added_at.min(now_ms);
            bookmarks.push(b);
        }
        // AUDIT 0.41.1 (Front B S1): the documented cap
        // rule is "oldest-added pruned", so sort by
        // added_at BEFORE truncating — through 0.41.0
        // bookmarks were pruned by file position (the
        // history half of this function already sorted).
        bookmarks.sort_by(|a, b| b.added_at.cmp(&a.added_at).then_with(|| a.url.cmp(&b.url)));
        bookmarks.truncate(MAX_BOOKMARKS);
        self.bookmarks = bookmarks;

        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut history_index: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for mut e in self.history {
            if !is_recordable_url(&e.url) {
                continue;
            }
            e.title = clean_title(&e.title);
            if e.title.is_empty() {
                e.title = fallback_title(&e.url);
            }
            if e.visits == 0 {
                e.visits = 1;
            }
            e.last_visited = e.last_visited.min(now_ms);
            // AUDIT 0.41.3 (F3): a history entry survives
            // only when visited after the clear watermark
            // AND after its tombstone (when one exists).
            if e.last_visited <= self.history_cleared_at {
                continue;
            }
            let key = canonical_nav_key(&e.url);
            if let Some(&at) = tombstone_at.get(key.as_str()) {
                if e.last_visited <= at {
                    continue;
                }
            }
            if let Some(&idx) = history_index.get(&key) {
                let existing = &mut history[idx];
                existing.visits = existing.visits.saturating_add(e.visits);
                if e.last_visited >= existing.last_visited {
                    existing.last_visited = e.last_visited;
                    existing.title = e.title;
                }
                continue;
            }
            history_index.insert(key, history.len());
            history.push(e);
        }
        history.sort_by(|a, b| {
            b.last_visited
                .cmp(&a.last_visited)
                .then_with(|| a.url.cmp(&b.url))
        });
        history.truncate(MAX_HISTORY);
        self.history = history;
        self
    }

    // -- bookmarks -------------------------------------------------

    pub fn is_bookmarked(&self, url: &str) -> bool {
        // Canonical identity (0.41.1, Front B F5): a
        // spelling variant of a bookmarked page IS the
        // bookmark.
        let key = canonical_nav_key(url);
        self.bookmarks
            .iter()
            .any(|b| canonical_nav_key(&b.url) == key)
    }

    /// Record a deletion tombstone for `key` at
    /// `now_ms` (AUDIT 0.41.3, Front A F3): deduped by
    /// key keeping the max `at_ms`, capped by
    /// [`prune_tombstones`].
    fn push_tombstone(&mut self, key: &str, now_ms: u64) {
        match self.tombstones.iter_mut().find(|t| t.key == key) {
            Some(existing) => existing.at_ms = existing.at_ms.max(now_ms),
            None => self.tombstones.push(Tombstone {
                key: key.to_string(),
                at_ms: now_ms,
            }),
        }
        prune_tombstones(&mut self.tombstones);
    }

    /// A fresh action on `key` supersedes its deletion
    /// (AUDIT 0.41.3, Front A F3): drop the tombstone
    /// when it is not newer than the action. A tombstone
    /// from the FUTURE relative to the caller's clock
    /// (another process ahead) is kept — it still
    /// describes a deletion this action cannot know it
    /// postdates.
    fn drop_tombstone(&mut self, key: &str, now_ms: u64) {
        self.tombstones
            .retain(|t| !(t.key == key && t.at_ms <= now_ms));
    }

    /// Toggle one URL's bookmark. Returns the NEW state
    /// (`true` = now bookmarked). An unrecordable URL
    /// changes nothing and returns `false`.
    pub fn toggle_bookmark(&mut self, url: &str, title: &str, now_ms: u64) -> bool {
        if self.remove_bookmark(url, now_ms) {
            return false;
        }
        if !is_recordable_url(url) {
            return false;
        }
        // AUDIT 0.41.3 (F3): bookmarking is a fresh
        // action — it supersedes any deletion of the key.
        self.drop_tombstone(&canonical_nav_key(url), now_ms);
        let mut title = clean_title(title);
        if title.is_empty() {
            title = fallback_title(url);
        }
        self.bookmarks.insert(
            0,
            Bookmark {
                url: url.to_string(),
                title,
                added_at: now_ms,
            },
        );
        self.bookmarks.truncate(MAX_BOOKMARKS);
        true
    }

    /// Remove one bookmark AND tombstone its canonical
    /// key at `now_ms` (AUDIT 0.41.3, Front A F3 — the
    /// `now_ms` parameter follows this module's standing
    /// pattern: no clock of its own, callers inject time
    /// exactly as `record_visit`/`toggle_bookmark`
    /// already did; the remove/clear verbs were the only
    /// ones taking no time, because through 0.41.2 a
    /// deletion needed no timestamp). A tombstone is
    /// pushed only when an entry was actually removed:
    /// a remove that matched nothing (a toggle's probe,
    /// a stale UI row) must not suppress another
    /// process's live entry.
    pub fn remove_bookmark(&mut self, url: &str, now_ms: u64) -> bool {
        let key = canonical_nav_key(url);
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| canonical_nav_key(&b.url) != key);
        let removed = self.bookmarks.len() != before;
        if removed {
            self.push_tombstone(&key, now_ms);
        }
        removed
    }

    // -- history ---------------------------------------------------

    /// Record one visit. Returns `true` when the store
    /// changed (a new entry, or an in-place update).
    /// Unrecordable URLs change nothing.
    pub fn record_visit(&mut self, url: &str, title: &str, now_ms: u64) -> bool {
        if !is_recordable_url(url) {
            return false;
        }
        let title = clean_title(title);
        let key = canonical_nav_key(url);
        // AUDIT 0.41.3 (F3): a fresh visit supersedes a
        // deletion of the same key (see drop_tombstone).
        self.drop_tombstone(&key, now_ms);
        if let Some(pos) = self
            .history
            .iter()
            .position(|e| canonical_nav_key(&e.url) == key)
        {
            let mut entry = self.history.remove(pos);
            if !title.is_empty() {
                entry.title = title;
            }
            entry.last_visited = now_ms;
            entry.visits = entry.visits.saturating_add(1);
            self.history.insert(0, entry);
            return true;
        }
        let title = if title.is_empty() {
            fallback_title(url)
        } else {
            title
        };
        self.history.insert(
            0,
            HistoryEntry {
                url: url.to_string(),
                title,
                last_visited: now_ms,
                visits: 1,
            },
        );
        // Cap: prune least-recently-visited (the tail —
        // the store is most-recent-first by construction).
        self.history.truncate(MAX_HISTORY);
        true
    }

    /// Follow a page's title as it arrives (the tab title
    /// channel reports after the commit that recorded the
    /// visit): update the matching entry's title in place.
    /// Returns `true` when an entry was updated.
    pub fn note_title(&mut self, url: &str, title: &str) -> bool {
        let title = clean_title(title);
        if title.is_empty() {
            return false;
        }
        let key = canonical_nav_key(url);
        match self
            .history
            .iter_mut()
            .find(|e| canonical_nav_key(&e.url) == key)
        {
            Some(e) if e.title != title => {
                e.title = title;
                true
            }
            _ => false,
        }
    }

    /// Remove one history entry AND tombstone its
    /// canonical key at `now_ms` (AUDIT 0.41.3, Front A
    /// F3 — see `remove_bookmark` for the timestamp
    /// pattern and the only-on-actual-removal rule).
    pub fn remove_history(&mut self, url: &str, now_ms: u64) -> bool {
        let key = canonical_nav_key(url);
        let before = self.history.len();
        self.history.retain(|e| canonical_nav_key(&e.url) != key);
        let removed = self.history.len() != before;
        if removed {
            self.push_tombstone(&key, now_ms);
        }
        removed
    }

    /// Clear the whole history AND stamp the watermark
    /// (AUDIT 0.41.3, Front A F3): entries visited at or
    /// before `now_ms` can never be resurrected by a
    /// merge with a pre-clear snapshot.
    pub fn clear_history(&mut self, now_ms: u64) {
        self.history.clear();
        self.history_cleared_at = now_ms;
    }

    /// The most recent `n` history entries (the store is
    /// already most-recent-first).
    pub fn recent(&self, n: usize) -> &[HistoryEntry] {
        &self.history[..n.min(self.history.len())]
    }

    /// History entries matching `query` (empty = all),
    /// most-recent-first.
    pub fn search_history(&self, query: &str) -> Vec<HistoryEntry> {
        let q = query.trim().to_lowercase();
        self.history
            .iter()
            .filter(|e| matches_query(&q, &e.title, &e.url))
            .cloned()
            .collect()
    }

    // -- suggestions -------------------------------------------------

    /// Address-bar suggestions for `query` at `now_ms`:
    /// bookmark matches first (most-recently-added
    /// first), then history matches by frecency (see
    /// [`frecency_score`] for the formula and the total
    /// tie-breaking order), with a URL that appears in
    /// both shown ONCE — as the bookmark. At most `limit`
    /// rows. The raw typed input's "go" row is NOT here:
    /// it is not data, the chrome appends it itself.
    pub fn suggestions(&self, query: &str, now_ms: u64, limit: usize) -> Vec<Suggestion> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<Suggestion> = Vec::new();
        for b in &self.bookmarks {
            if out.len() >= limit {
                return out;
            }
            if matches_query(&q, &b.title, &b.url) {
                out.push(Suggestion {
                    url: b.url.clone(),
                    title: b.title.clone(),
                    kind: SuggestionKind::Bookmark,
                });
            }
        }
        // PERF (0.41.2): the bookmark/history dedupe used to
        // recompute BOTH sides' canonical keys for every
        // (bookmark × history-entry) comparison — O(B·H) key
        // computations, each a URL parse plus allocations.
        // The bookmark keys are now computed once into a set
        // and each history entry's key once: membership in
        // the set is exactly the old `any(==)` verdict. The
        // empty-set short-circuit preserves the old cost
        // profile when no bookmark matched (the old `any()`
        // over an empty `out` computed no keys at all).
        let bookmark_keys: std::collections::HashSet<String> =
            out.iter().map(|s| canonical_nav_key(&s.url)).collect();
        let mut hist: Vec<&HistoryEntry> = self
            .history
            .iter()
            .filter(|e| matches_query(&q, &e.title, &e.url))
            .filter(|e| {
                bookmark_keys.is_empty() || !bookmark_keys.contains(&canonical_nav_key(&e.url))
            })
            .collect();
        hist.sort_by(|a, b| {
            frecency_score(b, now_ms)
                .cmp(&frecency_score(a, now_ms))
                .then_with(|| b.last_visited.cmp(&a.last_visited))
                .then_with(|| a.url.cmp(&b.url))
        });
        for e in hist {
            if out.len() >= limit {
                break;
            }
            out.push(Suggestion {
                url: e.url.clone(),
                title: e.title.clone(),
                kind: SuggestionKind::History,
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Persistence (the state machinery's discipline, for this file)
// ---------------------------------------------------------------------------

/// The nav file's path inside the Pico home.
pub fn nav_path() -> PathBuf {
    crate::core::state::pico_dir().join("nav.json")
}

/// Outcome of loading the nav store — the state loader's
/// three shapes, for the same startup log discipline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavLoadOutcome {
    Loaded(Box<NavStore>),
    Missing,
    Corrupt(String),
}

/// Parse a nav file's text: strict first (the shape
/// Pico writes), then the tolerant per-entry parse
/// (AUDIT 0.41.1, Front B F3 — through 0.41.0 the
/// all-or-nothing struct parse let ONE malformed entry
/// poison the whole file: every bookmark and every
/// history entry gone for one bad row).
fn parse_nav(text: &str) -> Option<NavStore> {
    if let Ok(store) = serde_json::from_str::<NavStore>(text) {
        return Some(store);
    }
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    let mut store = NavStore::default();
    if let Some(v) = obj.get("version").and_then(|v| v.as_u64()) {
        store.version = v as u32;
    }
    if let Some(arr) = obj.get("bookmarks").and_then(|v| v.as_array()) {
        store.bookmarks = arr
            .iter()
            .filter_map(|e| serde_json::from_value::<Bookmark>(e.clone()).ok())
            .collect();
    }
    if let Some(arr) = obj.get("history").and_then(|v| v.as_array()) {
        store.history = arr
            .iter()
            .filter_map(|e| serde_json::from_value::<HistoryEntry>(e.clone()).ok())
            .collect();
    }
    // AUDIT 0.41.3 (F3): the deletion record rides the
    // tolerant parse too — a file whose strict parse
    // failed on one entry must not lose its tombstones
    // with it.
    if let Some(v) = obj.get("history_cleared_at").and_then(|v| v.as_u64()) {
        store.history_cleared_at = v;
    }
    if let Some(arr) = obj.get("tombstones").and_then(|v| v.as_array()) {
        store.tombstones = arr
            .iter()
            .filter_map(|e| serde_json::from_value::<Tombstone>(e.clone()).ok())
            .collect();
    }
    Some(store)
}

/// Move an unusable nav file aside (AUDIT 0.41.1,
/// Front B F2): through 0.41.0 a corrupt file was left
/// in place and the in-memory store started empty — so
/// the FIRST save of the session (a window close saves
/// unconditionally) silently overwrote the only copy of
/// the user's record with near-empty defaults. The
/// corrupt bytes now move to `nav.json.corrupt` at load
/// time, before any save can happen; a previous
/// quarantine is replaced (the newest corruption is the
/// interesting one). Best-effort: a failed rename leaves
/// the file where it was and changes nothing else.
fn quarantine_corrupt(path: &Path) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "nav.json".to_string());
    let target = path.with_file_name(format!("{name}.corrupt"));
    let _ = std::fs::remove_file(&target);
    let _ = std::fs::rename(path, &target);
}

/// The save temps sitting next to the nav file
/// (`nav.json.<pid>.tmp` — see `save_to`), newest first.
/// AUDIT 0.41.1 (Front B F10): the state loader has
/// recovered interrupted saves from temps since v28;
/// the nav loader never looked, so a crash between the
/// temp's fsync and its rename lost the newest store
/// even though a complete copy sat beside the file.
fn temp_siblings(path: &Path) -> Vec<PathBuf> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "nav.json".to_string());
    let prefix = format!("{name}.");
    let mut temps: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.starts_with(&prefix) && fname.ends_with(".tmp") {
                let mtime = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                temps.push((mtime, p));
            }
        }
    }
    temps.sort_by_key(|t| std::cmp::Reverse(t.0));
    temps.into_iter().map(|(_, p)| p).collect()
}

fn temp_age_ms(mtime: std::time::SystemTime, now_ms: u64) -> u64 {
    let mtime_ms = mtime
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    now_ms.saturating_sub(mtime_ms)
}

/// Delete save temps older than an hour (a live save's
/// temp is seconds old; anything older is a crashed
/// save's leftover). Mirrors the state loader's rule.
fn clean_stale_temps(path: &Path, now_ms: u64) {
    const STALE_TEMP_MS: u64 = 3_600_000;
    for temp in temp_siblings(path) {
        let stale = std::fs::metadata(&temp)
            .and_then(|m| m.modified())
            .map(|t| temp_age_ms(t, now_ms) > STALE_TEMP_MS)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(&temp);
        }
    }
}

/// Promote the newest COMPLETE save temp over a missing
/// or corrupt main file (0.41.1, Front B F10). Returns
/// the recovered store; the temp is renamed onto the
/// real path (best-effort — the store is returned either
/// way, and the next save rewrites the file properly).
fn recover_from_temp(path: &Path, now_ms: u64) -> Option<NavStore> {
    for temp in temp_siblings(path) {
        let Ok(text) = std::fs::read_to_string(&temp) else {
            continue;
        };
        if let Some(store) = parse_nav(&text) {
            let _ = std::fs::rename(&temp, path);
            return Some(store.sanitized(now_ms));
        }
    }
    None
}

pub fn load_from_checked(path: &Path, now_ms: u64) -> NavLoadOutcome {
    let main = load_main(path, now_ms);
    match main {
        NavLoadOutcome::Loaded(_) => {
            clean_stale_temps(path, now_ms);
            main
        }
        missing_or_corrupt => match recover_from_temp(path, now_ms) {
            Some(recovered) => NavLoadOutcome::Loaded(Box::new(recovered)),
            None => missing_or_corrupt,
        },
    }
}

fn load_main(path: &Path, now_ms: u64) -> NavLoadOutcome {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_NAV_BYTES => {
            quarantine_corrupt(path);
            return NavLoadOutcome::Corrupt(format!(
                "nav file implausibly large ({} bytes)",
                meta.len()
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return NavLoadOutcome::Missing;
        }
        Err(e) => {
            return NavLoadOutcome::Corrupt(format!("unreadable: {e}"));
        }
    }
    match std::fs::read_to_string(path) {
        Ok(text) => match parse_nav(&text) {
            Some(store) => NavLoadOutcome::Loaded(Box::new(store.sanitized(now_ms))),
            None => {
                quarantine_corrupt(path);
                NavLoadOutcome::Corrupt("invalid JSON".to_string())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => NavLoadOutcome::Missing,
        Err(e) => NavLoadOutcome::Corrupt(format!("unreadable: {e}")),
    }
}

/// Merge two stores that diverged from a shared base
/// (AUDIT 0.41.1, Front B F9): bookmarks union by
/// canonical key (ours wins a conflict — it is the live
/// session's view); history union by canonical key with
/// `last_visited` = max and `visits` = MAX, never a sum
/// — both stores descend from the same base, so summing
/// would double-count the shared history on every save.
/// The title follows the newer `last_visited`.
pub fn merge_nav_stores(ours: &NavStore, disk: &NavStore) -> NavStore {
    // PERF (0.41.2): same O(N²)-key trap as `sanitized` —
    // the membership/find scans recomputed the canonical
    // key of every entry already in `out` for every disk
    // entry. Keys are now computed once per entry: a set
    // for bookmarks (updated as disk extras land, exactly
    // like the old scan over the growing `out`) and a
    // first-index map for history (the old `find` returns
    // the FIRST match; the map is built first-wins and
    // extended as disk extras are pushed). Pinned by the
    // differential tests in `tests/perf_equiv.rs`.
    let mut out = ours.clone();
    let mut bookmark_keys: std::collections::HashSet<String> = out
        .bookmarks
        .iter()
        .map(|x| canonical_nav_key(&x.url))
        .collect();
    for b in &disk.bookmarks {
        if bookmark_keys.insert(canonical_nav_key(&b.url)) {
            out.bookmarks.push(b.clone());
        }
    }
    let mut history_index: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, x) in out.history.iter().enumerate() {
        history_index.entry(canonical_nav_key(&x.url)).or_insert(i);
    }
    for e in &disk.history {
        let key = canonical_nav_key(&e.url);
        match history_index.get(&key) {
            Some(&idx) => {
                let existing = &mut out.history[idx];
                if e.last_visited > existing.last_visited {
                    existing.last_visited = e.last_visited;
                    existing.title = e.title.clone();
                }
                existing.visits = existing.visits.max(e.visits);
            }
            None => {
                history_index.insert(key, out.history.len());
                out.history.push(e.clone());
            }
        }
    }
    // AUDIT 0.41.3 (Front A F3): the union above is
    // deletion-blind — a store that still holds an entry
    // another process cleared or removed would resurrect
    // it here. The merged deletion record is the max
    // clear watermark plus the union of both sides'
    // tombstones; the merged entries are then filtered
    // by it: a history entry survives iff it was visited
    // after the watermark AND after its key's tombstone
    // (when one exists); a bookmark survives iff it was
    // added after its key's tombstone. Causality within
    // the SAME millisecond is not representable in these
    // stamps — a delete and a re-visit stamped with the
    // same `now_ms` resolve to the deletion (the strict
    // `>` survival rule), which is the safe direction
    // for a privacy record.
    out.history_cleared_at = ours.history_cleared_at.max(disk.history_cleared_at);
    let mut tombstones = out.tombstones.clone();
    tombstones.extend(disk.tombstones.iter().cloned());
    prune_tombstones(&mut tombstones);
    out.tombstones = tombstones;
    let tombstone_at: std::collections::HashMap<&str, u64> = out
        .tombstones
        .iter()
        .map(|t| (t.key.as_str(), t.at_ms))
        .collect();
    let cleared_at = out.history_cleared_at;
    out.history.retain(|e| {
        e.last_visited > cleared_at
            && tombstone_at
                .get(canonical_nav_key(&e.url).as_str())
                .is_none_or(|&at| e.last_visited > at)
    });
    out.bookmarks.retain(|b| {
        tombstone_at
            .get(canonical_nav_key(&b.url).as_str())
            .is_none_or(|&at| b.added_at > at)
    });
    out
}

/// Save with the two-process merge (AUDIT 0.41.1,
/// Front B F9): Move to New Window spawns a second
/// Pico, and through 0.41.0 both processes saved their
/// startup snapshot over the same file — whichever
/// saved last silently erased the other's whole
/// session of bookmarks and history. The save now
/// re-reads the on-disk store under a lock file
/// (`nav.lock`, the downloads SaveLock discipline) and
/// merges before writing, so neither process's records
/// are lost. (The in-memory store is not updated with
/// the merged result; the other process's entries
/// appear at next launch, and every later save merges
/// again, so nothing is ever overwritten away.)
pub fn save_merged(path: &Path, ours: &NavStore, now_ms: u64) -> std::io::Result<()> {
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let _lock = crate::core::download::SaveLock::acquire_named(&dir, "nav.lock");
    let merged = match load_from_checked(path, now_ms) {
        NavLoadOutcome::Loaded(disk) => merge_nav_stores(ours, &disk),
        _ => ours.clone(),
    };
    save_to(path, &merged.sanitized(now_ms))
}

/// Write the store as JSON via a per-process temp
/// sibling, an fsync, and a rename — the state writer's
/// exact discipline (`core::state::save_to`): a crash
/// mid-write can never leave a torn file at the real
/// path, and a failed save removes its temp so nothing
/// stale can be promoted.
pub fn save_to(path: &Path, store: &NavStore) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "nav.json".to_string());
    let tmp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(store)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        // The state writer's retry-poisoning rule: a
        // read-only destination would fail the rename on
        // Windows forever; clear it first, best-effort.
        clear_readonly(path);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

pub fn save(store: &NavStore) -> std::io::Result<()> {
    save_to(&nav_path(), store)
}

/// Best-effort read-only clearing, mirroring the state
/// module's helper (kept private there; the discipline is
/// the point, so it is replicated rather than shared
/// across the persistence boundary).
#[allow(clippy::permissions_set_readonly_false)]
fn clear_readonly(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().readonly() {
            let mut perms = meta.permissions();
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_730_000_000_000;

    fn store_with_visits() -> NavStore {
        let mut s = NavStore::default();
        s.record_visit("https://example.com/a", "Alpha", NOW - 1000);
        s.record_visit("https://example.org/b", "Beta", NOW - 2000);
        s
    }

    #[test]
    fn recordable_rules() {
        assert!(is_recordable_url("https://example.com/page"));
        assert!(is_recordable_url("http://localhost:5173/app"));
        assert!(is_recordable_url("http://127.0.0.1:49152/index.html"));
        assert!(is_recordable_url("file:///C:/dev/page.html"));
        // NEVER internal pages, either spelling.
        assert!(!is_recordable_url("pico://start"));
        assert!(!is_recordable_url("pico://downloads"));
        assert!(!is_recordable_url("pico://history"));
        assert!(!is_recordable_url("https://pico.start/"));
        assert!(!is_recordable_url("https://pico.downloads/"));
        // Nothing else either.
        assert!(!is_recordable_url("about:blank"));
        assert!(!is_recordable_url("data:text/html,hi"));
        assert!(!is_recordable_url("javascript:alert(1)"));
        assert!(!is_recordable_url(""));
        // Credentialed URLs never persist.
        assert!(!is_recordable_url("https://user:pass@example.com/"));
        // Controls / whitespace / over-long.
        assert!(!is_recordable_url("https://example.com/a b"));
        assert!(!is_recordable_url(&format!(
            "https://example.com/{}",
            "a".repeat(MAX_NAV_URL_CHARS)
        )));
    }

    #[test]
    fn internal_pages_are_never_recorded() {
        let mut s = NavStore::default();
        for u in ["pico://start", "pico://downloads", "pico://history"] {
            assert!(!s.record_visit(u, "Internal", NOW), "{u}");
            assert!(!s.toggle_bookmark(u, "Internal", NOW), "{u}");
        }
        assert!(s.history.is_empty() && s.bookmarks.is_empty());
    }

    #[test]
    fn revisit_updates_in_place_no_duplicates() {
        let mut s = store_with_visits();
        assert!(s.record_visit("https://example.com/a", "Alpha 2", NOW));
        assert_eq!(s.history.len(), 2, "no duplicate row");
        let e = &s.history[0];
        assert_eq!(e.url, "https://example.com/a");
        assert_eq!(e.title, "Alpha 2");
        assert_eq!(e.visits, 2);
        assert_eq!(e.last_visited, NOW);
        // An empty re-title does not erase a known title.
        assert!(s.record_visit("https://example.com/a", "", NOW + 1));
        assert_eq!(s.history[0].title, "Alpha 2");
        assert_eq!(s.history[0].visits, 3);
    }

    #[test]
    fn title_falls_back_to_host() {
        let mut s = NavStore::default();
        s.record_visit("https://example.com/deep/page", "", NOW);
        assert_eq!(s.history[0].title, "example.com");
        s.record_visit("http://localhost:5173/", "", NOW);
        assert_eq!(s.history[0].title, "localhost");
    }

    #[test]
    fn cap_prunes_least_recently_visited() {
        let mut s = NavStore::default();
        for i in 0..(MAX_HISTORY + 5) {
            s.record_visit(&format!("https://example.com/{i}"), "T", NOW + i as u64);
        }
        assert_eq!(s.history.len(), MAX_HISTORY);
        // The five oldest are gone; the newest is first.
        assert_eq!(
            s.history[0].url,
            format!("https://example.com/{}", MAX_HISTORY + 4)
        );
        assert!(!s.history.iter().any(|e| e.url == "https://example.com/0"));
        assert!(!s.history.iter().any(|e| e.url == "https://example.com/4"));
        assert!(s.history.iter().any(|e| e.url == "https://example.com/5"));
    }

    #[test]
    fn bookmark_toggle_roundtrip() {
        let mut s = NavStore::default();
        assert!(s.toggle_bookmark("https://example.com/", "Ex", NOW));
        assert!(s.is_bookmarked("https://example.com/"));
        assert!(!s.toggle_bookmark("https://example.com/", "Ex", NOW));
        assert!(!s.is_bookmarked("https://example.com/"));
        assert!(s.bookmarks.is_empty());
        // Remove of an absent bookmark is a clean false.
        assert!(!s.remove_bookmark("https://example.com/", NOW));
    }

    #[test]
    fn remove_and_clear_history() {
        let mut s = store_with_visits();
        assert!(s.remove_history("https://example.com/a", NOW));
        assert!(!s.remove_history("https://example.com/a", NOW));
        assert_eq!(s.history.len(), 1);
        s.clear_history(NOW);
        assert!(s.history.is_empty());
        assert_eq!(s.history_cleared_at, NOW, "the clear stamps its watermark");
    }

    #[test]
    fn frecency_formula_and_determinism() {
        let mut s = NavStore::default();
        // 50 visits a year ago: 50 * 10 = 500.
        for _ in 0..50 {
            s.record_visit("https://old.example/", "Old", NOW - 400 * 86_400_000);
        }
        // Fix the last_visited the loop stamped: record_visit
        // uses the injected now each time, all identical here.
        // 10 visits within the hour: 10 * 100 = 1000 — wins.
        for _ in 0..10 {
            s.record_visit("https://hot.example/", "Hot", NOW - 60_000);
        }
        // 2 visits within the hour (200) vs 4 visits last
        // month (4 * 20 = 80): the recent pair wins.
        for _ in 0..2 {
            s.record_visit("https://fresh.example/", "Fresh", NOW - 60_000);
        }
        for _ in 0..4 {
            s.record_visit("https://month.example/", "Month", NOW - 20 * 86_400_000);
        }
        let sug = s.suggestions("", NOW, 10);
        let urls: Vec<&str> = sug.iter().map(|x| x.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://hot.example/",
                "https://old.example/",
                "https://fresh.example/",
                "https://month.example/"
            ]
        );
        // Deterministic: the same call twice agrees exactly.
        assert_eq!(s.suggestions("", NOW, 10), sug);
        // Score function itself, at the bucket edges.
        let e = |visits, age| HistoryEntry {
            url: "https://x.example/".into(),
            title: "X".into(),
            last_visited: NOW - age,
            visits,
        };
        assert_eq!(frecency_score(&e(3, 0), NOW), 300);
        assert_eq!(frecency_score(&e(3, 3_600_000), NOW), 300);
        assert_eq!(frecency_score(&e(3, 3_600_001), NOW), 210);
        assert_eq!(frecency_score(&e(3, 86_400_000), NOW), 210);
        assert_eq!(frecency_score(&e(3, 86_400_001), NOW), 120);
    }

    #[test]
    fn suggestions_bookmarks_first_and_deduped() {
        let mut s = NavStore::default();
        for _ in 0..20 {
            s.record_visit("https://both.example/", "Both", NOW - 1000);
        }
        s.record_visit("https://hist.example/", "Hist Only", NOW - 1000);
        s.toggle_bookmark("https://both.example/", "Both (saved)", NOW);
        s.toggle_bookmark("https://mark.example/", "Mark Only", NOW);
        let sug = s.suggestions("example", NOW, 10);
        assert_eq!(sug.len(), 3);
        assert_eq!(sug[0].kind, SuggestionKind::Bookmark);
        assert_eq!(sug[1].kind, SuggestionKind::Bookmark);
        assert_eq!(sug[2].kind, SuggestionKind::History);
        assert_eq!(
            sug.iter()
                .filter(|x| x.url == "https://both.example/")
                .count(),
            1,
            "a bookmarked history URL appears once, as the bookmark"
        );
        // Limit applies across the whole list.
        assert_eq!(s.suggestions("example", NOW, 2).len(), 2);
        // No match, no rows.
        assert!(s.suggestions("zzz-no-match", NOW, 10).is_empty());
    }

    #[test]
    fn search_history_filters_case_insensitively() {
        let s = store_with_visits();
        assert_eq!(s.search_history("ALPHA").len(), 1);
        assert_eq!(s.search_history("example.org").len(), 1);
        assert_eq!(s.search_history("").len(), 2);
        assert!(s.search_history("nope").is_empty());
    }

    #[test]
    fn sanitized_drops_internal_and_merges_dupes() {
        let raw = NavStore {
            version: 99,
            // AUDIT 0.41.3: the F3 fields ride along at
            // their defaults (never cleared, no deletions).
            history_cleared_at: 0,
            tombstones: Vec::new(),
            bookmarks: vec![
                Bookmark {
                    url: "pico://start".into(),
                    title: "X".into(),
                    added_at: 1,
                },
                Bookmark {
                    url: "https://a.example/".into(),
                    title: "".into(),
                    added_at: 2,
                },
                Bookmark {
                    url: "https://a.example/".into(),
                    title: "Dupe".into(),
                    added_at: 3,
                },
            ],
            history: vec![
                HistoryEntry {
                    url: "https://a.example/".into(),
                    title: "A".into(),
                    last_visited: 10,
                    visits: 2,
                },
                HistoryEntry {
                    url: "https://a.example/".into(),
                    title: "A2".into(),
                    last_visited: 20,
                    visits: 3,
                },
                HistoryEntry {
                    url: "pico://history".into(),
                    title: "H".into(),
                    last_visited: 30,
                    visits: 1,
                },
            ],
        };
        let s = raw.sanitized(NOW);
        assert_eq!(s.version, NAV_SCHEMA_VERSION);
        assert_eq!(s.bookmarks.len(), 1);
        assert_eq!(s.bookmarks[0].title, "a.example", "host fallback title");
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.history[0].visits, 5, "duplicate visits merge");
        assert_eq!(s.history[0].last_visited, 20);
        assert_eq!(s.history[0].title, "A2", "latest title wins");
    }

    #[test]
    fn save_load_roundtrip_and_corrupt_is_safe() {
        let dir = std::env::temp_dir().join(format!("pico-navstore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nav.json");
        assert_eq!(load_from_checked(&path, NOW), NavLoadOutcome::Missing);
        let mut s = store_with_visits();
        s.toggle_bookmark("https://example.com/a", "Alpha", NOW);
        save_to(&path, &s).unwrap();
        match load_from_checked(&path, NOW) {
            // Loads pass through sanitized() (history
            // re-sorts by last_visited; the fixture records
            // out of timestamp order on purpose).
            NavLoadOutcome::Loaded(loaded) => assert_eq!(*loaded, s.sanitized(NOW)),
            other => panic!("expected Loaded, got {other:?}"),
        }
        std::fs::write(&path, "{not json").unwrap();
        assert!(matches!(
            load_from_checked(&path, NOW),
            NavLoadOutcome::Corrupt(_)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -------------------------------------------------
    // AUDIT 0.41.3 — PendingVisit id-keyed queue (F2)
    // -------------------------------------------------

    #[test]
    fn pending_verdict_pairs_by_id_not_position() {
        // Supersede interleave: B commits, C commits,
        // then B's verdict arrives — it must decide B
        // even though C's candidate queued after it.
        let mut p = PendingVisit::new();
        assert_eq!(p.note_candidate(1, "https://a.example/"), None);
        assert_eq!(p.note_candidate(2, "https://b.example/"), None);
        assert_eq!(p.note_candidate(3, "https://c.example/"), None);
        assert_eq!(
            p.note_verdict(2, true, 200),
            VerdictOutcome::Record("https://b.example/".to_string())
        );
        // C is still pending, decided by its own verdict.
        assert_eq!(
            p.note_verdict(3, false, 0),
            VerdictOutcome::Drop("https://c.example/".to_string())
        );
        assert_eq!(
            p.note_verdict(1, false, 404),
            VerdictOutcome::Record("https://a.example/".to_string()),
            "an HTTP error page is a visit (verdict_records_visit)"
        );
    }

    #[test]
    fn pending_verdict_for_a_middle_entry_and_repeat_id_replaces() {
        let mut p = PendingVisit::new();
        p.note_candidate(1, "https://a.example/");
        p.note_candidate(2, "https://b.example/");
        p.note_candidate(3, "https://c.example/");
        // A repeat nav_id replaces its URL in place.
        assert_eq!(p.note_candidate(2, "https://b2.example/"), None);
        assert_eq!(
            p.note_verdict(2, true, 200),
            VerdictOutcome::Record("https://b2.example/".to_string())
        );
        // FIFO order of the rest is untouched.
        assert_eq!(
            p.flush_all(),
            vec![
                "https://a.example/".to_string(),
                "https://c.example/".to_string()
            ]
        );
    }

    #[test]
    fn pending_late_verdict_after_flush_is_no_match() {
        let mut p = PendingVisit::new();
        p.note_candidate(7, "https://a.example/");
        assert_eq!(p.flush_all(), vec!["https://a.example/".to_string()]);
        assert_eq!(p.note_verdict(7, true, 200), VerdictOutcome::NoMatch);
        assert_eq!(p.note_verdict(99, true, 200), VerdictOutcome::NoMatch);
    }

    #[test]
    fn pending_eviction_returns_the_oldest() {
        let mut p = PendingVisit::new();
        for id in 1..=(MAX_PENDING_VISITS as u64) {
            assert_eq!(
                p.note_candidate(id, &format!("https://h{id}.example/")),
                None
            );
        }
        assert_eq!(
            p.note_candidate(99, "https://new.example/"),
            Some("https://h1.example/".to_string()),
            "the 9th candidate evicts the oldest for recording"
        );
        // The evicted id's late verdict decides nothing.
        assert_eq!(p.note_verdict(1, true, 200), VerdictOutcome::NoMatch);
        assert_eq!(p.flush_all().len(), MAX_PENDING_VISITS);
    }

    // -------------------------------------------------
    // AUDIT 0.41.3 — deletion tombstones (F3), unit half
    // -------------------------------------------------

    #[test]
    fn tombstones_record_and_fresh_actions_supersede() {
        let mut s = NavStore::default();
        s.record_visit("https://a.example/", "A", NOW - 3000);
        s.record_visit("https://b.example/", "B", NOW - 2000);
        assert!(s.remove_history("https://a.example/", NOW - 1000));
        assert_eq!(s.tombstones.len(), 1);
        assert_eq!(s.tombstones[0].key, canonical_nav_key("https://a.example/"));
        assert_eq!(s.tombstones[0].at_ms, NOW - 1000);
        // A remove that matches nothing pushes nothing.
        assert!(!s.remove_history("https://absent.example/", NOW));
        assert_eq!(s.tombstones.len(), 1);
        // A fresh visit supersedes the deletion.
        assert!(s.record_visit("https://a.example/", "A2", NOW));
        assert!(s.tombstones.is_empty(), "the re-visit drops its tombstone");
        // Bookmarks: same discipline through toggle/remove.
        assert!(s.toggle_bookmark("https://b.example/", "B", NOW - 500));
        assert!(s.remove_bookmark("https://b.example/", NOW - 400));
        assert_eq!(s.tombstones.len(), 1);
        assert!(s.toggle_bookmark("https://b.example/", "B", NOW));
        assert!(
            s.tombstones.is_empty(),
            "re-bookmarking drops its tombstone"
        );
    }

    #[test]
    fn sanitized_applies_the_watermark_and_tombstones() {
        let mut s = NavStore {
            history_cleared_at: NOW - 1000,
            ..Default::default()
        };
        s.history.push(HistoryEntry {
            url: "https://old.example/".into(),
            title: "Old".into(),
            last_visited: NOW - 2000,
            visits: 1,
        });
        s.history.push(HistoryEntry {
            url: "https://new.example/".into(),
            title: "New".into(),
            last_visited: NOW - 500,
            visits: 1,
        });
        s.tombstones.push(Tombstone {
            key: canonical_nav_key("https://new.example/"),
            at_ms: NOW - 400,
        });
        s.bookmarks.push(Bookmark {
            url: "https://new.example/".into(),
            title: "New".into(),
            added_at: NOW - 500,
        });
        let s = s.sanitized(NOW);
        assert!(
            s.history.is_empty(),
            "watermark kills the old entry, tombstone the newer one: {:?}",
            s.history
        );
        assert!(s.bookmarks.is_empty(), "added before its tombstone");
    }

    #[test]
    fn note_title_updates_the_entry() {
        let mut s = NavStore::default();
        s.record_visit("https://example.com/", "", NOW);
        assert_eq!(s.history[0].title, "example.com");
        assert!(s.note_title("https://example.com/", "Real Title"));
        assert_eq!(s.history[0].title, "Real Title");
        assert!(!s.note_title("https://example.com/", "Real Title"));
        assert!(!s.note_title("https://absent.example/", "Nope"));
    }
}
