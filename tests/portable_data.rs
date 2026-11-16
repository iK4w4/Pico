//! AppData data home + theme persistence (0.41.0).
//!
//! 0.41.0 moves ALL runtime data to `%LOCALAPPDATA%\Pico\`
//! (the v27 beside-exe design is reversed by the user's
//! order): the resolver chain (known folder → LOCALAPPDATA
//! → APPDATA-derived Local → home) and the migration
//! planner are pure with their facts injected; the copier
//! and the full legacy migration (copy → verify → remove)
//! are exercised against fixture trees in temp dirs —
//! including the profile that carries the user's logins.

use pico_core::core::state::{
    load_from_checked, plan_migration, resolve_data_dir, save_to, LoadOutcome, MigrationPlan,
    PersistState,
};
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pico-v41-test-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

// ---------------------------------------------------------------------------
// Resolver — the AppData chain (0.41.0)
// ---------------------------------------------------------------------------

#[test]
fn resolver_prefers_the_known_folder() {
    let c = resolve_data_dir(
        Some(Path::new("C:/Users/x/AppData/Local")),
        Some(Path::new("D:/Other/Local")),
        Some(Path::new("C:/Users/x/AppData/Roaming")),
        Some(Path::new("/home/x")),
    );
    assert_eq!(c.dir, Path::new("C:/Users/x/AppData/Local/Pico"));
    assert!(c.reason.contains("FOLDERID_LocalAppData"), "{}", c.reason);
}

#[test]
fn resolver_falls_back_to_the_localappdata_env() {
    let c = resolve_data_dir(
        None,
        Some(Path::new("C:/Users/x/AppData/Local")),
        Some(Path::new("C:/Users/x/AppData/Roaming")),
        None,
    );
    assert_eq!(c.dir, Path::new("C:/Users/x/AppData/Local/Pico"));
    assert!(c.reason.contains("LOCALAPPDATA"), "{}", c.reason);
    // An EMPTY known-folder value is no value at all.
    let c = resolve_data_dir(Some(Path::new("")), Some(Path::new("C:/L")), None, None);
    assert_eq!(c.dir, Path::new("C:/L/Pico"));
}

#[test]
fn resolver_derives_local_from_a_roaming_appdata() {
    // The standard profile layout: Roaming and Local are
    // siblings under AppData.
    let c = resolve_data_dir(
        None,
        None,
        Some(Path::new("C:/Users/x/AppData/Roaming")),
        None,
    );
    assert_eq!(c.dir, Path::new("C:/Users/x/AppData/Local/Pico"));
    // A non-standard APPDATA (no Roaming tail) is used
    // as-is — still per-user AppData, never beside the exe.
    let c = resolve_data_dir(None, None, Some(Path::new("D:/Profile/AppData")), None);
    assert_eq!(c.dir, Path::new("D:/Profile/AppData/Pico"));
}

#[test]
fn resolver_home_and_last_resort_fallbacks() {
    let c = resolve_data_dir(None, None, None, Some(Path::new("/home/x")));
    assert_eq!(c.dir, Path::new("/home/x/.local/share/Pico"));
    let c = resolve_data_dir(None, None, None, None);
    assert_eq!(c.dir, Path::new("Pico"));
    // Unicode and spaces are just path bytes to the join.
    let c = resolve_data_dir(
        Some(Path::new("C:/Users/x y/AppData/Local نسخه")),
        None,
        None,
        None,
    );
    assert_eq!(c.dir, Path::new("C:/Users/x y/AppData/Local نسخه/Pico"));
}

#[test]
fn downloads_dir_from_prefers_the_known_folder() {
    use pico_core::core::state::downloads_dir_from;
    assert_eq!(
        downloads_dir_from(
            Some(Path::new("C:/Users/x/Downloads")),
            Some(Path::new("C:/Users/x"))
        ),
        Some(PathBuf::from("C:/Users/x/Downloads")),
        "FOLDERID_Downloads wins"
    );
    assert_eq!(
        downloads_dir_from(None, Some(Path::new("C:/Users/x"))),
        Some(PathBuf::from("C:/Users/x/Downloads")),
        "%USERPROFILE%\\Downloads is the fallback"
    );
    assert_eq!(downloads_dir_from(None, None), None);
}

// ---------------------------------------------------------------------------
// Migration planner
// ---------------------------------------------------------------------------

#[test]
fn planner_copies_only_into_a_fresh_home() {
    let dest = Path::new("C:/Users/x/AppData/Local/Pico");
    let source = Path::new("E:/Pico/data");
    // Fresh home + legacy state → copy.
    assert!(matches!(
        plan_migration(dest, source, false, false, true, false, false),
        MigrationPlan::Copy { .. }
    ));
    // Fresh home + legacy profile only → copy.
    assert!(matches!(
        plan_migration(dest, source, false, false, false, true, false),
        MigrationPlan::Copy { .. }
    ));
    // Home state already exists → NEVER clobber.
    assert_eq!(
        plan_migration(dest, source, true, true, true, true, false),
        MigrationPlan::None
    );
    // Nothing in the legacy tree → nothing to do.
    assert_eq!(
        plan_migration(dest, source, false, false, false, false, false),
        MigrationPlan::None
    );
    // A leftover pending marker resumes the FULL copy even
    // when the home already has a profile (the partial-copy
    // law).
    assert!(matches!(
        plan_migration(dest, source, false, true, true, true, true),
        MigrationPlan::Copy { .. }
    ));
}

// ---------------------------------------------------------------------------
// Migration copier — fixture trees
// ---------------------------------------------------------------------------

fn seed_appdata(root: &Path) -> PathBuf {
    let appdata = root.join("appdata").join("Pico");
    let profile = appdata.join("WebView2").join("EBWebView").join("Default");
    fs::create_dir_all(&profile).expect("seed profile");
    fs::write(appdata.join("state.json"), br#"{"theme":"dark"}"#).expect("seed state");
    fs::write(profile.join("Cookies"), b"login-cookies").expect("seed cookies");
    fs::write(profile.join("History"), b"history").expect("seed history");
    // 0.41.0: the downloads task state rides the same tree
    // (the whole legacy home migrates, not just the profile),
    // and the legacy log is carried separately.
    let tasks = appdata.join("downloads");
    fs::create_dir_all(&tasks).expect("seed tasks");
    fs::write(tasks.join("7.json"), br#"{"id":7}"#).expect("seed task");
    fs::write(appdata.join("pico.log"), b"[1] old session\n").expect("seed log");
    appdata
}

#[test]
fn migration_copies_state_and_profile_and_keeps_the_original() {
    let root = tmp("full-copy");
    let appdata = seed_appdata(&root);
    let portable = root.join("exe").join("data");
    let plan = MigrationPlan::Copy {
        from: appdata.clone(),
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert!(report.state_copied);
    assert_eq!(report.profile_files, 3, "{report:?}");
    assert_eq!(report.skipped, 0);
    // The downloads task state carried over too.
    assert_eq!(
        fs::read(portable.join("downloads").join("7.json")).expect("task copied"),
        br#"{"id":7}"#
    );
    // The log is NOT tree-copied (it is carried separately
    // by the orchestrator, append-shaped).
    assert!(!portable.join("pico.log").exists());
    // The login-carrying profile arrived intact.
    let cookies = portable
        .join("WebView2")
        .join("EBWebView")
        .join("Default")
        .join("Cookies");
    assert_eq!(
        fs::read(&cookies).expect("cookies copied"),
        b"login-cookies"
    );
    assert_eq!(
        fs::read(portable.join("state.json")).expect("state copied"),
        br#"{"theme":"dark"}"#
    );
    // The original is untouched — it stays as the backup.
    assert!(appdata.join("state.json").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_tolerates_missing_pieces_and_is_idempotent() {
    let root = tmp("partial");
    let appdata = root.join("appdata").join("Pico");
    fs::create_dir_all(&appdata).expect("appdata dir");
    // State exists but the profile copy source is absent
    // mid-plan (raced deletion): the copy must not fail.
    fs::write(appdata.join("state.json"), b"{}").expect("seed state");
    let portable = root.join("exe").join("data");
    let plan = MigrationPlan::Copy {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert!(report.state_copied);
    assert_eq!(report.profile_files, 0);
    // Second run: the planner now sees portable state and
    // stands down — no re-copy, no clobber.
    let again = plan_migration(
        &portable,
        &root.join("appdata"),
        portable.join("state.json").is_file(),
        false,
        true,
        true,
        false,
    );
    assert_eq!(again, MigrationPlan::None);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn v29_migration_with_a_failed_profile_file_withholds_the_state_marker() {
    // A5-1: a profile file that cannot copy (here: its
    // destination is squatted by a directory) must NOT let
    // state.json land — state is the planner's completion
    // marker, and landing it would stand the planner down
    // forever over a half-copied profile. With the obstacle
    // removed, the re-performed migration completes.
    let root = tmp("v29-mig-fail");
    let appdata = seed_appdata(&root);
    let portable = root.join("exe").join("data");
    let squat = portable
        .join("WebView2")
        .join("EBWebView")
        .join("Default")
        .join("Cookies");
    fs::create_dir_all(&squat).expect("squat the Cookies destination");
    let plan = MigrationPlan::Copy {
        from: appdata.clone(),
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert!(report.failed > 0, "the squatted copy must fail: {report:?}");
    assert!(
        !report.state_copied,
        "state marker must be withheld on failure"
    );
    assert!(
        !portable.join("state.json").is_file(),
        "no state.json may land"
    );
    // The planner therefore re-issues the full copy.
    let replanned = plan_migration(
        &portable,
        &appdata,
        false,
        true,
        true,
        true,
        portable
            .join(pico_core::core::state::MIGRATION_PENDING_NAME)
            .is_file(),
    );
    assert!(
        matches!(replanned, MigrationPlan::Copy { .. }),
        "got {replanned:?}"
    );
    // Obstacle gone: the retry completes and lands the marker.
    fs::remove_dir_all(&squat).expect("unsquat");
    let report2 = pico_core::core::state::perform_migration(&plan);
    assert_eq!(report2.failed, 0);
    assert!(report2.state_copied);
    assert!(portable.join("state.json").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_details_name_the_failed_profile_file() {
    // AUDIT 0.34.1 (field addition): the 0.34 field log
    // could only say "1 failed, 1 skipped" — the report now
    // itemizes the failure with its source-root-relative
    // path and the captured error, and the counters stay
    // consistent with the details. The failure shape is
    // the v29 one: a directory squats the Cookies
    // destination.
    let root = tmp("mig-details");
    let appdata = seed_appdata(&root);
    let portable = root.join("exe").join("data");
    let squat = portable
        .join("WebView2")
        .join("EBWebView")
        .join("Default")
        .join("Cookies");
    fs::create_dir_all(&squat).expect("squat the Cookies destination");
    let plan = MigrationPlan::Copy {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(report.skipped, 1, "{report:?}");
    assert_eq!(
        report.profile_files, 2,
        "History and the task file still copy: {report:?}"
    );
    assert!(!report.state_copied, "state marker withheld on failure");
    assert_eq!(report.details.len(), 1, "{report:?}");
    let detail = &report.details[0];
    assert_eq!(detail.kind, "profile file");
    assert_eq!(detail.path, "WebView2/EBWebView/Default/Cookies");
    assert!(
        detail.error.as_deref().is_some_and(|e| !e.is_empty()),
        "the detail carries the captured io error: {detail:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn migration_details_record_a_symlink_as_a_pure_skip() {
    // A symlink inside the profile tree is a deliberate
    // skip: itemized with error None, counted in `skipped`
    // only, and it must NOT withhold the state marker.
    let root = tmp("mig-symlink-detail");
    let appdata = seed_appdata(&root);
    let profile_default = appdata.join("WebView2").join("EBWebView").join("Default");
    std::os::unix::fs::symlink("/nonexistent-pico-target", profile_default.join("link"))
        .expect("seed symlink");
    let portable = root.join("exe").join("data");
    let plan = MigrationPlan::Copy {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(report.state_copied, "a pure skip never withholds state");
    assert_eq!(report.details.len(), 1, "{report:?}");
    let detail = &report.details[0];
    assert!(detail.kind.contains("symlink"), "{detail:?}");
    assert_eq!(detail.path, "WebView2/EBWebView/Default/link");
    assert_eq!(detail.error, None);
    assert_eq!(report.skipped, 1, "{report:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_replaces_a_readonly_state_destination() {
    // AUDIT 0.34.1 (retry poisoning, state half): a
    // state.json a partial earlier run left read-only at
    // the destination must not make copy_atomic's rename
    // fail forever — the fresh state lands. (On the Linux
    // host a rename ignores the file mode, so this pins
    // the outcome; the Windows read-only semantics are
    // reasoned, per BUILD_NOTES.)
    let root = tmp("mig-readonly-state");
    let appdata = seed_appdata(&root);
    let portable = root.join("exe").join("data");
    fs::create_dir_all(&portable).expect("portable dir");
    let stale = portable.join("state.json");
    fs::write(&stale, br#"{"theme":"stale"}"#).expect("seed stale state");
    let mut perms = fs::metadata(&stale).expect("stale meta").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&stale, perms).expect("make stale read-only");
    // Profile already present + no portable state would
    // plan CopyStateOnly; a full Copy exercises the same
    // copy_atomic either way — use the planner's own shape:
    // portable state EXISTS here (the stale file), so call
    // perform directly on a state-only plan.
    let plan = MigrationPlan::CopyStateOnly {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert!(report.state_copied, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        fs::read(portable.join("state.json")).expect("state landed"),
        br#"{"theme":"dark"}"#
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_of_a_readonly_source_state_is_not_born_readonly() {
    // AUDIT 0.40.2: `fs::copy` copies the source's
    // permissions, so a read-only appdata state.json used
    // to produce a read-only portable state.json — which
    // then made every later `save_to` rename fail on
    // Windows. The copy must arrive writable. (The
    // permissions half is observable on any host; the
    // Windows rename consequence is reasoned, as in the
    // 0.34.1 test above.)
    let root = tmp("mig-readonly-source");
    let appdata = seed_appdata(&root);
    let src = appdata.join("state.json");
    let mut perms = fs::metadata(&src).expect("src meta").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&src, perms).expect("make source read-only");
    let portable = root.join("exe").join("data");
    let plan = MigrationPlan::CopyStateOnly {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert!(report.state_copied, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    let landed = portable.join("state.json");
    assert_eq!(
        fs::read(&landed).expect("state landed"),
        br#"{"theme":"dark"}"#
    );
    assert!(
        !fs::metadata(&landed)
            .expect("landed meta")
            .permissions()
            .readonly(),
        "the migrated state must not be born read-only"
    );
    // And the writer that runs constantly can replace it.
    save_to(&landed, &PersistState::default()).expect("save over migrated state");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_profile_files_are_not_born_readonly() {
    // AUDIT 0.40.2: the copy_tree half of the same pattern —
    // a read-only profile file in appdata must not arrive
    // read-only in the portable profile WebView2 writes to.
    let root = tmp("mig-readonly-profile");
    let appdata = seed_appdata(&root);
    let cookies_src = appdata
        .join("WebView2")
        .join("EBWebView")
        .join("Default")
        .join("Cookies");
    let mut perms = fs::metadata(&cookies_src)
        .expect("cookies meta")
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(&cookies_src, perms).expect("make cookies read-only");
    let portable = root.join("exe").join("data");
    let plan = MigrationPlan::Copy {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert_eq!(report.profile_files, 3, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    let landed = portable
        .join("WebView2")
        .join("EBWebView")
        .join("Default")
        .join("Cookies");
    assert_eq!(fs::read(&landed).expect("cookies landed"), b"login-cookies");
    assert!(
        !fs::metadata(&landed)
            .expect("landed meta")
            .permissions()
            .readonly(),
        "the migrated profile file must not be born read-only"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_failed_state_copy_leaves_no_temp_behind() {
    // AUDIT 0.40.2: copy_atomic's temp is named exactly
    // like a save_to temp, and the loader promotes a
    // complete temp as the newest intent — so a failed
    // migration copy that leaked its temp would silently
    // become the state at next load, contradicting the
    // migration report that withheld it. Squat the
    // destination with a directory so the rename fails.
    let root = tmp("mig-copy-failure");
    let appdata = seed_appdata(&root);
    let portable = root.join("exe").join("data");
    fs::create_dir_all(portable.join("state.json")).expect("squat destination");
    let plan = MigrationPlan::CopyStateOnly {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert!(!report.state_copied, "{report:?}");
    assert!(report.failed >= 1, "{report:?}");
    let leftovers: Vec<String> = fs::read_dir(&portable)
        .expect("read portable dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("state.json") && n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "leaked temps: {leftovers:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn v29_load_never_deletes_a_fresh_foreign_temp_but_reaps_an_old_one() {
    // A5-2: temp names are per-process and there is no
    // single-instance guard — a second instance's in-flight
    // save temp must survive another instance's load-time
    // cleanup. A genuinely stale (old) temp is still reaped.
    let root = tmp("v29-temp-guard");
    let path = root.join("state.json");
    fs::write(&path, br#"{"version":3,"theme":"light","recents":[]}"#).expect("seed main");
    let fresh = root.join("state.json.424242.tmp");
    fs::write(&fresh, b"{}").expect("seed fresh foreign temp");
    let old = root.join("state.json.31337.tmp");
    fs::write(&old, b"{}").expect("seed old temp");
    {
        let f = fs::File::options()
            .write(true)
            .open(&old)
            .expect("open old temp");
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .expect("age the old temp");
    }
    match load_from_checked(&path) {
        LoadOutcome::Loaded(s) => assert_eq!(s.theme, "light"),
        other => panic!("expected Loaded, got {other:?}"),
    }
    assert!(fresh.is_file(), "a live instance's temp must survive");
    assert!(!old.exists(), "an hour-old temp is a crash leftover");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn v29_corrupt_main_recovers_from_the_newest_complete_temp() {
    // A5-5: a corrupt main file falls back to temp recovery
    // (v28 cleaned the temps before parsing, destroying the
    // recovery source), and among several complete temps the
    // NEWEST wins — not read_dir order.
    let root = tmp("v29-corrupt-recover");
    let path = root.join("state.json");
    fs::write(&path, b"{corrupt").expect("seed corrupt main");
    let older = root.join("state.json.111.tmp");
    fs::write(&older, br#"{"version":3,"theme":"light","recents":[]}"#).expect("seed older temp");
    let newer = root.join("state.json.222.tmp");
    fs::write(&newer, br#"{"version":3,"theme":"dark","recents":[]}"#).expect("seed newer temp");
    {
        let f = fs::File::options()
            .write(true)
            .open(&older)
            .expect("open older");
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .expect("age the older temp");
    }
    match load_from_checked(&path) {
        LoadOutcome::Loaded(s) => assert_eq!(s.theme, "dark", "the newest temp must win"),
        other => panic!("expected Loaded from temp, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// theme_id persistence
// ---------------------------------------------------------------------------

#[test]
fn fresh_state_defaults_to_the_forest_palette() {
    assert_eq!(PersistState::default().theme_id, "forest");
}

#[test]
fn pre_v27_state_loads_with_the_empty_sentinel() {
    // A v26 state file has no theme_id field at all: serde's
    // default fills the empty sentinel, which the app resolves
    // to Forest on first v27 run (AUDIT V27-A2).
    let root = tmp("sentinel");
    let path = root.join("state.json");
    fs::write(&path, br#"{"theme":"dark","shield_on":true}"#).expect("seed v26 state");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(s) => {
            assert_eq!(s.theme_id, "", "the sentinel must survive load");
            assert_eq!(s.theme, "dark", "the mode is preserved");
            assert!(s.shield_on);
        }
        _ => panic!("v26 state must load"),
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn theme_id_roundtrips_and_unknown_ids_sanitize_to_forest() {
    let mut s = PersistState {
        theme_id: "ember".into(),
        ..Default::default()
    };
    s = s.sanitized();
    assert_eq!(s.theme_id, "ember", "known palettes survive sanitize");
    let mut s = PersistState {
        theme_id: "not-a-palette".into(),
        ..Default::default()
    };
    s = s.sanitized();
    assert_eq!(s.theme_id, "forest", "unknown ids fall back to the default");
}

#[test]
fn v31_rose_palette_roundtrips_through_disk() {
    // AUDIT 0.31 (A3-F8): a 0.31 palette id must survive a
    // full save → load round-trip, not just sanitize — if
    // "rose" were not a known palette, load's sanitize pass
    // would reset it to the Forest default.
    let root = tmp("rose-roundtrip");
    let path = root.join("state.json");
    let s = PersistState {
        theme_id: "rose".into(),
        ..Default::default()
    };
    save_to(&path, &s).expect("save rose state");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert_eq!(
                loaded.theme_id, "rose",
                "the 0.31 palette must still be rose after load, not reset to Forest"
            );
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// AUDIT V28 — A5-H1/M1/M2 + A3-LOW-3 hardening
// ---------------------------------------------------------------------------

#[test]
fn v28_planner_never_clobbers_a_portable_profile() {
    let dest = Path::new("C:/Users/x/AppData/Local/Pico");
    let appdata = Path::new("C:/A");
    // Portable has a PROFILE but no state; appdata has both.
    // Through v27 this planned a full Copy and overwrote the
    // live portable profile with the stale appdata one.
    assert!(matches!(
        plan_migration(dest, appdata, false, true, true, true, false),
        MigrationPlan::CopyStateOnly { .. }
    ));
    // Portable profile + appdata profile only (no appdata
    // state): nothing safe to move — profiles are never merged.
    assert_eq!(
        plan_migration(dest, appdata, false, true, false, true, false),
        MigrationPlan::None
    );
}

#[test]
fn v28_state_only_migration_keeps_the_portable_profile() {
    let root = tmp("state-only");
    let appdata = seed_appdata(&root);
    let portable = root.join("exe").join("data");
    // The portable dir already has its OWN profile.
    let own_profile = portable.join("WebView2").join("EBWebView").join("Default");
    fs::create_dir_all(&own_profile).expect("own profile");
    fs::write(own_profile.join("Cookies"), b"portable-cookies").expect("own cookies");
    let plan = MigrationPlan::CopyStateOnly {
        from: appdata,
        to: portable.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert!(report.state_copied);
    assert_eq!(report.profile_files, 0, "no profile file may be copied");
    assert_eq!(
        fs::read(own_profile.join("Cookies")).expect("own cookies intact"),
        b"portable-cookies",
        "the portable profile is untouched"
    );
    assert!(portable.join("state.json").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn v28_window_rect_i32_min_is_invalid() {
    // A5-M2: i32::MIN.abs() wraps in release arithmetic; the
    // i64 check rejects it in every build profile.
    let r = pico_core::core::state::WindowRect {
        x: i32::MIN,
        y: 0,
        w: 1000,
        h: 700,
        maximized: false,
    };
    assert!(!r.is_valid());
    assert!(r.validated().is_none());
}

#[test]
fn v28_state_file_without_theme_field_loads() {
    // A3-LOW-3: `theme` was the one field without a serde
    // default; a file missing it was "corrupt". Now it loads
    // with the light default.
    let root = tmp("no-theme-field");
    let path = root.join("state.json");
    fs::write(&path, br#"{"version":3,"recents":[]}"#).expect("seed");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(s) => assert_eq!(s.theme, "light"),
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn v28_interrupted_save_recovers_from_a_complete_temp() {
    // A save that died between temp-write and rename leaves a
    // complete temp and no main file: the loader recovers the
    // temp's state instead of reporting Missing.
    let root = tmp("tmp-recovery");
    let path = root.join("state.json");
    let tmpfile = root.join(format!("state.json.{}.tmp", std::process::id()));
    fs::write(&tmpfile, br#"{"version":3,"theme":"dark","recents":[]}"#).expect("seed tmp");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(s) => assert_eq!(s.theme, "dark"),
        other => panic!("expected Loaded from temp, got {other:?}"),
    }
    // A TORN temp never recovers.
    let root2 = tmp("tmp-torn");
    let path2 = root2.join("state.json");
    fs::write(
        root2.join("state.json.999.tmp"),
        br#"{"version":3,"theme":"da"#,
    )
    .expect("seed torn");
    assert_eq!(load_from_checked(&path2), LoadOutcome::Missing);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&root2);
}

#[test]
fn v28_unknown_palette_id_rewrite_is_flagged() {
    // A3-M3: the sanitize rewrite is observable via the flag
    // (the app logs it at startup).
    let mut s = PersistState {
        theme_id: "not-a-palette".into(),
        ..Default::default()
    };
    s = s.sanitized();
    assert_eq!(s.theme_id, "forest");
    assert!(s.theme_id_replaced, "the rewrite must be flagged");
    let s = PersistState {
        theme_id: "ember".into(),
        ..Default::default()
    }
    .sanitized();
    assert!(!s.theme_id_replaced);
}

// ---------------------------------------------------------------------------
// 0.41.0 — the full legacy migration (copy → verify → remove)
// ---------------------------------------------------------------------------

use pico_core::core::state::{
    migrate_beside_exe_webview, migrate_legacy_home, verify_migration_copy,
};

#[test]
fn legacy_migration_copies_verifies_and_removes_the_exe_data_tree() {
    let root = tmp("legacy-full");
    // seed_appdata builds <root>/appdata/Pico; use it AS the
    // legacy tree by pointing the migration at it directly.
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert!(outcome.verified, "{outcome:?}");
    assert!(outcome.source_removed, "{outcome:?}");
    assert!(outcome.log_carried, "{outcome:?}");
    assert!(!legacy.exists(), "verified legacy tree is removed");
    // State, profile (the logins), task state all landed.
    assert_eq!(
        fs::read(home.join("state.json")).expect("state landed"),
        br#"{"theme":"dark"}"#
    );
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("cookies landed"),
        b"login-cookies"
    );
    assert!(home.join("downloads").join("7.json").is_file());
    // The legacy log became the new home's log verbatim
    // (the logger had not opened it yet at migration time).
    assert_eq!(
        fs::read(home.join("pico.log")).expect("log carried"),
        b"[1] old session\n"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn legacy_migration_keeps_the_source_when_removal_is_not_asked() {
    // The Roaming shape: verified, but the source stays as
    // the standing backup.
    let root = tmp("legacy-keep");
    let legacy = root.join("appdata").join("Pico");
    let seeded = seed_appdata(&root);
    assert_eq!(seeded, legacy);
    let home = root.join("local").join("Pico");
    let outcome = migrate_legacy_home(&legacy, &home, false);
    assert!(outcome.verified, "{outcome:?}");
    assert!(!outcome.source_removed);
    assert!(legacy.join("state.json").is_file(), "source untouched");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn legacy_migration_failure_keeps_the_legacy_tree_untouched() {
    // Squat the destination profile root with a FILE: the
    // full copy's profile tree cannot be created, the copy
    // fails, state is withheld, verification cannot pass,
    // and the legacy tree must survive byte-for-byte while
    // Pico runs from whatever copies did land.
    // (AUDIT 0.41.1: the pre-0.41.1 fixture squatted a file
    // INSIDE the profile — but that made the destination
    // "have a profile", so the planner chose the
    // state-only copy and nothing failed; the old test
    // passed only because the old full-tree verify
    // incidentally caught the obstruction. See
    // legacy_state_only_copy_verifies_its_own_scope.)
    let root = tmp("legacy-fail");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    fs::create_dir_all(&home).expect("home dir");
    fs::write(home.join("WebView2"), b"squat").expect("squat");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert!(!outcome.verified, "{outcome:?}");
    assert!(!outcome.source_removed);
    assert!(
        !home.join("state.json").is_file(),
        "a failed copy withholds the state marker"
    );
    assert_eq!(
        fs::read(legacy.join("state.json")).expect("legacy state intact"),
        br#"{"theme":"dark"}"#
    );
    assert_eq!(
        fs::read(
            legacy
                .join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("legacy cookies intact"),
        b"login-cookies"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn legacy_fill_completes_a_live_home_without_touching_its_state() {
    // AUDIT 0.41.1 (Front A F2/F4) — contract change,
    // enumerated: through 0.41.0 ANY live dest state stood
    // the migration down completely, which stranded the
    // legacy profile (and task state) whenever the state
    // arrived first (split sources, state-only copies).
    // The refined law: a live dest state is never rewound
    // and a live dest profile is never overwritten — but
    // what the home LACKS is still filled from the legacy
    // tree, and a tree-only fill never removes the source
    // (its state was never carried).
    let root = tmp("legacy-live");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    fs::create_dir_all(&home).expect("home dir");
    fs::write(home.join("state.json"), b"{}").expect("live state");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert!(outcome.verified, "{outcome:?}");
    assert!(!outcome.source_removed, "a tree-only fill keeps the source");
    assert!(legacy.join("state.json").is_file(), "legacy untouched");
    assert_eq!(
        fs::read(home.join("state.json")).expect("home state"),
        b"{}",
        "the live home state is never clobbered"
    );
    // The missing profile and task state were filled in.
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("profile filled"),
        b"login-cookies"
    );
    assert!(home.join("downloads").join("7.json").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn legacy_fill_never_rewinds_a_newer_live_file() {
    // AUDIT 0.41.3 (Front A F1b): the Live-no-pending
    // FILL arm is missing-only now. The home has live
    // state and a NEWER downloads task file, but no
    // profile: the profile fills in, and the newer task
    // file stays byte-identical (through 0.41.2 the
    // fill copied over it with the legacy bytes).
    let root = tmp("legacy-fill-missing-only");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    fs::create_dir_all(home.join("downloads")).expect("live tasks dir");
    fs::write(home.join("state.json"), br#"{"theme":"light"}"#).expect("live state");
    let newer = br#"{"id":7,"note":"newer-live-version","progress":42}"#;
    fs::write(home.join("downloads").join("7.json"), newer).expect("newer task file");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert_eq!(
        fs::read(home.join("downloads").join("7.json")).expect("task file"),
        newer,
        "the fill must never rewind a file the live home already has"
    );
    assert_eq!(
        fs::read(home.join("state.json")).expect("live state"),
        br#"{"theme":"light"}"#
    );
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("profile filled"),
        b"login-cookies",
        "what WAS missing still arrives"
    );
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn verify_follows_a_top_level_symlink_to_a_directory() {
    // AUDIT 0.41.3 (A-S2): the performer follows a
    // top-level symlink-to-dir (fs::metadata) but the
    // verify walk skipped symlinked entries outright,
    // so files copied through the junction were never
    // checked. Now verify walks it like a directory: a
    // clean copy verifies, a tampered file behind the
    // junction fails.
    let root = tmp("verify-junction");
    let legacy = seed_appdata(&root);
    // Replace the seeded downloads dir with a symlink
    // to a real dir holding the task file.
    let real_tasks = root.join("real-tasks");
    fs::create_dir_all(&real_tasks).expect("real tasks");
    fs::write(real_tasks.join("7.json"), br#"{"id":7}"#).expect("task behind junction");
    fs::remove_dir_all(legacy.join("downloads")).expect("unseed downloads");
    std::os::unix::fs::symlink(&real_tasks, legacy.join("downloads")).expect("junction");
    let home = root.join("local").join("Pico");
    let plan = MigrationPlan::Copy {
        from: legacy.clone(),
        to: home.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(
        home.join("downloads").join("7.json").is_file(),
        "the copy followed the junction"
    );
    assert!(
        verify_migration_copy(&legacy, &home),
        "a clean copy through the junction verifies"
    );
    fs::write(
        home.join("downloads").join("7.json"),
        br#"{"id":8,"tampered":1}"#,
    )
    .expect("tamper");
    assert!(
        !verify_migration_copy(&legacy, &home),
        "a tampered file behind the junction fails verify"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn relative_legacy_source_stands_down() {
    // AUDIT 0.41.3 (A-S1): a CWD-relative legacy source
    // must never migrate — its verified removal would
    // delete a relative folder. "src" exists relative
    // to the test working directory (the package root),
    // so the stand-down is the guard, not a missing dir.
    let root = tmp("legacy-relative");
    let home = root.join("local").join("Pico");
    fs::create_dir_all(&home).expect("home dir");
    let outcome = migrate_legacy_home(std::path::Path::new("src"), &home, true);
    assert!(!outcome.attempted, "{outcome:?}");
    assert!(!outcome.verified, "{outcome:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn legacy_migration_stands_down_fully_when_home_has_state_and_profile() {
    // The other half of the refined law: a home with live
    // state AND a live profile (and the task folder) has
    // nothing to fill — total stand-down, as before.
    let root = tmp("legacy-live-full");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    fs::create_dir_all(home.join("WebView2").join("EBWebView").join("Default"))
        .expect("live profile");
    fs::create_dir_all(home.join("downloads")).expect("live tasks");
    fs::write(home.join("state.json"), b"{}").expect("live state");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(!outcome.attempted, "{outcome:?}");
    assert!(legacy.join("state.json").is_file(), "legacy untouched");
    assert_eq!(
        fs::read(home.join("state.json")).expect("home state"),
        b"{}",
        "the live home state is never clobbered"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn legacy_log_carry_on_a_retry_preserves_both_logs() {
    // The new home already has a pico.log (an earlier
    // partial run): the legacy log must land as
    // pico-legacy.log, not merged into the live log.
    let root = tmp("legacy-log-retry");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    fs::create_dir_all(&home).expect("home dir");
    fs::write(home.join("pico.log"), b"[2] new session\n").expect("new log");
    let outcome = migrate_legacy_home(&legacy, &home, false);
    assert!(outcome.log_carried, "{outcome:?}");
    assert_eq!(
        fs::read(home.join("pico.log")).expect("live log"),
        b"[2] new session\n"
    );
    assert_eq!(
        fs::read(home.join("pico-legacy.log")).expect("legacy sidecar"),
        b"[1] old session\n"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn verify_detects_a_tampered_copy() {
    let root = tmp("verify-tamper");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    let plan = MigrationPlan::Copy {
        from: legacy.clone(),
        to: home.clone(),
    };
    let report = pico_core::core::state::perform_migration(&plan);
    assert_eq!(report.failed, 0, "{report:?}");
    assert!(verify_migration_copy(&legacy, &home), "clean copy verifies");
    // A size change anywhere fails verification.
    fs::write(
        home.join("WebView2")
            .join("EBWebView")
            .join("Default")
            .join("History"),
        b"history!",
    )
    .expect("tamper size");
    assert!(!verify_migration_copy(&legacy, &home));
    // A same-size STATE change also fails (state is
    // byte-compared, not just sized).
    let home2 = root.join("local2").join("Pico");
    let plan2 = MigrationPlan::Copy {
        from: legacy.clone(),
        to: home2.clone(),
    };
    let _ = pico_core::core::state::perform_migration(&plan2);
    fs::write(home2.join("state.json"), br#"{"theme":"lite"}"#).expect("tamper state");
    assert!(!verify_migration_copy(&legacy, &home2));
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 0.41.0 — beside-exe WebView2 profile migration
// ---------------------------------------------------------------------------

#[test]
fn beside_exe_webview_profile_migrates_into_the_new_home() {
    let root = tmp("wv-beside");
    let exe = root.join("exe");
    let profile = exe
        .join("Pico.exe.WebView2")
        .join("EBWebView")
        .join("Default");
    fs::create_dir_all(&profile).expect("seed udf");
    fs::write(profile.join("Cookies"), b"login-cookies").expect("seed cookies");
    let home = root.join("local").join("Pico");
    let (attempted, report) = migrate_beside_exe_webview(&exe, &home);
    assert!(attempted);
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("cookies landed in the new profile"),
        b"login-cookies"
    );
    // The source is left in place (this migration never
    // removes).
    assert!(profile.join("Cookies").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn beside_exe_webview_bare_ebwebview_shape_and_no_clobber() {
    let root = tmp("wv-bare");
    let exe = root.join("exe");
    let bare = exe.join("EBWebView").join("Default");
    fs::create_dir_all(&bare).expect("seed bare");
    fs::write(bare.join("Cookies"), b"bare-cookies").expect("seed cookies");
    let home = root.join("local").join("Pico");
    let (attempted, _) = migrate_beside_exe_webview(&exe, &home);
    assert!(attempted);
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("bare profile landed"),
        b"bare-cookies"
    );
    // A second run with a live destination profile never
    // clobbers it.
    fs::write(bare.join("Cookies"), b"changed").expect("change source");
    let (attempted2, _) = migrate_beside_exe_webview(&exe, &home);
    assert!(!attempted2, "destination profile exists: stand down");
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("dest cookies"),
        b"bare-cookies"
    );
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 0.41.0 — the WebView2 folder is explicit on EVERY WebView
// ---------------------------------------------------------------------------

#[test]
fn every_webview_is_built_on_the_one_explicit_context() {
    // Source-scan pin (the media_tests pattern): the shell
    // has exactly ONE WebContext, created with the explicit
    // AppData WebView2 folder, and every WebViewBuilder —
    // chrome and content — goes through it. A builder
    // without the context would let WebView2 default its
    // user-data folder beside the exe.
    let app = fs::read_to_string(format!("{}/src/app.rs", env!("CARGO_MANIFEST_DIR")))
        .expect("read app.rs");
    assert_eq!(
        app.matches("WebContext::new(Some(").count(),
        1,
        "exactly one explicit WebContext"
    );
    assert_eq!(
        app.matches("WebViewBuilder::new_with_web_context").count(),
        2,
        "chrome + content builders both take the context"
    );
    assert!(
        !app.contains("WebViewBuilder::new()"),
        "no context-free WebViewBuilder may exist"
    );
    // The folder itself is the AppData WebView2 dir.
    assert!(
        app.contains("pstore::webview_data_dir()"),
        "the context folder comes from core::state::webview_data_dir"
    );
}

// ---------------------------------------------------------------------------
// 0.41.1 — Front A migration hardening regressions
// ---------------------------------------------------------------------------

#[test]
fn failed_migration_retries_after_the_interim_session_saves_state() {
    // AUDIT 0.41.1 (Front A F1): a failed first copy
    // leaves the pending marker and withholds state; the
    // interim session then saves its OWN state (the F-A1
    // legacy-downloads save makes this deterministic).
    // Through 0.41.0 that dest state stood the migration
    // down and retired the marker — the retry never
    // happened and the profile stayed partial forever.
    // Now the pending marker wins: the tree completes,
    // the interim state is kept, the marker retires.
    let root = tmp("legacy-f1");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    fs::create_dir_all(&home).expect("home dir");
    // First run: the profile copy fails (profile root
    // squatted by a file).
    fs::write(home.join("WebView2"), b"squat").expect("squat");
    let first = migrate_legacy_home(&legacy, &home, true);
    assert!(first.attempted && !first.verified, "{first:?}");
    assert!(
        home.join("migration.pending").is_file(),
        "the failed copy leaves its marker"
    );
    assert!(!home.join("state.json").is_file(), "state withheld");
    // The interim session runs on defaults and saves.
    fs::remove_file(home.join("WebView2")).expect("unsquat");
    fs::write(home.join("state.json"), br#"{"theme":"light"}"#).expect("interim save");
    // AUDIT 0.41.3 (Front A F1): the interim session
    // also has its OWN live WebView2 profile by now —
    // cookies it earned and a marker file the legacy
    // tree knows nothing about.
    let interim_profile = home.join("WebView2").join("EBWebView").join("Default");
    fs::create_dir_all(&interim_profile).expect("interim profile");
    fs::write(interim_profile.join("Cookies"), b"interim-cookies").expect("interim cookies");
    fs::write(interim_profile.join("interim-marker.txt"), b"mine").expect("interim marker");
    // Second run: the tree completes around the live
    // interim state AND profile.
    let second = migrate_legacy_home(&legacy, &home, true);
    assert!(second.attempted, "{second:?}");
    assert!(second.verified, "{second:?}");
    assert_eq!(
        fs::read(home.join("state.json")).expect("interim state kept"),
        br#"{"theme":"light"}"#,
        "the interim session's state is never rewound"
    );
    // CHANGED EXPECTATION (0.41.3): this asserted the
    // legacy "login-cookies" through 0.41.2 — the
    // pending+Live arm passed skip_profile: false, so
    // the retry REWOUND the interim session's live
    // profile with the legacy bytes. That was the bug,
    // not a contract: the interim profile wins now.
    assert_eq!(
        fs::read(interim_profile.join("Cookies")).expect("interim cookies kept"),
        b"interim-cookies",
        "the interim session's live profile is never rewound"
    );
    assert_eq!(
        fs::read(interim_profile.join("interim-marker.txt")).expect("marker kept"),
        b"mine"
    );
    assert!(
        !interim_profile.join("History").exists(),
        "a skipped profile subtree is skipped whole — no legacy files land in it"
    );
    // The rest of the tree still completes: the legacy
    // downloads task file arrives.
    assert_eq!(
        fs::read(home.join("downloads").join("7.json")).expect("task file completed"),
        br#"{"id":7}"#
    );
    assert!(
        !home.join("migration.pending").is_file(),
        "clean completion retires the marker"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn corrupt_dest_state_is_rescued_by_the_legacy_state() {
    // AUDIT 0.41.1 (Front A F12): existence of a dest
    // state.json used to be the whole stand-down test —
    // a truncated file blocked rescue by an intact
    // legacy state forever.
    let root = tmp("legacy-f12");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    fs::create_dir_all(&home).expect("home dir");
    fs::write(home.join("state.json"), br#"{"theme":"#).expect("corrupt state");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert!(outcome.verified, "{outcome:?}");
    assert_eq!(
        fs::read(home.join("state.json")).expect("rescued state"),
        br#"{"theme":"dark"}"#,
        "the legacy state replaces the corrupt one"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn legacy_state_only_copy_verifies_its_own_scope() {
    // AUDIT 0.41.1 (Front A F2): the destination has a
    // LIVE profile (state lost) — the plan is state-only.
    // Through 0.41.0 the full-tree verify then demanded
    // the source's profile bytes at the destination and
    // could never pass. The state-only plan now verifies
    // exactly what it copied; the live profile is never
    // clobbered, and the source is kept (its profile was
    // not carried anywhere).
    let root = tmp("legacy-f2");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    let live_profile = home.join("WebView2").join("EBWebView").join("Default");
    fs::create_dir_all(&live_profile).expect("live profile");
    fs::write(live_profile.join("Cookies"), b"live-cookies!!").expect("live cookies");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert!(outcome.verified, "{outcome:?}");
    assert_eq!(
        fs::read(home.join("state.json")).expect("state landed"),
        br#"{"theme":"dark"}"#
    );
    assert_eq!(
        fs::read(live_profile.join("Cookies")).expect("live cookies kept"),
        b"live-cookies!!",
        "the live profile is never clobbered"
    );
    assert!(
        !outcome.source_removed,
        "a state-only copy keeps the source"
    );
    assert!(legacy.join("state.json").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn deep_tree_beyond_the_copy_cap_still_verifies() {
    // AUDIT 0.41.1 (Front A F2): the copy skips subtrees
    // deeper than its cap as a pure skip; the old verify
    // walk had no cap and demanded those files at the
    // destination — verification could never pass.
    let root = tmp("legacy-deep");
    let legacy = seed_appdata(&root);
    let mut deep = legacy.join("WebView2");
    for i in 0..40 {
        deep = deep.join(format!("d{i}"));
    }
    fs::create_dir_all(&deep).expect("deep tree");
    fs::write(deep.join("leaf.txt"), b"deep").expect("deep leaf");
    let home = root.join("local").join("Pico");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert!(outcome.verified, "{outcome:?}");
    assert!(outcome.source_removed, "{outcome:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn removal_is_withheld_when_the_log_cannot_be_carried() {
    // AUDIT 0.41.1 (Front A F7): the log carry used to be
    // outside the verification gate — a failed carry was
    // silent and the source (holding the only copy of
    // the log) was deleted anyway.
    let root = tmp("legacy-f7");
    let legacy = seed_appdata(&root);
    let home = root.join("local").join("Pico");
    // Both carry destinations squatted by directories.
    fs::create_dir_all(home.join("pico.log")).expect("squat log");
    fs::create_dir_all(home.join("pico-legacy.log")).expect("squat sidecar");
    let outcome = migrate_legacy_home(&legacy, &home, true);
    assert!(outcome.attempted, "{outcome:?}");
    assert!(outcome.verified, "{outcome:?}");
    assert!(outcome.source_had_log, "{outcome:?}");
    assert!(!outcome.log_carried, "{outcome:?}");
    assert!(outcome.removal_withheld_for_log, "{outcome:?}");
    assert!(!outcome.source_removed);
    assert!(
        legacy.join("pico.log").is_file(),
        "the only copy of the log survives"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn verify_detects_a_same_size_tampered_copy() {
    // AUDIT 0.41.1 (Front A F8): small files are
    // byte-compared, not just length-compared.
    let root = tmp("verify-tamper-size");
    let src = root.join("src");
    let dst = root.join("dst");
    fs::create_dir_all(&src).expect("src");
    fs::create_dir_all(&dst).expect("dst");
    fs::write(src.join("History"), b"history").expect("src file");
    fs::write(dst.join("History"), b"HISTORY").expect("dst file");
    assert!(
        !verify_migration_copy(&src, &dst),
        "same length, different bytes must not verify"
    );
    fs::write(dst.join("History"), b"history").expect("dst fixed");
    assert!(verify_migration_copy(&src, &dst));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn beside_exe_webview_partial_copy_retries() {
    // AUDIT 0.41.1 (Front A F3): a first attempt that
    // created the EBWebView directory but copied nothing
    // (its files were locked) used to block every retry —
    // the directory's existence was the only guard. The
    // pending marker now marks the partial tree as ours
    // to resume.
    let root = tmp("wv-f3");
    let exe = root.join("exe");
    let profile = exe
        .join("Pico.exe.WebView2")
        .join("EBWebView")
        .join("Default");
    fs::create_dir_all(&profile).expect("exe profile");
    fs::write(profile.join("Cookies"), b"login-cookies").expect("cookies");
    let home = root.join("local").join("Pico");
    // The husk of the failed first attempt.
    fs::create_dir_all(home.join("WebView2").join("EBWebView")).expect("husk");
    fs::write(
        home.join("webview-migration.pending"),
        b"pico webview migration in progress\n",
    )
    .expect("pending marker");
    let (attempted, report) = migrate_beside_exe_webview(&exe, &home);
    assert!(attempted, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("cookies carried"),
        b"login-cookies"
    );
    assert!(home.join("webview-migration.done").is_file());
    assert!(!home.join("webview-migration.pending").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn beside_exe_webview_never_clobbers_a_live_profile() {
    // The other half of F3's discipline: a destination
    // profile this migration did not start (no pending
    // marker) is live — the beside-exe copy never lands.
    let root = tmp("wv-live");
    let exe = root.join("exe");
    let profile = exe
        .join("Pico.exe.WebView2")
        .join("EBWebView")
        .join("Default");
    fs::create_dir_all(&profile).expect("exe profile");
    fs::write(profile.join("Cookies"), b"old-cookies").expect("old cookies");
    let home = root.join("local").join("Pico");
    let live = home.join("WebView2").join("EBWebView").join("Default");
    fs::create_dir_all(&live).expect("live profile");
    fs::write(live.join("Cookies"), b"live-cookies").expect("live cookies");
    let (attempted, _) = migrate_beside_exe_webview(&exe, &home);
    assert!(!attempted);
    assert_eq!(
        fs::read(live.join("Cookies")).expect("live kept"),
        b"live-cookies"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn beside_exe_webview_falls_back_to_the_bare_spelling() {
    // AUDIT 0.41.1 (Front A F3): when the first candidate
    // exists but holds no files, the bare `EBWebView`
    // candidate still gets its chance (the old dir-exists
    // guard blocked it in the same run).
    let root = tmp("wv-fallback");
    let exe = root.join("exe");
    fs::create_dir_all(exe.join("Pico.exe.WebView2")).expect("empty first candidate");
    let bare = exe.join("EBWebView").join("Default");
    fs::create_dir_all(&bare).expect("bare profile");
    fs::write(bare.join("Cookies"), b"bare-cookies").expect("cookies");
    let home = root.join("local").join("Pico");
    let (attempted, report) = migrate_beside_exe_webview(&exe, &home);
    assert!(attempted, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        fs::read(
            home.join("WebView2")
                .join("EBWebView")
                .join("Default")
                .join("Cookies")
        )
        .expect("cookies carried"),
        b"bare-cookies"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolver_roaming_swap_is_case_insensitive() {
    // AUDIT 0.41.1 (Front A F11).
    let c = resolve_data_dir(
        None,
        None,
        Some(Path::new("C:/Users/x/AppData/roaming")),
        None,
    );
    assert_eq!(c.dir, Path::new("C:/Users/x/AppData/Local/Pico"));
}

#[test]
fn resolver_rejects_relative_and_blank_candidates() {
    // AUDIT 0.41.1 (Front A F11): a relative LOCALAPPDATA
    // would make the home working-directory-dependent;
    // it falls through to the APPDATA-derived candidate.
    let c = resolve_data_dir(
        None,
        Some(Path::new("relative/Local")),
        Some(Path::new("C:/Users/x/AppData/Roaming")),
        None,
    );
    assert_eq!(c.dir, Path::new("C:/Users/x/AppData/Local/Pico"));
    let c = resolve_data_dir(
        None,
        Some(Path::new("   ")),
        None,
        Some(Path::new("/home/x")),
    );
    assert_eq!(c.dir, Path::new("/home/x/.local/share/Pico"));
}

#[test]
fn verbatim_form_only_rewrites_long_windows_paths() {
    use pico_core::core::state::verbatim_form;
    let long_drive = format!("C:\\Users\\long\\{}", "a\\".repeat(120));
    assert!(long_drive.len() > 240);
    assert_eq!(
        verbatim_form(&long_drive).as_deref(),
        Some(format!("\\\\?\\{long_drive}").as_str())
    );
    let long_unc = format!("\\\\server\\share\\{}", "b\\".repeat(120));
    assert_eq!(
        verbatim_form(&long_unc).as_deref(),
        Some(format!("\\\\?\\UNC\\server\\share\\{}", "b\\".repeat(120)).as_str())
    );
    // Short, already-verbatim, and non-Windows shapes are
    // left alone.
    assert_eq!(verbatim_form("C:\\Users\\x"), None);
    assert_eq!(verbatim_form(&format!("\\\\?\\{long_drive}")), None);
    assert_eq!(verbatim_form(&format!("/home/{}", "c/".repeat(130))), None);
    assert_eq!(verbatim_form(&"x".repeat(300)), None);
}

// ---------------------------------------------------------------------------
// 0.41.0 — downloads state two-process guard (lock file)
// ---------------------------------------------------------------------------

#[test]
fn save_task_releases_its_lock() {
    use pico_core::core::download::{save_task, TaskState, TaskStatus};
    let root = tmp("lock-release");
    let dir = root.join("downloads");
    let task = TaskState {
        version: 1,
        id: 7,
        url: "https://example.com/f.bin".to_string(),
        file_name: "f.bin".to_string(),
        total: None,
        etag: None,
        last_modified: None,
        connections: 8,
        status: TaskStatus::Paused,
        added_ms: 1,
        segments: Vec::new(),
        cookies: Vec::new(),
        error: None,
        referer: None,
        sha256: None,
        auto_retries: 0,
        error_hint: None,
    };
    save_task(&dir, &task).expect("save under lock");
    assert!(dir.join("7.json").is_file());
    assert!(
        !dir.join("save.lock").exists(),
        "the lock is released after the save"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn save_task_breaks_a_stale_lock() {
    use pico_core::core::download::{save_task, TaskState, TaskStatus};
    let root = tmp("lock-stale");
    let dir = root.join("downloads");
    fs::create_dir_all(&dir).expect("state dir");
    let lock = dir.join("save.lock");
    fs::write(&lock, b"99999").expect("plant lock");
    {
        let f = fs::File::options()
            .write(true)
            .open(&lock)
            .expect("open lock");
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .expect("age the lock");
    }
    let task = TaskState {
        version: 1,
        id: 9,
        url: "https://example.com/g.bin".to_string(),
        file_name: "g.bin".to_string(),
        total: None,
        etag: None,
        last_modified: None,
        connections: 8,
        status: TaskStatus::Paused,
        added_ms: 1,
        segments: Vec::new(),
        cookies: Vec::new(),
        error: None,
        referer: None,
        sha256: None,
        auto_retries: 0,
        error_hint: None,
    };
    save_task(&dir, &task).expect("stale lock must not block the save");
    assert!(dir.join("9.json").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn save_task_breaks_a_fresh_but_unmoved_lock_after_the_wait() {
    // AUDIT 0.41.1 (Front D D-5): a crash leftover that
    // is still YOUNG (under the 30 s stale horizon) used
    // to tax every save the full 2 s wait and then save
    // unlocked anyway, for up to 30 s of saves. A lock
    // whose mtime never moves across the whole wait is
    // broken once: this save claims it, saves under it,
    // and releases it — the planted lock file is GONE
    // afterwards (under the old fail-open the plant
    // survived every save untouched).
    use pico_core::core::download::{save_task, TaskState, TaskStatus};
    let root = tmp("lock-unmoved");
    let dir = root.join("downloads");
    fs::create_dir_all(&dir).expect("state dir");
    let lock = dir.join("save.lock");
    fs::write(&lock, b"99999").expect("plant fresh lock");
    let task = TaskState {
        version: 1,
        id: 11,
        url: "https://example.com/h.bin".to_string(),
        file_name: "h.bin".to_string(),
        total: None,
        etag: None,
        last_modified: None,
        connections: 8,
        status: TaskStatus::Paused,
        added_ms: 1,
        segments: Vec::new(),
        cookies: Vec::new(),
        error: None,
        referer: None,
        sha256: None,
        auto_retries: 0,
        error_hint: None,
    };
    save_task(&dir, &task).expect("the save proceeds");
    assert!(dir.join("11.json").is_file());
    assert!(
        !lock.exists(),
        "the unmoved leftover was broken and our own lock released"
    );
    let _ = fs::remove_dir_all(&root);
}
