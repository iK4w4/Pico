//! Persisted app state: theme + recent files + window geometry
//! + the Shield's flag and lifetime counter.
//!
//! Stored as JSON in Pico's data directory. THROUGH v26 that
//! was `%APPDATA%\Pico` and nothing was ever written next to
//! the executable; **v27 reverses that by the user's explicit
//! order** (AUDIT V27-C): the data directory is now
//! `<exe dir>/data/` — state, log, and the WebView2 profile
//! all live beside `Pico.exe`, so deleting that one folder is
//! a factory reset and deleting the whole folder erases Pico
//! completely. `%APPDATA%\Pico` remains the fallback when the
//! exe's folder is not writable (and the one-time migration
//! source on the first v27 run). The choice is made ONCE by
//! [`resolve_data_dir`] over probed facts, cached, and logged
//! at startup (`data: portable …` / `data: fallback appdata
//! (…)`).
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
}

fn default_theme() -> String {
    "light".to_string()
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
        }
    }
}

impl PersistState {
    pub fn is_dark(&self) -> bool {
        self.theme == "dark"
    }

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
        self
    }
}

/// The pre-v27 per-user Pico data directory (`%APPDATA%\Pico`
/// on Windows, `$HOME/.config/Pico` elsewhere). In v27 this is
/// the FALLBACK location (exe folder not writable) and the
/// one-time migration source — never the first choice.
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

/// Where Pico's runtime data lives this run, and why
/// (AUDIT V27-C). Resolved ONCE from probed facts by
/// [`resolve_data_dir`], then cached by [`data_dir`]; the
/// reason string is what the startup log prints after
/// `data: portable …` / `data: fallback appdata (…)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDirChoice {
    pub dir: PathBuf,
    pub portable: bool,
    pub reason: String,
}

/// The pure chooser. Facts in, decision out — no I/O, so the
/// whole matrix is unit-tested:
///
/// - Exe folder writable → `<exe dir>/data`, portable. (A
///   filesystem-root exe dir is fine: `C:\data` is a normal
///   folder. A `data` path that exists as a FILE fails the
///   caller's probe, arriving here as `writable = false`.)
/// - Exe folder not writable → the pre-v27 appdata dir, with
///   the reason carried through for the log.
pub fn resolve_data_dir(exe_dir: &Path, appdata_dir: &Path, exe_writable: bool) -> DataDirChoice {
    if exe_writable {
        DataDirChoice {
            dir: exe_dir.join("data"),
            portable: true,
            reason: "exe folder is writable".to_string(),
        }
    } else {
        DataDirChoice {
            dir: appdata_dir.to_path_buf(),
            portable: false,
            reason: "exe folder is not writable".to_string(),
        }
    }
}

/// Probe: can we actually write beside the exe? Creates
/// `<exe dir>/data` and round-trips a temp file inside it.
/// Any failure — including `data` existing as a regular file —
/// is simply "not writable"; Pico then falls back, it never
/// runs data-less and never crashes over a path. (AUDIT V28,
/// A5-L4: `pub` so the data-as-file fallback is pinned by a
/// real tempdir test, not just asserted in prose. The
/// probe-then-use TOCTOU window is accepted: a probe that
/// passes and a dir that later vanishes degrades to failed
/// state saves, never to a crash.)
pub fn probe_exe_dir(exe_dir: &Path) -> bool {
    let data = exe_dir.join("data");
    if std::fs::create_dir_all(&data).is_err() {
        return false;
    }
    // AUDIT V29 (A5-7): the round-trip includes a RENAME.
    // Through v28 the probe proved only write+delete, but the
    // consumers need more: every state save is temp+rename and
    // startup creates the profile dir — a filesystem that
    // writes but won't rename passed the probe and then failed
    // every save for the whole session with the fallback never
    // reconsidered.
    let probe = data.join(format!(".pico-probe-{}", std::process::id()));
    let probe2 = data.join(format!(".pico-probe-{}-renamed", std::process::id()));
    let ok = std::fs::write(&probe, b"pico").is_ok() && std::fs::rename(&probe, &probe2).is_ok();
    let _ = std::fs::remove_file(&probe);
    let _ = std::fs::remove_file(&probe2);
    ok
}

/// The process's executable directory, or the current
/// directory if even that is unknowable (the fallback chain
/// then decides).
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

static DATA_DIR: std::sync::OnceLock<DataDirChoice> = std::sync::OnceLock::new();

/// The chosen data directory + reason, resolved on first use
/// (probe included) and cached for the process lifetime —
/// every path in the app derives from this one answer, so the
/// state, the log and the profile can never split across two
/// locations mid-run.
pub fn data_dir_choice() -> &'static DataDirChoice {
    DATA_DIR.get_or_init(|| {
        let exe = exe_dir();
        let writable = probe_exe_dir(&exe);
        resolve_data_dir(&exe, &appdata_pico_dir(), writable)
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
/// V22-16, moved by V27-C — the MIGRATION below is what
/// carries an existing profile across the move, and nothing
/// in Pico ever deletes it). (The state sanitizer above cannot
/// touch it either — it operates on the in-memory
/// `PersistState` value.)
pub fn webview_data_dir() -> PathBuf {
    pico_dir().join("WebView2")
}

/// The appdata install's WebView2 profile folder (the
/// migration SOURCE; never a context folder in v27).
pub fn appdata_webview_dir() -> PathBuf {
    appdata_pico_dir().join("WebView2")
}

/// What the first v27 run should do about pre-v27 data
/// (AUDIT V27-C3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationPlan {
    /// Nothing to migrate (no old data, or the portable dir
    /// already has state of its own — never clobber).
    None,
    /// Copy state + profile from the appdata dir into the
    /// portable dir.
    Copy { from: PathBuf, to: PathBuf },
    /// Copy ONLY the state file: the portable dir already has
    /// a profile of its own, which is never overwritten
    /// (AUDIT V28, A5-H1).
    CopyStateOnly { from: PathBuf, to: PathBuf },
}

/// The pure migration planner. Migration is considered ONLY
/// on the portable route (a fallback run reads appdata in
/// place — copying it onto itself would be nonsense), only
/// when the portable dir has no state yet, and only when the
/// appdata dir actually holds something worth carrying (a
/// state file OR a profile folder).
///
/// AUDIT V28 (A5-H1): the planner now also knows whether the
/// portable dir already has a PROFILE. Through v27 it keyed on
/// state alone, with two failure shapes: a profile-only
/// portable dir (state lost, profile present) got its live
/// profile CLOBBERED by the stale appdata copy; and because
/// the performer copied state FIRST, an interrupted migration
/// left state behind — the planner then stood down forever
/// and the profile copy was never resumed, while the log
/// could still claim a clean migration. The performer now
/// copies the profile first and the state last (atomically),
/// so state presence remains an honest completion marker,
/// and a profile-only portable dir plans a state-only copy.
/// Name of the migration-in-progress marker inside the
/// portable data dir (AUDIT V29, A5-1): written by the
/// performer before a full copy and removed only when the
/// copy completes cleanly. Its presence is how the planner
/// tells a PARTIAL portable profile (resume the copy — the
/// appdata source is its own origin) from a LIVE one whose
/// state was lost (never recopy — the v28 clobber law).
pub const MIGRATION_PENDING_NAME: &str = "migration.pending";

pub fn plan_migration(
    choice: &DataDirChoice,
    appdata_dir: &Path,
    portable_has_state: bool,
    portable_has_profile: bool,
    appdata_has_state: bool,
    appdata_has_profile: bool,
) -> MigrationPlan {
    if !choice.portable || portable_has_state || !(appdata_has_state || appdata_has_profile) {
        return MigrationPlan::None;
    }
    // A leftover pending marker means the portable profile is
    // a partial copy of appdata's: resume the FULL copy
    // (overwrite-idempotent), never the state-only shortcut.
    if choice.dir.join(MIGRATION_PENDING_NAME).is_file() {
        return MigrationPlan::Copy {
            from: appdata_dir.to_path_buf(),
            to: choice.dir.clone(),
        };
    }
    if portable_has_profile {
        return if appdata_has_state {
            MigrationPlan::CopyStateOnly {
                from: appdata_dir.to_path_buf(),
                to: choice.dir.clone(),
            }
        } else {
            MigrationPlan::None
        };
    }
    MigrationPlan::Copy {
        from: appdata_dir.to_path_buf(),
        to: choice.dir.clone(),
    }
}

/// Outcome of a performed migration, for the startup log.
/// `skipped` counts everything not copied; `failed` counts
/// only entries that SHOULD have copied and could not (AUDIT
/// V29, A5-1/A5-8: the completion-marker gate keys on
/// failures — a deliberately-uncopied symlink is honest in
/// `skipped` but must not stall migration forever).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MigrationReport {
    pub state_copied: bool,
    pub profile_files: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Perform a planned migration. Best-effort per file — a file
/// that cannot be read or written (locked, vanished mid-copy)
/// is counted as skipped, never fatal; the ORIGINAL appdata
/// tree is left untouched as the backup. Must run BEFORE any
/// WebView2 context is created against the new profile (the
/// app calls it during startup, before window construction).
///
/// AUDIT V28 (A5-H1/M1): for a full copy the PROFILE tree is
/// copied first and `state.json` LAST, atomically (pid-unique
/// temp + rename) — state presence is the planner's
/// completion marker, so it may only appear once everything
/// else has had its chance; an interrupted run is re-planned
/// next start instead of standing down forever.
pub fn perform_migration(plan: &MigrationPlan) -> MigrationReport {
    let (from, to, with_profile) = match plan {
        MigrationPlan::Copy { from, to } => (from, to, true),
        MigrationPlan::CopyStateOnly { from, to } => (from, to, false),
        MigrationPlan::None => return MigrationReport::default(),
    };
    let mut report = MigrationReport::default();
    let _ = std::fs::create_dir_all(to);
    // AUDIT V29 (A5-1): mark the copy in progress BEFORE the
    // profile lands; the marker is removed only at clean
    // completion (below). A crash or a failed file leaves it
    // behind, and the planner (see plan_migration) resumes
    // the full copy instead of mistaking the partial profile
    // for a live one.
    let pending = to.join(MIGRATION_PENDING_NAME);
    if with_profile {
        let _ = std::fs::write(&pending, b"pico migration in progress\n");
        let src_profile = from.join("WebView2");
        if src_profile.is_dir() {
            copy_tree(&src_profile, &to.join("WebView2"), &mut report);
        }
    }
    // AUDIT V29 (A5-1): state is the planner's completion
    // marker, so it may only land when the profile phase had
    // no FAILURES. Through v28 any per-file failure (a locked
    // Cookies file, an AV hold) still landed state — the
    // planner then stood down forever with a half-copied
    // profile and a success-shaped log line. Now a failed
    // profile copy leaves the marker absent: the plan is
    // re-issued next start, and copy_tree is
    // overwrite-idempotent, so the retry simply resumes.
    if with_profile && report.failed > 0 {
        return report;
    }
    let src_state = from.join("state.json");
    if src_state.is_file() {
        match copy_atomic(&src_state, &to.join("state.json")) {
            Ok(()) => report.state_copied = true,
            Err(_) => {
                report.skipped += 1;
                report.failed += 1;
            }
        }
    }
    // Clean completion retires the pending marker. (The
    // failure path above RETURNS EARLY and never reaches
    // this: the marker — and the withheld state — are what
    // make the next start resume honestly.)
    if with_profile && report.failed == 0 {
        let _ = std::fs::remove_file(&pending);
    }
    report
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
    std::fs::copy(src, &tmp)?;
    {
        let f = std::fs::File::open(&tmp)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dst)
}

/// Deepest directory nesting `copy_tree` will follow
/// (AUDIT 0.31, A5-L10): a pathological or hostile profile
/// tree (nested junctions, a self-similar backup) must not
/// recurse without bound; entries past the cap are skipped
/// and counted like every other skip.
const COPY_TREE_MAX_DEPTH: usize = 32;

fn copy_tree(src: &Path, dst: &Path, report: &mut MigrationReport) {
    copy_tree_at_depth(src, dst, report, 0)
}

fn copy_tree_at_depth(src: &Path, dst: &Path, report: &mut MigrationReport, depth: usize) {
    // AUDIT 0.31 (A5-L10): depth cap — deeper entries are
    // skipped (and counted), never followed.
    if depth > COPY_TREE_MAX_DEPTH {
        report.skipped += 1;
        return;
    }
    if std::fs::create_dir_all(dst).is_err() {
        report.skipped += 1;
        report.failed += 1;
        return;
    }
    let Ok(entries) = std::fs::read_dir(src) else {
        report.skipped += 1;
        report.failed += 1;
        return;
    };
    for entry in entries {
        // AUDIT V28 (A5-H1): a per-entry read_dir error is
        // counted as skipped — through v27 `flatten()` dropped
        // them silently and the report overstated the copy.
        let Ok(entry) = entry else {
            report.skipped += 1;
            report.failed += 1;
            continue;
        };
        let (s, d) = (entry.path(), dst.join(entry.file_name()));
        let Ok(kind) = entry.file_type() else {
            report.skipped += 1;
            report.failed += 1;
            continue;
        };
        if kind.is_dir() {
            copy_tree_at_depth(&s, &d, report, depth + 1);
        } else if kind.is_file() {
            match std::fs::copy(&s, &d) {
                Ok(_) => report.profile_files += 1,
                Err(_) => {
                    report.skipped += 1;
                    report.failed += 1;
                }
            }
        } else {
            // AUDIT V29 (A5-8): symlinks and exotic entries
            // are not copied, but they are COUNTED — through
            // v28 they vanished silently and the report (and
            // now the completion marker) overstated the copy.
            report.skipped += 1;
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
}
