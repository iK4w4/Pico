//! The download manager (0.32): owns every download task and
//! the threads that move its bytes.
//!
//! Shape: [`DownloadManager`] is a cheap `Clone` handle over
//! shared state (the app keeps one; worker threads hold clones).
//! All task state lives behind one `Mutex`; no lock is ever held
//! across a network read or a sleep, so the UI thread's calls
//! (`views`, `pause`, …) never wait on the network.
//!
//! - **Scheduler:** at most the configured `max_concurrent`
//!   tasks (0.33; the core's `MAX_ACTIVE_TASKS` is only the
//!   construction fallback) are Downloading at once. After
//!   every mutation, queued tasks are started oldest-first
//!   while slots remain.
//! - **One supervisor thread per running task:** it probes
//!   (`crate::winhttp::probe`), applies the core's
//!   `resume_decision` against the persisted state, preallocates
//!   the `.part` file, then spawns one worker thread per
//!   connection; workers claim segments from the shared core
//!   `Planner` (with stealing) until it is exhausted.
//! - **Worker file writes:** each worker opens its OWN handle
//!   to the `.part` file (write, never append), seeks to the
//!   exact chunk offset, writes, and only then reports
//!   `planner.progress`. Progress therefore never claims bytes
//!   that are not on disk — a state/file tear can only
//!   re-download, never splice.
//! - **Events:** the manager emits exactly one kind of UI
//!   notification — "downloads changed" — through the notifier
//!   closure it was built with (see [`DownloadManager::new`]
//!   for why it is a closure, and `new_with_proxy` for the
//!   `EventLoopProxy` form). `version()` bumps in step with
//!   every notification, and progress notifications are
//!   throttled to at most one per 250 ms per task.
//! - **Logging** (via `crate::log`): added / started / paused /
//!   resumed / completed / failed / cancelled, host-only — a
//!   download URL is reduced to its host by the core's
//!   `url_host` before it can reach a log line, cookies are
//!   never logged, and a completed task's name is logged only
//!   because by then it is a local file name the user owns.
//!
//! CONNECTION-COUNT RULE (the implemented semantics): a task's
//! connection count may be changed at any time. For a queued
//! task it is the plan used at start. For a running or paused
//! task it re-splits only the not-yet-started byte ranges
//! (`Planner::replan_pending`); in-flight segments are never
//! resized except by stealing. If every remaining byte is
//! already in flight, the new count takes effect at the next
//! steal/resume.
//!
//! Zero `unwrap`/`expect`/`panic!` in this module: mutex
//! poisoning recovers the guard (the `StatsBook::lock`
//! pattern), and every failure becomes a task error or a
//! logged line.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pico_core::core::download::{
    classify_failure, cookie_header, dedupe_file_name, delete_task_file, eta_seconds, eta_text,
    failure_hint, file_name_from_url, is_downloadable_url, load_tasks, percent, plan_segments,
    refresh_decision, resume_decision, retry_policy, sanitize_cookies, sanitize_file_name,
    save_task, speed_text, strip_url_userinfo, url_host, Claim, CookiePair, FailureClass,
    GenerationGate, Planner, RefreshDecision, ResumeDecision, RetryPolicy, Segment, SpeedMeter,
    TaskState, TaskStatus, TokenBucket, AUTO_RETRY_BACKOFF_MS, AUTO_RETRY_MAX, DEFAULT_CONNECTIONS,
    MAX_ACTIVE_TASKS, MAX_CONNECTIONS, MIN_CONNECTIONS, TASK_FILE_VERSION,
};
use pico_core::core::sha256::Sha256;
use pico_core::core::stats::humanize_bytes;
use tao::event_loop::EventLoopProxy;

use crate::winhttp::{self, FetchError};

/// The "downloads changed" notification: called (with no
/// arguments) on every visible state change. The app builds it
/// from its `EventLoopProxy` — see [`DownloadManager::new`].
pub type NotifyFn = Arc<dyn Fn() + Send + Sync>;

/// Segment fetch attempts before the task fails (the brief's
/// "retry the segment up to 3 attempts").
const MAX_ATTEMPTS: u32 = 3;
/// First retry backoff; each later retry doubles it
/// (500 ms × 2^n), slept in 100 ms slices so pause/cancel stay
/// responsive throughout.
const RETRY_BASE_MS: u64 = 500;
const RETRY_SLICE_MS: u64 = 100;
/// Supervisor progress-tick cadence; also the SpeedMeter
/// sampling cadence. Progress notifications are throttled to
/// this same 250 ms (the brief's per-task progress cap).
const TICK_MS: u64 = 250;
/// Sink sentinel: a ranged response body ran past the end of
/// its segment — the signature of a 200 answer to a ranged
/// request (the server sent the whole file). The worker maps
/// it to the supervisor's restart-as-single-stream path.
const RANGE_OVERFLOW: &str = "pico-range-overflow";
/// Sink sentinel: the segment was STOLEN from mid-flight and
/// shrunk — this worker's remaining range now belongs to the
/// thief. The worker stops writing at the segment's live end
/// and retires from the segment without an error (AUDIT
/// 0.32.1, P1-10: through 0.32 it kept writing to the OLD
/// end, through the stolen half, racing the thief's writes).
const SEGMENT_SHRUNK: &str = "pico-segment-shrunk";
/// Sink sentinel (AUDIT 0.32.2, Fix 2): the worker's run
/// generation went stale mid-flight — a pause/cancel (or a
/// newer run) retired this run at the manager, which already
/// holds the task's final word. The worker stops writing at
/// once and retires quietly: the bytes it wrote past the
/// manager's snapshot are simply not in the persisted map
/// and are re-fetched on resume.
const STALE_RUN: &str = "pico-stale-run";
/// Sink sentinel (0.33): the shared speed-limit bucket made
/// this worker wait, and while it waited its run went stale
/// (or the control stopped). Treated EXACTLY like
/// [`STALE_RUN`] by `fetch_segment`: release the segment and
/// retire quietly — the manager's snapshot is the final word.
const THROTTLE_STOP: &str = "pico-throttle-stop";
/// Task-record caps (AUDIT 0.32.1, P2-2): through 0.32 any
/// web page could mint download tasks without limit — three
/// writers fill the disk while thousands of state files and
/// queued records pile up. Unfinished tasks (queued,
/// downloading, paused, failed — everything holding or
/// awaiting disk) cap at 100; total records at 1,000.
const MAX_UNFINISHED_TASKS: usize = 100;
const MAX_TASK_RECORDS: usize = 1000;
/// Unknown-total growth guard (AUDIT 0.32.1, P1-2): the sink
/// re-checks the volume's free space every 32 MiB and fails
/// the task before the reserve is eaten.
const DISK_CHECK_EVERY: u64 = 32 * 1024 * 1024;
const DISK_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Lock with poison recovery (a panicked worker must not wedge
/// the manager for the rest of the session).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn clamp_connections(n: u32) -> u32 {
    n.clamp(MIN_CONNECTIONS, MAX_CONNECTIONS)
}

/// Sanitize a caller-supplied referer (0.33): userinfo
/// stripped, http/https only, at most 2048 chars — anything
/// else becomes `None`. Mirrors the core sanitizer's rule
/// for the persisted field, applied at the manager's door.
fn sanitize_referer(referer: Option<String>) -> Option<String> {
    let raw = referer?;
    let stripped = strip_url_userinfo(&raw);
    if stripped.chars().count() > 2048 || !is_downloadable_url(&stripped) {
        return None;
    }
    Some(stripped)
}

/// The per-task stop signal workers poll at chunk boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Run,
    Pause,
    Cancel,
}

/// Terminal facts the workers report back to their supervisor.
#[derive(Default)]
struct WorkerReport {
    /// A segment exhausted its retries (short message).
    failed: Option<String>,
    /// A ranged request was answered 200 / with an oversized
    /// body, or a segment response carried a validator that
    /// contradicts the task's: the file changed or ranges died.
    range_died: bool,
}

impl WorkerReport {
    fn is_terminal(&self) -> bool {
        self.failed.is_some() || self.range_died
    }
}

/// Everything one worker thread needs, shared immutably.
struct WorkerCtx {
    url: String,
    cookies: Arc<Vec<CookiePair>>,
    /// The page that handed Pico this download (0.33), sent
    /// as the `Referer` header on every segment request.
    referer: Option<String>,
    /// The manager-wide speed-limit bucket (0.33): every
    /// worker of every task draws from this one bucket.
    bucket: Arc<Mutex<TokenBucket>>,
    /// If-Range validator (etag preferred, else Last-Modified).
    validator: Option<String>,
    /// Which header `validator` came from — segment responses
    /// are cross-checked against the SAME header (AUDIT
    /// 0.32.1, P1-4): a response validator that contradicts
    /// the task's means the file changed mid-download.
    validator_is_etag: bool,
    part_path: PathBuf,
    /// Single-stream mode: one segment, plain GET, no Range.
    single: bool,
    total: Option<u64>,
    /// This run's generation token (AUDIT 0.32.2, Fix 2),
    /// minted by `schedule` from the task's gate.
    gen_token: u64,
    /// The task's live generation, shared with the manager:
    /// when it no longer equals `gen_token`, this run is
    /// stale and the sink refuses every further write.
    live_gen: Arc<AtomicU64>,
}

struct TaskRuntime {
    state: TaskState,
    control: Control,
    speed: SpeedMeter,
    /// The supervisor's live view of progress (its planner's
    /// count, mirrored at tick time); authoritative for
    /// Downloading tasks, whose on-disk state lags by design.
    live_downloaded: u64,
    /// The running task's planner, shared with its workers;
    /// registered so `pause_all_and_persist` can snapshot true
    /// progress synchronously at shutdown.
    live_planner: Option<Arc<Mutex<Planner>>>,
    /// A connection-count change waiting for the supervisor's
    /// next tick (`Planner::replan_pending`).
    replan_request: Option<u32>,
    supervisor: Option<JoinHandle<()>>,
    /// The run-generation gate (AUDIT 0.32.2, Fix 2): the
    /// manager invalidates it the instant a pause/cancel
    /// lands, so the retiring supervisor's gated writes and
    /// its workers' sink all stop at once — pause no longer
    /// waits for workers parked in a 30 s receive.
    gate: GenerationGate,
    /// Mirror of the gate's current generation, readable
    /// lock-free by worker sinks on the chunk path.
    live_gen: Arc<AtomicU64>,
    /// Auto-retry deadline (0.33): wall-clock ms at which a
    /// Failed task with a pending auto-retry becomes Queued
    /// again; 0 = no retry pending.
    retry_not_before: u64,
    /// The pending auto-retry is a from-zero restart (the
    /// `RestartOnce` policy), not a resume.
    retry_restart: bool,
    /// A `refresh_link` probe is in flight for this task (0.33).
    refreshing: bool,
}

struct Inner {
    tasks: HashMap<u64, TaskRuntime>,
    /// Task ids in arrival order (oldest first).
    order: Vec<u64>,
    next_id: u64,
    default_connections: u32,
    version: u64,
    downloads_dir: PathBuf,
    state_dir: PathBuf,
    /// How many tasks may Download at once (0.33): the
    /// app-configured replacement for the core's
    /// `MAX_ACTIVE_TASKS` constant, which now only supplies
    /// the fallback when the configured value is 0.
    max_concurrent: u32,
    /// Mirror of the shared bucket's configured limit, for
    /// the getter (0.33): `None` = off (a configured 0).
    speed_limit_kbps: Option<u64>,
}

struct Shared {
    inner: Mutex<Inner>,
    notify: NotifyFn,
    /// The one speed-limit bucket every worker draws from
    /// (0.33). Rate is bytes/sec (kbps × 1024); 0 = off.
    bucket: Arc<Mutex<TokenBucket>>,
}

/// What the chrome renders for one task (built by `views()`).
#[derive(Debug, Clone)]
pub struct DownloadView {
    pub id: u64,
    pub name: String,
    pub host: String,
    pub status: TaskStatus,
    pub total: Option<u64>,
    pub downloaded: u64,
    pub percent: Option<f64>,
    pub size_text: String,
    pub detail_text: String,
    pub connections: u32,
    pub error: Option<String>,
    /// The task's (userinfo-stripped) URL (0.33).
    pub url: String,
    /// Wall-clock ms when the task was added (0.33).
    pub added_ms: u64,
    /// Hex SHA-256 of the finished file, once computed (0.33).
    pub sha256: Option<String>,
    /// Plain-language hint for a finally-failed task (0.33).
    pub hint: Option<String>,
    /// "Retrying in {n} · attempt {k} of 3" (or the
    /// "Restarting in …" form) while an auto-retry is
    /// pending; replaces `detail_text` for such rows (0.33).
    pub retry_text: Option<String>,
    /// An auto-retry is pending for this Failed task (0.33).
    pub retry_pending: bool,
    /// A `refresh_link` probe is in flight (0.33).
    pub refreshing: bool,
    /// Live transfer rate in bytes/sec (0.35): `Some` only
    /// while Downloading with a finite, positive meter rate
    /// — the popover's aggregate speed sums these itself.
    pub speed_bps: Option<u64>,
    /// Seconds left at the current rate (0.35): the same
    /// `eta_seconds` value the Downloading `detail_text`
    /// renders; `None` for every other status.
    pub eta_secs: Option<u64>,
}

/// The download manager handle. Clone is cheap (one `Arc`).
#[derive(Clone)]
pub struct DownloadManager {
    shared: Arc<Shared>,
}

impl DownloadManager {
    /// Build the manager and load persisted tasks from
    /// `state_dir` (the core's `load_tasks`: Downloading on
    /// disk comes back Paused — nothing auto-resumes at boot).
    ///
    /// `notify` is the "downloads changed" callback. It is a
    /// closure rather than an `EventLoopProxy<UserEvent>`
    /// because `UserEvent` is a private enum of `app.rs`:
    /// sibling modules cannot name it (the other bin modules,
    /// `dpi`/`log`, are standalone and never reference app
    /// types). The closure keeps the manager decoupled from the
    /// event enum entirely — the app wraps its proxy in one
    /// line, or uses [`DownloadManager::new_with_proxy`].
    pub fn new(
        notify: NotifyFn,
        downloads_dir: PathBuf,
        state_dir: PathBuf,
        default_connections: u32,
        max_concurrent: u32,
        speed_limit_kbps: u64,
    ) -> DownloadManager {
        let _ = std::fs::create_dir_all(&downloads_dir);
        let _ = std::fs::create_dir_all(&state_dir);
        let loaded = load_tasks(&state_dir);
        // AUDIT 0.40.1: the record cap was enforced on `add`
        // only — a state dir planted with more than
        // MAX_TASK_RECORDS task files loaded every one. Keep
        // the newest records, unfinished ahead of history.
        let loaded = if loaded.len() > MAX_TASK_RECORDS {
            let mut loaded = loaded;
            loaded.sort_by(|a, b| {
                let history =
                    |s: &TaskState| matches!(s.status, TaskStatus::Complete | TaskStatus::Failed);
                history(a)
                    .cmp(&history(b))
                    .then(b.added_ms.cmp(&a.added_ms))
            });
            loaded.truncate(MAX_TASK_RECORDS);
            loaded
        } else {
            loaded
        };
        let mut tasks: HashMap<u64, TaskRuntime> = HashMap::new();
        let mut order: Vec<u64> = Vec::new();
        let mut max_id: u64 = 0;
        for state in loaded {
            max_id = max_id.max(state.id);
            order.push(state.id);
            let live_downloaded = state.downloaded();
            tasks.insert(
                state.id,
                TaskRuntime {
                    state,
                    control: Control::Run,
                    speed: SpeedMeter::new(),
                    live_downloaded,
                    live_planner: None,
                    replan_request: None,
                    supervisor: None,
                    gate: GenerationGate::new(),
                    live_gen: Arc::new(AtomicU64::new(0)),
                    retry_not_before: 0,
                    retry_restart: false,
                    refreshing: false,
                },
            );
        }
        // 0.33: a configured max_concurrent of 0 falls back
        // to the core constant; anything else clamps to the
        // settings range 1..=5. A speed limit of 0 is off.
        let max_concurrent = if max_concurrent == 0 {
            u32::try_from(MAX_ACTIVE_TASKS).unwrap_or(3)
        } else {
            max_concurrent.clamp(1, 5)
        };
        let bucket = Arc::new(Mutex::new(TokenBucket::new(
            speed_limit_kbps.saturating_mul(1024),
            now_ms(),
        )));
        DownloadManager {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    tasks,
                    order,
                    next_id: max_id.saturating_add(1).max(1),
                    default_connections: if default_connections == 0 {
                        DEFAULT_CONNECTIONS
                    } else {
                        clamp_connections(default_connections)
                    },
                    version: 0,
                    downloads_dir,
                    state_dir,
                    max_concurrent,
                    speed_limit_kbps: if speed_limit_kbps == 0 {
                        None
                    } else {
                        Some(speed_limit_kbps)
                    },
                }),
                notify,
                bucket,
            }),
        }
    }

    /// The `EventLoopProxy` form of [`DownloadManager::new`]:
    /// generic over the event type so this module never names
    /// `app::UserEvent` (a private type — see `new`). The app
    /// calls it as `new_with_proxy(proxy.clone(),
    /// || UserEvent::DownloadsChanged, dir, state_dir, n)`.
    pub fn new_with_proxy<E, F>(
        proxy: EventLoopProxy<E>,
        make_event: F,
        downloads_dir: PathBuf,
        state_dir: PathBuf,
        default_connections: u32,
        max_concurrent: u32,
        speed_limit_kbps: u64,
    ) -> DownloadManager
    where
        E: Send + 'static,
        F: Fn() -> E + Send + Sync + 'static,
    {
        let notify: NotifyFn = Arc::new(move || {
            let _ = proxy.send_event(make_event());
        });
        DownloadManager::new(
            notify,
            downloads_dir,
            state_dir,
            default_connections,
            max_concurrent,
            speed_limit_kbps,
        )
    }

    /// Add a task. The final name is chosen NOW (suggested →
    /// URL-derived → "download", sanitized, deduped against the
    /// target folder AND the other tasks' names) and locked;
    /// the probe's Content-Disposition name may replace it only
    /// before the task first starts (see the supervisor).
    /// `connections == 0` means "the current default". Returns
    /// the task id, or `None` for a non-http(s) URL.
    pub fn add(
        &self,
        url: &str,
        suggested_name: Option<&str>,
        cookies: Vec<CookiePair>,
        connections: u32,
        referer: Option<String>,
    ) -> Option<u64> {
        if !is_downloadable_url(url) {
            crate::log::info("download refused: not an http(s) URL");
            return None;
        }
        // AUDIT 0.32.1 (P2-4/P1-19): the stored URL drops any
        // embedded credentials (the transport never forwards
        // them anyway), and the captured cookies are
        // sanitized HERE — through 0.32 only the disk-load
        // path sanitized, so the live path stored them
        // verbatim, header-injection shapes included.
        let url = strip_url_userinfo(url);
        let url = url.as_str();
        let cookies = sanitize_cookies(cookies);
        // 0.33: the referer is sanitized at the door, like
        // the URL and the cookies (never logged, either).
        let referer = sanitize_referer(referer);
        let raw_name = match suggested_name {
            Some(s) if !s.trim().is_empty() => sanitize_file_name(s),
            _ => file_name_from_url(url),
        };
        let host = url_host(url).unwrap_or_else(|| "unknown host".to_string());
        let mut inner = lock(&self.shared.inner);
        // AUDIT 0.32.1 (P2-2): task-count caps — a page
        // looping download triggers must not be able to pile
        // up unbounded records, state files and queued work.
        if inner.order.len() >= MAX_TASK_RECORDS {
            crate::log::info("download refused: task record cap reached");
            return None;
        }
        let unfinished = inner
            .tasks
            .values()
            .filter(|t| t.state.status != TaskStatus::Complete)
            .count();
        if unfinished >= MAX_UNFINISHED_TASKS {
            crate::log::info("download refused: too many unfinished tasks");
            return None;
        }
        let downloads_dir = inner.downloads_dir.clone();
        let name = dedupe_file_name(&raw_name, |candidate| {
            downloads_dir.join(candidate).exists()
                || downloads_dir.join(format!("{candidate}.part")).exists()
                || inner.tasks.values().any(|t| t.state.file_name == candidate)
        });
        // AUDIT 0.40.1: dedupe exhaustion (~100k collisions)
        // returns a name that is still taken; refuse honestly
        // instead of minting a task that shares a file name.
        if downloads_dir.join(&name).exists()
            || downloads_dir.join(format!("{name}.part")).exists()
            || inner.tasks.values().any(|t| t.state.file_name == name)
        {
            crate::log::info("download refused: too many files with this name");
            return None;
        }
        let connections = if connections == 0 {
            inner.default_connections
        } else {
            clamp_connections(connections)
        };
        let id = inner.next_id;
        inner.next_id = inner.next_id.saturating_add(1).max(1);
        let state = TaskState {
            version: TASK_FILE_VERSION,
            id,
            url: url.to_string(),
            file_name: name,
            total: None,
            etag: None,
            last_modified: None,
            connections,
            status: TaskStatus::Queued,
            added_ms: now_ms(),
            segments: Vec::new(),
            cookies,
            error: None,
            referer,
            sha256: None,
            auto_retries: 0,
            error_hint: None,
        };
        inner.tasks.insert(
            id,
            TaskRuntime {
                live_downloaded: 0,
                state: state.clone(),
                control: Control::Run,
                speed: SpeedMeter::new(),
                live_planner: None,
                replan_request: None,
                supervisor: None,
                gate: GenerationGate::new(),
                live_gen: Arc::new(AtomicU64::new(0)),
                retry_not_before: 0,
                retry_restart: false,
                refreshing: false,
            },
        );
        inner.order.push(id);
        drop(inner);
        persist_task(&self.shared, &state);
        crate::log::info(&format!("download added: {host}"));
        bump_and_notify(&self.shared);
        schedule(&self.shared);
        Some(id)
    }

    /// Record a download Pico refused at the interception
    /// boundary as a Failed task the user can SEE (AUDIT
    /// 0.32.2, Fix 1): through 0.32.1 the refusal was one log
    /// line and the click vanished silently. Mirrors `add`'s
    /// bookkeeping — userinfo stripped, name from the
    /// suggestion (sanitized) or the URL, deduped against
    /// the folder and the other tasks, BOTH task caps — but
    /// the task is born Failed with the triage reason as its
    /// error, carries no cookies and no segments, and is
    /// NEVER scheduled. It deliberately does NOT gate on
    /// `is_downloadable_url`: a `blob:` URL is exactly the
    /// kind of record this exists for. Returns `None` only
    /// when a cap refuses the record.
    pub fn add_failed(&self, url: &str, suggested_name: Option<&str>, reason: &str) -> Option<u64> {
        let url = strip_url_userinfo(url);
        let url = url.as_str();
        let raw_name = match suggested_name {
            Some(s) if !s.trim().is_empty() => sanitize_file_name(s),
            _ => file_name_from_url(url),
        };
        let host = url_host(url).unwrap_or_else(|| "unknown host".to_string());
        let mut inner = lock(&self.shared.inner);
        if inner.order.len() >= MAX_TASK_RECORDS {
            crate::log::info("download rejected record refused: task record cap reached");
            return None;
        }
        let unfinished = inner
            .tasks
            .values()
            .filter(|t| t.state.status != TaskStatus::Complete)
            .count();
        if unfinished >= MAX_UNFINISHED_TASKS {
            crate::log::info("download rejected record refused: too many unfinished tasks");
            return None;
        }
        let downloads_dir = inner.downloads_dir.clone();
        let name = dedupe_file_name(&raw_name, |candidate| {
            downloads_dir.join(candidate).exists()
                || downloads_dir.join(format!("{candidate}.part")).exists()
                || inner.tasks.values().any(|t| t.state.file_name == candidate)
        });
        // AUDIT 0.40.1: same exhaustion guard as `add`.
        if downloads_dir.join(&name).exists()
            || downloads_dir.join(format!("{name}.part")).exists()
            || inner.tasks.values().any(|t| t.state.file_name == name)
        {
            crate::log::info("download rejected record refused: too many files with this name");
            return None;
        }
        let connections = inner.default_connections;
        let id = inner.next_id;
        inner.next_id = inner.next_id.saturating_add(1).max(1);
        let state = TaskState {
            version: TASK_FILE_VERSION,
            id,
            url: url.to_string(),
            file_name: name,
            total: None,
            etag: None,
            last_modified: None,
            connections,
            status: TaskStatus::Failed,
            added_ms: now_ms(),
            segments: Vec::new(),
            cookies: Vec::new(),
            error: Some(reason.to_string()),
            referer: None,
            sha256: None,
            auto_retries: 0,
            error_hint: None,
        };
        inner.tasks.insert(
            id,
            TaskRuntime {
                live_downloaded: 0,
                state: state.clone(),
                control: Control::Run,
                speed: SpeedMeter::new(),
                live_planner: None,
                replan_request: None,
                supervisor: None,
                gate: GenerationGate::new(),
                live_gen: Arc::new(AtomicU64::new(0)),
                retry_not_before: 0,
                retry_restart: false,
                refreshing: false,
            },
        );
        inner.order.push(id);
        drop(inner);
        persist_task(&self.shared, &state);
        crate::log::info(&format!("download rejected: {host}"));
        bump_and_notify(&self.shared);
        Some(id)
    }

    /// The locked final file name of one task (0.32.3):
    /// the app's "download started" toast speaks the name
    /// the manager actually deduped and locked at `add`
    /// time, never a second derivation of its own.
    pub fn name_of(&self, id: u64) -> Option<String> {
        let inner = lock(&self.shared.inner);
        inner.tasks.get(&id).map(|t| t.state.file_name.clone())
    }

    /// Pause a task. A queued task parks immediately; a
    /// running task parks HERE, in the manager, in this call
    /// (AUDIT 0.32.2, Fix 2): the run's generation is
    /// invalidated (its supervisor's remaining writes are
    /// gated out and its workers' sink refuses further
    /// bytes), the live planner is folded into the state,
    /// and the Paused snapshot is persisted before `pause`
    /// returns. Through 0.32.1 pause only set a control flag
    /// and the status flipped whenever the supervisor wound
    /// down — workers parked in a 30 s receive made that
    /// seconds away. Unconfirmed in-flight bytes are simply
    /// not in the folded map and are re-fetched on resume:
    /// that is the intended semantics (progress only ever
    /// claims written, confirmed bytes).
    pub fn pause(&self, id: u64) {
        let mut park_now: Option<TaskState> = None;
        {
            let mut inner = lock(&self.shared.inner);
            if let Some(rt) = inner.tasks.get_mut(&id) {
                match rt.state.status {
                    TaskStatus::Queued => {
                        rt.state.status = TaskStatus::Paused;
                        park_now = Some(rt.state.clone());
                    }
                    TaskStatus::Downloading => {
                        rt.gate.invalidate();
                        rt.live_gen.store(rt.gate.current(), Ordering::SeqCst);
                        rt.control = Control::Pause;
                        if let Some(planner) = rt.live_planner.take() {
                            lock(&planner).sync_into(&mut rt.state);
                        }
                        rt.state.status = TaskStatus::Paused;
                        rt.live_downloaded = rt.state.downloaded();
                        park_now = Some(rt.state.clone());
                    }
                    // 0.33: a Failed task with a pending
                    // auto-retry can be paused too — the retry
                    // is cancelled and the row shows as Paused,
                    // not Failed (its error is cleared with the
                    // retry). A Failed task WITHOUT a pending
                    // retry keeps the existing no-op.
                    TaskStatus::Failed if rt.retry_not_before != 0 => {
                        rt.retry_not_before = 0;
                        rt.retry_restart = false;
                        rt.state.status = TaskStatus::Paused;
                        rt.state.error = None;
                        rt.state.error_hint = None;
                        park_now = Some(rt.state.clone());
                    }
                    _ => {}
                }
            }
        }
        if let Some(state) = park_now {
            persist_task(&self.shared, &state);
            log_host(&self.shared, id, "download paused");
            bump_and_notify(&self.shared);
            schedule(&self.shared);
        }
    }

    /// Resume a Paused or Failed task (it re-enters the queue;
    /// the scheduler starts it when a slot frees).
    pub fn resume(&self, id: u64) {
        let mut queued: Option<TaskState> = None;
        {
            let mut inner = lock(&self.shared.inner);
            if let Some(rt) = inner.tasks.get_mut(&id) {
                if matches!(rt.state.status, TaskStatus::Paused | TaskStatus::Failed) {
                    rt.state.status = TaskStatus::Queued;
                    rt.state.error = None;
                    // 0.33: a manual resume is a fresh start
                    // for the auto-retry machinery — full
                    // retry budget again, no pending timer,
                    // no stale hint.
                    rt.state.error_hint = None;
                    rt.state.auto_retries = 0;
                    rt.retry_not_before = 0;
                    rt.retry_restart = false;
                    rt.control = Control::Run;
                    queued = Some(rt.state.clone());
                }
            }
        }
        if let Some(state) = queued {
            persist_task(&self.shared, &state);
            log_host(&self.shared, id, "download resumed");
            bump_and_notify(&self.shared);
            schedule(&self.shared);
        }
    }

    /// Restart a task from zero (0.33): only Queued / Paused /
    /// Failed tasks — a Downloading or Complete task is a
    /// no-op. The segment map and every validator are cleared
    /// under the lock, the `.part` is removed outside it, and
    /// the fresh state is persisted before scheduling.
    pub fn restart(&self, id: u64) {
        let mut work: Option<(TaskState, PathBuf)> = None;
        {
            let mut inner = lock(&self.shared.inner);
            let downloads_dir = inner.downloads_dir.clone();
            if let Some(rt) = inner.tasks.get_mut(&id) {
                if matches!(
                    rt.state.status,
                    TaskStatus::Queued | TaskStatus::Paused | TaskStatus::Failed
                ) {
                    let part = part_path(&downloads_dir, &rt.state.file_name);
                    rt.state.segments.clear();
                    rt.state.total = None;
                    rt.state.etag = None;
                    rt.state.last_modified = None;
                    rt.state.sha256 = None;
                    rt.state.status = TaskStatus::Queued;
                    rt.state.error = None;
                    rt.state.error_hint = None;
                    rt.state.auto_retries = 0;
                    rt.retry_not_before = 0;
                    rt.retry_restart = false;
                    rt.live_downloaded = 0;
                    rt.live_planner = None;
                    rt.control = Control::Run;
                    work = Some((rt.state.clone(), part));
                }
            }
        }
        if let Some((state, part)) = work {
            let _ = std::fs::remove_file(&part);
            persist_task(&self.shared, &state);
            log_host(&self.shared, id, "download restarted");
            bump_and_notify(&self.shared);
            schedule(&self.shared);
        }
    }

    /// Rebind a task to a fresh URL (0.33): only Failed /
    /// Paused / Queued tasks (a Downloading or Complete task
    /// is a no-op), and only one refresh per task at a time.
    /// The probe runs on a spawned thread (no lock is held
    /// across it); the core's `refresh_decision` decides
    /// resume-vs-restart from the saved state and the probe
    /// facts. The file NAME never changes here — the
    /// disposition rename only happens pre-first-start inside
    /// `run_task`.
    pub fn refresh_link(&self, id: u64, new_url: &str) {
        if !is_downloadable_url(new_url) {
            crate::log::info("download refresh refused: not an http(s) URL");
            return;
        }
        let new_url = strip_url_userinfo(new_url);
        let mut start: Option<(TaskState, Vec<CookiePair>)> = None;
        {
            let mut inner = lock(&self.shared.inner);
            if let Some(rt) = inner.tasks.get_mut(&id) {
                if rt.refreshing {
                    return;
                }
                if matches!(
                    rt.state.status,
                    TaskStatus::Failed | TaskStatus::Paused | TaskStatus::Queued
                ) {
                    rt.refreshing = true;
                    let cookies = rt.state.cookies.clone();
                    start = Some((rt.state.clone(), cookies));
                }
            }
        }
        let Some((snapshot, cookies)) = start else {
            return;
        };
        bump_and_notify(&self.shared);
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            refresh_link_thread(shared, id, snapshot, cookies, new_url);
        });
    }

    /// Delete a task AND its file (0.33): a Downloading task
    /// is a no-op. Complete: the finished file is removed,
    /// then the record + state file (mirroring `cancel`'s
    /// bookkeeping). Failed / Paused / Queued: the `.part`
    /// is removed with the record.
    pub fn delete_file(&self, id: u64) {
        enum After {
            Nothing,
            RemoveNow { file: PathBuf, host: String },
        }
        let after = {
            let mut inner = lock(&self.shared.inner);
            let downloads_dir = inner.downloads_dir.clone();
            match inner.tasks.get(&id) {
                Some(rt) => {
                    if matches!(rt.state.status, TaskStatus::Downloading) {
                        After::Nothing
                    } else {
                        let host =
                            url_host(&rt.state.url).unwrap_or_else(|| "unknown host".to_string());
                        let file = if matches!(rt.state.status, TaskStatus::Complete) {
                            downloads_dir.join(&rt.state.file_name)
                        } else {
                            part_path(&downloads_dir, &rt.state.file_name)
                        };
                        inner.tasks.remove(&id);
                        inner.order.retain(|t| *t != id);
                        After::RemoveNow { file, host }
                    }
                }
                None => After::Nothing,
            }
        };
        if let After::RemoveNow { file, host } = after {
            let _ = std::fs::remove_file(file);
            let state_dir = lock(&self.shared.inner).state_dir.clone();
            delete_task_file(&state_dir, id);
            crate::log::info(&format!("download deleted: {host}"));
            bump_and_notify(&self.shared);
            schedule(&self.shared);
        }
    }

    /// Remove every Complete record + its state file (0.33).
    /// The finished FILES stay on disk.
    pub fn clear_completed(&self) {
        let ids: Vec<u64> = {
            let mut inner = lock(&self.shared.inner);
            let ids: Vec<u64> = inner
                .tasks
                .values()
                .filter(|t| matches!(t.state.status, TaskStatus::Complete))
                .map(|t| t.state.id)
                .collect();
            for id in &ids {
                inner.tasks.remove(id);
            }
            inner.order.retain(|t| !ids.contains(t));
            ids
        };
        if ids.is_empty() {
            return;
        }
        let state_dir = lock(&self.shared.inner).state_dir.clone();
        for id in &ids {
            delete_task_file(&state_dir, *id);
        }
        crate::log::info(&format!("downloads cleared completed: {}", ids.len()));
        bump_and_notify(&self.shared);
    }

    /// Remove every Complete / Failed / Paused / Queued
    /// record + state file (0.33): the `.part` files of the
    /// unfinished ones are deleted, finished files stay, and
    /// Downloading tasks are untouched.
    pub fn clear_all(&self) {
        let (ids, parts): (Vec<u64>, Vec<PathBuf>) = {
            let mut inner = lock(&self.shared.inner);
            let downloads_dir = inner.downloads_dir.clone();
            let mut ids: Vec<u64> = Vec::new();
            let mut parts: Vec<PathBuf> = Vec::new();
            for rt in inner.tasks.values() {
                match rt.state.status {
                    TaskStatus::Downloading => {}
                    TaskStatus::Complete => ids.push(rt.state.id),
                    TaskStatus::Failed | TaskStatus::Paused | TaskStatus::Queued => {
                        ids.push(rt.state.id);
                        parts.push(part_path(&downloads_dir, &rt.state.file_name));
                    }
                }
            }
            for id in &ids {
                inner.tasks.remove(id);
            }
            inner.order.retain(|t| !ids.contains(t));
            (ids, parts)
        };
        if ids.is_empty() {
            return;
        }
        for part in &parts {
            let _ = std::fs::remove_file(part);
        }
        let state_dir = lock(&self.shared.inner).state_dir.clone();
        for id in &ids {
            delete_task_file(&state_dir, *id);
        }
        crate::log::info(&format!("downloads cleared: {}", ids.len()));
        bump_and_notify(&self.shared);
    }

    /// The finished file's path for a Complete task (0.33);
    /// `None` for every other status (and unknown ids).
    pub fn completed_path(&self, id: u64) -> Option<PathBuf> {
        let inner = lock(&self.shared.inner);
        let rt = inner.tasks.get(&id)?;
        if matches!(rt.state.status, TaskStatus::Complete) {
            Some(inner.downloads_dir.join(&rt.state.file_name))
        } else {
            None
        }
    }

    /// Set how many tasks may Download at once (0.33),
    /// clamped to 1..=5, then schedule — raising the cap may
    /// start queued work at once.
    pub fn set_max_concurrent(&self, n: u32) {
        let n = n.clamp(1, 5);
        let changed = {
            let mut inner = lock(&self.shared.inner);
            let changed = inner.max_concurrent != n;
            inner.max_concurrent = n;
            changed
        };
        if changed {
            bump_and_notify(&self.shared);
        }
        schedule(&self.shared);
    }

    /// The current concurrent-download cap (0.33). (0.34: its
    /// app-side readers moved to the coordinator's snapshot;
    /// kept as the manager's public settings read API.)
    #[allow(dead_code)]
    pub fn max_concurrent(&self) -> u32 {
        lock(&self.shared.inner).max_concurrent
    }

    /// Set the manager-wide speed limit (0.33): `None` (or a
    /// kbps of 0 at construction) is off. The bucket's rate
    /// is bytes/sec (kbps × 1024).
    pub fn set_speed_limit(&self, kbps: Option<u64>) {
        let changed = {
            let mut inner = lock(&self.shared.inner);
            let changed = inner.speed_limit_kbps != kbps;
            inner.speed_limit_kbps = kbps;
            changed
        };
        {
            let mut bucket = lock(&self.shared.bucket);
            bucket.set_rate(kbps.map(|k| k.saturating_mul(1024)).unwrap_or(0), now_ms());
        }
        if changed {
            bump_and_notify(&self.shared);
        }
    }

    /// The current speed limit in kbps (0.33); `None` = off.
    /// (0.34: kept as public settings read API — see
    /// `max_concurrent`.)
    #[allow(dead_code)]
    pub fn speed_limit_kbps(&self) -> Option<u64> {
        lock(&self.shared.inner).speed_limit_kbps
    }

    /// Cancel a task. Every status is removed HERE (AUDIT
    /// 0.32.2, Fix 2: a running task's cleanup used to wait
    /// for its supervisor to wind down): `.part` + state file
    /// deleted, record removed. Complete: only the record is
    /// removed — the finished file stays.
    pub fn cancel(&self, id: u64) {
        enum After {
            Nothing,
            RemoveNow { part: Option<PathBuf>, host: String },
        }
        let after = {
            let mut inner = lock(&self.shared.inner);
            let downloads_dir = inner.downloads_dir.clone();
            match inner.tasks.get_mut(&id) {
                Some(rt) => {
                    // AUDIT 0.32.2 (Fix 2): a running task is
                    // cancelled HERE, in the manager, in this
                    // lock hold — its generation is invalidated
                    // (the retiring supervisor is gated out,
                    // its workers' sink refuses further writes)
                    // and the record is removed at once, instead
                    // of waiting for the supervisor to notice.
                    // Deleting the `.part` below succeeds even
                    // while a stale worker still holds it open
                    // (std opens with FILE_SHARE_DELETE on
                    // Windows), and the sink gate refuses its
                    // further writes anyway.
                    if matches!(rt.state.status, TaskStatus::Downloading) {
                        rt.gate.invalidate();
                        rt.live_gen.store(rt.gate.current(), Ordering::SeqCst);
                        rt.control = Control::Cancel;
                    }
                    let host =
                        url_host(&rt.state.url).unwrap_or_else(|| "unknown host".to_string());
                    let part = if matches!(rt.state.status, TaskStatus::Complete) {
                        None
                    } else {
                        Some(part_path(&downloads_dir, &rt.state.file_name))
                    };
                    inner.tasks.remove(&id);
                    inner.order.retain(|t| *t != id);
                    After::RemoveNow { part, host }
                }
                None => After::Nothing,
            }
        };
        if let After::RemoveNow { part, host } = after {
            if let Some(part) = part {
                let _ = std::fs::remove_file(part);
            }
            let state_dir = lock(&self.shared.inner).state_dir.clone();
            delete_task_file(&state_dir, id);
            crate::log::info(&format!("download cancelled: {host}"));
            bump_and_notify(&self.shared);
            schedule(&self.shared);
        }
    }

    /// Change a task's connection count (the rule is in the
    /// module doc): queued → the plan at start; running → the
    /// supervisor re-plans the not-yet-started ranges at its
    /// next tick; paused/failed → the plan at resume.
    pub fn set_connections(&self, id: u64, n: u32) {
        let n = clamp_connections(n);
        let mut persist: Option<TaskState> = None;
        {
            let mut inner = lock(&self.shared.inner);
            if let Some(rt) = inner.tasks.get_mut(&id) {
                if rt.state.connections != n {
                    rt.state.connections = n;
                    if matches!(rt.state.status, TaskStatus::Downloading) {
                        rt.replan_request = Some(n);
                    }
                    persist = Some(rt.state.clone());
                }
            }
        }
        if let Some(state) = persist {
            persist_task(&self.shared, &state);
            bump_and_notify(&self.shared);
        }
    }

    /// Change the default connection count for FUTURE tasks
    /// (the app persists it in its own state; the manager only
    /// mirrors it so `add` can use it).
    pub fn set_default_connections(&self, n: u32) {
        let n = if n == 0 {
            DEFAULT_CONNECTIONS
        } else {
            clamp_connections(n)
        };
        let mut inner = lock(&self.shared.inner);
        if inner.default_connections != n {
            inner.default_connections = n;
            drop(inner);
            bump_and_notify(&self.shared);
        }
    }

    /// Render data for every task, newest first.
    pub fn views(&self) -> Vec<DownloadView> {
        let now = now_ms();
        let inner = lock(&self.shared.inner);
        let mut out: Vec<DownloadView> = Vec::with_capacity(inner.order.len());
        for id in inner.order.iter().rev() {
            let Some(rt) = inner.tasks.get(id) else {
                continue;
            };
            let st = &rt.state;
            let downloaded = if matches!(st.status, TaskStatus::Downloading) {
                rt.live_downloaded
            } else {
                st.downloaded()
            };
            let total = st.total;
            let rate = if matches!(st.status, TaskStatus::Downloading) {
                rt.speed.rate(now)
            } else {
                0.0
            };
            let dl_text = humanize_bytes(downloaded);
            let size_text = match total {
                Some(t) => humanize_bytes(t),
                None => "Unknown size".to_string(),
            };
            // 0.33: a Failed task with a pending auto-retry
            // carries the countdown in `retry_text` (seconds
            // remaining, ceil, min 1) — its own slot since
            // AUDIT 0.34.2 (G1); `detail_text` below always
            // stays the byte-progress detail.
            let retry_pending = matches!(st.status, TaskStatus::Failed) && rt.retry_not_before != 0;
            let retry_text = if retry_pending {
                let remaining_ms = rt.retry_not_before.saturating_sub(now);
                let secs = remaining_ms.saturating_add(999) / 1000;
                let secs = secs.max(1);
                if rt.retry_restart {
                    Some(format!(
                        "Restarting in {secs} · attempt {} of {AUTO_RETRY_MAX}",
                        st.auto_retries
                    ))
                } else {
                    Some(format!(
                        "Retrying in {secs} · attempt {} of {AUTO_RETRY_MAX}",
                        st.auto_retries
                    ))
                }
            } else {
                None
            };
            // 0.35: the ETA is computed ONCE for a Downloading
            // task and used for both the detail text and the
            // exposed `eta_secs` field; every other status
            // exposes `None`.
            let eta_secs = if matches!(st.status, TaskStatus::Downloading) {
                eta_seconds(total, downloaded, rate)
            } else {
                None
            };
            let speed_bps =
                if matches!(st.status, TaskStatus::Downloading) && rate.is_finite() && rate > 0.0 {
                    Some(rate.round() as u64)
                } else {
                    None
                };
            let detail_text = match st.status {
                TaskStatus::Downloading => {
                    let speed = speed_text(rate);
                    match (total, eta_secs) {
                        (Some(_), Some(eta)) => format!(
                            "{dl_text} of {size_text} · {speed} · {} left",
                            eta_text(Some(eta))
                        ),
                        (Some(_), None) => format!("{dl_text} of {size_text} · {speed}"),
                        (None, _) => format!("{dl_text} · {speed}"),
                    }
                }
                // AUDIT 0.32.1 (P4-10): a queued task that
                // already holds bytes shows them, like Paused —
                // through 0.32 a re-queued half-downloaded task
                // hid its progress behind a bare total.
                TaskStatus::Queued => match total {
                    Some(_) if downloaded > 0 => {
                        format!("{dl_text} of {size_text} · Queued")
                    }
                    _ => format!("{size_text} · Queued"),
                },
                TaskStatus::Paused => match total {
                    Some(_) => format!("{dl_text} of {size_text} · Paused"),
                    None => format!("{dl_text} · Paused"),
                },
                TaskStatus::Complete => humanize_bytes(total.unwrap_or(downloaded)),
                TaskStatus::Failed => match total {
                    Some(_) => format!("{dl_text} of {size_text} · Failed"),
                    None => format!("{dl_text} · Failed"),
                },
            };
            // AUDIT 0.34.2 (G1): the retry countdown rides
            // `retry_text` ONLY — through 0.34.1 it also
            // overwrote `detail_text`, so the downloads page
            // (which renders both slots) printed the countdown
            // twice and lost the byte-progress detail for the
            // whole pending window. The renderers own the
            // presentation now: the page shows detail + the
            // countdown as separate lines; the popover's
            // single detail slot prefers the countdown.
            out.push(DownloadView {
                id: st.id,
                name: st.file_name.clone(),
                host: url_host(&st.url).unwrap_or_default(),
                status: st.status,
                total,
                downloaded,
                percent: percent(downloaded, total),
                size_text,
                detail_text,
                connections: st.connections,
                error: st.error.clone(),
                url: st.url.clone(),
                added_ms: st.added_ms,
                sha256: st.sha256.clone(),
                hint: st.error_hint.clone(),
                retry_text,
                retry_pending,
                refreshing: rt.refreshing,
                speed_bps,
                eta_secs,
            });
        }
        out
    }

    /// How many tasks are Downloading right now.
    pub fn active_count(&self) -> usize {
        let inner = lock(&self.shared.inner);
        inner
            .tasks
            .values()
            .filter(|t| matches!(t.state.status, TaskStatus::Downloading))
            .count()
    }

    /// Change counter: bumps on every visible change, in step
    /// with the notifier (the app's snapshot signature folds it).
    pub fn version(&self) -> u64 {
        lock(&self.shared.inner).version
    }

    pub fn downloads_dir(&self) -> PathBuf {
        lock(&self.shared.inner).downloads_dir.clone()
    }

    /// Shutdown path (the app's WinClose arm calls this before
    /// teardown): set every running task's control to Pause and
    /// SYNCHRONOUSLY persist every task's state, snapshotting
    /// running tasks from their live planners. The supervisor
    /// threads are not joined — the process is leaving — but
    /// the disk state is exact, because progress only ever
    /// advances after bytes are written.
    pub fn pause_all_and_persist(&self) {
        let snapshots: Vec<TaskState> = {
            let mut inner = lock(&self.shared.inner);
            let mut out = Vec::with_capacity(inner.tasks.len());
            for rt in inner.tasks.values_mut() {
                if matches!(rt.state.status, TaskStatus::Downloading) {
                    rt.control = Control::Pause;
                    // AUDIT 0.32.2 (Fix 2): invalidate the run
                    // generation too, so a dying supervisor
                    // cannot persist over the snapshots taken
                    // here (its state writes are gated on the
                    // token this retires).
                    rt.gate.invalidate();
                    rt.live_gen.store(rt.gate.current(), Ordering::SeqCst);
                }
                let mut state = rt.state.clone();
                if let Some(planner) = &rt.live_planner {
                    lock(planner).sync_into(&mut state);
                }
                // AUDIT 0.32.1 (P1-21): the parked status must
                // not depend on a live planner existing — a
                // task still in its probe window persisted as
                // Downloading (the next launch's loader parks
                // it anyway, but the disk state should be exact
                // at shutdown, not eventually).
                if matches!(state.status, TaskStatus::Downloading) {
                    state.status = TaskStatus::Paused;
                }
                out.push(state);
            }
            out
        };
        for state in &snapshots {
            persist_task(&self.shared, state);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn part_path(downloads_dir: &std::path::Path, file_name: &str) -> PathBuf {
    downloads_dir.join(format!("{file_name}.part"))
}

fn bump_and_notify(shared: &Arc<Shared>) {
    {
        let mut inner = lock(&shared.inner);
        inner.version = inner.version.wrapping_add(1);
    }
    (shared.notify)();
}

fn persist_task(shared: &Arc<Shared>, state: &TaskState) {
    let dir = lock(&shared.inner).state_dir.clone();
    if let Err(e) = save_task(&dir, state) {
        crate::log::error(&format!(
            "download state save failed (task {}): {e}",
            state.id
        ));
    }
}

fn task_host(shared: &Arc<Shared>, id: u64) -> String {
    let inner = lock(&shared.inner);
    inner
        .tasks
        .get(&id)
        .and_then(|rt| url_host(&rt.state.url))
        .unwrap_or_else(|| "unknown host".to_string())
}

fn log_host(shared: &Arc<Shared>, id: u64, what: &str) {
    let host = task_host(shared, id);
    crate::log::info(&format!("{what}: {host}"));
}

fn control_of(shared: &Arc<Shared>, id: u64) -> Control {
    let inner = lock(&shared.inner);
    inner
        .tasks
        .get(&id)
        .map(|rt| rt.control)
        .unwrap_or(Control::Cancel)
}

/// Is `tok` still the task's live run generation? (AUDIT
/// 0.32.2, Fix 2.) A supervisor checks this before every
/// shared-state write: once the manager has retired the run
/// (pause/cancel/shutdown invalidated the gate) or a newer
/// run has begun, the answer is false and the stale
/// supervisor must touch nothing — the manager's snapshot,
/// or the newer run, owns the task now. A vanished task is
/// never current.
fn run_current(shared: &Arc<Shared>, id: u64, tok: u64) -> bool {
    let inner = lock(&shared.inner);
    inner
        .tasks
        .get(&id)
        .map(|rt| rt.gate.accepts(tok))
        .unwrap_or(false)
}

/// Start queued tasks (oldest first) while fewer than the
/// manager's configured `max_concurrent` (0.33; the core's
/// MAX_ACTIVE_TASKS is only its construction fallback) are
/// Downloading. Called after every mutation and at every
/// supervisor exit.
fn schedule(shared: &Arc<Shared>) {
    loop {
        let mut to_spawn: Vec<(u64, u64)> = Vec::new();
        {
            let mut inner = lock(&shared.inner);
            let active = inner
                .tasks
                .values()
                .filter(|t| matches!(t.state.status, TaskStatus::Downloading))
                .count();
            let mut slots = (inner.max_concurrent as usize).saturating_sub(active);
            if slots == 0 {
                return;
            }
            for id in inner.order.clone() {
                if slots == 0 {
                    break;
                }
                if let Some(rt) = inner.tasks.get_mut(&id) {
                    if matches!(rt.state.status, TaskStatus::Queued) {
                        rt.state.status = TaskStatus::Downloading;
                        rt.state.error = None;
                        rt.control = Control::Run;
                        rt.speed.reset();
                        rt.live_downloaded = rt.state.downloaded();
                        // AUDIT 0.32.2 (Fix 2): mint this
                        // run's generation token under the
                        // lock and mirror it for the workers'
                        // lock-free sink check; the supervisor
                        // carries the token for its whole life.
                        let tok = rt.gate.begin();
                        rt.live_gen.store(tok, Ordering::SeqCst);
                        to_spawn.push((id, tok));
                        slots -= 1;
                    }
                }
            }
            if !to_spawn.is_empty() {
                inner.version = inner.version.wrapping_add(1);
            }
        }
        if to_spawn.is_empty() {
            return;
        }
        (shared.notify)();
        for (id, tok) in to_spawn {
            let snapshot = {
                let inner = lock(&shared.inner);
                inner.tasks.get(&id).map(|rt| rt.state.clone())
            };
            if let Some(state) = snapshot {
                persist_task(shared, &state);
            }
            let sh = shared.clone();
            let handle = std::thread::spawn(move || run_task(sh, id, tok));
            let mut inner = lock(&shared.inner);
            match inner.tasks.get_mut(&id) {
                Some(rt) => {
                    rt.supervisor = Some(handle);
                }
                None => {
                    // The task was cancelled while its thread
                    // spawned; the thread sees the missing task
                    // and exits immediately. Detach it.
                    drop(handle);
                }
            }
        }
    }
}

/// Set a task Failed (used for failures before a planner
/// exists, and by the supervisor's finalization): status +
/// error persisted, `.part` KEPT for resume, event emitted,
/// scheduler re-run.
/// Fields the user can change through the manager WHILE a
/// supervisor works from its own snapshot (today: the
/// connection count) must survive the supervisor's state
/// overwrites (AUDIT 0.32.1, P4-A6: a count changed
/// mid-download was silently reverted at the phase boundary
/// when the supervisor's stale snapshot was written back).
fn preserve_live_connections(shared: &Arc<Shared>, id: u64, state: &mut TaskState) {
    let inner = lock(&shared.inner);
    if let Some(rt) = inner.tasks.get(&id) {
        state.connections = rt.state.connections;
    }
}

/// Fail a task — unless a Pause/Cancel landed first, in
/// which case the user's stop wins (AUDIT 0.32.1, P1-9: a
/// cancel arriving during the probe was silently discarded
/// when the probe then errored, resurrecting the task as
/// Failed with its `.part` kept).
fn fail_or_wind_down(shared: &Arc<Shared>, id: u64, state: TaskState, error: String, tok: u64) {
    if control_of(shared, id) != Control::Run {
        wind_down_stopped(shared, id, &state, None, tok);
    } else {
        fail_task(shared, id, state, error, tok);
    }
}

/// Free bytes available on the volume holding `dir`
/// (GetDiskFreeSpaceExW's caller-available figure).
/// `None` when the query is unavailable — callers treat
/// that as "no information" and proceed, exactly as 0.32
/// did; the check exists for the known-huge claim and the
/// unbounded stream, not as a new failure mode.
#[cfg(windows)]
fn free_disk_bytes(dir: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut free: u64 = 0;
    // SAFETY: the path buffer is NUL-terminated and outlives
    // the call; the out-pointer is a live local.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok != 0 {
        Some(free)
    } else {
        None
    }
}

#[cfg(not(windows))]
fn free_disk_bytes(_dir: &Path) -> Option<u64> {
    None
}

/// Validator-free resume spot-check (AUDIT 0.32.1, P1-5):
/// when a task resumes with progress but NEITHER side
/// produced a validator to compare, total-size equality is
/// the only identity check HTTP offers — and a same-size
/// replacement would splice silently. Before trusting the
/// partial file, re-fetch its first and last downloaded
/// chunks (≤ 4 KiB each) and compare them against the
/// `.part` on disk. Any mismatch, short read or transport
/// failure answers "do not trust" and the caller restarts
/// the task from zero. (A change confined to the un-sampled
/// middle remains undetectable — that is precisely what
/// validators exist for; this bounds the silent case.)
fn resume_spot_check(shared: &Arc<Shared>, id: u64, state: &TaskState, part: &Path) -> bool {
    use std::io::Read;
    let downloaded = state.downloaded();
    if downloaded == 0 {
        return true;
    }
    let cookies = state.cookies.clone();
    let cookie_for = |u: &str| cookie_header(&cookies, u);
    let stop = || control_of(shared, id) != Control::Run;
    let fetch_chunk = |start: u64, end_incl: u64| -> Option<Vec<u8>> {
        let mut buf: Vec<u8> = Vec::new();
        let mut sink = |bytes: &[u8]| -> Result<(), String> {
            buf.extend_from_slice(bytes);
            Ok(())
        };
        match winhttp::fetch_range(
            &state.url,
            Some((start, end_incl)),
            None,
            state.total,
            &cookie_for,
            state.referer.as_deref(),
            &stop,
            &mut sink,
        ) {
            Ok(out) if out.status == 206 => Some(buf),
            _ => None,
        }
    };
    let read_disk = |start: u64, len: u64| -> Option<Vec<u8>> {
        let mut f = std::fs::File::open(part).ok()?;
        f.seek(SeekFrom::Start(start)).ok()?;
        let mut buf = vec![0u8; usize::try_from(len).ok()?];
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let probe_len = downloaded.min(4096);
    let ranges = [
        (0u64, probe_len - 1),
        (downloaded - probe_len, downloaded - 1),
    ];
    for (start, end_incl) in ranges {
        let len = end_incl - start + 1;
        let (Some(wire), Some(disk)) = (fetch_chunk(start, end_incl), read_disk(start, len)) else {
            return false;
        };
        if wire.len() as u64 != len || wire != disk {
            return false;
        }
    }
    true
}

fn fail_task(shared: &Arc<Shared>, id: u64, state: TaskState, error: String, tok: u64) {
    fail_task_with_class(shared, id, state, error, None, tok);
}

/// Fail a task, with the 0.33 auto-retry policy applied
/// first. `class_override` lets a call site that already
/// knows the honest class force it (the supervisor's
/// post-restart range death is `Other`, not `RangeLost` —
/// it already spent its one restart in-run).
///
/// Policy (the core's `retry_policy` over `classify_failure`):
/// a retryable class parks the task as Failed with a
/// `retry_not_before` deadline and a timer thread that
/// promotes it back to Queued when the deadline passes;
/// `FailNow` falls through to the original failure body,
/// plus the class's plain-language hint.
fn fail_task_with_class(
    shared: &Arc<Shared>,
    id: u64,
    state: TaskState,
    error: String,
    class_override: Option<FailureClass>,
    tok: u64,
) {
    // AUDIT 0.32.2 (Fix 2): a stale run fails nothing — the
    // manager (or a newer run) owns the task now.
    if !run_current(shared, id, tok) {
        return;
    }
    let host = url_host(&state.url).unwrap_or_else(|| "unknown host".to_string());
    let mut state = state;
    preserve_live_connections(shared, id, &mut state);
    // ---- 0.33 auto-retry gate: before the failure body. ----
    let class = class_override.unwrap_or_else(|| classify_failure(&error));
    let policy = retry_policy(class, state.auto_retries);
    match policy {
        RetryPolicy::Retry { .. } | RetryPolicy::RestartOnce => {
            let restart = matches!(policy, RetryPolicy::RestartOnce);
            let after_ms = match policy {
                RetryPolicy::Retry { after_ms } => after_ms,
                // RestartOnce uses the first backoff step.
                _ => AUTO_RETRY_BACKOFF_MS[0],
            };
            state.auto_retries = state.auto_retries.saturating_add(1);
            let attempt = state.auto_retries;
            state.error = Some(error.clone());
            state.error_hint = None;
            state.status = TaskStatus::Failed;
            let retry_not_before = now_ms().saturating_add(after_ms);
            {
                let mut inner = lock(&shared.inner);
                if let Some(rt) = inner.tasks.get_mut(&id) {
                    rt.state = state.clone();
                    rt.live_planner = None;
                    rt.live_downloaded = state.downloaded();
                    rt.supervisor = None;
                    rt.retry_not_before = retry_not_before;
                    rt.retry_restart = restart;
                }
            }
            persist_task(shared, &state);
            crate::log::info(&format!(
                "download failed: {host}: {error} (class {}, auto-retry attempt {attempt} of {AUTO_RETRY_MAX} in {after_ms} ms)",
                class.label()
            ));
            bump_and_notify(shared);
            schedule(shared);
            spawn_retry_timer(shared.clone(), id, attempt, retry_not_before);
            return;
        }
        RetryPolicy::FailNow => {}
    }
    state.status = TaskStatus::Failed;
    state.error = Some(error.clone());
    state.error_hint = failure_hint(class).map(|h| h.to_string());
    {
        let mut inner = lock(&shared.inner);
        if let Some(rt) = inner.tasks.get_mut(&id) {
            rt.state = state.clone();
            rt.live_planner = None;
            rt.live_downloaded = state.downloaded();
            rt.supervisor = None;
        }
    }
    persist_task(shared, &state);
    crate::log::info(&format!("download failed: {host}: {error}"));
    bump_and_notify(shared);
    schedule(shared);
}

/// The auto-retry timer (0.33): sleeps until the deadline in
/// ≤ 250 ms slices — waking early when the task vanishes —
/// then promotes the task. No lock is held across a sleep;
/// `promote_retry` re-validates everything under the lock,
/// so a pause / resume / cancel that landed meanwhile wins.
fn spawn_retry_timer(shared: Arc<Shared>, id: u64, expected_auto_retries: u32, deadline_ms: u64) {
    std::thread::spawn(move || {
        loop {
            let now = now_ms();
            if now >= deadline_ms {
                break;
            }
            {
                let inner = lock(&shared.inner);
                if !inner.tasks.contains_key(&id) {
                    // Woken early: the task is gone, so
                    // there is nothing to promote.
                    return;
                }
            }
            let step = deadline_ms.saturating_sub(now).min(250);
            std::thread::sleep(Duration::from_millis(step));
        }
        promote_retry(&shared, id, expected_auto_retries);
    });
}

/// Promote a Failed task whose auto-retry deadline passed
/// back to Queued (0.33). Every condition is re-checked
/// under the lock — task exists, still Failed, a retry is
/// still pending, and the retry generation still matches
/// `expected` — and any mismatch is a quiet no-op: a pause,
/// a manual resume, or a newer failure owns the task now.
/// A restart-retry clears the segment map and validators
/// under the lock and removes the `.part` outside it.
fn promote_retry(shared: &Arc<Shared>, id: u64, expected: u32) {
    let mut work: Option<(TaskState, Option<PathBuf>)> = None;
    {
        let mut inner = lock(&shared.inner);
        let downloads_dir = inner.downloads_dir.clone();
        if let Some(rt) = inner.tasks.get_mut(&id) {
            if matches!(rt.state.status, TaskStatus::Failed)
                && rt.retry_not_before != 0
                && rt.state.auto_retries == expected
            {
                let part = if rt.retry_restart {
                    let part = part_path(&downloads_dir, &rt.state.file_name);
                    rt.state.segments.clear();
                    rt.state.total = None;
                    rt.state.etag = None;
                    rt.state.last_modified = None;
                    Some(part)
                } else {
                    None
                };
                rt.state.status = TaskStatus::Queued;
                rt.state.error = None;
                rt.retry_not_before = 0;
                rt.retry_restart = false;
                rt.live_downloaded = rt.state.downloaded();
                rt.control = Control::Run;
                work = Some((rt.state.clone(), part));
            }
        }
    }
    let Some((state, part)) = work else {
        return;
    };
    if let Some(part) = part {
        let _ = std::fs::remove_file(part);
    }
    persist_task(shared, &state);
    log_host(shared, id, "download auto-retry queued");
    bump_and_notify(shared);
    schedule(shared);
}

/// The `refresh_link` worker thread (0.33): probe the new
/// URL (no lock held), then apply the outcome under the
/// lock. The probe's stop predicate is the task-vanished
/// check — a removed task stops the probe and makes every
/// completion below a no-op. The file name never changes.
fn refresh_link_thread(
    shared: Arc<Shared>,
    id: u64,
    snapshot: TaskState,
    cookies: Vec<CookiePair>,
    new_url: String,
) {
    let cookie_for = |u: &str| cookie_header(&cookies, u);
    let stop_shared = shared.clone();
    let stop = move || {
        let inner = lock(&stop_shared.inner);
        !inner.tasks.contains_key(&id)
    };
    let probe = match winhttp::probe(&new_url, &cookie_for, snapshot.referer.as_deref(), &stop) {
        Ok(p) => p,
        Err(FetchError::Stopped) => {
            // The task vanished mid-probe (or is going): just
            // clear the flag if it somehow still exists.
            let mut inner = lock(&shared.inner);
            if let Some(rt) = inner.tasks.get_mut(&id) {
                rt.refreshing = false;
            }
            return;
        }
        Err(FetchError::Failed(e)) => {
            refresh_link_failed(&shared, id, &snapshot, &e);
            return;
        }
    };
    if !(200..400).contains(&probe.facts.status) {
        refresh_link_failed(
            &shared,
            id,
            &snapshot,
            &format!("HTTP {}", probe.facts.status),
        );
        return;
    }
    let decision = refresh_decision(&snapshot, &probe.facts);
    let mut work: Option<(TaskState, Option<PathBuf>)> = None;
    let mut dropped = false;
    {
        let mut inner = lock(&shared.inner);
        let downloads_dir = inner.downloads_dir.clone();
        if let Some(rt) = inner.tasks.get_mut(&id) {
            // AUDIT 0.40.1: revalidate before applying. Through
            // 0.35.1 the probe's outcome was written over
            // whatever the task had become meanwhile — a pause
            // or restart landing mid-probe was overridden and
            // the task silently re-queued. The apply now only
            // lands when the task is still exactly the one the
            // probe was taken from (same refresh in flight,
            // same URL, same status); otherwise the outcome is
            // dropped and the current owner keeps the task.
            if !rt.refreshing || rt.state.url != snapshot.url || rt.state.status != snapshot.status
            {
                rt.refreshing = false;
                dropped = true;
            } else {
                rt.state.url = new_url.clone();
                rt.state.total = probe.facts.total;
                rt.state.etag = probe.facts.etag.clone();
                rt.state.last_modified = probe.facts.last_modified.clone();
                let part = if matches!(decision, RefreshDecision::RebindRestart) {
                    rt.state.segments.clear();
                    Some(part_path(&downloads_dir, &rt.state.file_name))
                } else {
                    None
                };
                rt.state.status = TaskStatus::Queued;
                rt.state.error = None;
                rt.state.error_hint = None;
                rt.state.auto_retries = 0;
                rt.state.sha256 = None;
                rt.retry_not_before = 0;
                rt.retry_restart = false;
                rt.refreshing = false;
                rt.live_downloaded = rt.state.downloaded();
                rt.control = Control::Run;
                work = Some((rt.state.clone(), part));
            }
        }
    }
    if dropped {
        log_host(&shared, id, "download link refresh dropped (task changed)");
        bump_and_notify(&shared);
        return;
    }
    let Some((state, part)) = work else {
        return;
    };
    if let Some(part) = part {
        let _ = std::fs::remove_file(part);
    }
    persist_task(&shared, &state);
    log_host(&shared, id, "download link refreshed");
    bump_and_notify(&shared);
    schedule(&shared);
}

/// A `refresh_link` probe failed (0.33): the task keeps its
/// URL and its status, gains the honest error, and the
/// refreshing flag clears — persisted + notified.
fn refresh_link_failed(shared: &Arc<Shared>, id: u64, snapshot: &TaskState, reason: &str) {
    let mut failed: Option<TaskState> = None;
    {
        let mut inner = lock(&shared.inner);
        if let Some(rt) = inner.tasks.get_mut(&id) {
            // AUDIT 0.40.1: same revalidation as the success
            // apply — a failure is only painted onto the task
            // the probe was taken from; the in-flight flag is
            // always released (it is this probe's own).
            let still_ours =
                rt.refreshing && rt.state.url == snapshot.url && rt.state.status == snapshot.status;
            rt.refreshing = false;
            if still_ours {
                rt.state.error = Some(format!("refresh failed: {reason}"));
                failed = Some(rt.state.clone());
            }
        }
    }
    if let Some(state) = failed {
        persist_task(shared, &state);
        log_host(shared, id, "download link refresh failed");
        bump_and_notify(shared);
    }
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// One task's supervisor thread: probe → resume decision →
/// plan/preallocate → workers → finalize (complete / pause /
/// cancel / fail / restart-as-single-stream, at most once).
fn run_task(shared: Arc<Shared>, id: u64, tok: u64) {
    let snapshot = {
        let inner = lock(&shared.inner);
        inner.tasks.get(&id).map(|rt| rt.state.clone())
    };
    let Some(mut state) = snapshot else {
        return;
    };
    if control_of(&shared, id) != Control::Run {
        wind_down_stopped(&shared, id, &state, None, tok);
        return;
    }
    let cookies = state.cookies.clone();
    let cookie_for = |u: &str| cookie_header(&cookies, u);
    // The probe honors pause/cancel between hops (P1-13).
    let probe_stop = || control_of(&shared, id) != Control::Run;

    // ---- Probe. ----
    let probe = match winhttp::probe(
        &state.url,
        &cookie_for,
        state.referer.as_deref(),
        &probe_stop,
    ) {
        Ok(p) => p,
        Err(FetchError::Stopped) => {
            wind_down_stopped(&shared, id, &state, None, tok);
            return;
        }
        Err(FetchError::Failed(e)) => {
            // A stop that landed during the probe wins over
            // the failure (P1-9).
            fail_or_wind_down(&shared, id, state, e, tok);
            return;
        }
    };
    if control_of(&shared, id) != Control::Run {
        wind_down_stopped(&shared, id, &state, None, tok);
        return;
    }
    if !(200..400).contains(&probe.facts.status) {
        fail_or_wind_down(
            &shared,
            id,
            state,
            format!("HTTP {}", probe.facts.status),
            tok,
        );
        return;
    }

    // ---- Filename: the disposition name replaces the locked
    // name ONLY when the task has never started. ----
    let downloads_dir = lock(&shared.inner).downloads_dir.clone();
    let part = part_path(&downloads_dir, &state.file_name);
    if state.downloaded() == 0 && !part.exists() {
        if let Some(raw) = &probe.disposition_name {
            let candidate = sanitize_file_name(raw);
            if candidate != state.file_name {
                // AUDIT 0.32.1 (P1-3): the dedupe AND the
                // claim land in the SAME lock hold — the
                // winner is written into the task's shared
                // state before the lock drops. Through 0.32
                // the renamed name reached shared state only
                // after preallocation, so two supervisors
                // probing the same disposition name in the
                // same window both claimed it and downloaded
                // into ONE shared .part file.
                let mut inner = lock(&shared.inner);
                let renamed = dedupe_file_name(&candidate, |cand| {
                    downloads_dir.join(cand).exists()
                        || downloads_dir.join(format!("{cand}.part")).exists()
                        || inner
                            .tasks
                            .values()
                            .any(|t| t.state.id != id && t.state.file_name == cand)
                });
                // AUDIT 0.40.1: dedupe exhaustion returns a
                // still-taken name — keep the current name
                // rather than claim a colliding one.
                let renamed = if downloads_dir.join(&renamed).exists()
                    || downloads_dir.join(format!("{renamed}.part")).exists()
                    || inner
                        .tasks
                        .values()
                        .any(|t| t.state.id != id && t.state.file_name == renamed)
                {
                    state.file_name.clone()
                } else {
                    renamed
                };
                if let Some(rt) = inner.tasks.get_mut(&id) {
                    rt.state.file_name = renamed.clone();
                }
                drop(inner);
                state.file_name = renamed;
            }
        }
    }
    let part = part_path(&downloads_dir, &state.file_name);

    // ---- Resume decision + plan. ----
    let mut decision = resume_decision(&state, &probe.facts);
    let probed_total = probe.facts.total;
    // AUDIT 0.32.1 (P1-5): a resume with progress but no
    // validator shared by both sides rests on total-size
    // equality alone. Spot-check the partial file's edges
    // against the wire before trusting it; a mismatch is a
    // restart, never a splice.
    if matches!(decision, ResumeDecision::Resume) && state.downloaded() > 0 {
        let validators_shared = (state.etag.is_some() && probe.facts.etag.is_some())
            || (state.last_modified.is_some() && probe.facts.last_modified.is_some());
        if !validators_shared && !resume_spot_check(&shared, id, &state, &part) {
            if control_of(&shared, id) != Control::Run {
                wind_down_stopped(&shared, id, &state, None, tok);
                return;
            }
            crate::log::info(&format!(
                "download resume spot-check failed, restarting: {}",
                url_host(&state.url).unwrap_or_else(|| "unknown host".to_string())
            ));
            decision = ResumeDecision::Restart;
        }
    }
    // AUDIT 0.40.1: the .part's actual length was never
    // checked on the Resume path. A part truncated (or
    // extended) on disk between pause and resume was
    // `set_len`'d back to the total below and its segment map
    // trusted — splicing zeros into the finished file when a
    // shared validator had already waived the spot check
    // above. A length that differs from the probed total is a
    // restart, never a splice. (A MISSING part is the
    // preallocate step's own case: it replans the map there.)
    if matches!(decision, ResumeDecision::Resume) && state.downloaded() > 0 {
        if let Some(total) = probed_total {
            let part_len = std::fs::metadata(&part).map(|m| m.len()).ok();
            if part_len.is_some_and(|len| len != total) {
                if control_of(&shared, id) != Control::Run {
                    wind_down_stopped(&shared, id, &state, None, tok);
                    return;
                }
                crate::log::info(&format!(
                    "download resume part length mismatch, restarting: {}",
                    url_host(&state.url).unwrap_or_else(|| "unknown host".to_string())
                ));
                decision = ResumeDecision::Restart;
            }
        }
    }
    if matches!(decision, ResumeDecision::Restart) {
        let _ = std::fs::remove_file(&part);
        state.segments = Vec::new();
    }
    // Validators and total track the probe: on Resume they
    // matched by the decision's own rules; on Fresh/Restart
    // they ARE the file's identity from now on.
    state.total = probed_total;
    state.etag = probe.facts.etag.clone();
    state.last_modified = probe.facts.last_modified.clone();
    let single = !probe.facts.accept_ranges || probed_total.is_none();
    if single {
        // Single-stream by construction: a rangeless or
        // unknown-total transfer starts at zero (the core's
        // resume rules have already forced Restart whenever
        // prior progress existed in these shapes).
        state.segments = vec![Segment {
            start: 0,
            end: probed_total.unwrap_or(0),
            done: 0,
        }];
    } else if !matches!(decision, ResumeDecision::Resume) {
        state.segments = plan_segments(
            probed_total.unwrap_or(0),
            clamp_connections(state.connections),
        );
    }
    // (Resume + ranged: the stored segments ride as-is —
    // except the connection rule: a count changed while the
    // task was paused re-splits its not-yet-started ranges
    // now, before the phase builds its planner from them.
    // AUDIT 0.32.1, P4-A5: through 0.32 the stored map rode
    // verbatim, so a 2→32 change resumed onto a 2-piece map
    // and the new workers could only steal slivers.)
    if matches!(decision, ResumeDecision::Resume) && !single {
        let mut planner = Planner::from_task(&state);
        planner.replan_pending(clamp_connections(state.connections));
        planner.sync_into(&mut state);
    }

    // ---- Preallocate / verify the .part file. ----
    if let Some(total) = probed_total {
        // AUDIT 0.32.1 (P1-2): the claimed total is
        // server-controlled and nothing used to bound it — a
        // 500 GB claim preallocated (and then progressively
        // filled) on whatever volume Pico happened to sit
        // on. The disk itself is the cap: when the volume's
        // free space is knowable and the file's growth past
        // its current on-disk size exceeds it, fail BEFORE
        // set_len with an honest error.
        if let Some(free) = free_disk_bytes(&downloads_dir) {
            let on_disk = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if total.saturating_sub(on_disk) > free {
                fail_or_wind_down(
                    &shared,
                    id,
                    state,
                    format!(
                        "not enough disk space: the file needs {} more, {} free",
                        humanize_bytes(total.saturating_sub(on_disk)),
                        humanize_bytes(free)
                    ),
                    tok,
                );
                return;
            }
        }
        if matches!(decision, ResumeDecision::Resume) && !part.exists() {
            // The progress map claims bytes the disk no longer
            // has: restart honestly rather than splice zeros.
            state.segments = plan_segments(total, clamp_connections(state.connections));
        }
        let sized = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part)
            .and_then(|f| f.set_len(total));
        if let Err(e) = sized {
            fail_or_wind_down(&shared, id, state, format!("cannot prepare file: {e}"), tok);
            return;
        }
    } else {
        // Unknown total: a fresh single stream truncates any
        // stale .part (its bytes are unclaimable by rule).
        let created = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&part);
        if let Err(e) = created {
            fail_or_wind_down(&shared, id, state, format!("cannot prepare file: {e}"), tok);
            return;
        }
    }
    // AUDIT 0.32.2 (Fix 2): a pause/cancel that landed during
    // the probe or preallocation owns the task now — this
    // run writes nothing more, not even its starting state.
    if !run_current(&shared, id, tok) {
        return;
    }
    preserve_live_connections(&shared, id, &mut state);
    {
        let mut inner = lock(&shared.inner);
        if let Some(rt) = inner.tasks.get_mut(&id) {
            rt.state = state.clone();
            rt.live_downloaded = state.downloaded();
        }
    }
    persist_task(&shared, &state);
    log_host(&shared, id, "download started");

    // ---- Worker phases (the loop runs twice at most: the
    // restart-once-as-single-stream path re-enters it). ----
    let mut force_single = single;
    let mut restarted = false;
    loop {
        let phase = run_phase(&shared, id, &state, force_single, &part, tok);
        match phase {
            PhaseEnd::Finished(planned_state) => {
                state = planned_state;
                break;
            }
            PhaseEnd::Stopped(planned_state) => {
                wind_down_stopped(&shared, id, &planned_state, None, tok);
                return;
            }
            PhaseEnd::AlreadyFinalized => {
                return;
            }
            PhaseEnd::RangeDied(planned_state) => {
                state = planned_state;
                if restarted {
                    // 0.33: this range death already spent
                    // its one in-run single-stream restart, so
                    // it must NOT classify as RangeLost (whose
                    // policy would grant another restart) —
                    // the class is forced to Other (FailNow).
                    fail_task_with_class(
                        &shared,
                        id,
                        state,
                        "server stopped honoring ranges".to_string(),
                        Some(FailureClass::Other),
                        tok,
                    );
                    return;
                }
                if force_single {
                    fail_task(
                        &shared,
                        id,
                        state,
                        "server stopped honoring ranges".to_string(),
                        tok,
                    );
                    return;
                }
                restarted = true;
                force_single = true;
                // Restart as ONE stream from zero: truncate the
                // .part, fresh 1-segment plan (total is known —
                // ranged mode implies it).
                let total = state.total.unwrap_or(0);
                let truncated = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&part)
                    .and_then(|f| f.set_len(total));
                if let Err(e) = truncated {
                    fail_or_wind_down(&shared, id, state, format!("cannot prepare file: {e}"), tok);
                    return;
                }
                state.segments = vec![Segment {
                    start: 0,
                    end: total,
                    done: 0,
                }];
                preserve_live_connections(&shared, id, &mut state);
                {
                    let mut inner = lock(&shared.inner);
                    if let Some(rt) = inner.tasks.get_mut(&id) {
                        rt.state = state.clone();
                        rt.live_downloaded = 0;
                    }
                }
                persist_task(&shared, &state);
            }
        }
    }

    finalize_complete(&shared, id, state, &part, tok);
}

/// How one worker phase ended.
enum PhaseEnd {
    /// All workers exited with the control still at Run and no
    /// terminal report: the planner decides (complete or not).
    Finished(TaskState),
    /// Pause/Cancel was requested; the carried state is the
    /// one to persist (planner-synced — or a zeroed plan when
    /// a range death tainted the phase).
    Stopped(TaskState),
    /// A ranged request was answered as a whole-file 200.
    RangeDied(TaskState),
    /// A worker failure already finalized the task as Failed
    /// inside run_phase; the supervisor has nothing left to do.
    AlreadyFinalized,
}

/// Run one phase: build the planner from `state`, register it
/// live, spawn the workers, tick progress until they exit,
/// then fold the planner back into the returned state.
///
/// The worker POOL is dynamic (AUDIT 0.32.1, P4-A4): a
/// connection-count change mid-phase re-plans the pending
/// ranges AND resizes the pool — the tick spawns workers up
/// to the new count, and surplus workers retire at their
/// next claim through retire tokens. Through 0.32 the pool
/// was fixed at phase start, so a 1→32 change only re-split
/// ranges for a single worker to fetch one by one.
fn run_phase(
    shared: &Arc<Shared>,
    id: u64,
    state: &TaskState,
    single: bool,
    part: &Path,
    tok: u64,
) -> PhaseEnd {
    // An unknown-total single stream runs on an OPEN-ENDED
    // planner: its marker segment is claimable exactly once
    // (P4-A1). Every other shape uses the closed planner.
    let planner = Arc::new(Mutex::new(if single && state.total.is_none() {
        Planner::new_open_ended(state.segments.clone())
    } else {
        Planner::new(state.segments.clone())
    }));
    // AUDIT 0.32.2 (Fix 2): registering the live planner is a
    // shared-state write, and it is gated on this run still
    // being current — a task the manager retired (or one
    // that vanished) starts no phase at all. The workers'
    // generation view (token + the task's live_gen mirror)
    // is read from the same runtime in the same lock hold.
    let live_gen = {
        let mut inner = lock(&shared.inner);
        match inner.tasks.get_mut(&id) {
            Some(rt) if rt.gate.accepts(tok) => {
                rt.live_planner = Some(planner.clone());
                rt.live_downloaded = lock(&planner).downloaded();
                rt.live_gen.clone()
            }
            _ => return PhaseEnd::AlreadyFinalized,
        }
    };
    let report = Arc::new(Mutex::new(WorkerReport::default()));
    let ctx = Arc::new(WorkerCtx {
        url: state.url.clone(),
        cookies: Arc::new(state.cookies.clone()),
        referer: state.referer.clone(),
        bucket: shared.bucket.clone(),
        validator: state.etag.clone().or_else(|| state.last_modified.clone()),
        validator_is_etag: state.etag.is_some(),
        part_path: part.to_path_buf(),
        single,
        total: state.total,
        gen_token: tok,
        live_gen,
    });
    let worker_count = if single {
        1
    } else {
        clamp_connections(state.connections) as usize
    };
    let alive = Arc::new(AtomicUsize::new(0));
    let retire = Arc::new(AtomicUsize::new(0));
    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    for _ in 0..worker_count {
        handles.push(spawn_worker(
            shared, id, &planner, &ctx, &report, &alive, &retire,
        ));
    }
    // Tick: sample speed, mirror live progress, apply pending
    // replans (and the pool resizes that ride them), throttle
    // notifications to one per TICK_MS.
    let mut last_notify = now_ms();
    while alive.load(Ordering::SeqCst) > 0 {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        let downloaded = lock(&planner).downloaded();
        let replan = {
            let mut inner = lock(&shared.inner);
            if let Some(rt) = inner.tasks.get_mut(&id) {
                rt.speed.record(now_ms(), downloaded);
                rt.live_downloaded = downloaded;
                rt.replan_request.take()
            } else {
                None
            }
        };
        if let Some(n) = replan {
            if !single {
                lock(&planner).replan_pending(n);
                let want = clamp_connections(n) as usize;
                let alive_now = alive.load(Ordering::SeqCst);
                if alive_now < want {
                    // Growing: cancel any pending retirements
                    // first, then spawn the difference.
                    retire.store(0, Ordering::SeqCst);
                    for _ in 0..(want - alive_now) {
                        handles.push(spawn_worker(
                            shared, id, &planner, &ctx, &report, &alive, &retire,
                        ));
                    }
                } else if alive_now > want {
                    retire.store(alive_now - want, Ordering::SeqCst);
                }
            }
        }
        let now = now_ms();
        if now.saturating_sub(last_notify) >= TICK_MS {
            last_notify = now;
            bump_and_notify(shared);
        }
    }
    for handle in handles {
        let _ = handle.join();
    }
    // Fold the planner back into the task state. (Since the
    // transport's pre-body judgment, progress only ever
    // counts bytes of a validated response — there is no
    // untrusted-tail rewind left to apply.)
    let mut out = state.clone();
    lock(&planner).sync_into(&mut out);
    // AUDIT 0.32.2 (Fix 2): the fold-back is gated too — when
    // this run went stale mid-phase, the manager's snapshot
    // (or a newer run) is the final word and this block must
    // not touch shared state at all.
    if !run_current(shared, id, tok) {
        return PhaseEnd::AlreadyFinalized;
    }
    preserve_live_connections(shared, id, &mut out);
    {
        let mut inner = lock(&shared.inner);
        if let Some(rt) = inner.tasks.get_mut(&id) {
            rt.live_planner = None;
            rt.live_downloaded = out.downloaded();
            rt.state = out.clone();
        }
    }
    let report_failed = lock(&report).failed.clone();
    let range_died = lock(&report).range_died;
    if let Some(error) = report_failed {
        // A worker failure finalizes as Failed regardless of a
        // racing pause: the error is the news the user needs,
        // and resume re-enters cleanly from the rewound state.
        fail_task(shared, id, out, error, tok);
        return PhaseEnd::AlreadyFinalized;
    }
    if range_died {
        if control_of(shared, id) == Control::Run {
            return PhaseEnd::RangeDied(out);
        }
        // Paused/cancelled while a 200-on-ranged was in flight:
        // other segments may hold shifted bytes from the
        // whole-file body, so NO progress from this phase is
        // trustworthy. Persist a zeroed fresh plan instead of
        // spliced progress (the .part is re-cut on resume).
        let mut zeroed = out.clone();
        if let Some(total) = zeroed.total {
            zeroed.segments = plan_segments(total, clamp_connections(zeroed.connections));
        } else {
            zeroed.segments = vec![Segment {
                start: 0,
                end: 0,
                done: 0,
            }];
        }
        return PhaseEnd::Stopped(zeroed);
    }
    if control_of(shared, id) != Control::Run {
        return PhaseEnd::Stopped(out);
    }
    PhaseEnd::Finished(out)
}

/// Spawn one pool worker; `alive` is incremented before the
/// spawn and decremented by the wrapper when the worker
/// exits, so the supervisor's tick always sees the true
/// pool size.
#[allow(clippy::too_many_arguments)]
fn spawn_worker(
    shared: &Arc<Shared>,
    id: u64,
    planner: &Arc<Mutex<Planner>>,
    ctx: &Arc<WorkerCtx>,
    report: &Arc<Mutex<WorkerReport>>,
    alive: &Arc<AtomicUsize>,
    retire: &Arc<AtomicUsize>,
) -> JoinHandle<()> {
    alive.fetch_add(1, Ordering::SeqCst);
    let (sh, pl, rp, cx, al, rt) = (
        shared.clone(),
        planner.clone(),
        report.clone(),
        ctx.clone(),
        alive.clone(),
        retire.clone(),
    );
    std::thread::spawn(move || {
        worker_loop(sh, id, pl, cx, rp, rt);
        al.fetch_sub(1, Ordering::SeqCst);
    })
}

/// Pause/Cancel finalization from any supervisor point. Pause:
/// persist the (planner-synced when available) state as Paused.
/// Cancel: delete `.part` + state file and remove the record.
fn wind_down_stopped(
    shared: &Arc<Shared>,
    id: u64,
    state: &TaskState,
    planner: Option<&Arc<Mutex<Planner>>>,
    tok: u64,
) {
    // AUDIT 0.32.2 (Fix 2): FIRST LINE, before anything is
    // read or written — a stale run winds down nothing. The
    // manager action that retired this run has already
    // persisted the task's final word (or removed it).
    if !run_current(shared, id, tok) {
        return;
    }
    let control = control_of(shared, id);
    let downloads_dir = lock(&shared.inner).downloads_dir.clone();
    let mut state = state.clone();
    if let Some(planner) = planner {
        lock(planner).sync_into(&mut state);
    }
    preserve_live_connections(shared, id, &mut state);
    match control {
        Control::Cancel => {
            {
                let mut inner = lock(&shared.inner);
                inner.tasks.remove(&id);
                inner.order.retain(|t| *t != id);
            }
            let _ = std::fs::remove_file(part_path(&downloads_dir, &state.file_name));
            let state_dir = lock(&shared.inner).state_dir.clone();
            delete_task_file(&state_dir, id);
            let host = url_host(&state.url).unwrap_or_else(|| "unknown host".to_string());
            crate::log::info(&format!("download cancelled: {host}"));
            bump_and_notify(shared);
            schedule(shared);
        }
        Control::Pause => {
            state.status = TaskStatus::Paused;
            {
                let mut inner = lock(&shared.inner);
                if let Some(rt) = inner.tasks.get_mut(&id) {
                    rt.state = state.clone();
                    rt.live_planner = None;
                    rt.live_downloaded = state.downloaded();
                    rt.supervisor = None;
                }
            }
            persist_task(shared, &state);
            log_host(shared, id, "download paused");
            bump_and_notify(shared);
            schedule(shared);
        }
        Control::Run => {
            // 0.35 (WS3): a wind-down reached with the
            // control still at Run is a structural anomaly
            // — through 0.34 the catch-all arm treated it
            // as Pause and parked a task nobody paused. It
            // is unreachable today (every Pause/Cancel
            // writer invalidates the generation gate in the
            // same lock hold, and the gate check above
            // returns for any run those writers retired),
            // but the arm now says so in the log and writes
            // NOTHING, instead of silently writing Paused.
            let host = url_host(&state.url).unwrap_or_else(|| "unknown host".to_string());
            crate::log::info(&format!(
                "download wind-down reached with control=Run (no status written): {host}"
            ));
        }
    }
}

/// Completion: the planner says every byte is in (the caller
/// has verified the counts). Rename `.part` to the final name
/// — re-deduped at rename time against the folder — then mark
/// Complete and persist.
fn finalize_complete(shared: &Arc<Shared>, id: u64, mut state: TaskState, part: &Path, tok: u64) {
    // AUDIT 0.32.2 (Fix 2): a stale run completes nothing —
    // the gate comes before even the control check below.
    if !run_current(shared, id, tok) {
        return;
    }
    // AUDIT 0.32.1 (P1-9): the cancel/pause check happens
    // HERE, at the rename gate — through 0.32 a cancel that
    // landed after the supervisor's last control check but
    // before the rename was silently discarded: the download
    // completed and renamed anyway, and a cancel-then-remove
    // flow then deleted the COMPLETED file.
    if control_of(shared, id) != Control::Run {
        wind_down_stopped(shared, id, &state, None, tok);
        return;
    }
    preserve_live_connections(shared, id, &mut state);
    let downloaded = state.downloaded();
    if let Some(total) = state.total {
        if downloaded != total || !lock_planner_finished(&state) {
            fail_task(shared, id, state, "size mismatch".to_string(), tok);
            return;
        }
    }
    let downloads_dir = lock(&shared.inner).downloads_dir.clone();
    // Re-dedupe at rename time against the folder: the locked
    // name was free at ADD time, but a same-named file may have
    // landed since (another task's completion is already
    // excluded by the tasks' own name dedupe; this covers
    // files that arrived from outside the manager). Our own
    // final file does not exist yet — only its `.part` does —
    // so a plain existence check is honest.
    let final_name = dedupe_file_name(&state.file_name, |cand| downloads_dir.join(cand).exists());
    let final_path = downloads_dir.join(&final_name);
    // AUDIT 0.40.1: `dedupe_file_name` returns its last tried
    // name when every candidate is taken (the ~100k-collision
    // exhaustion path) — renaming onto it would REPLACE a
    // different file. A still-existing candidate here can only
    // be that exhaustion case, so completion fails honestly
    // instead of overwriting.
    if final_path.exists() {
        fail_task(
            shared,
            id,
            state,
            "cannot move finished file: too many files with this name".to_string(),
            tok,
        );
        return;
    }
    if let Err(e) = std::fs::rename(part, &final_path) {
        fail_task(
            shared,
            id,
            state,
            format!("cannot move finished file: {e}"),
            tok,
        );
        return;
    }
    state.file_name = final_name;
    // 0.33: SHA-256 of the finished file, streamed in
    // 64 KiB chunks AFTER the rename and BEFORE the persist
    // below, so the persisted Complete record already
    // carries it. Any io error leaves `sha256` as None and
    // completion still stands — the hash is a courtesy for
    // the page's copy button, never a completion gate.
    state.sha256 = sha256_of_file(&final_path);
    if state.sha256.is_none() {
        crate::log::error(&format!(
            "download sha256 failed (task {id}): finished file could not be hashed"
        ));
    }
    if state.total.is_none() {
        // Unknown-total stream: the downloaded count IS the
        // size from here on (the view's done line uses it too).
        state.total = Some(downloaded);
    }
    state.status = TaskStatus::Complete;
    state.error = None;
    state.error_hint = None;
    state.auto_retries = 0;
    // AUDIT 0.40.1: a completed task's session cookies are
    // never used again (refresh-link accepts only Failed /
    // Paused / Queued), so they stop being persisted here
    // instead of overstaying in the plaintext task file for
    // the record's whole lifetime.
    state.cookies.clear();
    let size = humanize_bytes(state.total.unwrap_or(downloaded));
    let name = state.file_name.clone();
    {
        let mut inner = lock(&shared.inner);
        if let Some(rt) = inner.tasks.get_mut(&id) {
            rt.state = state.clone();
            rt.live_planner = None;
            rt.live_downloaded = state.downloaded();
            rt.supervisor = None;
        }
    }
    persist_task(shared, &state);
    crate::log::info(&format!("download completed: {name} ({size})"));
    bump_and_notify(shared);
    schedule(shared);
}

/// Stream a finished file through SHA-256 in 64 KiB chunks
/// (0.33). `None` on any io error — the caller logs it and
/// completion stands without a hash.
fn sha256_of_file(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize_hex())
}

/// Did the state's own segment map finish? (The supervisor's
/// completion gate alongside the byte count.)
fn lock_planner_finished(state: &TaskState) -> bool {
    Planner::from_task(state).all_finished()
}

// ---------------------------------------------------------------------------
// Workers
// ---------------------------------------------------------------------------

/// One worker thread: claim → fetch → report, until the
/// planner is exhausted, the control stops it, a terminal
/// report (failure / range death) stops everyone, or the
/// pool shrinks and this worker draws a retire token
/// (AUDIT 0.32.1, P4-A4).
fn worker_loop(
    shared: Arc<Shared>,
    id: u64,
    planner: Arc<Mutex<Planner>>,
    ctx: Arc<WorkerCtx>,
    report: Arc<Mutex<WorkerReport>>,
    retire: Arc<AtomicUsize>,
) {
    let should_stop = |shared: &Arc<Shared>, report: &Arc<Mutex<WorkerReport>>| {
        control_of(shared, id) != Control::Run || lock(report).is_terminal()
    };
    loop {
        if should_stop(&shared, &report) {
            return;
        }
        // AUDIT 0.32.2 (Fix 2): a stale run claims no new
        // work. (Without this, a worker whose fetch just went
        // stale in the instant a resume started a newer run —
        // control back at Run — would re-claim from its OLD
        // planner forever, each fetch dying at its own gate.)
        if ctx.live_gen.load(Ordering::SeqCst) != ctx.gen_token {
            return;
        }
        // Pool shrink: holding a retire token means the pool
        // is over the (lowered) connection count — exit so a
        // later claim doesn't over-subscribe the task.
        if retire
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                if v > 0 {
                    Some(v - 1)
                } else {
                    None
                }
            })
            .is_ok()
        {
            return;
        }
        let claim = lock(&planner).claim();
        let Some(claim) = claim else {
            return;
        };
        // Segments are addressed by their start offset, never
        // by vector position (P1-10): a steal or replan can
        // rebuild the vector, but a start names one range
        // forever.
        let key = match claim {
            Claim::Fresh(start) => start,
            Claim::Steal { start, .. } => start,
        };
        let Some(seg) = lock(&planner).segment(key) else {
            return;
        };
        fetch_segment(&shared, id, &planner, &ctx, &report, key, seg);
        // fetch_segment returns after finishing the segment,
        // releasing it (stop), or recording a terminal report —
        // the loop head re-checks the stop conditions either way.
    }
}

/// Fetch one claimed segment, with the brief's retry policy:
/// up to 3 attempts, backoff 500 ms × 2^n slept in 100 ms
/// slices (control honored throughout). Writes go through the
/// worker's OWN file handle, seek-per-chunk; `planner.progress`
/// is reported only after bytes are written.
fn fetch_segment(
    shared: &Arc<Shared>,
    id: u64,
    planner: &Arc<Mutex<Planner>>,
    ctx: &Arc<WorkerCtx>,
    report: &Arc<Mutex<WorkerReport>>,
    key: u64,
    seg: Segment,
) {
    let base_done = seg.done;
    // Where the next attempt starts, as a segment `done` value:
    // after a transport error the planner's count is honest
    // (real file bytes) and the attempt resumes there; after
    // an invalid-status answer the attempt restarts from the
    // claim's base (the transport's pre-body judgment means
    // no untrusted bytes were written, so this is a choice,
    // not a repair).
    let mut resume_from = base_done;
    let mut attempt: u32 = 0;
    // Assigned on every path that can break out of the retry
    // loop below (each failure branch sets it before breaking).
    let mut last_error: String;
    loop {
        if control_of(shared, id) != Control::Run
            || lock(report).is_terminal()
            || ctx.live_gen.load(Ordering::SeqCst) != ctx.gen_token
        {
            lock(planner).release(key);
            return;
        }
        attempt += 1;
        if attempt > 1 {
            let backoff = RETRY_BASE_MS.saturating_mul(1u64 << (attempt - 2).min(6));
            if !sleep_sliced(shared, id, report, backoff) {
                lock(planner).release(key);
                return;
            }
        }
        let cur = lock(planner).segment(key).unwrap_or(Segment {
            start: seg.start,
            end: seg.end,
            done: resume_from,
        });
        if !ctx.single && cur.is_finished() {
            return;
        }
        let start = cur.start.saturating_add(resume_from);
        let range = if ctx.single {
            None
        } else {
            Some((start, cur.end.saturating_sub(1)))
        };
        let if_range = if range.is_some() {
            ctx.validator.as_deref()
        } else {
            None
        };
        let cookies = ctx.cookies.clone();
        let cookie_for = move |u: &str| cookie_header(&cookies, u);
        let stop_shared = shared.clone();
        let stop_report = report.clone();
        let stop_gen = ctx.live_gen.clone();
        let stop_token = ctx.gen_token;
        let should_stop = move || {
            control_of(&stop_shared, id) != Control::Run
                || stop_gen.load(Ordering::SeqCst) != stop_token
                || lock(&stop_report).is_terminal()
        };
        // The worker's OWN handle to the .part file: write at
        // exact offsets, never append.
        let mut file = match OpenOptions::new().write(true).open(&ctx.part_path) {
            Ok(f) => f,
            Err(e) => {
                last_error = format!("cannot open file: {e}");
                if attempt >= MAX_ATTEMPTS {
                    break;
                }
                continue;
            }
        };
        // Sink write bound. Ranged: the segment's LIVE end
        // (P1-10) — if a thief stole the tail mid-flight, the
        // bytes past the new end are the thief's, and writing
        // them would splice two bodies into one range. A body
        // running past an UNshrunk end is a protocol violation
        // (RANGE_OVERFLOW). Single-stream: the known total, or
        // the disk guard for an unknown one (P1-2).
        let cap: u64 = if range.is_some() {
            cur.end.saturating_sub(start)
        } else {
            match ctx.total {
                Some(t) => t.saturating_sub(start),
                None => u64::MAX,
            }
        };
        let mut written: u64 = 0;
        let mut disk_since_check: u64 = 0;
        let disk_dir = ctx
            .part_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let planner_sink = planner.clone();
        let mut sink = |bytes: &[u8]| -> Result<(), String> {
            // AUDIT 0.32.2 (Fix 2): the write gate is the
            // sink's FIRST statement — a stale run (or a
            // stopped control) writes not one more byte, even
            // a chunk already in hand.
            if ctx.live_gen.load(Ordering::SeqCst) != ctx.gen_token
                || control_of(shared, id) != Control::Run
            {
                return Err(STALE_RUN.to_string());
            }
            // 0.33 speed throttle: draw this chunk's FULL
            // length from the one shared bucket BEFORE any
            // byte is written (rate counts pre-write bytes —
            // that is the design). Rate 0 = off. While
            // waiting, the same stale/stop gate as above is
            // re-checked each slice; tripping it retires the
            // worker with THROTTLE_STOP, which fetch_segment
            // treats exactly like STALE_RUN. No lock is held
            // across the sleep.
            {
                let len = bytes.len() as u64;
                loop {
                    if ctx.live_gen.load(Ordering::SeqCst) != ctx.gen_token
                        || control_of(shared, id) != Control::Run
                    {
                        return Err(THROTTLE_STOP.to_string());
                    }
                    let wait = {
                        let mut bucket = lock(&ctx.bucket);
                        if bucket.rate() == 0 {
                            break;
                        }
                        if bucket.try_take(len, now_ms()) {
                            break;
                        }
                        bucket.wait_ms_for(len, now_ms())
                    };
                    std::thread::sleep(Duration::from_millis(wait.min(50)));
                }
            }
            if range.is_some() {
                let live_end = lock(&planner_sink)
                    .segment(key)
                    .map(|s| s.end)
                    .unwrap_or(cur.end);
                let allowed = live_end.saturating_sub(start).saturating_sub(written);
                if (bytes.len() as u64) > allowed {
                    if live_end < cur.end {
                        // Stolen from mid-flight: write the
                        // prefix that is still ours, then
                        // retire from this segment quietly.
                        if allowed > 0 {
                            let take = usize::try_from(allowed).unwrap_or(usize::MAX);
                            file.seek(SeekFrom::Start(start.saturating_add(written)))
                                .map_err(|e| format!("seek failed: {e}"))?;
                            file.write_all(&bytes[..take])
                                .map_err(|e| format!("write failed: {e}"))?;
                            written = written.saturating_add(allowed);
                            lock(&planner_sink).progress(key, resume_from.saturating_add(written));
                        }
                        return Err(SEGMENT_SHRUNK.to_string());
                    }
                    return Err(RANGE_OVERFLOW.to_string());
                }
            } else if written.saturating_add(bytes.len() as u64) > cap {
                // AUDIT 0.32.1 (P4-12): label the two overflow
                // shapes honestly.
                if ctx.single && ctx.total.is_some() {
                    return Err("server sent more bytes than its stated size".to_string());
                }
                return Err(RANGE_OVERFLOW.to_string());
            }
            file.seek(SeekFrom::Start(start.saturating_add(written)))
                .map_err(|e| format!("seek failed: {e}"))?;
            file.write_all(bytes)
                .map_err(|e| format!("write failed: {e}"))?;
            written = written.saturating_add(bytes.len() as u64);
            // Only now — bytes on disk — may progress advance.
            lock(&planner_sink).progress(key, resume_from.saturating_add(written));
            // Unknown-total disk guard (P1-2): the stream is
            // unbounded by design, so the volume is the bound.
            if ctx.total.is_none() {
                disk_since_check = disk_since_check.saturating_add(bytes.len() as u64);
                if disk_since_check >= DISK_CHECK_EVERY {
                    disk_since_check = 0;
                    if let Some(free) = free_disk_bytes(&disk_dir) {
                        if free < DISK_RESERVE_BYTES {
                            return Err("disk nearly full — download stopped".to_string());
                        }
                    }
                }
            }
            Ok(())
        };
        let outcome = winhttp::fetch_range(
            &ctx.url,
            range,
            if_range,
            ctx.total,
            &cookie_for,
            ctx.referer.as_deref(),
            &should_stop,
            &mut sink,
        );
        drop(sink);
        match outcome {
            Err(FetchError::Stopped) => {
                lock(planner).release(key);
                return;
            }
            Err(FetchError::Failed(e)) if e == STALE_RUN || e == THROTTLE_STOP => {
                // This run was retired at the manager (pause /
                // cancel / a newer run) — STALE_RUN at the
                // sink gate, THROTTLE_STOP (0.33) when the
                // retirement landed while this worker waited
                // on the speed bucket: the manager's snapshot
                // is the final word, so release the segment and
                // retire quietly — no error, no report.
                lock(planner).release(key);
                return;
            }
            Err(FetchError::Failed(e)) if e == SEGMENT_SHRUNK => {
                // Our range was stolen; the remainder is the
                // thief's assignment. No release (the segment
                // stays assigned — to the thief's split), no
                // error: just claim the next segment.
                return;
            }
            Err(FetchError::Failed(e)) if e == RANGE_OVERFLOW => {
                set_range_died(report);
                lock(planner).release(key);
                return;
            }
            Err(FetchError::Failed(e)) => {
                last_error = e;
                // Single-stream: a mid-body failure cannot be
                // retried in place (a fresh GET restarts at
                // byte 0 and would splice); the task fails and
                // a later resume restarts the stream cleanly.
                if ctx.single && (resume_from > 0 || written > 0) {
                    break;
                }
                // Transport error: the bytes written are real
                // (a validated 206/200 body prefix); resume
                // from the planner's honest count.
                resume_from = lock(planner)
                    .segment(key)
                    .map(|s| s.done)
                    .unwrap_or(resume_from);
                if attempt >= MAX_ATTEMPTS {
                    break;
                }
            }
            Ok(out) => {
                let status = out.status;
                // Validator cross-check (P1-4): if the task
                // carries a validator and this segment's
                // response states a DIFFERENT one in the same
                // header, the file changed mid-download — the
                // bytes already on disk are another version's.
                // Treat exactly like a range death: restart as
                // a single stream from zero.
                if range.is_some() {
                    if let Some(want) = &ctx.validator {
                        let header_name = if ctx.validator_is_etag {
                            "ETag"
                        } else {
                            "Last-Modified"
                        };
                        if let Some(got) = out.header(header_name) {
                            if got.trim() != want.trim() {
                                set_range_died(report);
                                lock(planner).release(key);
                                return;
                            }
                        }
                    }
                }
                if range.is_some() && status == 200 {
                    // Whole-file answer to a ranged request:
                    // ranges died or the file changed. (The
                    // transport judged before streaming, so no
                    // bytes of it reached the disk — but OTHER
                    // segments may hold a different version's
                    // bytes; the supervisor restarts the task
                    // as a single stream.)
                    set_range_died(report);
                    lock(planner).release(key);
                    return;
                }
                if status == 416 {
                    if lock(planner)
                        .segment(key)
                        .map(|s| s.is_finished())
                        .unwrap_or(false)
                    {
                        return; // already finished: ignore (brief)
                    }
                    set_failed(report, "HTTP 416".to_string());
                    lock(planner).release(key);
                    return;
                }
                if range.is_some() {
                    // 206 expected: validate Content-Range
                    // start (defense in depth — the transport
                    // already judged start, end AND total
                    // before a byte streamed).
                    let range_ok = status == 206
                        && out
                            .header("Content-Range")
                            .and_then(pico_core::core::download::parse_content_range)
                            .map(|(cs, _, _)| cs == start)
                            .unwrap_or(false);
                    if !range_ok {
                        last_error = if status == 206 {
                            "range mismatch (HTTP 206)".to_string()
                        } else {
                            format!("HTTP {status}")
                        };
                        resume_from = base_done;
                        if attempt >= MAX_ATTEMPTS {
                            break;
                        }
                        continue;
                    }
                    if lock(planner)
                        .segment(key)
                        .map(|s| s.is_finished())
                        .unwrap_or(false)
                    {
                        return;
                    }
                    // Short 206 body (clean EOF early): the
                    // bytes are real; continue from the count.
                    last_error = "short response (HTTP 206)".to_string();
                    resume_from = lock(planner)
                        .segment(key)
                        .map(|s| s.done)
                        .unwrap_or(resume_from);
                    if attempt >= MAX_ATTEMPTS {
                        break;
                    }
                    continue;
                }
                // Single-stream (no Range sent): 200 is the
                // expected status; EOF defines the end.
                if status != 200 {
                    last_error = format!("HTTP {status}");
                    resume_from = base_done;
                    if written > 0 {
                        break; // cannot retry in place (above)
                    }
                    if attempt >= MAX_ATTEMPTS {
                        break;
                    }
                    continue;
                }
                if let Some(total) = ctx.total {
                    if written < total {
                        // Short whole-file body (the stream
                        // started at byte 0, so `written` is
                        // directly comparable to the total).
                        last_error = "short response (HTTP 200)".to_string();
                        break;
                    }
                }
                return; // segment done
            }
        }
    }
    // Attempts exhausted.
    set_failed(report, last_error);
    lock(planner).release(key);
}

fn set_range_died(report: &Arc<Mutex<WorkerReport>>) {
    lock(report).range_died = true;
}

fn set_failed(report: &Arc<Mutex<WorkerReport>>, error: String) {
    let mut r = lock(report);
    if r.failed.is_none() {
        r.failed = Some(error);
    }
}

/// Sleep `ms` in 100 ms slices, honoring the task control and
/// any terminal report. Returns false when stopped early.
fn sleep_sliced(shared: &Arc<Shared>, id: u64, report: &Arc<Mutex<WorkerReport>>, ms: u64) -> bool {
    let mut left = ms;
    while left > 0 {
        if control_of(shared, id) != Control::Run || lock(report).is_terminal() {
            return false;
        }
        let step = left.min(RETRY_SLICE_MS);
        std::thread::sleep(Duration::from_millis(step));
        left -= step;
    }
    control_of(shared, id) == Control::Run && !lock(report).is_terminal()
}

// ---------------------------------------------------------------------------
// Manager tests (0.35, WS3). This module is bin-crate only:
// tests/download_tests.rs links pico_core alone and cannot
// construct a DownloadManager, so the manager-level pins
// live here. No new deps — temp dirs are std::env::temp_dir
// plus a unique subdir per test, cleaned up after.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64 as TestCounter;

    static TEST_SEQ: TestCounter = TestCounter::new(0);

    /// (root, downloads_dir, state_dir) in a fresh unique
    /// temp subdir.
    fn test_dirs(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let seq = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("pico-dl-mgr-{}-{seq}-{tag}", std::process::id()));
        let downloads_dir = root.join("downloads");
        let state_dir = root.join("state");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&downloads_dir);
        let _ = std::fs::create_dir_all(&state_dir);
        (root, downloads_dir, state_dir)
    }

    fn test_manager(
        downloads_dir: PathBuf,
        state_dir: PathBuf,
        max_concurrent: u32,
    ) -> DownloadManager {
        let notify: NotifyFn = Arc::new(|| {});
        DownloadManager::new(notify, downloads_dir, state_dir, 8, max_concurrent, 0)
    }

    fn status_of(mgr: &DownloadManager, id: u64) -> TaskStatus {
        mgr.views()
            .into_iter()
            .find(|v| v.id == id)
            .unwrap_or_else(|| panic!("task {id} missing from views"))
            .status
    }

    fn error_of(mgr: &DownloadManager, id: u64) -> Option<String> {
        mgr.views()
            .into_iter()
            .find(|v| v.id == id)
            .and_then(|v| v.error)
    }

    fn bare_state(id: u64, name: &str, status: TaskStatus) -> TaskState {
        TaskState {
            version: TASK_FILE_VERSION,
            id,
            url: format!("https://example.com/{name}"),
            file_name: name.to_string(),
            total: Some(100),
            etag: None,
            last_modified: None,
            connections: 8,
            status,
            added_ms: id * 10,
            segments: vec![Segment {
                start: 0,
                end: 100,
                done: 0,
            }],
            cookies: Vec::new(),
            error: None,
            referer: None,
            sha256: None,
            auto_retries: 0,
            error_hint: None,
        }
    }

    /// T4 (0.35, WS3 — the regression pin): a wind-down
    /// reached with control == Run must write NO status.
    /// Through 0.34 `wind_down_stopped` dispatched
    /// `match control { Cancel => remove, _ => Paused }`,
    /// so a Run control fell into the Paused arm — this
    /// test FAILS against that catch-all. It is currently
    /// unreachable as a cause (every Pause/Cancel writer
    /// invalidates the generation gate in the same lock
    /// hold, and wind_down returns early when the run is
    /// not current), so the test hands wind_down a CURRENT
    /// gate token directly, the shape the catch-all would
    /// have mishandled the moment any future writer forgot
    /// to invalidate. The runtime is inserted by hand (no
    /// supervisor thread exists for it), so nothing races
    /// the assertions.
    #[test]
    fn t4_wind_down_with_control_run_writes_no_status() {
        let (root, downloads_dir, state_dir) = test_dirs("t4");
        let mgr = test_manager(downloads_dir, state_dir, 3);
        let id = 42;
        let state = bare_state(id, "file.bin", TaskStatus::Downloading);
        let tok = {
            let mut inner = lock(&mgr.shared.inner);
            let mut gate = GenerationGate::new();
            let tok = gate.begin();
            inner.tasks.insert(
                id,
                TaskRuntime {
                    state: state.clone(),
                    control: Control::Run,
                    speed: SpeedMeter::new(),
                    live_downloaded: 0,
                    live_planner: None,
                    replan_request: None,
                    supervisor: None,
                    gate,
                    live_gen: Arc::new(AtomicU64::new(tok)),
                    retry_not_before: 0,
                    retry_restart: false,
                    refreshing: false,
                },
            );
            inner.order.push(id);
            tok
        };
        wind_down_stopped(&mgr.shared, id, &state, None, tok);
        assert_eq!(
            status_of(&mgr, id),
            TaskStatus::Downloading,
            "wind-down with control=Run must leave the status untouched (not Paused)"
        );
        // Positive control: control == Pause with the gate
        // NOT invalidated (the same current token) parks
        // the task as Paused through the same entry point.
        {
            let mut inner = lock(&mgr.shared.inner);
            inner.tasks.get_mut(&id).expect("task").control = Control::Pause;
        }
        wind_down_stopped(&mgr.shared, id, &state, None, tok);
        assert_eq!(
            status_of(&mgr, id),
            TaskStatus::Paused,
            "positive control: wind-down with control=Pause parks the task"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A stalling HTTP endpoint for T2/T3: a loopback
    /// server that ANSWERs every request with the headers
    /// of a 10 MB file (200 + Content-Length, no
    /// Accept-Ranges → the probe completes and the plan is
    /// a single stream) and then never sends a single body
    /// byte. The transfer sits in receive, so a started
    /// task stays Downloading for every assertion instead
    /// of racing ahead to Failed the way an
    /// instantly-refused unroutable address does.
    /// (Merely holding the connection open unanswered is
    /// NOT enough — measured in the sandbox this suite is
    /// also run in: an egress proxy in front of the
    /// loopback resets unanswered WinHTTP probes within
    /// milliseconds; a raw socket stalls fine, the probe
    /// does not.) Returns the port and the answered
    /// sockets' keep-alive (dropping it closes them).
    fn stall_server() -> (u16, Arc<Mutex<Vec<std::net::TcpStream>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("stall listener");
        let port = listener.local_addr().expect("stall addr").port();
        let held: Arc<Mutex<Vec<std::net::TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let held2 = held.clone();
        std::thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                let held3 = held2.clone();
                std::thread::spawn(move || {
                    // Read the request headers, answer with
                    // the file's headers, then hold the
                    // socket open without a body.
                    let mut buf: Vec<u8> = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16384 {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let response = "HTTP/1.1 200 OK\r\nContent-Length: 10000000\r\nContent-Type: application/octet-stream\r\n\r\n";
                    if stream.write_all(response.as_bytes()).is_err() {
                        return;
                    }
                    lock(&held3).push(stream);
                    loop {
                        std::thread::sleep(Duration::from_secs(60));
                    }
                });
            }
        });
        (port, held)
    }

    /// T2 (0.35, WS3): with a free slot, `add` births the
    /// task Queued and `schedule` starts it synchronously —
    /// the view is Downloading the moment `add` returns.
    /// The URL is the stalling loopback endpoint (see
    /// `stall_server`): its body never arrives, so the
    /// supervisor cannot move the task on
    /// before the assertion reads it.
    #[test]
    fn t2_add_with_a_free_slot_is_downloading_immediately() {
        let (root, downloads_dir, state_dir) = test_dirs("t2");
        let mgr = test_manager(downloads_dir, state_dir, 3);
        let (port, _held) = stall_server();
        let id = mgr
            .add(
                &format!("http://127.0.0.1:{port}/file.bin"),
                None,
                Vec::new(),
                0,
                None,
            )
            .expect("add accepted");
        assert_eq!(
            status_of(&mgr, id),
            TaskStatus::Downloading,
            "a fresh task with a free slot is Downloading immediately after add() (error: {:?})",
            error_of(&mgr, id)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// T3 (0.35, WS3 — the starvation pin): restored Paused
    /// tasks hold no scheduler slots (only Downloading
    /// tasks count), so five of them must never starve or
    /// park fresh work: with max_concurrent = 3 the first
    /// three fresh tasks start at once and the fourth
    /// queues — and no fresh task is EVER Paused. The
    /// fresh URLs are the stalling loopback endpoint (see
    /// T2), so a started task stays Downloading for the
    /// assertions instead of racing ahead to Failed.
    #[test]
    fn t3_paused_restores_never_starve_fresh_tasks() {
        let (root, downloads_dir, state_dir) = test_dirs("t3");
        // Pre-seed the state dir with 5 Paused task files.
        for i in 1..=5u64 {
            let state = bare_state(i, &format!("seeded-{i}.bin"), TaskStatus::Paused);
            save_task(&state_dir, &state).expect("seed task");
        }
        let mgr = test_manager(downloads_dir, state_dir, 3);
        let (port, _held) = stall_server();
        let views = mgr.views();
        assert_eq!(views.len(), 5, "all five restores are present");
        assert!(
            views.iter().all(|v| v.status == TaskStatus::Paused),
            "all five restores are Paused: {:?}",
            views.iter().map(|v| v.status).collect::<Vec<_>>()
        );
        let mut fresh: Vec<u64> = Vec::new();
        for n in 1..=4u64 {
            let id = mgr
                .add(
                    &format!("http://127.0.0.1:{port}/fresh-{n}.bin"),
                    None,
                    Vec::new(),
                    0,
                    None,
                )
                .expect("add accepted");
            fresh.push(id);
            for (k, fid) in fresh.iter().enumerate() {
                let status = status_of(&mgr, *fid);
                assert_ne!(
                    status,
                    TaskStatus::Paused,
                    "fresh task {fid} must never be Paused"
                );
                if k < 3 {
                    assert_eq!(
                        status,
                        TaskStatus::Downloading,
                        "fresh task {fid} (start #{}) takes a free slot at once (error: {:?})",
                        k + 1,
                        error_of(&mgr, *fid)
                    );
                } else {
                    assert_eq!(
                        status,
                        TaskStatus::Queued,
                        "the fourth fresh task queues behind the three running ones"
                    );
                }
            }
        }
        // The restores were never scheduled or disturbed.
        for i in 1..=5u64 {
            assert_eq!(status_of(&mgr, i), TaskStatus::Paused);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
