//! Tab visibility sequencing (0.41.3) — the black-frame fix.
//!
//! The pure plan lives in `core::visibility` (unit-tested
//! there); this suite pins the SHELL wiring in `src/app.rs`
//! by source scan (the portable_data.rs builder-census
//! pattern), because the shell only compiles for Windows:
//! the sequencing shapes that caused the flash must not be
//! able to creep back in without a test going red.

use pico_core::core::visibility::{
    activation_plan, reveal_plan, successor_after_close, PaintState, TabViewFact, VisStep,
};

fn app_src() -> String {
    std::fs::read_to_string(format!("{}/src/app.rs", env!("CARGO_MANIFEST_DIR")))
        .expect("read app.rs")
}

/// The slice of app.rs between two markers (first occurrence
/// of each), for function-scoped pins.
fn slice_between<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
    let a = src.find(from).unwrap_or_else(|| panic!("marker {from:?}"));
    let b = src[a..]
        .find(to)
        .unwrap_or_else(|| panic!("marker {to:?} after {from:?}"))
        + a;
    &src[a..b]
}

fn fact(id: u64, paint: PaintState) -> TabViewFact {
    TabViewFact { id, paint }
}

// ---------------------------------------------------------------------------
// The plan, through the public API
// ---------------------------------------------------------------------------

#[test]
fn public_plan_switch_both_directions_show_first_and_defer_the_hide() {
    let facts: Vec<TabViewFact> = [1, 2, 3]
        .iter()
        .map(|&id| fact(id, PaintState::Painted))
        .collect();
    for target in [1, 3] {
        assert_eq!(
            activation_plan(&facts, target),
            vec![VisStep::ShowNow(target), VisStep::RetireOthersDeferred],
            "switching to {target} must show first and defer the hide"
        );
    }
}

#[test]
fn public_plan_fresh_failed_and_reveal_shapes() {
    let facts = vec![
        fact(1, PaintState::Painted),
        fact(2, PaintState::Fresh),
        fact(3, PaintState::NoView),
    ];
    assert_eq!(
        activation_plan(&facts, 2),
        vec![VisStep::RevealOnFirstLoad(2)],
        "a fresh tab is reveal-pending: no show, no retire"
    );
    assert_eq!(
        activation_plan(&facts, 3),
        vec![VisStep::RetireOthersImmediate],
        "a failed tab retires the others at once"
    );
    assert_eq!(
        reveal_plan(&facts, 2, true),
        vec![VisStep::ShowNow(2), VisStep::RetireOthersDeferred]
    );
    assert!(reveal_plan(&facts, 2, false).is_empty());
    assert_eq!(successor_after_close(&[1, 2, 3], 2), Some(3));
    assert_eq!(successor_after_close(&[1, 2, 3], 3), Some(2));
    assert_eq!(successor_after_close(&[1], 1), None);
}

// ---------------------------------------------------------------------------
// Source-scan pins over the shell
// ---------------------------------------------------------------------------

#[test]
fn pin_no_single_pass_visibility_loop_remains_in_activate() {
    let src = app_src();
    assert!(
        !src.contains("set_visible(t.id =="),
        "the 0.41.2 single-pass hide/show loop must not exist anywhere in the shell"
    );
    let activate = slice_between(
        &src,
        "    fn activate(&mut self",
        "    fn close_tab(&mut self",
    );
    assert!(
        activate.contains("visibility::activation_plan"),
        "activate must take its sequencing from the pure plan"
    );
    assert!(
        activate.contains("self.exec_vis_plan(&plan, id)"),
        "activate must execute the plan through the one executor"
    );
    // Layout before presentation: the layout pass precedes
    // the plan execution inside activate.
    let layout_at = activate.find("self.layout();").expect("layout in activate");
    let exec_at = activate
        .find("self.exec_vis_plan(&plan, id)")
        .expect("exec in activate");
    assert!(
        layout_at < exec_at,
        "activate must lay out (bounds correct) before it shows anything"
    );
}

#[test]
fn pin_set_visible_has_exactly_three_call_sites() {
    let src = app_src();
    // One hide (the shared hide_views_except loop), one show
    // (the plan executor's ShowNow arm), one window show at
    // startup. Any new direct set_visible on a content view
    // bypasses the plan and can reintroduce the flash.
    assert_eq!(
        src.matches("set_visible(false)").count(),
        1,
        "exactly one content-view hide site (hide_views_except)"
    );
    assert_eq!(
        src.matches("set_visible(true)").count(),
        2,
        "exactly two show sites: the ShowNow arm and the startup window show"
    );
    assert!(
        src.contains("fn hide_views_except(&mut self, except: u64)"),
        "the hide loop is the shared helper"
    );
    assert!(
        src.contains("fn exec_vis_plan(&mut self, plan: &[VisStep], target: u64)"),
        "the plan executor exists"
    );
}

#[test]
fn pin_close_tab_activates_before_it_removes_in_the_active_paths() {
    let src = app_src();
    let close = slice_between(
        &src,
        "    fn close_tab(&mut self, id: u64)",
        "    fn cycle_tab(",
    );
    let remove_at = close
        .find("self.tabs.remove(idx)")
        .expect("close_tab removes the record exactly once, after activation");
    assert_eq!(
        close.matches("self.tabs.remove(idx)").count(),
        1,
        "close_tab must have exactly one removal site, after the activation"
    );
    let activate_next = close
        .find("self.activate(next, true)")
        .expect("the successor activation");
    assert!(
        activate_next < remove_at,
        "the successor must be activated BEFORE the closing tab is removed/dropped"
    );
    assert!(
        close.contains("visibility::successor_after_close"),
        "the successor comes from the pure plan, computed on the pre-removal vec"
    );
    // The last-tab path: the replacement is created
    // un-activated and then activated under the new rules,
    // both before the removal.
    let create_at = close
        .find("self.add_tab(START_URL, true, false)")
        .expect("last-tab replacement is created without the old activation");
    let activate_new = close
        .find("self.activate(new_id, false)")
        .expect("last-tab replacement activation");
    assert!(create_at < activate_new && activate_new < remove_at);
    // The pending-visit flush still precedes the removal.
    let flush_at = close
        .find("pending_visit.flush_all()")
        .expect("visit flush");
    assert!(
        flush_at < remove_at,
        "the visit flush precedes the record going"
    );
}

#[test]
fn pin_content_builder_keeps_the_themed_background_and_starts_hidden() {
    let src = app_src();
    let build = slice_between(
        &src,
        "    fn build_content_view(",
        "    fn add_tab(&mut self",
    );
    assert!(
        build.contains(".with_background_color(theme_bg_rgba("),
        "every content view keeps its explicit themed background (AUDIT V14-2)"
    );
    assert!(
        build.contains(".with_visible(false)"),
        "every content view is still built hidden"
    );
    // The chrome trap (AUDIT V16-2) stays shut: the chrome
    // builder is transparent and nothing backgrounds it.
    let chrome = slice_between(&src, "fn build_chrome(", "pub fn run()");
    assert!(chrome.contains(".with_transparent(true)"));
    assert!(
        !chrome.contains("with_background_color"),
        "the chrome view must never get a background color (it would kill the feather)"
    );
}

#[test]
fn pin_retire_and_reveal_wiring_exists() {
    let src = app_src();
    assert!(
        src.contains("RetireViews {")
            && src.contains("UserEvent::RetireViews { except } => app.retire_views(except),"),
        "the deferred retire event and its arm both exist"
    );
    assert!(
        src.contains("fn retire_views(&mut self, except: u64)"),
        "the retire arm exists"
    );
    let retire = slice_between(
        &src,
        "    fn retire_views(&mut self",
        "    /// Reveal a reveal-pending tab",
    );
    assert!(
        retire.contains("if self.active != except"),
        "the retire arm must refuse a superseded activation's retire"
    );
    assert!(
        src.contains("fn reveal_pending_tab(&mut self, id: u64)"),
        "the reveal path exists"
    );
    assert!(
        src.contains("visibility::reveal_plan"),
        "the reveal is planned by core, not improvised in the shell"
    );
    let on_load = slice_between(
        &src,
        "    fn on_page_load(&mut self",
        "/// Watch a root folder",
    );
    assert!(
        on_load.contains("self.reveal_pending_tab(tab_id);"),
        "on_page_load resolves the reveal (Started and Finished alike)"
    );
    // The NavigationSettled arm is the failure-path
    // resolution (a refused navigation has no page-load
    // event): the arm must call the reveal too.
    assert!(
        src.contains("app.reveal_pending_tab(tab);"),
        "the NavigationSettled arm resolves a refused navigation's reveal"
    );
    // Freshness is recorded at both build sites.
    assert!(
        src.contains("reveal_pending: !failed_flag,"),
        "add_tab marks a successfully built view fresh"
    );
    assert!(
        src.contains("tab.reveal_pending = true;"),
        "retry_tab_view marks a rebuilt view fresh"
    );
}
