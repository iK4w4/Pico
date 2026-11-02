//! Seam tests for the v17 start-page action rebuild (AUDIT V17-1).
//!
//! Two seams that no v16 test exercised end-to-end:
//! 1. `core::action::dispatch` — the single decision function for
//!    start-page actions, tested over the full
//!    requester × action matrix for BOTH transports (IPC JSON and
//!    legacy action URLs).
//! 2. `core::fileopen::resolve_open_target` — the shared file-open
//!    core, tested against real files on disk. The file case is
//!    the regression pin for AUDIT V17-2 (v13–v16 joined the file
//!    name onto the file itself and failed every file open).

use std::path::PathBuf;

use pico_core::core::action::{
    dispatch, kind_label, looks_like_action, ActionSource, Decision, DenyReason, Requester,
};
use pico_core::core::fileopen::{resolve_open_target, OpenResolveError};
use pico_core::core::ipc::SocialSite;

// ---------------------------------------------------------------------
// Requesters: the four corners of the gate.
// ---------------------------------------------------------------------

fn start_at_start() -> Requester {
    Requester {
        is_start_tab: true,
        current_url: "pico://start".to_string(),
    }
}
fn start_at_start_slash() -> Requester {
    Requester {
        is_start_tab: true,
        current_url: "pico://start/".to_string(),
    }
}
fn start_at_website() -> Requester {
    Requester {
        is_start_tab: true,
        current_url: "https://example.com/".to_string(),
    }
}
fn other_at_website() -> Requester {
    Requester {
        is_start_tab: false,
        current_url: "https://example.com/".to_string(),
    }
}
fn other_at_start() -> Requester {
    // Should be impossible at runtime (only the start tab can show
    // the start page) — the gate must deny it anyway.
    Requester {
        is_start_tab: false,
        current_url: "pico://start".to_string(),
    }
}

fn recents() -> Vec<String> {
    vec![
        "C:\\Users\\x\\Downloads\\Good Girl - Jasmine.html".to_string(),
        "C:\\Users\\x\\Desktop\\LLM.html".to_string(),
    ]
}

fn ipc(body: &str) -> ActionSource {
    ActionSource::Ipc(body.to_string())
}
fn url(u: &str) -> ActionSource {
    ActionSource::Url(u.to_string())
}

// ---------------------------------------------------------------------
// Happy path — start tab on the start page, both transports.
// ---------------------------------------------------------------------

#[test]
fn recent_valid_index_opens_the_path_via_both_transports() {
    let r = recents();
    let want = Decision::OpenRecent(PathBuf::from(&r[1]));
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recent","index":1}"#),
            &start_at_start(),
            &r
        ),
        want
    );
    assert_eq!(
        dispatch(&url("pico://recent/1"), &start_at_start(), &r),
        want
    );
    // Workaround spelling + trailing slash, as WebView2 reports it.
    assert_eq!(
        dispatch(&url("https://pico.recent/1"), &start_at_start_slash(), &r),
        want
    );
    assert_eq!(
        dispatch(&url("pico://recent/0/"), &start_at_start(), &r),
        Decision::OpenRecent(PathBuf::from(&r[0]))
    );
}

#[test]
fn pick_file_opens_the_picker_via_both_transports() {
    let r = recents();
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"pick-file"}"#),
            &start_at_start(),
            &r
        ),
        Decision::OpenPicker
    );
    assert_eq!(
        dispatch(&url("pico://pick-file"), &start_at_start(), &r),
        Decision::OpenPicker
    );
    assert_eq!(
        dispatch(&url("https://pico.pick-file/"), &start_at_start(), &r),
        Decision::OpenPicker
    );
}

#[test]
fn service_opens_via_both_transports() {
    let r = recents();
    let want = Decision::OpenService("http://localhost:5173/".to_string());
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"service","url":"http://localhost:5173/"}"#),
            &start_at_start(),
            &r
        ),
        want
    );
    assert_eq!(
        dispatch(&url("http://localhost:5173/"), &start_at_start(), &r),
        want
    );
}

#[test]
fn social_is_enum_only_via_both_transports() {
    let r = recents();
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"social","site":"telegram"}"#),
            &start_at_start(),
            &r
        ),
        Decision::OpenSocial(SocialSite::Telegram)
    );
    assert_eq!(
        dispatch(&url("pico://social/youtube"), &start_at_start(), &r),
        Decision::OpenSocial(SocialSite::Youtube)
    );
}

// ---------------------------------------------------------------------
// The gate — every non-genuine requester is denied, for every action.
// ---------------------------------------------------------------------

#[test]
fn gate_denies_everything_from_a_non_start_tab() {
    let r = recents();
    let sources = [
        ipc(r#"{"kind":"action","action":"pick-file"}"#),
        ipc(r#"{"kind":"action","action":"recent","index":0}"#),
        ipc(r#"{"kind":"action","action":"social","site":"telegram"}"#),
        url("pico://pick-file"),
        url("pico://recent/0"),
        url("http://localhost:5173/"),
    ];
    for req in [other_at_website(), other_at_start()] {
        for src in &sources {
            assert_eq!(
                dispatch(src, &req, &r),
                Decision::Deny(DenyReason::NotStartTab),
                "requester {req:?} source {src:?}"
            );
        }
    }
}

#[test]
fn gate_denies_a_start_tab_that_navigated_away() {
    let r = recents();
    for src in [
        ipc(r#"{"kind":"action","action":"pick-file"}"#),
        url("pico://recent/0"),
        url("pico://social/telegram"),
    ] {
        assert_eq!(
            dispatch(&src, &start_at_website(), &r),
            Decision::Deny(DenyReason::NotOnStartPage),
            "source {src:?}"
        );
    }
}

#[test]
fn gate_runs_before_parsing() {
    // Even total garbage from the wrong requester is a gate denial,
    // not a parse result: the parser is unreachable cross-tab.
    let r = recents();
    assert_eq!(
        dispatch(&ipc("not json at all"), &other_at_website(), &r),
        Decision::Deny(DenyReason::NotStartTab)
    );
    assert_eq!(
        dispatch(&ipc("not json at all"), &start_at_website(), &r),
        Decision::Deny(DenyReason::NotOnStartPage)
    );
}

// ---------------------------------------------------------------------
// Malformed and hostile inputs from the genuine requester.
// ---------------------------------------------------------------------

#[test]
fn recent_out_of_range_and_malformed_indexes() {
    let r = recents();
    assert_eq!(
        dispatch(&url("pico://recent/2"), &start_at_start(), &r),
        Decision::Deny(DenyReason::RecentOutOfRange)
    );
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recent","index":99}"#),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::RecentOutOfRange)
    );
    for bad in [
        "pico://recent/",
        "pico://recent/abc",
        "pico://recent/99999999999999999999999999",
        "pico://recent/-1",
        "pico://recent/..%2f..",
    ] {
        let d = dispatch(&url(bad), &start_at_start(), &r);
        assert!(
            matches!(
                d,
                Decision::Deny(DenyReason::Malformed)
                    | Decision::Deny(DenyReason::RecentOutOfRange)
            ),
            "{bad} -> {d:?}"
        );
    }
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recent"}"#),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::Malformed)
    );
}

#[test]
fn social_unknown_and_wrong_case_are_denied() {
    let r = recents();
    for name in ["myspace", "Telegram", "TELEGRAM", "telegram.evil", ""] {
        let body = format!(r#"{{"kind":"action","action":"social","site":"{name}"}}"#);
        assert_eq!(
            dispatch(&ipc(&body), &start_at_start(), &r),
            Decision::Deny(DenyReason::UnknownSocialSite),
            "site {name:?}"
        );
        if !name.is_empty() {
            assert_eq!(
                dispatch(
                    &url(&format!("pico://social/{name}")),
                    &start_at_start(),
                    &r
                ),
                Decision::Deny(DenyReason::UnknownSocialSite),
                "url site {name:?}"
            );
        }
    }
}

#[test]
fn service_url_must_be_on_the_navigation_allowlist() {
    let r = recents();
    for bad in [
        "javascript:alert(1)",
        "data:text/html,<b>x</b>",
        "file:///C:/secret.html",
        "ftp://localhost/x",
        "http://",
        "",
    ] {
        let body = format!(r#"{{"kind":"action","action":"service","url":"{bad}"}}"#);
        assert_eq!(
            dispatch(&ipc(&body), &start_at_start(), &r),
            Decision::Deny(DenyReason::BadServiceUrl),
            "url {bad:?}"
        );
    }
}

#[test]
fn malformed_sources_are_classified_not_panicked() {
    let r = recents();
    assert_eq!(
        dispatch(&ipc(""), &start_at_start(), &r),
        Decision::Deny(DenyReason::Malformed)
    );
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"nav","how":"push"}"#),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::Malformed)
    );
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"teleport"}"#),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::UnknownAction)
    );
    assert_eq!(
        dispatch(&url("pico://unknown/thing"), &start_at_start(), &r),
        Decision::Deny(DenyReason::UnknownAction)
    );
    assert_eq!(
        dispatch(&url(""), &start_at_start(), &r),
        Decision::Deny(DenyReason::Malformed)
    );
    // Oversized IPC body -> malformed, never a giant parse.
    let big = format!(
        r#"{{"kind":"action","action":"service","url":"http://localhost/{}}}"#,
        "a".repeat(5000)
    );
    assert_eq!(
        dispatch(&ipc(&big), &start_at_start(), &r),
        Decision::Deny(DenyReason::Malformed)
    );
}

#[test]
fn looks_like_action_routes_only_action_kind() {
    assert!(looks_like_action(
        r#"{"kind":"action","action":"pick-file"}"#
    ));
    assert!(!looks_like_action(
        r#"{"kind":"nav","how":"push","href":"http://x/"}"#
    ));
    assert!(!looks_like_action("garbage"));
    assert!(!looks_like_action(""));
}

#[test]
fn kind_labels_cover_both_transports() {
    assert_eq!(kind_label(&url("pico://recent/0")), "recent");
    assert_eq!(kind_label(&url("pico://pick-file")), "pick-file");
    assert_eq!(kind_label(&url("https://pico.social/telegram")), "social");
    assert_eq!(kind_label(&url("http://localhost:3000/")), "service");
    assert_eq!(
        kind_label(&ipc(r#"{"kind":"action","action":"recent","index":0}"#)),
        "recent"
    );
    assert_eq!(kind_label(&ipc("junk")), "unknown");
    // v24 kinds label distinctly — "recent" must not swallow
    // "recent-remove" on either transport.
    assert_eq!(kind_label(&url("pico://recent-remove/0")), "recent-remove");
    assert_eq!(kind_label(&url("pico://recents-clear")), "recents-clear");
    assert_eq!(
        kind_label(&url("pico://recents-visibility/hide")),
        "recents-visibility"
    );
    assert_eq!(
        kind_label(&ipc(
            r#"{"kind":"action","action":"recent-remove","index":0}"#
        )),
        "recent-remove"
    );
    assert_eq!(
        kind_label(&ipc(r#"{"kind":"action","action":"recents-clear"}"#)),
        "recents-clear"
    );
    assert_eq!(
        kind_label(&ipc(
            r#"{"kind":"action","action":"recents-visibility","hidden":true}"#
        )),
        "recents-visibility"
    );
}

// ---------------------------------------------------------------------
// v24 recents management (AUDIT V24-2): remove / clear / hide,
// through the SAME gate as every other gesture.
// ---------------------------------------------------------------------

#[test]
fn recent_remove_decides_via_both_transports() {
    let r = recents();
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recent-remove","index":1}"#),
            &start_at_start(),
            &r
        ),
        Decision::RemoveRecent(1)
    );
    assert_eq!(
        dispatch(&url("pico://recent-remove/0"), &start_at_start(), &r),
        Decision::RemoveRecent(0)
    );
    // Folded spelling lands in the same dispatch.
    assert_eq!(
        dispatch(
            &url("https://pico.recent-remove/1"),
            &start_at_start_slash(),
            &r
        ),
        Decision::RemoveRecent(1)
    );
}

#[test]
fn recents_clear_and_visibility_decide_via_both_transports() {
    let r = recents();
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recents-clear"}"#),
            &start_at_start(),
            &r
        ),
        Decision::ClearRecents
    );
    assert_eq!(
        dispatch(&url("pico://recents-clear"), &start_at_start(), &r),
        Decision::ClearRecents
    );
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recents-visibility","hidden":true}"#),
            &start_at_start(),
            &r
        ),
        Decision::SetRecentsHidden(true)
    );
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recents-visibility","hidden":false}"#),
            &start_at_start(),
            &r
        ),
        Decision::SetRecentsHidden(false)
    );
    assert_eq!(
        dispatch(
            &url("pico://recents-visibility/hide"),
            &start_at_start(),
            &r
        ),
        Decision::SetRecentsHidden(true)
    );
    assert_eq!(
        dispatch(
            &url("pico://recents-visibility/show"),
            &start_at_start(),
            &r
        ),
        Decision::SetRecentsHidden(false)
    );
}

#[test]
fn recents_management_is_gated_and_range_checked() {
    let r = recents();
    // The gate denies the new kinds from everywhere the old
    // kinds are denied — non-start tab, and start tab away.
    for req in [&other_at_website(), &other_at_start(), &start_at_website()] {
        assert_eq!(
            dispatch(
                &ipc(r#"{"kind":"action","action":"recent-remove","index":0}"#),
                req,
                &r
            ),
            Decision::Deny(if req.is_start_tab {
                DenyReason::NotOnStartPage
            } else {
                DenyReason::NotStartTab
            })
        );
        assert_eq!(
            dispatch(&url("pico://recents-clear"), req, &r),
            Decision::Deny(if req.is_start_tab {
                DenyReason::NotOnStartPage
            } else {
                DenyReason::NotStartTab
            })
        );
        assert_eq!(
            dispatch(&url("pico://recents-visibility/hide"), req, &r),
            Decision::Deny(if req.is_start_tab {
                DenyReason::NotOnStartPage
            } else {
                DenyReason::NotStartTab
            })
        );
    }
    // Out-of-range removes deny with the recents reason (they
    // are logged, never clamped, never panicked).
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recent-remove","index":2}"#),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::RecentOutOfRange)
    );
    assert_eq!(
        dispatch(&url("pico://recent-remove/99"), &start_at_start(), &r),
        Decision::Deny(DenyReason::RecentOutOfRange)
    );
    // Malformed shapes: missing index, missing hidden flag,
    // bogus visibility segment, non-numeric index.
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recent-remove"}"#),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::Malformed)
    );
    assert_eq!(
        dispatch(
            &ipc(r#"{"kind":"action","action":"recents-visibility"}"#),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::Malformed)
    );
    assert_eq!(
        dispatch(
            &url("pico://recents-visibility/sideways"),
            &start_at_start(),
            &r
        ),
        Decision::Deny(DenyReason::Malformed)
    );
    assert_eq!(
        dispatch(&url("pico://recent-remove/abc"), &start_at_start(), &r),
        Decision::Deny(DenyReason::Malformed)
    );
    // Clear/visibility work on an EMPTY list too (idempotent).
    let empty: Vec<String> = Vec::new();
    assert_eq!(
        dispatch(&url("pico://recents-clear"), &start_at_start(), &empty),
        Decision::ClearRecents
    );
}

// ---------------------------------------------------------------------
// resolve_open_target — the shared file-open core (AUDIT V17-2).
// ---------------------------------------------------------------------

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pico-v20-open-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn resolve_file_points_at_the_file_itself() {
    // THE v17 regression pin: with v13–v16's `canonical.join(&rel)`
    // check this resolution failed for every regular file.
    let dir = scratch("file");
    let file = dir.join("My Page.html");
    std::fs::write(&file, "<title>x</title>").expect("write file");
    let got = resolve_open_target(&file).expect("a plain HTML file must resolve");
    assert_eq!(got.rel, "My Page.html");
    assert_eq!(got.file, got.folder.join(&got.rel));
    assert!(got.file.is_file());
    // The served file is the file the user picked — not a namesake
    // appended beneath it.
    let canon = std::fs::canonicalize(&file).expect("canonicalize");
    assert_eq!(got.file, canon);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_directory_uses_its_index_html() {
    let dir = scratch("dir");
    std::fs::write(dir.join("index.html"), "home").expect("write index");
    let got = resolve_open_target(&dir).expect("directory with index.html resolves");
    assert_eq!(got.rel, "index.html");
    assert_eq!(
        got.folder,
        std::fs::canonicalize(&dir).expect("canonicalize")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_directory_without_index_is_missing_file() {
    let dir = scratch("dir-noindex");
    match resolve_open_target(&dir) {
        Err(OpenResolveError::MissingFile(p)) => {
            assert!(p.ends_with("index.html"), "target was {p:?}");
        }
        other => panic!("expected MissingFile, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_missing_path_is_cannot_resolve() {
    let dir = scratch("missing");
    let ghost = dir.join("nope.html");
    assert!(matches!(
        resolve_open_target(&ghost),
        Err(OpenResolveError::CannotResolve(_))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_unc_is_refused_before_any_filesystem_touch() {
    // Lexical gate: these strings must never reach canonicalize.
    for raw in [
        "\\\\server\\share\\page.html",
        "\\\\?\\UNC\\server\\share\\page.html",
        "//server/share/page.html",
    ] {
        assert_eq!(
            resolve_open_target(std::path::Path::new(raw)),
            Err(OpenResolveError::UnsafePath),
            "input {raw:?}"
        );
    }
}

// ---- AUDIT V26 (D5): conversion closes the privileged gate ----

/// The app assembles `Requester` from the tab's LIVE fields
/// (`is_start`, `url`) at dispatch time. `convert_start_tab`
/// flips both in the same instant it loads the target, so the
/// requester a converted tab presents is exactly the second
/// one below — and every start action from it must be denied
/// at the FIRST gate conjunct (NotStartTab), before parsing.
/// This is the core-level pin for the conversion's security
/// follow-through; the app-level wiring is compile-checked.
#[test]
fn v26_converted_start_tab_loses_the_gate() {
    let r = recents();
    let before = Requester {
        is_start_tab: true,
        current_url: "pico://start".to_string(),
    };
    let after = Requester {
        is_start_tab: false,
        current_url: "https://www.google.com/search?q=hello".to_string(),
    };
    for src in [
        ipc(r#"{"kind":"action","action":"pick-file"}"#),
        ipc(r#"{"kind":"action","action":"recent-remove","index":0}"#),
        url("pico://recent/0"),
        url("pico://social/telegram"),
    ] {
        assert_ne!(
            dispatch(&src, &before, &r),
            Decision::Deny(DenyReason::NotStartTab),
            "pre-conversion requester must pass the first conjunct: {src:?}"
        );
        assert_eq!(
            dispatch(&src, &after, &r),
            Decision::Deny(DenyReason::NotStartTab),
            "converted tab must be denied at the gate: {src:?}"
        );
    }
}
