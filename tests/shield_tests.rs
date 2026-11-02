//! Shield engine tests (AUDIT V21): ABP decision semantics on
//! synthetic rules, the hand-written synthetic corpus in
//! tests/fixtures/ (public build: the suites never touch the
//! real filter lists — the corpus mirrors their semantics on
//! reserved example/invalid hosts), the three safety laws as
//! named regressions, the compile-report accounting
//! thresholds, and the runtime surface the app layer drives.

use pico_core::core::shield::{
    neutered_body, registrable_domain, Decision, ResourceType, ShieldEngine, ShieldRuntime,
    TRANSPARENT_GIF,
};
use std::sync::Arc;

fn engine(lines: &str) -> ShieldEngine {
    ShieldEngine::compile(&[lines])
}

fn blocks(e: &ShieldEngine, url: &str, page: &str, rt: ResourceType) -> bool {
    e.decide(url, page, rt).is_block()
}

// ---------------------------------------------------------------------
// ABP decision semantics (canonical synthetic rules)
// ---------------------------------------------------------------------

#[test]
fn domain_anchor_third_party_blocks_only_third_party() {
    // The canonical third-party rule shape, on a synthetic
    // host: ||ads.example.net^$third-party.
    let e = engine("||ads.example.net^$third-party");
    assert!(blocks(
        &e,
        "https://ads.example.net/banner.js",
        "https://www.example.com/",
        ResourceType::Script
    ));
    // First-party: the page itself is example.net → allowed.
    assert!(!blocks(
        &e,
        "https://ads.example.net/banner.js",
        "https://example.net/home",
        ResourceType::Script
    ));
    // A different path on the same registrable domain is still
    // first-party (subdomain of the page's site).
    assert!(!blocks(
        &e,
        "https://example.net/x.png",
        "https://www.example.net/",
        ResourceType::Image
    ));
}

#[test]
fn exception_rule_allows_what_a_block_rule_catches() {
    let e = engine("||tracker.example^\n@@||tracker.example/allowed.js$script");
    assert!(blocks(
        &e,
        "https://tracker.example/pixel.js",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://tracker.example/allowed.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn important_block_beats_exception() {
    let e = engine("||ads.example^$important\n@@||ads.example^");
    assert!(blocks(
        &e,
        "https://ads.example/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn important_exception_beats_plain_block() {
    let e = engine("||ads.example^\n@@||ads.example^$important");
    assert!(!blocks(
        &e,
        "https://ads.example/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn domain_option_include_and_exclude() {
    let e = engine("||ads.example^$domain=good.example|~bad.good.example");
    assert!(blocks(
        &e,
        "https://ads.example/x.js",
        "https://good.example/",
        ResourceType::Script
    ));
    assert!(blocks(
        &e,
        "https://ads.example/x.js",
        "https://sub.good.example/",
        ResourceType::Script
    ));
    // Excluded sub-host of an included domain: not blocked.
    assert!(!blocks(
        &e,
        "https://ads.example/x.js",
        "https://bad.good.example/",
        ResourceType::Script
    ));
    // Unrelated page: the rule does not apply at all.
    assert!(!blocks(
        &e,
        "https://ads.example/x.js",
        "https://other.example/",
        ResourceType::Script
    ));
}

#[test]
fn domain_option_wildcard_tld() {
    // The lists' TLD-wildcard shape, `partner.*`.
    let e = engine("@@||cdn.example/lib.js$script,domain=partner.*");
    assert!(!blocks(
        &e,
        "https://cdn.example/lib.js",
        "https://www.partner.fr/article",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://cdn.example/lib.js",
        "https://partner.com/",
        ResourceType::Script
    ));
    // Not a partner TLD variant → the exception does not apply,
    // and with no block rule in this engine the request is allowed
    // either way; assert via the exception NOT shadowing a
    // co-compiled block on an unrelated page:
    let e2 = engine("||cdn.example^\n@@||cdn.example/lib.js$script,domain=partner.*");
    assert!(blocks(
        &e2,
        "https://cdn.example/lib.js",
        "https://unrelated.example/",
        ResourceType::Script
    ));
}

#[test]
fn type_restricted_rule_does_not_block_other_types() {
    let e = engine("||cdn.example/lib$script");
    assert!(blocks(
        &e,
        "https://cdn.example/lib",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://cdn.example/lib",
        "https://site.example/",
        ResourceType::Image
    ));
    assert!(!blocks(
        &e,
        "https://cdn.example/lib",
        "https://site.example/",
        ResourceType::Stylesheet
    ));
}

#[test]
fn negated_type_option() {
    let e = engine("||cdn.example/pixel$~image");
    assert!(blocks(
        &e,
        "https://cdn.example/pixel",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://cdn.example/pixel",
        "https://site.example/",
        ResourceType::Image
    ));
}

#[test]
fn badfilter_neutralizes_its_twin() {
    let e = engine("||ads.example/banner^\n||ads.example/banner^$badfilter");
    assert!(!blocks(
        &e,
        "https://ads.example/banner",
        "https://site.example/",
        ResourceType::Image
    ));
    assert_eq!(e.report().badfilter_neutralized, 1);
    // A badfilter whose options differ from any twin neutralizes
    // nothing (the real lists carry exactly such cases).
    let e2 = engine("||ads.example/banner^$image\n||ads.example/banner^$badfilter");
    assert!(blocks(
        &e2,
        "https://ads.example/banner",
        "https://site.example/",
        ResourceType::Image
    ));
}

#[test]
fn unsupported_options_skip_the_rule_never_misapply() {
    for opt in [
        "csp=script-src 'self'",
        "redirect=noopjs",
        "removeparam=utm_source",
        "rewrite=abp-resource:blank-css",
        "popup",
        "webrtc",
        "match-case",
        "unknownoption=1",
    ] {
        let e = engine(&format!("||ads.example^${opt}"));
        assert!(
            !blocks(
                &e,
                "https://ads.example/x.js",
                "https://site.example/",
                ResourceType::Script
            ),
            "option {opt} must skip, not apply"
        );
        assert_eq!(e.report().skipped_unsupported_option, 1, "option {opt}");
    }
}

#[test]
fn regex_rules_are_skipped_not_misapplied() {
    let e = engine(r"/ads?\d+\.example/");
    assert!(!blocks(
        &e,
        "https://ads9.example/x",
        "https://site.example/",
        ResourceType::Script
    ));
    assert_eq!(e.report().skipped_regex, 1);
}

#[test]
fn separators_and_wildcards() {
    let e = engine("||cdn.example/ads/*/banner^$image");
    // ^ satisfied by end-of-URL…
    assert!(blocks(
        &e,
        "https://cdn.example/ads/v2/banner",
        "https://site.example/",
        ResourceType::Image
    ));
    // …and by a true separator character.
    assert!(blocks(
        &e,
        "https://cdn.example/ads/v2/banner?size=2",
        "https://site.example/",
        ResourceType::Image
    ));
    // ABP semantics: '.' is NOT a separator (it is in the token
    // character set), so "banner.png" does not satisfy `banner^` —
    // real uBO agrees; this pins the definition, not a bug.
    assert!(!blocks(
        &e,
        "https://cdn.example/ads/v2/banner.png",
        "https://site.example/",
        ResourceType::Image
    ));
    // Missing path segment → no match.
    assert!(!blocks(
        &e,
        "https://cdn.example/ads/banner",
        "https://site.example/",
        ResourceType::Image
    ));
}

#[test]
fn edge_anchors() {
    let e = engine("|https://ads.example/start");
    assert!(blocks(
        &e,
        "https://ads.example/start-here",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://site.example/?u=https://ads.example/start",
        "https://site.example/",
        ResourceType::Script
    ));
    let e2 = engine("track.js|");
    assert!(blocks(
        &e2,
        "https://cdn.example/track.js",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e2,
        "https://cdn.example/track.js?v=2",
        "https://site.example/",
        ResourceType::Script
    ));
}

// ---------------------------------------------------------------------
// The safety laws (named for the v7 catastrophe)
// ---------------------------------------------------------------------

#[test]
fn law1_main_frame_document_never_blocked_without_document_rule() {
    // Even a rule that names the exact host may not block the
    // main document unless it explicitly carries $document.
    let e = engine(
        "||search.example^\n||search.example^$third-party\n||search.example^$script,image,subdocument",
    );
    assert_eq!(
        e.decide(
            "https://www.search.example/",
            "https://example.com/",
            ResourceType::Document
        ),
        Decision::Allow
    );
    // A rule that DOES carry document + a domain anchor may
    // block a document (narrow, explicit — the uBO posture).
    let e2 = engine("||evil.example^$document");
    assert!(blocks(
        &e2,
        "https://evil.example/",
        "https://example.com/",
        ResourceType::Document
    ));
    // But a document rule without a domain anchor still may not.
    let e3 = engine("/landing$page$document");
    assert!(!blocks(
        &e3,
        "https://evil.example/landing",
        "https://example.com/",
        ResourceType::Document
    ));
}

#[test]
fn law2_local_addresses_are_never_blocked() {
    // Synthetic worst case: rules that name the loopback hosts
    // directly and generically.
    let e = engine("||127.0.0.1^\n||localhost^\n/127.0.0.1/\n||0.0.0.0^");
    for url in [
        "http://127.0.0.1:5173/ads.js",
        "http://localhost:3000/track",
        "http://[::1]:8080/x.png",
        "http://dev.localhost/ads",
        // AUDIT V28 (A2-L1): 0.0.0.0 is this machine too — as a
        // request AND as a page it is never filtered.
        "http://0.0.0.0:4200/ads.js",
    ] {
        for page in ["https://example.com/", "http://127.0.0.1:5173/"] {
            assert!(
                !blocks(&e, url, page, ResourceType::Script),
                "{url} from {page} must never be blocked"
            );
        }
    }
}

#[test]
fn law4_engine_off_allows_everything() {
    // AUDIT V28 (A2-M2): this law used to be asserted through
    // the runtime's should_block seam — production-dead since
    // v26, deleted now. The law lives in the pipeline: with
    // shield_on=false the verdict is Allow even for a rule hit,
    // and a missing engine (lists not yet compiled) fails open
    // the same way. Both are pinned here, on the production
    // decision function.
    use pico_core::core::filterpipe::{decide, FilterState, RequestFacts, Verdict};
    let e = engine("||ads.example^");
    let facts = RequestFacts {
        url: "https://ads.example/x.js".to_string(),
        page_url: "https://site.example/".to_string(),
        rtype: ResourceType::Script,
        method: Some("GET".to_string()),
        page_watch_shaped: false,
        tab_chosen: false,
    };
    assert_eq!(
        decide(
            &facts,
            FilterState {
                shield_on: false,
                lite_on: false
            },
            Some(&e)
        ),
        Verdict::Allow
    );
    assert_eq!(
        decide(
            &facts,
            FilterState {
                shield_on: true,
                lite_on: false
            },
            None
        ),
        Verdict::Allow
    );
    // And the runtime's own on-flag still gates its counters'
    // meaning: a fresh runtime has compiled nothing.
    let rt2 = ShieldRuntime::new(true);
    assert!(rt2.engine_snapshot().is_none());
}

#[test]
fn v29_corpus_does_not_shield_block_the_chosen_videos_playback() {
    // A2-H2, settled empirically: the pipeline consults the
    // Shield BEFORE Lite's chosen exemption, so the exemption
    // would be worthless on YouTube if the lists themselves
    // blocked playback traffic. They do not: under the full
    // compiled corpus (the public-build stand-in for the
    // shipped lists), with Shield AND Lite on, the watch
    // page's segment fetches and stream manifests come out
    // Allow through the real pipeline — while the corpus's
    // own ad-host rule still blocks, proving the engine in
    // this test is armed, not empty.
    use pico_core::core::filterpipe::{decide, FilterState, RequestFacts, Verdict};
    let e = full_engine();
    let page = "https://www.youtube.com/watch?v=fixture1";
    let both_on = FilterState {
        shield_on: true,
        lite_on: true,
    };
    for (url, rt) in [
        (
            "https://rr2---sn-a5mekn6r.googlevideo.com/videoplayback?expire=1&itag=137",
            ResourceType::Xhr,
        ),
        (
            "https://rr2---sn-a5mekn6r.googlevideo.com/videoplayback?expire=1&itag=140",
            ResourceType::Other,
        ),
        (
            "https://www.youtube.com/youtubei/v1/player?key=fixture",
            ResourceType::Xhr,
        ),
    ] {
        let facts = RequestFacts {
            url: url.to_string(),
            page_url: page.to_string(),
            rtype: rt,
            method: Some("GET".to_string()),
            page_watch_shaped: true,
            tab_chosen: true,
        };
        assert_eq!(
            decide(&facts, both_on, Some(&e)),
            Verdict::Allow,
            "{url} must reach the player under the corpus"
        );
    }
    // The engine is armed: the corpus's dedicated ad host on
    // the same CDN family IS shield-blocked.
    assert!(blocks(
        &e,
        "https://ads.googlevideo.com/vast.xml",
        page,
        ResourceType::Xhr
    ));
}

#[test]
fn runtime_counts_blocks_per_tab_and_in_total() {
    // Counting is the runtime's production job (record_block is
    // called by the handler after a delivered neuter); the
    // decision that precedes it is the pipeline's — pinned in
    // filterpipe's matrix. (Through v27 this test drove the
    // deleted should_block seam to reach the counters.)
    let rt = ShieldRuntime::new(true);
    rt.install(engine("||ads.example^"), "x".to_string());
    let (total, tab, _) = rt.record_block(7);
    assert_eq!((total, tab), (1, 1));
    rt.record_block(7);
    rt.record_block(9);
    assert_eq!(rt.tab_blocked(7), 2);
    assert_eq!(rt.tab_blocked(9), 1);
    assert_eq!(
        rt.session_blocked
            .load(std::sync::atomic::Ordering::Relaxed),
        3
    );
}

#[test]
fn runtime_downgrades_iframe_documents() {
    // The reclassification is the pure effective_rtype the
    // handler itself calls (AUDIT V28, A2-M2).
    use pico_core::core::shield::effective_rtype;
    assert_eq!(effective_rtype(1, false), ResourceType::Subdocument);
    assert_eq!(effective_rtype(1, true), ResourceType::Document);
    assert_eq!(effective_rtype(6, false), ResourceType::Script);
    // And the engine treats the reclassified frame as a
    // subdocument (blockable), while law 1 protects the real
    // main document.
    let e = engine("||ads.example^$subdocument");
    assert!(blocks(
        &e,
        "https://ads.example/frame",
        "https://site.example/",
        effective_rtype(1, false)
    ));
    assert!(!blocks(
        &e,
        "https://site.example/",
        "https://site.example/",
        effective_rtype(1, true)
    ));
}

// ---------------------------------------------------------------------
// The synthetic corpus (tests/fixtures/): hand-written lists that
// mirror the real lists' semantics on reserved hosts. The full
// corpus compiles through `full_engine()` below; the three tests
// here pin individual corpus rules the way the pre-GitHub suites
// pinned lifted real rules.
// ---------------------------------------------------------------------

/// The four synthetic fixture lists, in canonical-source order.
fn synthetic_lists() -> Vec<String> {
    vec![
        include_str!("fixtures/easylist-synthetic.txt").to_string(),
        include_str!("fixtures/easyprivacy-synthetic.txt").to_string(),
        include_str!("fixtures/ublock-filters-synthetic.txt").to_string(),
        include_str!("fixtures/ublock-privacy-synthetic.txt").to_string(),
    ]
}

#[test]
fn corpus_rule_analytics_third_party_only() {
    // Corpus: ||analytics.example-corp.invalid^$script,third-party,xmlhttprequest
    let e = engine("||analytics.example-corp.invalid^$script,third-party,xmlhttprequest");
    assert!(blocks(
        &e,
        "https://analytics.example-corp.invalid/analytics.js",
        "https://www.example.com/",
        ResourceType::Script
    ));
    // First-party: the rule must not fire.
    assert!(!blocks(
        &e,
        "https://analytics.example-corp.invalid/analytics.js",
        "https://www.example-corp.invalid/",
        ResourceType::Script
    ));
    // Type-restricted: an image request is not covered.
    assert!(!blocks(
        &e,
        "https://analytics.example-corp.invalid/collect.gif",
        "https://www.example.com/",
        ResourceType::Image
    ));
}

#[test]
fn corpus_rule_block_and_its_domain_exception() {
    // Corpus: ||ads.example-corp.invalid^  and
    // @@||ads.example-corp.invalid/lib.js$script,domain=partner.*|shop.example
    let e = full_engine();
    assert!(blocks(
        &e,
        "https://ads.example-corp.invalid/lib.js",
        "https://news.example/",
        ResourceType::Script
    ));
    // The exception's domain= uses the `partner.*` TLD wildcard.
    assert!(!blocks(
        &e,
        "https://ads.example-corp.invalid/lib.js",
        "https://www.partner.fr/article",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://ads.example-corp.invalid/lib.js",
        "https://shop.example/article",
        ResourceType::Script
    ));
}

#[test]
fn corpus_rule_important_domain_scoped() {
    // Corpus: ||trackers.example.net^$script,important,domain=news.example
    // and     @@||trackers.example.net^$script,xhr,domain=docs.example
    let e = full_engine();
    assert!(blocks(
        &e,
        "https://trackers.example.net/ad.js",
        "https://www.news.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://trackers.example.net/ad.js",
        "https://docs.example/",
        ResourceType::Script
    ));
    // Neither applies on an unrelated page.
    assert!(!blocks(
        &e,
        "https://trackers.example.net/ad.js",
        "https://example.com/",
        ResourceType::Script
    ));
}

// ---------------------------------------------------------------------
// Full synthetic corpus: regression + accounting thresholds
// ---------------------------------------------------------------------

fn full_engine() -> Arc<ShieldEngine> {
    let lists = synthetic_lists();
    let refs: Vec<&str> = lists.iter().map(|s| s.as_str()).collect();
    Arc::new(ShieldEngine::compile(&refs))
}

#[test]
fn regression_google_com_document_never_blocked_corpus() {
    // The v7 catastrophe, pinned: under the FULL compiled
    // corpus, google.com as a main document is allowed — always.
    let e = full_engine();
    for page in ["https://example.com/", "https://news.site/"] {
        assert_eq!(
            e.decide("https://www.google.com/", page, ResourceType::Document),
            Decision::Allow
        );
        assert_eq!(
            e.decide("https://google.com/", page, ResourceType::Document),
            Decision::Allow
        );
    }
}

#[test]
fn regression_youtube_document_never_blocked_corpus() {
    let e = full_engine();
    assert_eq!(
        e.decide(
            "https://www.youtube.com/",
            "https://example.com/",
            ResourceType::Document
        ),
        Decision::Allow
    );
    assert_eq!(
        e.decide(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://example.com/",
            ResourceType::Document
        ),
        Decision::Allow
    );
}

#[test]
fn regression_loopback_never_blocked_corpus() {
    let e = full_engine();
    for url in [
        "http://127.0.0.1:5173/node_modules/ads.js",
        "http://localhost:3000/analytics.js",
        "http://[::1]:9000/doubleclick.net/x",
    ] {
        assert_eq!(
            e.decide(url, "https://example.com/", ResourceType::Script),
            Decision::Allow,
            "{url}"
        );
    }
}

#[test]
fn corpus_blocks_its_trackers() {
    let e = full_engine();
    assert!(blocks(
        &e,
        "https://ads.example-corp.invalid/banner.js",
        "https://news.example/",
        ResourceType::Script
    ));
    assert!(blocks(
        &e,
        "https://analytics.example-corp.invalid/analytics.js",
        "https://www.example.com/",
        ResourceType::Script
    ));
    // Benign same-site content is untouched.
    assert!(!blocks(
        &e,
        "https://www.example.com/assets/site.css",
        "https://www.example.com/",
        ResourceType::Stylesheet
    ));
}

#[test]
fn compile_report_accounts_for_every_line() {
    let e = full_engine();
    let r = e.report();
    let accounted = r.comments
        + r.network_rules
        + r.network_exceptions
        + r.duplicates_removed
        + r.badfilter_neutralized
        + r.badfilter_rules
        + r.cosmetic_lines
        + r.cosmetic_exceptions
        + r.page_flag_rules
        + r.scriptlet_rules
        + r.scriptlet_exceptions
        + r.skipped_total();
    assert_eq!(
        accounted, r.total_lines,
        "every list line must be accounted for: {r:?}"
    );
    // Network compilation yield: of the lines that present as
    // network rules (compiled + network-relevant skips), at
    // least 90% compile — the spec's threshold.
    let net_skips = r.skipped_regex + r.skipped_unsupported_option + r.skipped_invalid;
    let net_presented = r.network_accounted() + net_skips;
    assert!(
        r.network_accounted() * 100 >= net_presented * 90,
        "network yield below 90%: {r:?}"
    );
    // Corpus-relative floors (the corpus is ~170 lines by
    // design): the compiled sets must be substantively present.
    assert!(r.network_rules > 50, "sanity: {r:?}");
    assert!(r.cosmetic_generic >= 4, "sanity: {r:?}");
    // Duplicates and badfilter accounting happened (or not) —
    // both fields exist and are consistent by construction; the
    // exact counts are printed in BUILD_NOTES.
}

// ---------------------------------------------------------------------
// Cosmetic compiler
// ---------------------------------------------------------------------

#[test]
fn cosmetic_compiler_generic_set_and_exceptions() {
    let e = engine("##.ad-banner\n##.ad-banner\n###sponsored\nfoo.example##.local-ad\nfoo.example#@#.local-ad\nbar.example##.only-bar");
    let (script, hosts, generic) = e.build_cosmetic_script(150_000);
    assert_eq!(generic, 2, "two distinct generic selectors");
    assert!(script.contains(".ad-banner"));
    assert!(script.contains("#sponsored"));
    // Exact count, not a floor (AUDIT V28 vacuous sweep):
    // foo.example's only selector was excepted away, leaving
    // exactly bar.example in the payload.
    assert_eq!(hosts, 1);
    // The script is the runtime + payload, self-guarding.
    assert!(script.contains("__picoShieldCosmetic"));
    // Specific exception subtracts on its host in the payload:
    // foo.example's set must not include .local-ad (exception),
    // bar.example's must include .only-bar.
    assert!(script.contains(".only-bar"));
    assert!(!script.contains(".local-ad"));
}

#[test]
fn cosmetic_script_respects_budget() {
    let mut lines = String::new();
    for i in 0..2000 {
        lines.push_str(&format!("host{i}.example##.ad-{i}\n"));
    }
    lines.push_str("##.generic-one\n");
    let e = engine(&lines);
    let (script, hosts, _) = e.build_cosmetic_script(20_000);
    assert!(script.len() <= 20_000, "script is {} bytes", script.len());
    assert!(hosts < 2000, "budget must cut hosts, got {hosts}");
    assert!(script.contains(".generic-one"), "generic always ships");
}

#[test]
fn generichide_hosts_are_carried() {
    // Both canonical declaration forms: the $generichide option
    // (scoped by domain=) and the host#$#generichide line.
    let e = engine("*$generichide,domain=foo.example\nbar.example#$#generichide");
    assert_eq!(e.report().page_flag_rules, 2);
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(script.contains("foo.example"), "script: {script}");
    assert!(script.contains("bar.example"), "script: {script}");
    assert!(script.contains("\"gh\""));
}

// ---------------------------------------------------------------------
// Helpers the app layer relies on
// ---------------------------------------------------------------------

#[test]
fn registrable_domain_approximation() {
    assert_eq!(registrable_domain("www.example.com"), "example.com");
    assert_eq!(registrable_domain("a.b.example.co.uk"), "example.co.uk");
    assert_eq!(registrable_domain("example.com"), "example.com");
    assert_eq!(registrable_domain("localhost"), "localhost");
    assert_eq!(registrable_domain("127.0.0.1"), "127.0.0.1");
}

#[test]
fn neutered_bodies_are_type_appropriate() {
    let (body, ctype) = neutered_body(ResourceType::Image);
    assert_eq!(body, Some(TRANSPARENT_GIF));
    assert_eq!(ctype, "image/gif");
    assert!(TRANSPARENT_GIF.starts_with(b"GIF89a"));
    let (body, ctype) = neutered_body(ResourceType::Script);
    assert_eq!(body, None);
    assert_eq!(ctype, "text/javascript");
    let (_, ctype) = neutered_body(ResourceType::Stylesheet);
    assert_eq!(ctype, "text/css");
}

// ---------------------------------------------------------------------
// v22 audit fixes (AUDIT V22-4..V22-7)
// ---------------------------------------------------------------------

#[test]
fn host_span_skips_userinfo_and_handles_ipv6() {
    use pico_core::core::shield::host_span;
    // Plain authority.
    let (start, after) =
        host_span("https://ads.example.com:8443/x", "ads.example.com").expect("plain host span");
    assert_eq!(
        &"https://ads.example.com:8443/x"[start..start + 15],
        "ads.example.com"
    );
    assert_eq!(after, "https://".len() + "ads.example.com".len());
    // The host string appearing in userinfo must NOT be taken
    // as the authority host (AUDIT V22-4).
    let url = "https://ads.example.com@evil.example/x";
    let (start, _) = host_span(url, "evil.example").expect("real host span");
    assert_eq!(&url[start..], "evil.example/x");
    // host_span is positional (the caller passes host_of's
    // answer). For a host string that is NOT the authority's
    // host it returns None — the span would run past the URL —
    // and the positive userinfo property is pinned by the
    // evil.example span above: it starts at the real host,
    // after the '@'. (Through v27 this spot accepted None OR a
    // post-'@' span, i.e. either behavior passed — AUDIT V28,
    // vacuous-test sweep.)
    assert!(host_span(url, "ads.example.com").is_none());
    // IPv6 literal: the span must end after the closing bracket,
    // not at the first colon inside it.
    let url6 = "http://[::1]:8080/x";
    let (start, after) = host_span(url6, "[::1]").expect("ipv6 span");
    assert_eq!(&url6[start..after], "[::1]");
    assert_eq!(&url6[after..], ":8080/x");
}

#[test]
fn anchored_rules_judge_the_real_host_not_userinfo() {
    // ||evil.example^ must block when evil.example is the real
    // host even behind a lookalike userinfo, and must NOT block
    // when it appears only as userinfo (AUDIT V22-4).
    let e = engine("||evil.example^");
    assert!(blocks(
        &e,
        "https://tracker.example@evil.example/x.js",
        "https://news.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://evil.example@tracker.example/x.js",
        "https://news.example/",
        ResourceType::Script
    ));
}

#[test]
fn anchored_slash_rule_matches_explicit_port() {
    // AUDIT 0.31 (A2-F10): a slash-form anchored rule describes
    // the PATH, but with an explicit port the URL remainder
    // after the host begins with ':' — through 0.30.1 the rule
    // silently never matched a ported URL.
    let e = engine("||ads.example.com/banner.js");
    assert!(blocks(
        &e,
        "https://ads.example.com:8443/banner.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // The portless form still blocks exactly as before.
    assert!(blocks(
        &e,
        "https://ads.example.com/banner.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // A different path behind the same port does not match.
    assert!(!blocks(
        &e,
        "https://ads.example.com:8443/other.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // `||host^` behavior is unchanged: the ':' of the port is
    // the separator the caret expects, ported or not.
    let e2 = engine("||ads.example.com^");
    assert!(blocks(
        &e2,
        "https://ads.example.com:8443/anything.js",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(blocks(
        &e2,
        "https://ads.example.com/anything.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn whatwg_ipv4_number_forms_classify_loopback() {
    // AUDIT 0.31 (A2-F15): a browser resolves every one of
    // these to 127.0.0.1, so the local-host test must too.
    use pico_core::core::shield::is_local_host;
    for host in [
        "2130706433",   // single-integer decimal
        "0x7f000001",   // hex
        "0X7F000001",   // hex, upper-case spelling
        "017700000001", // octal
        "127.1",        // short form: last part spans 3 bytes
        "127.0.1",      // short form: last part spans 2 bytes
        "0x7f.1",       // mixed radix parts
        "127.0.0.1",    // the canonical form, for reference
    ] {
        assert!(is_local_host(host), "{host} must be local");
    }
    // The same spellings outside 127/8 are NOT local:
    // 0x08080808 is 8.8.8.8, and 134744072 is its decimal twin.
    for host in [
        "0x08080808",
        "134744072",
        "8.1",
        "256.0.0.1",
        "127.0.0.256",
        "08.0.0.1",
    ] {
        assert!(!is_local_host(host), "{host} must NOT be local");
    }
    // Ordinary names are untouched by the number parser.
    assert!(!is_local_host("example.com"));
    assert!(is_local_host("localhost"));
    assert!(is_local_host("dev.localhost"));
}

#[test]
fn genericblock_spares_generic_exceptions() {
    // AUDIT V22-5: on a $genericblock page, generic BLOCK rules
    // are suppressed — but a generic EXCEPTION stays in force
    // and still overrides a host-anchored block rule.
    let e = engine(
        "||ads.example^\n@@ads.example/allowed\n/banner.js\n*$genericblock,domain=news.example",
    );
    // Generic block suppressed on the flagged page.
    assert!(!blocks(
        &e,
        "https://cdn.example/banner.js",
        "https://news.example/story",
        ResourceType::Script
    ));
    // …while the same generic block still fires elsewhere.
    assert!(blocks(
        &e,
        "https://cdn.example/banner.js",
        "https://other.example/story",
        ResourceType::Script
    ));
    // The generic exception survives genericblock and beats the
    // anchored block rule.
    assert!(!blocks(
        &e,
        "https://ads.example/allowed/x.js",
        "https://news.example/story",
        ResourceType::Script
    ));
}

#[test]
fn elemhide_and_global_generichide_reach_the_cosmetic_payload() {
    // AUDIT V22-6/V22-7: v21 parsed these flags but dropped
    // them before the injected script was built.
    let e = engine("foo.example#$#elemhide\n*$generichide\n##.ad-banner");
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(script.contains("\"eh\""), "script: {script}");
    assert!(script.contains("foo.example"), "elemhide host carried");
    assert!(
        script.contains("\"ghAll\":true"),
        "global generichide carried"
    );
    assert!(
        script.contains("\"ehAll\":false"),
        "no global elemhide declared"
    );

    let e = engine("*$elemhide\n##.ad-banner");
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(script.contains("\"ehAll\":true"), "global elemhide carried");
}

// ---------------------------------------------------------------------
// v23: scriptlets, option spellings, catalogue regressions
// ---------------------------------------------------------------------

fn entries_for(e: &ShieldEngine, host: &str) -> Vec<(String, Vec<String>)> {
    e.specific_scriptlets()
        .get(host)
        .map(|v| {
            v.iter()
                .map(|en| (en.name.clone(), en.args.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn scriptlet_calls_parse_with_uvo_grammar() {
    // Plain shape: name + bare/quoted args.
    let e = engine("x.example##+js(set, ytInitialPlayerResponse.adPlacements, undefined)");
    assert_eq!(
        entries_for(&e, "x.example"),
        vec![(
            "set".to_string(),
            vec![
                "ytInitialPlayerResponse.adPlacements".to_string(),
                "undefined".to_string()
            ]
        )]
    );
    // A bare JSON-ish argument that merely starts with a quote
    // is ONE token (the rpnt shape the real lists use).
    let e = engine("x.example##+js(trusted-rpfr, \"a\":true, \"a\":false)");
    let got = entries_for(&e, "x.example");
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].1,
        vec!["\"a\":true".to_string(), "\"a\":false".to_string()]
    );
    // Canonicalization: long names fold to the dispatch names.
    let e = engine("x.example##+js(set-constant, a.b, true)\nx.example##+js(trusted-replace-fetch-response, 'x', 'y')");
    let got = entries_for(&e, "x.example");
    let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["set", "trusted-rpfr"]);
}

#[test]
fn scriptlet_tokenizer_survives_regex_args() {
    // The corpus's own replace rule (tests/fixtures, written
    // for this suite): a regex containing an escaped comma
    // `\\,`, an EMPTY second argument, and a regex condition —
    // the tokenizer must yield exactly three args.
    let line = "www.youtube.com##+js(trusted-replace-xhr-response, /\"adSlots\"\\s*:\\s*\\[.{0\\,60}\\]\\s*,?/, , /\\/player/)";
    let e = engine(line);
    let got = entries_for(&e, "www.youtube.com");
    assert_eq!(got.len(), 1, "entries: {got:?}");
    assert_eq!(got[0].0, "trusted-replace-xhr-response");
    assert_eq!(got[0].1.len(), 3, "args: {:?}", got[0].1);
    assert!(
        got[0].1[0].starts_with("/\"adSlots"),
        "arg0: {}",
        got[0].1[0]
    );
    assert_eq!(got[0].1[1], "", "the empty replacement arg survives");
    assert!(
        got[0].1[2].starts_with("/\\/player"),
        "arg2: {}",
        got[0].1[2]
    );
    // Flags after the closing slash stay on the token, and a
    // $1 backreference arg is separate (same hazard family,
    // hand-written rule).
    let line = "www.youtube.com##+js(trusted-replace-xhr-response, /\"adSlots(.*?)\"end\"/gms, $1, player)";
    let e = engine(line);
    let got = entries_for(&e, "www.youtube.com");
    assert_eq!(got.len(), 1);
    assert!(got[0].1[0].ends_with("/gms"), "arg0: {}", got[0].1[0]);
    assert_eq!(got[0].1[1], "$1");
}

#[test]
fn corpus_scriptlet_catalogue_compiles() {
    // The public-build analogue of the v23 YouTube catalogue
    // test: the corpus's video-site section (hand-written in
    // tests/fixtures) must compile into exactly the payload
    // shape the DOM suite drives.
    let e = full_engine();
    let www = entries_for(&e, "www.youtube.com");
    let by_name = |n: &str| www.iter().filter(|(name, _)| name == n).count();
    // The four `set` traps on the initial player data.
    assert_eq!(by_name("set"), 4, "www entries: {www:?}");
    let chains: Vec<&str> = www
        .iter()
        .filter(|(n, _)| n == "set")
        .map(|(_, a)| a[0].as_str())
        .collect();
    for want in [
        "ytInitialPlayerResponse.playerAds",
        "ytInitialPlayerResponse.adPlacements",
        "ytInitialPlayerResponse.adSlots",
        "playerResponse.adPlacements",
    ] {
        assert!(
            chains.contains(&want),
            "missing set chain {want}: {chains:?}"
        );
    }
    // The replace family and the reel prune entries.
    assert_eq!(by_name("trusted-replace-xhr-response"), 2);
    assert_eq!(by_name("trusted-rpfr"), 2);
    assert_eq!(by_name("json-prune-fetch-response"), 1);
    assert_eq!(by_name("json-prune"), 2);
    // The telemetry blocker from the privacy fixture
    // (method:POST arg rides along).
    assert_eq!(by_name("no-xhr-if"), 1);
    // tv.youtube.com carries its own replace rule.
    let tv = entries_for(&e, "tv.youtube.com");
    assert!(tv
        .iter()
        .any(|(n, a)| n == "trusted-replace-xhr-response" && a[2].starts_with("/playlist")));
    // m.youtube.com: the json-prune pair + the shared set traps.
    let m = entries_for(&e, "m.youtube.com");
    assert_eq!(m.iter().filter(|(n, _)| n == "json-prune").count(), 2);
    assert_eq!(m.iter().filter(|(n, _)| n == "set").count(), 3);
    // The json-edit line must NOT appear anywhere (the family
    // is deliberately unimplemented and counted as skipped).
    for (host, list) in e.specific_scriptlets() {
        for en in list {
            assert!(
                !en.name.starts_with("json-edit") && !en.name.starts_with("jsonl-edit"),
                "json-edit family must stay uncompiled ({host}: {})",
                en.name
            );
        }
    }
    // Report-level: the corpus's own scriptlet accounting.
    let r = e.report();
    assert_eq!(r.skipped_replace, 1, "report: {r:?}");
    assert!(r.scriptlet_rules >= 12, "report: {r:?}");
    assert!(r.scriptlet_entries >= 20, "report: {r:?}");
}

#[test]
fn scriptlet_exceptions_subtract_matching_entries() {
    let e = engine(
        "x.example##+js(set, a.b, undefined)\nx.example##+js(set, a.c, undefined)\nx.example#@#+js(set, a.b, undefined)",
    );
    let got = entries_for(&e, "x.example");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1[0], "a.c");
    assert_eq!(e.report().scriptlet_exceptions, 1);
    // An argument-less exception removes every entry of the name.
    let e = engine("x.example##+js(set, a.b, undefined)\nx.example#@#+js(set)");
    assert!(entries_for(&e, "x.example").is_empty());
    // Unsupported names are skipped, never compiled.
    let e = engine("x.example##+js(nostif, 0)");
    assert!(entries_for(&e, "x.example").is_empty());
    assert_eq!(e.report().skipped_scriptlet, 1);
}

#[test]
fn scriptlet_payload_ships_whole() {
    let e = full_engine();
    let (script, hosts, entries) = e.build_scriptlet_script(220_000);
    assert_eq!(entries, e.report().scriptlet_entries);
    assert_eq!(
        hosts,
        e.report().scriptlet_hosts,
        "every compiled host ships"
    );
    assert!(hosts >= 3, "hosts: {hosts}");
    for h in ["www.youtube.com", "m.youtube.com", "tv.youtube.com"] {
        assert!(script.contains(h), "payload missing host {h}");
    }
    assert!(script.contains("ytInitialPlayerResponse.adPlacements"));
    assert!(script.contains("trusted-replace-xhr-response"));
    // The document script is scriptlets first, cosmetics second.
    let (doc, info) = e.build_document_script(280_000, 220_000);
    let sp = doc.find("__picoShieldScriptlets").expect("scriptlet half");
    let cp = doc.find("__picoShieldCosmetic").expect("cosmetic half");
    assert!(sp < cp, "scriptlet runtime must precede cosmetics");
    assert_eq!(info.scriptlet_entries, e.report().scriptlet_entries);
    assert!(info.bytes < 600_000, "document script {} bytes", info.bytes);
}

#[test]
fn video_network_catalogue_decisions() {
    // The synthetic analogue of the v23 YouTube network
    // catalogue: `*_ad_$media,domain=video.example,3p` — a
    // path-pattern in-stream ad rule (the `3p` spelling
    // included) scoped to the corpus's video host.
    let e = full_engine();
    assert!(blocks(
        &e,
        "https://cdn.example.net/clip_ad_pod.mp4",
        "https://video.example/watch?v=x",
        ResourceType::Media
    ));
    // First-party media on the video host itself: 3p fails.
    assert!(!blocks(
        &e,
        "https://video.example/clip_ad_pod.mp4",
        "https://video.example/watch?v=x",
        ResourceType::Media
    ));
    // Off-scope pages: domain= fails.
    assert!(!blocks(
        &e,
        "https://cdn.example.net/clip_ad_pod.mp4",
        "https://vimeo.example/123",
        ResourceType::Media
    ));
    // The playback-init beacon (xhr, domain=video.example).
    assert!(blocks(
        &e,
        "https://stream.example.net/initplayback?source=video&x=1",
        "https://video.example/watch?v=x",
        ResourceType::Xhr
    ));
    assert!(!blocks(
        &e,
        "https://stream.example.net/initplayback?source=video&x=1",
        "https://example.com/",
        ResourceType::Xhr
    ));
    // The $replace player rule is counted in its own bucket
    // and never applied as a network rule — its scriptlet
    // twins do the work (catalogue test above).
    let e = engine("||video.example/player$xmlhttprequest,replace=/a/b/");
    assert_eq!(e.report().skipped_replace, 1);
    assert!(!blocks(
        &e,
        "https://video.example/player?key=x",
        "https://video.example/watch?v=x",
        ResourceType::Xhr
    ));
    // The redirect= rule stays skipped-unsupported as a
    // network rule.
    let e = engine("||video.example/get_video$media,redirect=noop.txt");
    assert_eq!(e.report().skipped_unsupported_option, 1);
}

#[test]
fn option_spellings_the_lists_use() {
    // 1p / 3p party spellings.
    let e = engine("||ads.example^$3p");
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Other
    ));
    assert!(!blocks(
        &e,
        "https://ads.example/x",
        "https://www.ads.example/",
        ResourceType::Other
    ));
    let e = engine("||ads.example^$1p");
    assert!(!blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Other
    ));
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://www.ads.example/",
        ResourceType::Other
    ));
    // Type aliases: xhr, doc, frame, css.
    let e = engine("||ads.example^$xhr");
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Xhr
    ));
    assert!(!blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Script
    ));
    let e = engine("||ads.example^$css");
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Stylesheet
    ));
    let e = engine("||ads.example^$frame");
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Subdocument
    ));
    assert!(!blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Document
    ));
    // doc: domain-anchored + document bit is exactly the shape
    // safety law 1 permits to block a main document.
    let e = engine("||ads.example^$doc");
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Document
    ));
    // from= is domain= under uBO's spelling.
    let e = engine("||ads.example^$from=site.example");
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://ads.example/x",
        "https://other.example/",
        ResourceType::Script
    ));
    // ghide: a page-flag declaration, counted and carried.
    let e = engine("||ads.example^$ghide,domain=foo.example");
    assert_eq!(e.report().page_flag_rules, 1);
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(script.contains("foo.example"));
    // $all: known no-op restriction.
    let e = engine("||ads.example^$all");
    assert!(blocks(
        &e,
        "https://ads.example/x",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn method_and_to_options_scope_decisions() {
    let e = engine("||lemonde.example/*?$xhr,method=post");
    let url = "https://lemonde.example/track?s=1";
    let page = "https://www.lemonde.example/article";
    assert!(e
        .decide_with_method(url, page, ResourceType::Xhr, Some("POST"))
        .is_block());
    assert!(!e
        .decide_with_method(url, page, ResourceType::Xhr, Some("GET"))
        .is_block());
    // Unknown method: fail-open, never blocked by a method rule.
    assert!(!e
        .decide_with_method(url, page, ResourceType::Xhr, None)
        .is_block());
    assert!(!e.decide(url, page, ResourceType::Xhr).is_block());
    // ~method= denies the listed methods only.
    let e = engine("||ads.example^$~method=get");
    assert!(!e
        .decide_with_method(
            "https://ads.example/x",
            page,
            ResourceType::Xhr,
            Some("GET")
        )
        .is_block());
    assert!(e
        .decide_with_method(
            "https://ads.example/x",
            page,
            ResourceType::Xhr,
            Some("POST")
        )
        .is_block());
    // to= scopes by the REQUEST host.
    let e = engine("/track.js$script,to=tracker.example");
    assert!(blocks(
        &e,
        "https://tracker.example/track.js",
        "https://site.example/",
        ResourceType::Script
    ));
    assert!(!blocks(
        &e,
        "https://cdn.example/track.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // The handler threads the method into the pipeline's
    // facts; the pipeline's method gate (filterpipe tests) is
    // what the browser obeys. (Through v27 this spot asserted
    // the same through the runtime seam — deleted in v28,
    // A2-M2, because the seam had no production callers and
    // its POST result described a browser that does not ship.)
}

#[test]
fn multi_domain_cosmetic_scopes_reach_their_hosts() {
    // v22 bug (fixed): comma scopes were filed under one
    // pseudo-host "a.example,b.example" that can never match.
    let e = engine("a.example,b.example##.ad-x\na.example#@#.ad-x");
    let specific = e.specific_cosmetic();
    assert!(
        specific.keys().all(|k| !k.contains(',')),
        "no comma-joined payload keys: {:?}",
        specific.keys().collect::<Vec<_>>()
    );
    assert!(specific
        .get("b.example")
        .map(|v| v.iter().any(|s| s == ".ad-x"))
        .unwrap_or(false));
    // The a.example exception subtracted there only.
    assert!(specific
        .get("a.example")
        .map(|v| !v.iter().any(|s| s == ".ad-x"))
        .unwrap_or(true));
}

#[test]
fn procedural_selectors_under_plain_marker_are_not_css() {
    let e = engine(
        "x.example##.row:has-text(Ad)\nx.example##^script:has-text(foo)\nx.example##.plain-ad",
    );
    assert_eq!(e.report().skipped_procedural, 2);
    assert_eq!(e.report().cosmetic_lines, 1);
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(script.contains(".plain-ad"));
    assert!(
        !script.contains("has-text"),
        "procedural text must not ship as CSS"
    );
}

#[test]
fn prune_log_budget_is_capped() {
    let rt = ShieldRuntime::new(true);
    for _ in 0..pico_core::core::shield::SHIELD_PRUNE_LOG_SAMPLE {
        assert!(rt.claim_prune_log());
    }
    assert!(!rt.claim_prune_log());
}

#[test]
fn anchored_ghide_scopes_to_its_anchor_not_the_world() {
    // The lists' actual spelling (401 of 405 shipped flag
    // lines): @@||host^$ghide. v21-v22 filed these as GLOBAL,
    // which suppressed generic cosmetics on every site once
    // global flags reached the runtime (V22-7).
    let e = engine("@@||tvtoday.example^$ghide\n##.generic-ad");
    assert_eq!(e.report().page_flag_rules, 1);
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(
        script.contains("\"ghAll\":false"),
        "no global generichide may be manufactured: {}",
        &script[script.find("\"gh\"").unwrap_or(0)..script.find("\"gh\"").unwrap_or(0) + 120]
    );
    assert!(
        script.contains("tvtoday.example"),
        "anchor host carried as scoped flag"
    );
    // A domain=-scoped flag keeps working (the other spelling).
    let e = engine("@@*$ghide,domain=web.example");
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(script.contains("\"ghAll\":false"));
    assert!(script.contains("web.example"));
    // And the runtime-visible consequence: on the flagged host
    // generic cosmetics are suppressed; elsewhere they are not.
    // (Asserted structurally here via the data payload; the
    // DOM suite's generichide check proves the runtime honors
    // it — that check exists since v28; through v27 this
    // comment cited a DOM proof that was never written, AUDIT
    // A2 dead-references.)
}

#[test]
fn v26_cosmetic_payload_covers_every_compiled_host() {
    // AUDIT V26 (D6-b): the coverage pin. Building the cosmetic
    // payload with the SHIPPED budget must include every host
    // the corpus compiled — the v25 budget shipped 578 of 1,152
    // (real lists) and long-tail sites silently got generic
    // cosmetics only. If a future list refresh grows the
    // specific set past the budget, THIS test fails loudly.
    // Public build: the counts are corpus-relative — the
    // corpus is small by design, and the invariant (payload
    // hosts == compiled hosts) is what is pinned.
    let e = full_engine();
    let (_script, hosts, _generic) =
        e.build_cosmetic_script(pico_core::core::shield::COSMETIC_SCRIPT_BUDGET);
    assert_eq!(
        hosts,
        e.report().cosmetic_specific_hosts,
        "payload must ship every compiled specific host"
    );
    // AUDIT V28 (history): on the real lists this count was
    // 1,119 — hosts whose ONLY specific cosmetics were uBO
    // procedural actions are counted as skipped_procedural,
    // not shipped. The corpus carries one such host
    // (proc.example) and it must likewise stay out.
    assert!(hosts >= 10, "corpus compiles a real specific set: {hosts}");
    // And the document script built with the shipped budgets
    // reports the same coverage through DocumentScriptInfo.
    let (_doc, info) = e.build_document_script(
        pico_core::core::shield::COSMETIC_SCRIPT_BUDGET,
        pico_core::core::shield::SCRIPTLET_SCRIPT_BUDGET,
    );
    assert_eq!(info.cosmetic_hosts, hosts);
    assert_eq!(info.scriptlet_entries, e.report().scriptlet_entries);
    // The collapse layer rides as the third segment (D6-a).
    let (doc2, _) = e.build_document_script(
        pico_core::core::shield::COSMETIC_SCRIPT_BUDGET,
        pico_core::core::shield::SCRIPTLET_SCRIPT_BUDGET,
    );
    assert!(
        doc2.contains("__picoCollapse"),
        "document script must carry the collapse runtime"
    );
    let cp = doc2.find("__picoShieldCosmetic").expect("cosmetic half");
    let xp = doc2.find("__picoCollapse").expect("collapse segment");
    assert!(cp < xp, "collapse segment follows cosmetics");
}

// ---------------------------------------------------------------------
// AUDIT V28 regressions (filter-pipeline audit, A2)
// ---------------------------------------------------------------------

#[test]
fn v28_style_pseudo_rules_are_procedural_skips_not_css() {
    // A2-H3: `:style(` (and :remove(, :matches-path(,
    // :remove-attr(, :remove-class() are uBO actions, not CSS.
    // Through v27 they shipped inside the concatenated cosmetic
    // rule, and CSS drops the WHOLE rule on one invalid
    // selector — every cosmetic on the host died with it.
    let e = engine(
        "shop.example###zedoads:style(height: 1px !important)\nshop.example##.real-ad\nshop.example##div:remove()\n##.generic-ad",
    );
    assert_eq!(e.report().skipped_procedural, 2);
    let (script, hosts, _) = e.build_cosmetic_script(150_000);
    assert_eq!(hosts, 1);
    assert!(script.contains(".real-ad"), "valid host cosmetic survives");
    assert!(script.contains(".generic-ad"));
    assert!(
        !script.contains("zedoads"),
        "the :style( selector must not ship as CSS"
    );
}

#[test]
fn v28_wildcard_and_trailing_dot_anchors_are_generic_patterns() {
    // A2-M5: these anchor shapes produced by_host keys no real
    // host could ever equal; the rules (and, worse, the @@
    // exceptions among them) were dead. They compile as generic
    // patterns now.
    let e = engine("||torlock.*/sww.js$script");
    assert!(blocks(
        &e,
        "https://torlock.com/sww.js",
        "https://site.example/",
        ResourceType::Script
    ));
    let e = engine("||adservice.google.^$script");
    assert!(blocks(
        &e,
        "https://adservice.google.com/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // The exception twin works too (the dangerous direction:
    // a dead exception leaves a block standing).
    let e = engine("||kwik.example^$script\n@@||kwik.*^$script");
    assert!(!blocks(
        &e,
        "https://kwik.example/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // host:port anchors reach their port too.
    let e = engine("||ads.example:8443^$script");
    assert!(blocks(
        &e,
        "https://ads.example:8443/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn v28_badfilter_and_dedupe_respect_the_exception_bit() {
    // A2-M8: identity() omitted the exception bit, so a block
    // rule and its exact @@ twin shared an identity — the
    // second line was dropped as a duplicate, order-dependent.
    let e = engine("||ads.example^\n@@||ads.example^");
    assert_eq!(e.report().network_rules, 1);
    assert_eq!(e.report().network_exceptions, 1);
    assert!(!blocks(
        &e,
        "https://ads.example/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // Reverse order: same outcome.
    let e = engine("@@||ads.example^\n||ads.example^");
    assert!(!blocks(
        &e,
        "https://ads.example/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
    // A $badfilter exception twin no longer neutralises the
    // block rule (and vice versa).
    let e = engine("||ads.example^\n@@||ads.example^$badfilter");
    assert!(blocks(
        &e,
        "https://ads.example/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn v28_important_exception_beats_important_block() {
    // A2-L14: precedence when BOTH sides carry $important —
    // previously unpinned. The exception wins (uBO semantics).
    let e = engine("||ads.example^$important\n@@||ads.example^$important");
    assert!(!blocks(
        &e,
        "https://ads.example/x.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn v28_generic_exception_channel_carries_scoped_exceptions() {
    // A2-M4: a scoped #@# against a GENERIC selector used to be
    // subtracted from the specific map only — i.e. from
    // nothing. The payload now carries a per-host suppression
    // channel ("ge") the runtime applies to the combined list.
    let e = engine("##.ad-carousel\ndez.example#@#.ad-carousel");
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(
        script.contains("\"ge\":{\"dez.example\":[\".ad-carousel\"]}"),
        "{script:.400}"
    );
    // A2-L4: an exclude-only generic rule suppresses itself on
    // the hosts it spares, through the same channel.
    let e = engine("~hyper.example##.ad-carousel");
    let (script, _, _) = e.build_cosmetic_script(150_000);
    assert!(
        script.contains("\"ge\":{\"hyper.example\":[\".ad-carousel\"]}"),
        "{script:.400}"
    );
}

#[test]
fn v28_trailing_dot_fqdn_still_matches_anchored_rules() {
    // A2-L6: host_of trims the trailing dot, so a dotted FQDN
    // walks the same suffixes as its plain spelling.
    let e = engine("||ads.example.com^$script");
    assert!(blocks(
        &e,
        "https://ads.example.com./x.js",
        "https://site.example/",
        ResourceType::Script
    ));
}

#[test]
fn v28_urls_equivalent_ignores_default_port_and_fragment() {
    // A2-L8: main-frame determination must not be defeated by
    // a spelled-out default port or a fragment.
    use pico_core::core::shield::urls_equivalent;
    assert!(urls_equivalent(
        "https://example.com:443/watch?v=1",
        "https://example.com/watch?v=1"
    ));
    assert!(urls_equivalent(
        "http://example.com:80/a",
        "http://example.com/a"
    ));
    assert!(urls_equivalent(
        "https://example.com/a#section",
        "https://example.com/a"
    ));
    // A path that merely ends in ":80" is not a port.
    assert!(!urls_equivalent(
        "https://example.com/x:80",
        "https://example.com/x"
    ));
    // Non-default ports still distinguish.
    assert!(!urls_equivalent(
        "https://example.com:8443/a",
        "https://example.com/a"
    ));
}
