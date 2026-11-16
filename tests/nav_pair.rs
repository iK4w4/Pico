//! 0.41.0 — the JS/Rust nav pair test, in the dl_pair
//! pattern (0.34 Law 3).
//!
//! `core::navcmd::NAV_COMMANDS` is the SINGLE vocabulary
//! for every bookmarks/history command on every surface,
//! and the JS in `assets/chrome.html`, `assets/start.html`
//! and `assets/history.html` is its only emitter set. This
//! test pairs the two sides mechanically:
//!
//! - every action token the JS can emit is a verb in the
//!   table (no orphan emitters), and
//! - every verb in the table is emitted by at least one
//!   of the three files (no orphan verbs), and
//! - each file emits ONLY the verbs its surface allows
//!   (the app transports enforce the same table — a file
//!   emitting a foreign surface's verb is a dead control
//!   the moment it ships).
//!
//! Extraction is scoped to nav contexts: a token counts
//! as emitted when it appears as the `action` of a
//! `sendNav({…})` / `postNav({…})` body. (The same files
//! carry other vocabularies — dl bodies, start actions,
//! chrome commands — with their own senders, which this
//! extraction never reads: `sendNav(`/`postNav(` match
//! neither `sendDl(`/`postDl(` nor `postAction(`/`post(`.)
//! Verb tokens are kebab-case by table law, so extraction
//! keeps only `^[a-z]+(-[a-z]+)*$` strings and skips the
//! `<verb>` placeholder in the files' own doc comments.

use std::collections::BTreeSet;

use pico_core::core::navcmd::{spec_for, NavSurface, NAV_COMMANDS};

const FILES: &[(&str, NavSurface)] = &[
    ("assets/chrome.html", NavSurface::Chrome),
    ("assets/start.html", NavSurface::Start),
    ("assets/history.html", NavSurface::History),
];

fn read_asset(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// True for the kebab-case shape every table verb has.
fn is_verb_shape(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
}

/// Read one `"…"` string starting at or after `from`:
/// the value half of an `action: "…"` pair.
fn quoted_value_after(src: &str, from: usize) -> Option<(String, usize)> {
    let bytes = src.as_bytes();
    let mut i = from;
    while i < bytes.len() && bytes[i] != b'"' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let start = i + 1;
    let mut j = start;
    while j < bytes.len() && bytes[j] != b'"' {
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    Some((src[start..j].to_string(), j + 1))
}

/// Every `action` value inside a `sendNav(` / `postNav(`
/// call in `src`.
fn call_action_tokens(src: &str, out: &mut BTreeSet<String>) {
    for call in ["sendNav(", "postNav("] {
        let mut pos = 0;
        while let Some(rel) = src[pos..].find(call) {
            let at = pos + rel + call.len();
            let head_end = (at + 240).min(src.len());
            let head = &src[at..head_end];
            if let Some(krel) = head.find("action") {
                let after_key = at + krel + "action".len();
                if let Some((tok, _)) = quoted_value_after(src, after_key) {
                    if is_verb_shape(&tok) {
                        out.insert(tok);
                    }
                }
            }
            pos = at;
        }
    }
}

fn emitted_tokens() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (rel, _) in FILES {
        call_action_tokens(&read_asset(rel), &mut out);
    }
    out
}

#[test]
fn every_emitted_token_is_a_table_verb() {
    let emitted = emitted_tokens();
    assert!(
        emitted.len() >= 9,
        "extraction must find the whole vocabulary, found {}: {emitted:?}",
        emitted.len()
    );
    for tok in &emitted {
        assert!(
            spec_for(tok).is_some(),
            "JS emits nav action {tok:?} which is not in NAV_COMMANDS \
             (add the verb to the table, or fix the emitter)"
        );
    }
}

#[test]
fn every_table_verb_is_emitted() {
    let emitted = emitted_tokens();
    for spec in NAV_COMMANDS {
        assert!(
            emitted.contains(spec.name),
            "NAV_COMMANDS verb {:?} (surfaces {:?}) has no JS emitter in \
             chrome.html / start.html / history.html \
             (add the emitter, or remove the verb)",
            spec.name,
            spec.surfaces
        );
    }
}

#[test]
fn per_file_emissions_stay_on_their_surface() {
    for (rel, surface) in FILES {
        let src = read_asset(rel);
        let mut toks = BTreeSet::new();
        call_action_tokens(&src, &mut toks);
        assert!(
            !toks.is_empty(),
            "{rel} extraction found no nav verbs — the enforcement test would be vacuous"
        );
        for tok in &toks {
            let spec = spec_for(tok).expect("emitted verb in table");
            assert!(
                spec.allows(*surface),
                "{rel} emits {tok:?}, which the {surface:?} surface does not allow \
                 (the app denies it — fix the emitter or the table)"
            );
        }
    }
}
