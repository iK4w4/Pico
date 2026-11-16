//! Equivalence evidence for the 0.41.2 measured-perf pass.
//!
//! Every optimization in this pass is behavior-preserving by
//! construction; these tests PROVE it the way the codebase's
//! generated-input harnesses do — old algorithm vs new
//! algorithm over generated inputs, compared exactly:
//!
//! * SHIELD: the golden decision strings and cosmetic-script
//!   digests below were produced by the PRISTINE 0.41.1
//!   engine (a throwaway generator run against an untouched
//!   source copy) over the corpus defined in this file. The
//!   optimized engine must reproduce every decision and the
//!   full 341,612-byte cosmetic script digest-for-digest.
//! * NAVSTORE: the pre-0.41.2 algorithms (`sanitized`,
//!   `merge_nav_stores`, `suggestions`, `search_history`
//!   with the per-entry-lowercasing matcher) are re-stated
//!   here as reference implementations over the unchanged
//!   public helpers, and run against the optimized ones
//!   over LCG-generated stores full of canonical-key
//!   collisions, spelling variants, unrecordable URLs and
//!   Unicode titles/queries.

use pico_core::core::navstore::{
    canonical_nav_key, fallback_title, frecency_score, host_of, is_recordable_url,
    merge_nav_stores, Bookmark, HistoryEntry, NavStore, Suggestion, SuggestionKind, MAX_BOOKMARKS,
    MAX_HISTORY, MAX_NAV_TITLE_CHARS, NAV_SCHEMA_VERSION,
};
use pico_core::core::shield::{Decision, ResourceType, ShieldEngine};

// ---------------------------------------------------------------------------
// Shield goldens (pristine 0.41.1 outputs)
// ---------------------------------------------------------------------------

fn mark(d: Decision) -> char {
    if d.is_block() {
        'B'
    } else {
        'A'
    }
}

/// Deterministic FNV-1a (DefaultHasher's keys are random per
/// process, so its digests cannot be golden values).
fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Split a built cosmetic script into (runtime digest, data
/// digest). The PRISTINE engine itself serializes one
/// HashMap (generic exceptions) in per-process random key
/// order, so raw script bytes differ even between two runs
/// of the old engine; the observable equivalence is the
/// runtime bytes verbatim plus the data payload as parsed
/// JSON — object key order free (the in-page runtime only
/// does property lookups), array order exact (host lists
/// and selector lists keep their order). The goldens below
/// were captured in this canonical form from the pristine
/// engine, twice, with identical results.
fn script_digests(script: &str) -> (u64, u64) {
    let at = script.find("var D = ").expect("data marker") + "var D = ".len();
    let mut stream =
        serde_json::Deserializer::from_str(&script[at..]).into_iter::<serde_json::Value>();
    let value = stream.next().expect("data value").expect("data parses");
    let end = at + stream.byte_offset();
    let runtime = format!("{}__DATA__{}", &script[..at], &script[end..]);
    let canonical = serde_json::to_string(&value).expect("reserialize");
    (fnv1a(&runtime), fnv1a(&canonical))
}

// (Public edition: the full-lists golden test of the
// dev tree has no counterpart here — this edition
// ships no embedded lists. The synthetic goldens
// below cover the same engine paths, including the
// page-flag partition.)

/// Synthetic lists exercising every page-flag kind in both
/// spellings — the partition's correctness case (the shipped
/// lists carry no genericblock/elemhide rules at all).
fn synth_lists() -> Vec<&'static str> {
    vec![
        "/banner.js\n\
         @@/banner.js$domain=good.example\n\
         ||ads.example^\n\
         ||ads.example/banner.js$important\n\
         @@||ads.example/banner.js$important,domain=good.example\n\
         *$genericblock,domain=news.example\n\
         *$genericblock\n\
         tracker.example##.adbox\n\
         ~foo.example,bar.example##.adbox\n\
         example.com#$#generichide\n\
         hide.example#$#elemhide\n\
         #$#generichide\n\
         ||tvtoday.de^$ghide\n\
         /method.js$method=POST\n\
         /to.js$to=tracker.example\n\
         generic.example##.generic-ad\n\
         ##.global-ad\n\
         plain.example#@#.global-ad\n",
    ]
}

const SYNTH_GOLDEN: &str = "AAAAABBABBBBABBAAAAAAAAAAAAAAAAAAAAAAAAABBABBAAAAAAAAAAAAAAAAAAAABBABBBBABBAAAAAAAAAAAAAAAAAAAABBABBBBABBAAAAAAAAAAAAAAAAAAAABBABBBBABBAAAAAAAAAAAAAAAAAAAABBABBBBABBAAAAAAAAAAAAAAA";

#[test]
fn shield_synthetic_flag_lists_match_pristine_golden() {
    let synth = ShieldEngine::compile(&synth_lists());
    let pages = [
        "https://news.example/",
        "https://good.example/",
        "https://plain.example/",
        "https://example.com/",
        "https://hide.example/",
        "https://tvtoday.de/",
    ];
    let reqs = [
        "https://cdn.example.org/banner.js",
        "https://ads.example/banner.js",
        "https://ads.example/other.js",
        "https://x.tracker.example/method.js",
        "https://x.tracker.example/to.js",
        "https://other.example/to.js",
    ];
    let mut out = String::new();
    for p in pages {
        for u in reqs {
            for rt in [
                ResourceType::Script,
                ResourceType::Image,
                ResourceType::Document,
            ] {
                out.push(mark(synth.decide(u, p, rt)));
            }
            out.push(mark(synth.decide_with_method(
                u,
                p,
                ResourceType::Script,
                Some("POST"),
            )));
            out.push(mark(synth.decide_with_method(
                u,
                p,
                ResourceType::Script,
                Some("GET"),
            )));
        }
    }
    assert_eq!(out, SYNTH_GOLDEN);
    for (budget, len, data_hash, hosts) in [
        (
            344_000usize,
            3905usize,
            11_027_238_329_461_350_098u64,
            3usize,
        ),
        (1_000, 3842, 5_748_750_442_745_851_440, 1),
        (0, 3842, 5_748_750_442_745_851_440, 1),
    ] {
        let (script, got_hosts, generic) = synth.build_cosmetic_script(budget);
        assert_eq!(script.len(), len, "budget {budget}");
        let (runtime, data) = script_digests(&script);
        assert_eq!(runtime, 5_382_498_930_382_958_403, "budget {budget}");
        assert_eq!(data, data_hash, "budget {budget}");
        assert_eq!(got_hosts, hosts, "budget {budget}");
        assert_eq!(generic, 1, "budget {budget}");
    }
    let rep = synth.report();
    assert_eq!(rep.network_rules, 5);
    assert_eq!(rep.network_exceptions, 2);
    assert_eq!(rep.page_flag_rules, 6);
    assert_eq!(rep.cosmetic_generic, 1);
    assert_eq!(rep.cosmetic_specific_hosts, 3);
}

// ---------------------------------------------------------------------------
// NavStore reference implementations (the pre-0.41.2 algorithms)
// ---------------------------------------------------------------------------

fn clean_title_ref(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAV_TITLE_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn sanitized_ref(mut store: NavStore, now_ms: u64) -> NavStore {
    store.version = NAV_SCHEMA_VERSION;
    let mut bookmarks: Vec<Bookmark> = Vec::new();
    for mut b in store.bookmarks {
        if !is_recordable_url(&b.url) {
            continue;
        }
        if bookmarks
            .iter()
            .any(|x: &Bookmark| canonical_nav_key(&x.url) == canonical_nav_key(&b.url))
        {
            continue;
        }
        b.title = clean_title_ref(&b.title);
        if b.title.is_empty() {
            b.title = fallback_title(&b.url);
        }
        b.added_at = b.added_at.min(now_ms);
        bookmarks.push(b);
    }
    bookmarks.sort_by(|a, b| b.added_at.cmp(&a.added_at).then_with(|| a.url.cmp(&b.url)));
    bookmarks.truncate(MAX_BOOKMARKS);
    store.bookmarks = bookmarks;

    let mut history: Vec<HistoryEntry> = Vec::new();
    for mut e in store.history {
        if !is_recordable_url(&e.url) {
            continue;
        }
        e.title = clean_title_ref(&e.title);
        if e.title.is_empty() {
            e.title = fallback_title(&e.url);
        }
        if e.visits == 0 {
            e.visits = 1;
        }
        e.last_visited = e.last_visited.min(now_ms);
        if let Some(existing) = history
            .iter_mut()
            .find(|x| canonical_nav_key(&x.url) == canonical_nav_key(&e.url))
        {
            existing.visits = existing.visits.saturating_add(e.visits);
            if e.last_visited >= existing.last_visited {
                existing.last_visited = e.last_visited;
                existing.title = e.title;
            }
            continue;
        }
        history.push(e);
    }
    history.sort_by(|a, b| {
        b.last_visited
            .cmp(&a.last_visited)
            .then_with(|| a.url.cmp(&b.url))
    });
    history.truncate(MAX_HISTORY);
    store.history = history;
    store
}

fn merge_ref(ours: &NavStore, disk: &NavStore) -> NavStore {
    let mut out = ours.clone();
    for b in &disk.bookmarks {
        let key = canonical_nav_key(&b.url);
        if !out
            .bookmarks
            .iter()
            .any(|x| canonical_nav_key(&x.url) == key)
        {
            out.bookmarks.push(b.clone());
        }
    }
    for e in &disk.history {
        let key = canonical_nav_key(&e.url);
        match out
            .history
            .iter_mut()
            .find(|x| canonical_nav_key(&x.url) == key)
        {
            Some(existing) => {
                if e.last_visited > existing.last_visited {
                    existing.last_visited = e.last_visited;
                    existing.title = e.title.clone();
                }
                existing.visits = existing.visits.max(e.visits);
            }
            None => out.history.push(e.clone()),
        }
    }
    out
}

fn matches_ref(query: &str, title: &str, url: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    title.to_lowercase().contains(&q)
        || url.to_lowercase().contains(&q)
        || host_of(url).contains(&q)
}

fn suggestions_ref(store: &NavStore, query: &str, now_ms: u64, limit: usize) -> Vec<Suggestion> {
    let q = query.trim();
    let mut out: Vec<Suggestion> = Vec::new();
    for b in &store.bookmarks {
        if out.len() >= limit {
            return out;
        }
        if matches_ref(q, &b.title, &b.url) {
            out.push(Suggestion {
                url: b.url.clone(),
                title: b.title.clone(),
                kind: SuggestionKind::Bookmark,
            });
        }
    }
    let mut hist: Vec<&HistoryEntry> = store
        .history
        .iter()
        .filter(|e| matches_ref(q, &e.title, &e.url))
        .filter(|e| {
            !out.iter()
                .any(|s| canonical_nav_key(&s.url) == canonical_nav_key(&e.url))
        })
        .collect();
    hist.sort_by(|a, b| {
        frecency_score(b, now_ms)
            .cmp(&frecency_score(a, now_ms))
            .then_with(|| b.last_visited.cmp(&a.last_visited))
            .then_with(|| a.url.cmp(&b.url))
    });
    for e in hist {
        if out.len() >= limit {
            break;
        }
        out.push(Suggestion {
            url: e.url.clone(),
            title: e.title.clone(),
            kind: SuggestionKind::History,
        });
    }
    out
}

fn search_ref(store: &NavStore, query: &str) -> Vec<HistoryEntry> {
    let q = query.trim();
    store
        .history
        .iter()
        .filter(|e| matches_ref(q, &e.title, &e.url))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Generated stores
// ---------------------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

const NOW: u64 = 1_760_000_000_000;

/// URL pool heavy on canonical-key collisions: the same page
/// under fragment / case / default-port / trailing-dot
/// spellings, plus unrecordable shapes.
const URL_POOL: &[&str] = &[
    "https://example.com/articles/1",
    "https://example.com/articles/1#comments",
    "https://EXAMPLE.com/articles/1",
    "https://example.com:443/articles/1",
    "https://example.com./articles/1",
    "https://example.com/articles/2",
    "http://example.com/articles/1",
    "http://example.com:80/articles/1",
    "https://site-a.example.org/",
    "https://site-a.example.org/deep/page?x=1",
    "https://site-a.example.org/deep/page?x=1#frag",
    "https://sub.site-b.example.net/a/b/c",
    "https://unicode.example/日本語",
    "https://k.example/Kelvin",
    "file:///C:/dev/page.html",
    "file:///C:/dev/other.html",
    "pico://start",
    "https://pico.start/",
    "https://user:pass@example.com/secret",
    "about:blank",
    "https://example.com/a b",
    "https://[2001:db8::1]/v6/page",
    "https://[2001:DB8::1]/v6/page",
    "https://trailing.example./",
    "https://trailing.example/",
];

const TITLE_POOL: &[&str] = &[
    "Réunion Ünïcodé — İstanbul",
    "日本語タイトル",
    "Kelvin \u{212A} sign and SIGMA Σ",
    "Plain ASCII Title",
    "MiXeD CaSe ToKeNs",
    "",
    "   ",
    "Control\u{7}Chars\u{1b}Here",
    "emoji 🎉 page",
    "Σίσυφος",
    // Over the 512-char title cap (the sanitizer truncates).
    concat!(
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
        "long long long long long long long long long long ",
    ),
];

const QUERY_POOL: &[&str] = &[
    "",
    " ",
    "a",
    "EXAMPLE",
    "example.com",
    "ünïcodé",
    "İ",
    "\u{212A}",
    "日本",
    "σ",
    "Σ",
    "kelvin",
    "articles/1",
    ".example.org/",
    "zzz-no-match",
    "🎉",
    "c:/dev",
    "page.html",
    "V6",
];

fn gen_store(rng: &mut Lcg, hist_n: usize, mark_n: usize) -> NavStore {
    let mut store = NavStore::default();
    for _ in 0..hist_n {
        store.history.push(HistoryEntry {
            url: rng.pick(URL_POOL).to_string(),
            title: rng.pick(TITLE_POOL).to_string(),
            // Mix of past, present and future stamps.
            last_visited: NOW - rng.below(40 * 86_400_000) + rng.below(2_000_000),
            visits: rng.below(6) as u32,
        });
    }
    for _ in 0..mark_n {
        store.bookmarks.push(Bookmark {
            url: rng.pick(URL_POOL).to_string(),
            title: rng.pick(TITLE_POOL).to_string(),
            added_at: NOW - rng.below(40 * 86_400_000) + rng.below(2_000_000),
        });
    }
    store
}

#[test]
fn navstore_sanitized_matches_reference_over_generated_stores() {
    let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
    for case in 0..40 {
        let (h, m) = ((rng.below(50)) as usize, (rng.below(20)) as usize);
        let store = gen_store(&mut rng, h, m);
        assert_eq!(
            store.clone().sanitized(NOW),
            sanitized_ref(store, NOW),
            "case {case}"
        );
    }
    // One wide case: hundreds of entries, pool-sized key
    // collisions everywhere.
    let store = gen_store(&mut rng, 600, 150);
    assert_eq!(store.clone().sanitized(NOW), sanitized_ref(store, NOW));
}

#[test]
fn navstore_merge_matches_reference_over_generated_stores() {
    let mut rng = Lcg(0xD1B5_4A32_D192_ED03);
    for case in 0..40 {
        let (h1, m1) = ((rng.below(40)) as usize, (rng.below(15)) as usize);
        let (h2, m2) = ((rng.below(40)) as usize, (rng.below(15)) as usize);
        let ours = gen_store(&mut rng, h1, m1);
        let disk = gen_store(&mut rng, h2, m2);
        assert_eq!(
            merge_nav_stores(&ours, &disk),
            merge_ref(&ours, &disk),
            "case {case}"
        );
    }
    let ours = gen_store(&mut rng, 400, 100);
    let disk = gen_store(&mut rng, 400, 100);
    assert_eq!(merge_nav_stores(&ours, &disk), merge_ref(&ours, &disk));
}

#[test]
fn navstore_suggestions_and_search_match_reference_over_generated_stores() {
    let mut rng = Lcg(0x2545_F491_4F6C_DD1D);
    for case in 0..25 {
        let (h, m) = ((rng.below(60)) as usize, (rng.below(20)) as usize);
        let store = gen_store(&mut rng, h, m);
        for q in QUERY_POOL {
            assert_eq!(
                store.suggestions(q, NOW, 8),
                suggestions_ref(&store, q, NOW, 8),
                "case {case} query {q:?} limit 8"
            );
            assert_eq!(
                store.suggestions(q, NOW, 3),
                suggestions_ref(&store, q, NOW, 3),
                "case {case} query {q:?} limit 3"
            );
            assert_eq!(
                store.suggestions(q, NOW, 0),
                suggestions_ref(&store, q, NOW, 0),
                "case {case} query {q:?} limit 0"
            );
            assert_eq!(
                store.search_history(q),
                search_ref(&store, q),
                "case {case} query {q:?}"
            );
        }
    }
}
