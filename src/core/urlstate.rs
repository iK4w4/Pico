//! Per-tab URL state — the single source of truth (AUDIT V26,
//! defect D7).
//!
//! v25 kept a tab's URL in a plain field written by exactly two
//! paths: wry page-load events (full navigation commits only)
//! and the injected observer script's same-document notes. That
//! composition had a fatal wedge, proven from the code:
//!
//! 1. WebView2's own live signal — `ICoreWebView2` SourceChanged
//!    — was never wired, so pushState/replaceState navigations
//!    (every SPA, YouTube included) reached Pico only through
//!    the observer script.
//! 2. Observer notes were accepted only when they stayed in the
//!    *stored* URL's origin (`on_nav_note`'s same-origin gate).
//!    Sites that flip host form mid-session (YouTube's router
//!    canonicalizes `youtube.com/@…` ↔ `www.youtube.com/@…` via
//!    replaceState) made every later note "cross-origin" against
//!    the frozen stored URL — so every later note was dropped,
//!    forever, and the address bar, the tab state and Teleport
//!    all froze at the pre-flip URL while the page played on.
//! 3. The observer itself bails permanently at document start
//!    when `window.ipc` is not yet defined — no retry.
//!
//! v26 makes the platform the authority: SourceChanged events
//! (wired in the app layer for every content WebView) feed this
//! reducer, commits feed it, and observer notes are demoted to a
//! classification hint that can never roll the URL back once a
//! Source event has been seen for the current document. All
//! three consumers — the address bar (via the snapshot), the tab
//! record, and Teleport — read the one value this reducer owns.
//!
//! The reducer is pure: the app layer supplies events in arrival
//! order (the event loop is single-threaded, so arrival order IS
//! recency order) plus the history machine's pending traversal
//! target, and applies the returned effects.

use crate::core::ipc::NavHow;

/// One URL-relevant happening for a tab, in arrival order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlEvent {
    /// A full navigation started (wry page-load Started with a
    /// real URL).
    CommitStart(String),
    /// A full navigation finished (wry page-load Finished).
    CommitFinish(String),
    /// SourceChanged: the WebView's live `Source` property.
    /// Authoritative — never origin-gated.
    Source(String),
    /// An observer-script note (same-document hint).
    Note { how: NavHow, href: String },
}

/// What the app layer should do to the tab's history machine
/// after applying an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistEffect {
    /// Nothing to record.
    None,
    /// A full document committed — `record_commit`.
    Commit,
    /// A same-document entry was pushed — `record_push`.
    Push,
    /// A same-document entry was replaced — `record_replace`.
    Replace,
    /// A traversal (back/forward) landed — `record_pop`.
    Pop,
    /// A Source change matched the pending traversal target —
    /// the traversal the app initiated has landed. The history
    /// machine's index has NOT moved yet (an intent only records
    /// `pending`; AUDIT V28 A6-1 caught the v26 doc here claiming
    /// otherwise): the app must call `record_traversal`, which
    /// moves the index and consumes the intent.
    Traversal,
}

/// The reducer's verdict for one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlOutcome {
    /// The new live URL, when the event changed it.
    pub adopted: Option<String>,
    /// The history bookkeeping to apply.
    pub hist: HistEffect,
    /// The event was an observer note rejected as stale (a
    /// Source event already settled this document, or the note
    /// claimed another origin in the pre-Source window). The
    /// app logs these at most in aggregate; they are the D7
    /// wedge being refused, not an error.
    pub stale_note: bool,
}

impl UrlOutcome {
    fn none() -> Self {
        UrlOutcome {
            adopted: None,
            hist: HistEffect::None,
            stale_note: false,
        }
    }
    fn stale() -> Self {
        UrlOutcome {
            adopted: None,
            hist: HistEffect::None,
            stale_note: true,
        }
    }
}

/// One tab's URL state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlState {
    url: String,
    /// A Source event has been seen for the current document.
    /// From then on, observer notes about this document are
    /// hints about a past the platform has already superseded.
    /// (AUDIT V29, A6-L7: the v26 `doc_open` field was written
    /// by three arms and read nowhere since A6-2 — deleted.)
    source_seen_in_doc: bool,
}

impl Default for UrlState {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlState {
    pub fn new() -> Self {
        UrlState {
            url: String::new(),
            source_seen_in_doc: false,
        }
    }

    /// Seed with the URL a tab was created for.
    pub fn with_url(url: &str) -> Self {
        UrlState {
            url: url.to_string(),
            source_seen_in_doc: false,
        }
    }

    /// The one live URL. The address bar, the tab record and
    /// Teleport all read this (through the app's mirror field).
    pub fn url(&self) -> &str {
        &self.url
    }

    fn adopt(&mut self, url: &str) -> Option<String> {
        if self.url == url {
            None
        } else {
            self.url = url.to_string();
            Some(url.to_string())
        }
    }

    /// Apply one event. `pending_target` is the history
    /// machine's pending back/forward target, when a traversal
    /// the app initiated is in flight.
    pub fn apply(&mut self, ev: UrlEvent, pending_target: Option<&str>) -> UrlOutcome {
        self.apply_landing(ev, pending_target, false)
    }

    /// [`Self::apply`], plus the one fact the Source arm needs
    /// from the history machine (AUDIT V29, A6-H1):
    /// `adopted_in_history` — the event's URL already exists
    /// in the tab's history at a non-current entry. The app
    /// computes it for Source events; it is ignored for every
    /// other event kind.
    pub fn apply_landing(
        &mut self,
        ev: UrlEvent,
        pending_target: Option<&str>,
        adopted_in_history: bool,
    ) -> UrlOutcome {
        match ev {
            UrlEvent::CommitStart(u) => {
                if !usable(&u) {
                    return UrlOutcome::none();
                }
                // A new document begins: the observer is a live
                // source again until Source speaks for this doc.
                // AUDIT V28 A6-9: a Source event for THIS document
                // can arrive before its CommitStart (the two
                // travel different channels); it has already
                // adopted this very URL, so the flag is preserved
                // when the start URL is the live URL — resetting
                // unconditionally re-opened the note window to a
                // late note from the PREVIOUS document.
                let source_was_for_this_doc = self.source_seen_in_doc && self.url == u;

                self.source_seen_in_doc = source_was_for_this_doc;
                UrlOutcome {
                    adopted: self.adopt(&u),
                    hist: HistEffect::None,
                    stale_note: false,
                }
            }
            UrlEvent::CommitFinish(u) => {
                if !usable(&u) {
                    return UrlOutcome::none();
                }
                // AUDIT V28 A6-9: a finish whose start was never
                // delivered still opens a NEW document — reset
                // the Source flag (same preservation rule as
                // CommitStart: a Source that already adopted this
                // URL was speaking for this document).
                let source_was_for_this_doc = self.source_seen_in_doc && self.url == u;

                self.source_seen_in_doc = source_was_for_this_doc;
                UrlOutcome {
                    adopted: self.adopt(&u),
                    hist: HistEffect::Commit,
                    stale_note: false,
                }
            }
            UrlEvent::Source(u) => {
                if !usable(&u) {
                    return UrlOutcome::none();
                }
                self.source_seen_in_doc = true;
                let adopted = self.adopt(&u);
                if adopted.is_none() {
                    return UrlOutcome::none();
                }
                // Classification of a same-document Source
                // change (AUDIT V29, A6-H1 — this arm's v28
                // form is DISPROVEN as a general rule):
                //
                // 1. A pending intent exists: this landing IS
                //    the traversal the app asked for, whether
                //    it hits the target exactly or was
                //    redirected/coalesced on the way —
                //    `record_traversal` reconciles both (the
                //    v28 exact-match requirement left its
                //    redirect branch unreachable and every
                //    other landing fell through to Push).
                // 2. No intent, but the URL already lives in
                //    the tab's history at another entry: an
                //    UNINITIATED traversal (mouse buttons,
                //    gesture, the page's own history.go() —
                //    every SPA's in-app back). Classify Pop so
                //    `record_pop`'s landing search moves the
                //    index and preserves the forward branch.
                //    The v28 code pushed here: entries
                //    [a,b,c] landing on b became [a,b,c,b] with
                //    forward history destroyed, permanently.
                // 3. Otherwise: a genuine same-document push.
                //
                // The ambiguity in branch 2 is real and
                // unresolved (AUDIT 0.31, A6-F3 — the earlier
                // claim here that this case is "rare and
                // self-heals on its next landing" was FALSE
                // and is withdrawn): the reducer's signals
                // cannot distinguish a pushState to a
                // previously-visited URL from a traversal
                // landing, so branch 2 classifies it Pop.
                // Pico's index can then diverge from the
                // WebView's real stack — Pico can show Back
                // disabled while the view can still go back,
                // or present a phantom Forward entry the view
                // does not have — because the WebView's own
                // stack gained an entry Pico's did not. The
                // divergence reconverges only at the next
                // real document commit, whose landing search
                // reconciles by URL. This is an accepted
                // ambiguity of the available signals (0.31):
                // branch 2 is kept because uninitiated
                // traversals are constant in real SPA use and
                // misclassifying THEM as pushes permanently
                // corrupted the forward branch, while no
                // signal on hand can tell the two cases
                // apart. The two traces are pinned as tests
                // below, documenting current behavior.
                // AUDIT V28 A6-8's caveat stands for
                // replaceState in branch 3 (recorded as a
                // push; Back can land on the pre-replace URL).
                let hist = if pending_target.is_some() {
                    HistEffect::Traversal
                } else if adopted_in_history {
                    HistEffect::Pop
                } else {
                    HistEffect::Push
                };
                UrlOutcome {
                    adopted,
                    hist,
                    stale_note: false,
                }
            }
            UrlEvent::Note { how, href } => {
                if !usable(&href) {
                    return UrlOutcome::stale();
                }
                if href == self.url {
                    return UrlOutcome::none();
                }
                if self.source_seen_in_doc {
                    // The platform already settled this document;
                    // a note describing anything else is the
                    // past. This is the D7 wedge, refused.
                    return UrlOutcome::stale();
                }
                if !self.url.is_empty() {
                    // Pre-Source window: the v25 origin gate,
                    // kept for exactly the span where the
                    // observer is the only signal. A page cannot
                    // spoof another origin's URL into the bar.
                    // AUDIT V28 A6-2: the gate no longer requires
                    // doc state — a freshly seeded tab (created
                    // for a URL, no commit processed yet) was an
                    // ungated spoof window on every new tab and
                    // conversion. The seed URL is a web URL by
                    // construction, so the gate always has an
                    // origin to check against.
                    let same = match (
                        crate::core::addr::web_origin(&self.url),
                        crate::core::addr::web_origin(&href),
                    ) {
                        (Some(a), Some(b)) => a == b,
                        _ => false,
                    };
                    if !same {
                        return UrlOutcome::stale();
                    }
                }
                let adopted = self.adopt(&href);
                let hist = match how {
                    NavHow::Push => HistEffect::Push,
                    NavHow::Replace => HistEffect::Replace,
                    NavHow::Pop => HistEffect::Pop,
                    NavHow::Init => HistEffect::None,
                };
                UrlOutcome {
                    adopted,
                    hist,
                    stale_note: false,
                }
            }
        }
    }
}

/// URLs that may become a tab's live URL: non-empty, not the
/// blank bootstrap document. Internal `pico:` URLs never reach
/// here (the app handles start tabs separately); if one does,
/// it is not a web URL the reducer may adopt.
fn usable(u: &str) -> bool {
    !u.is_empty() && u != "about:blank" && crate::core::addr::web_origin(u).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAN_WWW: &str = "https://www.youtube.com/@iK4w4";
    const CHAN_BARE: &str = "https://youtube.com/@iK4w4";
    const WATCH_BARE: &str = "https://youtube.com/watch?v=abc123";
    const WATCH_WWW: &str = "https://www.youtube.com/watch?v=abc123";

    fn committed(url: &str) -> UrlState {
        let mut s = UrlState::with_url(url);
        s.apply(UrlEvent::CommitStart(url.to_string()), None);
        s.apply(UrlEvent::CommitFinish(url.to_string()), None);
        s
    }

    #[test]
    fn commit_then_source_spa_pushes() {
        let mut s = committed(CHAN_WWW);
        let out = s.apply(UrlEvent::Source(WATCH_WWW.to_string()), None);
        assert_eq!(out.adopted.as_deref(), Some(WATCH_WWW));
        assert_eq!(out.hist, HistEffect::Push);
        assert_eq!(s.url(), WATCH_WWW);
    }

    #[test]
    fn the_d7_wedge_is_refused_and_source_unfreezes() {
        // Commit lands on the www channel URL (what the address
        // bar showed). YouTube's router replaceStates to the
        // bare host form: the observer note for it is
        // cross-origin against the stored URL…
        let mut s = committed(CHAN_WWW);
        let note = s.apply(
            UrlEvent::Note {
                how: NavHow::Replace,
                href: CHAN_BARE.to_string(),
            },
            None,
        );
        assert!(note.stale_note, "pre-Source window keeps the origin gate");
        // …but SourceChanged is never gated: the flip is adopted.
        let flip = s.apply(UrlEvent::Source(CHAN_BARE.to_string()), None);
        assert_eq!(flip.adopted.as_deref(), Some(CHAN_BARE));
        // From now on the tab tracks the bare-origin SPA.
        let watch = s.apply(UrlEvent::Source(WATCH_BARE.to_string()), None);
        assert_eq!(watch.adopted.as_deref(), Some(WATCH_BARE));
        assert_eq!(watch.hist, HistEffect::Push);
        // And a late observer note describing the old www world
        // (its `last` advanced before the drop) cannot roll the
        // URL back.
        let stale = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: WATCH_WWW.to_string(),
            },
            None,
        );
        assert!(stale.stale_note);
        assert_eq!(s.url(), WATCH_BARE);
    }

    #[test]
    fn note_before_source_still_works_legacy_path() {
        // No Source wiring on some hypothetical build: notes in
        // the pre-Source window behave as v25's did.
        let mut s = committed("https://example.com/a");
        let out = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: "https://example.com/b".to_string(),
            },
            None,
        );
        assert_eq!(out.adopted.as_deref(), Some("https://example.com/b"));
        assert_eq!(out.hist, HistEffect::Push);
        // A cross-origin spoof in the same window is refused.
        let spoof = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: "https://evil.example/c".to_string(),
            },
            None,
        );
        assert!(spoof.stale_note);
        assert_eq!(s.url(), "https://example.com/b");
    }

    #[test]
    fn source_matching_pending_target_is_a_traversal() {
        let mut s = committed(WATCH_WWW);
        s.apply(UrlEvent::Source(CHAN_WWW.to_string()), None);
        let out = s.apply(UrlEvent::Source(WATCH_WWW.to_string()), Some(WATCH_WWW));
        assert_eq!(out.hist, HistEffect::Traversal);
        assert_eq!(out.adopted.as_deref(), Some(WATCH_WWW));
    }

    #[test]
    fn ordering_commit_source_source_title_irrelevant() {
        // commit → source-change → source-change: each adoption
        // lands in order, the last one wins.
        let mut s = UrlState::new();
        s.apply(UrlEvent::CommitStart("https://a.example/".into()), None);
        let f = s.apply(UrlEvent::CommitFinish("https://a.example/".into()), None);
        assert_eq!(f.hist, HistEffect::Commit);
        s.apply(UrlEvent::Source("https://a.example/one".into()), None);
        let last = s.apply(UrlEvent::Source("https://a.example/two".into()), None);
        assert_eq!(last.adopted.as_deref(), Some("https://a.example/two"));
        assert_eq!(s.url(), "https://a.example/two");
        // A duplicate source is a no-op.
        let dup = s.apply(UrlEvent::Source("https://a.example/two".into()), None);
        assert_eq!(dup, UrlOutcome::none());
    }

    #[test]
    fn unusable_urls_never_adopt() {
        let mut s = committed("https://example.com/");
        for bad in ["", "about:blank", "pico://start", "javascript:x"] {
            let out = s.apply(UrlEvent::Source(bad.to_string()), None);
            assert_eq!(out.adopted, None, "source {bad:?}");
            let out = s.apply(UrlEvent::CommitStart(bad.to_string()), None);
            assert_eq!(out.adopted, None, "commit {bad:?}");
        }
        assert_eq!(s.url(), "https://example.com/");
    }

    #[test]
    fn seed_window_notes_are_origin_gated() {
        // AUDIT V28 A6-2: a tab seeded for a URL but with no
        // commit processed yet must not adopt a cross-origin
        // observer note — the seed URL is the gate's origin.
        let mut s = UrlState::with_url("https://bank.example/home");
        let spoof = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: "https://evil.example/phish".to_string(),
            },
            None,
        );
        assert!(spoof.stale_note);
        assert_eq!(s.url(), "https://bank.example/home");
        // A same-origin note in the same window still works.
        let ok = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: "https://bank.example/next".to_string(),
            },
            None,
        );
        assert_eq!(ok.adopted.as_deref(), Some("https://bank.example/next"));
    }

    #[test]
    fn composed_source_first_traversal_repairs_history() {
        // AUDIT V28 A6-1, the seam test that did not exist: the
        // reducer's outcomes feed a real TabHistory in the app's
        // order. Source beats the observer note to the landing.
        use crate::core::history::TabHistory;
        let mut s = committed("https://example.com/a");
        let mut h = TabHistory::new();
        h.record_commit("https://example.com/a");
        let push = s.apply(UrlEvent::Source("https://example.com/b".into()), None);
        assert_eq!(push.hist, HistEffect::Push);
        h.record_push("https://example.com/b");
        assert_eq!(h.go_back(0), Some("https://example.com/a"));
        let target = h.pending_target().map(|t| t.to_string());
        let land = s.apply(
            UrlEvent::Source("https://example.com/a".into()),
            target.as_deref(),
        );
        assert_eq!(land.hist, HistEffect::Traversal);
        h.record_traversal("https://example.com/a");
        assert_eq!(h.current(), Some("https://example.com/a"));
        assert!(h.can_go_forward(), "forward entry to b survives");
        assert!(!h.has_pending(), "the intent is consumed");
        // The late observer Pop note is a duplicate, harmlessly.
        let late = s.apply(
            UrlEvent::Note {
                how: NavHow::Pop,
                href: "https://example.com/a".to_string(),
            },
            None,
        );
        assert_eq!(late, UrlOutcome::none());
        // A fresh navigation then appends cleanly.
        let fin = s.apply(UrlEvent::CommitFinish("https://example.com/d".into()), None);
        assert_eq!(fin.hist, HistEffect::Commit);
        h.record_commit("https://example.com/d");
        assert_eq!(
            h.entries(),
            ["https://example.com/a", "https://example.com/d"]
        );
    }

    #[test]
    fn pushstate_to_visited_url_is_classified_pop_accepted_ambiguity() {
        // AUDIT 0.31 (A6-F3), Trace A — documents CURRENT
        // behavior, not desired behavior: a pushState to a
        // previously-visited URL arrives as a Source event
        // whose URL lives in history, so branch 2 classifies
        // it Pop and the index moves back, even though the
        // WebView's real stack GAINED an entry (it can go
        // back from the pushed entry; Pico shows Back
        // disabled at index 0 while entries remain ahead).
        // The divergence reconverges only at the next real
        // document commit (see the Source arm's comment).
        use crate::core::history::TabHistory;
        let mut s = committed("https://example.com/a");
        let mut h = TabHistory::new();
        h.record_commit("https://example.com/a");
        for u in ["https://example.com/b", "https://example.com/c"] {
            let out = s.apply_landing(UrlEvent::Source(u.into()), None, false);
            assert_eq!(out.hist, HistEffect::Push);
            h.record_push(u);
        }
        assert_eq!(h.current(), Some("https://example.com/c"));
        // The page now pushStates to a (visited at index 0).
        let out = s.apply_landing(UrlEvent::Source("https://example.com/a".into()), None, true);
        assert_eq!(out.hist, HistEffect::Pop, "classified as a traversal");
        h.record_pop("https://example.com/a");
        assert_eq!(
            h.entries(),
            [
                "https://example.com/a",
                "https://example.com/b",
                "https://example.com/c"
            ]
        );
        assert_eq!(h.index(), Some(0));
        assert!(
            !h.can_go_back(),
            "Pico shows Back disabled — the WebView can still go back"
        );
        assert!(h.can_go_forward(), "entries remain ahead in Pico's model");
    }

    #[test]
    fn replacestate_to_visited_url_yields_forward_entry_accepted_ambiguity() {
        // AUDIT 0.31 (A6-F3), Trace B — documents CURRENT
        // behavior: a replaceState to a previously-visited
        // URL also arrives as a Source event, is classified
        // Pop for the same reason as Trace A, and produces
        // the forward-entry shape: the index steps back and
        // the pre-replace entry sits ahead as a Forward
        // entry, although replaceState created no forward
        // entry in the WebView's real stack.
        use crate::core::history::TabHistory;
        let mut s = committed("https://example.com/a");
        let mut h = TabHistory::new();
        h.record_commit("https://example.com/a");
        for u in ["https://example.com/b", "https://example.com/c"] {
            let out = s.apply_landing(UrlEvent::Source(u.into()), None, false);
            assert_eq!(out.hist, HistEffect::Push);
            h.record_push(u);
        }
        assert_eq!(h.current(), Some("https://example.com/c"));
        assert!(!h.can_go_forward());
        // The page now replaceStates the current document
        // to b (visited at index 1): no new entry, no
        // traversal in the WebView.
        let out = s.apply_landing(UrlEvent::Source("https://example.com/b".into()), None, true);
        assert_eq!(out.hist, HistEffect::Pop, "classified as a traversal");
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
        assert!(
            h.can_go_forward(),
            "the forward-entry shape: c sits ahead though replaceState made none"
        );
    }

    #[test]
    fn source_before_commit_start_keeps_the_note_window_closed() {
        // AUDIT V28 A6-9: the new document's Source arrived
        // before its CommitStart; the start must not re-open
        // note acceptance for that document.
        let mut s = committed("https://example.com/old");
        s.apply(UrlEvent::Source("https://example.com/new".into()), None);
        s.apply(
            UrlEvent::CommitStart("https://example.com/new".into()),
            None,
        );
        let late = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: "https://example.com/old".to_string(),
            },
            None,
        );
        assert!(late.stale_note, "a previous-document note stays refused");
        assert_eq!(s.url(), "https://example.com/new");
    }

    #[test]
    fn commit_finish_without_start_reopens_the_note_window() {
        // AUDIT V28 A6-9 (converse): a document whose CommitStart
        // was never delivered must not inherit the previous
        // document's Source flag at its CommitFinish.
        let mut s = committed("https://example.com/a");
        s.apply(UrlEvent::Source("https://example.com/spa".into()), None);
        s.apply(
            UrlEvent::CommitFinish("https://example.com/doc2".into()),
            None,
        );
        let note = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: "https://example.com/doc2/next".to_string(),
            },
            None,
        );
        assert_eq!(
            note.adopted.as_deref(),
            Some("https://example.com/doc2/next")
        );
        assert_eq!(note.hist, HistEffect::Push);
    }

    #[test]
    fn new_commit_reopens_the_note_window() {
        // After a fresh full navigation the observer may speak
        // for the new document until Source does.
        let mut s = committed(CHAN_WWW);
        s.apply(UrlEvent::Source(CHAN_BARE.to_string()), None);
        s.apply(UrlEvent::CommitStart("https://example.com/x".into()), None);
        let out = s.apply(
            UrlEvent::Note {
                how: NavHow::Push,
                href: "https://example.com/y".to_string(),
            },
            None,
        );
        assert_eq!(out.adopted.as_deref(), Some("https://example.com/y"));
    }
}
