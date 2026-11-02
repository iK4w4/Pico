//! New-window routing matrix (AUDIT V22-10/15).
//!
//! Pico v21 and earlier denied EVERY `window.open` /
//! `target=_blank` request from content (the handler was a flat
//! `NewWindowResponse::Deny`), which broke full-page sign-in
//! flows — Google/YouTube sign-in among them — that open their
//! credential step in a new window. v22 routes the request
//! through the pure decision in `core::addr::route_new_window`:
//! allowlisted http/https (and the start page) open in a NEW
//! TAB, internal action URLs are denied (actions belong to the
//! gated start-page dispatch only), and every other scheme is
//! denied. These tests pin the whole matrix without a window.

use pico_core::core::addr::{route_new_window, NewWindowRoute};

fn opens(url: &str) -> String {
    match route_new_window(url) {
        NewWindowRoute::OpenTab(u) => u,
        NewWindowRoute::Deny(reason) => panic!("{url} must open a tab, denied: {reason}"),
    }
}

fn denies(url: &str) -> &'static str {
    match route_new_window(url) {
        NewWindowRoute::Deny(reason) => reason,
        NewWindowRoute::OpenTab(u) => panic!("{url} must be denied, opened: {u}"),
    }
}

#[test]
fn ordinary_web_popups_open_in_a_new_tab() {
    assert_eq!(
        opens("https://example.com/page"),
        "https://example.com/page"
    );
    assert_eq!(
        opens("http://127.0.0.1:5173/app"),
        "http://127.0.0.1:5173/app"
    );
    assert_eq!(opens("http://localhost:3000/"), "http://localhost:3000/");
    // Query strings ride along untouched (OAuth state parameters
    // live there).
    assert_eq!(
        opens("https://accounts.google.com/signin?client_id=x&state=y"),
        "https://accounts.google.com/signin?client_id=x&state=y"
    );
}

#[test]
fn the_signin_shaped_popup_opens() {
    // The exact shape that failed in v21: a YouTube/Google
    // sign-in continuation opened via window.open.
    let u = opens("https://accounts.google.com/v3/signin/identifier?flowName=WebLiteSignIn");
    assert!(u.starts_with("https://accounts.google.com/"));
}

#[test]
fn the_start_page_may_open_as_a_tab() {
    assert_eq!(opens("pico://start"), "pico://start");
    // wry's folded spelling normalizes to the same tab.
    assert_eq!(opens("https://pico.start/"), "pico://start");
}

#[test]
fn internal_action_urls_are_denied_with_their_own_reason() {
    // Actions may only ever be decided by the gated dispatch;
    // a page must not mint one by opening a window.
    for u in [
        "pico://pick-file",
        "pico://recent/2",
        "pico://recent",
        "pico://social/telegram",
        "pico://social",
        "https://pico.recent/2",
        "https://pico.social/youtube",
        "pico://social/telegram?x=1",
    ] {
        assert_eq!(denies(u), "internal-action-url", "url: {u}");
    }
}

#[test]
fn non_web_schemes_are_denied() {
    for u in [
        "javascript:alert(1)",
        "data:text/html,<h1>x</h1>",
        "file:///C:/Users/x/secret.html",
        "about:blank",
        "about:config",
        "view-source:https://example.com/",
        "chrome://settings",
        "ms-settings:something",
        "ftp://example.com/file",
        "pico://chrome",
        "",
        "not a url",
    ] {
        assert_eq!(denies(u), "scheme-not-allowed", "url: {u}");
    }
}

#[test]
fn routing_never_produces_a_url_outside_the_allowlist() {
    // Belt-and-braces over the matrix above: whatever comes out
    // as OpenTab must itself pass the navigation allowlist, so
    // the shell's add_tab can never be handed a surprise.
    for u in [
        "https://example.com",
        "pico://start",
        "https://pico.start/",
        "http://192.168.1.10:8080/x",
    ] {
        let opened = opens(u);
        assert!(
            pico_core::core::addr::is_allowed_navigation_url(&opened),
            "routed URL must be allowlisted: {opened}"
        );
    }
}

#[test]
fn the_shell_wires_the_pure_router_and_logs_decisions() {
    // Source pins: the event arm must call the pure function
    // (not re-implement the policy inline, as v21 did) and must
    // log both outcomes.
    let app = std::fs::read_to_string(format!("{}/src/app.rs", env!("CARGO_MANIFEST_DIR")))
        .expect("read src/app.rs");
    assert!(
        app.contains("route_new_window(&raw_url)"),
        "the NewWindow event arm must use core::addr::route_new_window"
    );
    assert!(
        app.contains("new-window: opened in new tab"),
        "allowed routes must be logged"
    );
    assert!(
        app.contains("new-window: denied reason="),
        "denied routes must be logged with their reason"
    );
}
