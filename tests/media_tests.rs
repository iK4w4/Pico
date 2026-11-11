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
    // the shell sources for the WebView2 API names that could
    // do it — a future edit that introduces one fails here
    // before it can silently break sign-in persistence.
    //
    // 0.32 AMENDMENT (coordinator, on the user's download-manager
    // order): the download capture READS the profile's cookies
    // for the exact download target via the cookie manager's
    // GetCookies — a read-only call that restricts, clears, and
    // rewrites nothing, so the guarantee's intent is untouched.
    // The pin therefore narrows from the manager's NAME to its
    // MUTATION surface: every cookie-mutating method name stays
    // banned in EVERY shell source (app.rs included), and
    // "cookiemanager" itself stays banned everywhere except
    // src/app.rs, where the single GetCookies call site lives.
    let root = env!("CARGO_MANIFEST_DIR");
    for rel in [
        "src/app.rs",
        "src/main.rs",
        "src/downloads.rs",
        "src/winhttp.rs",
    ] {
        let src = std::fs::read_to_string(format!("{root}/{rel}"))
            .unwrap_or_else(|e| panic!("read {rel}: {e}"));
        let lower = src.to_lowercase();
        for banned in [
            "clearbrowsingdata",
            // One substring covers DeleteCookie, DeleteCookies
            // and DeleteCookiesWithDomainAndPath.
            "deletecookie",
            "deleteallcookies",
            "addorupdatecookie",
            "createcookie",
            "copycookie",
            "incognito",
            "clear_all_browsing_data",
        ] {
            assert!(
                !lower.contains(banned),
                "{rel} must never reference {banned} (remember-me pin, AUDIT V22-12)"
            );
        }
        if rel != "src/app.rs" {
            assert!(
                !lower.contains("cookiemanager"),
                "{rel} must never reference cookiemanager (remember-me pin, \
                 AUDIT V22-12; the 0.32 read-only GetCookies exception \
                 lives in src/app.rs only)"
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

#[test]
fn probe_budget_forget_tab_releases_the_entry() {
    // AUDIT 0.32.1 (P3-5): the per-tab probe budget was never
    // forgotten on tab close — the map grew by one entry per
    // tab ever opened, for the whole session.
    let mut budget = MediaProbeBudget::new();
    for _ in 0..MEDIA_PROBE_LOG_LIMIT_PER_TAB {
        assert!(budget.should_log(7));
    }
    assert!(!budget.should_log(7), "cap reached");
    budget.forget_tab(7);
    assert!(
        budget.should_log(7),
        "a forgotten tab's budget starts fresh (tab ids recycle)"
    );
    budget.forget_tab(999); // unknown tab: no panic, no effect
}

#[test]
fn source_scan_download_handler_lives_only_in_the_two_builders() {
    // AUDIT 0.32.2 (Fix 1 pin): download interception has ONE
    // choke point — every content view is built by
    // build_content_view (which routes the attempt to the
    // download manager) and the chrome by build_chrome (which
    // denies). The handler name must occur exactly twice in
    // src/app.rs, once inside each builder's body span, so a
    // future third builder cannot silently bypass (or
    // duplicate) the interception.
    let app = app_source();
    assert_eq!(
        app.matches("with_download_started_handler").count(),
        2,
        "exactly two download-started handlers: content + chrome"
    );
    let content_start = app
        .find("fn build_content_view")
        .expect("build_content_view exists");
    let content_end = app[content_start..]
        .find("\n    fn ")
        .map(|i| content_start + i)
        .expect("a next method follows build_content_view");
    let content_span = &app[content_start..content_end];
    assert_eq!(
        content_span
            .matches("with_download_started_handler")
            .count(),
        1,
        "the content builder registers the handler"
    );
    let chrome_start = app.find("fn build_chrome").expect("build_chrome exists");
    let chrome_end = app[chrome_start..]
        .find("\npub fn ")
        .map(|i| chrome_start + i)
        .expect("a next top-level fn follows build_chrome");
    let chrome_span = &app[chrome_start..chrome_end];
    assert_eq!(
        chrome_span.matches("with_download_started_handler").count(),
        1,
        "the chrome builder registers the (denying) handler"
    );
}

#[test]
fn source_scan_capture_facts_in_on_download_starting_nav_guard_gone() {
    // AUDIT 0.34 (WS1 pin, re-pointed): the 0.33 pin locked
    // the load-start navigation guard (`NavGuard` snapshot +
    // `drop_top_entry` repair). The 0.34 coordinator rebuild
    // deleted that machinery: capture facts are now taken in
    // `on_download_starting` — from the tab's current state at
    // capture — and handed straight to the coordinator's
    // `capture_started`, which decides the tab's fate ONCE
    // from them. This pin locks the new shape: the facts are
    // assembled and handed over in that one handler, and the
    // old guard machinery is gone from the shell entirely.
    let app = app_source();
    let start = app
        .find("fn on_download_starting")
        .expect("on_download_starting exists");
    let end = app[start..]
        .find("fn on_download_cookies")
        .map(|i| start + i)
        .expect("on_download_cookies follows on_download_starting");
    let span = &app[start..end];
    assert!(
        span.contains("CaptureFacts {"),
        "capture facts must be assembled in on_download_starting"
    );
    assert!(
        span.contains("capture_started("),
        "the facts must be handed to the coordinator's capture_started"
    );
    assert!(
        app.contains("capture_started("),
        "the coordinator capture entry point must be called from the shell"
    );
    assert!(
        !app.contains("nav_guard"),
        "the 0.33 navigation guard state must be gone (0.34)"
    );
    assert!(
        !app.contains("NavGuard"),
        "the 0.33 NavGuard type must be gone (0.34)"
    );
}

#[test]
fn source_scan_exec_dl_fate_logs_every_executed_fate() {
    // AUDIT 0.34.1 (field addition pin): `exec_dl_fate` used
    // to execute a landed capture's tab fate with NO log
    // line, so a field log could not say what happened to
    // the capturing tab. Each arm now logs exactly one
    // `download tab fate: …` line at its point of
    // commitment. This pin locks the four exact format
    // strings inside exec_dl_fate's own body span (the
    // restore line's URL placeholder is the redacted value
    // bound just above it).
    let app = app_source();
    let start = app.find("fn exec_dl_fate").expect("exec_dl_fate exists");
    let end = app[start..]
        .find("\n    fn ")
        .map(|i| start + i)
        .expect("a next method follows exec_dl_fate");
    let span = &app[start..end];
    for line in [
        "download tab fate: closed tab {tab_id}",
        "download tab fate: converted tab {tab_id} to start",
        "download tab fate: restored tab {tab_id} to {redacted}",
        "download tab fate: tab {tab_id} back",
    ] {
        assert!(
            span.contains(line),
            "exec_dl_fate must log the fate line: {line}"
        );
    }
    assert!(
        span.contains("redact_url(&url)"),
        "the restore line's URL must pass through the redactor"
    );
}

#[test]
fn source_scan_chrome_dl_body_dedupe_polarity() {
    // AUDIT 0.34.2 (the field failure): `on_chrome_dl_body`
    // guarded its dispatch with `if deduped { return; }`
    // where the variable held `Dedupe::check`'s return —
    // and `check` returns TRUE for a first delivery (the
    // pair was unseen and is now recorded; see
    // core::ipc::Dedupe and the addr_ipc tests). The guard
    // therefore dropped every chrome-surface downloads verb
    // on its normal single delivery (toolbar button,
    // popover rows, Add URL, toast dismiss — all dead in
    // the field) while re-deliveries and stale-boot bodies,
    // for which `check` returns false, were dispatched.
    // The command path (`on_chrome_cmd`) has always used
    // the correct polarity; this pin locks the dl handler
    // to the same shape: drop exactly when the check FAILS,
    // with the drop logged, never silent.
    let app = app_source();
    let start = app
        .find("fn on_chrome_dl_body")
        .expect("on_chrome_dl_body exists");
    let end = app[start..]
        .find("\n    fn ")
        .map(|i| start + i)
        .expect("a next method follows on_chrome_dl_body");
    let span = &app[start..end];
    assert!(
        span.contains("if !self.dedupe.check("),
        "on_chrome_dl_body must drop on a FAILED dedupe check (first delivery dispatches)"
    );
    assert!(
        !span.contains("if deduped"),
        "the inverted 0.34-0.34.1 guard must never return"
    );
    assert!(
        span.contains("downloads command dropped (duplicate or stale boot, req {req})"),
        "a dropped dl body must write its log line, never vanish silently"
    );
}

#[test]
fn source_scan_download_finish_swallow_precedes_commit_finish() {
    // AUDIT 0.33 (WS1 pin), re-pointed 0.34: the swallowed
    // download finish must be consulted BEFORE the
    // CommitFinish application in `on_page_load` — after it,
    // the file URL is already committed into the tab's
    // history and the swallow is meaningless. In 0.33 the
    // check read a per-tab `dl_swallow` marker; in 0.34 the
    // coordinator owns the capture records, so the check is
    // `navigation_finished(` — same position, same law.
    // Ordering is checked by byte offsets, like the decision
    // itself.
    let app = app_source();
    let start = app.find("fn on_page_load").expect("on_page_load exists");
    let end = app[start..]
        .find("fn ensure_watch")
        .map(|i| start + i)
        .expect("ensure_watch follows on_page_load");
    let span = &app[start..end];
    let swallow = span
        .find("navigation_finished(")
        .expect("the finished path must consult the coordinator's navigation_finished");
    let commit_finish = span
        .find("UrlEvent::CommitFinish")
        .expect("on_page_load applies CommitFinish");
    assert!(
        swallow < commit_finish,
        "the navigation_finished consult must precede the CommitFinish application"
    );
    assert!(
        span.contains("download finish swallowed"),
        "the swallow must log its line"
    );
}

#[test]
fn source_scan_adopted_clears_only_on_commit() {
    // AUDIT 0.33 (WS1 pin), re-pointed 0.34: `apply_url_event`
    // must clear `Tab.adopted` ONLY on a HistEffect::Commit —
    // the pre-0.33 unconditional landed-clear let a download
    // file's same-document Push clear it before the capture
    // decided, and the deferred close re-judged wrongly. In
    // 0.34 the same Commit block also bumps the tab's
    // NAVIGATION EPOCH (`nav_epoch += 1`) — the counter the
    // coordinator's landing revalidation compares against
    // the capture epoch (it replaces 0.33's `commits`
    // counter). Both writes must live inside the one
    // HistEffect::Commit outcome block.
    let app = app_source();
    let start = app
        .find("fn apply_url_event")
        .expect("apply_url_event exists");
    let end = app[start..]
        .find("fn on_source_changed")
        .map(|i| start + i)
        .expect("on_source_changed follows apply_url_event");
    let span = &app[start..end];
    let block = span
        .find("if matches!(outcome.hist, HistEffect::Commit)")
        .expect("the adopted clear must be gated on HistEffect::Commit");
    // The Commit outcome block: from the gate to the end of
    // apply_url_event is too wide, so bound the search to
    // the block's own extent (the next outcome consumer,
    // `if let Some(new_url) = outcome.adopted`, follows it).
    let block_end = span[block..]
        .find("if let Some(new_url) = outcome.adopted")
        .map(|i| block + i)
        .expect("the adopted-URL consumer follows the Commit block");
    let commit_block = &span[block..block_end];
    assert!(
        commit_block.contains("adopted = false"),
        "the adopted clear must live in the HistEffect::Commit block"
    );
    assert!(
        commit_block.contains("nav_epoch += 1"),
        "a Commit must bump the tab's navigation epoch inside the same block \
         (the coordinator revalidation's fact)"
    );
    assert!(
        !span.contains("if landed"),
        "the old unconditional landed-clear must be gone"
    );
}

#[test]
fn source_scan_got_focus_registration_lives_in_build_content_view() {
    // AUDIT 0.32.4 (Fix A pin): outside-click panel dismissal
    // rides ONE registration — the GotFocus hook attached in
    // build_content_view beside the download handler. A plain
    // click into the page goes to the CONTENT webview (a
    // separate HWND) and produces no navigation, so the
    // chrome panels can only learn about it from the app
    // layer: the content controller gaining focus. A second
    // registration, or one living anywhere else, breaks the
    // law this pin exists to hold.
    let app = app_source();
    assert_eq!(
        app.matches("add_GotFocus").count(),
        1,
        "exactly one GotFocus registration"
    );
    // The attach fn's name occurs exactly three times: the
    // Windows definition, its #[cfg(not(windows))] stub, and
    // its ONE call in build_content_view.
    assert_eq!(
        app.matches("attach_panel_focus_close").count(),
        3,
        "the attach fn: windows definition + non-windows stub + its one call in build_content_view"
    );
    assert_eq!(
        app.matches("attach_panel_focus_close(&view").count(),
        1,
        "exactly one attach call site"
    );
    let content_start = app
        .find("fn build_content_view")
        .expect("build_content_view exists");
    let content_end = app[content_start..]
        .find("\n    fn ")
        .map(|i| content_start + i)
        .expect("a next method follows build_content_view");
    let content_span = &app[content_start..content_end];
    assert_eq!(
        content_span
            .matches("attach_panel_focus_close(&view")
            .count(),
        1,
        "the attach call lies within build_content_view's body span"
    );
    // The signal's whole path exists: the GotFocus handler
    // sends the event, and its arm is the panel funnel.
    assert!(
        app.contains("UserEvent::ContentFocus"),
        "the ContentFocus user event must exist"
    );
    // `close_all_panels()` (with parens) matches ONLY call
    // sites — the definition is `fn close_all_panels(&mut
    // self)`, which does not contain that pattern. Exactly
    // three call sites: the ContentFocus arm, the window's
    // Focused(false) arm, and (0.34.1) `on_context_menu` —
    // an opening context menu closes every panel first,
    // through the same funnel.
    assert_eq!(
        app.matches("close_all_panels()").count(),
        3,
        "close_all_panels is called from the ContentFocus arm, the Focused(false) arm \
         and on_context_menu"
    );
}

#[test]
fn source_scan_context_menu_registration_lives_in_build_content_view() {
    // AUDIT 0.34.1 (context-menu pin): Pico's own menu rides
    // ONE registration — the ContextMenuRequested hook
    // attached in build_content_view beside the GotFocus
    // hook. That hook is what suppresses WebView2's stock
    // menu (SetHandled first, unconditionally) and forwards
    // the target facts to the event loop; a second
    // registration, or one living anywhere else, breaks the
    // law this pin exists to hold (the stock menu would
    // render for some views, or facts would double-arrive).
    let app = app_source();
    assert_eq!(
        app.matches("add_ContextMenuRequested").count(),
        1,
        "exactly one ContextMenuRequested registration"
    );
    assert_eq!(
        app.matches("attach_context_menu(&view").count(),
        1,
        "exactly one attach call site"
    );
    let content_start = app
        .find("fn build_content_view")
        .expect("build_content_view exists");
    let content_end = app[content_start..]
        .find("\n    fn ")
        .map(|i| content_start + i)
        .expect("a next method follows build_content_view");
    let content_span = &app[content_start..content_end];
    assert_eq!(
        content_span.matches("attach_context_menu(&view").count(),
        1,
        "the attach call lies within build_content_view's body span"
    );
    // The signal's whole path exists: the registration
    // handler forwards the event, and its arm owns the menu.
    assert!(
        app.contains("UserEvent::ContextMenu"),
        "the ContextMenu user event must exist"
    );
}

#[test]
fn source_scan_accelerator_key_registration_lives_in_build_content_view() {
    // 0.35, WS5 (Ctrl+J from content focus): the downloads
    // shortcut rides ONE registration — the
    // AcceleratorKeyPressed hook attached in
    // build_content_view beside the GotFocus hook. Through
    // 0.34 the chord lived only in the chrome document, so
    // it did nothing while the page held focus; a second
    // registration, or one living anywhere else (the chrome
    // builder included), would double-fire or miss views.
    let app = app_source();
    assert_eq!(
        app.matches("add_AcceleratorKeyPressed").count(),
        1,
        "exactly one AcceleratorKeyPressed registration"
    );
    // The attach fn's name occurs exactly three times: the
    // Windows definition, its #[cfg(not(windows))] stub, and
    // its ONE call in build_content_view.
    assert_eq!(
        app.matches("attach_downloads_shortcut").count(),
        3,
        "the attach fn: windows definition + non-windows stub + its one call in build_content_view"
    );
    assert_eq!(
        app.matches("attach_downloads_shortcut(&view").count(),
        1,
        "exactly one attach call site"
    );
    let content_start = app
        .find("fn build_content_view")
        .expect("build_content_view exists");
    let content_end = app[content_start..]
        .find("\n    fn ")
        .map(|i| content_start + i)
        .expect("a next method follows build_content_view");
    let content_span = &app[content_start..content_end];
    assert_eq!(
        content_span
            .matches("attach_downloads_shortcut(&view")
            .count(),
        1,
        "the attach call lies within build_content_view's body span"
    );
    // The signal's whole path exists: the registration
    // handler forwards the event, and its arm opens the
    // downloads page with its log line.
    assert!(
        app.contains("UserEvent::DownloadsPageShortcut"),
        "the DownloadsPageShortcut user event must exist"
    );
    assert!(
        app.contains("downloads page opened (ctrl+j)"),
        "the shortcut arm must log its line"
    );
}

#[test]
fn source_scan_dl_surface_gates_enforced() {
    // AUDIT 0.40.1: the dlcmd per-verb surface table was
    // documentation through 0.35.1 — both transports parsed
    // and dispatched every verb, so a downloads-page context
    // could invoke chrome-only verbs (set-connections,
    // set-default-connections) and a chrome context page-only
    // ones (delete-file, refresh, clear-all). Both handlers
    // are Windows-gated shell code (host-untestable), so the
    // enforcement is pinned here like the dedupe polarity:
    // each handler must consult `spec_for(...).allows(...)`
    // for ITS surface and deny with the surface reason.
    let app = app_source();
    let start = app
        .find("fn on_chrome_dl_body")
        .expect("on_chrome_dl_body exists");
    let end = app[start..]
        .find("\n    fn ")
        .map(|i| start + i)
        .expect("a next method follows on_chrome_dl_body");
    let span = &app[start..end];
    assert!(
        span.contains("DlSurface::Chrome"),
        "on_chrome_dl_body must gate verbs against the Chrome surface"
    );
    assert!(
        span.contains("reason=surface"),
        "a chrome-surface denial must be logged with its reason"
    );
    assert!(
        span.contains("missing chrome envelope"),
        "an envelope-less chrome dl body must be dropped, not dispatched (AUDIT 0.40.1)"
    );
    let start = app
        .find("fn on_downloads_page_body")
        .expect("on_downloads_page_body exists");
    let end = app[start..]
        .find("\n    fn ")
        .map(|i| start + i)
        .expect("a next method follows on_downloads_page_body");
    let span = &app[start..end];
    assert!(
        span.contains("DlSurface::Page"),
        "on_downloads_page_body must gate verbs against the Page surface"
    );
    assert!(
        span.contains("reason=surface"),
        "a page-surface denial must be logged with its reason"
    );
}

#[test]
fn source_scan_ctx_pick_must_be_offered_and_enabled() {
    // AUDIT 0.40.1: the IPC gate checks a ctx pick against
    // the global whitelist only; the EXECUTION must also
    // require the item to have been offered, enabled, by the
    // menu that is actually open — otherwise a pick naming a
    // greyed item (Paste with no clipboard text, Back with
    // no history) still runs.
    let app = app_source();
    let start = app.find("fn on_ctx_pick").expect("on_ctx_pick exists");
    let end = app[start..]
        .find("\n    fn ")
        .map(|i| start + i)
        .expect("a next method follows on_ctx_pick");
    let span = &app[start..end];
    assert!(
        span.contains("state.items.iter().any(|i| i.id == id && i.enabled)"),
        "on_ctx_pick must require the pick to be offered and enabled"
    );
}
