//! Address-parsing and IPC-protocol tests: the two input surfaces a
//! user (or a hostile page) can reach without any other gate.

use pico_core::core::addr::{
    is_allowed_navigation_url, is_safe_local_path, is_unc_address, normalize_pico_url, parse_input,
    same_origin, web_origin, Target,
};
use pico_core::core::ipc::{parse_command, parse_nav_note, Command, Dedupe, NavHow, SocialSite};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// parse_input
// ---------------------------------------------------------------------------

#[test]
fn bare_port_becomes_loopback_url() {
    assert_eq!(
        parse_input("3000"),
        Some(Target::Url("http://127.0.0.1:3000/".to_string()))
    );
    assert_eq!(
        parse_input("5173"),
        Some(Target::Url("http://127.0.0.1:5173/".to_string()))
    );
}

#[test]
fn localhost_forms_normalize() {
    assert_eq!(
        parse_input("localhost:8080/app"),
        Some(Target::Url("http://localhost:8080/app".to_string()))
    );
    assert_eq!(
        parse_input("127.0.0.1:3000"),
        Some(Target::Url("http://127.0.0.1:3000".to_string()))
    );
    assert_eq!(
        parse_input("[::1]:9000/x"),
        Some(Target::Url("http://[::1]:9000/x".to_string()))
    );
}

#[test]
fn full_urls_pass_through() {
    assert_eq!(
        parse_input("https://example.com/a?b=1"),
        Some(Target::Url("https://example.com/a?b=1".to_string()))
    );
    assert_eq!(
        parse_input("http://192.168.1.10:8000/"),
        Some(Target::Url("http://192.168.1.10:8000/".to_string()))
    );
}

#[test]
fn folded_start_url_typed_by_hand_is_the_start_page() {
    // AUDIT 0.30.1 (A6-M1 residue): wry's folded spelling of
    // the internal start URL, typed into the address bar,
    // must parse to the canonical start target — through
    // 0.30 it parsed as a web URL and bypassed the shell's
    // go-home guard.
    // The spellings wry's rewrite actually produces (and a
    // hand-typed lowercase twin) — the fold is deliberately
    // case-sensitive, matching normalize_pico_url.
    for spelling in [
        "https://pico.start/",
        "https://pico.start",
        "http://pico.start/",
    ] {
        assert_eq!(
            parse_input(spelling),
            Some(Target::Url("pico://start".to_string())),
            "{spelling} is the start page"
        );
    }
    // Real pico.* websites are NOT folded (the V15-2 law).
    assert_eq!(
        parse_input("https://pico.dev/"),
        Some(Target::Url("https://pico.dev/".to_string()))
    );
    assert_eq!(
        parse_input("https://pico.example.com/start"),
        Some(Target::Url("https://pico.example.com/start".to_string()))
    );
}

#[test]
fn domain_without_scheme_gets_https() {
    assert_eq!(
        parse_input("example.com/docs"),
        Some(Target::Url("https://example.com/docs".to_string()))
    );
    assert_eq!(
        parse_input("sub.example.co.uk"),
        Some(Target::Url("https://sub.example.co.uk".to_string()))
    );
}

#[test]
fn junk_inputs_are_rejected() {
    // v24 note: "hello world" and "a b c" USED to be junk —
    // since the address bar learned to search (AUDIT V24-4),
    // prose is a query, not junk, and they are pinned in the
    // search matrix below. What remains junk: empties and
    // scheme-smuggling.
    for junk in [
        "",
        "   ",
        "javascript:alert(1)",
        "java script:alert(1)",
        "data:text/html,<b>x</b>",
        "http://",
    ] {
        assert_eq!(parse_input(junk), None, "input {junk:?} must be rejected");
    }
}

fn search(query: &str) -> Option<Target> {
    Some(Target::Url(format!(
        "https://www.google.com/search?q={query}"
    )))
}

/// The v24 address-bar search matrix (AUDIT V24-4): what
/// becomes a Google search, what still navigates, what still
/// belongs to the file logic — and what stays refused.
#[test]
fn address_bar_search_matrix() {
    // ---- prose and single labels become searches ----
    assert_eq!(parse_input("youtube"), search("youtube"));
    assert_eq!(parse_input("github"), search("github"));
    assert_eq!(parse_input("pico"), search("pico"));
    assert_eq!(
        parse_input("how to cook rice"),
        search("how%20to%20cook%20rice")
    );
    assert_eq!(parse_input("hello world"), search("hello%20world"));
    assert_eq!(parse_input("a b c"), search("a%20b%20c"));
    // Leading/trailing space is trimmed before anything else.
    assert_eq!(parse_input("  youtube  "), search("youtube"));
    // Reserved characters in the query are percent-encoded.
    assert_eq!(parse_input("what is 2+2?"), search("what%20is%202%2B2%3F"));
    // Unicode queries encode as UTF-8 percent-escapes.
    assert_eq!(parse_input("café menu"), search("caf%C3%A9%20menu"));
    match parse_input("پرسش") {
        Some(Target::Url(u)) => {
            assert!(
                u.starts_with("https://www.google.com/search?q=%D9%BE"),
                "{u}"
            )
        }
        other => panic!("a unicode single word must search, got {other:?}"),
    }
    // A colon after a MULTI-WORD prefix is prose punctuation:
    // the prefix is no scheme token and the tail has whitespace
    // too, so the input falls through to the search branch.
    assert_eq!(
        parse_input("how to: cook rice"),
        search("how%20to%3A%20cook%20rice")
    );
    // But a scheme-SHAPED single word before the colon is a
    // scheme claim and stays refused, prose after it or not.
    assert_eq!(parse_input("recipe: pasta sauce"), None);
    // Not plausible hosts: numeric / one-letter TLDs, single
    // labels with path segments, underscore labels.
    assert_eq!(parse_input("foo.123"), search("foo.123"));
    assert_eq!(parse_input("example.c"), search("example.c"));
    assert_eq!(parse_input("foo/bar"), search("foo%2Fbar"));
    assert_eq!(parse_input("my_server"), search("my_server"));

    // ---- plausible bare hosts still navigate ----
    assert_eq!(
        parse_input("example.com"),
        Some(Target::Url("https://example.com".to_string()))
    );
    assert_eq!(
        parse_input("sub.example.co.uk/path"),
        Some(Target::Url("https://sub.example.co.uk/path".to_string()))
    );
    assert_eq!(
        parse_input("docs.rs/crate"),
        Some(Target::Url("https://docs.rs/crate".to_string()))
    );
    // A bare IPv4 navigates (it is a host, not a query).
    assert_eq!(
        parse_input("192.168.1.1"),
        Some(Target::Url("https://192.168.1.1".to_string()))
    );
    // Local forms are untouched.
    assert_eq!(
        parse_input("localhost:3000"),
        Some(Target::Url("http://localhost:3000".to_string()))
    );
    assert_eq!(
        parse_input("127.0.0.1:8080"),
        Some(Target::Url("http://127.0.0.1:8080".to_string()))
    );
    // Full URLs pass through byte-identical.
    assert_eq!(
        parse_input("https://example.com/a?b=1"),
        Some(Target::Url("https://example.com/a?b=1".to_string()))
    );

    // ---- file-name-shaped tokens follow the FILE logic ----
    // A typed "LLM.html" that does not exist on disk takes the
    // file logic's legacy https tail — NEVER a search. (The
    // existing-file case is File, pinned by
    // existing_file_path_becomes_file_target.)
    for name in [
        "LLM.html",
        "notes.txt",
        "PAGE.HTML",
        "app.json",
        "pages/manual.pdf",
    ] {
        match parse_input(name) {
            Some(Target::Url(u)) => assert!(
                !u.contains("google.com/search"),
                "{name} must follow file logic, not search (got {u})"
            ),
            other => panic!("{name}: expected the legacy file-logic tail, got {other:?}"),
        }
    }
    // Another process's stray CWD file named like a host word
    // must not flip a search into a navigation: a plain single
    // word with no dot and no file extension searches even
    // though a file could theoretically share its name in some
    // directory — parse_input only opens EXISTING files, and
    // "readme" here resolves through the search branch unless
    // the file exists (it does not in the test CWD).
    assert_eq!(parse_input("readme"), search("readme"));
}

#[test]
fn control_characters_are_rejected() {
    assert_eq!(parse_input("http://example.com/\u{7}"), None);
    assert_eq!(parse_input("exam\nple.com"), None);
    assert_eq!(parse_input("\u{0}3000"), None);
}

#[test]
fn existing_file_path_becomes_file_target() {
    let f = std::env::temp_dir().join("pico-v15-addr-test.html");
    std::fs::write(&f, "<title>x</title>").expect("write temp file");
    let got = parse_input(&f.to_string_lossy());
    assert_eq!(got, Some(Target::File(f.clone())));
    // The file:// spelling of the same existing file resolves to the
    // same File target (the app then serves it through the static
    // server — never as a raw file:// document).
    let url = format!("file://{}", f.to_string_lossy());
    assert_eq!(parse_input(&url), Some(Target::File(f.clone())));
    let _ = std::fs::remove_file(&f);
}

#[test]
fn unc_paths_are_refused_as_input() {
    assert_eq!(parse_input(r"\\evil.example\share\x.html"), None);
    assert_eq!(parse_input(r"\\?\C:\secret.html"), None);
    assert!(is_unc_address(r"\\server\share"));
    assert!(is_unc_address("//server/share"));
    assert!(!is_unc_address(r"C:\pages\a.html"));
    assert!(!is_safe_local_path(Path::new(r"\\server\share\x.html")));
    assert!(!is_safe_local_path(Path::new(r"\\.\pipe\x")));
    assert!(is_safe_local_path(Path::new("/tmp/a.html")));
}

// ---------------------------------------------------------------------------
// navigation allowlist / origins
// ---------------------------------------------------------------------------

#[test]
fn navigation_allowlist() {
    for ok in [
        "http://example.com/",
        "https://example.com/a?b=c#d",
        "pico://start",
        "http://127.0.0.1:4321/r/123/x.html",
    ] {
        assert!(is_allowed_navigation_url(ok), "{ok} should be allowed");
    }
    for no in [
        "javascript:alert(1)",
        "data:text/html;base64,PHNjcmlwdD4=",
        "file:///C:/Windows/win.ini",
        "view-source:http://example.com/",
        "res://ieframe.dll/",
        "vbscript:msgbox",
        "chrome://settings",
        "about:blank",
        "",
        "http://exa mple.com/",
        "https://example.com/\u{1}",
    ] {
        assert!(!is_allowed_navigation_url(no), "{no:?} must be refused");
    }
}

#[test]
fn origins_compare_by_scheme_host_port() {
    assert!(same_origin("http://a.test:80/x", "http://a.test/y"));
    assert!(same_origin("https://a.test/x", "https://a.test:443/y"));
    assert!(!same_origin("http://a.test/", "https://a.test/"));
    assert!(!same_origin("http://a.test:1/", "http://a.test:2/"));
    assert!(!same_origin("http://a.test/", "http://b.test/"));
    assert_eq!(
        web_origin("http://EXAMPLE.com:8080/x"),
        Some(("http".to_string(), "example.com".to_string(), 8080))
    );
    assert_eq!(web_origin("pico://start"), None);
    assert_eq!(web_origin("not a url"), None);
}

// ---------------------------------------------------------------------------
// IPC commands
// ---------------------------------------------------------------------------

fn cmd_json(t: &str) -> String {
    format!(r#"{{"v":1,"boot":"bootA","req":1,"t":"{t}"}}"#)
}

#[test]
fn commands_parse() {
    let env = parse_command(&cmd_json("new_tab")).expect("new_tab parses");
    assert_eq!(env.cmd, Command::NewTab);
    let env = parse_command(r#"{"boot":"b","req":7,"t":"close_tab","tab":42}"#).expect("close_tab");
    assert_eq!(env.cmd, Command::CloseTab { tab: 42 });
    let env = parse_command(r#"{"boot":"b","req":8,"t":"navigate","text":"localhost:3000"}"#)
        .expect("nav");
    assert_eq!(
        env.cmd,
        Command::Navigate {
            text: "localhost:3000".to_string()
        }
    );
    let env = parse_command(r#"{"boot":"b","req":9,"t":"cycle_tab","dir":-1}"#).expect("cycle");
    assert_eq!(env.cmd, Command::CycleTab { dir: -1 });
}

#[test]
fn v25_commands_parse() {
    // The wire names the chrome page emits (scripts/dom_test.mjs
    // pins the emitting side of the same strings).
    for (wire, want) in [
        ("lite_toggle", Command::LiteToggle),
        ("teleport", Command::Teleport),
        ("teleport_close", Command::TeleportClose),
        ("teleport_copy", Command::TeleportCopy),
    ] {
        let env = parse_command(&cmd_json(wire)).expect(wire);
        assert_eq!(env.cmd, want, "{wire}");
    }
}

#[test]
fn commands_reject_garbage() {
    for bad in [
        "",
        "not json",
        "{}",
        r#"{"req":1,"t":"new_tab"}"#,                    // no boot
        r#"{"boot":"b","t":"new_tab"}"#,                 // no req
        r#"{"boot":"b","req":1,"t":"rm_rf"}"#,           // unknown command
        r#"{"boot":"","req":1,"t":"new_tab"}"#,          // empty boot
        r#"{"v":2,"boot":"b","req":1,"t":"new_tab"}"#,   // future protocol
        r#"{"v":"1","boot":"b","req":1,"t":"new_tab"}"#, // version not a number
    ] {
        assert!(parse_command(bad).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn commands_enforce_size_caps() {
    let big_text = "x".repeat(9000);
    let body = format!(r#"{{"boot":"b","req":1,"t":"navigate","text":"{big_text}"}}"#);
    assert!(parse_command(&body).is_err());
    let big_body = format!(r#"{{"boot":"{}","req":1,"t":"new_tab"}}"#, "b".repeat(200));
    assert!(parse_command(&big_body).is_err(), "boot id cap");
}

#[test]
fn dedupe_drops_replays_and_resets_on_new_boot() {
    let mut d = Dedupe::new(4);
    let env = parse_command(&cmd_json("reload")).expect("parse");
    assert!(d.check(&env), "first delivery accepted");
    assert!(!d.check(&env), "exact replay dropped");
    // AUDIT V28 A1-L1: a different boot's ordinary command does
    // NOT flip the live boot (a stale queued command from a
    // reloaded generation must not clear the live seen-set)…
    let other_boot =
        parse_command(r#"{"boot":"bootB","req":1,"t":"reload"}"#).expect("parse other boot");
    assert!(!d.check(&other_boot), "non-ready new boot is not adopted");
    assert!(!d.check(&env), "live boot's dedupe state survived");
    // …but a new generation's `ready` handshake adopts it and
    // resets the set.
    let other_ready =
        parse_command(r#"{"boot":"bootB","req":1,"t":"ready"}"#).expect("parse other ready");
    assert!(d.check(&other_ready), "ready adopts the new boot");
    assert!(!d.check(&other_ready));
    let other_cmd =
        parse_command(r#"{"boot":"bootB","req":2,"t":"reload"}"#).expect("parse other cmd");
    assert!(d.check(&other_cmd), "the adopted boot's commands flow");
}

#[test]
fn v28_command_parse_pins_and_cycle_dir_validation() {
    // A1-I2: direct parse pins for the verbs that previously had
    // none, and A1-L3: cycle_tab dir is ±1 or rejection.
    for (body, want) in [
        (
            r#"{"boot":"b","req":1,"t":"switch_tab","tab":3}"#,
            Command::SwitchTab { tab: 3 },
        ),
        (r#"{"boot":"b","req":1,"t":"back"}"#, Command::Back),
        (r#"{"boot":"b","req":1,"t":"forward"}"#, Command::Forward),
        (r#"{"boot":"b","req":1,"t":"home"}"#, Command::Home),
        (r#"{"boot":"b","req":1,"t":"open_file"}"#, Command::OpenFile),
        (
            r#"{"boot":"b","req":1,"t":"theme_toggle"}"#,
            Command::ThemeToggle,
        ),
        (r#"{"boot":"b","req":1,"t":"win_min"}"#, Command::WinMin),
        (r#"{"boot":"b","req":1,"t":"win_max"}"#, Command::WinMax),
        (r#"{"boot":"b","req":1,"t":"win_drag"}"#, Command::WinDrag),
        (r#"{"boot":"b","req":1,"t":"win_close"}"#, Command::WinClose),
        (
            r#"{"boot":"b","req":1,"t":"cycle_tab","dir":1}"#,
            Command::CycleTab { dir: 1 },
        ),
    ] {
        let env = parse_command(body).expect(body);
        assert_eq!(env.cmd, want, "{body}");
    }
    for bad_dir in ["0", "2", "-2", "2147483647"] {
        let body = format!(r#"{{"boot":"b","req":1,"t":"cycle_tab","dir":{bad_dir}}}"#);
        assert!(
            parse_command(&body).is_err(),
            "dir {bad_dir} must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// social command (v15 branding): enum-only, constant URLs
// ---------------------------------------------------------------------------

#[test]
fn social_command_carries_only_the_enum() {
    let env = parse_command(r#"{"v":1,"boot":"b","req":1,"t":"social","site":"telegram"}"#)
        .expect("telegram social command parses");
    assert_eq!(
        env.cmd,
        Command::Social {
            site: SocialSite::Telegram
        }
    );
    let env = parse_command(r#"{"v":1,"boot":"b","req":2,"t":"social","site":"youtube"}"#)
        .expect("youtube social command parses");
    assert_eq!(
        env.cmd,
        Command::Social {
            site: SocialSite::Youtube
        }
    );
    // The enum maps to exactly the author's two constant URLs.
    assert_eq!(SocialSite::Telegram.url(), "https://t.me/iK4w4");
    assert_eq!(SocialSite::Youtube.url(), "https://www.youtube.com/@iK4w4");
    // Unknown variants, missing fields, and URL-shaped smuggling are
    // all rejected — the feature cannot become an open redirect.
    for bad in [
        r#"{"v":1,"boot":"b","req":3,"t":"social","site":"myspace"}"#,
        r#"{"v":1,"boot":"b","req":3,"t":"social","site":"https://evil.example"}"#,
        r#"{"v":1,"boot":"b","req":3,"t":"social"}"#,
        r#"{"v":1,"boot":"b","req":3,"t":"social","site":"Telegram"}"#,
    ] {
        assert!(parse_command(bad).is_err(), "{bad} must be rejected");
    }
    // The start-page action channel shares the same enum discipline.
    assert_eq!(
        SocialSite::from_wire("telegram"),
        Some(SocialSite::Telegram)
    );
    assert_eq!(SocialSite::from_wire("youtube"), Some(SocialSite::Youtube));
    assert_eq!(SocialSite::from_wire("Telegram"), None);
    assert_eq!(SocialSite::from_wire("https://t.me/iK4w4"), None);
    assert_eq!(SocialSite::from_wire(""), None);
}

// ---------------------------------------------------------------------------
// dedupe retirement (AUDIT V15-1): a destroyed chrome generation
// must never speak again
// ---------------------------------------------------------------------------

#[test]
fn dedupe_retired_boot_is_dropped_forever() {
    let mut d = Dedupe::new(16);
    let gen1 = parse_command(r#"{"boot":"gen1","req":1,"t":"ready"}"#).expect("parse");
    assert!(d.check(&gen1), "gen1 first delivery accepted");
    // The watchdog destroys gen1's WebView and retires its boot.
    d.retire_current_boot();
    // A stale gen1 command still queued in the event loop (even a
    // late `ready`, even a fresh req number) is dropped…
    let stale_ready = parse_command(r#"{"boot":"gen1","req":2,"t":"ready"}"#).expect("parse");
    assert!(!d.check(&stale_ready), "retired boot must be dropped");
    let stale_close = parse_command(r#"{"boot":"gen1","req":3,"t":"win_close"}"#).expect("parse");
    assert!(!d.check(&stale_close), "retired boot must stay dropped");
    // …and dropping it must NOT disturb the live generation: gen2's
    // boot is adopted once and its dedupe state survives further
    // stale deliveries.
    let gen2 = parse_command(r#"{"boot":"gen2","req":1,"t":"ready"}"#).expect("parse");
    assert!(d.check(&gen2), "gen2 first delivery accepted");
    let stale_again = parse_command(r#"{"boot":"gen1","req":9,"t":"reload"}"#).expect("parse");
    assert!(
        !d.check(&stale_again),
        "gen1 stays retired after gen2 boots"
    );
    assert!(
        !d.check(&gen2),
        "gen2 dedupe state must survive stale gen1 traffic"
    );
    // Retiring with no live boot is a no-op, not a panic.
    let mut fresh = Dedupe::new(16);
    fresh.retire_current_boot();
    let env = parse_command(&cmd_json("reload")).expect("parse");
    assert!(fresh.check(&env));
}

// ---------------------------------------------------------------------------
// normalize_pico_url (AUDIT V15-2): fold ONLY Pico's internal hosts
// ---------------------------------------------------------------------------

#[test]
fn normalize_folds_only_internal_hosts() {
    // wry's rewrites of Pico's own pages/actions fold back.
    for (raw, want) in [
        ("https://pico.chrome/", "pico://chrome/"),
        ("https://pico.chrome", "pico://chrome"),
        ("http://pico.start/", "pico://start/"),
        ("https://pico.recent/3", "pico://recent/3"),
        ("https://pico.pick-file/", "pico://pick-file/"),
        ("https://pico.social/telegram", "pico://social/telegram"),
        // 0.30: the band's Reset anchor folds too (A1/A4).
        ("https://pico.stats-reset/", "pico://stats-reset/"),
        ("https://pico.stats-reset", "pico://stats-reset"),
        ("pico://chrome", "pico://chrome"),
        ("pico://start", "pico://start"),
    ] {
        assert_eq!(normalize_pico_url(raw), want, "fold of {raw}");
    }
    // Real websites on pico.* domains are NOT folded — they stay
    // ordinary https URLs and remain browsable.
    for raw in [
        "https://pico.dev/",
        "https://pico.sh/docs",
        "https://pico.example.com/",
        "https://pico.chrome.evil.example/",
    ] {
        assert_eq!(normalize_pico_url(raw), raw, "{raw} must not fold");
    }
    // Trick shapes never fold INTO an internal page: a dotted host
    // is not an internal host even when its first label matches.
    assert_ne!(
        normalize_pico_url("https://pico.chrome.evil.example/"),
        "pico://chrome/"
    );
    // Ordinary URLs pass through untouched.
    assert_eq!(
        normalize_pico_url("https://example.com/pico.chrome"),
        "https://example.com/pico.chrome"
    );
}

#[test]
fn dedupe_stays_bounded() {
    let mut d = Dedupe::new(16);
    for req in 0..500u64 {
        let body = format!(r#"{{"boot":"b","req":{req},"t":"reload"}}"#);
        let env = parse_command(&body).expect("parse");
        assert!(d.check(&env));
    }
    // The oldest entries were evicted (that's fine — req ids are
    // monotonic per boot, an evicted replay is ancient history).
    let old = parse_command(r#"{"boot":"b","req":0,"t":"reload"}"#).expect("parse");
    assert!(d.check(&old), "evicted id is accepted again by design");
}

// ---------------------------------------------------------------------------
// navigation notes
// ---------------------------------------------------------------------------

#[test]
fn nav_notes_parse_and_classify() {
    let note =
        parse_nav_note(r#"{"kind":"nav","how":"push","href":"http://a.test/x","title":"T"}"#)
            .expect("push note");
    assert_eq!(note.how, NavHow::Push);
    assert_eq!(note.href, "http://a.test/x");
    let note =
        parse_nav_note(r#"{"kind":"nav","how":"pop","href":"https://a.test/"}"#).expect("pop note");
    assert_eq!(note.how, NavHow::Pop);
    assert_eq!(note.title, "");
}

#[test]
fn nav_notes_reject_non_web_and_oversize() {
    for bad in [
        r#"{"kind":"nav","how":"push","href":"file:///etc/passwd"}"#,
        r#"{"kind":"nav","how":"push","href":"pico://start"}"#,
        r#"{"kind":"nav","how":"teleport","href":"http://a.test/"}"#,
        r#"{"kind":"cmd","how":"push","href":"http://a.test/"}"#,
        r#"{"kind":"nav","how":"push"}"#,
        "garbage",
    ] {
        assert!(parse_nav_note(bad).is_none(), "{bad} must be dropped");
    }
    let long_title = "t".repeat(5000);
    let body =
        format!(r#"{{"kind":"nav","how":"init","href":"http://a.test/","title":"{long_title}"}}"#);
    let note = parse_nav_note(&body).expect("long titles truncate, not drop");
    assert!(note.title.chars().count() <= 512);
}

#[test]
fn file_url_conversion_is_windows_only_safe() {
    // On any platform a file:// URL inside a served page is not a
    // local path we honor; the helper exists for completeness.
    let p = pico_core::core::addr::file_url_to_path("http://example.com/x");
    assert_eq!(p, None::<PathBuf>);
    // A genuine file:// URL converts (the app re-checks it through
    // the UNC gate and canonicalization before touching the disk).
    assert_eq!(
        pico_core::core::addr::file_url_to_path("file:///tmp/x.html"),
        Some(PathBuf::from("/tmp/x.html"))
    );
    // UNC file URLs never convert to a local path.
    assert_eq!(
        pico_core::core::addr::file_url_to_path("file://evil.example/share/x.html"),
        None::<PathBuf>
    );
}

#[test]
fn navigate_text_boundary_is_exact() {
    // 8192 bytes of navigate text is the cap: at it passes, above fails.
    let at = format!(
        r#"{{"boot":"b","req":1,"t":"navigate","text":"{}"}}"#,
        "a".repeat(8192)
    );
    assert!(parse_command(&at).is_ok());
    let over = format!(
        r#"{{"boot":"b","req":1,"t":"navigate","text":"{}"}}"#,
        "a".repeat(8193)
    );
    assert!(parse_command(&over).is_err());
    // A nav note with an over-cap href is dropped, not truncated.
    let long_href = format!(
        r#"{{"kind":"nav","how":"push","href":"http://a.test/{}"}}"#,
        "p".repeat(9000)
    );
    assert!(parse_nav_note(&long_href).is_none());
}

// ---------------------------------------------------------------------
// v23: shield_prune IPC parsing (caps, shape, forgery posture)
// ---------------------------------------------------------------------

use pico_core::core::ipc::{looks_like_shield_prune, parse_shield_prune};

#[test]
fn shield_prune_parses_well_formed_reports() {
    let body = r#"{"kind":"shield_prune","items":[{"p":"adPlacements","n":3},{"p":"entries.[-].command.reelWatchEndpoint.adClientParams.isAd","n":12}]}"#;
    assert!(looks_like_shield_prune(body));
    let items = parse_shield_prune(body).expect("well-formed");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].path, "adPlacements");
    assert_eq!(items[0].count, 3);
    assert_eq!(items[1].count, 12);
}

#[test]
fn shield_prune_rejects_other_kinds_and_garbage() {
    assert!(parse_shield_prune(r#"{"kind":"nav","items":[]}"#).is_none());
    assert!(parse_shield_prune("not json").is_none());
    assert!(parse_shield_prune(r#"{"kind":"shield_prune"}"#).is_none());
    assert!(!looks_like_shield_prune(
        r#"{"kind":"nav","how":"init","href":"https://x.example/"}"#
    ));
    // Oversized body: rejected by length before parsing.
    let big = format!(
        "{{\"kind\":\"shield_prune\",\"items\":[{{\"p\":\"{}\",\"n\":1}}]}}",
        "a".repeat(20_000)
    );
    assert!(parse_shield_prune(&big).is_none());
    assert!(!looks_like_shield_prune(&big));
}

#[test]
fn report_pre_filters_match_the_kind_assignment_not_quoted_words() {
    // AUDIT 0.30 (A1-2): a nav note whose page TITLE contains
    // the quoted marker words must route as a nav note, never
    // into a report branch (each handler branch returns
    // unconditionally, so a misroute swallows the note).
    use pico_core::core::ipc::looks_like_shield_pruned;
    let note = r#"{"kind":"nav","how":"push","href":"https://x.example/","title":"all about \"shield_prune\" and \"lite_chosen\" and \"shield_pruned\""}"#;
    assert!(!looks_like_shield_prune(note));
    assert!(!looks_like_lite_chosen(note));
    assert!(!looks_like_shield_pruned(note));
    // The genuine bodies still match.
    assert!(looks_like_shield_prune(
        r#"{"kind":"shield_prune","items":[]}"#
    ));
    assert!(looks_like_lite_chosen(r#"{"kind":"lite_chosen"}"#));
    assert!(looks_like_shield_pruned(
        r#"{"kind":"shield_pruned","bytes":1,"events":1}"#
    ));
}

#[test]
fn shield_prune_caps_items_paths_and_counts() {
    // 17 items: over the cap, whole message refused.
    let items: Vec<String> = (0..17)
        .map(|i| format!("{{\"p\":\"p{i}\",\"n\":1}}"))
        .collect();
    let body = format!(
        "{{\"kind\":\"shield_prune\",\"items\":[{}]}}",
        items.join(",")
    );
    assert!(parse_shield_prune(&body).is_none());
    // Bad items are dropped individually; counts clamp.
    let body = r#"{"kind":"shield_prune","items":[{"p":"","n":1},{"p":"ok.path","n":0},{"p":"ok.path","n":999999999},{"p":"badpath","n":2}]}"#;
    let items = parse_shield_prune(body).expect("parses");
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0].count, 100_000,
        "count clamps to the payload ceiling"
    );
    assert_eq!(items[1].path, "badpath");
    // Control characters in a path: dropped.
    let body = "{\"kind\":\"shield_prune\",\"items\":[{\"p\":\"a\\u0007b\",\"n\":1}]}";
    let items = parse_shield_prune(body).expect("parses");
    assert!(items.is_empty());
}

// ---------------------------------------------------------------------------
// v29: lite_chosen IPC parsing (payload-free, forgery posture)
// ---------------------------------------------------------------------------

use pico_core::core::ipc::{looks_like_lite_chosen, parse_lite_chosen};

#[test]
fn lite_chosen_parses_only_the_exact_report() {
    assert!(looks_like_lite_chosen(r#"{"kind":"lite_chosen"}"#));
    assert!(parse_lite_chosen(r#"{"kind":"lite_chosen"}"#));
    // Any payload at all is a rejection: the report is the fact,
    // nothing rides along (deny_unknown_fields).
    assert!(!parse_lite_chosen(
        r#"{"kind":"lite_chosen","url":"https://evil.example/"}"#
    ));
    assert!(!parse_lite_chosen(r#"{"kind":"lite_chosen","n":1}"#));
    // Other kinds and garbage never parse.
    assert!(!parse_lite_chosen(r#"{"kind":"nav"}"#));
    assert!(!parse_lite_chosen(r#"{"kind":"shield_prune"}"#));
    assert!(!parse_lite_chosen("not json"));
    assert!(!parse_lite_chosen(""));
    assert!(!looks_like_lite_chosen(r#"{"kind":"nav"}"#));
    // Oversized bodies are rejected by both stages.
    let big = format!(
        "{{\"kind\":\"lite_chosen\",\"pad\":\"{}\"}}",
        "x".repeat(70_000)
    );
    assert!(!looks_like_lite_chosen(&big));
    assert!(!parse_lite_chosen(&big));
}

// ---- AUDIT V26 (D5): address-submit routing ----

use pico_core::core::addr::{submit_route, SubmitRoute};

/// The dispatch-layer distinction the brief pins: an address-bar
/// submission on a start tab converts it in place (URL and file
/// targets alike); on a content tab a URL loads in place and a
/// file keeps its open-in-new-tab route. Start-PAGE launcher
/// actions never pass through submit_route at all — they are
/// dispatched by core::action and keep OpenNew semantics there.
#[test]
fn v26_submit_route_matrix() {
    let url = Target::Url("https://example.com/".to_string());
    let search = Target::Url("https://www.google.com/search?q=x".to_string());
    let file = Target::File(std::path::PathBuf::from("/tmp/a.html"));
    assert_eq!(submit_route(true, &url), SubmitRoute::ConvertStart);
    assert_eq!(submit_route(true, &search), SubmitRoute::ConvertStart);
    assert_eq!(submit_route(true, &file), SubmitRoute::ConvertStartWithFile);
    assert_eq!(submit_route(false, &url), SubmitRoute::LoadCurrent);
    assert_eq!(submit_route(false, &file), SubmitRoute::OpenFileInNewTab);
}

// ---- AUDIT V27 (Workstream A): theme picker commands ----

/// The picker speaks three verbs: open, close, and set. The
/// set id is bounded at parse time (the app re-validates it
/// against the palette table before persisting anything).
#[test]
fn v27_theme_commands_parse() {
    let env = parse_command(&cmd_json("palette")).expect("palette parses");
    assert_eq!(env.cmd, Command::Palette);
    let env = parse_command(&cmd_json("palette_close")).expect("palette_close parses");
    assert_eq!(env.cmd, Command::PaletteClose);
    let env = parse_command(r#"{"boot":"b","req":3,"t":"theme_set","id":"ember"}"#)
        .expect("theme_set parses");
    assert_eq!(env.cmd, Command::ThemeSet { id: "ember".into() });
    // A page cannot invent an unbounded id.
    let huge = format!(
        r#"{{"boot":"b","req":4,"t":"theme_set","id":"{}"}}"#,
        "x".repeat(200)
    );
    assert!(
        parse_command(&huge).is_err(),
        "oversized id must be rejected"
    );
    assert!(parse_command(r#"{"boot":"b","req":5,"t":"theme_set"}"#).is_err());
    assert!(
        parse_command(r#"{"boot":"b","req":6,"t":"theme_set","id":""}"#).is_err(),
        "empty id must be rejected"
    );
}

// ---------------------------------------------------------------------------
// AUDIT V28 (A6-5/A6-6/A6-12): the bad-port and bracket matrices
// ---------------------------------------------------------------------------

#[test]
fn v28_bad_ports_are_refused_uniformly() {
    for bad in [
        "127.0.0.1:99999",
        "127.0.0.1:0",
        "[::1]:99999",
        "[::1]:0",
        ":99999",
        ":0",
        "65536",
        "0",
        "00",
        "example.com:99999",
        "localhost:abc",
        "256.256.256.256:8080",
    ] {
        assert_eq!(
            parse_input(bad),
            None,
            "{bad} must be refused, not searched"
        );
    }
    // The valid neighbours of every refused form still navigate.
    assert_eq!(
        parse_input("127.0.0.1:65535"),
        Some(Target::Url("http://127.0.0.1:65535".to_string()))
    );
    assert_eq!(
        parse_input(":8080"),
        Some(Target::Url("http://127.0.0.1:8080/".to_string()))
    );
}

#[test]
fn v28_bracketed_ipv6_suffixes_navigate() {
    for (input, want) in [
        ("[::1]/path", "http://[::1]/path"),
        ("[::1]?q=1", "http://[::1]?q=1"),
        ("[::1]#f", "http://[::1]#f"),
        ("[::1]", "http://[::1]"),
    ] {
        assert_eq!(
            parse_input(input),
            Some(Target::Url(want.to_string())),
            "{input}"
        );
    }
}

#[test]
fn v28_bare_loopback_and_unspecified_go_http() {
    assert_eq!(
        parse_input("127.0.0.1"),
        Some(Target::Url("http://127.0.0.1".to_string()))
    );
    assert_eq!(
        parse_input("127.0.0.1/app"),
        Some(Target::Url("http://127.0.0.1/app".to_string()))
    );
    assert_eq!(
        parse_input("0.0.0.0"),
        Some(Target::Url("http://0.0.0.0".to_string()))
    );
    // The pinned historical half: a bare PRIVATE address is https.
    assert_eq!(
        parse_input("192.168.1.1"),
        Some(Target::Url("https://192.168.1.1".to_string()))
    );
}

#[test]
fn v28_credentialed_urls_typed_pass_pageminted_denied() {
    use pico_core::core::addr::{route_new_window, NewWindowRoute};
    // Typed by the user: preserved verbatim (their own act).
    assert_eq!(
        parse_input("http://user:pass@example.com/"),
        Some(Target::Url("http://user:pass@example.com/".to_string()))
    );
    // Minted by a page via window.open: denied.
    assert!(matches!(
        route_new_window("https://google.com:443@evil.example/"),
        NewWindowRoute::Deny("credentialed-url")
    ));
    assert!(matches!(
        route_new_window("https://example.com/"),
        NewWindowRoute::OpenTab(_)
    ));
}

// ---------------------------------------------------------------------------
// AUDIT 0.31 (A1-F5): parse_command rejects unknown fields;
// (A6-F2): the userinfo navigation predicate's decision table.
// ---------------------------------------------------------------------------

#[test]
fn v31_commands_reject_unknown_fields() {
    // A valid navigate command carrying an extra field is
    // rejected — serde's internally-tagged enum would
    // otherwise ignore the key silently.
    assert!(
        parse_command(r#"{"boot":"b","req":1,"t":"navigate","text":"example.com","tab":9}"#)
            .is_err()
    );
    // A junk field on a field-carrying variant (theme_set).
    assert!(
        parse_command(r#"{"boot":"b","req":1,"t":"theme_set","id":"forest","junk":true}"#).is_err()
    );
    // A junk field on cycle_tab.
    assert!(parse_command(r#"{"boot":"b","req":1,"t":"cycle_tab","dir":1,"text":"x"}"#).is_err());
    // A junk field on a field-less variant.
    assert!(parse_command(r#"{"boot":"b","req":1,"t":"new_tab","tab":9}"#).is_err());
    assert!(parse_command(
        r#"{"boot":"b","req":1,"t":"social","site":"telegram","url":"https://evil.example"}"#
    )
    .is_err());
    // Another variant's field is unknown HERE even though it
    // exists elsewhere in the protocol.
    assert!(parse_command(r#"{"boot":"b","req":1,"t":"close_tab","tab":3,"dir":1}"#).is_err());
    // The clean shapes keep parsing (the whole existing suite
    // pins them too).
    assert!(parse_command(r#"{"boot":"b","req":1,"t":"navigate","text":"example.com"}"#).is_ok());
    assert!(parse_command(r#"{"v":1,"boot":"b","req":1,"t":"theme_set","id":"forest"}"#).is_ok());
    assert!(parse_command(r#"{"boot":"b","req":1,"t":"cycle_tab","dir":-1}"#).is_ok());
    assert!(parse_command(r#"{"boot":"b","req":1,"t":"new_tab"}"#).is_ok());
}

#[test]
fn v31_userinfo_nav_allowed_decision_table() {
    use pico_core::core::ipc::userinfo_nav_allowed;
    let typed = "https://user:pass@example.com/account";
    // No userinfo: not this predicate's concern, always true.
    assert!(userinfo_nav_allowed("https://example.com/", None));
    assert!(userinfo_nav_allowed(
        "https://example.com/",
        Some("https://other.example/")
    ));
    // Credentialed + exact expectation (the typed URL): true.
    assert!(userinfo_nav_allowed(typed, Some(typed)));
    // Credentialed without an expectation: false.
    assert!(!userinfo_nav_allowed(typed, None));
    // Credentialed with a DIFFERENT expectation (a page
    // racing the user's own submission): false.
    assert!(!userinfo_nav_allowed(
        "https://google.com:443@evil.example/",
        Some(typed)
    ));
    // A near-miss expectation (trailing slash) is not a match:
    // the comparison is byte-exact.
    assert!(!userinfo_nav_allowed(
        typed,
        Some("https://user:pass@example.com/account/")
    ));
    // An '@' in the PATH is not userinfo.
    assert!(userinfo_nav_allowed("https://example.com/@user", None));
}

#[test]
fn v28_parse_input_direct_over_cap_prose_searches_without_panic() {
    // A6-14 gap: parse_input called directly (no IPC cap in the
    // way) with a multi-KB prose string produces a search URL —
    // linear work, no panic, no truncation of the query.
    let prose = "word ".repeat(5000);
    match parse_input(&prose) {
        Some(Target::Url(u)) => {
            assert!(u.starts_with("https://www.google.com/search?q="));
            assert!(u.len() > 20_000);
        }
        other => panic!("expected a search URL, got {other:?}"),
    }
}
