//! Log-redaction tests (AUDIT M5): pico.log must never carry query
//! strings, fragments, or userinfo — while still being useful to
//! debug with. (AUDIT V29, A5-9: the path — including a server
//! root token inside it — is KEPT by deliberate, documented
//! decision; see core::redact's module docs. An earlier header
//! here claimed the opposite and invited a future "fix" that
//! would have re-broken debuggability.)

use pico_core::core::redact::{cap_and_clean, is_invisible_format, redact_url};

#[test]
fn queries_and_fragments_are_hidden_not_dropped_silently() {
    assert_eq!(
        redact_url("http://localhost:3000/login?code=SECRET&next=/x"),
        "http://localhost:3000/login?…"
    );
    assert_eq!(
        redact_url("https://ex.test/a/b#token=SECRET"),
        "https://ex.test/a/b#…"
    );
    assert_eq!(
        redact_url("https://ex.test/a?x=1#y=2"),
        "https://ex.test/a?…#…"
    );
    assert_eq!(redact_url("https://ex.test/plain"), "https://ex.test/plain");
}

#[test]
fn userinfo_never_survives() {
    let out = redact_url("http://user:hunter2@127.0.0.1:8080/x?y=1");
    assert_eq!(out, "http://127.0.0.1:8080/x?…");
    assert!(!out.contains("hunter2"));
    assert!(!out.contains("user"));
}

#[test]
fn server_root_tokens_are_paths_but_queries_still_hidden() {
    // The path (including a root token) stays visible for debugging;
    // anything after ? does not.
    let out = redact_url("http://127.0.0.1:51000/r/123456789/index.html?key=zzz");
    assert!(out.starts_with("http://127.0.0.1:51000/r/123456789/index.html"));
    assert!(!out.contains("zzz"));
    assert!(out.ends_with("?…"));
}

#[test]
fn non_url_strings_get_same_treatment() {
    assert_eq!(redact_url("pico://recent/2"), "pico://recent/2");
    assert_eq!(redact_url("plain error text"), "plain error text");
    let out = redact_url("weird?query=1");
    assert!(!out.contains("query=1"));
}

#[test]
fn cap_and_clean_strips_controls_and_bounds_length() {
    let out = cap_and_clean("a\u{1b}[31mred\u{7}\nnext line");
    assert!(!out.contains('\u{1b}'));
    assert!(!out.contains('\n'));
    assert!(out.contains("red"));
    let long = "x".repeat(5000);
    let out = cap_and_clean(&long);
    assert!(
        out.chars().count() <= 320,
        "capped, got {}",
        out.chars().count()
    );
    assert!(out.ends_with("…[truncated]"));
}

#[test]
fn tags_range_and_u2065_are_invisible_format() {
    // AUDIT 0.31 (A5-L5): U+2065 and the Unicode Tags range
    // (U+E0001, U+E0020..=U+E007F) join the invisible-format
    // set; the neighbours just outside the ranges do not.
    assert!(is_invisible_format('\u{2065}'));
    assert!(is_invisible_format('\u{E0001}'));
    assert!(is_invisible_format('\u{E0020}'));
    assert!(is_invisible_format('\u{E0041}'));
    assert!(is_invisible_format('\u{E007F}'));
    assert!(!is_invisible_format('\u{E0000}'));
    assert!(!is_invisible_format('\u{E0002}'));
    assert!(!is_invisible_format('\u{E001F}'));
    assert!(!is_invisible_format('\u{E0080}'));
    // And cap_and_clean strips them from log output.
    let clean = cap_and_clean("a\u{2065}b\u{E0041}c");
    assert!(!clean.contains('\u{2065}'));
    assert!(!clean.contains('\u{E0041}'));
}

#[test]
fn redaction_output_is_bounded_for_monster_urls() {
    let monster = format!("http://h/{}?q={}", "p".repeat(3000), "s".repeat(3000));
    let out = redact_url(&monster);
    assert!(out.chars().count() <= 320);
    assert!(!out.contains("ssss"));
}

#[test]
fn embedded_userinfo_in_a_raw_body_is_stripped() {
    // AUDIT 0.32.1 (P2-6): a rejected chrome command is
    // logged through redact_url with its RAW JSON body. The
    // schemed-URL branch never fires for a body (the text
    // before "://" is the JSON prefix), and the fallback used
    // to cut only at '?'/'#' — so a download_add URL's
    // user:pass@, sitting before any '?', reached pico.log
    // verbatim. The fallback now strips userinfo from any
    // embedded URL too.
    let body = r#"{"t":"download_add","url":"https://user:hunter2@files.example.com/x.zip"}"#;
    let out = redact_url(body);
    assert!(!out.contains("hunter2"), "password leaked: {out}");
    assert!(!out.contains("user:"), "userinfo leaked: {out}");
    assert!(
        out.contains("files.example.com"),
        "the host stays for debugging: {out}"
    );
    // A bare (non-JSON) string carrying a credentialed URL
    // gets the same treatment.
    let out = redact_url("navigation failed for https://a:b@h.example/p end");
    assert!(!out.contains("a:b@"), "{out}");
    assert!(out.contains("h.example/p"), "{out}");
}
