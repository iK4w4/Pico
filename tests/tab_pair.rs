//! 0.41.0 (Workstream C) — the JS/Rust tab pair test, in
//! the dl_pair / nav_pair pattern.
//!
//! `core::tabcmd::TAB_COMMANDS` is the SINGLE vocabulary
//! for every movable-tab command, and the JS in
//! `assets/chrome.html` is its only emitter. This test
//! pairs the two sides mechanically:
//!
//! - every action token the JS can emit is a verb in the
//!   table (no orphan emitters), and
//! - every verb in the table is emitted by the chrome
//!   (no orphan verbs), and
//! - the chrome emits ONLY verbs its surface allows.
//!
//! Extraction is scoped to tab contexts: a token counts
//! as emitted when it appears as the `action` of a
//! `sendTab({…})` body. (The same file carries other
//! vocabularies — dl bodies, nav bodies, chrome commands
//! — with their own senders, which this extraction never
//! reads: `sendTab(` matches neither `sendDl(`/`sendNav(`
//! nor `send(`.) Verb tokens are kebab-case by table law,
//! so extraction keeps only `^[a-z]+(-[a-z]+)*$` strings
//! and skips the `<verb>` placeholder in the file's own
//! doc comments.

use std::collections::BTreeSet;

use pico_core::core::tabcmd::{spec_for, TabSurface, TAB_COMMANDS};

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

/// Every `action` value inside a `sendTab(` call in `src`.
fn call_action_tokens(src: &str, out: &mut BTreeSet<String>) {
    let call = "sendTab(";
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

fn emitted_tokens() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    call_action_tokens(&read_asset("assets/chrome.html"), &mut out);
    out
}

#[test]
fn every_emitted_token_is_a_table_verb() {
    let emitted = emitted_tokens();
    assert!(
        emitted.len() >= 3,
        "extraction must find the whole vocabulary, found {}: {emitted:?}",
        emitted.len()
    );
    for tok in &emitted {
        assert!(
            spec_for(tok).is_some(),
            "JS emits tab action {tok:?} which is not in TAB_COMMANDS \
             (add the verb to the table, or fix the emitter)"
        );
    }
}

#[test]
fn every_table_verb_is_emitted() {
    let emitted = emitted_tokens();
    for spec in TAB_COMMANDS {
        assert!(
            emitted.contains(spec.name),
            "TAB_COMMANDS verb {:?} (surfaces {:?}) has no JS emitter in \
             chrome.html (add the emitter, or remove the verb)",
            spec.name,
            spec.surfaces
        );
    }
}

#[test]
fn chrome_emissions_stay_on_the_chrome_surface() {
    let src = read_asset("assets/chrome.html");
    let mut toks = BTreeSet::new();
    call_action_tokens(&src, &mut toks);
    assert!(
        !toks.is_empty(),
        "chrome.html extraction found no tab verbs — the enforcement test would be vacuous"
    );
    for tok in &toks {
        let spec = spec_for(tok).expect("emitted verb in table");
        assert!(
            spec.allows(TabSurface::Chrome),
            "chrome.html emits {tok:?}, which the Chrome surface does not allow"
        );
    }
}
