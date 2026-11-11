//! 0.34 Law 3 — the JS/Rust downloads pair test.
//!
//! `core::dlcmd::DL_COMMANDS` is the SINGLE vocabulary for
//! every downloads command on every surface, and the JS in
//! `assets/chrome.html`, `assets/downloads.html`,
//! `assets/start.html` and the shared renderer
//! `assets/dlrows.js` is its only emitter set. This test
//! pairs the two sides mechanically:
//!
//! - every action token the JS can emit is a verb in the
//!   table (no orphan emitters), and
//! - every verb in the table that has at least one JS
//!   surface (Chrome / Page / Start) is emitted by at
//!   least one of the four files (no orphan verbs).
//!
//! It MUST fail if someone adds a verb to the table
//! without a JS emitter, or adds a JS emitter for a verb
//! that is not in the table — that failure is the point:
//! the 0.33 split (chrome snake_case commands in
//! `core::ipc`, page kebab verbs in `core::dlaction`, ad-hoc
//! arms in `app.rs`) drifted precisely because nothing
//! paired the sides.
//!
//! Extraction is scoped to downloads contexts, because the
//! same files carry OTHER action vocabularies (the start
//! page's `postAction({action: "service", …})` family is
//! `core::action`'s, not this table's). A token counts as
//! emitted when it appears as:
//!
//! - the `action` of a `sendDl({…})` / `postDl({…})` /
//!   DlRows `post({…})` body (any of the spellings
//!   `action:"…"`, `action: "…"`, `"action":"…"`);
//! - a static `data-dl-action="…"` attribute value
//!   (dlrows.js renders the row buttons this way);
//! - a `pageBtn("Label", "verb", …)` verb argument in
//!   dlrows.js (the page row builder);
//! - a branch string of a `data-dl-action` construction
//!   in dlrows.js (the popover's pause/resume ternary and
//!   its `cancelVerb` remove/cancel choice — the two
//!   places the attribute value is computed, not literal).
//!
//! Verb tokens are kebab-case by table law, so extraction
//! keeps only `^[a-z]+(-[a-z]+)*$` strings (a capitalized
//! aria label like `"Resume"` in the same expression is
//! not a token) and skips the `<verb>` placeholder in the
//! files' own doc comments.

use std::collections::BTreeSet;

use pico_core::core::dlcmd::{spec_for, DL_COMMANDS};

const FILES: &[&str] = &[
    "assets/dlrows.js",
    "assets/chrome.html",
    "assets/downloads.html",
    "assets/start.html",
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

/// Read one `"…"` string starting at or after `from`
/// (skipping whitespace, quotes-as-syntax and a colon):
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

/// Every `action` value inside a `sendDl(` / `postDl(` /
/// `post(` call in `src`. The three call names are mutually
/// exclusive substrings (`post(` matches neither `postDl(`
/// nor `postAction(`), so no call is read twice and the
/// start page's separate action family is never read.
fn call_action_tokens(src: &str, out: &mut BTreeSet<String>) {
    for call in ["sendDl(", "postDl(", "post("] {
        let mut pos = 0;
        while let Some(rel) = src[pos..].find(call) {
            let at = pos + rel + call.len();
            // Bound the search to this call's argument head:
            // the action key of a dl body rides within the
            // first object literal.
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

/// Every static `data-dl-action="…"` value in `src`.
/// Computed values (`data-dl-action="' + …`) start with a
/// quote-plus, not a letter — they are handled by
/// [`construction_tokens`].
fn data_action_tokens(src: &str, out: &mut BTreeSet<String>) {
    let needle = "data-dl-action=\"";
    let mut pos = 0;
    while let Some(rel) = src[pos..].find(needle) {
        let start = pos + rel + needle.len();
        let rest = &src[start..];
        let end = rest.find('"').unwrap_or(rest.len());
        let tok = &rest[..end];
        if is_verb_shape(tok) {
            out.insert(tok.to_string());
        }
        pos = start + end;
    }
}

/// dlrows.js's computed emissions: `pageBtn` verb arguments
/// and the branch strings of lines that build a
/// `data-dl-action` value (or choose `cancelVerb`).
fn construction_tokens(src: &str, out: &mut BTreeSet<String>) {
    // pageBtn("Label", "verb", id, …): the verb is the
    // second quoted string of the call.
    let mut pos = 0;
    while let Some(rel) = src[pos..].find("pageBtn(") {
        let at = pos + rel + "pageBtn(".len();
        if let Some((_label, next)) = quoted_value_after(src, at) {
            if let Some((verb, _)) = quoted_value_after(src, next) {
                if is_verb_shape(&verb) {
                    out.insert(verb);
                }
            }
        }
        pos = at;
    }
    // Ternary branches on construction lines:
    // `(resume ? "resume" : "pause")` and
    // `st === "done" ? "remove" : "cancel"` (cancelVerb).
    for line in src.lines() {
        if !(line.contains("data-dl-action") || line.contains("cancelVerb")) {
            continue;
        }
        let Some(q) = line.find('?') else { continue };
        let mut rest = &line[q..];
        while let Some((tok, next)) = quoted_value_after(rest, 0) {
            if is_verb_shape(&tok) {
                out.insert(tok);
            }
            if next >= rest.len() {
                break;
            }
            rest = &rest[next..];
        }
    }
}

/// The full emitted-token set across the four files.
fn emitted_tokens() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for rel in FILES {
        let src = read_asset(rel);
        call_action_tokens(&src, &mut out);
        data_action_tokens(&src, &mut out);
        if *rel == "assets/dlrows.js" {
            construction_tokens(&src, &mut out);
        }
    }
    out
}

#[test]
fn every_emitted_token_is_a_table_verb() {
    let emitted = emitted_tokens();
    assert!(
        emitted.len() >= 21,
        "extraction must find the whole vocabulary, found {}: {emitted:?}",
        emitted.len()
    );
    for tok in &emitted {
        assert!(
            spec_for(tok).is_some(),
            "JS emits downloads action {tok:?} which is not in DL_COMMANDS \
             (add the verb to the table, or fix the emitter)"
        );
    }
}

#[test]
fn every_surfaced_table_verb_is_emitted() {
    let emitted = emitted_tokens();
    for spec in DL_COMMANDS {
        if spec.surfaces.is_empty() {
            continue;
        }
        assert!(
            emitted.contains(spec.name),
            "DL_COMMANDS verb {:?} (surfaces {:?}) has no JS emitter in \
             chrome.html / downloads.html / start.html / dlrows.js \
             (add the emitter, or remove the verb)",
            spec.name,
            spec.surfaces
        );
    }
    // The two easy-to-lose rows, called out by name:
    // copy-link is client-only (dlrows.js handles it in the
    // page and never posts it) but IS emitted as a
    // data-dl-action through pageBtn; dismiss-notice is
    // emitted by chrome.html's toast handler as a post
    // body, not a data attribute.
    assert!(emitted.contains("copy-link"), "copy-link emission lost");
    assert!(
        emitted.contains("dismiss-notice"),
        "dismiss-notice emission lost"
    );
}

#[test]
fn no_legacy_download_command_spellings() {
    // The chrome `Command` spellings W2 deleted from
    // core::ipc in the 0.34 rebuild. As QUOTED command
    // strings they must not survive in any of the four
    // files — the canonical bodies name kebab verbs now.
    const LEGACY_QUOTED: &[&str] = &[
        "downloads_close",
        "downloads_page",
        "download_open_folder",
        "download_set_conn",
        "download_set_default_conn",
    ];
    // The snake_case per-task spellings, in any context.
    const LEGACY_BARE: &[&str] = &[
        "download_pause",
        "download_resume",
        "download_cancel",
        "download_add",
        "downloads_close",
    ];
    for rel in FILES {
        let src = read_asset(rel);
        for spelling in LEGACY_QUOTED {
            assert!(
                !src.contains(&format!("\"{spelling}\"")),
                "{rel} still carries the legacy command string \"{spelling}\""
            );
        }
        for spelling in LEGACY_BARE {
            assert!(
                !src.contains(spelling),
                "{rel} still carries the legacy command spelling {spelling}"
            );
        }
        // `"downloads"` as a quoted string is the deleted
        // panel command's name — with exactly two benign
        // survivors, both whitelisted by their immediate
        // context: the downloads BUTTON's element id
        // (`id="downloads"` in the markup,
        // `getElementById("downloads")` in the script).
        // (The `"#downloads"` selector is not an occurrence:
        // its quote precedes the `#`, not the word.)
        let needle = "\"downloads\"";
        let mut pos = 0;
        while let Some(rel_idx) = src[pos..].find(needle) {
            let at = pos + rel_idx;
            let prefix = &src[..at];
            let benign = prefix.ends_with("id=") || prefix.ends_with("getElementById(");
            assert!(
                benign,
                "{rel}: \"downloads\" occurs outside the whitelisted \
                 element-id contexts (legacy command string?): …{}",
                &prefix[prefix.len().saturating_sub(40)..]
            );
            pos = at + needle.len();
        }
    }
}

#[test]
fn per_file_emissions_stay_on_their_surface() {
    // AUDIT 0.40.1: the app transports now enforce the
    // surface table, so each surface's OWN sender file may
    // only emit verbs that surface allows. (dlrows.js is the
    // shared renderer — its two row builders are pinned by
    // the dlcmd surface-matrix unit test instead; start.html
    // emits no dl bodies of its own.)
    use pico_core::core::dlcmd::DlSurface;
    for (rel, surface) in [
        ("assets/chrome.html", DlSurface::Chrome),
        ("assets/downloads.html", DlSurface::Page),
    ] {
        let src = read_asset(rel);
        let mut toks = BTreeSet::new();
        call_action_tokens(&src, &mut toks);
        data_action_tokens(&src, &mut toks);
        assert!(
            !toks.is_empty(),
            "{rel} extraction found no dl verbs — the enforcement test would be vacuous"
        );
        for tok in &toks {
            let spec = spec_for(tok).expect("emitted verb in table");
            assert!(
                spec.allows(surface),
                "{rel} emits {tok:?}, which the {surface:?} surface does not allow \
                 (the app now denies it — fix the emitter or the table)"
            );
        }
    }
}
