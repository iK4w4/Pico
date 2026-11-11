//! Downloads coordinator (0.34) — the pure, host-testable
//! state machine that owns downloads coordination: capture
//! triage and tab fate, the cookie race, the notice
//! machine, and command dispatch.
//!
//! It owns NO I/O. The app feeds it events (a capture
//! started, cookies arrived, a navigation finished, the
//! backend changed, a command was dispatched) and executes
//! the [`DlEffect`]s it returns — log lines, snapshot
//! pushes, shell opens, tab fates. Every decision that can
//! be made without a window or a network lives here.
//!
//! THE DESIGN LAW (learned across the 0.32.x patch rounds
//! and 0.33's WS1): decisions are made ONCE, at capture
//! time, from capture-time facts. A tab's fate is decided
//! by [`decide_tab_fate`] when the download is captured;
//! at landing time the coordinator only REVALIDATES plain
//! facts the app supplies (is the tab alive, is its
//! navigation epoch still the capture epoch, is it the
//! only tab now) — it never re-derives history from a live
//! tab whose state the captured navigation itself may
//! have polluted.
//!
//! The coordinator talks to the download manager only
//! through [`DlBackend`]; the app implements that trait
//! over `src/downloads.rs`'s `DownloadManager` (Worker 2's
//! fold). Tests drive it through a fake backend and replay
//! the field stories end to end (`tests/dl_replay.rs`).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::core::dlcmd::DlCommand;
use crate::core::download::{
    triage_download_uri, unwind_action, url_host, CookiePair, DownloadTriage, TaskStatus,
    UnwindAction, UnwindOrigin,
};
use crate::core::redact::redact_url;

// ---------------------------------------------------------------------------
// Backend: the coordinator's only view of the manager
// ---------------------------------------------------------------------------

/// The download manager, as the coordinator may use it.
/// Exactly the verbs the coordination layer needs — no
/// more. All methods take `&self`: the real manager is a
/// cheap clone handle over shared state (every
/// `DownloadManager` method is already `&self`).
pub trait DlBackend {
    /// Add a task; returns its id, or `None` when the
    /// manager refuses (non-http(s) URL, task caps).
    fn add(
        &self,
        url: &str,
        suggested: Option<&str>,
        cookies: Vec<CookiePair>,
        connections: u32,
        referer: Option<String>,
    ) -> Option<u64>;
    /// Record a refused capture as a visible Failed task;
    /// returns its id, or `None` when a cap refuses the
    /// record itself.
    fn add_failed(&self, url: &str, suggested: Option<&str>, reason: &str) -> Option<u64>;
    fn pause(&self, id: u64);
    fn resume(&self, id: u64);
    fn cancel(&self, id: u64);
    fn restart(&self, id: u64);
    fn refresh_link(&self, id: u64, url: &str);
    /// Remove a task's record from the list.
    ///
    /// MAPPING (flagged for integration): the 0.33 manager
    /// has NO pure remove-record verb. The page's `remove`
    /// was served by `DownloadManager::cancel`, whose
    /// semantics are exactly the page's intent — a Complete
    /// task loses only its record (the finished file
    /// stays); an unfinished task loses its `.part` and its
    /// record. The app-side adapter therefore implements
    /// this verb as `cancel`, and `cancel` (the chrome
    /// verb) maps to the same manager method: the two
    /// coordinator verbs are aliases over ONE manager verb
    /// until the manager grows a distinct remove.
    fn remove(&self, id: u64);
    fn delete_file(&self, id: u64);
    fn clear_completed(&self);
    fn clear_all(&self);
    fn set_connections(&self, id: u64, n: u32);
    fn set_default_connections(&self, n: u32);
    fn set_max_concurrent(&self, n: u32);
    fn set_speed_limit(&self, kbps: Option<u64>);
    fn views(&self) -> Vec<DlTaskView>;
    fn version(&self) -> u64;
    fn active_count(&self) -> usize;
    /// How many tasks are Failed right now. (The manager
    /// computes this from its views; the trait carries it
    /// because the chrome badge arithmetic needs the total,
    /// while the coordinator tracks only the UNSEEN delta.)
    fn failed_count(&self) -> usize;
    fn name_of(&self, id: u64) -> Option<String>;
    /// The finished file's path for a Complete task;
    /// `None` for every other status and unknown ids.
    fn completed_path(&self, id: u64) -> Option<PathBuf>;
    fn downloads_dir(&self) -> PathBuf;
}

/// One task as the coordinator (and the snapshot) sees it:
/// a core-side mirror of `src/downloads.rs`'s
/// `DownloadView`, field for field. `status` is the core
/// [`TaskStatus`]; in the serialized snapshot it renders
/// through `TaskStatus::label()` — the page vocabulary
/// ("done", not "complete") the 0.33 surfaces used.
#[derive(Debug, Clone, PartialEq)]
pub struct DlTaskView {
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
    pub url: String,
    pub added_ms: u64,
    pub sha256: Option<String>,
    pub hint: Option<String>,
    pub retry_text: Option<String>,
    pub retry_pending: bool,
    pub refreshing: bool,
    /// Live transfer rate in bytes/sec (0.35), mirrored
    /// field-for-field from the manager's `DownloadView`.
    pub speed_bps: Option<u64>,
    /// Seconds left at the current rate (0.35), mirrored
    /// field-for-field from the manager's `DownloadView`.
    pub eta_secs: Option<u64>,
}

impl Serialize for DlTaskView {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("DlTaskView", 20)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("name", &self.name)?;
        s.serialize_field("host", &self.host)?;
        // The wire status is the core LABEL ("done" for
        // Complete), exactly as app.rs's DownloadSnap and
        // downloads_page_json rendered it in 0.33.
        s.serialize_field("status", &self.status.label())?;
        s.serialize_field("total", &self.total)?;
        s.serialize_field("downloaded", &self.downloaded)?;
        s.serialize_field("percent", &self.percent)?;
        s.serialize_field("size_text", &self.size_text)?;
        s.serialize_field("detail_text", &self.detail_text)?;
        s.serialize_field("connections", &self.connections)?;
        s.serialize_field("error", &self.error)?;
        s.serialize_field("url", &self.url)?;
        s.serialize_field("added_ms", &self.added_ms)?;
        s.serialize_field("sha256", &self.sha256)?;
        s.serialize_field("hint", &self.hint)?;
        s.serialize_field("retry_text", &self.retry_text)?;
        s.serialize_field("retry_pending", &self.retry_pending)?;
        s.serialize_field("refreshing", &self.refreshing)?;
        s.serialize_field("speed_bps", &self.speed_bps)?;
        s.serialize_field("eta_secs", &self.eta_secs)?;
        s.end()
    }
}

// ---------------------------------------------------------------------------
// Settings + snapshot (the ONE data shape, frozen field names)
// ---------------------------------------------------------------------------

/// The downloads settings the coordinator mirrors: it is
/// initialized with them (`Coordinator::new`), updates
/// them on the settings verbs, and hands them back in
/// [`DlEffect::PersistDownloadsSettings`] for the app to
/// persist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DlSettings {
    pub default_connections: u32,
    pub max_concurrent: u32,
    /// Global speed limit in kbps; `None` = off.
    pub speed_limit_kbps: Option<u64>,
}

/// What kind of glance a [`Notice`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Started,
    Finished,
    Failed,
    Blocked,
}

impl NoticeKind {
    /// The 0.33 wire/log spelling.
    pub fn label(self) -> &'static str {
        match self {
            NoticeKind::Started => "started",
            NoticeKind::Finished => "finished",
            NoticeKind::Failed => "failed",
            NoticeKind::Blocked => "blocked",
        }
    }
}

/// The newest download notice (the toast): born `open`,
/// retired by expiry (seq-checked), a panel opening, or a
/// click. Absorbs app.rs's `dl_notice` state (0.33).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub seq: u64,
    pub kind: NoticeKind,
    pub name: String,
    /// The refusal/failure reason, or — for a Finished
    /// notice — the finished task's size text (0.33's
    /// finished toast carries name + size).
    pub reason: Option<String>,
    pub open: bool,
}

/// The start page's Downloads card model (0.35.1): a pure
/// reduction of the task views + the unseen-failed count
/// into the ONE glance the card renders. The field names
/// are FROZEN (serde snake_case, exactly as declared) —
/// the start page's JS reads exactly these:
/// `{"state":"empty"|"active"|"waiting"|"failed"|"finished",
/// "count":usize,"speed_bps":Option<u64>,"paused":usize,
/// "queued":usize,"newest_name":Option<String>,
/// "unseen":bool}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DlCardView {
    pub state: String,
    pub count: usize,
    pub speed_bps: Option<u64>,
    pub paused: usize,
    pub queued: usize,
    pub newest_name: Option<String>,
    pub unseen: bool,
}

impl DlCardView {
    /// The empty card: no tasks, nothing to say.
    pub fn empty() -> Self {
        DlCardView {
            state: "empty".to_string(),
            count: 0,
            speed_bps: None,
            paused: 0,
            queued: 0,
            newest_name: None,
            unseen: false,
        }
    }
}

impl Default for DlCardView {
    fn default() -> Self {
        Self::empty()
    }
}

/// Reduce the task views to the card model (0.35.1). Pure:
/// the same inputs always give the same card. Precedence,
/// first match wins:
///
/// - no tasks → `empty`;
/// - `failed_unseen > 0` → `failed` (unseen): count is the
///   unseen count itself, newest_name the newest Failed
///   task's name — an unseen failure outranks everything,
///   including a running download;
/// - any Downloading → `active`: count is the number
///   Downloading, speed_bps the SUM of their rates (None
///   counts as 0 — the same inputs the chrome popover
///   header sums);
/// - else any Paused or Queued → `waiting`: count is
///   paused + queued;
/// - else any Complete → `finished`: newest_name the
///   newest Complete task's name;
/// - else (only seen-failed tasks remain) → `failed`
///   (seen): count is the total Failed, newest_name the
///   newest Failed name.
///
/// "Newest" always means max `added_ms`, never vec order.
/// The paused/queued fields are populated truthfully in
/// EVERY state; speed_bps is Some only in `active`;
/// newest_name is Some only in `failed`/`finished` (when a
/// task of that kind exists to name).
pub fn card_view(tasks: &[DlTaskView], failed_unseen: u32) -> DlCardView {
    let paused = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Paused)
        .count();
    let queued = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Queued)
        .count();
    if tasks.is_empty() {
        return DlCardView::empty();
    }
    let newest_name_of = |status: TaskStatus| -> Option<String> {
        tasks
            .iter()
            .filter(|t| t.status == status)
            .max_by_key(|t| t.added_ms)
            .map(|t| t.name.clone())
    };
    if failed_unseen > 0 {
        return DlCardView {
            state: "failed".to_string(),
            count: failed_unseen as usize,
            speed_bps: None,
            paused,
            queued,
            newest_name: newest_name_of(TaskStatus::Failed),
            unseen: true,
        };
    }
    let downloading: Vec<&DlTaskView> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Downloading)
        .collect();
    if !downloading.is_empty() {
        let speed: u64 = downloading.iter().map(|t| t.speed_bps.unwrap_or(0)).sum();
        return DlCardView {
            state: "active".to_string(),
            count: downloading.len(),
            speed_bps: Some(speed),
            paused,
            queued,
            newest_name: None,
            unseen: false,
        };
    }
    if paused + queued > 0 {
        return DlCardView {
            state: "waiting".to_string(),
            count: paused + queued,
            speed_bps: None,
            paused,
            queued,
            newest_name: None,
            unseen: false,
        };
    }
    let complete = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Complete)
        .count();
    if complete > 0 {
        return DlCardView {
            state: "finished".to_string(),
            count: complete,
            speed_bps: None,
            paused,
            queued,
            newest_name: newest_name_of(TaskStatus::Complete),
            unseen: false,
        };
    }
    let failed = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .count();
    DlCardView {
        state: "failed".to_string(),
        count: failed,
        speed_bps: None,
        paused,
        queued,
        newest_name: newest_name_of(TaskStatus::Failed),
        unseen: false,
    }
}

/// The ONE downloads data shape (0.34): it replaces both
/// the chrome snapshot's `downloads`/`downloads_notice`
/// fields and the downloads page's init/push JSON. The
/// field names are FROZEN — Worker 3's JS rebuild reads
/// exactly these. 0.35.1 adds `card` (the start page's
/// Downloads card model, [`DlCardView`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DlSnapshot {
    pub version: u64,
    pub tasks: Vec<DlTaskView>,
    pub active_count: usize,
    pub failed_unseen: u32,
    pub settings: DlSettings,
    pub notice: Option<Notice>,
    pub card: DlCardView,
}

// ---------------------------------------------------------------------------
// Effects: what the app executes on the coordinator's behalf
// ---------------------------------------------------------------------------

/// One effect the app executes. The coordinator returns
/// these; it performs none of them itself. Notices are NOT
/// an effect: a notice change is coordinator STATE, read
/// back through [`Coordinator::snapshot`], and announced
/// by the `PushChrome` effect that accompanies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DlEffect {
    /// Write this exact line to pico.log at info level.
    /// The coordinator composes it (URLs host-only or
    /// redacted; cookie VALUES never appear, counts only).
    Log(String),
    /// Write this exact line to pico.log at error level.
    LogError(String),
    /// Execute a capture-time tab fate for `tab` (decided
    /// once at capture; revalidated at landing — see
    /// [`Coordinator::capture_started`]).
    ExecuteFate { tab: u64, fate: TabFate },
    /// Push a fresh [`DlSnapshot`] to the chrome.
    PushChrome,
    /// Push a fresh [`DlSnapshot`] to the downloads page.
    PushPage,
    /// Open (or focus) the downloads page tab.
    OpenDownloadsPage,
    /// Open the downloads popover panel.
    OpenPanel,
    /// Close the downloads popover panel.
    ClosePanel,
    /// Open this folder in the OS file explorer. Never a
    /// file, never a URL.
    ShellOpenFolder(PathBuf),
    /// Open this finished file (a Complete task's path,
    /// resolved by the backend). Only ever produced by an
    /// explicit `open-file` command — never automatically.
    ShellOpenFile(PathBuf),
    /// Persist these downloads settings (the app owns the
    /// state file; the coordinator owns the values).
    PersistDownloadsSettings(DlSettings),
}

// ---------------------------------------------------------------------------
// Capture: facts, origin, fate
// ---------------------------------------------------------------------------

/// How the capturing tab came to exist (mirrors
/// `core::download::UnwindOrigin`, named for this API).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOrigin {
    /// A tab the SITE opened (new-window/popup adoption).
    Adopted,
    /// A tab the USER opened.
    User,
}

/// The capture-time facts a tab fate is decided from —
/// assembled by the app from its navigation guard /
/// tab state AT CAPTURE, never re-derived later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFacts {
    /// The tab's live URL (the load-start adoption
    /// included).
    pub live_url: String,
    /// The URL the download was captured from.
    pub captured_url: String,
    /// Every URL of the navigation being captured (0.35,
    /// WS2): the navigation's initial target plus each
    /// redirect hop, ending at `captured_url`. The app
    /// assembles it from the tab's pending-navigation
    /// record plus the live and captured URLs. A capture
    /// that is not a navigation (a context-menu link)
    /// carries `vec![captured_url]` alone, so the page
    /// being read is never in it and its fate stays None.
    pub chain: Vec<String>,
    /// The history machine's current entry: the last page
    /// that actually committed.
    pub committed_url: Option<String>,
    /// The history entry before the current one.
    pub prev_url: Option<String>,
    pub origin: CaptureOrigin,
    /// Whether this is the window's only tab.
    pub only_tab: bool,
    /// A back/forward traversal is already in flight.
    pub traversal_pending: bool,
    /// The tab's navigation epoch (commit counter) at
    /// capture; the landing revalidation compares the
    /// app-supplied current epoch against this value.
    pub nav_epoch: u64,
}

/// What the shell should do with a tab whose navigation
/// was captured as a download — [`decide_tab_fate`]'s
/// vocabulary (the coordinator's renaming of
/// `core::download::UnwindAction`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabFate {
    /// Leave the tab alone.
    None,
    /// Reload the tab's last committed page.
    RestoreTo(String),
    /// Traverse back to the previous history entry.
    GoBack,
    /// The tab has no page of its own: convert it to the
    /// start page.
    ConvertToStart,
    /// The tab exists only to carry this download: close
    /// it through the ordinary tab-close path.
    CloseTab,
}

/// Decide a captured tab's fate — the 0.33 decision matrix
/// in its 0.35 chain form, delegated to the proven
/// `core::download::unwind_action` (its test matrix in
/// `tests/download_tests.rs` is the contract):
///
/// - live ∉ chain → [`TabFate::None`] (through 0.34 this
///   gate was the exact match live ≠ captured, which a
///   redirected navigation could never pass: the tab's
///   live URL is the pre-redirect hop, the captured URL
///   is post-redirect);
/// - a traversal is pending → [`TabFate::None`];
/// - a real page is committed (∉ the chain) →
///   [`TabFate::RestoreTo`] of it;
/// - a chain member itself committed and a different,
///   non-chain previous entry exists → [`TabFate::GoBack`]
///   (a previous entry that is itself a chain member is
///   a redirect hop, dropped before judging);
/// - a chain member (or nothing) committed, origin
///   Adopted and not the only tab → [`TabFate::CloseTab`];
/// - otherwise → [`TabFate::ConvertToStart`].
pub fn decide_tab_fate(f: &CaptureFacts) -> TabFate {
    let origin = match f.origin {
        CaptureOrigin::Adopted => UnwindOrigin::Adopted,
        CaptureOrigin::User => UnwindOrigin::User,
    };
    match unwind_action(
        &f.live_url,
        f.committed_url.as_deref(),
        f.prev_url.as_deref(),
        &f.captured_url,
        &f.chain,
        f.traversal_pending,
        origin,
        f.only_tab,
    ) {
        UnwindAction::None => TabFate::None,
        UnwindAction::Back => TabFate::GoBack,
        UnwindAction::NavigateTo(url) => TabFate::RestoreTo(url),
        UnwindAction::StartPage => TabFate::ConvertToStart,
        UnwindAction::CloseTab => TabFate::CloseTab,
    }
}

/// The plain landing-time facts the app supplies when a
/// capture's cookie race resolves: is the tab still
/// alive, what is its navigation epoch NOW, and is it the
/// window's only tab NOW. The coordinator compares these
/// against the capture record — it derives nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandingFacts {
    pub tab_alive: bool,
    pub nav_epoch: u64,
    pub only_tab: bool,
}

/// Which surface a dispatched command arrived from. (The
/// coordinator routes identically for every source today —
/// the gates live in the transports, see `core::dlcmd` —
/// but the source rides the call so the app's log line and
/// future surface rules have it.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlSource {
    Chrome,
    Page,
    Start,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapturePhase {
    AwaitingCookies,
    Landed,
}

#[derive(Debug, Clone)]
struct Capture {
    token: u64,
    tab: u64,
    epoch: u64,
    fate: TabFate,
    uri: String,
    /// The captured navigation's URL chain (0.35, WS2),
    /// copied from the capture facts: the finish swallow
    /// recognises a Finished for ANY member of it, not
    /// only the post-redirect captured URI.
    chain: Vec<String>,
    suggested: String,
    referer: Option<String>,
    phase: CapturePhase,
    /// A navigation Finished for this capture was already
    /// swallowed while the cookie race was still open: the
    /// record must not swallow a second one (a reload is a
    /// different navigation).
    finish_seen: bool,
}

// ---------------------------------------------------------------------------
// The coordinator
// ---------------------------------------------------------------------------

/// The downloads coordinator. Owns the capture records,
/// the notice machine, the mirrored settings, and the
/// previous-views diff — nothing else. Generic over the
/// backend so tests drive it with a fake.
pub struct Coordinator<B: DlBackend> {
    backend: B,
    settings: DlSettings,
    captures: HashMap<u64, Capture>,
    next_token: u64,
    notice_seq: u64,
    notice: Option<Notice>,
    failed_unseen: u32,
    prev_views: Vec<DlTaskView>,
    prev_version: u64,
    /// False until the first `backend_changed`: startup
    /// restore is silent (0.33 rule — a restored failed or
    /// finished task fires no toast).
    started: bool,
}

impl<B: DlBackend> Coordinator<B> {
    pub fn new(backend: B, settings: DlSettings) -> Self {
        Coordinator {
            backend,
            settings,
            captures: HashMap::new(),
            next_token: 1,
            notice_seq: 0,
            notice: None,
            failed_unseen: 0,
            prev_views: Vec::new(),
            prev_version: 0,
            started: false,
        }
    }

    /// The backend this coordinator drives (the app keeps
    /// its own handle too; this is for tests and folds
    /// that hold the coordinator as the only owner).
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// The mirrored settings (also in every snapshot).
    pub fn settings(&self) -> DlSettings {
        self.settings
    }

    /// The current data shape, pulled fresh from the
    /// backend: `{version, tasks, active_count,
    /// failed_unseen, settings, notice, card}` — the card
    /// (0.35.1) is reduced from the SAME views + unseen
    /// count this snapshot carries, so it can never
    /// disagree with them.
    pub fn snapshot(&self) -> DlSnapshot {
        let tasks = self.backend.views();
        let card = card_view(&tasks, self.failed_unseen);
        DlSnapshot {
            version: self.backend.version(),
            tasks,
            active_count: self.backend.active_count(),
            failed_unseen: self.failed_unseen,
            settings: self.settings,
            notice: self.notice.clone(),
            card,
        }
    }

    // -- capture flow ----------------------------------------------------

    /// A download capture started on `tab`. Runs the core
    /// triage (`core::download::triage_download_uri`) and
    /// decides the tab fate ONCE, from `facts`.
    ///
    /// REJECT: a non-empty uri is recorded via
    /// `backend.add_failed`, a Blocked notice is set, and
    /// the fate executes immediately (revalidated against
    /// the capture facts themselves, which are current by
    /// construction). The capture record is still stored
    /// (phase Landed) so the captured navigation's
    /// Finished is swallowed like an accepted capture's.
    ///
    /// ACCEPT: the capture is stored as AwaitingCookies
    /// and the ONLY effect is the fate-decided log line
    /// (0.35, WS2 — no silent fates) — the app starts the
    /// cookie fetch + timer and answers with
    /// [`Coordinator::cookies_arrived`] /
    /// [`Coordinator::cookies_timeout`]. When the app
    /// already knows cookies cannot apply (cross-site, per
    /// `core::download::cookie_capture_allowed`), it calls
    /// `cookies_arrived` immediately with an empty vec;
    /// the coordinator itself fetches nothing.
    ///
    /// Returns the capture token (minted here, from 1) —
    /// the app hands it back to the cookie answers — plus
    /// the effects to execute.
    pub fn capture_started(
        &mut self,
        tab: u64,
        uri: String,
        suggested: String,
        referer: Option<String>,
        facts: CaptureFacts,
    ) -> (u64, Vec<DlEffect>) {
        let token = self.next_token;
        self.next_token = self.next_token.saturating_add(1).max(1);
        // The stored chain and the fate decision must see
        // the SAME chain (0.35, WS2): the facts' chain is
        // normalized once, here — an empty chain becomes
        // the captured URL alone, and a chain missing the
        // captured URL gains it (it is the navigation's
        // final hop by definition). The decision is NOT
        // widened anywhere else: the live URL is never
        // added, so a non-navigation capture (context
        // menu, live = the page being read) still refuses.
        let mut facts = facts;
        if !facts.chain.contains(&facts.captured_url) {
            facts.chain.push(facts.captured_url.clone());
        }
        let fate = decide_tab_fate(&facts);
        // No silent fates (0.35, WS2): EVERY capture emits
        // exactly one fate-decided line at capture time, in
        // the same shape — through 0.34 a None fate was
        // silent (`fate_step` returns an empty vec for it),
        // so a field log showed a download task and no fate
        // line at all for exactly the captures whose tab
        // was left wedged. The line names no URL (a
        // RestoreTo fate is named by kind alone).
        let fate_effect = DlEffect::Log(format!(
            "download capture {token}: fate decided {}",
            fate_name(&fate)
        ));
        match triage_download_uri(&uri) {
            DownloadTriage::Reject(reason) => {
                let mut effects = vec![
                    DlEffect::Log(format!(
                        "download rejected ({reason}): {}",
                        redact_url(&uri)
                    )),
                    fate_effect,
                ];
                if !uri.trim().is_empty() {
                    let suggested_opt = if suggested.is_empty() {
                        None
                    } else {
                        Some(suggested.as_str())
                    };
                    let id = self.backend.add_failed(&uri, suggested_opt, reason);
                    let name = id
                        .and_then(|id| self.backend.name_of(id))
                        .unwrap_or_else(|| suggested_or_host(&suggested, &uri));
                    self.set_notice(NoticeKind::Blocked, name, Some(reason.to_string()));
                    effects.push(DlEffect::PushChrome);
                    effects.push(DlEffect::PushPage);
                }
                // The record lives on for the finish
                // swallow even though the fate lands now.
                self.captures.insert(
                    token,
                    Capture {
                        token,
                        tab,
                        epoch: facts.nav_epoch,
                        fate: fate.clone(),
                        uri,
                        chain: facts.chain.clone(),
                        suggested,
                        referer,
                        phase: CapturePhase::Landed,
                        finish_seen: false,
                    },
                );
                // Fate executes immediately: the facts are
                // the capture's own, so the landing facts
                // are (tab alive, same epoch, same only-tab).
                let landing = LandingFacts {
                    tab_alive: true,
                    nav_epoch: facts.nav_epoch,
                    only_tab: facts.only_tab,
                };
                let capture = self.captures.get(&token).cloned();
                if let Some(capture) = capture {
                    effects.extend(fate_step(&capture, &landing));
                }
                (token, effects)
            }
            DownloadTriage::Accept => {
                self.captures.insert(
                    token,
                    Capture {
                        token,
                        tab,
                        epoch: facts.nav_epoch,
                        fate,
                        uri,
                        chain: facts.chain.clone(),
                        suggested,
                        referer,
                        phase: CapturePhase::AwaitingCookies,
                        finish_seen: false,
                    },
                );
                (token, vec![fate_effect])
            }
        }
    }

    /// The cookie fetch answered. First arrival wins the
    /// race against [`Coordinator::cookies_timeout`]; the
    /// loser gets a single Log effect. The winner creates
    /// the task through the backend and lands the fate
    /// (revalidated against `facts`).
    pub fn cookies_arrived(
        &mut self,
        token: u64,
        cookies: Vec<CookiePair>,
        facts: LandingFacts,
    ) -> Vec<DlEffect> {
        self.land(token, cookies, facts)
    }

    /// The cookie fetch timed out: the task is created
    /// with no cookies. Same race discipline as
    /// [`Coordinator::cookies_arrived`].
    pub fn cookies_timeout(&mut self, token: u64, facts: LandingFacts) -> Vec<DlEffect> {
        self.land(token, Vec::new(), facts)
    }

    fn land(&mut self, token: u64, cookies: Vec<CookiePair>, facts: LandingFacts) -> Vec<DlEffect> {
        let Some(capture) = self.captures.get_mut(&token) else {
            // No record: the capture already landed AND its
            // record was consumed by the finish swallow, or
            // the token was never ours. Nothing to land.
            return Vec::new();
        };
        if capture.phase == CapturePhase::Landed {
            return vec![DlEffect::Log(format!(
                "download capture {token}: late cookie answer ignored"
            ))];
        }
        capture.phase = CapturePhase::Landed;
        let capture = capture.clone();
        let cookie_count = cookies.len();
        let host = url_host(&capture.uri).unwrap_or_else(|| "unknown host".to_string());
        let suggested_opt = if capture.suggested.is_empty() {
            None
        } else {
            Some(capture.suggested.as_str())
        };
        let mut effects = Vec::new();
        match self.backend.add(
            &capture.uri,
            suggested_opt,
            cookies,
            self.settings.default_connections,
            capture.referer.clone(),
        ) {
            Some(id) => {
                effects.push(DlEffect::Log(format!(
                    "download task created: {host} ({cookie_count} cookies)"
                )));
                let name = self
                    .backend
                    .name_of(id)
                    .unwrap_or_else(|| suggested_or_host(&capture.suggested, &capture.uri));
                self.set_notice(NoticeKind::Started, name, None);
                effects.push(DlEffect::PushChrome);
                effects.push(DlEffect::PushPage);
            }
            None => {
                effects.push(DlEffect::LogError(format!(
                    "download creation FAILED (manager refused): {host}"
                )));
                self.set_notice(
                    NoticeKind::Failed,
                    suggested_or_host(&capture.suggested, &capture.uri),
                    Some("Pico couldn't start this download".to_string()),
                );
                effects.push(DlEffect::PushChrome);
                effects.push(DlEffect::PushPage);
            }
        }
        effects.extend(fate_step(&capture, &facts));
        effects
    }

    /// A tab's navigation Finished. Returns true (swallow
    /// the event whole — no commit, no error state) iff a
    /// capture record exists for `tab` whose captured
    /// navigation's CHAIN contains `uri` (0.35, WS2 —
    /// through 0.34 only the captured URI itself matched,
    /// so a Finished carrying the live Source at
    /// completion, or an intermediate hop, could miss the
    /// swallow in the mirror direction of the fate gate):
    ///
    /// - a Landed record swallows once and is dropped;
    /// - an AwaitingCookies record swallows its FIRST
    ///   finish (marked `finish_seen`) and stays — its
    ///   fate is still pending; a second identical finish
    ///   is a different navigation (a reload) and is NOT
    ///   swallowed.
    pub fn navigation_finished(&mut self, tab: u64, uri: &str) -> bool {
        // Landed records first: a landing already happened,
        // so this finish belongs to the captured navigation
        // and consuming the record ends the capture.
        let landed_token = self
            .captures
            .values()
            .filter(|c| {
                c.tab == tab && c.chain.iter().any(|u| u == uri) && c.phase == CapturePhase::Landed
            })
            .map(|c| c.token)
            .min();
        if let Some(token) = landed_token {
            self.captures.remove(&token);
            return true;
        }
        let waiting_token = self
            .captures
            .values()
            .filter(|c| {
                c.tab == tab
                    && c.chain.iter().any(|u| u == uri)
                    && c.phase == CapturePhase::AwaitingCookies
                    && !c.finish_seen
            })
            .map(|c| c.token)
            .min();
        if let Some(token) = waiting_token {
            if let Some(capture) = self.captures.get_mut(&token) {
                capture.finish_seen = true;
            }
            return true;
        }
        false
    }

    // -- notice machine --------------------------------------------------

    fn set_notice(&mut self, kind: NoticeKind, name: String, reason: Option<String>) {
        // 0.35 (WS5): an open, unseen Failed notice is
        // sticky — a Started / Finished / Blocked notice
        // must not silently bury it (through 0.34 every
        // set_notice overwrote the single slot, so the
        // failure the user never saw vanished under the
        // next routine toast). A Failed notice replaces
        // anything, including another Failed notice.
        if kind != NoticeKind::Failed {
            if let Some(current) = &self.notice {
                if current.open && current.kind == NoticeKind::Failed {
                    return;
                }
            }
        }
        self.notice_seq = self.notice_seq.saturating_add(1);
        self.notice = Some(Notice {
            seq: self.notice_seq,
            kind,
            name,
            reason,
            open: true,
        });
    }

    fn retire_notice(&mut self) -> bool {
        match &mut self.notice {
            Some(notice) if notice.open => {
                notice.open = false;
                true
            }
            _ => false,
        }
    }

    /// Retire the open notice UNLESS it is a Failed one
    /// (0.35, WS5): the guarded retire for panel open/close
    /// paths in the app (closing panels, opening Teleport
    /// or the palette) — those gestures are not the user
    /// looking at the failure, so an unseen Failed notice
    /// survives them. Genuine exits (notice click,
    /// DismissNotice, panel_opened, page_visited, expiry)
    /// keep the hard `retire_notice`.
    pub fn retire_notice_unless_failed(&mut self) -> Vec<DlEffect> {
        if let Some(notice) = &self.notice {
            if notice.open && notice.kind == NoticeKind::Failed {
                return Vec::new();
            }
        }
        if self.retire_notice() {
            vec![DlEffect::PushChrome]
        } else {
            Vec::new()
        }
    }

    /// The backend's visible state changed. Pulls the
    /// views + version and diffs them against the previous
    /// views:
    ///
    /// - a task newly Done (from Queued/Downloading/
    ///   Paused) → a Finished notice (name + size text);
    /// - a task newly Failed-and-final (not
    ///   `retry_pending`) → a Failed notice and
    ///   `failed_unseen += 1`. "Newly final" includes a
    ///   task that was Failed with a retry pending and is
    ///   now Failed without one (its retries ran out);
    /// - the FIRST call after construction is silent
    ///   (startup restore — 0.33 rule), whatever it holds.
    ///
    /// Any views/version change pushes both surfaces; a
    /// notice change rides the same pushes.
    pub fn backend_changed(&mut self) -> Vec<DlEffect> {
        let views = self.backend.views();
        let version = self.backend.version();
        let first = !self.started;
        self.started = true;
        if !first {
            for view in &views {
                let prev = self.prev_views.iter().find(|p| p.id == view.id);
                match view.status {
                    TaskStatus::Complete => {
                        let was_active = prev.is_some_and(|p| {
                            matches!(
                                p.status,
                                TaskStatus::Queued | TaskStatus::Downloading | TaskStatus::Paused
                            )
                        });
                        if was_active {
                            self.set_notice(
                                NoticeKind::Finished,
                                view.name.clone(),
                                Some(view.size_text.clone()),
                            );
                        }
                    }
                    TaskStatus::Failed if !view.retry_pending => {
                        let newly_final = match prev {
                            Some(p) => {
                                p.status != TaskStatus::Failed
                                    || (p.retry_pending && !view.retry_pending)
                            }
                            None => false,
                        };
                        if newly_final {
                            self.failed_unseen = self.failed_unseen.saturating_add(1);
                            self.set_notice(
                                NoticeKind::Failed,
                                view.name.clone(),
                                view.error.clone(),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        let changed = views != self.prev_views || version != self.prev_version;
        self.prev_views = views;
        self.prev_version = version;
        if changed {
            vec![DlEffect::PushChrome, DlEffect::PushPage]
        } else {
            Vec::new()
        }
    }

    /// The notice's display window ended. Closes the
    /// current notice iff `seq` matches it (an older timer
    /// must never retire a newer notice — the 0.33 rule).
    pub fn notice_expired(&mut self, seq: u64) -> Vec<DlEffect> {
        let matches = self.notice.as_ref().is_some_and(|n| n.seq == seq && n.open);
        if matches && self.retire_notice() {
            vec![DlEffect::PushChrome]
        } else {
            Vec::new()
        }
    }

    /// The downloads panel opened: it IS the list now, so
    /// an open notice retires.
    pub fn panel_opened(&mut self) -> Vec<DlEffect> {
        if self.retire_notice() {
            vec![DlEffect::PushChrome]
        } else {
            Vec::new()
        }
    }

    /// The downloads page was visited: every failed task
    /// counts as seen, and an open notice retires (0.35,
    /// WS5 — the page IS the failure list, so visiting it
    /// is a legitimate exit for even a Failed notice; the
    /// app-side comments claimed this through 0.34 but the
    /// retire never happened here).
    pub fn page_visited(&mut self) -> Vec<DlEffect> {
        let retired = self.retire_notice();
        if self.failed_unseen > 0 {
            self.failed_unseen = 0;
            vec![DlEffect::PushChrome]
        } else if retired {
            vec![DlEffect::PushChrome]
        } else {
            Vec::new()
        }
    }

    /// The notice was clicked: retire it and open the
    /// downloads page.
    pub fn notice_clicked(&mut self) -> Vec<DlEffect> {
        let mut effects = Vec::new();
        if self.retire_notice() {
            effects.push(DlEffect::PushChrome);
        }
        effects.push(DlEffect::OpenDownloadsPage);
        effects
    }

    // -- commands ----------------------------------------------------------

    /// Dispatch one typed command. Task verbs call the
    /// backend and push both surfaces; settings verbs also
    /// update the mirrored settings and emit the persist
    /// effect; routing verbs (`open-page`, `open-panel`,
    /// `close-panel`, `dismiss-notice`) touch no backend
    /// state.
    pub fn dispatch(&mut self, cmd: DlCommand, _source: DlSource) -> Vec<DlEffect> {
        match cmd {
            DlCommand::OpenPanel => vec![DlEffect::OpenPanel],
            DlCommand::ClosePanel => vec![DlEffect::ClosePanel],
            DlCommand::OpenPage => vec![DlEffect::OpenDownloadsPage],
            DlCommand::DismissNotice => {
                if self.retire_notice() {
                    vec![DlEffect::PushChrome]
                } else {
                    Vec::new()
                }
            }
            DlCommand::Pause(id) => {
                self.backend.pause(id);
                pushes()
            }
            DlCommand::Resume(id) => {
                self.backend.resume(id);
                pushes()
            }
            DlCommand::Cancel(id) => {
                self.backend.cancel(id);
                pushes()
            }
            DlCommand::Restart(id) => {
                self.backend.restart(id);
                pushes()
            }
            DlCommand::Refresh(id, url) => {
                self.backend.refresh_link(id, &url);
                pushes()
            }
            DlCommand::Remove(id) => {
                self.backend.remove(id);
                pushes()
            }
            DlCommand::DeleteFile(id) => {
                self.backend.delete_file(id);
                pushes()
            }
            DlCommand::ClearCompleted => {
                self.backend.clear_completed();
                pushes()
            }
            DlCommand::ClearAll => {
                self.backend.clear_all();
                pushes()
            }
            DlCommand::SetConnections(id, n) => {
                self.backend.set_connections(id, n);
                pushes()
            }
            DlCommand::SetDefaultConnections(n) => {
                self.backend.set_default_connections(n);
                self.settings.default_connections = n;
                persist_then_pushes(self.settings)
            }
            DlCommand::SetMaxConcurrent(n) => {
                self.backend.set_max_concurrent(n);
                self.settings.max_concurrent = n;
                persist_then_pushes(self.settings)
            }
            DlCommand::SetSpeedLimitKbps(kbps) => {
                self.backend.set_speed_limit(kbps);
                self.settings.speed_limit_kbps = kbps;
                persist_then_pushes(self.settings)
            }
            DlCommand::OpenFile(id) => match self.backend.completed_path(id) {
                Some(path) => vec![DlEffect::ShellOpenFile(path)],
                None => vec![DlEffect::Log(format!(
                    "download open-file denied: id={id} (not a completed task)"
                ))],
            },
            DlCommand::OpenFolder(id) => {
                let dir = match id {
                    Some(id) => self
                        .backend
                        .completed_path(id)
                        .and_then(|p| p.parent().map(|parent| parent.to_path_buf()))
                        .unwrap_or_else(|| self.backend.downloads_dir()),
                    None => self.backend.downloads_dir(),
                };
                vec![DlEffect::ShellOpenFolder(dir)]
            }
            DlCommand::AddUrl(url) => self.add_url(url),
        }
    }

    /// `add-url` dispatch: triage first (mirroring the
    /// capture reject semantics — a refusal is recorded
    /// and noticed, never silent), then the backend add
    /// with the mirrored default connection count, no
    /// cookies and no referer (a pasted link carries no
    /// page session).
    fn add_url(&mut self, url: String) -> Vec<DlEffect> {
        let host = url_host(&url).unwrap_or_else(|| "unknown host".to_string());
        match triage_download_uri(&url) {
            DownloadTriage::Reject(reason) => {
                let mut effects = vec![DlEffect::Log(format!(
                    "download rejected ({reason}): {}",
                    redact_url(&url)
                ))];
                if !url.trim().is_empty() {
                    let id = self.backend.add_failed(&url, None, reason);
                    let name = id
                        .and_then(|id| self.backend.name_of(id))
                        .unwrap_or_else(|| suggested_or_host("", &url));
                    self.set_notice(NoticeKind::Blocked, name, Some(reason.to_string()));
                    effects.push(DlEffect::PushChrome);
                    effects.push(DlEffect::PushPage);
                }
                effects
            }
            DownloadTriage::Accept => {
                match self.backend.add(
                    &url,
                    None,
                    Vec::new(),
                    self.settings.default_connections,
                    None,
                ) {
                    Some(id) => {
                        let name = self.backend.name_of(id).unwrap_or_else(|| host.clone());
                        self.set_notice(NoticeKind::Started, name, None);
                        vec![
                            DlEffect::Log(format!("download task created: {host} (0 cookies)")),
                            DlEffect::PushChrome,
                            DlEffect::PushPage,
                        ]
                    }
                    None => {
                        self.set_notice(
                            NoticeKind::Failed,
                            host.clone(),
                            Some("Pico couldn't start this download".to_string()),
                        );
                        vec![
                            DlEffect::LogError(format!(
                                "download creation FAILED (manager refused): {host}"
                            )),
                            DlEffect::PushChrome,
                            DlEffect::PushPage,
                        ]
                    }
                }
            }
        }
    }
}

/// The post-mutation pushes: any state-changing verb
/// pushes both surfaces (the snapshot signature gate in
/// the app decides whether anything actually renders).
fn pushes() -> Vec<DlEffect> {
    vec![DlEffect::PushChrome, DlEffect::PushPage]
}

fn persist_then_pushes(settings: DlSettings) -> Vec<DlEffect> {
    vec![
        DlEffect::PersistDownloadsSettings(settings),
        DlEffect::PushChrome,
        DlEffect::PushPage,
    ]
}

/// A fate's name for the capture-time fate-decided log
/// line (0.35, WS2): the variant name alone — a RestoreTo
/// fate's URL is deliberately NOT part of it (the line
/// carries no URLs at all).
fn fate_name(fate: &TabFate) -> &'static str {
    match fate {
        TabFate::None => "None",
        TabFate::RestoreTo(_) => "RestoreTo",
        TabFate::GoBack => "GoBack",
        TabFate::ConvertToStart => "ConvertToStart",
        TabFate::CloseTab => "CloseTab",
    }
}

/// The name a notice speaks when the manager's locked
/// name is unavailable: the suggested name when there is
/// one, else the URL's host.
fn suggested_or_host(suggested: &str, url: &str) -> String {
    if !suggested.is_empty() {
        return suggested.to_string();
    }
    url_host(url).unwrap_or_else(|| "unknown host".to_string())
}

/// The fate revalidation at execution time. The fate was
/// decided ONCE at capture; this only checks the plain
/// landing facts the app supplied:
///
/// - the tab is gone → no fate (a Log line);
/// - the tab's epoch moved → no fate (a Log line: the tab
///   committed a real page since capture and is the
///   user's tab now);
/// - a CloseTab fate on what is NOW the only tab →
///   ConvertToStart (a window never ends with zero tabs);
/// - otherwise the capture-time fate executes.
///
/// AUDIT 0.34.1 (field addition): the only-tab conversion
/// used to be SILENT — the fate changed shape with no log
/// line naming why. It now emits its Log line BEFORE the
/// ExecuteFate effect, so this step returns a Vec (it was
/// an Option through 0.34): drops and plain executions
/// are one-element vectors, the conversion is two.
fn fate_step(capture: &Capture, facts: &LandingFacts) -> Vec<DlEffect> {
    if capture.fate == TabFate::None {
        return Vec::new();
    }
    if !facts.tab_alive {
        return vec![DlEffect::Log(format!(
            "download capture {}: tab gone; fate dropped",
            capture.token
        ))];
    }
    if facts.nav_epoch != capture.epoch {
        return vec![DlEffect::Log(format!(
            "download capture {}: tab moved on; fate dropped",
            capture.token
        ))];
    }
    let mut effects = Vec::new();
    let fate = match &capture.fate {
        TabFate::CloseTab if facts.only_tab => {
            effects.push(DlEffect::Log(format!(
                "download capture {}: tab is now the only tab; close converted to start",
                capture.token
            )));
            TabFate::ConvertToStart
        }
        fate => fate.clone(),
    };
    effects.push(DlEffect::ExecuteFate {
        tab: capture.tab,
        fate,
    });
    effects
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "https://dl2.example.com/soft/s/tool.rar?1794333393";
    const PAGE: &str = "https://example.com/download/tool/";
    const OLDER: &str = "https://example.com/";

    fn facts(
        live: &str,
        committed: Option<&str>,
        prev: Option<&str>,
        origin: CaptureOrigin,
        only_tab: bool,
        traversal: bool,
    ) -> CaptureFacts {
        CaptureFacts {
            live_url: live.to_string(),
            captured_url: FILE.to_string(),
            chain: vec![FILE.to_string()],
            committed_url: committed.map(|s| s.to_string()),
            prev_url: prev.map(|s| s.to_string()),
            origin,
            only_tab,
            traversal_pending: traversal,
            nav_epoch: 7,
        }
    }

    // The fate matrix — the download_tests.rs unwind
    // expectations, re-pointed at decide_tab_fate (the
    // mapping is 1:1: NavigateTo→RestoreTo, Back→GoBack,
    // StartPage→ConvertToStart).

    #[test]
    fn fate_restores_a_committed_real_page() {
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(PAGE),
                Some(OLDER),
                CaptureOrigin::User,
                false,
                false
            )),
            TabFate::RestoreTo(PAGE.to_string())
        );
        // Origin-independent: an adopted tab with a real
        // committed page restores it too.
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(PAGE),
                Some(OLDER),
                CaptureOrigin::Adopted,
                false,
                false
            )),
            TabFate::RestoreTo(PAGE.to_string())
        );
    }

    #[test]
    fn fate_none_when_the_tab_moved_on_or_a_traversal_is_pending() {
        assert_eq!(
            decide_tab_fate(&facts(
                PAGE,
                Some(PAGE),
                Some(OLDER),
                CaptureOrigin::User,
                false,
                false
            )),
            TabFate::None,
            "live != captured"
        );
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(PAGE),
                Some(OLDER),
                CaptureOrigin::Adopted,
                false,
                true
            )),
            TabFate::None,
            "traversal pending dominates origin"
        );
    }

    #[test]
    fn fate_goes_back_when_the_file_itself_committed_over_a_page() {
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(FILE),
                Some(PAGE),
                CaptureOrigin::User,
                false,
                false
            )),
            TabFate::GoBack
        );
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(FILE),
                Some(PAGE),
                CaptureOrigin::Adopted,
                false,
                false
            )),
            TabFate::GoBack
        );
    }

    #[test]
    fn fate_closes_an_adopted_download_only_tab() {
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                None,
                None,
                CaptureOrigin::Adopted,
                false,
                false
            )),
            TabFate::CloseTab
        );
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(FILE),
                None,
                CaptureOrigin::Adopted,
                false,
                false
            )),
            TabFate::CloseTab
        );
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(FILE),
                Some(FILE),
                CaptureOrigin::Adopted,
                false,
                false
            )),
            TabFate::CloseTab,
            "prev == file would loop a Back, so it closes"
        );
    }

    #[test]
    fn fate_closes_an_adopted_tab_through_a_redirect_chain() {
        // 0.35, WS2 — his exact shape at the matrix level:
        // the tab's live URL is the PRE-redirect hop, the
        // captured URL is the POST-redirect final, and the
        // chain carries both. The pre-0.35 exact-match gate
        // decided None here; the chain gate closes.
        const HOP: &str = "https://dl2.example.com/get/tool.rar";
        let mut f = facts(HOP, None, None, CaptureOrigin::Adopted, false, false);
        f.chain = vec![HOP.to_string(), FILE.to_string()];
        assert_eq!(decide_tab_fate(&f), TabFate::CloseTab);
        // A live URL outside the chain still refuses, chain
        // or no chain: a genuine navigation is not this
        // capture's tab to close.
        let mut moved = facts(PAGE, None, None, CaptureOrigin::Adopted, false, false);
        moved.chain = vec![HOP.to_string(), FILE.to_string()];
        assert_eq!(decide_tab_fate(&moved), TabFate::None);
    }

    #[test]
    fn fate_converts_when_closing_is_not_allowed() {
        // The only tab never closes.
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                None,
                None,
                CaptureOrigin::Adopted,
                true,
                false
            )),
            TabFate::ConvertToStart
        );
        // A user's own tab never closes this way.
        assert_eq!(
            decide_tab_fate(&facts(FILE, None, None, CaptureOrigin::User, false, false)),
            TabFate::ConvertToStart
        );
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(FILE),
                None,
                CaptureOrigin::User,
                false,
                false
            )),
            TabFate::ConvertToStart
        );
        assert_eq!(
            decide_tab_fate(&facts(
                FILE,
                Some(FILE),
                Some(FILE),
                CaptureOrigin::User,
                false,
                false
            )),
            TabFate::ConvertToStart
        );
    }

    #[test]
    fn snapshot_serializes_with_the_frozen_field_names() {
        // Worker 3's JS reads exactly these keys.
        let view = DlTaskView {
            id: 7,
            name: "tool.rar".to_string(),
            host: "dl2.example.com".to_string(),
            status: TaskStatus::Complete,
            total: Some(100),
            downloaded: 100,
            percent: Some(100.0),
            size_text: "100 B".to_string(),
            detail_text: "100 B".to_string(),
            connections: 8,
            error: None,
            url: FILE.to_string(),
            added_ms: 1,
            sha256: None,
            hint: None,
            retry_text: None,
            retry_pending: false,
            refreshing: false,
            speed_bps: None,
            eta_secs: None,
        };
        let snap = DlSnapshot {
            version: 3,
            tasks: vec![view],
            active_count: 0,
            failed_unseen: 2,
            settings: DlSettings {
                default_connections: 8,
                max_concurrent: 3,
                speed_limit_kbps: None,
            },
            notice: Some(Notice {
                seq: 1,
                kind: NoticeKind::Finished,
                name: "tool.rar".to_string(),
                reason: Some("100 B".to_string()),
                open: true,
            }),
            card: DlCardView::empty(),
        };
        let json = serde_json::to_value(&snap).expect("serialize");
        assert_eq!(json["version"], 3);
        assert_eq!(json["active_count"], 0);
        assert_eq!(json["failed_unseen"], 2);
        assert_eq!(json["settings"]["default_connections"], 8);
        assert_eq!(json["settings"]["max_concurrent"], 3);
        assert_eq!(
            json["settings"]["speed_limit_kbps"],
            serde_json::Value::Null
        );
        assert_eq!(
            json["tasks"][0]["status"], "done",
            "status is the core label"
        );
        assert_eq!(json["tasks"][0]["size_text"], "100 B");
        assert_eq!(
            json["tasks"][0]["speed_bps"],
            serde_json::Value::Null,
            "0.35: the producer fields ride the snapshot under exactly these names"
        );
        assert_eq!(json["tasks"][0]["eta_secs"], serde_json::Value::Null);
        assert_eq!(json["notice"]["kind"], "finished");
        assert_eq!(json["notice"]["seq"], 1);
        assert_eq!(json["notice"]["open"], true);
        // 0.35.1: the card rides the snapshot under exactly
        // these snake_case names — pinned here for the
        // empty case (the literal above carries the empty
        // card).
        assert_eq!(json["card"]["state"], "empty");
        assert_eq!(json["card"]["count"], 0);
        assert_eq!(json["card"]["speed_bps"], serde_json::Value::Null);
        assert_eq!(json["card"]["paused"], 0);
        assert_eq!(json["card"]["queued"], 0);
        assert_eq!(json["card"]["newest_name"], serde_json::Value::Null);
        assert_eq!(json["card"]["unseen"], false);
    }

    // -- card_view (0.35.1) ---------------------------------------------

    fn card_task(
        name: &str,
        status: TaskStatus,
        added_ms: u64,
        speed_bps: Option<u64>,
    ) -> DlTaskView {
        DlTaskView {
            id: added_ms,
            name: name.to_string(),
            host: "dl2.example.com".to_string(),
            status,
            total: None,
            downloaded: 0,
            percent: None,
            size_text: String::new(),
            detail_text: String::new(),
            connections: 1,
            error: None,
            url: FILE.to_string(),
            added_ms,
            sha256: None,
            hint: None,
            retry_text: None,
            retry_pending: false,
            refreshing: false,
            speed_bps,
            eta_secs: None,
        }
    }

    #[test]
    fn card_empty_when_no_tasks() {
        let card = card_view(&[], 0);
        assert_eq!(card, DlCardView::empty());
        assert_eq!(card.state, "empty");
        assert_eq!(card.count, 0);
        assert_eq!(card.speed_bps, None);
    }

    #[test]
    fn card_active_sums_speeds_and_counts_downloading() {
        let tasks = vec![
            card_task("a.bin", TaskStatus::Downloading, 10, Some(100)),
            card_task("b.bin", TaskStatus::Downloading, 20, None),
            card_task("c.bin", TaskStatus::Downloading, 30, Some(250)),
            card_task("d.bin", TaskStatus::Paused, 40, Some(999)),
        ];
        let card = card_view(&tasks, 0);
        assert_eq!(card.state, "active");
        assert_eq!(card.count, 3);
        assert_eq!(
            card.speed_bps,
            Some(350),
            "None counts as 0; non-downloading speeds never join the sum"
        );
        assert_eq!(card.paused, 1, "paused stays truthful in active");
        assert_eq!(card.queued, 0);
        assert_eq!(card.newest_name, None);
        assert!(!card.unseen);
    }

    #[test]
    fn card_waiting_counts_paused_plus_queued() {
        let tasks = vec![
            card_task("a.bin", TaskStatus::Paused, 10, None),
            card_task("b.bin", TaskStatus::Queued, 20, None),
            card_task("c.bin", TaskStatus::Queued, 30, None),
        ];
        let card = card_view(&tasks, 0);
        assert_eq!(card.state, "waiting");
        assert_eq!(card.count, 3);
        assert_eq!(card.paused, 1);
        assert_eq!(card.queued, 2);
        assert_eq!(card.speed_bps, None);
        assert_eq!(card.newest_name, None);
    }

    #[test]
    fn card_finished_names_newest_complete_by_added_ms_not_vec_order() {
        // The newest Complete sits FIRST in the vec: the
        // pick must follow added_ms, not position.
        let tasks = vec![
            card_task("newest.zip", TaskStatus::Complete, 900, None),
            card_task("older.zip", TaskStatus::Complete, 100, None),
            card_task("middle.zip", TaskStatus::Complete, 500, None),
        ];
        let card = card_view(&tasks, 0);
        assert_eq!(card.state, "finished");
        assert_eq!(card.count, 3);
        assert_eq!(card.newest_name.as_deref(), Some("newest.zip"));
        assert_eq!(card.speed_bps, None);
        assert!(!card.unseen);
    }

    #[test]
    fn card_failed_unseen_beats_active() {
        let tasks = vec![
            card_task("run.bin", TaskStatus::Downloading, 10, Some(500)),
            card_task("old-fail.bin", TaskStatus::Failed, 20, None),
            card_task("new-fail.bin", TaskStatus::Failed, 30, None),
        ];
        let card = card_view(&tasks, 2);
        assert_eq!(card.state, "failed");
        assert_eq!(card.count, 2, "count is the unseen count itself");
        assert!(card.unseen);
        assert_eq!(card.newest_name.as_deref(), Some("new-fail.bin"));
        assert_eq!(card.speed_bps, None, "speed rides only the active state");
    }

    #[test]
    fn card_active_beats_waiting_and_waiting_beats_finished() {
        let active = vec![
            card_task("done.zip", TaskStatus::Complete, 10, None),
            card_task("wait.bin", TaskStatus::Paused, 20, None),
            card_task("run.bin", TaskStatus::Downloading, 30, Some(1)),
        ];
        assert_eq!(card_view(&active, 0).state, "active");
        let waiting = vec![
            card_task("done.zip", TaskStatus::Complete, 10, None),
            card_task("wait.bin", TaskStatus::Queued, 20, None),
        ];
        let card = card_view(&waiting, 0);
        assert_eq!(card.state, "waiting");
        assert_eq!(card.count, 1);
    }

    #[test]
    fn card_seen_failed_is_the_fallback() {
        // Only seen-failed tasks remain (failed_unseen == 0):
        // still "failed", but unseen is false and the count
        // is the total Failed.
        let tasks = vec![
            card_task("first.bin", TaskStatus::Failed, 10, None),
            card_task("second.bin", TaskStatus::Failed, 20, None),
        ];
        let card = card_view(&tasks, 0);
        assert_eq!(card.state, "failed");
        assert_eq!(card.count, 2);
        assert!(!card.unseen);
        assert_eq!(card.newest_name.as_deref(), Some("second.bin"));
        // A Complete alongside a seen failure still reads
        // finished — the fallback is only for when nothing
        // else remains.
        let mixed = vec![
            card_task("fail.bin", TaskStatus::Failed, 10, None),
            card_task("done.zip", TaskStatus::Complete, 20, None),
        ];
        assert_eq!(card_view(&mixed, 0).state, "finished");
    }
}
