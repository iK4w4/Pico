//! Tab visibility sequencing (0.41.3) — the pure plan behind
//! the shell's `set_visible` calls.
//!
//! The witnessed defect: switching, opening or closing a tab
//! flashed a black/blank frame for about one frame on
//! Windows. WebView2 controllers compose asynchronously: a
//! `set_visible(false)` stops the old controller's
//! presentation at once, while a newly shown controller
//! needs a beat before it presents its (retained) frame —
//! and a controller that has never been shown has no frame
//! to present at all, because controllers do not compose
//! while invisible. In the gap the content region shows the
//! tao window background (the theme surface; effectively
//! black in a dark theme). Three shell shapes fed the gap:
//!
//! 1. `activate` flipped visibility in a single pass over
//!    the tab vec, so a target sitting AFTER the current tab
//!    was shown only after the current view had already been
//!    hidden (hide-before-show; switching right flashed,
//!    switching left could escape it by vec order luck).
//! 2. `close_tab` removed and dropped the active view
//!    BEFORE activating its successor (drop-before-activate:
//!    a guaranteed empty frame on every active-tab close).
//! 3. A fresh tab's view was shown by that same single pass
//!    although its controller had never composed a frame
//!    (fresh-controller first frame lag).
//!
//! The fix is sequencing, decided here as a pure plan and
//! executed by the thin shell (`src/app.rs`):
//!
//! - An already-painted target is shown NOW, and every other
//!   view is retired DEFERRED — the hide is posted through
//!   the event-loop proxy and runs on a later turn, after
//!   the target's compositor has had its beat to present
//!   the retained frame.
//! - A fresh target (view exists, first load not yet
//!   started) is neither shown nor does it retire anyone:
//!   it is marked reveal-pending, and the previous view
//!   stays on screen until the fresh tab's first page-load
//!   event reveals it (see [`reveal_plan`]).
//! - A target with NO view (creation failed) retires the
//!   others IMMEDIATELY: there is nothing to present, the
//!   themed window background is the honest surface, and it
//!   matches the failed-tab behavior the shell has always
//!   had.
//!
//! This module decides; it never touches a WebView.

/// What the shell knows about one tab's view, for planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabViewFact {
    pub id: u64,
    /// The view's paint state (see [`PaintState`]).
    pub paint: PaintState,
}

/// A view's relationship to a presented frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintState {
    /// View creation failed: the tab exists, there is no
    /// view and nothing can ever be shown for it (until a
    /// retry builds one — a retry-built view is `Fresh`).
    NoView,
    /// The view exists but its first load has not started:
    /// the controller has never composed, so showing it now
    /// would present only its (themed) background while it
    /// catches up. It must wait for its first load event.
    Fresh,
    /// The view has been shown / its load has started: the
    /// compositor holds (or is producing) a frame for it,
    /// so it can be shown immediately on activation.
    Painted,
}

/// One ordered step of a visibility plan. The shell executes
/// the steps in order; `RetireOthersDeferred` is the only
/// step that leaves the current event-loop turn (it is
/// posted through the proxy), which is exactly the point:
/// the hide must land after the show has presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisStep {
    /// Show this tab's view now (it has a frame to present).
    ShowNow(u64),
    /// Do not show this tab's view yet and do not hide the
    /// currently visible one: mark the tab reveal-pending;
    /// its first page-load event reveals it via
    /// [`reveal_plan`].
    RevealOnFirstLoad(u64),
    /// Hide every other tab's view, but on a LATER
    /// event-loop turn (posted through the proxy). The
    /// retire arm re-checks that the excepted tab is still
    /// the active one before hiding anything.
    RetireOthersDeferred,
    /// Hide every other tab's view right now. Only for a
    /// target that can present nothing (no view): waiting
    /// would strand the previous page under a tab strip
    /// that says the failed tab is active.
    RetireOthersImmediate,
}

/// The activation plan for `target`, from the tabs' view
/// facts in tab-vec order. The plan is order-independent by
/// construction — that is the point: through 0.41.2 the
/// outcome depended on whether the target sat before or
/// after the current tab in the vec.
///
/// - Target not in `facts`: no steps (the shell refuses
///   unknown tabs before planning; the empty plan keeps
///   this function total).
/// - Target has no view: retire the others immediately.
/// - Target is fresh: reveal-pending only — the currently
///   visible view (if any) stays up until the reveal.
/// - Target is painted: show it now, retire the others on
///   a later turn.
pub fn activation_plan(facts: &[TabViewFact], target: u64) -> Vec<VisStep> {
    let Some(fact) = facts.iter().find(|f| f.id == target) else {
        return Vec::new();
    };
    match fact.paint {
        PaintState::NoView => vec![VisStep::RetireOthersImmediate],
        PaintState::Fresh => vec![VisStep::RevealOnFirstLoad(target)],
        PaintState::Painted => vec![VisStep::ShowNow(target), VisStep::RetireOthersDeferred],
    }
}

/// The reveal plan for a reveal-pending tab whose first
/// load event (Started or Finished), or whose navigation
/// failure resolution, has arrived.
///
/// - The tab is the ACTIVE tab and has a view: show it now
///   and retire the others deferred — the same shape as a
///   painted activation. Deferred, not immediate, for the
///   same reason: at load Started the fresh controller has
///   been told to present its themed background and first
///   paint, but has not necessarily presented yet; the old
///   view retiring one turn later covers that beat, and
///   the worst case in between is two views stacked with
///   the fresh one on its correct bounds (layout ran at
///   activation), never an empty region.
/// - The tab is NOT active (its load started while it sat
///   in the background — it was switched away from while
///   pending, or was never activated): no steps. The tab
///   stops being reveal-pending (the shell clears the
///   flag) and is an ordinary background tab from here:
///   a later activation shows it under
///   [`activation_plan`]'s painted arm. Its load has
///   started, so it is no longer `Fresh` by the time any
///   later plan is made.
/// - The tab has no view or is unknown: no steps (a failed
///   tab's activation already retired the others
///   immediately; there is nothing to reveal).
pub fn reveal_plan(facts: &[TabViewFact], tab: u64, is_active: bool) -> Vec<VisStep> {
    if !is_active {
        return Vec::new();
    }
    let Some(fact) = facts.iter().find(|f| f.id == tab) else {
        return Vec::new();
    };
    match fact.paint {
        PaintState::Fresh | PaintState::Painted => {
            vec![VisStep::ShowNow(tab), VisStep::RetireOthersDeferred]
        }
        PaintState::NoView => Vec::new(),
    }
}

/// The successor an ACTIVE tab's close activates, from the
/// tab ids in vec order and the closing tab's id: the tab
/// that slides into the closing tab's index (its right
/// neighbour), or — when the closing tab was last — the
/// new last tab (its left neighbour). `None` when the
/// closing tab is unknown or is the only tab (the caller
/// creates the replacement start tab in that case, then
/// activates THAT under the fresh-tab rules). This is
/// exactly the pre-0.41.3 shell arithmetic
/// (`tabs[idx.min(len - 1)]` after removal), stated purely
/// so the close path can ask BEFORE anything is removed.
pub fn successor_after_close(ids: &[u64], closing: u64) -> Option<u64> {
    let idx = ids.iter().position(|&id| id == closing)?;
    let remaining: Vec<u64> = ids.iter().copied().filter(|&id| id != closing).collect();
    if remaining.is_empty() {
        return None;
    }
    Some(remaining[idx.min(remaining.len() - 1)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: u64, paint: PaintState) -> TabViewFact {
        TabViewFact { id, paint }
    }

    fn painted(ids: &[u64]) -> Vec<TabViewFact> {
        ids.iter()
            .map(|&id| fact(id, PaintState::Painted))
            .collect()
    }

    #[test]
    fn switch_right_shows_before_any_retire() {
        // Current tab 1, target 3 sits AFTER it in the vec —
        // the order that flashed through 0.41.2. The plan
        // shows the target first and defers every hide.
        let facts = painted(&[1, 2, 3]);
        assert_eq!(
            activation_plan(&facts, 3),
            vec![VisStep::ShowNow(3), VisStep::RetireOthersDeferred]
        );
    }

    #[test]
    fn switch_left_plan_is_identical_in_shape() {
        // Vec order must not matter: switching left produces
        // the same show-then-deferred-retire shape.
        let facts = painted(&[1, 2, 3]);
        assert_eq!(
            activation_plan(&facts, 1),
            vec![VisStep::ShowNow(1), VisStep::RetireOthersDeferred]
        );
    }

    #[test]
    fn fresh_tab_is_reveal_pending_and_retires_nobody() {
        // A fresh target (just built, first load not yet
        // started): no show, no retire — the old view stays
        // on screen until the reveal.
        let mut facts = painted(&[1, 2]);
        facts.push(fact(3, PaintState::Fresh));
        assert_eq!(
            activation_plan(&facts, 3),
            vec![VisStep::RevealOnFirstLoad(3)]
        );
    }

    #[test]
    fn failed_tab_retires_others_immediately() {
        // No view: nothing can be presented, so the others
        // retire at once (the themed window background is
        // the surface), matching the historical failed-tab
        // behavior.
        let mut facts = painted(&[1, 2]);
        facts.push(fact(3, PaintState::NoView));
        assert_eq!(
            activation_plan(&facts, 3),
            vec![VisStep::RetireOthersImmediate]
        );
    }

    #[test]
    fn unknown_target_plans_nothing() {
        assert!(activation_plan(&painted(&[1, 2]), 99).is_empty());
        assert!(activation_plan(&[], 1).is_empty());
    }

    #[test]
    fn reveal_of_the_active_fresh_tab_shows_and_defers_retire() {
        let facts = vec![fact(1, PaintState::Painted), fact(2, PaintState::Fresh)];
        assert_eq!(
            reveal_plan(&facts, 2, true),
            vec![VisStep::ShowNow(2), VisStep::RetireOthersDeferred]
        );
    }

    #[test]
    fn reveal_in_the_background_shows_nothing() {
        // The pending tab's load started after the user had
        // already switched away: it must NOT pop visible
        // over the tab the user is looking at.
        let facts = vec![fact(1, PaintState::Painted), fact(2, PaintState::Fresh)];
        assert!(reveal_plan(&facts, 2, false).is_empty());
    }

    #[test]
    fn reveal_of_a_failed_or_unknown_tab_shows_nothing() {
        let facts = vec![fact(1, PaintState::Painted), fact(2, PaintState::NoView)];
        assert!(reveal_plan(&facts, 2, true).is_empty());
        assert!(reveal_plan(&facts, 99, true).is_empty());
    }

    #[test]
    fn close_successor_is_the_right_neighbour_when_there_is_one() {
        assert_eq!(successor_after_close(&[1, 2, 3], 1), Some(2));
        assert_eq!(successor_after_close(&[1, 2, 3], 2), Some(3));
    }

    #[test]
    fn close_successor_falls_back_to_the_left_neighbour_at_the_end() {
        assert_eq!(successor_after_close(&[1, 2, 3], 3), Some(2));
        assert_eq!(successor_after_close(&[1, 2], 2), Some(1));
    }

    #[test]
    fn close_successor_of_the_only_tab_is_none() {
        // The caller creates the replacement start tab
        // first and activates that instead.
        assert_eq!(successor_after_close(&[1], 1), None);
        assert_eq!(successor_after_close(&[1, 2], 99), None);
        assert_eq!(successor_after_close(&[], 1), None);
    }

    #[test]
    fn close_successor_cases_feed_activation_plans() {
        // Closing the active middle tab: the successor is
        // painted, so its activation is show-then-defer —
        // and crucially the plan is computed while the
        // closing tab still exists in the facts.
        let facts = painted(&[1, 2, 3]);
        let next = successor_after_close(&[1, 2, 3], 2).expect("successor");
        assert_eq!(
            activation_plan(&facts, next),
            vec![VisStep::ShowNow(3), VisStep::RetireOthersDeferred]
        );
        // Closing onto a FAILED successor: its activation
        // retires the others (the closing view included)
        // immediately — no deferred step can strand it.
        let facts = vec![
            fact(1, PaintState::Painted),
            fact(2, PaintState::Painted),
            fact(3, PaintState::NoView),
        ];
        let next = successor_after_close(&[1, 2, 3], 2).expect("successor");
        assert_eq!(
            activation_plan(&facts, next),
            vec![VisStep::RetireOthersImmediate]
        );
    }
}
