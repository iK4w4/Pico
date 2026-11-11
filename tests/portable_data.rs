//! Portable data + theme persistence (AUDIT V27-A2/V27-C).
//!
//! v27 moves ALL runtime data beside the exe (`<exe>/data/`)
//! by the user's order, with %APPDATA%\Pico as the
//! not-writable fallback and a one-time copy migration from
//! the appdata install. The resolver and the migration
//! planner are pure; the copier is exercised against fixture
//! trees in temp dirs — including the profile that carries
//! the user's logins.

use pico_core::core::state::{
    load_from_checked, plan_migration, resolve_data_dir, save_to, LoadOutcome, MigrationPlan,
    PersistState,
};
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pico-v27-test-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

#[test]
fn resolver_prefers_portable_when_writable() {
    let c = resolve_data_dir(
        Path::new("D:/Pico"),
        Path::new("C:/Users/x/AppData/Roaming"),
        true,
    );
    assert!(c.portable, "writable exe dir must win: {}", c.reason);
    assert_eq!(c.dir, Path::new("D:/Pico/data"));
}

#[test]
fn resolver_falls_back_when_the_probe_fails() {
    // The resolver's appdata input is the appdata Pico dir
    // itself (what appdata_pico_dir computes); the fallback
    // uses it unchanged.
    let c = resolve_data_dir(
        Path::new("C:/Program Files/Pico"),
        Path::new("C:/Users/x/AppData/Roaming/Pico"),
        false,
    );
    assert!(!c.portable);
    assert_eq!(c.dir, Path::new("C:/Users/x/AppData/Roaming/Pico"));
    assert!(
        c.reason.contains("not writable"),
        "the reason rides the startup log line: {}",
        c.reason
    );
}

#[test]
fn resolver_handles_root_and_unicode_exe_dirs() {
    // Exe at a filesystem root: data lands at the root's data/.
    let c = resolve_data_dir(Path::new("/"), Path::new("/home/x/.config"), true);
    assert_eq!(c.dir, Path::new("/data"));
    // Spaces + unicode in the path are just bytes to the join.
    let c = resolve_data_dir(
        Path::new("D:/My Apps/Pico نسخه"),
        Path::new("C:/Users/x/AppData/Roaming"),
        true,
    );
    assert_eq!(c.dir, Path::new("D:/My Apps/Pico نسخه/data"));
    // A network (UNC-style) exe path is accepted as his choice.
    let c = resolve_data_dir(
        Path::new("//NAS/share/Pico"),
        Path::new("C:/Users/x/AppData/Roaming"),
        true,
    );
    assert!(c.portable);
    assert_eq!(c.dir, Path::new("//NAS/share/Pico/data"));
}

// ---------------------------------------------------------------------------
// Migration planner
// ---------------------------------------------------------------------------

fn choice(portable: bool) -> pico_core::core::state::DataDirChoice {
    pico_core::core::state::DataDirChoice {
        dir: PathBuf::from(if portable {
            "E:/Pico/data"
        } else {
            "C:/A/Pico"
        }),
        portable,
        reason: String::new(),
    }
}

#[test]
fn planner_copies_only_into_a_fresh_portable_dir() {
    let appdata = Path::new("C:/A");
    // Fresh portable + appdata state → copy.
    assert!(matches!(
        plan_migration(&choice(true), appdata, false, false, true, false),
        MigrationPlan::Copy { .. }
    ));
    // Fresh portable + appdata profile only → copy.
    assert!(matches!(
        plan_migration(&choice(true), appdata, false, false, false, true),
        MigrationPlan::Copy { .. }
    ));
    // Portable state already exists → NEVER clobber.
    assert_eq!(
        plan_migration(&choice(true), appdata, true, true, true, true),
        MigrationPlan::None
    );
    // Nothing in appdata → nothing to do.
    assert_eq!(
        plan_migration(&choice(true), appdata, false, false, false, false),
        MigrationPlan::None
    );
    // Fallback mode: the appdata dir IS the data dir.
    assert_eq!(
        plan_migration(&choice(false), appdata, false, false, true, true),
        MigrationPlan::None
    );
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
    assert_eq!(report.profile_files, 2);
    assert_eq!(report.skipped, 0);
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
        &pico_core::core::state::DataDirChoice {
            dir: portable.clone(),
            portable: true,
            reason: String::new(),
        },
        &root.join("appdata"),
        portable.join("state.json").is_file(),
        false,
        true,
        true,
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
        &pico_core::core::state::DataDirChoice {
            dir: portable.clone(),
            portable: true,
            reason: String::new(),
        },
        &appdata,
        false,
        true,
        true,
        true,
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
    assert_eq!(report.profile_files, 1, "History still copies: {report:?}");
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
    assert_eq!(report.profile_files, 2, "{report:?}");
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
    let appdata = Path::new("C:/A");
    // Portable has a PROFILE but no state; appdata has both.
    // Through v27 this planned a full Copy and overwrote the
    // live portable profile with the stale appdata one.
    assert!(matches!(
        plan_migration(&choice(true), appdata, false, true, true, true),
        MigrationPlan::CopyStateOnly { .. }
    ));
    // Portable profile + appdata profile only (no appdata
    // state): nothing safe to move — profiles are never merged.
    assert_eq!(
        plan_migration(&choice(true), appdata, false, true, false, true),
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

#[test]
fn v28_probe_rejects_data_as_a_file_and_accepts_a_real_dir() {
    // A5-L4: the fallback trigger, exercised for real.
    let root = tmp("probe");
    let exe = root.join("exe");
    fs::create_dir_all(&exe).expect("exe dir");
    assert!(
        pico_core::core::state::probe_exe_dir(&exe),
        "writable dir probes true"
    );
    assert!(exe.join("data").is_dir(), "the probe creates data/");
    // Now make `data` a regular file: the probe must fail and
    // the resolver must fall back to appdata.
    fs::remove_dir_all(exe.join("data")).expect("remove data dir");
    fs::write(exe.join("data"), b"not a dir").expect("data as file");
    assert!(!pico_core::core::state::probe_exe_dir(&exe));
    let c = resolve_data_dir(&exe, Path::new("C:/appdata/Pico"), false);
    assert!(!c.portable);
    let _ = fs::remove_dir_all(&root);
}
