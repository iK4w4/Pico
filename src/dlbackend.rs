//! The downloads coordinator's backend adapter (0.34):
//! [`ManagerBackend`] implements the core's
//! `pico_core::core::dlcoord::DlBackend` trait by forwarding
//! to the bin's [`DownloadManager`]. This is the ONLY seam
//! between the pure coordinator and the real manager — the
//! coordinator decides, this adapter executes against the
//! manager's `&self` handle API.
//!
//! `remove` is the one non-1:1 verb: the 0.33 manager has
//! no pure remove-record method, so it forwards to
//! `DownloadManager::cancel`, whose semantics are exactly
//! the downloads page's intent for `remove` — a Complete
//! task loses only its record (the finished file stays);
//! an unfinished task loses its `.part` and its record.
//! The coordinator's `cancel` (the chrome verb) maps to
//! the same manager method: the two coordinator verbs are
//! aliases over ONE manager verb until the manager grows
//! a distinct remove. (Worker 1's mapping note on
//! `DlBackend::remove` says the same.)

use std::path::PathBuf;

use pico_core::core::dlcoord::{DlBackend, DlTaskView};
use pico_core::core::download::{CookiePair, TaskStatus};

use crate::downloads::{DownloadManager, DownloadView};

/// The coordinator's view of the real download manager:
/// a newtype over the cheap clone handle so the trait
/// implementation lives here, not on the manager itself.
pub struct ManagerBackend(pub DownloadManager);

impl ManagerBackend {
    /// Wrap the manager and write the downloads init line
    /// (Law 6, startup self-announcement): how many tasks
    /// were loaded from disk and the resolved downloads
    /// folder — the two facts every downloads question
    /// starts from.
    pub fn new(manager: DownloadManager) -> ManagerBackend {
        let count = manager.views().len();
        let dir = manager.downloads_dir();
        crate::log::info(&format!(
            "downloads: {count} task(s) loaded; folder {}",
            dir.display()
        ));
        ManagerBackend(manager)
    }
}

/// Convert one manager view into the coordinator's task
/// view, field for field (the two structs carry the same
/// twenty fields by design).
fn to_dl_view(v: DownloadView) -> DlTaskView {
    DlTaskView {
        id: v.id,
        name: v.name,
        host: v.host,
        status: v.status,
        total: v.total,
        downloaded: v.downloaded,
        percent: v.percent,
        size_text: v.size_text,
        detail_text: v.detail_text,
        connections: v.connections,
        error: v.error,
        url: v.url,
        added_ms: v.added_ms,
        sha256: v.sha256,
        hint: v.hint,
        retry_text: v.retry_text,
        retry_pending: v.retry_pending,
        refreshing: v.refreshing,
        speed_bps: v.speed_bps,
        eta_secs: v.eta_secs,
    }
}

impl DlBackend for ManagerBackend {
    fn add(
        &self,
        url: &str,
        suggested: Option<&str>,
        cookies: Vec<CookiePair>,
        connections: u32,
        referer: Option<String>,
    ) -> Option<u64> {
        // The manager's `add` takes exactly the core's
        // `CookiePair` (src/downloads.rs imports it from
        // `pico_core::core::download`, the same type the
        // coordinator passes) — no adaptation needed.
        self.0.add(url, suggested, cookies, connections, referer)
    }

    fn add_failed(&self, url: &str, suggested: Option<&str>, reason: &str) -> Option<u64> {
        self.0.add_failed(url, suggested, reason)
    }

    fn pause(&self, id: u64) {
        self.0.pause(id);
    }

    fn resume(&self, id: u64) {
        self.0.resume(id);
    }

    fn cancel(&self, id: u64) {
        self.0.cancel(id);
    }

    fn restart(&self, id: u64) {
        self.0.restart(id);
    }

    fn refresh_link(&self, id: u64, url: &str) {
        self.0.refresh_link(id, url);
    }

    /// Documented alias (see the module doc): the manager
    /// has no pure remove verb, so `remove` forwards to
    /// its `cancel`.
    fn remove(&self, id: u64) {
        self.0.cancel(id);
    }

    fn delete_file(&self, id: u64) {
        self.0.delete_file(id);
    }

    fn clear_completed(&self) {
        self.0.clear_completed();
    }

    fn clear_all(&self) {
        self.0.clear_all();
    }

    fn set_connections(&self, id: u64, n: u32) {
        self.0.set_connections(id, n);
    }

    fn set_default_connections(&self, n: u32) {
        self.0.set_default_connections(n);
    }

    fn set_max_concurrent(&self, n: u32) {
        self.0.set_max_concurrent(n);
    }

    fn set_speed_limit(&self, kbps: Option<u64>) {
        self.0.set_speed_limit(kbps);
    }

    fn views(&self) -> Vec<DlTaskView> {
        self.0.views().into_iter().map(to_dl_view).collect()
    }

    fn version(&self) -> u64 {
        self.0.version()
    }

    fn active_count(&self) -> usize {
        self.0.active_count()
    }

    fn failed_count(&self) -> usize {
        // The manager exposes no failed counter of its own;
        // it is computed from its views, as the trait doc
        // prescribes.
        self.0
            .views()
            .iter()
            .filter(|v| v.status == TaskStatus::Failed)
            .count()
    }

    fn name_of(&self, id: u64) -> Option<String> {
        self.0.name_of(id)
    }

    fn completed_path(&self, id: u64) -> Option<PathBuf> {
        self.0.completed_path(id)
    }

    fn downloads_dir(&self) -> PathBuf {
        self.0.downloads_dir()
    }
}
