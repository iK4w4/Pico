//! Downloads command table (0.34) — the SINGLE vocabulary
//! for every downloads command on every surface.
//!
//! Before 0.34 the downloads verbs lived in three places
//! that had drifted apart: the chrome's snake_case commands
//! (`core::ipc::Command::DownloadPause`, `download_pause`,
//! …), the downloads page's kebab verbs (`core::dlaction`,
//! `{"kind":"dl","action":"pause",…}`), and ad-hoc arms in
//! `app.rs`. Five patch rounds against that split failed in
//! the field, so 0.34 rebuilds the coordination layer from
//! zero around one table: [`DL_COMMANDS`].
//!
//! The canonical wire body on EVERY surface is a JSON
//! object `{"kind":"dl","action":"<verb>", ...args}` with
//! the kebab verb names below. [`parse_dl_body`] is the one
//! parser; it absorbs `core::dlaction`'s `decide()`
//! discipline wholesale:
//!
//! - the body is size-capped ([`MAX_DL_BODY_BYTES`]);
//! - `kind` must be `"dl"` (anything else is `Malformed`);
//! - an unknown action is its own denial (`UnknownAction`);
//! - unknown FIELDS are denied (`deny_unknown_fields`) —
//!   exactly dlaction's strictness: a field that exists in
//!   the shared body shape but is irrelevant to the verb is
//!   ignored, a field outside the shape is `Malformed`;
//! - a task `id` must be present and `> 0` where required;
//! - URL args must be downloadable http/https URLs of at
//!   most [`MAX_DL_URL_CHARS`] chars (the same validation
//!   dlaction used: `core::download::is_downloadable_url`);
//! - numeric args are clamped/validated exactly as
//!   dlaction did: `set-max-concurrent` clamps to `1..=5`,
//!   `set-speed-limit` maps `0` to off and caps the rest at
//!   1_000_000 kbps, and connection counts clamp to the
//!   core's `1..=32` (`MIN_CONNECTIONS..=MAX_CONNECTIONS`,
//!   the clamp the chrome arms applied before the manager).
//!
//! GATES. Two transports reach the coordinator, and each
//! has its own gate, applied by the app BEFORE parsing:
//!
//! - The downloads PAGE is gated by [`page_gate`]: the
//!   conjunctive law — the requesting webview must be a
//!   downloads tab AND be on the downloads URL. Both facts
//!   are app-owned tab state, never page-supplied.
//! - The CHROME transport is reachable only from the chrome
//!   webview (its IPC handler is registered on that view
//!   alone, and downloads bodies ride it as raw bodies, not
//!   as `core::ipc::Command`s), and its envelope's boot/req
//!   dedupe drops replays and stale boots.
//!
//! Both transports additionally enforce the per-verb surface
//! table below: after parsing, the app checks
//! [`DlCommandSpec::allows`] for the transport's surface and
//! denies a verb that surface never emits (AUDIT 0.40.1 —
//! through 0.35.1 the table was documentation only and both
//! transports accepted every verb).
//!
//! One verb is client-only: `copy-link` copies the task's
//! URL to the clipboard inside the page and is NEVER
//! dispatched to Rust. It sits in the table so the
//! vocabulary is complete (and so a body claiming it is
//! denied distinctly, as [`DlDeny::ClientOnly`], instead of
//! looking unknown), but [`DlCommand`] has no variant for
//! it and [`parse_dl_body`] never produces one.

use serde::Deserialize;

use crate::core::addr::normalize_pico_url;
use crate::core::download::{is_downloadable_url, MAX_CONNECTIONS, MIN_CONNECTIONS};

/// The downloads page's canonical URL (moved here from
/// `core::dlaction`, which 0.34 folds into this module).
pub const DOWNLOADS_URL: &str = "pico://downloads";

/// Hard cap on a downloads-command body. The largest
/// legitimate body carries one 2048-char URL plus framing
/// (inherited from dlaction).
pub const MAX_DL_BODY_BYTES: usize = 8192;
/// Hard cap on a URL carried by a downloads command
/// (inherited from dlaction).
pub const MAX_DL_URL_CHARS: usize = 2048;

/// A surface a downloads command can originate from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlSurface {
    /// The chrome webview (popover, toast, toolbar).
    Chrome,
    /// The downloads page (`pico://downloads`).
    Page,
    /// The start page (`pico://start`).
    Start,
}

/// The argument shape one verb carries on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlArgSchema {
    /// No arguments.
    None,
    /// `{"id": N}` — a task id, required, `> 0`.
    Id,
    /// `{"id": N}` when present, no id at all otherwise
    /// (only `open-folder`: no id opens the downloads
    /// folder itself). A present id must still be `> 0`.
    OptionalId,
    /// `{"id": N, "conn": C}`.
    IdConn,
    /// `{"url": U}` — a downloadable URL.
    Url,
    /// `{"id": N, "url": U}` (generic id+url shape; no
    /// current verb uses it — `refresh` uses the named
    /// [`DlArgSchema::RefreshPair`] for the same bytes).
    IdUrl,
    /// `{"conn": C}`.
    Conn,
    /// `{"maxConcurrent": N}` — clamped to `1..=5`.
    MaxConcurrent,
    /// `{"speedLimitKbps": N}` — `0` is off, else capped
    /// at 1_000_000.
    SpeedLimit,
    /// `{"id": N, "url": U}` — `refresh`'s pair: rebind
    /// task `id` to the fresh URL `url`.
    RefreshPair,
}

/// One row of the command table: a verb's canonical name,
/// its argument schema, the surfaces allowed to send it,
/// and whether it is client-only (never dispatched to
/// Rust).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlCommandSpec {
    pub name: &'static str,
    pub args: DlArgSchema,
    pub surfaces: &'static [DlSurface],
    pub client_only: bool,
}

impl DlCommandSpec {
    /// True when `surface` may send this verb.
    pub fn allows(&self, surface: DlSurface) -> bool {
        self.surfaces.contains(&surface)
    }
}

use DlSurface::{Chrome, Page, Start};

/// THE command table (0.34). The verb set is the union of
/// the 0.33 chrome snake names and the downloads page's
/// dlaction verbs, canonicalized to the page's kebab
/// verbs:
///
/// - chrome `downloads` / `downloads_close` /
///   `downloads_page` → `open-panel` / `close-panel` /
///   `open-page`;
/// - chrome `download_pause` / `download_resume` /
///   `download_cancel` / `download_add` /
///   `download_set_conn` / `download_set_default_conn` /
///   `download_open_folder` → `pause` / `resume` /
///   `cancel` / `add-url` / `set-connections` /
///   `set-default-connections` / `open-folder` (no id);
/// - page verbs keep their kebab names (`pause`, `resume`,
///   `restart`, `refresh`, `remove`, `delete-file`,
///   `open-file`, `open-folder` with id, `add-url`,
///   `set-max-concurrent`, `set-speed-limit`,
///   `clear-completed`, `clear-all`);
/// - `copy-link` is the page's clipboard-local row action;
/// - `dismiss-notice` is the popover toast's click
///   semantics (0.33: the toast click retires the notice
///   and routes onward) as a first-class verb.
///
/// `cancel` and `remove` are DISTINCT verbs with distinct
/// histories — the chrome's cancel button (which reads
/// "Remove from list" on a finished row) and the page's
/// `remove` — even though the 0.33 manager serves both
/// through one method; see the mapping note on
/// `core::dlcoord::DlBackend::remove`.
pub const DL_COMMANDS: &[DlCommandSpec] = &[
    DlCommandSpec {
        name: "open-panel",
        args: DlArgSchema::None,
        surfaces: &[Chrome],
        client_only: false,
    },
    DlCommandSpec {
        name: "close-panel",
        args: DlArgSchema::None,
        surfaces: &[Chrome],
        client_only: false,
    },
    DlCommandSpec {
        name: "open-page",
        args: DlArgSchema::None,
        surfaces: &[Chrome, Start],
        client_only: false,
    },
    DlCommandSpec {
        name: "pause",
        args: DlArgSchema::Id,
        surfaces: &[Chrome, Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "resume",
        args: DlArgSchema::Id,
        surfaces: &[Chrome, Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "cancel",
        args: DlArgSchema::Id,
        surfaces: &[Chrome],
        client_only: false,
    },
    DlCommandSpec {
        name: "restart",
        args: DlArgSchema::Id,
        surfaces: &[Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "refresh",
        args: DlArgSchema::RefreshPair,
        surfaces: &[Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "remove",
        args: DlArgSchema::Id,
        // AUDIT 0.40.1: the popover's × on a finished row is
        // "Remove from list" and emits `remove` too — the
        // table now matches the shipped renderers.
        surfaces: &[Chrome, Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "delete-file",
        args: DlArgSchema::Id,
        surfaces: &[Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "open-file",
        args: DlArgSchema::Id,
        surfaces: &[Chrome, Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "open-folder",
        args: DlArgSchema::OptionalId,
        surfaces: &[Chrome, Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "copy-link",
        args: DlArgSchema::Id,
        surfaces: &[Page],
        client_only: true,
    },
    DlCommandSpec {
        name: "add-url",
        args: DlArgSchema::Url,
        surfaces: &[Chrome, Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "set-connections",
        args: DlArgSchema::IdConn,
        surfaces: &[Chrome],
        client_only: false,
    },
    DlCommandSpec {
        name: "set-default-connections",
        args: DlArgSchema::Conn,
        surfaces: &[Chrome],
        client_only: false,
    },
    DlCommandSpec {
        name: "set-max-concurrent",
        args: DlArgSchema::MaxConcurrent,
        surfaces: &[Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "set-speed-limit",
        args: DlArgSchema::SpeedLimit,
        surfaces: &[Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "dismiss-notice",
        args: DlArgSchema::None,
        surfaces: &[Chrome],
        client_only: false,
    },
    DlCommandSpec {
        name: "clear-completed",
        args: DlArgSchema::None,
        surfaces: &[Chrome, Page],
        client_only: false,
    },
    DlCommandSpec {
        name: "clear-all",
        args: DlArgSchema::None,
        surfaces: &[Page],
        client_only: false,
    },
];

/// Look up one verb's spec by its canonical name.
pub fn spec_for(name: &str) -> Option<&'static DlCommandSpec> {
    DL_COMMANDS.iter().find(|s| s.name == name)
}

/// Why a downloads command was denied. Each variant is one
/// log word; the set is closed so a denial is always
/// classifiable from pico.log. The first four variants are
/// dlaction's, unchanged; `ClientOnly` is new in 0.34 (the
/// unified table names a verb dlaction never saw).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlDeny {
    NotDownloadsTab,
    NotOnDownloadsPage,
    Malformed,
    UnknownAction,
    ClientOnly,
}

impl DlDeny {
    /// Stable log spelling.
    pub fn label(self) -> &'static str {
        match self {
            DlDeny::NotDownloadsTab => "not-downloads-tab",
            DlDeny::NotOnDownloadsPage => "not-on-downloads-page",
            DlDeny::Malformed => "malformed",
            DlDeny::UnknownAction => "unknown-action",
            DlDeny::ClientOnly => "client-only",
        }
    }

    /// Alias of [`Self::label`], mirroring dlaction.
    pub fn as_str(self) -> &'static str {
        self.label()
    }
}

/// One typed downloads command — one variant per
/// non-client-only verb in [`DL_COMMANDS`]. This is what
/// the coordinator's `dispatch` consumes; the app builds
/// it either through [`parse_dl_body`] (the canonical wire
/// body) or by translating a chrome `core::ipc::Command`
/// 1:1 (Worker 2's fold).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DlCommand {
    OpenPanel,
    ClosePanel,
    OpenPage,
    Pause(u64),
    Resume(u64),
    Cancel(u64),
    Restart(u64),
    /// Rebind task to a fresh URL (id, url).
    Refresh(u64, String),
    Remove(u64),
    DeleteFile(u64),
    OpenFile(u64),
    /// `None` = the downloads folder itself.
    OpenFolder(Option<u64>),
    AddUrl(String),
    /// Set one task's connection count (id, conn count,
    /// already clamped to `1..=32`).
    SetConnections(u64, u32),
    /// Set the default connection count for future tasks
    /// (conn count, already clamped to `1..=32`).
    SetDefaultConnections(u32),
    /// Set the concurrent-download cap (already clamped
    /// to `1..=5`).
    SetMaxConcurrent(u32),
    /// Set the global speed limit in kbps; `None` = off.
    SetSpeedLimitKbps(Option<u64>),
    DismissNotice,
    ClearCompleted,
    ClearAll,
}

impl DlCommand {
    /// The canonical verb name (the [`DL_COMMANDS`] row
    /// this command came from) — the app's log spelling.
    pub fn name(&self) -> &'static str {
        match self {
            DlCommand::OpenPanel => "open-panel",
            DlCommand::ClosePanel => "close-panel",
            DlCommand::OpenPage => "open-page",
            DlCommand::Pause(_) => "pause",
            DlCommand::Resume(_) => "resume",
            DlCommand::Cancel(_) => "cancel",
            DlCommand::Restart(_) => "restart",
            DlCommand::Refresh(..) => "refresh",
            DlCommand::Remove(_) => "remove",
            DlCommand::DeleteFile(_) => "delete-file",
            DlCommand::OpenFile(_) => "open-file",
            DlCommand::OpenFolder(_) => "open-folder",
            DlCommand::AddUrl(_) => "add-url",
            DlCommand::SetConnections(..) => "set-connections",
            DlCommand::SetDefaultConnections(_) => "set-default-connections",
            DlCommand::SetMaxConcurrent(_) => "set-max-concurrent",
            DlCommand::SetSpeedLimitKbps(_) => "set-speed-limit",
            DlCommand::DismissNotice => "dismiss-notice",
            DlCommand::ClearCompleted => "clear-completed",
            DlCommand::ClearAll => "clear-all",
        }
    }
}

/// The downloads-page gate (the conjunctive law, as a pure
/// fn): a page command is honored only when the sender is
/// a downloads tab AND is on the downloads URL. The app
/// computes `on_downloads_url` from its own tab state —
/// see [`is_downloads_url`] for the spelling rule dlaction
/// used — and calls this BEFORE parsing, so a hostile page
/// cannot probe the parser through a denied request.
pub fn page_gate(is_downloads_tab: bool, on_downloads_url: bool) -> bool {
    is_downloads_tab && on_downloads_url
}

/// True when `url` names the downloads page: dlaction's
/// rule — the normalized `pico://` form equals
/// [`DOWNLOADS_URL`], with the trailing-slash spelling
/// accepted as the same page.
pub fn is_downloads_url(url: &str) -> bool {
    let current = normalize_pico_url(url);
    current == DOWNLOADS_URL || current == "pico://downloads/"
}

/// True when an IPC body *claims* to be a downloads
/// command (its `kind` field is `"dl"`). Mirrors
/// dlaction's `looks_like_dl_action`: a cheap JSON
/// pre-parse the content IPC handler routes on; the real
/// gate + strict parse happen in [`parse_dl_body`].
pub fn looks_like_dl_body(body: &str) -> bool {
    if body.len() > MAX_DL_BODY_BYTES {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    v.get("kind").and_then(|k| k.as_str()) == Some("dl")
}

/// Best-effort verb name for the app's log line, derived
/// from the body alone (used even when parsing denies):
/// the canonical name when the body names a known verb,
/// `"unknown"` otherwise. Mirrors dlaction's
/// `action_label`.
pub fn action_label(body: &str) -> &'static str {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return "unknown";
    };
    match v.get("action").and_then(|a| a.as_str()) {
        Some(name) => match spec_for(name) {
            Some(spec) => spec.name,
            None => "unknown",
        },
        None => "unknown",
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDl {
    kind: String,
    action: String,
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    conn: Option<u32>,
    #[serde(default, rename = "maxConcurrent")]
    max_concurrent: Option<u32>,
    #[serde(default, rename = "speedLimitKbps")]
    speed_limit_kbps: Option<u64>,
}

fn valid_dl_url(url: &str) -> bool {
    url.chars().count() <= MAX_DL_URL_CHARS && is_downloadable_url(url)
}

fn require_id(raw: &RawDl) -> Result<u64, DlDeny> {
    match raw.id {
        Some(id) if id >= 1 => Ok(id),
        _ => Err(DlDeny::Malformed),
    }
}

fn optional_id(raw: &RawDl) -> Result<Option<u64>, DlDeny> {
    match raw.id {
        None => Ok(None),
        Some(id) if id >= 1 => Ok(Some(id)),
        Some(_) => Err(DlDeny::Malformed),
    }
}

fn require_url(raw: &RawDl) -> Result<String, DlDeny> {
    match &raw.url {
        Some(url) if valid_dl_url(url) => Ok(url.clone()),
        _ => Err(DlDeny::Malformed),
    }
}

fn require_connections(raw: &RawDl) -> Result<u32, DlDeny> {
    match raw.conn {
        Some(n) => Ok(n.clamp(MIN_CONNECTIONS, MAX_CONNECTIONS)),
        None => Err(DlDeny::Malformed),
    }
}

/// Parse one canonical downloads body
/// (`{"kind":"dl","action":"<verb>", ...args}`) into a
/// typed [`DlCommand`]. Pure and total: every input is a
/// command or a [`DlDeny`]. The gate is NOT here — the
/// caller applies [`page_gate`] (page transport) or the
/// chrome transport's inherent gate first, exactly as
/// dlaction's `dispatch` ran its gate before parsing.
pub fn parse_dl_body(raw: &str) -> Result<DlCommand, DlDeny> {
    if raw.len() > MAX_DL_BODY_BYTES {
        return Err(DlDeny::Malformed);
    }
    let raw: RawDl = serde_json::from_str(raw).map_err(|_| DlDeny::Malformed)?;
    if raw.kind != "dl" {
        return Err(DlDeny::Malformed);
    }
    match raw.action.as_str() {
        "open-panel" => Ok(DlCommand::OpenPanel),
        "close-panel" => Ok(DlCommand::ClosePanel),
        "open-page" => Ok(DlCommand::OpenPage),
        "pause" => Ok(DlCommand::Pause(require_id(&raw)?)),
        "resume" => Ok(DlCommand::Resume(require_id(&raw)?)),
        "cancel" => Ok(DlCommand::Cancel(require_id(&raw)?)),
        "restart" => Ok(DlCommand::Restart(require_id(&raw)?)),
        "refresh" => Ok(DlCommand::Refresh(require_id(&raw)?, require_url(&raw)?)),
        "remove" => Ok(DlCommand::Remove(require_id(&raw)?)),
        "delete-file" => Ok(DlCommand::DeleteFile(require_id(&raw)?)),
        "open-file" => Ok(DlCommand::OpenFile(require_id(&raw)?)),
        "open-folder" => Ok(DlCommand::OpenFolder(optional_id(&raw)?)),
        "copy-link" => Err(DlDeny::ClientOnly),
        "add-url" => Ok(DlCommand::AddUrl(require_url(&raw)?)),
        "set-connections" => Ok(DlCommand::SetConnections(
            require_id(&raw)?,
            require_connections(&raw)?,
        )),
        "set-default-connections" => {
            Ok(DlCommand::SetDefaultConnections(require_connections(&raw)?))
        }
        "set-max-concurrent" => match raw.max_concurrent {
            Some(n) => Ok(DlCommand::SetMaxConcurrent(n.clamp(1, 5))),
            None => Err(DlDeny::Malformed),
        },
        "set-speed-limit" => match raw.speed_limit_kbps {
            Some(0) => Ok(DlCommand::SetSpeedLimitKbps(None)),
            Some(v) => Ok(DlCommand::SetSpeedLimitKbps(Some(v.min(1_000_000)))),
            None => Err(DlDeny::Malformed),
        },
        "dismiss-notice" => Ok(DlCommand::DismissNotice),
        "clear-completed" => Ok(DlCommand::ClearCompleted),
        "clear-all" => Ok(DlCommand::ClearAll),
        _ => Err(DlDeny::UnknownAction),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The parse matrix: every verb's ok-case, plus the
    // deny-cases inherited from dlaction's tests (they are
    // the contract — ported, not paraphrased).

    #[test]
    fn table_is_complete_and_consistent() {
        assert_eq!(DL_COMMANDS.len(), 21);
        for spec in DL_COMMANDS {
            assert!(!spec.surfaces.is_empty(), "{} has no surface", spec.name);
            assert_eq!(spec_for(spec.name).map(|s| s.name), Some(spec.name));
        }
        // Exactly one client-only verb.
        let client_only: Vec<&str> = DL_COMMANDS
            .iter()
            .filter(|s| s.client_only)
            .map(|s| s.name)
            .collect();
        assert_eq!(client_only, vec!["copy-link"]);
        // Every DlCommand variant names a table row.
        let cmds = [
            DlCommand::OpenPanel,
            DlCommand::ClosePanel,
            DlCommand::OpenPage,
            DlCommand::Pause(1),
            DlCommand::Resume(1),
            DlCommand::Cancel(1),
            DlCommand::Restart(1),
            DlCommand::Refresh(1, "https://example.com/f".to_string()),
            DlCommand::Remove(1),
            DlCommand::DeleteFile(1),
            DlCommand::OpenFile(1),
            DlCommand::OpenFolder(None),
            DlCommand::AddUrl("https://example.com/f".to_string()),
            DlCommand::SetConnections(1, 8),
            DlCommand::SetDefaultConnections(8),
            DlCommand::SetMaxConcurrent(3),
            DlCommand::SetSpeedLimitKbps(None),
            DlCommand::DismissNotice,
            DlCommand::ClearCompleted,
            DlCommand::ClearAll,
        ];
        for cmd in &cmds {
            assert!(
                spec_for(cmd.name()).is_some(),
                "{} not in table",
                cmd.name()
            );
        }
    }

    #[test]
    fn surface_matrix_matches_the_shipped_renderers() {
        // AUDIT 0.40.1: both app transports now ENFORCE this
        // table (denying a verb on a surface that never emits
        // it), so the table must match the shipped emitters
        // exactly — assets/dlrows.js (renderPopoverRow for the
        // chrome popover, renderPageRow for the page) plus each
        // surface's own senders in chrome.html / downloads.html:
        // - popover rows: pause/resume, cancel, remove (done
        //   rows), open-file, open-folder, set-connections;
        // - page rows: pause/resume, restart, refresh, remove,
        //   delete-file, open-file, open-folder, copy-link;
        // - chrome shell: open/close-panel, open-page, add-url,
        //   set-default-connections, dismiss-notice,
        //   clear-completed, open-folder;
        // - page shell: add-url, set-max-concurrent,
        //   set-speed-limit, clear-completed, clear-all,
        //   open-folder.
        for verb in [
            "open-panel",
            "close-panel",
            "cancel",
            "set-connections",
            "set-default-connections",
            "dismiss-notice",
        ] {
            let spec = spec_for(verb).expect("verb in table");
            assert!(spec.allows(DlSurface::Chrome), "{verb} on Chrome");
            assert!(!spec.allows(DlSurface::Page), "{verb} not on Page");
        }
        for verb in [
            "restart",
            "refresh",
            "delete-file",
            "set-max-concurrent",
            "set-speed-limit",
            "clear-all",
        ] {
            let spec = spec_for(verb).expect("verb in table");
            assert!(!spec.allows(DlSurface::Chrome), "{verb} not on Chrome");
            assert!(spec.allows(DlSurface::Page), "{verb} on Page");
        }
        for verb in [
            "pause",
            "resume",
            "remove",
            "open-file",
            "open-folder",
            "add-url",
            "clear-completed",
        ] {
            let spec = spec_for(verb).expect("verb in table");
            assert!(spec.allows(DlSurface::Chrome), "{verb} on Chrome");
            assert!(spec.allows(DlSurface::Page), "{verb} on Page");
        }
        let open_page = spec_for("open-page").expect("verb in table");
        assert!(open_page.allows(DlSurface::Chrome));
        assert!(open_page.allows(DlSurface::Start));
        assert!(!open_page.allows(DlSurface::Page));
        let copy_link = spec_for("copy-link").expect("verb in table");
        assert!(copy_link.allows(DlSurface::Page));
        assert!(!copy_link.allows(DlSurface::Chrome));
        assert!(copy_link.client_only);
    }

    #[test]
    fn widened_verbs_allow_chrome_and_page() {
        // 0.35: the popover (chrome surface) emits both
        // verbs too — clear-completed from its footer and
        // open-file from its rows (the shared dlrows
        // renderer). Both rows carry [Chrome, Page].
        for verb in ["clear-completed", "open-file"] {
            let spec = spec_for(verb).expect("verb in table");
            assert!(spec.allows(DlSurface::Chrome), "{verb} on Chrome");
            assert!(spec.allows(DlSurface::Page), "{verb} on Page");
            assert!(!spec.allows(DlSurface::Start), "{verb} not on Start");
        }
    }

    #[test]
    fn page_gate_is_conjunctive() {
        assert!(page_gate(true, true));
        assert!(!page_gate(true, false));
        assert!(!page_gate(false, true));
        assert!(!page_gate(false, false));
        assert!(is_downloads_url("pico://downloads"));
        assert!(is_downloads_url("pico://downloads/"));
        assert!(!is_downloads_url("pico://start"));
    }

    #[test]
    fn no_arg_verbs_parse() {
        for (body, want) in [
            (
                r#"{"kind":"dl","action":"open-panel"}"#,
                DlCommand::OpenPanel,
            ),
            (
                r#"{"kind":"dl","action":"close-panel"}"#,
                DlCommand::ClosePanel,
            ),
            (r#"{"kind":"dl","action":"open-page"}"#, DlCommand::OpenPage),
            (
                r#"{"kind":"dl","action":"dismiss-notice"}"#,
                DlCommand::DismissNotice,
            ),
            (
                r#"{"kind":"dl","action":"clear-completed"}"#,
                DlCommand::ClearCompleted,
            ),
            (r#"{"kind":"dl","action":"clear-all"}"#, DlCommand::ClearAll),
        ] {
            assert_eq!(parse_dl_body(body), Ok(want), "{body}");
        }
    }

    #[test]
    fn id_actions_parse_and_id_zero_is_malformed() {
        // Ported from dlaction: pause/delete-file ok-cases,
        // id 0 and missing id are Malformed.
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"pause","id":7}"#),
            Ok(DlCommand::Pause(7))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"delete-file","id":3}"#),
            Ok(DlCommand::DeleteFile(3))
        );
        for verb in [
            "pause",
            "resume",
            "cancel",
            "restart",
            "remove",
            "delete-file",
            "open-file",
        ] {
            let zero = format!(r#"{{"kind":"dl","action":"{verb}","id":0}}"#);
            assert_eq!(parse_dl_body(&zero), Err(DlDeny::Malformed), "{verb} id 0");
            let missing = format!(r#"{{"kind":"dl","action":"{verb}"}}"#);
            assert_eq!(
                parse_dl_body(&missing),
                Err(DlDeny::Malformed),
                "{verb} missing id"
            );
        }
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"resume","id":7}"#),
            Ok(DlCommand::Resume(7))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"cancel","id":7}"#),
            Ok(DlCommand::Cancel(7))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"restart","id":7}"#),
            Ok(DlCommand::Restart(7))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"remove","id":7}"#),
            Ok(DlCommand::Remove(7))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"open-file","id":7}"#),
            Ok(DlCommand::OpenFile(7))
        );
    }

    #[test]
    fn open_folder_id_is_optional_but_must_be_valid() {
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"open-folder"}"#),
            Ok(DlCommand::OpenFolder(None))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"open-folder","id":4}"#),
            Ok(DlCommand::OpenFolder(Some(4)))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"open-folder","id":0}"#),
            Err(DlDeny::Malformed)
        );
    }

    #[test]
    fn url_actions_validate_the_url() {
        // Ported from dlaction.
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"add-url","url":"https://example.com/f.zip"}"#),
            Ok(DlCommand::AddUrl("https://example.com/f.zip".to_string()))
        );
        assert_eq!(
            parse_dl_body(
                r#"{"kind":"dl","action":"refresh","id":2,"url":"https://example.com/f.zip"}"#
            ),
            Ok(DlCommand::Refresh(
                2,
                "https://example.com/f.zip".to_string()
            ))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"add-url","url":"file:///C:/f.zip"}"#),
            Err(DlDeny::Malformed)
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"refresh","id":2}"#),
            Err(DlDeny::Malformed),
            "refresh without its url"
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"add-url"}"#),
            Err(DlDeny::Malformed),
            "add-url without its url"
        );
    }

    #[test]
    fn connection_actions_clamp_to_the_core_range() {
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-connections","id":7,"conn":8}"#),
            Ok(DlCommand::SetConnections(7, 8))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-connections","id":7,"conn":99}"#),
            Ok(DlCommand::SetConnections(7, 32)),
            "clamped to MAX_CONNECTIONS"
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-connections","id":7,"conn":0}"#),
            Ok(DlCommand::SetConnections(7, 1)),
            "clamped to MIN_CONNECTIONS"
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-connections","id":7}"#),
            Err(DlDeny::Malformed)
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-connections","id":7,"connections":8}"#),
            Err(DlDeny::Malformed),
            "the wire name is conn (0.34 integration settlement): \
             the old connections spelling is an unknown field"
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-default-connections","conn":16}"#),
            Ok(DlCommand::SetDefaultConnections(16))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-default-connections"}"#),
            Err(DlDeny::Malformed)
        );
    }

    #[test]
    fn settings_actions_clamp() {
        // Ported from dlaction, verbatim expectations.
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-max-concurrent","maxConcurrent":99}"#),
            Ok(DlCommand::SetMaxConcurrent(5))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-max-concurrent","maxConcurrent":0}"#),
            Ok(DlCommand::SetMaxConcurrent(1))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-max-concurrent"}"#),
            Err(DlDeny::Malformed)
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-speed-limit","speedLimitKbps":0}"#),
            Ok(DlCommand::SetSpeedLimitKbps(None))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-speed-limit","speedLimitKbps":2000000}"#),
            Ok(DlCommand::SetSpeedLimitKbps(Some(1_000_000)))
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"set-speed-limit"}"#),
            Err(DlDeny::Malformed)
        );
    }

    #[test]
    fn unknown_action_wrong_kind_unknown_fields_and_client_only() {
        // Ported from dlaction, plus the 0.34 client-only row.
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"explode","id":1}"#),
            Err(DlDeny::UnknownAction)
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"action","action":"pause","id":1}"#),
            Err(DlDeny::Malformed)
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"pause","id":1,"extra":true}"#),
            Err(DlDeny::Malformed),
            "deny_unknown_fields: an extra field is malformed"
        );
        assert_eq!(
            parse_dl_body(r#"{"kind":"dl","action":"copy-link","id":1}"#),
            Err(DlDeny::ClientOnly),
            "copy-link is a real verb that never reaches Rust"
        );
        assert!(looks_like_dl_body(
            r#"{"kind":"dl","action":"pause","id":1}"#
        ));
        assert!(!looks_like_dl_body(r#"{"kind":"action","action":"pause"}"#));
        assert!(!looks_like_dl_body("not json"));
        assert_eq!(
            action_label(r#"{"kind":"dl","action":"pause","id":1}"#),
            "pause"
        );
        assert_eq!(action_label("junk"), "unknown");
        assert_eq!(
            action_label(r#"{"kind":"dl","action":"explode"}"#),
            "unknown"
        );
    }

    #[test]
    fn oversized_body_is_malformed() {
        let big = format!(
            r#"{{"kind":"dl","action":"add-url","url":"https://example.com/{}"}}"#,
            "a".repeat(MAX_DL_BODY_BYTES)
        );
        assert_eq!(parse_dl_body(&big), Err(DlDeny::Malformed));
        assert!(!looks_like_dl_body(&big));
    }
}
