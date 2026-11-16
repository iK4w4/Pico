//! Persisted app state: theme + recent files + window geometry
//! + the Shield's flag and lifetime counter.
//!
//! Stored as JSON in Pico's data directory. **0.41.0 reverses
//! the v27 portable design by the user's explicit order:**
//! ALL persistent state now lives in `%LOCALAPPDATA%\Pico\` —
//! state, log, downloads task state, and the WebView2 profile
//! (`%LOCALAPPDATA%\Pico\WebView2`). NOTHING is written beside
//! the exe ever again: no `data\`, no log, no `Downloads`
//! folder. The home is resolved ONCE by [`resolve_data_dir`]
//! over injected facts (the Windows known-folder value for
//! FOLDERID_LocalAppData first, then the `LOCALAPPDATA`
//! environment variable, then an APPDATA-derived Local path,
//! then a home-based path) and cached; the pure resolver is
//! host-tested with the values injected, exactly like the
//! patterns before it. On the first 0.41 run the legacy
//! homes — `<exe dir>/data/` (v27–0.40) and `%APPDATA%\Pico`
//! (pre-v27 / the v27 fallback) — are migrated into the new
//! home BEFORE anything reads state: copied, verified, and
//! only then (for the beside-exe tree) removed. See
//! [`migrate_legacy_home`]. Downloaded FILES are not state:
//! they land in the user's Windows Downloads folder
//! (`core::download::resolve_downloads_dir`).
//!
//! Schema **version 3** (v21 added `shield_on` and
//! `blocked_total`; v22's header finally says so — v21's said
//! "version 2" while shipping 3, AUDIT V22-8). Migration is
//! structural, not coded per-version: v1 files (v12.x:
//! `{theme, recents}`, no `version` key) and v2 files load
//! transparently because every field a later schema added
//! carries a serde default, and the next save writes v3.
//! Unknown *newer* versions are also loaded best-effort (serde
//! ignores unknown fields) because every field is independently
//! sanitized before use. A corrupt file never breaks startup:
//! [`load_from_checked`] reports the failure so the app can log
//! it, and the caller proceeds with defaults — the file itself
//! is left on disk untouched for inspection.
//!
//! v24 adds ONE field on the same structural rule:
//! `recents_hidden` (the start page's Recents section collapsed)
//! is serde-defaulted, so a verbatim v3 file from v21–v23 loads
//! with it `false` — schema stays 3, no migration (AUDIT V24-2).
//!
//! v25 adds ONE more field the same way: `lite_on` (Lite Mode).
//! Its serde default is `false` — Lite is opt-in, so files from
//! every earlier version load with it off (AUDIT V25).
//!
//! v27 adds `theme_id` (the palette; the mode stays in
//! `theme`). Its serde default is the EMPTY string, which is
//! the migration sentinel: a pre-v27 file has no palette, and
//! the app resolves empty → [`DEFAULT_THEME_ID`] on the first
//! v27 run and logs the migration (AUDIT V27-A2: the user's
//! "use this as the new theme" wins over silently preserving
//! the v26 look; Violet stays one picker click away). Unknown
//! non-empty ids sanitize to the default.
//!
//! 0.30 adds `stats` (the savings book — AUDIT 0.30) the same
//! structural way: serde-defaulted to a zero book, sanitized
//! on load by the stats module's own caps. The legacy
//! `blocked_total` stays as the downgrade-era mirror of the
//! Shield's lifetime count (the app keeps them equal), and a
//! pre-0.30 file's `blocked_total` seeds the book once at
//! startup when the book itself is empty. Authority inverted
//! in 0.30 (AUDIT 0.30, A2): the BOOK is now the source of
//! truth — a hand-edit to `blocked_total` alone is
//! overwritten from the book on the next save.
//!
//! 0.32 adds `download_connections` (the download manager's
//! global default connection count) the same structural way:
//! serde-defaulted to 8, and sanitized by clamping — 0 becomes
//! 8 and anything above 32 becomes 32 (the bounds are
//! `core::download`'s `DEFAULT_CONNECTIONS`/`MAX_CONNECTIONS`,
//! named here as literals so this module stays independent of
//! the download module).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_RECENTS: usize = 10;
pub const SCHEMA_VERSION: u32 = 3;

/// Window geometry persisted across runs (logical pixels, the
/// units tao's window builder accepts). Every component is
/// validated before it is allowed anywhere near the window
/// builder: a hand-edited state file must not place the window
/// off-screen or at a nonsense size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub maximized: bool,
}

/// Smallest window the app itself allows (matches the builder).
pub const MIN_WINDOW_W: u32 = 760;
pub const MIN_WINDOW_H: u32 = 500;
/// Largest believable coordinate/size for a single desktop.
/// Values beyond this are treated as corrupt, not clamped
/// silently — a window must land somewhere a human can see it.
const MAX_COORD: i32 = 32_768;
const MAX_SIZE: u32 = 16_384;

impl WindowRect {
    /// Is this rectangle sane enough to hand to the window
    /// builder?
    pub fn is_valid(&self) -> bool {
        // AUDIT V28 (A5-M2): the coordinate check runs in i64.
        // In release arithmetic i32::MIN.abs() wraps back to
        // i32::MIN (still negative, still passing `<= MAX_COORD`
        // comparisons in the wrong direction) — a crafted state
        // file could validate an off-screen-forever window.
        self.w >= MIN_WINDOW_W
            && self.h >= MIN_WINDOW_H
            && self.w <= MAX_SIZE
            && self.h <= MAX_SIZE
            && (self.x as i64).abs() <= MAX_COORD as i64
            && (self.y as i64).abs() <= MAX_COORD as i64
    }

    /// The rectangle if valid, else `None` (caller uses
    /// defaults).
    pub fn validated(self) -> Option<Self> {
        if self.is_valid() {
            Some(self)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistState {
    /// Schema version of this value. Absent in v1 files (= 0).
    #[serde(default)]
    pub version: u32,
    /// "light" or "dark". AUDIT V28 (A3-LOW-3): this was the
    /// one field without a serde default — a state file missing
    /// it failed to parse at all and was treated as corrupt.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Most-recent-first list of opened file/folder paths.
    #[serde(default)]
    pub recents: Vec<String>,
    /// Last window geometry, when known.
    #[serde(default)]
    pub window: Option<WindowRect>,
    /// Shield (ad blocker) enabled — v3 field. Defaults ON for
    /// files written before the Shield existed, matching the
    /// feature's shipped default.
    #[serde(default = "default_true")]
    pub shield_on: bool,
    /// Lifetime count of requests the Shield has blocked,
    /// across all sessions — v3 field. A count, not a history:
    /// no hosts or URLs are ever persisted.
    #[serde(default)]
    pub blocked_total: u64,
    /// Start page: the Recents section is collapsed to its
    /// header line — v24 field riding schema v3 via the serde
    /// default (see the module header).
    #[serde(default)]
    pub recents_hidden: bool,
    /// Lite Mode (v25): block images/fonts/media on public
    /// pages. Opt-in — the serde default is `false`, so every
    /// pre-v25 state file loads with Lite off.
    #[serde(default)]
    pub lite_on: bool,
    /// The palette id (v27; `core::themes`). Empty = pre-v27
    /// file, resolved by the app at first v27 run (see the
    /// module header). The light/dark MODE stays in `theme`.
    #[serde(default)]
    pub theme_id: String,
    /// AUDIT V28 (A3-M3): set by `sanitized()` when it had to
    /// replace an unknown non-empty palette id with the
    /// default. Never persisted; the app reads it once at
    /// startup and logs the rewrite, so a hand-edited or
    /// corrupt id no longer vanishes silently (and the next
    /// save makes the replacement permanent — the log line is
    /// the only trace, which is the point of having one).
    #[serde(skip)]
    pub theme_id_replaced: bool,
    /// The savings book's LIFETIME half (0.30): per-engine
    /// blocked counts, estimated bytes/time credited, the
    /// Shield's exact pruned-payload figures, and the
    /// response-size/duration calibration ledger those
    /// estimates are computed from. Counts and totals only —
    /// no hosts, no URLs, ever (the `blocked_total` rule).
    /// Serde-defaulted: pre-0.30 files load with a zero book.
    #[serde(default)]
    pub stats: crate::core::stats::Savings,
    /// The download manager's global default connection count
    /// (0.32): the count every newly added task starts with.
    /// Serde-defaulted to 8, so every pre-0.32 file loads with
    /// the standard default; `sanitized()` clamps it into the
    /// core range (0 → 8, > 32 → 32).
    #[serde(default = "default_download_connections")]
    pub download_connections: u32,
    /// How many downloads may run at once (0.33): the
    /// manager's concurrency cap, user-settable from the
    /// downloads page. Serde-defaulted to 3, so every
    /// pre-0.33 file loads with the standard default;
    /// `sanitized()` maps 0 → 3 and clamps into 1..=5.
    #[serde(default = "default_downloads_max_concurrent")]
    pub downloads_max_concurrent: u32,
    /// The global download speed limit in KiB/s (0.33);
    /// 0 = off. Serde-defaulted, so pre-0.33 files load
    /// unlimited; `sanitized()` caps it at 1_000_000.
    #[serde(default)]
    pub downloads_speed_limit_kbps: u64,
    /// The legacy beside-exe downloads folder
    /// (`<exe>\Downloads`, 0.41.0 Workstream D / F-A1):
    /// set ONCE by the app during the first-0.41 startup
    /// migration when that folder exists and differs from
    /// the new default downloads folder, and never set on
    /// a fresh install. Task file resolution
    /// (`core::download::resolve_task_file`) falls back
    /// to it when a task's file is not in the current
    /// downloads folder — the files themselves are never
    /// moved or copied and task records are never
    /// rewritten. Serde-defaulted to `None`, so every
    /// pre-WS-D file loads with no legacy folder;
    /// `sanitized()` drops anything that is not a
    /// plausible local path string (the recents rule).
    #[serde(default)]
    pub legacy_downloads_dir: Option<String>,
}

fn default_theme() -> String {
    "light".to_string()
}

/// The download manager's default connection count (0.32) —
/// `core::download::DEFAULT_CONNECTIONS`, as a literal so this
/// module does not depend on the download module.
fn default_download_connections() -> u32 {
    8
}

/// The downloads concurrency default (0.33).
fn default_downloads_max_concurrent() -> u32 {
    3
}

fn default_true() -> bool {
    true
}

impl Default for PersistState {
    fn default() -> Self {
        PersistState {
            version: SCHEMA_VERSION,
            theme: "light".to_string(),
            recents: Vec::new(),
            window: None,
            shield_on: true,
            blocked_total: 0,
            recents_hidden: false,
            lite_on: false,
            theme_id: crate::core::themes::DEFAULT_PALETTE_ID.to_string(),
            theme_id_replaced: false,
            stats: crate::core::stats::Savings::default(),
            download_connections: default_download_connections(),
            downloads_max_concurrent: default_downloads_max_concurrent(),
            downloads_speed_limit_kbps: 0,
            legacy_downloads_dir: None,
        }
    }
}

impl PersistState {
    /// Syntactically sanitize a state value that came from disk
    /// (or anywhere else untrusted): the theme must be one of
    /// the two known values, recents are capped, deduped, and
    /// stripped of anything that is not a plausible *local*
    /// path string, and the window rectangle must pass its
    /// geometry validation. This deliberately performs **no
    /// filesystem access** — existence is re-checked by the
    /// caller, and only after this lexical pass has rejected
    /// UNC/device-namespace strings (resolving one of those
    /// could itself contact a hostile SMB server).
    pub fn sanitized(mut self) -> Self {
        self.version = SCHEMA_VERSION;
        if self.theme != "light" && self.theme != "dark" {
            self.theme = "light".to_string();
        }
        // Palette (v27): the empty migration sentinel passes
        // through untouched (the app resolves it, so it can log
        // the migration); an unknown non-empty id falls back to
        // the default palette rather than an unstyled shell.
        if !self.theme_id.is_empty() && !crate::core::themes::is_known_palette(&self.theme_id) {
            self.theme_id = crate::core::themes::DEFAULT_PALETTE_ID.to_string();
            // A3-M3: remember that a rewrite happened (the app
            // logs it at startup; see the field's doc).
            self.theme_id_replaced = true;
        }
        let mut seen = std::collections::HashSet::new();
        self.recents.retain(|r| {
            r.len() <= 4096
                && !r.chars().any(|c: char| c.is_control())
                && crate::core::addr::is_safe_local_path(std::path::Path::new(r))
                && seen.insert(r.clone())
        });
        self.recents.truncate(MAX_RECENTS);
        self.window = self.window.and_then(WindowRect::validated);
        // The savings book (0.30): totals are capped at what
        // their own sample counts can support, engine figures
        // at the plausibility ceiling — see core::stats.
        self.stats = self.stats.sanitized();
        // Download connections (0.32): clamp into the core
        // range — 0 is the "unset" shape and becomes the
        // default 8; anything above the 32 ceiling clamps
        // down to it (core::download::MIN/MAX_CONNECTIONS).
        self.download_connections = match self.download_connections {
            0 => default_download_connections(),
            n if n > 32 => 32,
            n => n,
        };
        // Downloads settings (0.33): 0 is the "unset" shape
        // for the concurrency cap and becomes the default 3;
        // anything else clamps into 1..=5. The speed limit
        // is 0 = off, capped at 1_000_000 KiB/s.
        self.downloads_max_concurrent = match self.downloads_max_concurrent {
            0 => default_downloads_max_concurrent(),
            n => n.clamp(1, 5),
        };
        self.downloads_speed_limit_kbps = self.downloads_speed_limit_kbps.min(1_000_000);
        // Legacy downloads folder (0.41.0 WS-D, F-A1):
        // kept only when it is a plausible local path
        // string — same lexical gate as recents (no
        // filesystem access here; existence is the
        // resolver's question, answered per task file).
        self.legacy_downloads_dir = self.legacy_downloads_dir.take().filter(|dir| {
            !dir.is_empty()
                && dir.len() <= 4096
                && !dir.chars().any(|c: char| c.is_control())
                && crate::core::addr::is_safe_local_path(std::path::Path::new(dir))
        });
        self
    }
}

/// The pre-v27 per-user Pico data directory (`%APPDATA%\Pico`
/// on Windows — the ROAMING appdata — `$HOME/.config/Pico`
/// elsewhere). Since 0.41.0 this is only a MIGRATION SOURCE
/// (the v27 fallback home), never a live location.
pub fn appdata_pico_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.is_empty() {
            return PathBuf::from(appdata).join("Pico");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("Pico");
    }
    PathBuf::from("Pico")
}

/// The legacy beside-exe data directory (`<exe dir>/data`,
/// the v27–0.40 home). 0.41.0 reads it ONLY as the migration
/// source; nothing writes there anymore.
pub fn legacy_exe_data_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join("data")
}

/// The legacy beside-exe DOWNLOADS folder candidate
/// (0.41.0 Workstream D, F-A1): `<exe dir>\Downloads`,
/// where downloads finished (and `.part` files were
/// left) under ≤0.40.3. It becomes the persisted
/// `PersistState::legacy_downloads_dir` only when it
/// actually IS a directory and differs from the new
/// default downloads folder — a folder equal to the
/// current one needs no fallback, and a missing one
/// must never be recorded (a fresh install's field
/// stays unset). The directory test is injected so the
/// decision is pure and host-testable.
pub fn legacy_downloads_candidate_with(
    exe_dir: &Path,
    current_downloads: &Path,
    is_dir: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let candidate = exe_dir.join("Downloads");
    if candidate == current_downloads {
        return None;
    }
    if !is_dir(&candidate) {
        return None;
    }
    Some(candidate)
}

/// [`legacy_downloads_candidate_with`] against the real
/// filesystem.
pub fn legacy_downloads_candidate(exe_dir: &Path, current_downloads: &Path) -> Option<PathBuf> {
    legacy_downloads_candidate_with(exe_dir, current_downloads, |p| p.is_dir())
}

/// The Windows known folder for `folder_id`, via
/// SHGetKnownFolderPath (0.41.0). Windows-only: on every
/// other host this is `None` and the env chain decides —
/// which is also what makes the resolver host-testable (the
/// known-folder VALUE is just one more injected fact there).
#[cfg(windows)]
fn known_folder(folder_id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;
    let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
    let hr = unsafe { SHGetKnownFolderPath(folder_id, 0, std::ptr::null_mut(), &mut raw) };
    if hr < 0 || raw.is_null() {
        return None;
    }
    let mut len = 0usize;
    unsafe {
        while *raw.add(len) != 0 {
            len += 1;
        }
    }
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(raw, len) });
    unsafe {
        windows_sys::Win32::System::Com::CoTaskMemFree(raw as *const std::ffi::c_void);
    }
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

/// The FOLDERID_LocalAppData known folder (Windows only;
/// `None` elsewhere or on failure).
#[cfg(windows)]
pub fn known_local_app_data() -> Option<PathBuf> {
    known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_LocalAppData)
}

/// The FOLDERID_Downloads known folder (Windows only;
/// `None` elsewhere or on failure).
#[cfg(windows)]
pub fn known_downloads_dir() -> Option<PathBuf> {
    known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Downloads)
}

#[cfg(not(windows))]
pub fn known_local_app_data() -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
pub fn known_downloads_dir() -> Option<PathBuf> {
    None
}

/// Does a path STRING have an absolute shape, on either
/// platform's terms (AUDIT 0.41.1, Front A F11)? POSIX
/// (`/…`), Windows drive (`C:\…` / `C:/…`), and UNC
/// (`\\…`) forms all count. A pure string test, so the
/// resolver chain behaves identically on the test host.
fn looks_absolute(s: &str) -> bool {
    if s.starts_with('/') || s.starts_with("\\\\") {
        return true;
    }
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// The extended-length (`\\?\`) form of a Windows path
/// string, when one is needed (AUDIT 0.41.1, Front A F6):
/// Pico ships with NO embedded manifest (the gate law), so
/// the classic 260-char Win32 limit applies to plain paths
/// — and the new home's prefix is longer than the old
/// beside-exe one, so a deep profile file that fit under
/// `<exe>\data\…` can cross the limit under
/// `%LOCALAPPDATA%\Pico\…` and fail its migration copy on
/// EVERY start. Paths longer than
/// [`EXTENDED_PATH_MIN_CHARS`] are rewritten to the
/// verbatim form, which has no such limit. Returns `None`
/// for short paths, already-verbatim paths, and shapes
/// that are not Windows-absolute. Pure string work, so it
/// is unit-tested on any host.
pub fn verbatim_form(path_str: &str) -> Option<String> {
    if path_str.len() <= EXTENDED_PATH_MIN_CHARS || path_str.starts_with("\\\\?\\") {
        return None;
    }
    if let Some(rest) = path_str.strip_prefix("\\\\") {
        // UNC \\server\share\... → \\?\UNC\server\share\...
        return Some(format!("\\\\?\\UNC\\{}", rest.replace('/', "\\")));
    }
    let b = path_str.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
    {
        return Some(format!("\\\\?\\{}", path_str.replace('/', "\\")));
    }
    None
}

/// Path-length threshold above which migration file
/// operations use the extended-length form (see
/// [`verbatim_form`]): comfortably below the 260-char
/// classic limit, leaving room for the child names a
/// migration joins onto the root.
pub const EXTENDED_PATH_MIN_CHARS: usize = 240;

/// The form of `path` migration file operations should
/// use: the extended-length form when the path is long
/// enough to need it, the path itself otherwise. On
/// non-Windows-shaped paths this is always the identity.
fn migration_fs_path(path: &Path) -> PathBuf {
    verbatim_form(&path.to_string_lossy())
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

/// The pure Pico-home resolver (0.41.0). Facts in, decision
/// out — no I/O, so the whole chain is unit-tested:
///
/// 1. The Windows known folder FOLDERID_LocalAppData (the
///    shell injects its value; `None` off Windows or on
///    failure) → `<local>\Pico`.
/// 2. The `LOCALAPPDATA` env value → `<local>\Pico`.
/// 3. The `APPDATA` (Roaming) env value, with a trailing
///    `Roaming` component swapped for `Local` (the standard
///    profile layout) → `<local>\Pico`; a Roaming value with
///    no such component is used as-is (still per-user
///    AppData, still never beside the exe).
/// 4. `HOME` → `<home>/.local/share/Pico` (the non-Windows /
///    test fallback shape).
/// 5. Nothing known → the relative `Pico` (the current
///    directory): degraded, but never beside the exe by
///    design and never a world-writable substitute.
pub fn resolve_pico_home<'a>(
    known_local: Option<&'a Path>,
    env_local: Option<&'a Path>,
    env_roaming: Option<&'a Path>,
    home: Option<&'a Path>,
) -> (PathBuf, &'static str) {
    // AUDIT 0.41.1 (Front A F11): a candidate must be
    // non-empty AND absolute-shaped. A relative or
    // whitespace-only env value would make the home depend
    // on the process working directory (data "vanishes"
    // between launch methods); it now falls through to the
    // next candidate instead. The absoluteness test is a
    // string check so Windows-shaped values (`C:\…`, `\\…`)
    // count as absolute on every host — the resolver's
    // unit tests feed Windows paths on non-Windows hosts.
    let usable = |p: Option<&'a Path>| -> Option<&'a Path> {
        p.filter(|p| {
            let s = p.to_string_lossy();
            !s.trim().is_empty() && looks_absolute(&s)
        })
    };
    if let Some(local) = usable(known_local) {
        return (local.join("Pico"), "known folder FOLDERID_LocalAppData");
    }
    if let Some(local) = usable(env_local) {
        return (local.join("Pico"), "LOCALAPPDATA env");
    }
    if let Some(roaming) = usable(env_roaming) {
        // AUDIT 0.41.1 (Front A F11): the Roaming→Local swap
        // compares case-insensitively — `…\AppData\roaming`
        // is the same folder as `…\AppData\Roaming`, and an
        // exact-case miss used to leave the home INSIDE the
        // Roaming tree (silently un-relocating Pico, with
        // the log still claiming "APPDATA-derived Local").
        let local = match roaming.file_name() {
            Some(name) if name.to_string_lossy().eq_ignore_ascii_case("Roaming") => roaming
                .parent()
                .map(|p| p.join("Local"))
                .unwrap_or_else(|| roaming.to_path_buf()),
            _ => roaming.to_path_buf(),
        };
        return (local.join("Pico"), "APPDATA-derived Local");
    }
    if let Some(home) = usable(home) {
        return (
            home.join(".local").join("share").join("Pico"),
            "home fallback",
        );
    }
    (PathBuf::from("Pico"), "last-resort relative Pico")
}

/// Where Pico's runtime data lives this run, and why
/// (0.41.0). Resolved ONCE by [`resolve_data_dir`] over the
/// injected facts, then cached by [`data_dir_choice`]; the
/// reason string is what the startup log prints after
/// `data: appdata …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDirChoice {
    pub dir: PathBuf,
    pub reason: String,
}

/// The pure chooser (see [`resolve_pico_home`] for the
/// chain). Kept under the historical name: every path in
/// the app derives from this one answer.
pub fn resolve_data_dir<'a>(
    known_local: Option<&'a Path>,
    env_local: Option<&'a Path>,
    env_roaming: Option<&'a Path>,
    home: Option<&'a Path>,
) -> DataDirChoice {
    let (dir, why) = resolve_pico_home(known_local, env_local, env_roaming, home);
    DataDirChoice {
        dir,
        reason: why.to_string(),
    }
}

/// The user's Downloads folder from injected facts
/// (0.41.0): the FOLDERID_Downloads known folder when the
/// shell could read it, else `%USERPROFILE%\Downloads`.
/// Pure; the runtime wrapper is [`user_downloads_dir`].
pub fn downloads_dir_from(
    known_downloads: Option<&Path>,
    user_profile: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(d) = known_downloads.filter(|p| !p.as_os_str().is_empty()) {
        return Some(d.to_path_buf());
    }
    user_profile
        .filter(|p| !p.as_os_str().is_empty())
        .map(|home| home.join("Downloads"))
}

/// Runtime form of [`downloads_dir_from`]: known folder
/// first (Windows), then the USERPROFILE env value.
pub fn user_downloads_dir() -> Option<PathBuf> {
    let profile = std::env::var("USERPROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    downloads_dir_from(known_downloads_dir().as_deref(), profile.as_deref())
}

/// The process's executable directory, or the current
/// directory if even that is unknowable. 0.41.0 uses it ONLY
/// to find the legacy migration sources beside the exe.
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

static DATA_DIR: std::sync::OnceLock<DataDirChoice> = std::sync::OnceLock::new();

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// The chosen data directory + reason, resolved on first use
/// and cached for the process lifetime — every path in the
/// app derives from this one answer, so the state, the log
/// and the profile can never split across two locations
/// mid-run.
pub fn data_dir_choice() -> &'static DataDirChoice {
    DATA_DIR.get_or_init(|| {
        resolve_data_dir(
            known_local_app_data().as_deref(),
            env_path("LOCALAPPDATA").as_deref(),
            env_path("APPDATA").as_deref(),
            env_path("HOME").as_deref(),
        )
    })
}
/// Pico's data directory for this run (see [`data_dir_choice`]).
pub fn pico_dir() -> PathBuf {
    data_dir_choice().dir.clone()
}

pub fn state_path() -> PathBuf {
    pico_dir().join("state.json")
}

/// The WebView2 user-data directory: `<data dir>/WebView2`.
/// This folder IS the browser profile — cookies, storage, and
/// any signed-in session live here — so its path is
/// load-bearing: derived here exactly once from the resolved
/// data dir and pinned by tests (AUDIT V18-3, re-pinned
/// V22-16, moved by V27-C, moved again by 0.41.0 to
/// `%LOCALAPPDATA%\Pico\WebView2` — the MIGRATION below is
/// what carries an existing profile across the move, and
/// nothing in Pico ever deletes it). (The state sanitizer above cannot
/// touch it either — it operates on the in-memory
/// `PersistState` value.)
pub fn webview_data_dir() -> PathBuf {
    pico_dir().join("WebView2")
}

/// The Roaming appdata install's WebView2 profile folder
/// (a migration SOURCE; never a context folder).
pub fn appdata_webview_dir() -> PathBuf {
    appdata_pico_dir().join("WebView2")
}

/// What a migration should do about a legacy home
/// (0.41.0: the beside-exe `data\` tree or the Roaming
/// appdata tree, into the LocalAppData home).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationPlan {
    /// Nothing to migrate (no old data, or the new home
    /// already has state of its own — never clobber).
    None,
    /// Copy the whole legacy tree (state + profile +
    /// downloads task state) into the new home.
    Copy { from: PathBuf, to: PathBuf },
    /// Copy ONLY the state file: the new home already has
    /// a profile of its own, which is never overwritten
    /// (AUDIT V28, A5-H1).
    CopyStateOnly { from: PathBuf, to: PathBuf },
    /// Copy the whole legacy tree EXCEPT the state file
    /// (AUDIT 0.41.1, Front A F1/F2): the new home's state
    /// is LIVE (an interim session already saved its own,
    /// or it was always healthy) and must not be rewound
    /// to the legacy copy — but the tree (profile,
    /// downloads task state) is still owed. With
    /// `skip_profile`, the source's `WebView2` subtree is
    /// left out too: the destination already has a live
    /// profile of its own (the A5-H1 clobber law).
    CopyTreeOnly {
        from: PathBuf,
        to: PathBuf,
        skip_profile: bool,
        /// Fill ONLY what is missing (AUDIT 0.41.3): a
        /// file that already exists at the destination is
        /// skipped, never rewound. Only the Live-no-pending
        /// fill arm passes `true` — it completes a LIVE
        /// home, whose existing files are newer by
        /// definition of it being live. The pending-retry
        /// and corrupt-rescue arms pass `false`: their
        /// destinations are partial copies or rescues, and
        /// the copy stays overwrite-idempotent as before.
        missing_only: bool,
    },
}

/// Name of the migration-in-progress marker inside the
/// destination data dir (AUDIT V29, A5-1): written by the
/// performer before a full copy and removed only when the
/// copy completes cleanly. Its presence is how the planner
/// tells a PARTIAL destination profile (resume the copy —
/// the legacy source is its own origin) from a LIVE one
/// whose state was lost (never recopy — the v28 clobber
/// law).
pub const MIGRATION_PENDING_NAME: &str = "migration.pending";

/// The pure migration planner (0.41.0 form). Migration is
/// considered only when the destination has no state yet,
/// and only when the source actually holds something worth
/// carrying (a state file OR a profile folder).
///
/// AUDIT V28 (A5-H1): the planner also knows whether the
/// destination already has a PROFILE. A profile-only
/// destination (state lost, profile present) must never
/// get its live profile CLOBBERED by the stale source
/// copy; it plans a state-only copy instead. A leftover
/// pending marker overrides that: the destination profile
/// is then a PARTIAL copy of the source's, and the full
/// copy resumes.
pub fn plan_migration(
    dest_dir: &Path,
    source_dir: &Path,
    dest_has_state: bool,
    dest_has_profile: bool,
    source_has_state: bool,
    source_has_profile: bool,
    dest_has_pending: bool,
) -> MigrationPlan {
    if dest_has_state || !(source_has_state || source_has_profile) {
        return MigrationPlan::None;
    }
    // A leftover pending marker means the destination
    // profile is a partial copy of the source's: resume
    // the FULL copy (overwrite-idempotent), never the
    // state-only shortcut.
    if dest_has_pending {
        return MigrationPlan::Copy {
            from: source_dir.to_path_buf(),
            to: dest_dir.to_path_buf(),
        };
    }
    if dest_has_profile {
        return if source_has_state {
            MigrationPlan::CopyStateOnly {
                from: source_dir.to_path_buf(),
                to: dest_dir.to_path_buf(),
            }
        } else {
            MigrationPlan::None
        };
    }
    MigrationPlan::Copy {
        from: source_dir.to_path_buf(),
        to: dest_dir.to_path_buf(),
    }
}

/// Outcome of a performed migration, for the startup log.
/// `profile_files` counts every file copied OTHER than
/// `state.json` (the WebView2 profile tree is the bulk of
/// it; downloads task files count too since 0.41.0, whose
/// performer copies the whole legacy tree, not just the
/// profile). `skipped` counts everything not copied;
/// `failed` counts only entries that SHOULD have copied
/// and could not (AUDIT V29, A5-1/A5-8: the
/// completion-marker gate keys on failures — a
/// deliberately-uncopied symlink is honest in `skipped`
/// but must not stall migration forever).
///
/// AUDIT 0.34.1 (field addition): `details` itemizes the
/// FAILURES and the PURE SKIPS (never the successes — a
/// profile tree is thousands of files). Through 0.34 the
/// report was counts only, so a field log could say
/// "1 failed, 1 skipped" without naming WHAT failed; the
/// per-item lines the app logs from `details` are the fix.
/// (The `Copy` derive goes with this change: a `Vec` field
/// cannot be `Copy`.)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MigrationReport {
    pub state_copied: bool,
    pub profile_files: usize,
    pub skipped: usize,
    pub failed: usize,
    pub details: Vec<MigrationDetail>,
}

/// One itemized migration outcome (0.34.1): a single failed
/// copy or a single deliberate skip. `path` is RELATIVE to
/// the migration source root (e.g.
/// `WebView2/Default/Network/Cookies`; `.` for the tree
/// root itself) — never an absolute path, never contents.
/// `error` is the io error's Display for a failure and
/// `None` for a deliberate skip; for skips the reason rides
/// in `kind` (`(symlink)` / `(depth cap)` / `(exotic)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationDetail {
    pub kind: &'static str,
    pub path: String,
    pub error: Option<String>,
}

impl MigrationReport {
    /// Record one FAILED item: counts as failed AND
    /// skipped (the standing counting rule) and itemized.
    fn note_failure(&mut self, kind: &'static str, path: String, error: std::io::Error) {
        self.skipped += 1;
        self.failed += 1;
        self.details.push(MigrationDetail {
            kind,
            path,
            error: Some(error.to_string()),
        });
    }

    /// Record one PURE SKIP: counted as skipped only (it
    /// must not stall the completion marker) and itemized.
    fn note_skip(&mut self, kind: &'static str, path: String) {
        self.skipped += 1;
        self.details.push(MigrationDetail {
            kind,
            path,
            error: None,
        });
    }
}

/// A path rendered relative to the migration source root,
/// with `/` separators on every platform (the log form).
/// Anything outside the root (never expected) falls back
/// to its file name rather than leaking an absolute path.
fn migration_rel(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string()),
    }
}

/// Is this top-level legacy entry the log, the pending
/// marker, or a state temp — entries the tree copy must
/// not carry verbatim (0.41.0)? The log is carried by
/// [`carry_legacy_log`] (append-shaped, not copy-shaped);
/// temps are crash leftovers by definition.
fn migration_excluded_top(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name == "pico.log"
        || name == MIGRATION_PENDING_NAME
        || (name.starts_with("state.json.") && name.ends_with(".tmp"))
}

/// Perform a planned migration. Best-effort per file — a
/// file that cannot be read or written (locked, vanished
/// mid-copy) is counted and itemized, never fatal. Must
/// run BEFORE any WebView2 context is created against the
/// new profile and before anything reads state (the app
/// runs it during startup, before the log even opens —
/// the log itself initializes into the NEW home).
///
/// AUDIT V28 (A5-H1/M1): for a full copy everything else
/// is copied first and `state.json` LAST, atomically
/// (pid-unique temp + rename) — state presence is the
/// planner's completion marker, so it may only appear
/// once everything else has had its chance; an
/// interrupted run is re-planned next start instead of
/// standing down forever. 0.41.0: the full copy carries
/// the WHOLE legacy tree (profile, downloads task state,
/// any other Pico file), not just `WebView2\` — the
/// exclusions are the log (carried separately), the
/// pending marker, and state temps.
pub fn perform_migration(plan: &MigrationPlan) -> MigrationReport {
    // AUDIT 0.41.1: the plan decomposes into (copy the
    // tree?, copy the state?) plus the tree copy's profile
    // guard. `Copy` is (yes, yes); `CopyStateOnly` is
    // (no, yes); `CopyTreeOnly` is (yes, no) — the
    // destination state is live and is never rewound.
    let (from, to, tree, copy_state, skip_profile, missing_only) = match plan {
        MigrationPlan::Copy { from, to } => (from, to, true, true, false, false),
        MigrationPlan::CopyStateOnly { from, to } => (from, to, false, true, false, false),
        MigrationPlan::CopyTreeOnly {
            from,
            to,
            skip_profile,
            missing_only,
        } => (from, to, true, false, *skip_profile, *missing_only),
        MigrationPlan::None => return MigrationReport::default(),
    };
    let full = tree;
    let mut report = MigrationReport::default();
    let _ = std::fs::create_dir_all(to);
    // AUDIT V29 (A5-1): mark the copy in progress BEFORE
    // anything lands; the marker is removed only at clean
    // completion (below). A crash or a failed file leaves
    // it behind, and the planner (see plan_migration)
    // resumes the full copy instead of mistaking the
    // partial tree for a live one.
    let pending = to.join(MIGRATION_PENDING_NAME);
    if full {
        let _ = std::fs::write(&pending, b"pico migration in progress\n");
        // Deterministic order (sorted names) so reports
        // and retries behave identically across runs.
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(from) {
            Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
            Err(e) => {
                report.note_failure("tree root", ".".to_string(), e);
                return report;
            }
        };
        entries.sort();
        for src in entries {
            let Some(name) = src.file_name() else {
                continue;
            };
            if name == "state.json" || migration_excluded_top(name) {
                continue;
            }
            // AUDIT 0.41.1 (Front A F2): a tree-only fill
            // over a LIVE destination profile never touches
            // the source's profile subtree (A5-H1).
            if skip_profile && name == "WebView2" {
                continue;
            }
            let dst = to.join(name);
            match std::fs::metadata(&src) {
                Ok(meta) if meta.is_dir() => {
                    copy_tree_rooted(&src, &dst, from, &mut report, missing_only);
                }
                // AUDIT 0.41.3: a missing-only fill never
                // rewinds a live destination file — an
                // existing file is a pure skip, itemized
                // with kind "(exists)".
                Ok(meta) if meta.is_file() && missing_only && dst.exists() => {
                    report.note_skip("(exists)", migration_rel(from, &src));
                }
                Ok(meta) if meta.is_file() => match copy_atomic(&src, &dst) {
                    Ok(()) => report.profile_files += 1,
                    Err(e) => {
                        report.note_failure("file", migration_rel(from, &src), e);
                    }
                },
                Ok(_) => {
                    report.note_skip("entry (exotic)", migration_rel(from, &src));
                }
                Err(e) => {
                    report.note_failure("entry", migration_rel(from, &src), e);
                }
            }
        }
    }
    // AUDIT V29 (A5-1): state is the planner's completion
    // marker, so it may only land when the copy phase had
    // no FAILURES. Through v28 any per-file failure (a
    // locked Cookies file, an AV hold) still landed state
    // — the planner then stood down forever with a
    // half-copied profile and a success-shaped log line.
    // Now a failed copy leaves the marker absent: the plan
    // is re-issued next start, and the copy is
    // overwrite-idempotent, so the retry simply resumes.
    if full && report.failed > 0 {
        return report;
    }
    // AUDIT 0.41.1: a tree-only plan never lands state —
    // the destination's own state is the live one.
    if copy_state {
        let src_state = from.join("state.json");
        if src_state.is_file() {
            match copy_atomic(&src_state, &to.join("state.json")) {
                Ok(()) => report.state_copied = true,
                Err(e) => {
                    // 0.34.1: the error is CAPTURED into the
                    // detail, not discarded — a failed state
                    // copy is itemized like any other failure.
                    report.note_failure("state file", "state.json".to_string(), e);
                }
            }
        }
    }
    // Clean completion retires the pending marker. (The
    // failure path above RETURNS EARLY and never reaches
    // this: the marker — and the withheld state — are what
    // make the next start resume honestly.)
    if full && report.failed == 0 {
        let _ = std::fs::remove_file(&pending);
    }
    report
}

/// Verify a performed migration copy (0.41.0): every file
/// the source holds (same exclusions as the copy) must
/// exist at the destination with the same length, and
/// `state.json` — the file whose loss is silent data loss
/// — must additionally be byte-identical. Symlink entries
/// are exempt (the copy deliberately skips them).
/// Destination extras (the new home's own log, say) are
/// ignored. This is the gate the legacy tree's REMOVAL
/// keys on: unverified means untouched, however complete
/// the copy looks.
pub fn verify_migration_copy(from: &Path, to: &Path) -> bool {
    verify_migration_copy_scoped(
        from,
        to,
        VerifyScope::Full {
            include_profile: true,
        },
    )
}

/// Which part of a performed copy a verification covers
/// (AUDIT 0.41.1, Front A F2). Through 0.41.0 the single
/// full-tree verify ran for EVERY plan — including
/// `CopyStateOnly`, which by design copies nothing but
/// the state file, so its verification demanded files
/// the plan never carried and could essentially never
/// pass (the source then stayed forever, the log said
/// NOT VERIFIED forever, and any files the plan did not
/// cover were stranded). Each plan now verifies exactly
/// what it copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyScope {
    /// Everything the full copy carries, state included.
    Full { include_profile: bool },
    /// Only `state.json`, byte-identical.
    StateOnly,
    /// Everything except `state.json` (the destination
    /// state is live and is not the copy's business).
    /// `missing_only` (AUDIT 0.41.3) mirrors the plan's
    /// fill discipline — see the verify walk for what it
    /// changes about the check.
    TreeOnly {
        include_profile: bool,
        missing_only: bool,
    },
}

/// Files at or below this size are byte-compared during
/// verification, not just length-compared (AUDIT 0.41.1,
/// Front A F8): a same-length torn copy of a small,
/// high-value file (Cookies, History, a task JSON) used
/// to verify clean right before the only other copy was
/// deleted. Large files stay length-compared — the copy
/// is a plain `fs::copy` of a quiescent tree, and reading
/// back gigabytes of cache would punish every migration.
const VERIFY_BYTE_COMPARE_MAX: u64 = 1 << 20;

fn verify_migration_copy_scoped(from: &Path, to: &Path, scope: VerifyScope) -> bool {
    if matches!(scope, VerifyScope::StateOnly) {
        let (Ok(s), Ok(d)) = (
            std::fs::read(from.join("state.json")),
            std::fs::read(to.join("state.json")),
        ) else {
            return false;
        };
        return s == d;
    }
    let (include_profile, missing_only) = match scope {
        VerifyScope::Full { include_profile } => (include_profile, false),
        VerifyScope::TreeOnly {
            include_profile,
            missing_only,
        } => (include_profile, missing_only),
        VerifyScope::StateOnly => unreachable!(),
    };
    // AUDIT 0.41.1 (Front A F2): the walk mirrors the
    // performer's depth discipline EXACTLY — the copy
    // skips subtrees deeper than COPY_TREE_MAX_DEPTH as a
    // pure skip, and through 0.41.0 this uncapped walk
    // then demanded those very files at the destination,
    // failing verification permanently. Top-level entries
    // are depth-less (the performer's top loop); each
    // top-level directory is walked at depth 0, like
    // `copy_tree_rooted`.
    fn walk_dir(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> bool {
        if depth > COPY_TREE_MAX_DEPTH {
            return true;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in rd {
            let Ok(entry) = entry else { return false };
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if !walk_dir(&path, depth + 1, out) {
                    return false;
                }
            } else if kind.is_file() {
                out.push(path);
            }
        }
        true
    }
    let mut files = Vec::new();
    let Ok(rd) = std::fs::read_dir(from) else {
        return false;
    };
    for entry in rd {
        let Ok(entry) = entry else { return false };
        let Ok(kind) = entry.file_type() else {
            return false;
        };
        let path = entry.path();
        if kind.is_symlink() {
            // AUDIT 0.41.3 (A-S2): the performer follows
            // TOP-LEVEL symlinks (its top loop reads
            // `fs::metadata`, which resolves them), so a
            // top-level symlink-to-dir is copied through
            // like a directory — the verify walk must
            // treat it the same way, with the same depth
            // discipline, or files copied through a
            // junction are never actually checked.
            // (Symlinks BELOW the top level stay skipped
            // in both walks: the performer's tree copy
            // does not follow them either.)
            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => {
                    if !walk_dir(&path, 0, &mut files) {
                        return false;
                    }
                }
                Ok(meta) if meta.is_file() => files.push(path),
                _ => continue,
            }
            continue;
        }
        if kind.is_dir() {
            if !walk_dir(&path, 0, &mut files) {
                return false;
            }
        } else if kind.is_file() {
            files.push(path);
        }
    }
    for src in files {
        let Ok(rel) = src.strip_prefix(from) else {
            return false;
        };
        if rel.components().count() == 1 {
            if let Some(name) = rel.file_name() {
                if migration_excluded_top(name) {
                    continue;
                }
            }
        }
        if !include_profile
            && rel.components().next() == Some(std::path::Component::Normal("WebView2".as_ref()))
        {
            continue;
        }
        if matches!(scope, VerifyScope::TreeOnly { .. }) && rel == Path::new("state.json") {
            continue;
        }
        let dst = to.join(rel);
        let (Ok(smeta), Ok(dmeta)) = (std::fs::metadata(&src), std::fs::metadata(&dst)) else {
            return false;
        };
        if !dmeta.is_file() || smeta.len() != dmeta.len() {
            return false;
        }
        // AUDIT 0.41.1 (Front A F8): byte-compare the
        // small files (and state.json at any size), not
        // just their lengths.
        // AUDIT 0.41.3: a missing-only fill is the one
        // exception — its destination files that already
        // existed were deliberately NOT copied, so their
        // bytes are the live home's own, not the source's;
        // byte-comparing them would fail every honest
        // fill. Presence + equal length is the honest
        // fill check: which individual files the
        // performer actually copied is not knowable
        // post-hoc, so no stronger claim is made here.
        if !missing_only
            && (rel == Path::new("state.json") || smeta.len() <= VERIFY_BYTE_COMPARE_MAX)
        {
            let (Ok(s), Ok(d)) = (std::fs::read(&src), std::fs::read(&dst)) else {
                return false;
            };
            if s != d {
                return false;
            }
        }
    }
    true
}

/// Carry the legacy log into the new home (0.41.0). Runs
/// BEFORE the logger initializes into the new home, so
/// there is no open handle to fight: when the new home
/// has no `pico.log` yet, the legacy log becomes it
/// verbatim (the new session appends after the old
/// history); when one already exists (a migration
/// retry), the legacy log is preserved beside it as
/// `pico-legacy.log` instead of being merged twice.
fn carry_legacy_log(from: &Path, to: &Path) -> bool {
    let src = from.join("pico.log");
    if !src.is_file() {
        return false;
    }
    let dst = if to.join("pico.log").exists() {
        to.join("pico-legacy.log")
    } else {
        to.join("pico.log")
    };
    std::fs::copy(&src, &dst).is_ok()
}

/// The full first-run legacy migration outcome (0.41.0),
/// for the startup log: every step the app must report —
/// whether a migration ran at all, its copy report, the
/// log carry, the verification verdict, and whether the
/// legacy tree was removed (only ever after a verified
/// copy, and only when the caller asked for removal —
/// the beside-exe tree; the Roaming tree stays as the
/// standing backup, as it did through the v27 move).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LegacyMigrationOutcome {
    pub attempted: bool,
    pub report: MigrationReport,
    pub log_carried: bool,
    pub verified: bool,
    pub source_removed: bool,
    pub remove_failed: bool,
    /// The source held a `pico.log` (AUDIT 0.41.1, Front
    /// A F7): lets the caller log honestly when the carry
    /// failed, and gates source removal (a verified copy
    /// whose log never arrived does not authorize deleting
    /// the only copy of that log).
    pub source_had_log: bool,
    /// Removal was withheld because the legacy log could
    /// not be carried (AUDIT 0.41.1, Front A F7) — the
    /// source stays until a start manages the carry.
    pub removal_withheld_for_log: bool,
    /// When removal renamed the source aside but could
    /// not finish deleting it: where the remains live
    /// (AUDIT 0.41.1, Front A F5), so the log can say
    /// exactly what is left instead of "left in place"
    /// about a half-deleted tree.
    pub remove_leftover: Option<PathBuf>,
}

/// The destination state file's health, for the
/// migration stand-down decision (AUDIT 0.41.1, Front A
/// F12): through 0.41.0 the check was existence-only, so
/// a truncated or corrupt `state.json` counted as a live
/// home and blocked rescue by an intact legacy state.
enum DestState {
    Absent,
    Live,
    Corrupt,
}

/// Remove a migrated legacy source (AUDIT 0.41.1, Front
/// A F5): the tree is first RENAMED to a sibling
/// quarantine name, and only the renamed tree is
/// deleted. Through 0.41.0, verify walked the tree and
/// `remove_dir_all` then deleted whatever was in the
/// source at deletion time — anything written into the
/// source between the two walks (a second Pico instance
/// starting mid-migration, a script, an old build still
/// running) was deleted unverified. After the rename,
/// new writes to the original path land in a fresh
/// folder that this removal never touches. A removal
/// that fails after the rename reports the quarantine
/// path, so the remains are findable.
fn remove_migrated_source(source: &Path, outcome: &mut LegacyMigrationOutcome) {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "data".to_string());
    let quarantine = source.with_file_name(format!("{name}.pico-migrated-{}", std::process::id()));
    if quarantine.exists() {
        // Our own earlier leftover: it was already
        // verified-and-renamed once; clearing it is safe.
        let _ = std::fs::remove_dir_all(&quarantine);
    }
    match std::fs::rename(source, &quarantine) {
        Ok(()) => match std::fs::remove_dir_all(&quarantine) {
            Ok(()) => outcome.source_removed = true,
            Err(_) => {
                outcome.remove_failed = true;
                outcome.remove_leftover = Some(quarantine);
            }
        },
        Err(_) => outcome.remove_failed = true,
    }
}

/// Migrate one legacy home into the new home (0.41.0):
/// plan → copy → carry the log → verify → (optionally)
/// remove the source. A source equal to the destination,
/// or not a directory at all, is a no-op. On ANY failure
/// or failed verification the source is left exactly as
/// it was and Pico runs from the migrated copies; the
/// plan re-issues next start.
///
/// AUDIT 0.41.1 (Front A F1): through 0.41.0 ANY dest
/// `state.json` stood the migration down and retired the
/// pending marker — but a FAILED copy leaves the marker
/// and withholds state precisely so the next start
/// resumes, and the interim session then saves its own
/// state (the F-A1 legacy-downloads save does so
/// deterministically). The retry was therefore defeated
/// by the session it was promised to: pending is now
/// consulted FIRST, and a pending migration over a live
/// dest state completes the TREE only (the interim state
/// is newer and is never rewound).
pub fn migrate_legacy_home(
    source: &Path,
    dest: &Path,
    remove_source: bool,
) -> LegacyMigrationOutcome {
    let outcome = LegacyMigrationOutcome::default();
    // AUDIT 0.41.3 (A-S1): a relative source stands down.
    // A CWD-relative `data` must never become a migration
    // source — its "verified removal" would delete
    // whatever folder the process's working directory
    // happens to name, which is not a legacy home.
    if !source.is_absolute() {
        return outcome;
    }
    if source == dest || !source.is_dir() {
        return outcome;
    }
    // AUDIT 0.41.1 (Front A F6): the file operations run
    // on extended-length roots when a root is long enough
    // to need them (the outcome and the caller keep
    // speaking of the original paths).
    let source_fs = migration_fs_path(source);
    let dest_fs = migration_fs_path(dest);
    let source = &source_fs;
    let dest = &dest_fs;
    let dest_state = match load_from_checked(&dest.join("state.json")) {
        LoadOutcome::Loaded(_) => DestState::Live,
        LoadOutcome::Missing => DestState::Absent,
        LoadOutcome::Corrupt(_) => DestState::Corrupt,
    };
    let pending = dest.join(MIGRATION_PENDING_NAME).is_file();
    let source_has_state = source.join("state.json").is_file();
    let source_has_profile = source.join("WebView2").is_dir();
    let dest_has_profile = dest.join("WebView2").is_dir();

    if matches!(dest_state, DestState::Live) && !pending {
        // A healthy live home stands the migration down —
        // with ONE completion exception (AUDIT 0.41.1,
        // Front A F2/F4): the tree may still owe this home
        // a profile it lacks, or non-profile files (the
        // downloads task state) that a state-only copy
        // never carried. Fill exactly what is missing,
        // never touching the live state, and never
        // overwriting a live profile.
        let fill_profile = !dest_has_profile && source_has_profile;
        let fill_tree = legacy_tree_fill_needed(source, dest);
        if !fill_profile && !fill_tree {
            return outcome;
        }
        let plan = MigrationPlan::CopyTreeOnly {
            from: source.to_path_buf(),
            to: dest.to_path_buf(),
            skip_profile: dest_has_profile,
            // AUDIT 0.41.3: the fill arm fills only what
            // is missing — a live home's existing files
            // (a NEWER downloads task file, say) are
            // never rewound to the legacy copy.
            missing_only: true,
        };
        return run_migration_plan(plan, source, dest, remove_source, outcome);
    }

    let plan = if pending {
        // An earlier copy did not complete. Over a live
        // dest state, complete the tree only; otherwise
        // resume the full copy (state lands last, as
        // always).
        if matches!(dest_state, DestState::Live) {
            MigrationPlan::CopyTreeOnly {
                from: source.to_path_buf(),
                to: dest.to_path_buf(),
                // AUDIT 0.41.3 (Front A F1 companion):
                // the interim session's LIVE profile must
                // never be rewound either — this arm used
                // to pass `false` unconditionally, unlike
                // both sibling arms. The tree copy stays
                // overwrite-shaped for everything else
                // (missing_only: false): the destination
                // tree outside the profile is a partial
                // copy of this very source, not live data.
                skip_profile: dest_has_profile,
                missing_only: false,
            }
        } else {
            MigrationPlan::Copy {
                from: source.to_path_buf(),
                to: dest.to_path_buf(),
            }
        }
    } else if matches!(dest_state, DestState::Corrupt) {
        // AUDIT 0.41.1 (Front A F12): a corrupt dest
        // state does not stand the migration down when
        // the legacy tree holds a state to rescue it
        // with; plan as if the dest had no state (the
        // copy lands state atomically over the corrupt
        // file). A corrupt state with nothing to rescue
        // it is the app loader's own affair.
        if !source_has_state && !source_has_profile {
            return outcome;
        }
        if source_has_profile && !source_has_state {
            MigrationPlan::CopyTreeOnly {
                from: source.to_path_buf(),
                to: dest.to_path_buf(),
                skip_profile: dest_has_profile,
                missing_only: false,
            }
        } else {
            plan_migration(
                dest,
                source,
                false,
                dest_has_profile,
                source_has_state,
                source_has_profile,
                false,
            )
        }
    } else {
        plan_migration(
            dest,
            source,
            false,
            dest_has_profile,
            source_has_state,
            source_has_profile,
            false,
        )
    };
    if matches!(plan, MigrationPlan::None) {
        return outcome;
    }
    run_migration_plan(plan, source, dest, remove_source, outcome)
}

/// Does the legacy tree hold a non-state, non-profile
/// top-level entry the destination lacks (AUDIT 0.41.1,
/// Front A F2)? Presence at the top level is the check —
/// the stranded class is whole folders (the downloads
/// task state), not individual files.
fn legacy_tree_fill_needed(source: &Path, dest: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(source) else {
        return false;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        if name == "state.json" || name == "WebView2" || migration_excluded_top(&name) {
            continue;
        }
        if !dest.join(&name).exists() {
            return true;
        }
    }
    false
}

/// The shared tail of [`migrate_legacy_home`]: perform,
/// carry the log, verify what the plan copied, and —
/// only for a verified copy whose log arrived — remove
/// the source when asked.
fn run_migration_plan(
    plan: MigrationPlan,
    source: &Path,
    dest: &Path,
    remove_source: bool,
    mut outcome: LegacyMigrationOutcome,
) -> LegacyMigrationOutcome {
    let scope = match &plan {
        MigrationPlan::Copy { .. } => VerifyScope::Full {
            include_profile: true,
        },
        MigrationPlan::CopyStateOnly { .. } => VerifyScope::StateOnly,
        MigrationPlan::CopyTreeOnly {
            skip_profile,
            missing_only,
            ..
        } => VerifyScope::TreeOnly {
            include_profile: !skip_profile,
            missing_only: *missing_only,
        },
        MigrationPlan::None => return outcome,
    };
    outcome.attempted = true;
    outcome.source_had_log = source.join("pico.log").is_file();
    outcome.report = perform_migration(&plan);
    outcome.log_carried = carry_legacy_log(source, dest);
    outcome.verified =
        outcome.report.failed == 0 && verify_migration_copy_scoped(source, dest, scope);
    // AUDIT 0.41.1: source removal — the one irreversible
    // step — keys on a FULL verified copy only. A
    // state-only or tree-only plan carried a subset by
    // design; the source remains the standing backup for
    // the part it still solely holds (its state in the
    // tree-only case, its profile in the state-only case).
    let full_copy = matches!(plan, MigrationPlan::Copy { .. });
    if outcome.verified && remove_source && full_copy {
        // AUDIT 0.41.1 (Front A F7): the carried log is
        // part of what "verified" must mean before the
        // only other copy is deleted — through 0.41.0 a
        // failed carry was silent and the source (with
        // the log) was removed anyway.
        if outcome.source_had_log && !outcome.log_carried {
            outcome.removal_withheld_for_log = true;
        } else {
            remove_migrated_source(source, &mut outcome);
        }
    }
    outcome
}

/// Migrate a beside-exe WebView2 user-data folder into
/// the new home (0.41.0). What the current build creates:
/// nothing beside the exe — every WebView is built on the
/// one explicit [`webview_data_dir`] context (pinned by a
/// source-scan test). But a default-context WebView2 (an
/// older experiment, another build) leaves
/// `<exe>\Pico.exe.WebView2\` — a full user-data folder
/// whose `EBWebView` subtree is the profile — and some
/// shapes leave a bare `<exe>\EBWebView\`. Both are
/// carried into `<new home>\WebView2` (contents, so
/// `EBWebView` lands where the runtime expects it)
/// BEFORE any WebView is created. Best-effort per file:
/// a locked file counts as failed in the report and the
/// source is left in place — this migration never
/// removes anything. An existing destination profile is
/// never clobbered.
/// Marker names for the beside-exe WebView2 migration
/// (AUDIT 0.41.1, Front A F3), kept in the destination
/// home: `webview-migration.pending` while a copy is
/// incomplete, `webview-migration.done` once one has
/// completed cleanly.
pub const WEBVIEW_MIGRATION_PENDING_NAME: &str = "webview-migration.pending";
/// See [`WEBVIEW_MIGRATION_PENDING_NAME`].
pub const WEBVIEW_MIGRATION_DONE_NAME: &str = "webview-migration.done";

pub fn migrate_beside_exe_webview(exe_dir: &Path, dest_home: &Path) -> (bool, MigrationReport) {
    let mut report = MigrationReport::default();
    let mut attempted = false;
    // AUDIT 0.41.1 (Front A F6): long roots get the
    // extended-length form for the file operations.
    let exe_dir_fs = migration_fs_path(exe_dir);
    let dest_home_fs = migration_fs_path(dest_home);
    let exe_dir = &exe_dir_fs;
    let dest_home = &dest_home_fs;
    let dest_wv = dest_home.join("WebView2");
    let candidates: [(PathBuf, PathBuf); 2] = [
        (exe_dir.join("Pico.exe.WebView2"), dest_wv.clone()),
        (exe_dir.join("EBWebView"), dest_wv.join("EBWebView")),
    ];
    // AUDIT 0.41.1 (Front A F3): through 0.41.0 the only
    // stand-down guard was "the EBWebView directory
    // exists at the destination" — but the copy CREATES
    // that directory before copying any file, so one
    // interrupted or locked-file attempt left an empty or
    // partial profile that blocked every retry forever
    // (and blocked the second candidate in the same run).
    // The migration now keeps the main migration's marker
    // discipline: stand down only on a recorded CLEAN
    // completion, or when a destination profile exists
    // that this migration did not start (a live profile —
    // never clobbered). A leftover pending marker means
    // the partial tree is ours: resume it.
    let done_marker = dest_home.join(WEBVIEW_MIGRATION_DONE_NAME);
    if done_marker.is_file() {
        return (attempted, report);
    }
    let pending_marker = dest_home.join(WEBVIEW_MIGRATION_PENDING_NAME);
    if dest_wv.join("EBWebView").is_dir() && !pending_marker.is_file() {
        return (attempted, report);
    }
    if !candidates.iter().any(|(src, _)| src.is_dir()) {
        return (attempted, report);
    }
    let _ = std::fs::create_dir_all(dest_home);
    let _ = std::fs::write(&pending_marker, b"pico webview migration in progress\n");
    for (src, dst) in candidates {
        if !src.is_dir() {
            continue;
        }
        // The second candidate is the fallback spelling
        // of the same profile: it runs only when the
        // first delivered no files at all.
        if attempted && report.profile_files > 0 {
            break;
        }
        attempted = true;
        copy_tree_rooted(&src, &dst, exe_dir, &mut report, false);
    }
    if attempted && report.failed == 0 {
        let _ = std::fs::write(&done_marker, b"pico webview migration complete\n");
        let _ = std::fs::remove_file(&pending_marker);
    }
    (attempted, report)
}
/// Flush a just-copied file to disk (AUDIT 0.41.1, Front
/// A F9). The handle is opened for writing because
/// `sync_all` needs write access on Windows (the 0.40.2
/// lesson from `copy_atomic`).
fn sync_copied_file(path: &Path) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.sync_all()
}

/// Copy one file atomically: write to a per-process temp
/// sibling, fsync, then rename over the destination, so a
/// crash or a second concurrent instance can never leave a
/// torn file at the real path (AUDIT V28, A5-M1; the fsync is
/// AUDIT V29, A5-8 — rename alone publishes whatever the OS
/// had buffered, which a power loss can still tear).
fn copy_atomic(src: &Path, dst: &Path) -> std::io::Result<()> {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state.json".to_string());
    let tmp = dst.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let copied = (|| -> std::io::Result<()> {
        std::fs::copy(src, &tmp)?;
        // AUDIT 0.40.2: `fs::copy` copies the source's
        // permissions, so a read-only source would make the
        // copy itself read-only — the destination would be
        // born poisoned even with the 0.34.1 clearing below.
        // Clear the attribute on the temp, before it is
        // published under the real name.
        clear_readonly(&tmp);
        {
            // AUDIT 0.40.2: the sync handle must be opened
            // with WRITE access. On Windows `sync_all` is
            // FlushFileBuffers, which requires a GENERIC_WRITE
            // handle; the read-only `File::open` used here
            // before failed with access denied on Windows
            // every time, so the migration's state copy could
            // never complete there — while passing every test
            // on the Linux build host, where fsync accepts a
            // read-only fd.
            let f = std::fs::OpenOptions::new().write(true).open(&tmp)?;
            f.sync_all()?;
        }
        // AUDIT 0.34.1 (retry poisoning): a previous partial
        // migration can leave the destination state.json
        // READ-ONLY (fs::copy copies the source's permissions);
        // on Windows a rename over a read-only destination
        // fails with access denied, so every retry would fail
        // forever even after the original cause cleared. Clear
        // the attribute first, best-effort. (The Windows
        // read-only rename semantics are reasoned from the
        // platform docs, not verifiable on the Linux build
        // host, where rename ignores the file's mode.)
        clear_readonly(dst);
        std::fs::rename(&tmp, dst)
    })();
    if copied.is_err() {
        // AUDIT 0.40.2: a leftover temp is named exactly like
        // a `save_to` temp, and the loader promotes a complete
        // temp as the newest intent — a failed migration copy
        // must not become the state after the migration
        // report said the copy was withheld. Best-effort
        // cleanup; the copy's own error is the one returned.
        let _ = std::fs::remove_file(&tmp);
    }
    copied
}

/// Best-effort: if `path` exists and is read-only, clear
/// its read-only attribute (0.34.1 retry-poisoning fix).
/// The `set_readonly(false)` call is the point — on
/// Windows it clears exactly the one attribute; the
/// clippy lint against it assumes a unix mode rewrite,
/// which `Permissions::set_readonly` is not.
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

/// Deepest directory nesting `copy_tree` will follow
/// (AUDIT 0.31, A5-L10): a pathological or hostile profile
/// tree (nested junctions, a self-similar backup) must not
/// recurse without bound; entries past the cap are skipped
/// and counted like every other skip.
const COPY_TREE_MAX_DEPTH: usize = 32;

/// Test-only convenience (0.34.1): the performer calls
/// `copy_tree_rooted` with the migration source root; the
/// in-module pins call this wrapper, whose detail paths
/// are relative to the tree root itself.
#[cfg(test)]
fn copy_tree(src: &Path, dst: &Path, report: &mut MigrationReport) {
    copy_tree_at_depth(src, dst, src, report, 0, false)
}

/// The migration performer's entry: detail paths are
/// relative to the migration SOURCE root, so a profile
/// file reads `WebView2/...` in the log (0.34.1).
/// `missing_only` (AUDIT 0.41.3) is the performer's fill
/// discipline: directories are still created, but a file
/// that already exists at the destination is skipped
/// (kind "(exists)"), never rewound.
fn copy_tree_rooted(
    src: &Path,
    dst: &Path,
    root: &Path,
    report: &mut MigrationReport,
    missing_only: bool,
) {
    copy_tree_at_depth(src, dst, root, report, 0, missing_only)
}

fn copy_tree_at_depth(
    src: &Path,
    dst: &Path,
    root: &Path,
    report: &mut MigrationReport,
    depth: usize,
    missing_only: bool,
) {
    // AUDIT 0.31 (A5-L10): depth cap — deeper entries are
    // skipped (and counted), never followed.
    if depth > COPY_TREE_MAX_DEPTH {
        // 0.34.1: the skip is itemized, with its reason.
        report.note_skip("profile dir (depth cap)", migration_rel(root, src));
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dst) {
        // 0.34.1: the error is captured, not discarded.
        report.note_failure("profile dir", migration_rel(root, src), e);
        return;
    }
    let entries = match std::fs::read_dir(src) {
        Ok(entries) => entries,
        Err(e) => {
            report.note_failure("profile dir", migration_rel(root, src), e);
            return;
        }
    };
    for entry in entries {
        // AUDIT V28 (A5-H1): a per-entry read_dir error is
        // counted as skipped — through v27 `flatten()` dropped
        // them silently and the report overstated the copy.
        // (0.34.1: the entry has no name to itemize, so the
        // detail names the directory being read.)
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                report.note_failure("entry", migration_rel(root, src), e);
                continue;
            }
        };
        let (s, d) = (entry.path(), dst.join(entry.file_name()));
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(e) => {
                report.note_failure("entry", migration_rel(root, &s), e);
                continue;
            }
        };
        if kind.is_dir() {
            copy_tree_at_depth(&s, &d, root, report, depth + 1, missing_only);
        } else if kind.is_file() && missing_only && d.exists() {
            // AUDIT 0.41.3: the fill never rewinds a live
            // destination file (see copy_tree_rooted).
            report.note_skip("(exists)", migration_rel(root, &s));
        } else if kind.is_file() {
            match std::fs::copy(&s, &d) {
                // AUDIT 0.40.2: `fs::copy` copies the source's
                // permissions — clear a read-only attribute on
                // the NEW copy, so the portable profile is not
                // born read-only (the 0.34.1 fix below only
                // clears a PRE-EXISTING destination on retry).
                Ok(_) => {
                    clear_readonly(&d);
                    // AUDIT 0.41.1 (Front A F9): the copy is
                    // not done when the OS accepts the bytes —
                    // a verified copy authorizes deleting the
                    // only other one, so the file is synced
                    // before it counts (the A5-8 discipline
                    // copy_atomic already had; the tree copy
                    // never did). A sync failure is a copy
                    // failure.
                    match sync_copied_file(&d) {
                        Ok(()) => report.profile_files += 1,
                        Err(e) => {
                            report.note_failure("profile file", migration_rel(root, &s), e);
                        }
                    }
                }
                Err(first) => {
                    // AUDIT 0.34.1 (retry poisoning): a
                    // partial earlier copy may have left
                    // the destination READ-ONLY (fs::copy
                    // copies the source's permissions); on
                    // Windows the retry then fails with
                    // access denied forever, even after the
                    // original lock clears. If a destination
                    // exists, clear its read-only attribute
                    // and retry ONCE; a successful retry is
                    // a copy, not a failure, and a failed
                    // retry records the SECOND error. (The
                    // Windows semantics are reasoned, not
                    // verifiable on the Linux build host.)
                    if d.exists() {
                        clear_readonly(&d);
                        match std::fs::copy(&s, &d) {
                            // AUDIT 0.40.2: same born-read-only
                            // clearing as the first-copy arm.
                            Ok(_) => {
                                clear_readonly(&d);
                                match sync_copied_file(&d) {
                                    Ok(()) => report.profile_files += 1,
                                    Err(e) => {
                                        report.note_failure(
                                            "profile file",
                                            migration_rel(root, &s),
                                            e,
                                        );
                                    }
                                }
                            }
                            Err(second) => {
                                report.note_failure(
                                    "profile file",
                                    migration_rel(root, &s),
                                    second,
                                );
                            }
                        }
                    } else {
                        report.note_failure("profile file", migration_rel(root, &s), first);
                    }
                }
            }
        } else {
            // AUDIT V29 (A5-8): symlinks and exotic entries
            // are not copied, but they are COUNTED — through
            // v28 they vanished silently and the report (and
            // now the completion marker) overstated the copy.
            // 0.34.1: itemized as pure skips (error None),
            // with the reason in the kind.
            if kind.is_symlink() {
                report.note_skip("profile entry (symlink)", migration_rel(root, &s));
            } else {
                report.note_skip("profile entry (exotic)", migration_rel(root, &s));
            }
        }
    }
}

/// Outcome of loading state, distinguishing "no file yet" from
/// "file existed but was unusable" (the latter is worth a log
/// line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// Boxed (0.30): the savings book grew `PersistState` past
    /// clippy's large-variant threshold; the value is produced
    /// once at startup, so the indirection costs nothing.
    Loaded(Box<PersistState>),
    Missing,
    Corrupt(String),
}

/// Largest state file we will even read (the real thing is
/// < 4 KiB).
pub const MAX_STATE_BYTES: u64 = 1024 * 1024;

pub fn load_from_checked(path: &Path) -> LoadOutcome {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_STATE_BYTES => {
            // AUDIT 0.40.2: this was the one failure branch
            // that did not attempt temp recovery — every other
            // branch (missing, unreadable, unparseable) treats
            // a complete temp sibling as the newest intent
            // (the A5-L2/L3 rationale below), and an oversized
            // main file does not change that.
            if let Some(recovered) = recover_from_temp(path) {
                return LoadOutcome::Loaded(Box::new(recovered));
            }
            return LoadOutcome::Corrupt(format!(
                "state file implausibly large ({} bytes)",
                meta.len()
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // AUDIT V28 (A3-LOW-3): the main file is missing but
            // a save may have died between temp-write and
            // rename. A temp sibling that parses COMPLETELY is
            // the newest intent — recover from it. A torn temp
            // never parses, so this cannot resurrect a partial
            // write.
            if let Some(recovered) = recover_from_temp(path) {
                return LoadOutcome::Loaded(Box::new(recovered));
            }
            return LoadOutcome::Missing;
        }
        Err(e) => {
            // AUDIT 0.31 (A5-L2+L3): a non-NotFound failure
            // (a sharing violation, a transient lock) gets the
            // same temp-recovery attempt — the temp may be the
            // newest complete intent while the main file is
            // merely unreadable right now.
            if let Some(recovered) = recover_from_temp(path) {
                return LoadOutcome::Loaded(Box::new(recovered));
            }
            return LoadOutcome::Corrupt(format!("unreadable: {e}"));
        }
    }
    // AUDIT V29 (A5-5): temps are cleaned only AFTER the main
    // file parses — through v28 the cleanup ran first, so a
    // corrupt main file destroyed its own recovery source
    // before the parse that needed it. A corrupt main now
    // falls back to the same temp recovery the missing case
    // uses: a complete temp is the newest intent either way.
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<PersistState>(&text) {
            Ok(state) => {
                // A good main file makes every temp sibling
                // stale: clean them up best-effort so they
                // cannot accumulate (or be mistaken for a
                // recovery source after a later loss).
                clean_stale_temps(path);
                LoadOutcome::Loaded(Box::new(state.sanitized()))
            }
            Err(e) => match recover_from_temp(path) {
                Some(recovered) => LoadOutcome::Loaded(Box::new(recovered)),
                None => LoadOutcome::Corrupt(format!("invalid JSON: {e}")),
            },
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match recover_from_temp(path) {
            Some(recovered) => LoadOutcome::Loaded(Box::new(recovered)),
            None => LoadOutcome::Missing,
        },
        // AUDIT 0.31 (A5-L2+L3): a non-NotFound READ failure
        // (sharing violation and friends) also attempts temp
        // recovery before declaring the state corrupt.
        Err(e) => match recover_from_temp(path) {
            Some(recovered) => LoadOutcome::Loaded(Box::new(recovered)),
            None => LoadOutcome::Corrupt(format!("unreadable: {e}")),
        },
    }
}

/// Sibling temp files of a state path (`state.json.<pid>.tmp`,
/// plus the pre-v28 fixed `state.json.tmp` spelling).
fn temp_siblings(path: &Path) -> Vec<PathBuf> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let name = name.to_string_lossy();
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let en = entry.file_name();
            let en = en.to_string_lossy();
            if en.starts_with(name.as_ref()) && en.ends_with(".tmp") {
                out.push(entry.path());
            }
        }
    }
    out
}

fn recover_from_temp(path: &Path) -> Option<PersistState> {
    // AUDIT V29 (A5-5): newest first. Through v28 the first
    // parseable temp in read_dir order won — with temps from
    // several crashed runs that could resurrect the OLDEST
    // intent (stale recents, a dead theme) over the newest.
    let mut temps = temp_siblings(path);
    temps.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    for tmp in temps.into_iter().rev() {
        // AUDIT 0.31 (A5-L2+L3): check the temp's metadata
        // length against MAX_STATE_BYTES BEFORE reading it —
        // the main path has always gated on metadata; the
        // recovery path read the whole file into memory first
        // and only then compared lengths, so an oversized temp
        // cost exactly the allocation the cap exists to stop.
        match std::fs::metadata(&tmp) {
            Ok(meta) if meta.len() > MAX_STATE_BYTES => continue,
            Ok(_) => {}
            Err(_) => continue,
        }
        if let Ok(text) = std::fs::read_to_string(&tmp) {
            if let Ok(state) = serde_json::from_str::<PersistState>(&text) {
                return Some(state.sanitized());
            }
        }
    }
    None
}

fn clean_stale_temps(path: &Path) {
    for tmp in temp_siblings(path) {
        // AUDIT V29 (A5-2): never delete a FRESH temp. Temp
        // names are per-process, and Pico has no
        // single-instance guard: a second live instance's
        // in-flight save temp (written, not yet renamed) is
        // indistinguishable from a crash leftover by name —
        // but not by age. A save completes in milliseconds;
        // anything under a minute old is treated as live and
        // left alone. Crash leftovers age past the guard and
        // are reaped on a later start.
        let fresh = std::fs::metadata(&tmp)
            .and_then(|m| m.modified())
            .map(|t| {
                t.elapsed()
                    .map(|age| age < std::time::Duration::from_secs(60))
                    .unwrap_or(true)
            })
            .unwrap_or(false);
        if !fresh {
            let _ = std::fs::remove_file(tmp);
        }
    }
}

/// Write state as JSON (via a temp file + rename, so a crash
/// mid-write cannot corrupt the previous state).
///
/// AUDIT V28 (A5-M1 + A3-LOW-3): the temp name is per-process
/// (`state.json.<pid>.tmp`) — the old fixed name was shared by
/// every Pico process, and with no single-instance guard two
/// first-run instances could tear each other's writes — and
/// the temp is fsynced before the rename, so the rename never
/// publishes bytes the disk has not taken.
pub fn save_to(path: &Path, state: &PersistState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state.json".to_string());
    let tmp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    // AUDIT 0.40.2: `save_to` was the one writer the 0.34.1
    // retry-poisoning fix missed — on Windows the rename
    // fails with access denied when the destination is
    // read-only, and this writer runs at every state change,
    // so one read-only state.json (e.g. born that way from a
    // migration copy) would fail every save until a human
    // cleared the attribute by hand.
    clear_readonly(path);
    std::fs::rename(&tmp, path)
}

pub fn save(state: &PersistState) -> std::io::Result<()> {
    save_to(&state_path(), state)
}

/// Add a path to the front of the recents list (deduped,
/// capped). Unsafe path strings (UNC/device namespaces,
/// control characters, absurd lengths) are refused rather
/// than persisted.
pub fn push_recent(state: &mut PersistState, path: &str) {
    if path.len() > 4096
        || path.chars().any(|c: char| c.is_control())
        || !crate::core::addr::is_safe_local_path(Path::new(path))
    {
        return;
    }
    state.recents.retain(|r| r != path);
    state.recents.insert(0, path.to_string());
    state.recents.truncate(MAX_RECENTS);
}

// ---------------------------------------------------------------------------
// AUDIT 0.31 (A5-L2+L3, A5-L10): recovery + copy-depth pins.
// (The broader state suite lives in tests/history_state.rs;
// these live inline because they pin this file's recovery
// internals.)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod audit_031_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pico-031-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("mkdir: {e}"));
        dir
    }

    const VALID: &str = r#"{"version":3,"theme":"dark","recents":[]}"#;

    #[test]
    fn temp_recovery_attempted_on_non_notfound_read_failure() {
        // A directory standing at the state path makes the
        // main READ fail with a non-NotFound error (IsADirectory
        // on unix, AccessDenied on Windows) while metadata
        // succeeds — the exact A5-L3 shape. A complete temp
        // sibling must still be recovered.
        let dir = scratch("nonnotfound");
        let path = dir.join("state.json");
        std::fs::create_dir_all(&path).unwrap_or_else(|e| panic!("mkdir state: {e}"));
        std::fs::write(dir.join("state.json.4242.tmp"), VALID)
            .unwrap_or_else(|e| panic!("write temp: {e}"));
        match load_from_checked(&path) {
            LoadOutcome::Loaded(s) => assert_eq!(s.theme, "dark"),
            other => panic!("expected temp recovery, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_temp_is_not_recovered() {
        // A5-L2: the temp's metadata length is gated BEFORE
        // the read — a temp padded past MAX_STATE_BYTES (with
        // JSON that would otherwise parse: valid value +
        // trailing whitespace) is never recovered.
        let dir = scratch("oversized");
        let path = dir.join("state.json");
        let mut big = VALID.to_string();
        big.push_str(&" ".repeat((MAX_STATE_BYTES as usize) + 16));
        std::fs::write(dir.join("state.json.4242.tmp"), big)
            .unwrap_or_else(|e| panic!("write temp: {e}"));
        assert_eq!(load_from_checked(&path), LoadOutcome::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_tree_depth_cap_skips_deep_entries() {
        // A5-L10: nesting past the cap is skipped (counted),
        // shallow files still copy.
        let dir = scratch("copydepth");
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::create_dir_all(&src).unwrap_or_else(|e| panic!("mkdir src: {e}"));
        std::fs::write(src.join("top.txt"), "top").unwrap_or_else(|e| panic!("write top: {e}"));
        let mut deep = src.clone();
        for _ in 0..40 {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).unwrap_or_else(|e| panic!("mkdir deep: {e}"));
        std::fs::write(deep.join("bottom.txt"), "bottom")
            .unwrap_or_else(|e| panic!("write bottom: {e}"));
        let mut report = MigrationReport::default();
        copy_tree(&src, &dst, &mut report);
        assert!(dst.join("top.txt").is_file(), "shallow file copies");
        assert!(report.skipped > 0, "deep entries counted as skipped");
        let mut copied_deep = dst.clone();
        for _ in 0..40 {
            copied_deep = copied_deep.join("d");
        }
        assert!(
            !copied_deep.join("bottom.txt").exists(),
            "the file past the depth cap must not be copied"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn copy_tree_skips_symlink_dirs() {
        // A5-L10 companion: a symlinked directory is an exotic
        // entry — counted as skipped, never followed.
        let dir = scratch("copysymlink");
        let src = dir.join("src");
        let dst = dir.join("dst");
        let real = dir.join("real");
        std::fs::create_dir_all(&src).unwrap_or_else(|e| panic!("mkdir src: {e}"));
        std::fs::create_dir_all(&real).unwrap_or_else(|e| panic!("mkdir real: {e}"));
        std::fs::write(real.join("x.txt"), "x").unwrap_or_else(|e| panic!("write: {e}"));
        std::os::unix::fs::symlink(&real, src.join("link"))
            .unwrap_or_else(|e| panic!("symlink: {e}"));
        let mut report = MigrationReport::default();
        copy_tree(&src, &dst, &mut report);
        assert!(report.skipped >= 1, "symlink counted as skipped");
        assert!(!dst.join("link").join("x.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------
    // AUDIT 0.34.1 (field addition): per-item migration details.
    // -----------------------------------------------------------------

    #[test]
    fn copy_tree_details_name_a_failed_file() {
        // A directory squatting the destination makes the
        // file copy fail portably (the v29 test's shape, at
        // copy_tree level): the failure is itemized with the
        // file's root-relative path and the captured error,
        // and the counters stay consistent with the details
        // (one failure = failed 1, skipped 1, one detail).
        let dir = scratch("detailfail");
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::create_dir_all(src.join("sub")).unwrap_or_else(|e| panic!("mkdir src: {e}"));
        std::fs::write(src.join("sub").join("Cookies"), b"cookies")
            .unwrap_or_else(|e| panic!("write: {e}"));
        std::fs::write(src.join("ok.txt"), b"ok").unwrap_or_else(|e| panic!("write: {e}"));
        std::fs::create_dir_all(dst.join("sub").join("Cookies"))
            .unwrap_or_else(|e| panic!("squat: {e}"));
        let mut report = MigrationReport::default();
        copy_tree(&src, &dst, &mut report);
        assert_eq!(report.profile_files, 1, "the clean file still copies");
        assert_eq!(report.failed, 1, "{report:?}");
        assert_eq!(report.skipped, 1, "{report:?}");
        assert_eq!(report.details.len(), 1, "{report:?}");
        let detail = &report.details[0];
        assert_eq!(detail.kind, "profile file");
        assert_eq!(detail.path, "sub/Cookies");
        assert!(
            detail.error.as_deref().is_some_and(|e| !e.is_empty()),
            "the failure detail carries the captured error: {detail:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn copy_tree_details_name_a_symlink_skip_without_an_error() {
        // A pure skip is itemized with error None (it is
        // deliberate, not a failure) and never moves the
        // failed counter.
        let dir = scratch("detailskip");
        let src = dir.join("src");
        let dst = dir.join("dst");
        let real = dir.join("real");
        std::fs::create_dir_all(&src).unwrap_or_else(|e| panic!("mkdir src: {e}"));
        std::fs::create_dir_all(&real).unwrap_or_else(|e| panic!("mkdir real: {e}"));
        std::fs::write(real.join("x.txt"), "x").unwrap_or_else(|e| panic!("write: {e}"));
        std::os::unix::fs::symlink(&real, src.join("link"))
            .unwrap_or_else(|e| panic!("symlink: {e}"));
        let mut report = MigrationReport::default();
        copy_tree(&src, &dst, &mut report);
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.skipped, 1, "{report:?}");
        assert_eq!(report.details.len(), 1, "{report:?}");
        let detail = &report.details[0];
        assert!(
            detail.kind.contains("symlink"),
            "the skip detail names its reason: {detail:?}"
        );
        assert_eq!(detail.path, "link");
        assert_eq!(detail.error, None, "a deliberate skip has no error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_tree_details_name_the_depth_cap_skip() {
        // Past the depth cap, the skip detail names the
        // capped directory with the reason in its kind and
        // no error — and it stays a pure skip (failed 0 for
        // the capped entry itself).
        let dir = scratch("detaildepth");
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::create_dir_all(&src).unwrap_or_else(|e| panic!("mkdir src: {e}"));
        let mut deep = src.clone();
        for _ in 0..40 {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).unwrap_or_else(|e| panic!("mkdir deep: {e}"));
        let mut report = MigrationReport::default();
        copy_tree(&src, &dst, &mut report);
        assert_eq!(report.failed, 0, "{report:?}");
        let cap_details: Vec<&MigrationDetail> = report
            .details
            .iter()
            .filter(|d| d.kind.contains("depth cap"))
            .collect();
        assert_eq!(cap_details.len(), 1, "{report:?}");
        assert_eq!(cap_details[0].error, None);
        assert!(
            report.details.len() == report.skipped,
            "every counted item is itemized exactly once: {report:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_tree_retries_once_over_a_readonly_destination() {
        // AUDIT 0.34.1 (retry poisoning): a destination a
        // partial earlier copy left read-only must not doom
        // every retry. The stale read-only file is replaced
        // by the fresh copy and counts as a copy, not a
        // failure. (On hosts where the first copy already
        // succeeds — e.g. a privileged test runner — the
        // same outcome assertions hold trivially.)
        let dir = scratch("retrypoison");
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::create_dir_all(&src).unwrap_or_else(|e| panic!("mkdir src: {e}"));
        std::fs::create_dir_all(&dst).unwrap_or_else(|e| panic!("mkdir dst: {e}"));
        std::fs::write(src.join("f.txt"), b"fresh").unwrap_or_else(|e| panic!("write src: {e}"));
        let stale = dst.join("f.txt");
        std::fs::write(&stale, b"stale").unwrap_or_else(|e| panic!("write dst: {e}"));
        let mut perms = std::fs::metadata(&stale).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&stale, perms).unwrap_or_else(|e| panic!("readonly: {e}"));
        let mut report = MigrationReport::default();
        copy_tree(&src, &dst, &mut report);
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(report.profile_files, 1, "{report:?}");
        assert!(report.details.is_empty(), "{report:?}");
        assert_eq!(
            std::fs::read(&stale).unwrap_or_else(|e| panic!("read back: {e}")),
            b"fresh",
            "the fresh bytes replaced the stale read-only file"
        );
        // Leave the tree removable on every platform.
        clear_readonly(&stale);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
