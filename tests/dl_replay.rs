//! 0.34 downloads coordinator: replay tests.
//!
//! A scripted `FakeBackend` (it records every verb call in
//! order and serves scripted views) drives
//! `core::dlcoord::Coordinator` through the field stories
//! that broke across the 0.32.x patch rounds and 0.33:
//! the soft98 adopted-tab capture, the finish/cookie race
//! orderings, the user-moved-on epoch guard, the triage
//! reject, command dispatch, and the notice machine.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use pico_core::core::dlcmd::DlCommand;
use pico_core::core::dlcoord::{
    CaptureFacts, CaptureOrigin, Coordinator, DlBackend, DlEffect, DlSettings, DlSource,
    DlTaskView, LandingFacts, NoticeKind, TabFate,
};
use pico_core::core::download::{file_name_from_url, url_host, CookiePair, TaskStatus};

const FILE: &str = "https://dl2.example.com/soft/s/tool.rar?1794333393";

// ---------------------------------------------------------------------------
// FakeBackend
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeInner {
    /// Every backend verb call, in order ("add",
    /// "pause(7)", "set_max_concurrent(5)", …).
    log: Vec<String>,
    views: Vec<DlTaskView>,
    version: u64,
    next_id: u64,
    refuse_add: bool,
    names: HashMap<u64, String>,
    dir: PathBuf,
}

#[derive(Clone)]
struct FakeBackend {
    inner: Rc<RefCell<FakeInner>>,
}

impl FakeBackend {
    fn new() -> FakeBackend {
        FakeBackend {
            inner: Rc::new(RefCell::new(FakeInner {
                next_id: 1,
                dir: PathBuf::from("/downloads"),
                ..FakeInner::default()
            })),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.inner.borrow().log.clone()
    }

    fn set_views(&self, views: Vec<DlTaskView>) {
        let mut inner = self.inner.borrow_mut();
        inner.views = views;
        inner.version += 1;
    }

    fn set_refuse_add(&self, refuse: bool) {
        self.inner.borrow_mut().refuse_add = refuse;
    }
}

impl DlBackend for FakeBackend {
    fn add(
        &self,
        url: &str,
        suggested: Option<&str>,
        cookies: Vec<CookiePair>,
        connections: u32,
        _referer: Option<String>,
    ) -> Option<u64> {
        let mut inner = self.inner.borrow_mut();
        inner.log.push(format!(
            "add(cookies={}, connections={connections})",
            cookies.len()
        ));
        if inner.refuse_add {
            return None;
        }
        let id = inner.next_id;
        inner.next_id += 1;
        let name = match suggested {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => file_name_from_url(url),
        };
        inner.names.insert(id, name.clone());
        inner.views.push(DlTaskView {
            id,
            name,
            host: url_host(url).unwrap_or_default(),
            status: TaskStatus::Queued,
            total: None,
            downloaded: 0,
            percent: None,
            size_text: "Unknown size".to_string(),
            detail_text: "Queued".to_string(),
            connections,
            error: None,
            url: url.to_string(),
            added_ms: 0,
            sha256: None,
            hint: None,
            retry_text: None,
            retry_pending: false,
            refreshing: false,
            speed_bps: None,
            eta_secs: None,
        });
        inner.version += 1;
        Some(id)
    }

    fn add_failed(&self, url: &str, suggested: Option<&str>, reason: &str) -> Option<u64> {
        let mut inner = self.inner.borrow_mut();
        inner.log.push(format!("add_failed({reason})"));
        let id = inner.next_id;
        inner.next_id += 1;
        let name = match suggested {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => file_name_from_url(url),
        };
        inner.names.insert(id, name);
        inner.version += 1;
        Some(id)
    }

    fn pause(&self, id: u64) {
        self.inner.borrow_mut().log.push(format!("pause({id})"));
    }
    fn resume(&self, id: u64) {
        self.inner.borrow_mut().log.push(format!("resume({id})"));
    }
    fn cancel(&self, id: u64) {
        self.inner.borrow_mut().log.push(format!("cancel({id})"));
    }
    fn restart(&self, id: u64) {
        self.inner.borrow_mut().log.push(format!("restart({id})"));
    }
    fn refresh_link(&self, id: u64, _url: &str) {
        self.inner
            .borrow_mut()
            .log
            .push(format!("refresh_link({id})"));
    }
    fn remove(&self, id: u64) {
        self.inner.borrow_mut().log.push(format!("remove({id})"));
    }
    fn delete_file(&self, id: u64) {
        self.inner
            .borrow_mut()
            .log
            .push(format!("delete_file({id})"));
    }
    fn clear_completed(&self) {
        self.inner
            .borrow_mut()
            .log
            .push("clear_completed".to_string());
    }
    fn clear_all(&self) {
        self.inner.borrow_mut().log.push("clear_all".to_string());
    }
    fn set_connections(&self, id: u64, n: u32) {
        self.inner
            .borrow_mut()
            .log
            .push(format!("set_connections({id}, {n})"));
    }
    fn set_default_connections(&self, n: u32) {
        self.inner
            .borrow_mut()
            .log
            .push(format!("set_default_connections({n})"));
    }
    fn set_max_concurrent(&self, n: u32) {
        self.inner
            .borrow_mut()
            .log
            .push(format!("set_max_concurrent({n})"));
    }
    fn set_speed_limit(&self, kbps: Option<u64>) {
        self.inner
            .borrow_mut()
            .log
            .push(format!("set_speed_limit({kbps:?})"));
    }
    fn views(&self) -> Vec<DlTaskView> {
        self.inner.borrow().views.clone()
    }
    fn version(&self) -> u64 {
        self.inner.borrow().version
    }
    fn active_count(&self) -> usize {
        self.inner
            .borrow()
            .views
            .iter()
            .filter(|v| v.status == TaskStatus::Downloading)
            .count()
    }
    fn failed_count(&self) -> usize {
        self.inner
            .borrow()
            .views
            .iter()
            .filter(|v| v.status == TaskStatus::Failed)
            .count()
    }
    fn name_of(&self, id: u64) -> Option<String> {
        self.inner.borrow().names.get(&id).cloned()
    }
    fn completed_path(&self, id: u64) -> Option<PathBuf> {
        let inner = self.inner.borrow();
        let view = inner.views.iter().find(|v| v.id == id)?;
        if view.status == TaskStatus::Complete {
            Some(inner.dir.join(&view.name))
        } else {
            None
        }
    }
    fn downloads_dir(&self) -> PathBuf {
        self.inner.borrow().dir.clone()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn settings() -> DlSettings {
    DlSettings {
        default_connections: 8,
        max_concurrent: 3,
        speed_limit_kbps: None,
    }
}

fn coordinator() -> (FakeBackend, Coordinator<FakeBackend>) {
    let backend = FakeBackend::new();
    let coord = Coordinator::new(backend.clone(), settings());
    (backend, coord)
}

/// The soft98 story's capture facts: an adopted tab whose
/// live URL is the file, nothing committed, no previous
/// page, not the only tab, epoch 7.
fn adopted_facts() -> CaptureFacts {
    CaptureFacts {
        live_url: FILE.to_string(),
        captured_url: FILE.to_string(),
        chain: vec![FILE.to_string()],
        committed_url: None,
        prev_url: None,
        origin: CaptureOrigin::Adopted,
        only_tab: false,
        traversal_pending: false,
        nav_epoch: 7,
    }
}

fn landing(epoch: u64) -> LandingFacts {
    LandingFacts {
        tab_alive: true,
        nav_epoch: epoch,
        only_tab: false,
    }
}

fn cookie(name: &str) -> CookiePair {
    CookiePair {
        name: name.to_string(),
        value: "v".to_string(),
        domain: "example.com".to_string(),
        host_only: false,
        secure: false,
        path: "/".to_string(),
        expires_unix_ms: None,
    }
}

fn view(id: u64, status: TaskStatus) -> DlTaskView {
    DlTaskView {
        id,
        name: format!("file{id}.bin"),
        host: "example.com".to_string(),
        status,
        total: Some(2_048),
        downloaded: 0,
        percent: Some(0.0),
        size_text: "2 KB".to_string(),
        detail_text: String::new(),
        connections: 8,
        error: None,
        url: "https://example.com/file.bin".to_string(),
        added_ms: 0,
        sha256: None,
        hint: None,
        retry_text: None,
        retry_pending: false,
        refreshing: false,
        speed_bps: None,
        eta_secs: None,
    }
}

fn has_fate(effects: &[DlEffect], tab: u64, fate: TabFate) -> bool {
    effects.contains(&DlEffect::ExecuteFate { tab, fate })
}

// ---------------------------------------------------------------------------
// Story 1: the soft98 capture, cookies first
// ---------------------------------------------------------------------------

#[test]
fn story1_soft98_capture_cookies_then_finish() {
    let (backend, mut coord) = coordinator();
    let (token, effects) =
        coord.capture_started(42, FILE.to_string(), String::new(), None, adopted_facts());
    // 0.35, WS2: an accepted capture returns exactly the
    // fate-decided log line (no silent fates) and still
    // waits for cookies — no task, no fate execution yet.
    assert_eq!(
        effects,
        vec![DlEffect::Log(format!(
            "download capture {token}: fate decided CloseTab"
        ))],
        "accept waits for cookies, but the fate decision is logged"
    );
    assert!(backend.calls().is_empty());

    let effects = coord.cookies_arrived(token, vec![cookie("a"), cookie("b")], landing(7));
    assert_eq!(
        backend.calls(),
        vec!["add(cookies=2, connections=8)"],
        "exactly one backend add"
    );
    assert!(
        effects.contains(&DlEffect::Log(
            "download task created: dl2.example.com (2 cookies)".to_string()
        )),
        "{effects:?}"
    );
    let notice = coord.snapshot().notice.expect("a notice");
    assert_eq!(notice.kind, NoticeKind::Started);
    assert_eq!(notice.name, "tool.rar", "the manager's locked name");
    assert!(notice.open);
    assert!(has_fate(&effects, 42, TabFate::CloseTab), "{effects:?}");

    assert!(
        coord.navigation_finished(42, FILE),
        "the finish is swallowed"
    );
    assert!(
        !coord.navigation_finished(42, FILE),
        "the Landed record was consumed: a second finish is a new navigation"
    );
}

// ---------------------------------------------------------------------------
// Story 2: the finish arrives BEFORE the cookies
// ---------------------------------------------------------------------------

#[test]
fn story2_finish_before_cookies_still_lands_the_fate() {
    let (backend, mut coord) = coordinator();
    let (token, _) =
        coord.capture_started(42, FILE.to_string(), String::new(), None, adopted_facts());
    assert!(
        coord.navigation_finished(42, FILE),
        "an AwaitingCookies finish is swallowed"
    );
    assert!(
        !coord.navigation_finished(42, FILE),
        "once finish_seen, the same record swallows nothing more"
    );
    let effects = coord.cookies_arrived(token, vec![cookie("a")], landing(7));
    assert_eq!(backend.calls().len(), 1, "the task still lands");
    assert!(
        has_fate(&effects, 42, TabFate::CloseTab),
        "the fate still lands at cookies: {effects:?}"
    );
    // The record is Landed now: one more finish (the real
    // completion of the captured navigation, whenever it
    // arrives) swallows and consumes it.
    assert!(coord.navigation_finished(42, FILE));
    assert!(!coord.navigation_finished(42, FILE));
}

#[test]
fn cookie_race_loser_logs_once_and_adds_nothing() {
    let (backend, mut coord) = coordinator();
    let (token, _) =
        coord.capture_started(42, FILE.to_string(), String::new(), None, adopted_facts());
    let effects = coord.cookies_timeout(token, landing(7));
    assert!(has_fate(&effects, 42, TabFate::CloseTab));
    assert_eq!(backend.calls().len(), 1, "timeout creates the task");
    let loser = coord.cookies_arrived(token, vec![cookie("a")], landing(7));
    assert_eq!(
        loser,
        vec![DlEffect::Log(format!(
            "download capture {token}: late cookie answer ignored"
        ))]
    );
    assert_eq!(backend.calls().len(), 1, "the loser adds nothing");
}

// ---------------------------------------------------------------------------
// Story 3: the user moved on — the epoch guard drops the fate
// ---------------------------------------------------------------------------

#[test]
fn story3_epoch_moved_on_drops_the_fate_with_one_log_line() {
    let (backend, mut coord) = coordinator();
    let (token, _) =
        coord.capture_started(42, FILE.to_string(), String::new(), None, adopted_facts());
    let effects = coord.cookies_arrived(token, vec![cookie("a")], landing(8));
    assert_eq!(backend.calls().len(), 1, "the download itself still lands");
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, DlEffect::ExecuteFate { .. })),
        "no fate executes: {effects:?}"
    );
    let logs: Vec<&DlEffect> = effects
        .iter()
        .filter(|e| matches!(e, DlEffect::Log(s) if s.contains("fate dropped")))
        .collect();
    assert_eq!(
        logs.len(),
        1,
        "exactly one fate-dropped log line: {effects:?}"
    );
}

#[test]
fn fate_revalidation_converts_close_to_start_for_a_now_only_tab() {
    let (_backend, mut coord) = coordinator();
    let (token, _) =
        coord.capture_started(42, FILE.to_string(), String::new(), None, adopted_facts());
    let mut facts = landing(7);
    facts.only_tab = true;
    let effects = coord.cookies_arrived(token, Vec::new(), facts);
    assert!(
        has_fate(&effects, 42, TabFate::ConvertToStart),
        "CloseTab on a now-only tab converts: {effects:?}"
    );
    // And a dead tab drops the fate with its own log line.
    let (token2, _) =
        coord.capture_started(43, FILE.to_string(), String::new(), None, adopted_facts());
    let mut facts = landing(7);
    facts.tab_alive = false;
    let effects = coord.cookies_timeout(token2, facts);
    assert!(
        effects.contains(&DlEffect::Log(format!(
            "download capture {token2}: tab gone; fate dropped"
        ))),
        "{effects:?}"
    );
}

#[test]
fn fate_only_tab_conversion_logs_its_reason_before_the_fate() {
    // AUDIT 0.34.1 (field addition): the CloseTab →
    // ConvertToStart conversion at landing used to be
    // silent. The effect stream must now end with exactly
    // [Log(reason), ExecuteFate{ConvertToStart}], in that
    // order — the reason names itself before the fate runs.
    let (_backend, mut coord) = coordinator();
    let (token, _) =
        coord.capture_started(42, FILE.to_string(), String::new(), None, adopted_facts());
    let mut facts = landing(7);
    facts.only_tab = true;
    let effects = coord.cookies_arrived(token, Vec::new(), facts);
    assert!(effects.len() >= 2, "{effects:?}");
    let tail = &effects[effects.len() - 2..];
    assert_eq!(
        tail,
        [
            DlEffect::Log(format!(
                "download capture {token}: tab is now the only tab; close converted to start"
            )),
            DlEffect::ExecuteFate {
                tab: 42,
                fate: TabFate::ConvertToStart
            },
        ],
        "the conversion reason precedes the converted fate: {effects:?}"
    );
    // The two pre-existing drop lines still fire verbatim.
    let (token2, _) =
        coord.capture_started(43, FILE.to_string(), String::new(), None, adopted_facts());
    let effects = coord.cookies_arrived(token2, Vec::new(), landing(8));
    assert!(
        effects.contains(&DlEffect::Log(format!(
            "download capture {token2}: tab moved on; fate dropped"
        ))),
        "{effects:?}"
    );
}

// ---------------------------------------------------------------------------
// Story 4: the triage reject
// ---------------------------------------------------------------------------

#[test]
fn story4_blob_reject_records_failed_and_executes_the_fate() {
    let (backend, mut coord) = coordinator();
    let blob = "blob:https://example.com/9f2c";
    let facts = CaptureFacts {
        live_url: blob.to_string(),
        captured_url: blob.to_string(),
        chain: vec![blob.to_string()],
        ..adopted_facts()
    };
    let (_token, effects) = coord.capture_started(42, blob.to_string(), String::new(), None, facts);
    assert_eq!(
        backend.calls(),
        vec!["add_failed(Pico can only download http/https links)"]
    );
    let notice = coord.snapshot().notice.expect("a notice");
    assert_eq!(notice.kind, NoticeKind::Blocked);
    assert_eq!(
        notice.reason.as_deref(),
        Some("Pico can only download http/https links")
    );
    assert!(
        has_fate(&effects, 42, TabFate::CloseTab),
        "the reject fate executes immediately: {effects:?}"
    );
    assert!(
        coord.navigation_finished(42, blob),
        "the rejected capture's finish is swallowed too"
    );
}

#[test]
fn manager_refusal_at_landing_is_a_failed_notice_not_silence() {
    let (backend, mut coord) = coordinator();
    backend.set_refuse_add(true);
    let (token, _) = coord.capture_started(
        42,
        FILE.to_string(),
        "tool.rar".to_string(),
        None,
        adopted_facts(),
    );
    let effects = coord.cookies_arrived(token, Vec::new(), landing(7));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, DlEffect::LogError(s) if s.contains("download creation FAILED"))),
        "{effects:?}"
    );
    let notice = coord.snapshot().notice.expect("a notice");
    assert_eq!(notice.kind, NoticeKind::Failed);
    assert_eq!(notice.name, "tool.rar", "the suggested name speaks");
    assert_eq!(
        notice.reason.as_deref(),
        Some("Pico couldn't start this download")
    );
    assert!(
        has_fate(&effects, 42, TabFate::CloseTab),
        "the fate still lands"
    );
}

// ---------------------------------------------------------------------------
// Story 5: dispatch
// ---------------------------------------------------------------------------

#[test]
fn story5_dispatch_routes_every_verb() {
    let (backend, mut coord) = coordinator();

    assert_eq!(
        coord.dispatch(DlCommand::OpenPage, DlSource::Page),
        vec![DlEffect::OpenDownloadsPage]
    );
    assert_eq!(
        coord.dispatch(DlCommand::OpenPanel, DlSource::Chrome),
        vec![DlEffect::OpenPanel]
    );
    assert_eq!(
        coord.dispatch(DlCommand::ClosePanel, DlSource::Chrome),
        vec![DlEffect::ClosePanel]
    );

    let effects = coord.dispatch(DlCommand::Pause(7), DlSource::Chrome);
    assert_eq!(backend.calls(), vec!["pause(7)"]);
    assert_eq!(effects, vec![DlEffect::PushChrome, DlEffect::PushPage]);

    let effects = coord.dispatch(DlCommand::SetMaxConcurrent(5), DlSource::Page);
    assert_eq!(backend.calls(), vec!["pause(7)", "set_max_concurrent(5)"]);
    assert!(
        effects.contains(&DlEffect::PersistDownloadsSettings(DlSettings {
            default_connections: 8,
            max_concurrent: 5,
            speed_limit_kbps: None,
        })),
        "{effects:?}"
    );
    assert_eq!(coord.settings().max_concurrent, 5);

    coord.dispatch(DlCommand::SetSpeedLimitKbps(Some(1024)), DlSource::Page);
    coord.dispatch(DlCommand::SetDefaultConnections(16), DlSource::Chrome);
    coord.dispatch(DlCommand::SetConnections(7, 4), DlSource::Chrome);
    coord.dispatch(
        DlCommand::Refresh(7, "https://example.com/new".to_string()),
        DlSource::Page,
    );
    coord.dispatch(DlCommand::Remove(7), DlSource::Page);
    coord.dispatch(DlCommand::Cancel(7), DlSource::Chrome);
    coord.dispatch(DlCommand::DeleteFile(7), DlSource::Page);
    coord.dispatch(DlCommand::ClearCompleted, DlSource::Page);
    coord.dispatch(DlCommand::ClearAll, DlSource::Page);
    assert_eq!(
        backend.calls(),
        vec![
            "pause(7)",
            "set_max_concurrent(5)",
            "set_speed_limit(Some(1024))",
            "set_default_connections(16)",
            "set_connections(7, 4)",
            "refresh_link(7)",
            "remove(7)",
            "cancel(7)",
            "delete_file(7)",
            "clear_completed",
            "clear_all",
        ]
    );
    assert_eq!(coord.settings().speed_limit_kbps, Some(1024));
    assert_eq!(coord.settings().default_connections, 16);
}

#[test]
fn story5_open_file_only_for_a_completed_task() {
    let (backend, mut coord) = coordinator();
    backend.set_views(vec![
        view(7, TaskStatus::Complete),
        view(8, TaskStatus::Downloading),
    ]);
    let effects = coord.dispatch(DlCommand::OpenFile(7), DlSource::Page);
    assert_eq!(
        effects,
        vec![DlEffect::ShellOpenFile(PathBuf::from(
            "/downloads/file7.bin"
        ))]
    );
    let effects = coord.dispatch(DlCommand::OpenFile(8), DlSource::Page);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, DlEffect::ShellOpenFile(_))),
        "no shell effect for a downloading task: {effects:?}"
    );
    assert!(
        effects.iter().any(|e| matches!(e, DlEffect::Log(_))),
        "the denial is logged: {effects:?}"
    );
    // Open folder: no id is the downloads folder itself;
    // a completed task's folder is its parent (the same
    // directory in the manager's layout).
    assert_eq!(
        coord.dispatch(DlCommand::OpenFolder(None), DlSource::Chrome),
        vec![DlEffect::ShellOpenFolder(PathBuf::from("/downloads"))]
    );
    assert_eq!(
        coord.dispatch(DlCommand::OpenFolder(Some(7)), DlSource::Page),
        vec![DlEffect::ShellOpenFolder(PathBuf::from("/downloads"))]
    );
}

#[test]
fn story5_add_url_dispatch_triages_and_notices() {
    let (backend, mut coord) = coordinator();
    let effects = coord.dispatch(
        DlCommand::AddUrl("https://example.com/f.zip".to_string()),
        DlSource::Page,
    );
    assert_eq!(backend.calls(), vec!["add(cookies=0, connections=8)"]);
    assert!(effects.contains(&DlEffect::PushChrome));
    assert_eq!(
        coord.snapshot().notice.map(|n| n.kind),
        Some(NoticeKind::Started)
    );

    let effects = coord.dispatch(
        DlCommand::AddUrl("file:///C:/f.zip".to_string()),
        DlSource::Page,
    );
    // (parse would have denied this body; dispatch triages
    // programmatically-built commands the same way.)
    assert!(
        backend.calls().iter().any(|c| c.starts_with("add_failed")),
        "{:?}",
        backend.calls()
    );
    assert!(effects.contains(&DlEffect::PushChrome));
    assert_eq!(
        coord.snapshot().notice.map(|n| n.kind),
        Some(NoticeKind::Blocked)
    );
}

// ---------------------------------------------------------------------------
// Story 6: the notice machine
// ---------------------------------------------------------------------------

#[test]
fn story6_backend_changed_notice_machine() {
    let (backend, mut coord) = coordinator();

    // Startup silence: the first backend_changed never
    // notices, even with a Done task already present.
    backend.set_views(vec![view(1, TaskStatus::Complete)]);
    coord.backend_changed();
    assert!(
        coord.snapshot().notice.is_none(),
        "a restored Done task stays silent"
    );

    // A task flipping to Done → a Finished notice
    // (name + size text).
    backend.set_views(vec![view(2, TaskStatus::Downloading)]);
    coord.backend_changed();
    assert!(coord.snapshot().notice.is_none());
    let mut done = view(2, TaskStatus::Complete);
    done.downloaded = 2_048;
    backend.set_views(vec![done]);
    let effects = coord.backend_changed();
    assert!(effects.contains(&DlEffect::PushChrome));
    let notice = coord.snapshot().notice.expect("finished notice");
    assert_eq!(notice.kind, NoticeKind::Finished);
    assert_eq!(notice.name, "file2.bin");
    assert_eq!(
        notice.reason.as_deref(),
        Some("2 KB"),
        "the size rides along"
    );
    assert_eq!(coord.snapshot().failed_unseen, 0);

    // A task flipping to Failed (final) → a Failed notice
    // and failed_unseen 1.
    backend.set_views(vec![view(3, TaskStatus::Downloading)]);
    coord.backend_changed();
    let mut failed = view(3, TaskStatus::Failed);
    failed.error = Some("HTTP 404".to_string());
    backend.set_views(vec![failed]);
    coord.backend_changed();
    let notice = coord.snapshot().notice.expect("failed notice");
    assert_eq!(notice.kind, NoticeKind::Failed);
    assert_eq!(notice.reason.as_deref(), Some("HTTP 404"));
    assert_eq!(coord.snapshot().failed_unseen, 1);

    // Visiting the page zeroes the unseen count.
    coord.page_visited();
    assert_eq!(coord.snapshot().failed_unseen, 0);
}

#[test]
fn notice_retry_pending_failed_is_not_final_until_the_retry_clears() {
    let (backend, mut coord) = coordinator();
    backend.set_views(vec![view(1, TaskStatus::Downloading)]);
    coord.backend_changed(); // startup
    let mut retrying = view(1, TaskStatus::Failed);
    retrying.retry_pending = true;
    retrying.retry_text = Some("Retrying in 5 · attempt 1 of 3".to_string());
    backend.set_views(vec![retrying]);
    coord.backend_changed();
    assert!(
        coord.snapshot().notice.is_none(),
        "a retry-pending failure is not final: no toast yet"
    );
    assert_eq!(coord.snapshot().failed_unseen, 0);
    // The retries run out: same Failed status, no longer
    // pending — THAT is the final failure.
    backend.set_views(vec![view(1, TaskStatus::Failed)]);
    coord.backend_changed();
    assert_eq!(
        coord.snapshot().notice.map(|n| n.kind),
        Some(NoticeKind::Failed)
    );
    assert_eq!(coord.snapshot().failed_unseen, 1);
}

#[test]
fn notice_expiry_is_seq_checked_and_click_opens_the_page() {
    let (backend, mut coord) = coordinator();
    backend.set_views(vec![view(1, TaskStatus::Downloading)]);
    coord.backend_changed();
    backend.set_views(vec![view(1, TaskStatus::Complete)]);
    coord.backend_changed();
    let seq = coord.snapshot().notice.expect("notice").seq;
    // A stale timer (older seq) retires nothing.
    assert_eq!(coord.notice_expired(seq + 99), Vec::<DlEffect>::new());
    assert!(coord.snapshot().notice.is_some_and(|n| n.open));
    // The matching timer closes it.
    assert_eq!(coord.notice_expired(seq), vec![DlEffect::PushChrome]);
    assert!(coord.snapshot().notice.is_some_and(|n| !n.open));

    // A click retires the notice and opens the page.
    backend.set_views(vec![view(2, TaskStatus::Downloading)]);
    coord.backend_changed();
    backend.set_views(vec![view(2, TaskStatus::Complete)]);
    coord.backend_changed();
    let effects = coord.notice_clicked();
    assert!(
        effects.contains(&DlEffect::OpenDownloadsPage),
        "{effects:?}"
    );
    assert!(coord.snapshot().notice.is_some_and(|n| !n.open));

    // A panel opening retires an open notice too.
    backend.set_views(vec![view(3, TaskStatus::Downloading)]);
    coord.backend_changed();
    backend.set_views(vec![view(3, TaskStatus::Complete)]);
    coord.backend_changed();
    assert!(coord.snapshot().notice.is_some_and(|n| n.open));
    assert_eq!(coord.panel_opened(), vec![DlEffect::PushChrome]);
    assert!(coord.snapshot().notice.is_some_and(|n| !n.open));
    // Dismiss-notice on a closed notice is a no-op.
    assert_eq!(
        coord.dispatch(DlCommand::DismissNotice, DlSource::Chrome),
        Vec::<DlEffect>::new()
    );
}

// ---------------------------------------------------------------------------
// 0.35, WS2: the redirect-chain replays. His exact flow:
// page tab → the site opens a NEW tab at the pre-redirect
// URL → 302 → CDN host → file. The tab's live URL holds
// the PRE-redirect hop while wry's download handler
// hands over the POST-redirect URI, so through 0.34 two
// exact-string matches (the fate gate, the finish
// swallow) both missed and the tab never closed — with
// no fate line in the log at all. The capture now
// carries the whole chain.
// ---------------------------------------------------------------------------

/// The pre-redirect hop (the soft98-shaped download host).
const HOP0: &str = "https://dl2.example.com/get/tool.zip";
/// A second redirect hop (replay 3).
const HOP1: &str = "https://mirror.example.com/hop/tool.zip";
/// The post-redirect final URL (the CDN host) — the URL
/// the download handler captures and the backend must use.
const FINAL: &str = "https://cdn.example.com/files/tool.zip";
/// A real page the user might navigate the tab to.
const REAL_PAGE: &str = "https://example.com/somewhere-else/";

/// His capture facts: an adopted tab, live on the
/// pre-redirect hop, captured at the final URL, the
/// chain carrying both, nothing committed, epoch 7.
fn his_facts() -> CaptureFacts {
    CaptureFacts {
        live_url: HOP0.to_string(),
        captured_url: FINAL.to_string(),
        chain: vec![HOP0.to_string(), FINAL.to_string()],
        committed_url: None,
        prev_url: None,
        origin: CaptureOrigin::Adopted,
        only_tab: false,
        traversal_pending: false,
        nav_epoch: 7,
    }
}

fn fate_dropped_logs(effects: &[DlEffect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, DlEffect::Log(s) if s.contains("tab moved on; fate dropped")))
        .count()
}

#[test]
fn replay1_his_exact_redirect_flow_closes_the_tab() {
    let (backend, mut coord) = coordinator();
    let (token, effects) =
        coord.capture_started(42, FINAL.to_string(), String::new(), None, his_facts());
    // The fate is decided AT CAPTURE and logged — through
    // 0.34 this capture decided None, silently.
    assert!(
        effects.contains(&DlEffect::Log(format!(
            "download capture {token}: fate decided CloseTab"
        ))),
        "{effects:?}"
    );
    // Cross-site landing: cookies cannot apply, so the app
    // answers immediately with an empty vec at the capture
    // epoch — no event-loop turn in between.
    let effects = coord.cookies_arrived(token, Vec::new(), landing(7));
    assert_eq!(backend.calls().len(), 1, "exactly one backend add");
    let tasks = coord.snapshot().tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].url, FINAL,
        "the backend add used the FINAL (post-redirect) URL"
    );
    assert!(
        has_fate(&effects, 42, TabFate::CloseTab),
        "the landing executes the close: {effects:?}"
    );
}

#[test]
fn replay2_control_direct_file_no_redirect_still_closes() {
    // Control: the pre-0.35 shape that already worked — a
    // direct file navigation, live == captured, singleton
    // chain — keeps closing (story 1's shape, restated as
    // this round's control).
    let (backend, mut coord) = coordinator();
    let (token, _) =
        coord.capture_started(42, FILE.to_string(), String::new(), None, adopted_facts());
    let effects = coord.cookies_arrived(token, vec![cookie("a")], landing(7));
    assert_eq!(backend.calls().len(), 1);
    assert!(has_fate(&effects, 42, TabFate::CloseTab), "{effects:?}");
    assert!(coord.navigation_finished(42, FILE));
}

#[test]
fn replay3a_two_hops_final_finish_swallowed_once_while_waiting() {
    let (_backend, mut coord) = coordinator();
    let mut facts = his_facts();
    facts.chain = vec![HOP0.to_string(), HOP1.to_string(), FINAL.to_string()];
    let (token, _) = coord.capture_started(42, FINAL.to_string(), String::new(), None, facts);
    assert!(
        coord.navigation_finished(42, FINAL),
        "the final URL's finish is the captured navigation's: swallowed"
    );
    assert!(
        !coord.navigation_finished(42, FINAL),
        "swallowed once while awaiting cookies — a second is a new navigation"
    );
    let effects = coord.cookies_arrived(token, Vec::new(), landing(7));
    assert!(
        has_fate(&effects, 42, TabFate::CloseTab),
        "the landing still closes: {effects:?}"
    );
    assert!(
        coord.navigation_finished(42, FINAL),
        "the Landed record swallows its one finish and is consumed"
    );
    assert!(!coord.navigation_finished(42, FINAL));
}

#[test]
fn replay3b_two_hops_intermediate_finish_is_the_captured_navigation() {
    let (_backend, mut coord) = coordinator();
    let mut facts = his_facts();
    facts.chain = vec![HOP0.to_string(), HOP1.to_string(), FINAL.to_string()];
    let (token, _) = coord.capture_started(42, FINAL.to_string(), String::new(), None, facts);
    assert!(
        coord.navigation_finished(42, HOP1),
        "a Finished carrying an intermediate hop is recognised as the captured navigation (swallowed, not a commit)"
    );
    assert!(
        !coord.navigation_finished(42, HOP1),
        "and it consumes the one awaiting-cookies swallow"
    );
    let effects = coord.cookies_arrived(token, Vec::new(), landing(7));
    assert!(has_fate(&effects, 42, TabFate::CloseTab), "{effects:?}");
}

#[test]
fn replay4_user_navigates_the_download_tab_before_landing() {
    let (backend, mut coord) = coordinator();
    let (token, _) = coord.capture_started(42, FINAL.to_string(), String::new(), None, his_facts());
    assert!(
        !coord.navigation_finished(42, REAL_PAGE),
        "a real page's finish is NOT swallowed — it commits (and bumps the epoch)"
    );
    // The landing revalidation sees the moved epoch: the
    // fate drops, the task still lands.
    let effects = coord.cookies_arrived(token, Vec::new(), landing(8));
    assert_eq!(backend.calls().len(), 1, "the task still lands");
    assert_eq!(coord.snapshot().tasks.len(), 1);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, DlEffect::ExecuteFate { .. })),
        "no fate executes: {effects:?}"
    );
    assert_eq!(
        fate_dropped_logs(&effects),
        1,
        "exactly one fate-dropped log line: {effects:?}"
    );
}

#[test]
fn replay5_redirect_then_real_commit_before_the_cookie_answer() {
    // The same-site/async shape: the captured navigation's
    // finish is swallowed, then a REAL commit (epoch 8)
    // lands before the cookie answer — the epoch guard,
    // unchanged, drops the fate at landing.
    let (_backend, mut coord) = coordinator();
    let (token, _) = coord.capture_started(42, FINAL.to_string(), String::new(), None, his_facts());
    assert!(
        coord.navigation_finished(42, FINAL),
        "the redirect chain's finish is swallowed while awaiting cookies"
    );
    let effects = coord.cookies_arrived(token, Vec::new(), landing(8));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, DlEffect::ExecuteFate { .. })),
        "epoch moved: no fate executes: {effects:?}"
    );
    assert_eq!(fate_dropped_logs(&effects), 1, "{effects:?}");
    // The record is Landed now: a post-landing finish for
    // a chain URL is still the captured navigation's —
    // swallowed exactly once, then the record is gone.
    assert!(coord.navigation_finished(42, FINAL));
    assert!(
        !coord.navigation_finished(42, FINAL),
        "the second identical call is a new navigation, not swallowed"
    );
}

// ---------------------------------------------------------------------------
// 0.35, WS5: failed-notice persistence. Through 0.34 the
// single notice slot was overwritten unconditionally, so
// a Started/Finished/Blocked notice silently buried an
// open Failed notice the user never saw, and panel
// open/close paths retired even a Failed notice. The
// coordinator now keeps an open Failed notice sticky in
// set_notice, page_visited retires it (a legitimate
// exit), and the guarded retire leaves it alone.
// ---------------------------------------------------------------------------

/// Drive a fresh coordinator to an open Failed notice for
/// task `id` (the startup-silent first backend_changed,
/// then a Downloading → Failed flip).
fn failed_notice(coord: &mut Coordinator<FakeBackend>, backend: &FakeBackend, id: u64) {
    backend.set_views(vec![view(id, TaskStatus::Downloading)]);
    coord.backend_changed();
    let mut failed = view(id, TaskStatus::Failed);
    failed.error = Some("HTTP 404".to_string());
    backend.set_views(vec![failed]);
    coord.backend_changed();
    let notice = coord.snapshot().notice.expect("failed notice");
    assert_eq!(notice.kind, NoticeKind::Failed);
    assert!(notice.open);
}

#[test]
fn failed_notice_survives_started_finished_and_blocked() {
    let (backend, mut coord) = coordinator();
    failed_notice(&mut coord, &backend, 1);
    let before = coord.snapshot().notice.expect("notice");
    assert_eq!(coord.snapshot().failed_unseen, 1);

    // A Started notice (add-url landing) must not bury it.
    coord.dispatch(
        DlCommand::AddUrl("https://example.com/f.zip".to_string()),
        DlSource::Page,
    );
    let after = coord.snapshot().notice.expect("notice");
    assert_eq!(after.kind, NoticeKind::Failed, "Started buried the Failed");
    assert_eq!(after.seq, before.seq, "no seq bump for a buried notice");
    assert_eq!(after.name, before.name);
    assert!(after.open);

    // A Finished notice (a task completing) must not bury it.
    backend.set_views(vec![view(2, TaskStatus::Downloading)]);
    coord.backend_changed();
    backend.set_views(vec![view(2, TaskStatus::Complete)]);
    coord.backend_changed();
    let after = coord.snapshot().notice.expect("notice");
    assert_eq!(after.kind, NoticeKind::Failed, "Finished buried the Failed");
    assert_eq!(after.seq, before.seq);

    // A Blocked notice (a rejected add-url) must not bury it.
    coord.dispatch(
        DlCommand::AddUrl("file:///C:/f.zip".to_string()),
        DlSource::Page,
    );
    let after = coord.snapshot().notice.expect("notice");
    assert_eq!(after.kind, NoticeKind::Failed, "Blocked buried the Failed");
    assert_eq!(after.seq, before.seq);
    assert!(after.open);
}

#[test]
fn failed_notice_is_replaced_by_a_newer_failed() {
    let (backend, mut coord) = coordinator();
    failed_notice(&mut coord, &backend, 1);
    let first_seq = coord.snapshot().notice.expect("notice").seq;
    failed_notice(&mut coord, &backend, 2);
    let notice = coord.snapshot().notice.expect("notice");
    assert_eq!(notice.kind, NoticeKind::Failed);
    assert_eq!(notice.name, "file2.bin", "the newer failure speaks");
    assert!(
        notice.seq > first_seq,
        "Failed-over-Failed replaces (seq bumps)"
    );
}

#[test]
fn started_then_finished_replaces_normally() {
    let (backend, mut coord) = coordinator();
    coord.dispatch(
        DlCommand::AddUrl("https://example.com/f.zip".to_string()),
        DlSource::Page,
    );
    assert_eq!(
        coord.snapshot().notice.map(|n| n.kind),
        Some(NoticeKind::Started)
    );
    // The normal path is intact: a Finished notice
    // replaces a Started one (no Failed notice is open).
    backend.set_views(vec![view(9, TaskStatus::Downloading)]);
    coord.backend_changed();
    backend.set_views(vec![view(9, TaskStatus::Complete)]);
    coord.backend_changed();
    assert_eq!(
        coord.snapshot().notice.map(|n| n.kind),
        Some(NoticeKind::Finished)
    );
}

#[test]
fn page_visited_retires_an_open_failed_notice() {
    let (backend, mut coord) = coordinator();
    failed_notice(&mut coord, &backend, 1);
    let effects = coord.page_visited();
    assert!(effects.contains(&DlEffect::PushChrome), "{effects:?}");
    assert_eq!(coord.snapshot().failed_unseen, 0);
    assert!(
        coord.snapshot().notice.is_some_and(|n| !n.open),
        "visiting the downloads page retires even a Failed notice"
    );
}

#[test]
fn guarded_retire_leaves_a_failed_notice_open() {
    let (backend, mut coord) = coordinator();
    failed_notice(&mut coord, &backend, 1);
    let effects = coord.retire_notice_unless_failed();
    assert_eq!(effects, Vec::<DlEffect>::new());
    assert!(
        coord.snapshot().notice.is_some_and(|n| n.open),
        "the guarded retire must not retire an unseen Failed notice"
    );
}

#[test]
fn guarded_retire_retires_a_started_notice() {
    let (_backend, mut coord) = coordinator();
    coord.dispatch(
        DlCommand::AddUrl("https://example.com/f.zip".to_string()),
        DlSource::Page,
    );
    assert!(coord.snapshot().notice.is_some_and(|n| n.open));
    let effects = coord.retire_notice_unless_failed();
    assert_eq!(effects, vec![DlEffect::PushChrome]);
    assert!(coord.snapshot().notice.is_some_and(|n| !n.open));
}
