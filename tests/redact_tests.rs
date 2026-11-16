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

#[test]
fn schemeless_authority_userinfo_is_stripped() {
    // AUDIT 0.41.3 (Front C F3 / D-S1): `//user:pass@host`
    // has no scheme, so the `://` scan never saw it and
    // the password survived into the log.
    let out = redact_url("//user:pass@evil.example/x?y=1");
    assert!(!out.contains("pass"), "password leaked: {out}");
    assert!(!out.contains("user:"), "userinfo leaked: {out}");
    assert!(out.contains("evil.example/x"), "host/path stay: {out}");
    // Embedded in a JSON body: the `//` follows a quote,
    // one of the authority-start delimiters.
    let out = redact_url(r#"{"u":"//user:pass@h/x"}"#);
    assert!(!out.contains("pass"), "password leaked: {out}");
    assert!(!out.contains("user:"), "userinfo leaked: {out}");
    assert!(out.contains("h/x"), "host/path stay: {out}");
    // The other delimiter shapes: whitespace, `=`, `(`.
    for input in [
        "see //user:pass@h.example/x end",
        "u=//user:pass@h.example/x",
        "(//user:pass@h.example/x)",
    ] {
        let out = redact_url(input);
        assert!(!out.contains("pass"), "{input} -> {out}");
    }
}

#[test]
fn schemeless_non_authorities_and_schemed_urls_are_unchanged() {
    // AUDIT 0.41.3 (F3): shapes that must NOT change —
    // `a//b` (the `//` follows a letter: no authority
    // begins), ordinary paths, and the schemed URL the
    // original branch already stripped (proving the
    // `://` case is not double-processed: `:` is not a
    // schemeless delimiter).
    assert_eq!(redact_url("a//b"), "a//b");
    assert_eq!(redact_url("some/path//other"), "some/path//other");
    assert_eq!(
        redact_url("https://user:pass@h.example/x"),
        "https://h.example/x"
    );
}

#[test]
fn second_embedded_url_with_short_prefix_never_leaks_userinfo() {
    let body = r#"{"url":"https://a.test/","ref":"https://user:hunter2@b.test/p"}"#;
    let out = redact_url(body);
    assert!(!out.contains("hunter2"), "password leaked: {out}");
    assert!(!out.contains("user:"), "userinfo leaked: {out}");
    assert!(out.contains("b.test/p"), "second host preserved: {out}");

    let msg = redact_url("Failed https://a.test/ -> https://user:hunter2@b.test/p?tok=SECRET");
    assert!(!msg.contains("hunter2"), "password leaked: {msg}");
    assert!(!msg.contains("SECRET"), "query leaked: {msg}");
    assert!(msg.starts_with("Failed "), "prose casing preserved: {msg}");

    let nested =
        redact_url("https://proxy.test/go/https://user:hunter2@b.test/file.zip?sig=SECRET");
    assert!(
        !nested.contains("hunter2"),
        "path-embedded password leaked: {nested}"
    );
    assert!(!nested.contains("SECRET"), "query leaked: {nested}");
}
