//! History state-machine scenarios and persisted-state hardening:
//! schema migration, corruption recovery, sanitization gates.

use pico_core::core::history::{TabHistory, MAX_ENTRIES};
use pico_core::core::state::{
    load_from_checked, push_recent, save_to, LoadOutcome, PersistState, WindowRect,
};
use std::path::PathBuf;

fn hist_with(urls: &[&str]) -> TabHistory {
    let mut h = TabHistory::new();
    for u in urls {
        h.record_commit(u);
    }
    h
}

// ---------------------------------------------------------------------------
// TabHistory
// ---------------------------------------------------------------------------

#[test]
fn commits_append_and_dedupe_consecutive() {
    let h = hist_with(&["http://a/", "http://a/", "http://b/", "http://b/"]);
    assert_eq!(h.entries(), ["http://a/", "http://b/"]);
    assert_eq!(h.current(), Some("http://b/"));
    assert!(h.can_go_back());
    assert!(!h.can_go_forward());
}

#[test]
fn back_forward_intents_land_on_commit() {
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    // Index has NOT moved yet — the intent is pending.
    assert_eq!(h.current(), Some("http://c/"));
    h.record_commit("http://b/");
    assert_eq!(h.current(), Some("http://b/"));
    assert_eq!(h.go_forward(0), Some("http://c/"));
    h.record_commit("http://c/");
    assert_eq!(h.current(), Some("http://c/"));
    assert!(!h.can_go_forward());
}

#[test]
fn redirect_during_traversal_replaces_destination() {
    let mut h = hist_with(&["http://a/", "http://b/"]);
    assert_eq!(h.go_back(0), Some("http://a/"));
    h.record_commit("http://a-redirected/");
    assert_eq!(h.current(), Some("http://a-redirected/"));
    assert_eq!(h.entries(), ["http://a-redirected/", "http://b/"]);
}

#[test]
fn spa_push_replace_pop() {
    let mut h = hist_with(&["http://a/"]);
    h.record_push("http://a/spa1");
    h.record_push("http://a/spa2");
    assert_eq!(h.entries(), ["http://a/", "http://a/spa1", "http://a/spa2"]);
    h.record_replace("http://a/spa2b");
    assert_eq!(h.current(), Some("http://a/spa2b"));
    h.record_pop("http://a/spa1");
    assert_eq!(h.current(), Some("http://a/spa1"));
    // A pop the user asked for consumes the pending intent.
    assert_eq!(h.go_back(0), Some("http://a/"));
    h.record_pop("http://a/");
    assert_eq!(h.current(), Some("http://a/"));
    // The later native commit for the same URL is a harmless no-op.
    h.record_commit("http://a/");
    assert_eq!(h.entries().len(), 3);
    assert_eq!(h.current(), Some("http://a/"));
}

#[test]
fn fresh_navigation_truncates_forward_branch() {
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    h.go_back(0);
    h.record_commit("http://b/");
    h.record_commit("http://d/");
    assert_eq!(h.entries(), ["http://a/", "http://b/", "http://d/"]);
    assert!(!h.can_go_forward());
}

#[test]
fn uninitiated_traversal_reconciles_as_move() {
    // Mouse back-button: no intent recorded, a commit for the
    // previous entry arrives — must move, not append.
    let mut h = hist_with(&["http://a/", "http://b/"]);
    h.record_commit("http://a/");
    assert_eq!(h.entries(), ["http://a/", "http://b/"]);
    assert_eq!(h.current(), Some("http://a/"));
    h.record_commit("http://b/");
    assert_eq!(h.current(), Some("http://b/"));
}

#[test]
fn pending_intent_cancelled_by_fresh_navigation() {
    let mut h = hist_with(&["http://a/", "http://b/"]);
    assert_eq!(h.go_back(0), Some("http://a/"));
    h.cancel_pending();
    h.record_commit("http://z/");
    assert_eq!(h.current(), Some("http://z/"));
    assert_eq!(h.entries(), ["http://a/", "http://b/", "http://z/"]);
}

#[test]
fn history_is_capped() {
    let mut h = TabHistory::new();
    for i in 0..(MAX_ENTRIES + 25) {
        h.record_push(&format!("http://a/{i}"));
    }
    assert_eq!(h.len(), MAX_ENTRIES);
    assert_eq!(h.current(), Some("http://a/124"));
    assert!(h.can_go_back());
}

#[test]
fn traversal_signal_moves_the_index_and_consumes_the_intent() {
    // AUDIT V28 A6-1: a SourceChanged landing on the pending
    // target arrives as record_traversal (the app used to drop
    // it as a no-op, stranding the index on the old entry and
    // leaving the intent dangling to corrupt the next commit).
    let mut h = hist_with(&["http://a/", "http://b/"]);
    assert_eq!(h.go_back(0), Some("http://a/"));
    h.record_traversal("http://a/");
    assert_eq!(h.current(), Some("http://a/"));
    assert!(!h.has_pending());
    assert!(h.can_go_forward(), "the forward entry survives");
    // The observer's late pop note for the same URL is a no-op.
    h.record_pop("http://a/");
    assert_eq!(h.entries(), ["http://a/", "http://b/"]);
    // And the next fresh navigation appends cleanly — no entry
    // is overwritten and no phantom forward entry appears.
    h.record_commit("http://d/");
    assert_eq!(h.entries(), ["http://a/", "http://d/"]);
}

#[test]
fn multi_step_pop_reconciles_to_the_matching_entry() {
    // AUDIT V28 A6-7: a coalesced double-Back lands two entries
    // away in a single pop signal; it must move, not push.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/", "http://d/"]);
    h.record_pop("http://b/");
    assert_eq!(h.current(), Some("http://b/"));
    assert_eq!(h.entries().len(), 4, "no truncation, no duplicate");
    assert!(h.can_go_forward());
    // Same for an uninitiated multi-step commit landing.
    h.record_commit("http://d/");
    assert_eq!(h.current(), Some("http://d/"));
    assert_eq!(h.entries().len(), 4);
}

#[test]
fn replace_drops_a_pending_intent() {
    // AUDIT V28 A6-7: replace rewrites the current entry; a
    // pending intent aimed from the old index must not survive
    // to reconcile a later landing against the wrong branch.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.record_replace("http://c2/");
    assert!(!h.has_pending());
    assert_eq!(h.pending_target(), None);
    assert_eq!(h.current(), Some("http://c2/"));
}

#[test]
fn v29_uninitiated_spa_traversal_source_first_reconciles_as_pop() {
    // A6-H1 (the v29 headline of this area): an SPA session
    // [a,b,c] sitting on c, Source already seen for the
    // document; the page traverses back to b on its own
    // (history.go, mouse buttons — no Pico intent). The
    // Source event must classify Pop (the app passes
    // in_history), NOT Push: v28 produced [a,b,c,b] with the
    // forward branch destroyed.
    use pico_core::core::urlstate::{HistEffect, UrlEvent, UrlState};
    let mut s = UrlState::with_url("https://example.com/a");
    let mut h = TabHistory::new();
    s.apply_landing(
        UrlEvent::CommitFinish("https://example.com/a".into()),
        None,
        false,
    );
    h.record_commit("https://example.com/a");
    for u in ["https://example.com/b", "https://example.com/c"] {
        let out = s.apply_landing(UrlEvent::Source(u.into()), None, false);
        assert_eq!(out.hist, HistEffect::Push);
        h.record_push(u);
    }
    assert_eq!(h.current(), Some("https://example.com/c"));
    // The uninitiated traversal lands on b: b is in history at
    // a non-current entry, so the app passes in_history=true.
    let out = s.apply_landing(UrlEvent::Source("https://example.com/b".into()), None, true);
    assert_eq!(
        out.hist,
        HistEffect::Pop,
        "a landing on an existing entry is a traversal"
    );
    h.record_pop("https://example.com/b");
    assert_eq!(
        h.entries(),
        [
            "https://example.com/a",
            "https://example.com/b",
            "https://example.com/c"
        ]
    );
    assert_eq!(h.current(), Some("https://example.com/b"));
    assert!(h.can_go_forward(), "the forward branch survives");
    // Control: a genuinely new URL still pushes.
    let out = s.apply_landing(
        UrlEvent::Source("https://example.com/new".into()),
        None,
        false,
    );
    assert_eq!(out.hist, HistEffect::Push);
}

#[test]
fn v29_redirected_traversal_reaches_record_traversal() {
    // A6-H1 adjacent: a Pico-initiated traversal whose landing
    // URL differs from the target (redirect/canonicalization)
    // still classifies Traversal while an intent is pending —
    // v28's exact-match rule classified it Push and
    // record_traversal's redirect branch was unreachable.
    use pico_core::core::urlstate::{HistEffect, UrlEvent, UrlState};
    let mut s = UrlState::with_url("https://example.com/a");
    let mut h = TabHistory::new();
    s.apply_landing(
        UrlEvent::CommitFinish("https://example.com/a".into()),
        None,
        false,
    );
    h.record_commit("https://example.com/a");
    s.apply_landing(
        UrlEvent::Source("https://example.com/b".into()),
        None,
        false,
    );
    h.record_push("https://example.com/b");
    assert_eq!(h.go_back(0), Some("https://example.com/a"));
    let target = h.pending_target().map(|t| t.to_string());
    // The traversal lands on a canonicalized form of a.
    let out = s.apply_landing(
        UrlEvent::Source("https://example.com/a/".into()),
        target.as_deref(),
        false,
    );
    assert_eq!(out.hist, HistEffect::Traversal);
    h.record_traversal("https://example.com/a/");
    assert_eq!(h.current(), Some("https://example.com/a/"));
    assert!(!h.has_pending());
    assert!(h.can_go_forward());
}

#[test]
fn v29_pending_commit_landing_reconciles_multi_step() {
    // A6-M2: [a,b,c], Pico asks Back (target b), but the
    // landing commit is a (the page coalesced its own
    // traversal into ours). v28 stepped one and replaced:
    // [a,a,c]. The landing search must win.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.record_commit("http://a/");
    assert_eq!(h.entries(), ["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.current(), Some("http://a/"));
    assert!(!h.has_pending());
    // The redirect shape still replaces the destination:
    // target b, landing on an unseen URL r.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.record_commit("http://r/");
    assert_eq!(h.entries(), ["http://a/", "http://r/", "http://c/"]);
    assert_eq!(h.current(), Some("http://r/"));
}

#[test]
fn v30_1_traversal_signal_reconciles_multi_step_landing() {
    // AUDIT 0.30.1 (A6 HIGH — the S1 trace from the corrective
    // report): [a,b,c] at c, Pico asks Back (target b), but
    // the page coalesces a 2-step traversal and the ONLY
    // signal is SourceChanged landing on a. record_commit got
    // the landing search in v29 (A6-M2); record_traversal did
    // not, and stepped one entry and REPLACED it: [a,a,c] at
    // index 1 — duplicates plus a phantom Back. SPA
    // same-document traversals produce no commit, so this
    // seam is the final word for exactly those sessions.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.record_traversal("http://a/");
    assert_eq!(
        h.entries(),
        ["http://a/", "http://b/", "http://c/"],
        "no entry is replaced, no duplicate appears"
    );
    assert_eq!(h.current(), Some("http://a/"));
    assert!(!h.has_pending(), "the intent is consumed");
    assert!(h.can_go_forward(), "the forward branch survives intact");
    // Forward multi-step through the same seam: from a, a
    // Forward intent (target b) landing straight on c.
    assert_eq!(h.go_forward(0), Some("http://b/"));
    h.record_traversal("http://c/");
    assert_eq!(h.entries(), ["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.current(), Some("http://c/"));
    assert!(!h.has_pending());
}

#[test]
fn v30_1_traversal_redirect_still_step_replaces() {
    // The S13 shape, preserved: with an intent in flight, a
    // landing on a URL NO entry holds is a redirect during
    // the traversal — the one-step destination is replaced
    // by the final URL, exactly as record_commit has always
    // done (the landing search must not swallow it).
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.record_traversal("http://r/");
    assert_eq!(h.entries(), ["http://a/", "http://r/", "http://c/"]);
    assert_eq!(h.current(), Some("http://r/"));
    assert!(!h.has_pending());
    // One-step exact landing still just moves (no replace).
    let mut h = hist_with(&["http://a/", "http://b/"]);
    assert_eq!(h.go_back(0), Some("http://a/"));
    h.record_traversal("http://a/");
    assert_eq!(h.entries(), ["http://a/", "http://b/"]);
    assert_eq!(h.current(), Some("http://a/"));
}

#[test]
fn v30_1_source_first_multi_step_traversal_keeps_history() {
    // The S1 trace composed through the reducer, the way the
    // app actually drives the machine: the Source landing is
    // classified Traversal while the intent is pending, and
    // record_traversal must reconcile it as a move.
    use pico_core::core::urlstate::{HistEffect, UrlEvent, UrlState};
    let mut s = UrlState::with_url("https://example.com/a");
    let mut h = TabHistory::new();
    s.apply_landing(
        UrlEvent::CommitFinish("https://example.com/a".into()),
        None,
        false,
    );
    h.record_commit("https://example.com/a");
    for u in ["https://example.com/b", "https://example.com/c"] {
        let out = s.apply_landing(UrlEvent::Source(u.into()), None, false);
        assert_eq!(out.hist, HistEffect::Push);
        h.record_push(u);
    }
    assert_eq!(h.go_back(0), Some("https://example.com/b"));
    let target = h.pending_target().map(|t| t.to_string());
    let out = s.apply_landing(
        UrlEvent::Source("https://example.com/a".into()),
        target.as_deref(),
        false,
    );
    assert_eq!(out.hist, HistEffect::Traversal);
    h.record_traversal("https://example.com/a");
    assert_eq!(
        h.entries(),
        [
            "https://example.com/a",
            "https://example.com/b",
            "https://example.com/c"
        ]
    );
    assert_eq!(h.current(), Some("https://example.com/a"));
    assert!(h.can_go_forward());
    assert!(!h.has_pending());
}

#[test]
fn v30_1_pending_intent_expires_only_after_the_ttl() {
    // AUDIT 0.30.1 (A6-MED residue): an intent whose landing
    // never signals must not wedge the tab forever — the
    // shell retires it at the re-click once it is older than
    // INTENT_TTL_MS and issues the click's own intent.
    use pico_core::core::history::INTENT_TTL_MS;
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(1_000), Some("http://b/"));
    assert!(!h.pending_expired(1_000), "a fresh intent is live");
    assert!(
        !h.pending_expired(1_000 + INTENT_TTL_MS),
        "exactly at the TTL is not yet expired (strictly greater)"
    );
    assert!(h.pending_expired(1_000 + INTENT_TTL_MS + 1));
    // The shell's move: cancel the dead intent, re-issue.
    h.cancel_pending();
    assert_eq!(h.go_back(9_000), Some("http://b/"));
    assert!(!h.pending_expired(9_000 + INTENT_TTL_MS));
    // A landing consumes the intent AND its stamp: nothing
    // can expire afterwards.
    h.record_traversal("http://b/");
    assert!(!h.has_pending());
    assert!(!h.pending_expired(u64::MAX / 2));
    // No intent at all: never "expired".
    assert!(!h.pending_expired(0));
}

#[test]
fn v29_replace_never_creates_consecutive_duplicates() {
    // A6-L6: [a,b,a] with the middle current, replaced by a —
    // v28 left [a,a].
    let mut h = hist_with(&["http://a/", "http://b/", "http://a/"]);
    // hist_with lands on the last entry; step back to b first.
    h.record_pop("http://b/");
    assert_eq!(h.current(), Some("http://b/"));
    h.record_replace("http://a/");
    assert_eq!(h.entries(), ["http://a/"]);
    assert_eq!(h.current(), Some("http://a/"));
}

#[test]
fn v31_commit_start_reconciles_coalesced_traversal_redirect() {
    // AUDIT 0.31 (A6-F1): [a,b,c] at c, Pico asks Back
    // (intent target b), but the coalesced traversal's load
    // STARTS at a (an entry) and finishes at a′ — a redirect
    // of a that no entry holds. The remembered commit-start
    // identifies a's slot as the one the finish replaces:
    // [a′,b,c] at index 0 — NOT the step-replace [a,a′,c]
    // the pending branches produced through 0.30.1.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.note_commit_start("http://a/");
    h.record_commit("http://a-redirected/");
    assert_eq!(
        h.entries(),
        ["http://a-redirected/", "http://b/", "http://c/"]
    );
    assert_eq!(h.index(), Some(0));
    assert_eq!(h.current(), Some("http://a-redirected/"));
    assert!(!h.has_pending(), "the intent is consumed");
    assert!(h.can_go_forward(), "the forward branch survives");
    // The remembered start was taken by the commit: a later
    // unrelated commit is not reconciled against it.
    h.record_commit("http://z/");
    assert_eq!(
        h.entries(),
        ["http://a-redirected/", "http://z/"],
        "a fresh navigation from a′ truncates and appends normally"
    );
}

#[test]
fn v31_commit_start_equal_to_finish_is_a_normal_traversal_commit() {
    // AUDIT 0.31 (A6-F1, variant): the load starts AND
    // finishes at the intent's target — the remembered
    // start equals the commit URL, the new branch is
    // skipped, and the existing pending landing path moves
    // the index exactly as before.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.note_commit_start("http://b/");
    h.record_commit("http://b/");
    assert_eq!(h.entries(), ["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.current(), Some("http://b/"));
    assert!(!h.has_pending());
    // An empty commit-start note is ignored, and a
    // same-document signal supersedes a remembered start:
    // after record_push, a commit reconciles without it.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    h.note_commit_start("");
    assert_eq!(h.go_back(0), Some("http://b/"));
    h.note_commit_start("http://a/");
    h.record_push("http://a/spa");
    assert!(!h.has_pending(), "the push drops the intent too");
    h.record_commit("http://a-redirected/");
    assert_eq!(
        h.entries(),
        [
            "http://a/",
            "http://b/",
            "http://c/",
            "http://a/spa",
            "http://a-redirected/"
        ],
        "with the start superseded, the commit is a plain fresh navigation"
    );
}

#[test]
fn v31_same_url_pop_consumes_a_pending_intent() {
    // AUDIT 0.31 (A6-F5): a pop landing on the CURRENT
    // entry's URL (hash-only noise) must clear a pending
    // intent like both sibling branches of record_pop do —
    // through 0.30.1 the early return left it dangling.
    let mut h = hist_with(&["http://a/", "http://b/", "http://c/"]);
    assert_eq!(h.go_back(0), Some("http://b/"));
    assert!(h.has_pending());
    h.record_pop("http://c/");
    assert!(
        !h.has_pending(),
        "the same-URL pop consumes the pending intent"
    );
    assert_eq!(h.pending_target(), None);
    assert_eq!(h.current(), Some("http://c/"));
    assert_eq!(h.entries(), ["http://a/", "http://b/", "http://c/"]);
}

#[test]
fn empty_history_is_inert() {
    let mut h = TabHistory::new();
    assert_eq!(h.current(), None);
    assert_eq!(h.go_back(0), None);
    assert_eq!(h.go_forward(0), None);
    assert!(!h.can_go_back() && !h.can_go_forward());
    h.record_commit("");
    assert!(h.is_empty());
}

// ---------------------------------------------------------------------------
// PersistState
// ---------------------------------------------------------------------------

fn temp_state_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pico-v15-state-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn state_roundtrips_through_disk() {
    let dir = temp_state_dir("roundtrip");
    let path = dir.join("state.json");
    let s = PersistState {
        theme: "dark".to_string(),
        window: Some(WindowRect {
            x: 10,
            y: 20,
            w: 1000,
            h: 700,
            maximized: false,
        }),
        ..PersistState::default()
    };
    save_to(&path, &s).expect("save");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert_eq!(loaded.theme, "dark");
            assert_eq!(loaded.version, 3, "schema v3 stamped");
            let w = loaded.window.expect("window persisted");
            assert_eq!((w.x, w.y, w.w, w.h), (10, 20, 1000, 700));
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v1_state_file_migrates_to_v2() {
    // A v12-era file: no `version`, no `window`.
    let dir = temp_state_dir("v1");
    let path = dir.join("state.json");
    std::fs::write(
        &path,
        r#"{"theme":"dark","recents":["/tmp/a.html","/tmp/b.html"]}"#,
    )
    .expect("write v1 file");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert_eq!(loaded.version, 3);
            assert_eq!(loaded.theme, "dark");
            assert_eq!(loaded.recents.len(), 2);
            assert!(loaded.window.is_none());
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v2_state_file_migrates_to_v3_with_shield_defaults() {
    // A v20-era file: schema v2, no Shield fields. Loading must
    // default the Shield ON with a zero lifetime count, keep the
    // rest, and re-stamp v3 on the next save.
    let dir = temp_state_dir("v2tov3");
    let path = dir.join("state.json");
    std::fs::write(
        &path,
        r#"{"version":2,"theme":"light","recents":["/tmp/a.html"],
            "window":{"x":1,"y":2,"w":900,"h":600,"maximized":false}}"#,
    )
    .expect("write v2 file");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert_eq!(loaded.version, 3);
            assert_eq!(loaded.theme, "light");
            assert!(loaded.shield_on, "shield defaults ON after migration");
            assert_eq!(loaded.blocked_total, 0);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    // And a v3 file with the fields set round-trips them.
    let s = PersistState {
        shield_on: false,
        blocked_total: 4321,
        ..PersistState::default()
    };
    save_to(&path, &s).expect("save v3");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert!(!loaded.shield_on);
            assert_eq!(loaded.blocked_total, 4321);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn v3_state_file_loads_with_recents_visible() {
    // A VERBATIM v3 file as v21–v23 wrote it: no
    // `recents_hidden` key anywhere. v24 rides the field on
    // schema v3 via the serde default (AUDIT V24-2) — the file
    // must load with the section visible, schema untouched.
    let dir = temp_state_dir("v3verbatim");
    let path = dir.join("state.json");
    std::fs::write(
        &path,
        r#"{"version":3,"theme":"dark","recents":["/tmp/a.html","/tmp/b.html"],
            "window":null,"shield_on":false,"blocked_total":77}"#,
    )
    .expect("write v3 file");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert_eq!(loaded.version, 3, "schema stays 3");
            assert!(!loaded.recents_hidden, "recents default to visible");
            assert_eq!(loaded.recents.len(), 2);
            assert!(!loaded.shield_on);
            assert_eq!(loaded.blocked_total, 77);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recents_hidden_roundtrips_through_disk() {
    let dir = temp_state_dir("hidden");
    let path = dir.join("state.json");
    let s = PersistState {
        recents_hidden: true,
        ..PersistState::default()
    };
    save_to(&path, &s).expect("save");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert!(loaded.recents_hidden, "the hidden flag must persist");
            assert_eq!(loaded.version, 3);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    // And the written JSON really carries the key (a v23 build
    // reading this file just ignores it — downgrade-safe).
    let text = std::fs::read_to_string(&path).expect("read back");
    assert!(text.contains("\"recents_hidden\": true"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_state_recovers_to_defaults() {
    let dir = temp_state_dir("corrupt");
    let path = dir.join("state.json");
    std::fs::write(&path, "{not json at all").expect("write corrupt");
    match load_from_checked(&path) {
        LoadOutcome::Corrupt(why) => assert!(why.contains("invalid JSON")),
        other => panic!("expected Corrupt, got {other:?}"),
    }
    // The file is left untouched (a human can inspect/repair it).
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json at all");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn oversized_state_file_is_corrupt_not_loaded() {
    let dir = temp_state_dir("oversized");
    let path = dir.join("state.json");
    let big = format!(
        "{{\"theme\":\"dark\",\"pad\":\"{}\"}}",
        "x".repeat(1_100_000)
    );
    std::fs::write(&path, big).expect("write oversized");
    match load_from_checked(&path) {
        LoadOutcome::Corrupt(why) => assert!(why.contains("implausibly large")),
        other => panic!("expected Corrupt, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_state_is_missing_not_corrupt() {
    let dir = temp_state_dir("missing");
    let path = dir.join("state.json");
    assert_eq!(load_from_checked(&path), LoadOutcome::Missing);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sanitize_drops_hostile_recents_and_bad_theme() {
    let s = PersistState {
        theme: "solarized".to_string(),
        recents: vec![
            "/ok/a.html".to_string(),
            "\\\\evil\\share\\x.html".to_string(), // UNC: must go
            "/ok/a.html".to_string(),              // duplicate: deduped
            "bad\u{7}path".to_string(),            // control char: must go
            "x".repeat(5000),                      // absurd length: must go
        ],
        ..PersistState::default()
    };
    let s = s.sanitized();
    assert_eq!(s.theme, "light");
    assert_eq!(s.recents, vec!["/ok/a.html".to_string()]);
}

#[test]
fn window_rect_validation_bounds() {
    assert!(WindowRect {
        x: 0,
        y: 0,
        w: 760,
        h: 500,
        maximized: false
    }
    .is_valid());
    for bad in [
        WindowRect {
            x: 0,
            y: 0,
            w: 100,
            h: 500,
            maximized: false,
        }, // too narrow
        WindowRect {
            x: 0,
            y: 0,
            w: 760,
            h: 100,
            maximized: false,
        }, // too short
        WindowRect {
            x: 0,
            y: 0,
            w: 99999,
            h: 500,
            maximized: false,
        }, // absurd size
        WindowRect {
            x: 99999,
            y: 0,
            w: 760,
            h: 500,
            maximized: false,
        }, // off-world
    ] {
        assert!(!bad.is_valid(), "{bad:?} must be invalid");
    }
    // An invalid persisted rect is dropped by sanitize, never trusted.
    let s = PersistState {
        window: Some(WindowRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            maximized: false,
        }),
        ..PersistState::default()
    };
    assert!(s.sanitized().window.is_none());
}

#[test]
fn push_recent_gates_and_caps() {
    let mut s = PersistState::default();
    push_recent(&mut s, "/a/one.html");
    push_recent(&mut s, "/a/two.html");
    push_recent(&mut s, "/a/one.html"); // re-open moves to front
    assert_eq!(s.recents, vec!["/a/one.html", "/a/two.html"]);
    push_recent(&mut s, "\\\\server\\share\\x.html"); // UNC refused
    assert_eq!(s.recents.len(), 2);
    for i in 0..40 {
        push_recent(&mut s, &format!("/a/f{i}.html"));
    }
    assert_eq!(s.recents.len(), 10, "recents capped at MAX_RECENTS");
}

#[test]
fn relaunch_preserves_theme_shield_recents_and_window() {
    // The state-level half of the remember-me promise (AUDIT
    // v22 Phase 2c): what one launch saves, the next launch
    // loads — theme, the Shield toggle, the lifetime blocked
    // count, recents, and window geometry all survive the
    // round trip through disk unchanged.
    let dir = temp_state_dir("relaunch");
    let path = dir.join("state.json");
    let s = PersistState {
        theme: "dark".to_string(),
        shield_on: false, // user turned the Shield off: it stays off
        blocked_total: 4321,
        recents: vec![
            "C:\\work\\site\\index.html".to_string(),
            "D:\\notes\\page.htm".to_string(),
        ],
        window: Some(WindowRect {
            x: 120,
            y: 90,
            w: 1280,
            h: 800,
            maximized: true,
        }),
        ..PersistState::default()
    };
    save_to(&path, &s).expect("save like a closing launch");
    match load_from_checked(&path) {
        LoadOutcome::Loaded(loaded) => {
            assert_eq!(loaded.theme, "dark");
            assert!(!loaded.shield_on, "shield toggle must persist");
            assert_eq!(loaded.blocked_total, 4321);
            assert_eq!(loaded.recents, s.recents);
            assert_eq!(loaded.window, s.window);
        }
        other => panic!("expected Loaded after a clean save, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
