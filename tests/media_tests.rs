//! Tests for the v18 media environment (AUDIT V18-1..V18-3):
//! the user-agent composer, the pinned browser-argument string
//! and its media-safety predicate, the media-probe target /
//! classification / budget logic, the profile-folder derivation
//! pin, and source-scan pins that keep the GUI layer wired the
//! way this file's constants demand.

use pico_core::core::media::{
    args_are_media_safe, classify_media_status, compose_user_agent, probe_target, MediaClass,
    MediaProbeBudget, FALLBACK_BROWSER_VERSION, MEDIA_PROBE_LOG_LIMIT_PER_TAB, PICO_BROWSER_ARGS,
};
use pico_core::core::state::{pico_dir, webview_data_dir};

// ---------------------------------------------------------------------------
// User agent (AUDIT V18-1)
// ---------------------------------------------------------------------------

#[test]
fn ua_is_the_desktop_chrome_form_with_the_runtime_major() {
    // AUDIT V22-14: the v18–v21 Edge-token form
    // (`… Chrome/<v> Safari/537.36 Edg/<v>`) is one of the
    // signals Google sign-in uses to refuse embedded contexts.
    // The composed UA is the standard reduced desktop-Chrome
    // form: runtime major, `.0.0.0` tail, no Edg token, no app
    // token.
    let ua = compose_user_agent(Some("130.0.2849.80"));
    assert_eq!(
        ua,
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
    );
    assert!(!ua.contains("Edg/"), "no Edge token may remain: {ua}");
}

#[test]
fn ua_falls_back_when_the_version_is_missing_or_garbage() {
    for bad in [
        None,
        Some(""),
        Some("   "),
        Some("abc"),
        Some("."),
        Some("1"),
        Some("no.digits.here"),
    ] {
        let ua = compose_user_agent(bad);
        assert!(
            ua.contains("Chrome/140.0.0.0"),
            "input {bad:?} must fall back to the pinned major, got: {ua}"
        );
        assert!(!ua.contains("Edg/"), "fallback must stay Chrome-form: {ua}");
        assert!(
            ua.contains(&format!(
                "Chrome/{}.0.0.0",
                FALLBACK_BROWSER_VERSION.split('.').next().unwrap_or("")
            )),
            "fallback major must derive from FALLBACK_BROWSER_VERSION: {ua}"
        );
    }
}

#[test]
fn ua_never_carries_an_app_token_or_header_shaped_bytes() {
    let ua = compose_user_agent(Some("130.0.1.2\r\nX-Injected: 1"));
    assert!(
        !ua.to_lowercase().contains("pico"),
        "UA must not name the app: {ua}"
    );
    assert!(
        !ua.chars().any(|c| c.is_control()),
        "UA must stay a single header-safe line: {ua:?}"
    );
    assert!(
        !ua.contains("X-Injected"),
        "injection must not survive: {ua}"
    );
}

// ---------------------------------------------------------------------------
// Browser arguments (AUDIT V18-2)
// ---------------------------------------------------------------------------

#[test]
fn the_pinned_args_are_media_safe_and_match_wrys_former_default() {
    assert!(args_are_media_safe(PICO_BROWSER_ARGS));
    // The two halves wry 0.53.5 substituted in v17: the explicit
    // disable list and the autoplay policy from `autoplay = true`.
    assert!(PICO_BROWSER_ARGS
        .contains("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
    assert!(PICO_BROWSER_ARGS.contains("--autoplay-policy=no-user-gesture-required"));
}

#[test]
fn media_hostile_args_are_refused() {
    for hostile in [
        "--disable-gpu",
        "--disable-features=EncryptedMedia",
        "--disable-features=Widevine",
        "--disable-features=msWebOOUI,EncryptedMedia",
        "--enable-features=Something",
        "--disable-quic",
        "--disable-webrtc",
        "--autoplay-policy=document-user-activation-required",
        "--mute-video",
        "--proxy-server=http://127.0.0.1:8080",
        "--disable-features=",
        "--unknown-flag",
    ] {
        assert!(!args_are_media_safe(hostile), "must refuse: {hostile}");
        assert!(
            !args_are_media_safe(&format!("{PICO_BROWSER_ARGS} {hostile}")),
            "must refuse when appended: {hostile}"
        );
    }
    assert!(args_are_media_safe(""), "an empty string sets nothing");
}

// ---------------------------------------------------------------------------
// Media probe (AUDIT V18-3)
// ---------------------------------------------------------------------------

#[test]
fn probe_targets_googlevideo_hosts_and_videoplayback_paths() {
    let t = probe_target("https://rr2---sn-a5mekn6r.googlevideo.com/videoplayback?expire=1&sig=x")
        .expect("googlevideo media URL is a target");
    assert_eq!(t.host, "rr2---sn-a5mekn6r.googlevideo.com");

    let t = probe_target("https://www.youtube.com/videoplayback?x=1")
        .expect("/videoplayback on any host is a target");
    assert_eq!(t.host, "www.youtube.com");

    // Case and ports do not hide a target.
    let t = probe_target("https://GOOGLEVIDEO.COM:443/videoplayback")
        .expect("uppercase host is a target");
    assert_eq!(t.host, "googlevideo.com");

    // Userinfo tricks resolve to the real host.
    let t = probe_target("https://googlevideo.com@evil.example/videoplayback")
        .expect("path still makes it a target");
    assert_eq!(t.host, "evil.example");
}

#[test]
fn probe_ignores_non_media_traffic() {
    for uri in [
        "https://www.youtube.com/watch?v=abc",
        "https://i.ytimg.com/vi/abc/hqdefault.jpg",
        "https://www.youtube.com/api/stats/playback?ns=yt",
        "https://evil.example/?next=googlevideo.com",
        "https://evil.example/?next=/videoplayback",
        "https://googlevideo.com.evil.example/watch",
        "not a url",
        "",
    ] {
        assert!(probe_target(uri).is_none(), "must not probe: {uri}");
    }
}

#[test]
fn media_statuses_classify_into_the_log_vocabulary() {
    assert_eq!(classify_media_status(Some(200)), MediaClass::StreamServed);
    assert_eq!(classify_media_status(Some(206)), MediaClass::StreamServed);
    assert_eq!(classify_media_status(Some(403)), MediaClass::ServerRefusal);
    assert_eq!(classify_media_status(Some(401)), MediaClass::ServerRefusal);
    assert_eq!(classify_media_status(None), MediaClass::TransportFailure);
    assert_eq!(classify_media_status(Some(0)), MediaClass::TransportFailure);
    assert_eq!(
        classify_media_status(Some(404)),
        MediaClass::Unexpected(404)
    );
    assert_eq!(
        classify_media_status(Some(302)),
        MediaClass::Unexpected(302)
    );
    assert_eq!(MediaClass::ServerRefusal.label(), "server-refusal");
}

#[test]
fn probe_budget_caps_per_tab_and_is_per_tab_independent() {
    let mut budget = MediaProbeBudget::new();
    for _ in 0..MEDIA_PROBE_LOG_LIMIT_PER_TAB {
        assert!(budget.should_log(7), "within the cap must log");
    }
    assert!(!budget.should_log(7), "past the cap must be dropped");
    assert!(!budget.should_log(7), "and stays dropped");
    assert!(budget.should_log(8), "another tab has its own budget");
}

// ---------------------------------------------------------------------------
// Profile folder (AUDIT V18-3)
// ---------------------------------------------------------------------------

#[test]
fn webview_data_dir_is_the_fixed_profile_folder() {
    let dir = webview_data_dir();
    assert_eq!(dir, pico_dir().join("WebView2"));
    assert_eq!(
        dir.file_name().map(|n| n.to_string_lossy().into_owned()),
        Some("WebView2".to_string())
    );
    assert_eq!(dir.parent(), Some(pico_dir().as_path()));
}

// ---------------------------------------------------------------------------
// Source-scan pins: the GUI layer must stay wired to these constants
// ---------------------------------------------------------------------------

fn app_source() -> String {
    let path = format!("{}/src/app.rs", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).expect("read src/app.rs")
}

#[test]
fn source_scan_browser_args_are_pinned_everywhere_and_never_hostile() {
    let app = app_source();
    // Exactly two call sites — chrome and content — both naming
    // the pinned constant (one user-data folder ⇒ one options set;
    // environments over a folder must agree, per wry's warning).
    let sites: Vec<&str> = app.matches("with_additional_browser_args(").collect();
    assert_eq!(sites.len(), 2, "args must be pinned on both builders");
    let named: Vec<&str> = app
        .matches("with_additional_browser_args(pico_core::core::media::PICO_BROWSER_ARGS)")
        .collect();
    assert_eq!(named.len(), 2, "both call sites must use the constant");
    // And no media-hostile flag text anywhere in the shell.
    for hostile in [
        "--disable-gpu",
        "--disable-quic",
        "--disable-webrtc",
        "EncryptedMedia",
        "--autoplay-policy=document",
        "--mute-video",
    ] {
        assert!(!app.contains(hostile), "app.rs must never carry: {hostile}");
    }
}

#[test]
fn source_scan_user_agent_and_probe_are_wired() {
    let app = app_source();
    assert!(
        app.contains(".with_user_agent(self.user_agent.clone())"),
        "content views must set the composed UA"
    );
    assert!(
        app.contains("compose_user_agent(runtime_version.as_deref())"),
        "the UA must be composed from the runtime version at startup"
    );
    assert!(
        app.contains("webview2 runtime version:"),
        "the runtime version must be logged at startup"
    );
    assert!(
        app.contains("add_WebResourceResponseReceived"),
        "the media probe must be registered"
    );
    assert!(
        app.contains(
            "attach_media_probe(&view, id, self.probe_budget.clone(), self.stats.clone())"
        ),
        "every content view must get the probe (and the 0.30 ledger observer)"
    );
    assert!(
        app.contains("pstore::webview_data_dir()"),
        "the profile folder must come from the core derivation"
    );
    assert!(
        !app.contains("join(\"WebView2\")"),
        "no second, unpinned derivation of the profile folder"
    );
}

// ---------------------------------------------------------------------------
// Remember-me pins (AUDIT V22-12/V22-16)
// ---------------------------------------------------------------------------

#[test]
fn source_scan_no_cookie_or_storage_restriction_apis_anywhere_in_the_shell() {
    // The remember-me guarantee is an ABSENCE: no code path may
    // restrict, clear, or rewrite cookies/storage/cache. Scan
    // both shell sources for the WebView2 API names that could
    // do it — a future edit that introduces one fails here
    // before it can silently break sign-in persistence.
    let root = env!("CARGO_MANIFEST_DIR");
    for rel in ["src/app.rs", "src/main.rs"] {
        let src = std::fs::read_to_string(format!("{root}/{rel}"))
            .unwrap_or_else(|e| panic!("read {rel}: {e}"));
        let lower = src.to_lowercase();
        for banned in [
            "cookiemanager",
            "clearbrowsingdata",
            "deletecookies",
            "addorupdatecookie",
            "incognito",
            "clear_all_browsing_data",
        ] {
            assert!(
                !lower.contains(banned),
                "{rel} must never reference {banned} (remember-me pin, AUDIT V22-12)"
            );
        }
    }
}

#[test]
fn profile_path_is_stable_across_simulated_launches() {
    // A relaunch must land on the SAME profile folder, or
    // cookies (and every sign-in) would strand in the old one.
    // The derivation is a pure function of the environment bases
    // — evaluate it repeatedly, as consecutive launches would.
    let first = webview_data_dir();
    for _ in 0..8 {
        assert_eq!(webview_data_dir(), first, "profile path must not drift");
        assert_eq!(
            pico_dir(),
            first.parent().expect("profile has a parent").to_path_buf()
        );
    }
}

#[test]
fn cookies_db_witness_recognizes_both_webview2_layouts() {
    use pico_core::core::media::cookies_db_present;
    let base = std::env::temp_dir().join(format!("pico-v22-cookies-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("create temp profile");
    assert!(
        !cookies_db_present(&base),
        "empty profile has no cookies db"
    );

    // Modern layout: Default/Network/Cookies.
    let net = base.join("Default").join("Network");
    std::fs::create_dir_all(&net).expect("create Network dir");
    std::fs::write(net.join("Cookies"), b"").expect("write Cookies file");
    assert!(
        cookies_db_present(&base),
        "Default/Network/Cookies must be seen"
    );

    // Legacy layout: Default/Cookies.
    let base2 = base.join("legacy");
    std::fs::create_dir_all(base2.join("Default")).expect("create Default dir");
    std::fs::write(base2.join("Default").join("Cookies"), b"").expect("write legacy Cookies");
    assert!(cookies_db_present(&base2), "Default/Cookies must be seen");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn startup_logs_the_ua_form_and_the_profile_witness() {
    // Source pins for the two v22 startup lines pico.log must be
    // able to answer sign-in questions with.
    let app = app_source();
    assert!(
        app.contains("content user-agent ("),
        "the UA startup line must name its form (AUDIT V22-14)"
    );
    assert!(
        app.contains("UA_FORM"),
        "the form label must come from core::media::UA_FORM"
    );
    assert!(
        app.contains("profile: persistent, cookies db present="),
        "the remember-me witness line must be logged at startup (AUDIT V22-16)"
    );
    assert!(
        app.contains("cookies_db_present(&data_dir)"),
        "the witness must check the pinned profile folder"
    );
}
