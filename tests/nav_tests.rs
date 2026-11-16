//! 0.41.0 (Workstream B) — the routing fix, pinned at the
//! core boundary the shell consults.
//!
//! Typed (or otherwise submitted) `pico://downloads` and
//! `pico://history` must open the MANAGED pages through
//! their openers — never a bare URL load into a normal
//! tab, whose command verbs would all be gate-denied.
//! `App::navigate_active` consults
//! `core::addr::typed_internal_page` BEFORE the generic
//! parse; these tests pin the classifier's exact-spelling
//! discipline and the wall behind it: spellings that are
//! not real pages classify as nothing AND stay refused
//! by `parse_input`.

use pico_core::core::addr::{
    is_allowed_navigation_url, normalize_pico_url, parse_input, route_new_window,
    typed_internal_page, NewWindowRoute, TypedInternalPage,
};

#[test]
fn typed_downloads_routes_to_the_opener() {
    for spelling in [
        "pico://downloads",
        "pico://downloads/",
        "pico://downloads//",
        "pico://downloads?filter=active",
        "pico://downloads#top",
        "https://pico.downloads/",
        "  pico://downloads  ",
    ] {
        assert_eq!(
            typed_internal_page(spelling),
            Some(TypedInternalPage::Downloads),
            "{spelling:?} must route to open_downloads_page"
        );
    }
}

#[test]
fn typed_history_routes_to_the_opener() {
    for spelling in [
        "pico://history",
        "pico://history/",
        "pico://history?q=example",
        "https://pico.history/",
    ] {
        assert_eq!(
            typed_internal_page(spelling),
            Some(TypedInternalPage::History),
            "{spelling:?} must route to open_history_page"
        );
    }
}

#[test]
fn lookalikes_are_refused() {
    for spelling in [
        "pico://downloads.evil",
        "pico://downloads.evil/",
        "pico://historyx",
        "pico://history.",
        "pico://downloads.",
        "pico://download",
        "pico://histor",
        "pico://downloads@evil.example",
    ] {
        assert_eq!(
            typed_internal_page(spelling),
            None,
            "{spelling:?} must NOT classify as a managed page"
        );
        // And the wall behind the classifier: the generic
        // parse refuses the pico: scheme outright, so a
        // lookalike can never load as a document either.
        assert_eq!(
            parse_input(spelling),
            None,
            "{spelling:?} must stay refused by parse_input"
        );
    }
}

#[test]
fn typed_start_still_means_home() {
    // The pre-existing behavior the fix must not disturb:
    // pico://start parses to the start URL, and
    // navigate_active's start check turns it into
    // go_home(). The managed-page classifier does not
    // claim it.
    assert_eq!(typed_internal_page("pico://start"), None);
    assert_eq!(
        parse_input("pico://start"),
        Some(pico_core::core::addr::Target::Url(
            pico_core::core::addr::START_URL.to_string()
        ))
    );
}

#[test]
fn history_joins_the_allowlist_and_the_fold() {
    assert!(is_allowed_navigation_url("pico://history"));
    assert!(is_allowed_navigation_url("pico://history/"));
    assert!(!is_allowed_navigation_url("pico://historyx"));
    assert!(!is_allowed_navigation_url("pico://history.evil"));
    assert_eq!(
        normalize_pico_url("https://pico.history/"),
        "pico://history/"
    );
    // A real pico.* website is untouched by the fold.
    assert_eq!(normalize_pico_url("https://pico.dev/"), "https://pico.dev/");
}

#[test]
fn source_scan_nav_surface_gates_enforced() {
    // The nav handlers are Windows-gated shell code
    // (host-untestable), so their gate enforcement is
    // pinned by source scan like the dl handlers' (the
    // 0.40.1 precedent in media_tests): the chrome handler
    // must consult the surface table for Chrome and
    // require the envelope; the page handler must run
    // BOTH conjunctive gates before parsing.
    let path = format!("{}/src/app.rs", env!("CARGO_MANIFEST_DIR"));
    let app = std::fs::read_to_string(&path).expect("read src/app.rs");
    let span_of = |name: &str| -> String {
        let start = app.find(name).unwrap_or_else(|| panic!("{name} exists"));
        let end = app[start..]
            .find("\n    fn ")
            .map(|i| start + i)
            .unwrap_or(app.len());
        app[start..end].to_string()
    };
    let chrome = span_of("fn on_chrome_nav_body");
    assert!(
        chrome.contains("NavSurface::Chrome"),
        "on_chrome_nav_body must gate verbs against the Chrome surface"
    );
    assert!(
        chrome.contains("reason=surface"),
        "a chrome-surface denial must be logged with its reason"
    );
    assert!(
        chrome.contains("missing chrome envelope"),
        "an envelope-less chrome nav body must be dropped, not dispatched"
    );
    let page = span_of("fn on_nav_page_body");
    assert!(
        page.contains("history_page_gate"),
        "on_nav_page_body must run the conjunctive history gate"
    );
    assert!(
        page.contains("on_start_page"),
        "on_nav_page_body must run the conjunctive start gate"
    );
    assert!(
        page.contains("reason=not-on-nav-page"),
        "a gate denial must be logged with its reason"
    );
}

#[test]
fn pages_cannot_mint_a_history_tab() {
    // route_new_window: the history page joins the
    // downloads page under the explicit internal-page
    // denial — a page must not mint a tab showing the
    // user's browsing record.
    for spelling in ["pico://history", "pico://history/", "https://pico.history/"] {
        assert_eq!(
            route_new_window(spelling),
            NewWindowRoute::Deny("internal-page"),
            "{spelling} must be denied as an internal page"
        );
    }
}

// ---------------------------------------------------------------------------
// 0.41.1 — Front B: nav store hardening regressions
// ---------------------------------------------------------------------------

use pico_core::core::navstore::{
    canonical_nav_key, is_recordable_url, load_from_checked, merge_nav_stores, save_merged,
    save_to, Bookmark, HistoryEntry, NavLoadOutcome, NavStore,
};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn nav_tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pico-nav0411-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn folded_internal_spellings_are_never_recordable() {
    // AUDIT 0.41.1 (Front B F4): case, port and
    // trailing-dot variants of the folded internal hosts
    // escaped the gate through 0.41.0.
    for u in [
        "https://pico.START/",
        "https://PICO.start/",
        "https://pico.start:443/",
        "https://pico.start./",
        "https://pico.downloads./",
        "https://pico.history:443/x",
    ] {
        assert!(!is_recordable_url(u), "{u} must not be recordable");
    }
    // A public lookalike host is unaffected.
    assert!(is_recordable_url("https://pico.start.example.com/"));
    assert!(is_recordable_url("https://example.com/pico.start"));
}

#[test]
fn canonical_key_unifies_spelling_variants() {
    // AUDIT 0.41.1 (Front B F5).
    assert_eq!(
        canonical_nav_key("https://EXAMPLE.com:443/a#frag"),
        "https://example.com/a"
    );
    assert_eq!(
        canonical_nav_key("https://example.com./a"),
        canonical_nav_key("https://example.com/a")
    );
    assert_eq!(
        canonical_nav_key("http://example.com:8080/a"),
        "http://example.com:8080/a",
        "a non-default port is identity"
    );
    assert_ne!(
        canonical_nav_key("https://example.com/A"),
        canonical_nav_key("https://example.com/a"),
        "paths stay case-sensitive"
    );
    let mut s = NavStore::default();
    let now = now_ms();
    assert!(s.record_visit("https://EXAMPLE.com/a#one", "A", now));
    assert!(s.record_visit("https://example.com/a#two", "A", now + 1));
    assert_eq!(s.history.len(), 1, "fragment variants are one entry");
    assert_eq!(s.history[0].visits, 2);
    assert!(s.toggle_bookmark("https://example.com/a", "A", now));
    assert!(s.is_bookmarked("https://example.com.:443/a#x"));
    assert!(!s.toggle_bookmark("https://example.com/a#other", "A", now));
    assert!(s.bookmarks.is_empty(), "the variant spelling toggles off");
}

#[test]
fn corrupt_nav_file_is_quarantined_before_any_save() {
    // AUDIT 0.41.1 (Front B F2).
    let dir = nav_tmp("f2");
    let path = dir.join("nav.json");
    std::fs::write(&path, "{torn write").expect("plant corrupt");
    let outcome = load_from_checked(&path, now_ms());
    assert!(matches!(outcome, NavLoadOutcome::Corrupt(_)), "{outcome:?}");
    assert!(!path.exists(), "the corrupt file moved aside at load");
    assert_eq!(
        std::fs::read(dir.join("nav.json.corrupt")).expect("quarantine kept"),
        b"{torn write",
        "the user's only copy survives as .corrupt"
    );
    // And the session's first save writes a FRESH file —
    // it cannot destroy the quarantined copy.
    save_to(&path, &NavStore::default()).expect("fresh save");
    assert!(path.is_file());
    assert!(dir.join("nav.json.corrupt").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_malformed_entry_no_longer_poisons_the_file() {
    // AUDIT 0.41.1 (Front B F3).
    let dir = nav_tmp("f3");
    let path = dir.join("nav.json");
    let text = r#"{
        "version": 1,
        "bookmarks": [
            {"url": "https://example.com/good", "title": "Good", "added_at": 5},
            {"url": "https://example.com/bad"},
            42
        ],
        "history": [
            {"url": "https://example.org/h", "title": "H", "last_visited": 7, "visits": 3},
            {"url": 9, "title": "broken", "last_visited": 1, "visits": 1}
        ]
    }"#;
    std::fs::write(&path, text).expect("plant");
    match load_from_checked(&path, now_ms()) {
        NavLoadOutcome::Loaded(s) => {
            assert_eq!(s.bookmarks.len(), 1);
            assert_eq!(s.bookmarks[0].url, "https://example.com/good");
            assert_eq!(s.history.len(), 1);
            assert_eq!(s.history[0].visits, 3);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_complete_save_temp_is_recovered_and_stale_temps_reaped() {
    // AUDIT 0.41.1 (Front B F10).
    let dir = nav_tmp("f10");
    let path = dir.join("nav.json");
    let mut s = NavStore::default();
    let now = now_ms();
    s.record_visit("https://example.com/kept", "Kept", now);
    let text = serde_json::to_string_pretty(&s).expect("serialize");
    let temp = dir.join("nav.json.4242.tmp");
    std::fs::write(&temp, &text).expect("plant temp");
    match load_from_checked(&path, now) {
        NavLoadOutcome::Loaded(loaded) => {
            assert_eq!(loaded.history.len(), 1);
            assert_eq!(loaded.history[0].url, "https://example.com/kept");
        }
        other => panic!("expected recovery, got {other:?}"),
    }
    assert!(path.is_file(), "the temp was promoted");
    assert!(!temp.exists());
    // A stale temp beside a healthy main file is reaped.
    let stale = dir.join("nav.json.777.tmp");
    std::fs::write(&stale, &text).expect("plant stale temp");
    let f = std::fs::File::options()
        .write(true)
        .open(&stale)
        .expect("open stale");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7_200))
        .expect("age the temp");
    drop(f);
    let _ = load_from_checked(&path, now_ms());
    assert!(!stale.exists(), "stale temps are cleaned on load");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn merge_unions_two_processes_without_double_counting() {
    // AUDIT 0.41.1 (Front B F9).
    let mut ours = NavStore::default();
    ours.bookmarks.push(Bookmark {
        url: "https://example.com/ours".into(),
        title: "Ours".into(),
        added_at: 10,
    });
    ours.history.push(HistoryEntry {
        url: "https://example.com/shared".into(),
        title: "Old title".into(),
        last_visited: 100,
        visits: 3,
    });
    let mut disk = NavStore::default();
    disk.bookmarks.push(Bookmark {
        url: "https://example.com/theirs".into(),
        title: "Theirs".into(),
        added_at: 20,
    });
    disk.history.push(HistoryEntry {
        url: "https://example.com/shared#variant".into(),
        title: "New title".into(),
        last_visited: 200,
        visits: 5,
    });
    let merged = merge_nav_stores(&ours, &disk);
    assert_eq!(merged.bookmarks.len(), 2, "bookmarks union");
    assert_eq!(merged.history.len(), 1, "history unions by canonical key");
    assert_eq!(
        merged.history[0].visits, 5,
        "visits take the max, never a sum"
    );
    assert_eq!(merged.history[0].last_visited, 200);
    assert_eq!(merged.history[0].title, "New title");
}

#[test]
fn save_merged_preserves_the_other_processs_records() {
    // AUDIT 0.41.1 (Front B F9), end to end at the file.
    let dir = nav_tmp("f9");
    let path = dir.join("nav.json");
    let now = now_ms();
    let mut process_a = NavStore::default();
    process_a.record_visit("https://example.com/from-a", "A", now);
    save_to(&path, &process_a).expect("A saves");
    // Process B still holds its startup snapshot (empty)
    // plus its own new record; its save must not erase A.
    let mut process_b = NavStore::default();
    process_b.record_visit("https://example.com/from-b", "B", now);
    save_merged(&path, &process_b, now).expect("B saves merged");
    match load_from_checked(&path, now) {
        NavLoadOutcome::Loaded(s) => {
            let urls: Vec<&str> = s.history.iter().map(|e| e.url.as_str()).collect();
            assert!(urls.contains(&"https://example.com/from-a"), "{urls:?}");
            assert!(urls.contains(&"https://example.com/from-b"), "{urls:?}");
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sanitizer_prunes_bookmarks_by_added_at_and_clamps_the_future() {
    // AUDIT 0.41.1 (Front B S1 + S2).
    let now = 1_000_000_000_000u64;
    let mut s = NavStore::default();
    // Over the cap, added_at ASCENDING in file order —
    // file-position pruning would keep the OLDEST.
    for i in 0..(pico_core::core::navstore::MAX_BOOKMARKS + 5) {
        s.bookmarks.push(Bookmark {
            url: format!("https://example.com/{i}"),
            title: "T".into(),
            added_at: i as u64,
        });
    }
    s.history.push(HistoryEntry {
        url: "https://example.com/future".into(),
        title: "F".into(),
        last_visited: now + 999_999_999_999,
        visits: 1,
    });
    let s = s.sanitized(now);
    assert_eq!(s.bookmarks.len(), pico_core::core::navstore::MAX_BOOKMARKS);
    assert_eq!(
        s.bookmarks[0].added_at,
        (pico_core::core::navstore::MAX_BOOKMARKS + 4) as u64,
        "the newest-added survive, most recent first"
    );
    assert_eq!(s.history[0].last_visited, now, "future timestamps clamp");
}

#[test]
fn failed_navigations_are_not_recorded_but_http_errors_are() {
    // REWRITTEN 0.41.2: the 0.41.1 gate compared the
    // committed URL against the tab's "last failed
    // source" mark — see the witness test
    // `superseded_navigation_cannot_poison_the_next_record`
    // below for how that mark refused genuine visits.
    // The gate is now the navigation's OWN verdict:
    // (IsSuccess, HttpStatusCode) from its own
    // NavigationCompleted args. A failure that still
    // received an HTTP response (404/500) displayed a
    // real page — WebView2 reports it IsSuccess=false —
    // and records; a failure with no response (status
    // 0) displayed the browser's error page and does
    // not.
    use pico_core::core::navstore::verdict_records_visit;
    assert!(verdict_records_visit(true, 200));
    assert!(
        verdict_records_visit(true, 0),
        "local/succeeded loads record"
    );
    assert!(verdict_records_visit(false, 404), "a 404 page is a visit");
    assert!(verdict_records_visit(false, 500), "a 500 page is a visit");
    assert!(
        verdict_records_visit(false, 200),
        "a page that stopped its own load (window.stop) still displayed"
    );
    assert!(
        !verdict_records_visit(false, 0),
        "a transport failure (no response) is not a visit"
    );
}

// ---------------------------------------------------------------------------
// 0.41.2 — the history witness: first (bookmark) visit
// recorded, every later navigation refused. The record
// flow is modeled exactly as the shell runs it: a
// commit stashes the tab's pending candidate, the
// navigation's own verdict decides it, and a new
// navigation's start flushes a verdict-less candidate
// as a record.
// 0.41.3 — the harness runs against the id-keyed queue
// (PendingVisit::note_candidate / note_verdict /
// flush_all): each commit takes the next navigation
// id, each verdict is addressed to the id of the
// commit it answers, an evicted oldest candidate
// records (lost-verdict doctrine), and a start drains
// every queued candidate as a record.
// ---------------------------------------------------------------------------

use pico_core::core::navstore::{PendingVisit, VerdictOutcome};

struct RecordFlow {
    store: NavStore,
    pending: PendingVisit,
    now: u64,
    next_id: u64,
    last_id: u64,
}

impl RecordFlow {
    fn new() -> Self {
        RecordFlow {
            store: NavStore::default(),
            pending: PendingVisit::new(),
            now: now_ms(),
            next_id: 0,
            last_id: 0,
        }
    }
    fn commit(&mut self, url: &str) {
        self.next_id += 1;
        self.last_id = self.next_id;
        if let Some(evicted) = self.pending.note_candidate(self.last_id, url) {
            self.store.record_visit(&evicted, "", self.now);
        }
    }
    fn verdict(&mut self, success: bool, http_status: i32) {
        if let VerdictOutcome::Record(url) =
            self.pending
                .note_verdict(self.last_id, success, http_status)
        {
            self.store.record_visit(&url, "", self.now);
        }
    }
    fn start(&mut self) {
        for url in self.pending.flush_all() {
            self.store.record_visit(&url, "", self.now);
        }
    }
    fn urls(&self) -> Vec<String> {
        let mut v: Vec<String> = self.store.history.iter().map(|e| e.url.clone()).collect();
        v.sort();
        v
    }
}

#[test]
fn user_sequence_bookmark_then_two_navigations_all_record() {
    // The user's exact sequence: open a site from a
    // bookmark, then navigate to two more sites. All
    // three must be in history, persisted, and survive
    // a save/load cycle.
    let dir = nav_tmp("seq0412");
    let path = dir.join("nav.json");
    let mut flow = RecordFlow::new();
    flow.commit("https://www.youtube.com/");
    flow.verdict(true, 200);
    flow.commit("https://example.com/");
    flow.verdict(true, 200);
    flow.commit("https://rust-lang.org/");
    flow.verdict(true, 200);
    assert_eq!(
        flow.urls(),
        vec![
            "https://example.com/".to_string(),
            "https://rust-lang.org/".to_string(),
            "https://www.youtube.com/".to_string(),
        ]
    );
    save_merged(&path, &flow.store, flow.now).expect("save");
    match load_from_checked(&path, flow.now) {
        NavLoadOutcome::Loaded(s) => {
            let mut urls: Vec<String> = s.history.iter().map(|e| e.url.clone()).collect();
            urls.sort();
            assert_eq!(
                urls,
                vec![
                    "https://example.com/".to_string(),
                    "https://rust-lang.org/".to_string(),
                    "https://www.youtube.com/".to_string(),
                ],
                "all three visits survive the save/load cycle"
            );
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn superseded_navigation_cannot_poison_the_next_record() {
    // The witnessed 0.41.1 failure, replayed: site B
    // commits and is still loading when the user clicks
    // onward to C (on a slow connection a page is
    // clickable seconds before its load completes). B's
    // completion reports failure (aborted by the
    // navigate-away) — under 0.41.1 that verdict was
    // stamped with C's URL (the handler read the LIVE
    // Source) and C's record was refused by the mark.
    // Here the verdict carries no URL and decides only
    // B's own candidate; B had received its document
    // (status 200), so B records — and C records too.
    let mut flow = RecordFlow::new();
    flow.commit("https://www.youtube.com/");
    flow.verdict(true, 200);
    flow.commit("https://example.com/");
    flow.verdict(false, 200); // aborted mid-load, document received
    flow.commit("https://rust-lang.org/");
    flow.verdict(true, 200);
    assert_eq!(
        flow.urls(),
        vec![
            "https://example.com/".to_string(),
            "https://rust-lang.org/".to_string(),
            "https://www.youtube.com/".to_string(),
        ],
        "an aborted navigation's verdict must never refuse the next page"
    );
}

#[test]
fn failed_attempt_does_not_block_the_successful_retry() {
    // The 0.41.1 gate's second proven failure: a failed
    // attempt left the tab's mark equal to the URL, and
    // the user's successful retry of the SAME URL was
    // refused (the mark is consulted at the retry's
    // commit, before the retry's success could clear
    // it). The verdict design has no cross-navigation
    // state: the failure drops its own candidate only.
    let mut flow = RecordFlow::new();
    flow.commit("https://unreachable.example/");
    flow.verdict(false, 0); // DNS failure: error page, not a visit
    assert!(flow.urls().is_empty(), "the failure itself must not record");
    flow.commit("https://unreachable.example/");
    flow.verdict(true, 200); // the retry succeeds
    assert_eq!(
        flow.urls(),
        vec!["https://unreachable.example/".to_string()]
    );
}

#[test]
fn lost_verdict_degrades_to_recording_never_to_silence() {
    // If a navigation's verdict never arrives (the raw
    // registration failed, or a read failed), its
    // candidate is flushed as a record when the next
    // navigation starts — the page committed and
    // displayed. Nothing is silently lost.
    let mut flow = RecordFlow::new();
    flow.commit("https://example.com/");
    flow.start(); // next navigation begins; no verdict ever came
    flow.commit("https://rust-lang.org/");
    flow.verdict(true, 200);
    assert_eq!(
        flow.urls(),
        vec![
            "https://example.com/".to_string(),
            "https://rust-lang.org/".to_string(),
        ]
    );
}

// ---------------------------------------------------------------------------
// 0.41.3 — deletion tombstones (Front A F3): a second
// process's stale copy must not resurrect what another
// process cleared or removed. Two-store simulations at
// the file level: A and B start from the same saved
// store; B deletes and saves; A — still holding the
// pre-delete copy — records something new and saves
// MERGED. The deletion must win for everything at or
// before its timestamp; the genuinely new record must
// survive. Timestamps are explicit and kept in the
// past so the load-time clamp never rewrites them.
// ---------------------------------------------------------------------------

fn tomb_base() -> u64 {
    now_ms() - 100_000
}

fn history_urls(s: &NavStore) -> Vec<String> {
    let mut v: Vec<String> = s.history.iter().map(|e| e.url.clone()).collect();
    v.sort();
    v
}

fn load_ok(path: &std::path::Path, now: u64) -> NavStore {
    match load_from_checked(path, now) {
        NavLoadOutcome::Loaded(s) => *s,
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn cleared_history_stays_cleared_across_a_stale_merge() {
    let dir = nav_tmp("tombclear");
    let path = dir.join("nav.json");
    let t = tomb_base();
    let mut a = NavStore::default();
    assert!(a.record_visit("https://one.example/", "1", t + 1_000));
    assert!(a.record_visit("https://two.example/", "2", t + 2_000));
    save_to(&path, &a).expect("seed save");
    // B loads the same store, clears, saves.
    let mut b = load_ok(&path, t + 2_500);
    b.clear_history(t + 3_000);
    save_to(&path, &b).expect("clear save");
    // A, still holding the pre-clear copy, records one
    // NEW visit and saves merged.
    assert!(a.record_visit("https://three.example/", "3", t + 4_000));
    save_merged(&path, &a, t + 4_000).expect("merged save");
    let merged = load_ok(&path, t + 5_000);
    assert_eq!(
        history_urls(&merged),
        vec!["https://three.example/".to_string()],
        "cleared entries stay gone; the post-clear visit survives"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removed_history_entry_stays_removed_across_a_stale_merge() {
    let dir = nav_tmp("tombremove");
    let path = dir.join("nav.json");
    let t = tomb_base();
    let mut a = NavStore::default();
    assert!(a.record_visit("https://one.example/", "1", t + 1_000));
    assert!(a.record_visit("https://two.example/", "2", t + 2_000));
    save_to(&path, &a).expect("seed save");
    let mut b = load_ok(&path, t + 2_500);
    assert!(b.remove_history("https://one.example/", t + 3_000));
    save_to(&path, &b).expect("remove save");
    assert!(a.record_visit("https://three.example/", "3", t + 4_000));
    save_merged(&path, &a, t + 4_000).expect("merged save");
    let merged = load_ok(&path, t + 5_000);
    assert_eq!(
        history_urls(&merged),
        vec![
            "https://three.example/".to_string(),
            "https://two.example/".to_string(),
        ],
        "the removed entry stays gone; its sibling and the new visit survive"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removed_bookmark_stays_removed_across_a_stale_merge() {
    let dir = nav_tmp("tombbookmark");
    let path = dir.join("nav.json");
    let t = tomb_base();
    let mut a = NavStore::default();
    assert!(a.toggle_bookmark("https://keep.example/", "K", t + 1_000));
    assert!(a.toggle_bookmark("https://drop.example/", "D", t + 1_500));
    save_to(&path, &a).expect("seed save");
    let mut b = load_ok(&path, t + 2_500);
    assert!(b.remove_bookmark("https://drop.example/", t + 3_000));
    save_to(&path, &b).expect("remove save");
    // A's stale copy still holds both bookmarks; a new
    // visit + merged save must not resurrect the drop.
    assert!(a.record_visit("https://three.example/", "3", t + 4_000));
    save_merged(&path, &a, t + 4_000).expect("merged save");
    let merged = load_ok(&path, t + 5_000);
    let mut marks: Vec<String> = merged.bookmarks.iter().map(|b| b.url.clone()).collect();
    marks.sort();
    assert_eq!(marks, vec!["https://keep.example/".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn revisit_after_clear_survives_the_next_merge() {
    let dir = nav_tmp("tombrevisit");
    let path = dir.join("nav.json");
    let t = tomb_base();
    let mut a = NavStore::default();
    assert!(a.record_visit("https://one.example/", "1", t + 1_000));
    save_to(&path, &a).expect("seed save");
    let mut b = load_ok(&path, t + 2_500);
    b.clear_history(t + 3_000);
    save_to(&path, &b).expect("clear save");
    // The user visits the same page again AFTER the
    // clear: the fresh action supersedes the deletion.
    assert!(a.record_visit("https://one.example/", "1", t + 4_000));
    save_merged(&path, &a, t + 4_000).expect("merged save");
    let merged = load_ok(&path, t + 5_000);
    assert_eq!(
        history_urls(&merged),
        vec!["https://one.example/".to_string()],
        "a visit after the clear is a new fact, not a resurrection"
    );
    assert_eq!(merged.history[0].last_visited, t + 4_000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_format_nav_json_loads_without_tombstone_fields() {
    // AUDIT 0.41.3 (F3): nav.json written before the
    // tombstone fields existed has neither key — serde
    // defaults must load it with entries intact and an
    // empty deletion state.
    let dir = nav_tmp("tomboldfmt");
    let path = dir.join("nav.json");
    let t = tomb_base();
    let json = format!(
        r#"{{"bookmarks":[{{"url":"https://mark.example/","title":"M","added_at":{}}}],"history":[{{"url":"https://hist.example/","title":"H","last_visited":{},"visits":3}}]}}"#,
        t + 1_000,
        t + 2_000
    );
    std::fs::write(&path, json).expect("write old-format nav.json");
    let s = load_ok(&path, t + 5_000);
    assert_eq!(s.bookmarks.len(), 1);
    assert_eq!(s.history.len(), 1);
    assert_eq!(s.history[0].visits, 3);
    assert_eq!(s.history_cleared_at, 0);
    assert!(s.tombstones.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn canonical_key_unifies_bare_host_and_slash() {
    // AUDIT 0.41.3 (Front A F7): an empty path tail
    // normalizes to "/", so the bare host and the
    // slashed spelling are ONE entry, not two.
    assert_eq!(
        canonical_nav_key("https://example.com"),
        canonical_nav_key("https://example.com/")
    );
    assert_eq!(
        canonical_nav_key("https://example.com"),
        "https://example.com/"
    );
    assert_eq!(
        canonical_nav_key("https://example.com?x=1"),
        "https://example.com/?x=1"
    );
    let mut s = NavStore::default();
    let now = now_ms();
    assert!(s.record_visit("https://example.com", "A", now));
    assert!(s.record_visit("https://example.com/", "A", now + 1));
    assert_eq!(s.history.len(), 1, "bare and slashed are one visit");
    assert_eq!(s.history[0].visits, 2);
}
