//! Per-tab session-history state machine.
//!
//! One reconciler per tab consumes three kinds of facts and
//! keeps `entries`/`index` coherent:
//!
//! - **native commits** ([`TabHistory::record_commit`]) — wry
//!   page-load-finished events; the authority for real document
//!   loads. Redirect chains collapse naturally because only the
//!   final URL is ever committed by the webview.
//! - **SPA notes** ([`TabHistory::record_push`],
//!   [`TabHistory::record_replace`], [`TabHistory::record_pop`])
//!   — from the injected navigation observer (pushState /
//!   replaceState / popstate / hashchange), the *only* signals
//!   same-document navigations produce.
//! - **intents** ([`TabHistory::go_back`] /
//!   [`TabHistory::go_forward`]) — recorded when Pico itself asks
//!   the webview to traverse, so the resulting commit/pop
//!   reconciles as a move instead of a new entry.
//!
//! Invariants: entries never contain consecutive duplicates;
//! `index` always points at the entry the tab is showing; a
//! pending intent is consumed by whichever signal (pop, commit,
//! or the SourceChanged traversal confirmation) satisfies it
//! first, and the second signal then degenerates to a harmless
//! no-op.

/// Hard cap on remembered entries per tab (real browsers cap too).
pub const MAX_ENTRIES: usize = 100;

/// How long a back/forward intent may await its landing
/// before the shell treats it as dead (AUDIT 0.30.1). A real
/// landing — commit, pop, or the SourceChanged traversal
/// confirmation — arrives in the same event-loop breath as
/// the `history.back()` call that caused it; five seconds is
/// orders of magnitude past any genuine landing, so an intent
/// older than this is one whose signal never came (the
/// webview's own stack and this model diverged far enough
/// that the traversal was a silent no-op). Time is injected
/// by the caller (the shell's monotonic millisecond clock),
/// keeping this machine pure and its tests deterministic.
pub const INTENT_TTL_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Back,
    Forward,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabHistory {
    entries: Vec<String>,
    index: Option<usize>,
    pending: Option<Pending>,
    /// When the pending intent was declared, on the caller's
    /// injected clock (AUDIT 0.30.1). Set only by
    /// [`TabHistory::go_back`] / [`TabHistory::go_forward`],
    /// cleared wherever `pending` is.
    pending_at: Option<u64>,
    /// The URL a document load STARTED at, remembered between
    /// the load's start and its commit (AUDIT 0.31, A6-F1).
    /// Set by [`TabHistory::note_commit_start`], taken by
    /// [`TabHistory::record_commit`], and dropped by any
    /// same-document signal (push / replace / pop /
    /// traversal), which supersedes a started load.
    commit_start: Option<String>,
}

impl TabHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<&str> {
        self.index
            .and_then(|i| self.entries.get(i))
            .map(String::as_str)
    }

    pub fn can_go_back(&self) -> bool {
        self.index.map(|i| i > 0).unwrap_or(false)
    }

    pub fn can_go_forward(&self) -> bool {
        match self.index {
            Some(i) => i + 1 < self.entries.len(),
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn index(&self) -> Option<usize> {
        self.index
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// The URL a pending back/forward intent expects to land on.
    pub fn pending_target(&self) -> Option<&str> {
        let idx = self.index?;
        match self.pending? {
            Pending::Back => idx.checked_sub(1).and_then(|i| self.entries.get(i)),
            Pending::Forward => self.entries.get(idx + 1),
        }
        .map(String::as_str)
    }

    /// Declare "the user asked to go back"; returns the expected
    /// target. The index does not move until a pop/commit
    /// confirms the landing. `now_ms` is the caller's monotonic
    /// clock, stamped so [`Self::pending_expired`] can later
    /// tell a live intent from a dead one (AUDIT 0.30.1).
    pub fn go_back(&mut self, now_ms: u64) -> Option<&str> {
        if !self.can_go_back() {
            return None;
        }
        self.pending = Some(Pending::Back);
        self.pending_at = Some(now_ms);
        self.pending_target()
    }

    /// Declare "the user asked to go forward". See
    /// [`Self::go_back`].
    pub fn go_forward(&mut self, now_ms: u64) -> Option<&str> {
        if !self.can_go_forward() {
            return None;
        }
        self.pending = Some(Pending::Forward);
        self.pending_at = Some(now_ms);
        self.pending_target()
    }

    /// Drop a pending intent (e.g. a different navigation started
    /// first).
    pub fn cancel_pending(&mut self) {
        self.clear_pending();
    }

    fn clear_pending(&mut self) {
        self.pending = None;
        self.pending_at = None;
    }

    /// Whether the pending intent has outlived
    /// [`INTENT_TTL_MS`] with no landing (AUDIT 0.30.1, the
    /// A6-MED residue): through 0.30 an intent whose signal
    /// never arrived wedged Back/Forward for the tab — the
    /// shell's no-stacking guard refused every later traversal
    /// — until an unrelated navigation happened to clear it.
    /// The shell now consults this at the re-click: an expired
    /// intent is cancelled and the click issues its own. A
    /// late landing after expiry is harmless: with no intent
    /// in flight it reconciles by URL via the landing search,
    /// exactly like an uninitiated traversal. An intent with
    /// no stamp (impossible through `go_back`/`go_forward`;
    /// defensive) never expires.
    pub fn pending_expired(&self, now_ms: u64) -> bool {
        match (self.pending, self.pending_at) {
            (Some(_), Some(t0)) => now_ms.saturating_sub(t0) > INTENT_TTL_MS,
            _ => false,
        }
    }

    /// Whether a back/forward intent is still awaiting its
    /// landing. The shell refuses to stack a second intent on top
    /// of the first (AUDIT V28 A6-7): the single pending slot
    /// cannot represent two traversals, and overwriting it made
    /// the eventual landing reconcile against the wrong branch.
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The entry a pop/commit landing on `url` most plausibly
    /// means: the pending direction's side first (the NEAREST
    /// matching entry in that direction — the intent's own
    /// target is by construction the nearest entry on its
    /// side, and a coalesced multi-step traversal's landing
    /// is the nearest match beyond it), otherwise the nearest
    /// matching entry, ties preferring the earlier entry.
    /// (AUDIT 0.30.1: this doc said "furthest" through 0.30;
    /// the code has always taken the nearest — the doc now
    /// agrees with the code and with the tests that pin it.)
    fn landing_index(&self, url: &str) -> Option<usize> {
        let i = self.index?;
        match self.pending {
            Some(Pending::Back) => {
                if let Some(j) = (0..i).rev().find(|&j| self.entries[j] == url) {
                    return Some(j);
                }
            }
            Some(Pending::Forward) => {
                if let Some(j) = (i + 1..self.entries.len()).find(|&j| self.entries[j] == url) {
                    return Some(j);
                }
            }
            None => {}
        }
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.as_str() == url)
            .map(|(j, _)| j)
            .min_by_key(|&j| (j.abs_diff(i), j > i))
    }

    fn push_new(&mut self, url: &str) {
        // Truncate the forward branch, then append.
        if let Some(i) = self.index {
            self.entries.truncate(i + 1);
        } else {
            self.entries.clear();
        }
        self.entries.push(url.to_string());
        self.index = Some(self.entries.len() - 1);
        // Enforce the cap by dropping the oldest entries.
        while self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
            if let Some(i) = self.index {
                self.index = Some(i.saturating_sub(1));
            }
        }
    }

    /// Remember the URL a document load STARTED at (AUDIT
    /// 0.31, A6-F1). The shell calls this at page-load start
    /// so [`Self::record_commit`] can reconcile a coalesced
    /// traversal landing against the entry the load actually
    /// departed from, not just the entry it finished on.
    /// Empty URLs carry no information and are ignored.
    pub fn note_commit_start(&mut self, url: &str) {
        if !url.is_empty() {
            self.commit_start = Some(url.to_string());
        }
    }

    /// A real document navigation committed at `url`
    /// (page-load finish).
    pub fn record_commit(&mut self, url: &str) {
        if url.is_empty() {
            return;
        }
        // AUDIT 0.31 (A6-F1, the core half): take the
        // remembered commit-start FIRST. With an intent in
        // flight, a coalesced traversal's load may START at
        // an entry far from the intent's one-step target and
        // finish at a redirect of THAT entry (a URL no entry
        // holds): entries [a,b,c] at c, Back intent (target
        // b), load starts at a and finishes at a′. The start
        // URL identifies which entry the finish replaces —
        // landing_index(a) is 0, so a′ takes a's slot and the
        // index lands on 0: [a′,b,c]. Without the remembered
        // start, the branches below step-replace the intent's
        // target instead: [a,a′,c], the A6-M2 corruption one
        // seam over. A start equal to the finish is the
        // ordinary traversal commit and falls through to the
        // existing paths untouched.
        let commit_start = self.commit_start.take();
        if self.pending.is_some() {
            if let Some(s) = commit_start.as_deref() {
                if s != url {
                    if let Some(j) = self.landing_index(s) {
                        // AUDIT 0.40.2: never replace the slot
                        // with a URL the list already holds —
                        // above all not with a NEIGHBOUR's URL,
                        // which mints consecutive duplicates
                        // ([a,b]: start a, finish b → [b,b]).
                        // When the finish URL is already
                        // present, skip the replace and let
                        // the landing search below reconcile
                        // by position instead.
                        let finish_already_present = self.landing_index(url).is_some()
                            || (j > 0 && self.entries[j - 1] == url)
                            || self.entries.get(j + 1).map(String::as_str) == Some(url);
                        if !finish_already_present {
                            self.entries[j] = url.to_string();
                            self.index = Some(j);
                            self.clear_pending();
                            return;
                        }
                    }
                }
            }
        }
        // AUDIT V29 (A6-M2): with an intent in flight, consult
        // the landing search FIRST (while `pending` is still
        // set, so the search is direction-aware) — a coalesced
        // multi-step landing (the page raced our intent with
        // its own traversal) moves the index to the matching
        // entry, however far. Through v28 the branches below
        // blindly stepped one and REPLACED that entry: [a,b,c]
        // with a Back intent landing on a became [a,a,c]. Only
        // when NO entry matches is it a redirect during the
        // traversal, which replaces the one-step destination
        // with the final URL, as before.
        if self.pending.is_some() {
            if let Some(j) = self.landing_index(url) {
                self.index = Some(j);
                self.clear_pending();
                return;
            }
        }
        let pending = self.pending.take();
        self.pending_at = None;
        match pending {
            Some(Pending::Back) => {
                if let Some(i) = self.index {
                    if i > 0 && self.entries.get(i).map(String::as_str) != Some(url) {
                        self.entries[i - 1] = url.to_string();
                        self.index = Some(i - 1);
                        return;
                    }
                }
                // Fell through: index 0 or same URL — treat as plain.
                self.record_commit_plain(url);
            }
            Some(Pending::Forward) => {
                if let Some(i) = self.index {
                    if i + 1 < self.entries.len()
                        && self.entries.get(i).map(String::as_str) != Some(url)
                    {
                        self.entries[i + 1] = url.to_string();
                        self.index = Some(i + 1);
                        return;
                    }
                }
                self.record_commit_plain(url);
            }
            None => self.record_commit_plain(url),
        }
    }

    fn record_commit_plain(&mut self, url: &str) {
        match self.index {
            None => self.push_new(url),
            Some(i) => {
                if self.entries.get(i).map(String::as_str) == Some(url) {
                    // Same-URL reload: nothing moves.
                } else if let Some(j) = self.landing_index(url) {
                    // A traversal we did not initiate (mouse
                    // buttons, gesture, or a coalesced multi-step
                    // jump): reconcile as a move, however far.
                    self.index = Some(j);
                } else {
                    self.push_new(url);
                }
            }
        }
    }

    /// A SourceChanged event landed exactly on the pending
    /// traversal's target (AUDIT V28 A6-1). This is the third
    /// signal that can satisfy an intent, beside pop and commit:
    /// the index moves to the pending neighbour (a redirect
    /// during the traversal replaces the destination entry with
    /// the final URL, exactly as `record_commit` does) and the
    /// intent is consumed — so the observer's late pop note for
    /// the same URL degenerates to a same-URL no-op instead of
    /// being the only thing that could have repaired the index.
    pub fn record_traversal(&mut self, url: &str) {
        if url.is_empty() {
            return;
        }
        // AUDIT 0.31 (A6-F1): a same-document signal
        // supersedes any started load.
        self.commit_start = None;
        let Some(i) = self.index else {
            self.push_new(url);
            return;
        };
        if self.entries.get(i).map(String::as_str) == Some(url) {
            self.clear_pending();
            return;
        }
        // AUDIT 0.30.1 (A6 HIGH — the seam the v29 fix missed):
        // `record_commit` received the A6-M2 landing search in
        // v29 and `record_pop` always had one, but this third
        // consumer of a pending intent still stepped ONE entry
        // and REPLACED it with the landed URL. A coalesced
        // multi-step traversal whose final signal is
        // SourceChanged — entries [a,b,c] at c, Back intent
        // (target b), the page landing straight on a — became
        // [a,a,c] at index 1: consecutive duplicates and a
        // phantom Back. SPA same-document traversals produce
        // no commit, so this function is the final word for
        // exactly the sessions the A6-M2 fix was about. With
        // an intent in flight, consult the landing search
        // FIRST (while `pending` is still set, so the search
        // is direction-aware), exactly as `record_commit`
        // does. Only when NO entry matches is the landing a
        // redirect during the traversal, which step-replaces
        // the one-step destination as before (the S13 shape,
        // pinned by the tests).
        if self.pending.is_some() {
            if let Some(j) = self.landing_index(url) {
                self.index = Some(j);
                self.clear_pending();
                return;
            }
        }
        let target = match self.pending {
            Some(Pending::Back) if i > 0 => Some(i - 1),
            Some(Pending::Forward) if i + 1 < self.entries.len() => Some(i + 1),
            _ => None,
        };
        if let Some(t) = target {
            if self.entries.get(t).map(String::as_str) != Some(url) {
                self.entries[t] = url.to_string();
            }
            self.index = Some(t);
            self.clear_pending();
            return;
        }
        // No (or no longer) a pending intent: reconcile like a pop.
        self.record_pop(url);
    }

    /// SPA `history.pushState` / programmatic same-document
    /// navigation.
    pub fn record_push(&mut self, url: &str) {
        if url.is_empty() {
            return;
        }
        // AUDIT 0.31 (A6-F1): a same-document signal
        // supersedes any started load.
        self.commit_start = None;
        if self.current() == Some(url) {
            return;
        }
        self.clear_pending();
        self.push_new(url);
    }

    /// Remove the CURRENT entry (AUDIT 0.33): the repair for a
    /// download navigation that polluted history via
    /// [`Self::record_push`] (WS1) — the shell drops the file
    /// entry the push created so the tab's history is what it
    /// was before the captured navigation. The index becomes
    /// `index - 1` when it was > 0, and `None` when it was 0
    /// or the entries are now empty. Pending intent and the
    /// remembered commit-start are cleared, mirroring
    /// `record_push`'s hygiene: neither may survive pointing
    /// at an entry that no longer exists. No-op when there
    /// is no current entry.
    pub fn drop_top_entry(&mut self) {
        let Some(i) = self.index else {
            return;
        };
        if i >= self.entries.len() {
            return;
        }
        self.entries.remove(i);
        // AUDIT 0.40.2: the removal can splice two EQUAL
        // neighbours together ([a,b,a] at b drops to [a,a])
        // — the same consecutive duplicate the A6-L6 second
        // drop prevents in `record_replace`. Drop the
        // shifted-in copies at the seam too.
        if i >= 1 {
            while self.entries.get(i - 1).is_some()
                && self.entries.get(i - 1) == self.entries.get(i)
            {
                self.entries.remove(i);
            }
        }
        self.index = if self.entries.is_empty() || i == 0 {
            None
        } else {
            Some(i - 1)
        };
        // Edge: removing entry 0 while later entries remain
        // leaves index None with a non-empty list — the same
        // shape a fresh history has before its first commit;
        // the next commit/pop reconciles by push. (The WS1
        // repair case is a single-entry or top-entry drop,
        // where this lands exactly on the previous page or
        // on no page at all.)
        self.clear_pending();
        self.commit_start = None;
    }

    /// SPA `history.replaceState`: rewrite the current entry in
    /// place.
    pub fn record_replace(&mut self, url: &str) {
        if url.is_empty() {
            return;
        }
        // AUDIT 0.31 (A6-F1): a same-document signal
        // supersedes any started load.
        self.commit_start = None;
        // A replace rewrites the CURRENT entry — possibly
        // collapsing it into a neighbour, which moves the index.
        // A pending traversal aimed from the old index would
        // afterwards point at a different entry than the user
        // asked for, so the intent is dropped (AUDIT V28 A6-7);
        // if the traversal still lands, its pop/commit reconciles
        // by URL via the landing search.
        self.clear_pending();
        match self.index {
            Some(i) => {
                // Replacing with the previous/next entry's URL
                // would create ambiguity; collapse to that entry.
                if i > 0 && self.entries.get(i - 1).map(String::as_str) == Some(url) {
                    self.entries.remove(i);
                    self.index = Some(i - 1);
                    // AUDIT V29 (A6-L6): the entry that shifted
                    // into the vacated slot can be the same URL
                    // again ([a,b,a] replacing b with a left
                    // [a,a]) — drop it too, keeping the
                    // no-consecutive-duplicates invariant.
                    if self.entries.get(i).map(String::as_str) == Some(url) {
                        self.entries.remove(i);
                    }
                } else if self.entries.get(i + 1).map(String::as_str) == Some(url) {
                    self.entries.remove(i);
                } else {
                    self.entries[i] = url.to_string();
                }
            }
            None => self.push_new(url),
        }
    }

    /// SPA `popstate` / `hashchange`: the document traversed
    /// without a reload. Moves the index when the URL matches a
    /// neighbour; a matching pop also consumes any pending
    /// intent (the traversal we asked for has happened — a later
    /// native commit for the same URL then degenerates to a
    /// same-URL no-op).
    pub fn record_pop(&mut self, url: &str) {
        if url.is_empty() {
            return;
        }
        // AUDIT 0.31 (A6-F1): a same-document signal
        // supersedes any started load.
        self.commit_start = None;
        let Some(i) = self.index else {
            self.push_new(url);
            return;
        };
        if self.entries.get(i).map(String::as_str) == Some(url) {
            // AUDIT 0.31 (A6-F5): the same-URL early return
            // must consume a pending intent like both sibling
            // branches below do — through 0.30.1 a hash-only
            // pop at the current entry left the intent
            // dangling to reconcile a later, unrelated
            // landing against the wrong branch.
            self.clear_pending();
            return; // hash-only noise at the same entry
        }
        if let Some(j) = self.landing_index(url) {
            // A neighbour — or, for a coalesced multi-step
            // traversal (double-Back, `history.go(-2)`), the
            // matching entry however far away (AUDIT V28 A6-7).
            self.index = Some(j);
            self.clear_pending();
        } else {
            // A jump to a URL history has never seen: safest is
            // to treat it as fresh.
            self.clear_pending();
            self.push_new(url);
        }
    }
}
