//! 0.30 savings statistics: the reconciliation contract and
//! the protocol/persistence seams.
//!
//! The reconciliation test is the accuracy core of the
//! feature: it drives the REAL `filterpipe::decide` over a
//! fixture corpus, feeds each verdict into the book exactly
//! as the request handler does, and asserts the band-bound
//! totals equal an independently summed expectation — every
//! block counted once, attributed to the engine the verdict
//! named, credited from the calibrated ledger, and loopback
//! contributing exactly zero.

use pico_core::core::action::{dispatch, ActionSource, Decision, Requester};
use pico_core::core::filterpipe::{decide, FilterState, RequestFacts, Verdict};
use pico_core::core::ipc::{looks_like_shield_pruned, parse_shield_pruned};
use pico_core::core::shield::{ResourceType, ShieldEngine};
use pico_core::core::state::PersistState;
use pico_core::core::stats::{
    credit_bytes, credit_duration_ms, Engine, EngineStats, Savings, StatsBook, TypeLedger,
    MIN_SAMPLES,
};
use ResourceType::*;

fn facts(url: &str, page: &str, rtype: ResourceType) -> RequestFacts {
    RequestFacts {
        url: url.to_string(),
        page_url: page.to_string(),
        rtype,
        method: Some("GET".to_string()),
        page_watch_shaped: pico_core::core::lite::is_watch_shaped_url(page),
        tab_chosen: false,
    }
}

const BOTH_ON: FilterState = FilterState {
    shield_on: true,
    lite_on: true,
};

/// Calibrate the book's ledger with uniform samples so every
/// creditable type's credit is a known constant.
fn calibrated_book() -> StatsBook {
    let book = StatsBook::new();
    for (rt, bytes, ms) in [
        (Script, 31_000u64, 610u64),
        (Image, 51_000, 810),
        (Stylesheet, 16_000, 410),
        (Xhr, 21_000, 510),
        (Media, 410_000, 2_510),
        (Font, 41_000, 710),
        (Other, 11_000, 410),
    ] {
        for _ in 0..MIN_SAMPLES {
            book.record_sample(rt, Some(bytes), Some(ms));
        }
    }
    book
}

#[test]
fn band_totals_reconcile_with_decide_verdicts() {
    let engine = ShieldEngine::compile(&[
        "||ads.example.com^",
        "/banner-ad.",
        "||tracker.example^$document",
    ]);
    let book = calibrated_book();
    let ledger = book.lifetime().ledger;

    let page = "https://news.example/story";
    let watch = "https://videos.example/watch?v=1";
    let mut chosen_tab = facts("https://cdn.example/media/movie.mp4", page, Media);
    chosen_tab.tab_chosen = true;

    let corpus: Vec<RequestFacts> = vec![
        // Shield's: a rule-hit image and a generic-rule script.
        facts("https://ads.example.com/banner.png", page, Image),
        facts("https://cdn.example/banner-ad.js", page, Script),
        // A $document block: counted, credited zero.
        facts("https://tracker.example/pop", page, Document),
        // Lite's: image/font/media classes on a plain page,
        // plus an MSE stream shape through XHR.
        facts("https://cdn.example/pic.png", page, Image),
        facts("https://cdn.example/font.woff2", page, Font),
        facts("https://cdn.example/media/movie.mp4", page, Media),
        facts("https://cdn.example/hls/master.m3u8", page, Xhr),
        // Allowed: the watch page's own video bytes, the
        // chosen tab's media, an ordinary script, the
        // innertube JSON (Shield prune territory, never a
        // network block).
        facts("https://cdn.example/media/movie.mp4", watch, Media),
        facts("https://rr.example/videoplayback?id=x", watch, Xhr),
        chosen_tab,
        facts("https://cdn.example/app.js", page, Script),
        facts(
            "https://www.youtube.com/youtubei/v1/player?key=k",
            page,
            Xhr,
        ),
        // Loopback is sacred in both directions: rule-hit
        // URLs that must contribute NOTHING.
        facts(
            "https://ads.example.com/banner.png",
            "http://127.0.0.1:5173/",
            Image,
        ),
        facts("http://localhost:9000/banner-ad.js", page, Script),
        facts("https://ads.example.com/banner.png", "pico://start", Image),
    ];

    // The independent expectation: sum the verdicts by hand,
    // crediting from the same calibrated ledger.
    let mut want_shield = EngineStats::default();
    let mut want_lite = EngineStats::default();
    for f in &corpus {
        match decide(f, BOTH_ON, Some(&engine)) {
            Verdict::Allow => {}
            Verdict::ShieldBlock(rt) => {
                want_shield.blocked += 1;
                want_shield.est_bytes += credit_bytes(&ledger, rt);
                want_shield.est_duration_ms += credit_duration_ms(&ledger, rt);
                book.record_block(Engine::Shield, rt, None);
            }
            Verdict::LiteBlock(rt) => {
                want_lite.blocked += 1;
                want_lite.est_bytes += credit_bytes(&ledger, rt);
                want_lite.est_duration_ms += credit_duration_ms(&ledger, rt);
                book.record_block(Engine::Lite, rt, None);
            }
        }
    }

    // Sanity: the corpus actually exercised both engines and
    // the zero-credit document path.
    assert_eq!(want_shield.blocked, 3, "shield corpus blocks");
    assert_eq!(want_lite.blocked, 4, "lite corpus blocks");
    assert!(want_shield.est_bytes > 0 && want_lite.est_bytes > 0);

    let view = book.view();
    assert_eq!(view.shield.blocked, want_shield.blocked);
    assert_eq!(view.shield.est_bytes, want_shield.est_bytes);
    assert_eq!(view.shield.est_duration_ms, want_shield.est_duration_ms);
    assert_eq!(view.lite.blocked, want_lite.blocked);
    assert_eq!(view.lite.est_bytes, want_lite.est_bytes);
    assert_eq!(view.lite.est_duration_ms, want_lite.est_duration_ms);
    // Fresh book: session == lifetime.
    assert_eq!(view.shield.session_blocked, want_shield.blocked);
    assert_eq!(view.lite.session_blocked, want_lite.blocked);
    // The combined time-saved line is the sum of both engines.
    assert_eq!(
        view.time_saved_ms(),
        want_shield.est_duration_ms + want_lite.est_duration_ms
    );
    // Known-constant spot check (calibration uniforms): the
    // shield's image earned the Image average, its document
    // earned nothing.
    assert_eq!(
        view.shield.est_bytes,
        51_000 + 31_000,
        "image + script credits; the document block credits 0"
    );
}

#[test]
fn toggling_engines_off_stops_their_accrual() {
    // The handler only records a verdict decide produced;
    // with an engine off, decide never attributes to it. Pin
    // the composition end: shield off → the rule-hit image is
    // Lite's; both off → nothing is recorded at all.
    let engine = ShieldEngine::compile(&["||ads.example.com^"]);
    let book = StatsBook::new();
    let f = facts(
        "https://ads.example.com/banner.png",
        "https://news.example/",
        Image,
    );
    match decide(
        &f,
        FilterState {
            shield_on: false,
            lite_on: true,
        },
        Some(&engine),
    ) {
        Verdict::LiteBlock(rt) => book.record_block(Engine::Lite, rt, None),
        other => panic!("expected LiteBlock, got {other:?}"),
    }
    match decide(
        &f,
        FilterState {
            shield_on: false,
            lite_on: false,
        },
        Some(&engine),
    ) {
        Verdict::Allow => {}
        other => panic!("expected Allow, got {other:?}"),
    }
    let view = book.view();
    assert_eq!(view.shield.blocked, 0);
    assert_eq!(view.lite.blocked, 1);
}

#[test]
fn shield_pruned_ipc_shape_is_exact() {
    assert!(looks_like_shield_pruned(
        "{\"kind\":\"shield_pruned\",\"bytes\":10,\"events\":2}"
    ));
    let r = parse_shield_pruned("{\"kind\":\"shield_pruned\",\"bytes\":10,\"events\":2}")
        .expect("valid report");
    assert_eq!(r.bytes, 10);
    assert_eq!(r.events, 2);
    // Zero is a legal report (subtrees whose serialization
    // was empty still count as events).
    assert!(parse_shield_pruned("{\"kind\":\"shield_pruned\",\"bytes\":0,\"events\":3}").is_some());
    // Bounds.
    assert!(parse_shield_pruned(
        "{\"kind\":\"shield_pruned\",\"bytes\":16777216,\"events\":100000}"
    )
    .is_some());
    assert!(
        parse_shield_pruned("{\"kind\":\"shield_pruned\",\"bytes\":16777217,\"events\":1}")
            .is_none()
    );
    assert!(
        parse_shield_pruned("{\"kind\":\"shield_pruned\",\"bytes\":1,\"events\":100001}").is_none()
    );
    // Unknown fields are denied — nothing rides along.
    assert!(parse_shield_pruned(
        "{\"kind\":\"shield_pruned\",\"bytes\":1,\"events\":1,\"url\":\"https://x\"}"
    )
    .is_none());
    // Wrong kind, missing fields, negative and float payloads.
    assert!(parse_shield_pruned("{\"kind\":\"shield_prune\",\"bytes\":1,\"events\":1}").is_none());
    assert!(parse_shield_pruned("{\"kind\":\"shield_pruned\",\"bytes\":1}").is_none());
    assert!(
        parse_shield_pruned("{\"kind\":\"shield_pruned\",\"bytes\":-1,\"events\":1}").is_none()
    );
    assert!(
        parse_shield_pruned("{\"kind\":\"shield_pruned\",\"bytes\":1.5,\"events\":1}").is_none()
    );
    assert!(parse_shield_pruned("not json").is_none());
}

#[test]
fn stats_reset_rides_the_gated_dispatch() {
    let start = Requester {
        is_start_tab: true,
        current_url: "pico://start".to_string(),
    };
    // Both transports reach the same decision.
    assert_eq!(
        dispatch(
            &ActionSource::Ipc("{\"kind\":\"action\",\"action\":\"stats-reset\"}".into()),
            &start,
            &[]
        ),
        Decision::ResetStats
    );
    assert_eq!(
        dispatch(&ActionSource::Url("pico://stats-reset".into()), &start, &[]),
        Decision::ResetStats
    );
    // The wry-rewritten spelling folds back too (AUDIT 0.30,
    // A1/A4: normalize_pico_url carries stats-reset).
    assert_eq!(
        dispatch(
            &ActionSource::Url("https://pico.stats-reset/".into()),
            &start,
            &[]
        ),
        Decision::ResetStats
    );
    // The gate is conjunctive as always: wrong tab or wrong
    // page denies before parsing.
    let other_tab = Requester {
        is_start_tab: false,
        current_url: "pico://start".to_string(),
    };
    assert!(matches!(
        dispatch(
            &ActionSource::Ipc("{\"kind\":\"action\",\"action\":\"stats-reset\"}".into()),
            &other_tab,
            &[]
        ),
        Decision::Deny(_)
    ));
    let navigated = Requester {
        is_start_tab: true,
        current_url: "https://example.com/".to_string(),
    };
    assert!(matches!(
        dispatch(
            &ActionSource::Url("pico://stats-reset".into()),
            &navigated,
            &[]
        ),
        Decision::Deny(_)
    ));
}

#[test]
fn stats_persist_roundtrip_and_pre_030_files_load_zero() {
    let mut state = PersistState::default();
    state.stats.shield.blocked = 7;
    state.stats.shield.est_bytes = 123_456;
    state.stats.ledger[3] = TypeLedger {
        total_bytes: 1_020_000,
        byte_samples: 20,
        total_duration_ms: 16_200,
        duration_samples: 20,
    };
    let text = serde_json::to_string(&state).expect("serialize");
    let back: PersistState = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(back.stats.shield.blocked, 7);
    assert_eq!(back.stats.ledger[3].byte_samples, 20);

    // A pre-0.30 state file (no stats key at all) loads with
    // a zero book — the structural migration every field uses.
    let legacy = "{\"version\":3,\"theme\":\"dark\",\"recents\":[],\"shield_on\":true,\"blocked_total\":42,\"lite_on\":false,\"theme_id\":\"forest\"}";
    let loaded: PersistState = serde_json::from_str(legacy).expect("legacy loads");
    assert!(loaded.stats.is_zero());
    assert_eq!(loaded.blocked_total, 42, "the seed source survives");

    // Sanitization on the load path: a hand-inflated book is
    // capped at what its samples support.
    let mut wild = PersistState::default();
    wild.stats.ledger[3].total_bytes = u64::MAX;
    wild.stats.ledger[3].byte_samples = 1;
    let clean = wild.sanitized();
    assert_eq!(
        clean.stats.ledger[3].total_bytes,
        pico_core::core::stats::MAX_SAMPLE_BYTES
    );
}

#[test]
fn partial_stats_objects_load_with_zeroed_siblings() {
    // AUDIT 0.30.1: every stats field carries a serde
    // default, so a state file whose stats object is missing
    // sibling fields — hand-trimmed, or written by a build
    // that knew fewer fields — loads with the missing pieces
    // at zero instead of failing the WHOLE state parse.
    let text = "{\"version\":3,\"theme\":\"dark\",\"recents\":[],\"shield_on\":true,\"blocked_total\":3,\"lite_on\":false,\"theme_id\":\"forest\",\"stats\":{\"shield\":{\"blocked\":5}}}";
    let loaded: PersistState = serde_json::from_str(text).expect("partial stats must load");
    assert_eq!(loaded.stats.shield.blocked, 5);
    assert_eq!(loaded.stats.shield.est_bytes, 0);
    assert_eq!(loaded.stats.shield.pruned_bytes, 0);
    assert!(loaded.stats.lite == pico_core::core::stats::EngineStats::default());
    assert!(loaded
        .stats
        .ledger
        .iter()
        .all(|l| *l == pico_core::core::stats::TypeLedger::default()));
    // A ledger slot missing its sibling fields behaves the
    // same way. (The array's LENGTH stays structural — serde
    // fixed arrays, like the schema around them; the 0.30.1
    // fix is about missing fields, not malformed arrays.)
    let text = "{\"version\":3,\"theme\":\"dark\",\"stats\":{\"ledger\":[{\"total_bytes\":1000},{},{},{},{},{},{},{},{},{},{}]}}";
    let loaded: PersistState = serde_json::from_str(text).expect("partial ledger must load");
    assert_eq!(loaded.stats.ledger[0].total_bytes, 1000);
    assert_eq!(loaded.stats.ledger[0].byte_samples, 0);
    // (Sanitization then caps the unsupported total at zero —
    // the load itself is what this test pins.)
    let clean = loaded.sanitized();
    assert_eq!(clean.stats.ledger[0].total_bytes, 0);
}

#[test]
fn seed_carries_the_legacy_count_exactly_once() {
    let book = StatsBook::new();
    // AUDIT 0.31 (A5-L4): the seed reports whether it applied.
    assert!(
        book.seed_shield_blocked(42),
        "seed into an empty book applies"
    );
    assert_eq!(book.lifetime().shield.blocked, 42);
    // A book that already has figures is never seeded over.
    book.record_block(Engine::Shield, Image, None);
    assert!(
        !book.seed_shield_blocked(999),
        "a non-empty book refuses the seed"
    );
    assert_eq!(book.lifetime().shield.blocked, 43);
    // A zero legacy total seeds nothing either.
    let fresh = StatsBook::new();
    assert!(!fresh.seed_shield_blocked(0));
    // The seed is count-only: bytes/time were never measured.
    let lt: Savings = book.lifetime();
    assert_eq!(
        lt.shield.est_bytes, 50_000,
        "only the new block's (uncalibrated default) credit"
    );
    assert_eq!(lt.shield.est_duration_ms, 800);
}

#[test]
fn prune_flush_caps_events_like_bytes_and_keeps_the_remainder() {
    // AUDIT 0.30 (A2-C1): the runtime flush must chunk BOTH
    // fields at the host's per-report ceilings and subtract
    // only what it sent — an uncapped events field makes the
    // host reject the whole report after the runtime has
    // already discarded the accumulators.
    let script =
        std::fs::read_to_string(format!("{}/src/core/shield.rs", env!("CARGO_MANIFEST_DIR")))
            .expect("read src/core/shield.rs");
    assert!(
        script.contains("Math.min(prunedEvents, 100000)"),
        "events must be chunked at MAX_PRUNE_REPORT_EVENTS"
    );
    assert!(
        script.contains("prunedEvents -= sendEvents"),
        "only the sent events may be subtracted"
    );
    assert!(
        script.contains("Math.min(prunedBytes, 16777216)"),
        "bytes chunk at MAX_PRUNE_REPORT_BYTES"
    );
}

#[test]
fn snapshot_version_comes_from_the_display_version_constant() {
    // AUDIT 0.30 (A1-C2): the badge shows the snapshot's
    // version verbatim, so the snapshot must be BUILT from
    // core::DISPLAY_VERSION — never from CARGO_PKG_VERSION
    // (which would print the raw Cargo string while the
    // start page and log say the display version).
    let app = std::fs::read_to_string(format!("{}/src/app.rs", env!("CARGO_MANIFEST_DIR")))
        .expect("read src/app.rs");
    assert!(
        app.contains("version: DISPLAY_VERSION,"),
        "the snapshot's version field must be DISPLAY_VERSION"
    );
}

#[test]
fn band_lives_only_on_the_start_page() {
    // The savings band is a start-page surface, full stop:
    // the assembled start page carries it, the assembled
    // chrome carries no trace of it (no new chrome UI).
    let start = pico_core::core::assets::assemble_start_html("{}");
    for id in [
        "stats-grid",
        "stat-shield",
        "stat-lite",
        "stat-time",
        "stat-zero",
        "stats-reset",
    ] {
        assert!(
            start.contains(&format!("id=\"{id}\"")),
            "start page missing #{id}"
        );
    }
    assert!(start.contains("__picoSetStats"), "stats hook missing");
    let chrome = pico_core::core::assets::assemble_chrome_html();
    assert!(
        !chrome.contains("stat-shield") && !chrome.contains("__picoSetStats"),
        "the band leaked into the chrome"
    );
}

// ---------------------------------------------------------------------
// AUDIT 0.31 fixes (A2-F9, A5-M1, A5-L13)
// ---------------------------------------------------------------------

#[test]
fn note_response_validates_its_byte_sample() {
    // AUDIT 0.31 (A2-F9/A2-F17): note_response must apply the
    // same byte validation as its siblings — 0 and
    // >MAX_SAMPLE_BYTES are treated as None INSIDE
    // note_response, on both the correlated and the
    // bucket-fallback paths.
    use pico_core::core::stats::{type_index, MAX_SAMPLE_BYTES};
    let book = StatsBook::new();
    // Correlated path: out-of-range bytes teach nothing, but
    // a valid duration on the same response still counts.
    book.note_request(1, Script, 1_000);
    book.note_response(1, Some(0), Some("text/html"), 1_300);
    book.note_request(2, Script, 2_000);
    book.note_response(2, Some(MAX_SAMPLE_BYTES + 1), Some("text/html"), 2_300);
    let lt = book.lifetime();
    let script = lt.ledger[type_index(Script)];
    assert_eq!(script.byte_samples, 0, "invalid bytes never land");
    assert_eq!(script.total_bytes, 0);
    assert_eq!(script.duration_samples, 2, "durations unaffected");
    // Fallback path: an uncorrelated response with invalid
    // bytes teaches nothing even with a bucketable type.
    book.note_response(99, Some(0), Some("image/png"), 3_000);
    book.note_response(98, Some(MAX_SAMPLE_BYTES + 1), Some("image/png"), 3_000);
    let lt = book.lifetime();
    assert_eq!(lt.ledger[type_index(Image)].byte_samples, 0);
    // And the boundaries themselves are valid samples.
    book.note_response(97, Some(MAX_SAMPLE_BYTES), Some("image/png"), 3_000);
    book.note_response(96, Some(1), Some("image/png"), 3_000);
    let image = book.lifetime().ledger[type_index(Image)];
    assert_eq!(image.byte_samples, 2);
    assert_eq!(image.total_bytes, MAX_SAMPLE_BYTES + 1);
}

#[test]
fn head_responses_do_not_teach_the_ledger() {
    // AUDIT 0.31 (A2-F9): a HEAD response's Content-Length
    // describes a body never transferred; its sample must
    // not calibrate bytes OR duration. The correlation entry
    // is still consumed.
    use pico_core::core::stats::type_index;
    let book = StatsBook::new();
    book.note_request_with_method(7, Image, 1_000, Some("HEAD"));
    book.note_response(7, Some(80_000), Some("image/png"), 1_400);
    let lt = book.lifetime();
    assert!(
        lt.ledger.iter().all(|l| *l == TypeLedger::default()),
        "a HEAD response taught the ledger: {:?}",
        lt.ledger[type_index(Image)]
    );
    // Consumed: the same token again is a correlation MISS,
    // so its bytes fall back to the Content-Type bucket.
    book.note_response(7, Some(80_000), Some("image/png"), 2_000);
    let image = book.lifetime().ledger[type_index(Image)];
    assert_eq!(image.byte_samples, 1, "second response hit the fallback");
    // The method match is case-insensitive, and non-HEAD
    // methods through the same entry point teach normally.
    let book2 = StatsBook::new();
    book2.note_request_with_method(1, Image, 0, Some("head"));
    book2.note_response(1, Some(10_000), None, 100);
    assert_eq!(
        book2.lifetime().ledger[type_index(Image)].byte_samples,
        0,
        "lowercase head is still HEAD"
    );
    book2.note_request_with_method(2, Image, 0, Some("GET"));
    book2.note_response(2, Some(10_000), None, 100);
    let image = book2.lifetime().ledger[type_index(Image)];
    assert_eq!(image.byte_samples, 1);
    assert_eq!(image.duration_samples, 1);
}

#[test]
fn malformed_stats_subtree_never_fails_the_state_parse() {
    // AUDIT 0.31 (A5-M1): every one of these state files has
    // a broken stats subtree; the parse must succeed, the
    // stats must land on sane defaults, and the OTHER fields
    // must survive intact.
    let base = |stats: &str| {
        format!(
            "{{\"version\":3,\"theme\":\"dark\",\"recents\":[],\"shield_on\":true,\"blocked_total\":42,\"lite_on\":false,\"theme_id\":\"forest\",\"stats\":{stats}}}"
        )
    };
    // stats: null → a zero book.
    let loaded: PersistState = serde_json::from_str(&base("null")).expect("stats:null loads");
    assert!(loaded.stats.is_zero());
    assert_eq!(loaded.blocked_total, 42, "sibling fields preserved");
    assert_eq!(loaded.theme_id, "forest");
    // engine: null → that engine defaults, the other survives.
    let loaded: PersistState =
        serde_json::from_str(&base("{\"shield\":null,\"lite\":{\"blocked\":3}}"))
            .expect("engine:null loads");
    assert_eq!(loaded.stats.shield, EngineStats::default());
    assert_eq!(loaded.stats.lite.blocked, 3);
    // Wrong-typed scalars → the field's default, siblings kept.
    for bad in ["-1", "1.5", "\"5\"", "true", "[1]"] {
        let text = base(&format!(
            "{{\"shield\":{{\"blocked\":{bad},\"est_bytes\":7}}}}"
        ));
        let loaded: PersistState =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("blocked:{bad} loads: {e}"));
        assert_eq!(loaded.stats.shield.blocked, 0, "blocked:{bad} → default");
        assert_eq!(loaded.stats.shield.est_bytes, 7, "valid sibling preserved");
        assert_eq!(loaded.blocked_total, 42);
    }
    // Ledger arrays of the wrong length: short pads with
    // zero slots, long truncates at exactly TYPE_COUNT.
    let short = base(
        "{\"ledger\":[{\"total_bytes\":1000,\"byte_samples\":2},{\"total_bytes\":2000},{\"total_bytes\":3000}]}",
    );
    let loaded: PersistState = serde_json::from_str(&short).expect("short ledger loads");
    assert_eq!(loaded.stats.ledger.len(), 11);
    assert_eq!(loaded.stats.ledger[0].total_bytes, 1000);
    assert_eq!(loaded.stats.ledger[0].byte_samples, 2);
    assert_eq!(loaded.stats.ledger[2].total_bytes, 3000);
    assert!(
        loaded.stats.ledger[3..]
            .iter()
            .all(|l| *l == TypeLedger::default()),
        "padded slots are zero"
    );
    let mut entries = String::new();
    for i in 0..14 {
        if i > 0 {
            entries.push(',');
        }
        entries.push_str(&format!("{{\"total_bytes\":{}}}", (i + 1) * 100));
    }
    let long = base(&format!("{{\"ledger\":[{entries}]}}"));
    let loaded: PersistState = serde_json::from_str(&long).expect("long ledger loads");
    assert_eq!(loaded.stats.ledger.len(), 11);
    assert_eq!(loaded.stats.ledger[10].total_bytes, 1100, "first 11 kept");
    // Ledger entries that are null / wrong-typed default too.
    let loaded: PersistState =
        serde_json::from_str(&base("{\"ledger\":[null,{\"byte_samples\":\"x\"}]}"))
            .expect("null ledger entries load");
    assert_eq!(loaded.stats.ledger[0], TypeLedger::default());
    assert_eq!(loaded.stats.ledger[1], TypeLedger::default());
    // Round-trip is unchanged: a serialized book parses back
    // to itself through the tolerant reader.
    let mut state = PersistState::default();
    state.stats.shield.blocked = 9;
    state.stats.ledger[3].total_bytes = 5_000;
    let text = serde_json::to_string(&state).expect("serialize");
    let back: PersistState = serde_json::from_str(&text).expect("own output re-loads");
    assert_eq!(back.stats, state.stats);
}

#[test]
fn humanize_bytes_reaches_eb_at_the_sanitize_cap() {
    // AUDIT 0.31 (A5-L13): the engine figures cap at 2^62;
    // that value must print in EB, not as thousands of PB.
    assert_eq!(pico_core::core::stats::humanize_bytes(1 << 62), "4.61 EB");
    // The PB range still prints as PB below the EB step.
    assert_eq!(
        pico_core::core::stats::humanize_bytes(5_500_000_000_000_000),
        "5.5 PB"
    );
}
