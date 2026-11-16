//! 0.41.0 (Workstream C) — movable tabs: the parts of
//! the feature the GUI cannot prove on this host, pinned
//! at the levels that CAN run here.
//!
//! - The reorder math and the new-Pico plan are pure
//!   `core::tabcmd` functions; their unit tests live in
//!   the module. These integration tests re-check the
//!   public surface end-to-end (a strip of ids through
//!   the drag gaps, the plan matrix through the facts
//!   the app assembles) so the contract the app codes
//!   against is the contract that ships.
//! - The app-side wiring that only a Windows runtime can
//!   execute (the reorder application, the accelerator
//!   chords, the new-process spawn) is pinned by source
//!   scans in the house style: the exact choke points
//!   must exist and must route through the pure core
//!   functions, so a future refactor cannot silently
//!   re-implement (or drop) them elsewhere.

use pico_core::core::tabcmd::{
    insertion_to_final, new_pico_plan, reorder_ids, step_final, tab_menu_items, NewPicoFacts,
    NewPicoPlan, TAB_MENU_MOVE_LEFT, TAB_MENU_MOVE_RIGHT, TAB_MENU_OPEN_NEW_PICO,
};

fn app_source() -> String {
    let path = format!("{}/src/app.rs", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read app.rs: {e}"))
}

fn plain_web() -> NewPicoFacts {
    NewPicoFacts {
        is_internal: false,
        is_web: true,
        is_local_server: false,
        has_userinfo: false,
        has_query_secret: false,
        has_pending_capture: false,
        failed: false,
    }
}

/// A whole strip dragged through every gap: the moved id
/// lands exactly where `insertion_to_final` says, the
/// relative order of every OTHER id is preserved, and
/// the result is always a permutation.
#[test]
fn drag_gaps_preserve_everyone_elses_order() {
    let ids = [1u64, 2, 3, 4, 5];
    for from in 0..ids.len() {
        for gap in 0..=ids.len() {
            let out = reorder_ids(&ids, from, gap).unwrap();
            let fin = insertion_to_final(ids.len(), from, gap).unwrap();
            assert_eq!(out[fin], ids[from], "from={from} gap={gap}");
            let rest: Vec<u64> = out.iter().copied().filter(|i| *i != ids[from]).collect();
            let want: Vec<u64> = ids.iter().copied().filter(|i| *i != ids[from]).collect();
            assert_eq!(rest, want, "others keep their order, from={from} gap={gap}");
        }
    }
}

/// The keyboard step applied repeatedly walks a tab to
/// the edge and stops there (clamped, never wrapping).
#[test]
fn step_walk_reaches_the_edge_and_stays() {
    let mut idx = 0usize;
    for _ in 0..10 {
        idx = step_final(4, idx, 1).unwrap();
    }
    assert_eq!(idx, 3);
    for _ in 0..10 {
        idx = step_final(4, idx, -1).unwrap();
    }
    assert_eq!(idx, 0);
}

/// The new-Pico plan, through the public facts shape the
/// app assembles: only a plain web tab with nothing in
/// flight is a true move (the original closes).
#[test]
fn only_the_plain_web_tab_is_a_true_move() {
    assert_eq!(new_pico_plan(&plain_web()), NewPicoPlan::MoveAndClose);
    for mutate in [
        |f: &mut NewPicoFacts| f.has_pending_capture = true,
        |f: &mut NewPicoFacts| f.failed = true,
    ] {
        let mut f = plain_web();
        mutate(&mut f);
        assert_ne!(new_pico_plan(&f), NewPicoPlan::MoveAndClose);
    }
    let mut internal = plain_web();
    internal.is_internal = true;
    internal.is_web = false;
    assert_eq!(new_pico_plan(&internal), NewPicoPlan::OpenBare);
}

/// The menu the app stores is the core model's: the
/// new-Pico row's label is the plan's honest name.
#[test]
fn menu_open_row_label_is_the_plan_name() {
    let label_for = |plan| {
        tab_menu_items(0, 2, plan)
            .into_iter()
            .find(|i| i.id == TAB_MENU_OPEN_NEW_PICO)
            .map(|i| i.label)
    };
    assert_eq!(
        label_for(NewPicoPlan::MoveAndClose),
        Some("Move to New Window")
    );
    assert_eq!(
        label_for(NewPicoPlan::OpenWithUrl),
        Some("Open in New Pico")
    );
    assert_eq!(label_for(NewPicoPlan::OpenBare), Some("Open in New Pico"));
    assert_eq!(label_for(NewPicoPlan::NotOffered), None);
    // The move rows are always present, ids stable.
    let items = tab_menu_items(0, 2, NewPicoPlan::NotOffered);
    assert_eq!(items[0].id, TAB_MENU_MOVE_LEFT);
    assert_eq!(items[1].id, TAB_MENU_MOVE_RIGHT);
}

/// Source pin: the app's reorder paths route through the
/// pure core math — there is no second reorder
/// implementation in the shell.
#[test]
fn source_scan_app_reorder_routes_through_core_math() {
    let app = app_source();
    assert!(
        app.contains("tabcmd::final_from_neighbor"),
        "the drag path must resolve the drop neighbour via core::tabcmd::final_from_neighbor"
    );
    assert!(
        app.contains("tabcmd::step_final"),
        "the step path must clamp via core::tabcmd::step_final"
    );
    assert_eq!(
        app.matches("self.tabs.remove(from)").count(),
        1,
        "exactly one vec mutation applies a tab move (apply_tab_move)"
    );
    assert!(
        app.contains("fn on_chrome_tab_body"),
        "tab bodies have their one chrome handler (envelope dedupe + parser + gate)"
    );
    assert!(
        app.contains("tabcmd::looks_like_tab_body(raw)"),
        "on_chrome_cmd routes tab bodies before the command parser"
    );
}

/// Source pin: the keyboard move rides the ONE
/// accelerator choke point (the Ctrl+J registration) —
/// PageUp/PageDown arms with the inverted Shift rule —
/// and the chrome document sends the same step verb.
#[test]
fn source_scan_keyboard_move_rides_the_accelerator_choke_point() {
    let app = app_source();
    assert!(
        app.contains("0x21 => UserEvent::MoveTabShortcut { dir: -1 }"),
        "VK_PRIOR (PageUp) arm exists in the accelerator registration"
    );
    assert!(
        app.contains("0x22 => UserEvent::MoveTabShortcut { dir: 1 }"),
        "VK_NEXT (PageDown) arm exists in the accelerator registration"
    );
    assert!(
        app.contains("UserEvent::MoveTabShortcut { dir } =>"),
        "the event arm moves the active tab"
    );
    assert!(
        app.contains("app.move_tab_step(active, dir)"),
        "the event arm routes through the one step function"
    );
    let chrome =
        std::fs::read_to_string(format!("{}/assets/chrome.html", env!("CARGO_MANIFEST_DIR")))
            .expect("read chrome.html");
    assert!(
        chrome.contains("ev.key === \"PageUp\"") && chrome.contains("ev.key === \"PageDown\""),
        "the chrome document handles Ctrl+Shift+PageUp/PageDown itself"
    );
}

/// Source pin: the new-Pico route is a new PROCESS (the
/// current exe, the tab's URL as its argument when the
/// plan carries one), the original closes ONLY for the
/// MoveAndClose plan and ONLY after a successful spawn,
/// and the pending-capture fact comes from the
/// coordinator's own query.
#[test]
fn source_scan_new_pico_is_a_new_process_with_an_honest_close() {
    let app = app_source();
    assert!(
        app.contains("std::env::current_exe()"),
        "the new Pico is this exe, relaunched"
    );
    assert!(
        app.contains("std::process::Command::new(&exe)"),
        "the launch is a fresh process, not a second in-process window"
    );
    assert!(
        app.contains("self.dl.has_pending_capture(tab.id)"),
        "the closable gate reads the coordinator's pending-capture state"
    );
    let spawn_at = app.find("cmd.spawn()").expect("the spawn exists");
    let close_at = app
        .find("self.close_tab(tab_id)")
        .expect("the MoveAndClose close exists");
    assert!(
        spawn_at < close_at,
        "the original closes only after the spawn (the handoff point)"
    );
    assert!(
        app.contains("if plan == tabcmd::NewPicoPlan::MoveAndClose"),
        "the close is gated on the MoveAndClose plan alone"
    );
}

// ---------------------------------------------------------------------------
// 0.41.1 — Front C: facts assembly, query secrets, tab id 0
// ---------------------------------------------------------------------------

#[test]
fn facts_assembly_classifies_the_localhost_alias_as_local_server() {
    // AUDIT 0.41.1 (Front C F1): the tab sits on Pico's
    // own file server under its LOCALHOST alias — no
    // root id on the tab, no exact root match (the alias
    // spelling is not the 127.0.0.1 prefix). Through
    // 0.41.0 this was "plain web": Move to New Window,
    // token URL on argv, original closed, dead page.
    use pico_core::core::tabcmd::{new_pico_facts_for, NewPicoPlan};
    let facts = new_pico_facts_for(
        "http://localhost:49152/r/TOKEN123/page.html",
        false,
        false,
        false,
        Some(49152),
        false,
        false,
    );
    assert!(facts.is_local_server, "{facts:?}");
    assert_eq!(new_pico_plan(&facts), NewPicoPlan::NotOffered);
    // The same URL on a DIFFERENT port is a real dev
    // server — plain web, movable.
    let facts = new_pico_facts_for(
        "http://localhost:5173/r/TOKEN123/page.html",
        false,
        false,
        false,
        Some(49152),
        false,
        false,
    );
    assert!(!facts.is_local_server, "{facts:?}");
    assert_eq!(new_pico_plan(&facts), NewPicoPlan::MoveAndClose);
    // And the plain web control still moves.
    let facts = new_pico_facts_for(
        "https://example.com/page",
        false,
        false,
        false,
        Some(49152),
        false,
        false,
    );
    assert!(facts.is_web && !facts.is_local_server);
    assert_eq!(new_pico_plan(&facts), NewPicoPlan::MoveAndClose);
}

#[test]
fn query_and_fragment_secrets_are_never_offered() {
    // AUDIT 0.41.1 (Front C F2).
    use pico_core::core::tabcmd::{has_query_secret, new_pico_facts_for, NewPicoPlan};
    for u in [
        "https://example.com/cb?token=abc",
        "https://example.com/cb?x=1&Access_Token=abc",
        "https://example.com/app#access_token=abc&scope=read",
        "https://example.com/dl?X-Amz-Signature=zzz&signature=abc",
        "https://example.com/?api_key=abc",
        "https://example.com/?key=abc",
    ] {
        assert!(has_query_secret(u), "{u}");
        let facts = new_pico_facts_for(u, false, false, false, None, false, false);
        assert_eq!(
            new_pico_plan(&facts),
            NewPicoPlan::NotOffered,
            "{u} must not ride a command line"
        );
    }
    for u in [
        "https://example.com/search?q=token",
        "https://example.com/?monkey=1&donkey=2",
        "https://example.com/page#section-2",
        "https://example.com/?keyboard=us",
    ] {
        assert!(!has_query_secret(u), "{u}");
    }
}

#[test]
fn tab_id_zero_is_refused_by_the_tab_parser() {
    // AUDIT 0.41.1 (Front E F5).
    use pico_core::core::tabcmd::{parse_tab_body, TabDeny};
    for body in [
        // AUDIT 0.41.3: the move body names its neighbour
        // (`before`) — the 0.41.1 body carried the gap
        // field `to`, which the parser no longer knows.
        r#"{"kind":"tab","action":"move","tab":0,"before":1}"#,
        r#"{"kind":"tab","action":"move-step","tab":0,"dir":-1}"#,
        r#"{"kind":"tab","action":"open-in-new-pico","tab":0}"#,
    ] {
        assert_eq!(parse_tab_body(body), Err(TabDeny::BadArgs), "{body}");
    }
}
