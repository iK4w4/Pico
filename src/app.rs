//! Pico v26 GUI shell.
//!
//! One borderless tao window. The browser chrome (merged brand/tab
//! row, address bar, feather) is an HTML/CSS/JS page in a child
//! WebView docked at the top; every tab is its own child WebView
//! below it, created once and shown/hidden on tab switches (never
//! recreated). The content rect starts 12 DIP above the chrome's
//! bottom edge, underneath the chrome's transparent feather strip
//! (core::layout) — the header visually melts into the page.
//!
//! - **Shell resilience (AUDIT V14-1..4):** v13 shipped a chrome that
//!   could fail *silently* — its deny-all navigation handler also
//!   cancelled the chrome document's own initial load, leaving a
//!   black, empty WebView on screen with no error anywhere. v14's
//!   rules: every surface has an explicit background color; the
//!   chrome navigation handler allows exactly the chrome document;
//!   the chrome's `ready` handshake is recorded, and a watchdog
//!   recreates the chrome once if the handshake never arrives, then
//!   escalates to a native dialog; the window stays hidden until the
//!   shell and first tab exist. The chrome can fail loudly, never
//!   silently.
//! - **v16 transparency exception (AUDIT V16-2):** the *chrome*
//!   WebView's controller background is transparent — the one
//!   deliberate exception to the v14 explicit-background rule, made
//!   so the feather can blend the header into the page. It is safe
//!   because the tao window behind the chrome always carries the
//!   themed background color: a chrome document that fails to paint
//!   shows the theme color through the transparency, never the
//!   platform's unpainted black — and every other v14 mitigation
//!   (handshake, watchdog, warn strip, hidden-until-ready) is
//!   unchanged. Content WebViews keep their explicit themed
//!   backgrounds.
//!
//! State ownership:
//! - **Rust owns all UI state.** The chrome page is a renderer: it
//!   sends typed commands (`pico_core::core::ipc`) and draws the full
//!   snapshots Rust pushes back. Pushes are signature-gated, so an
//!   unchanged state costs zero IPC.
//! - Chrome commands carry `(boot, req)` and are deduped — a
//!   double-fired click can never act twice.
//! - Per-tab session history is the unit-tested
//!   `pico_core::core::history::TabHistory` state machine, reconciled
//!   from native page-load commits *and* the injected navigation
//!   observer (SPA pushState/replaceState/popstate).
//! - **Start-page actions (AUDIT V17-1):** every start-page gesture
//!   (recent file, file picker, service card, social chip) is
//!   decided by the single pure dispatch in `core::action` and
//!   executed in `App::on_start_action`, which logs each decision.
//!   Clicks travel by `window.ipc` (the transport chrome commands
//!   already prove), with the legacy `pico://…` anchor navigation
//!   kept only as a fallback that lands in the same dispatch. The
//!   v16 chain — anchor navigations to an unregistered scheme,
//!   decided by string matching split across two handlers — died
//!   silently on the user's machine; see AUDIT.md V17-1/V17-2.
//! - **Media environment (AUDIT V18-1..3, V22-14):** the content
//!   user agent and browser arguments are pinned in `core::media`,
//!   set on *every* builder (all WebViews share one user-data
//!   folder, and environments over one folder must agree on
//!   options); startup logs the runtime version, the effective UA
//!   and its form, and the profile folder; a passive
//!   `WebResourceResponseReceived` probe classifies media-stream
//!   responses (host + status only) in pico.log. v22 composes the
//!   UA in the standard desktop **Chrome** form: the v18–v21
//!   Edge-token form is one of the signals Google's sign-in uses
//!   to refuse embedded contexts ("browser or app may not be
//!   secure"); see AUDIT V22-14.
//! - **Video & login (AUDIT v22 Phase 2):** anonymous YouTube
//!   playback is refused server-side on the user's egress — his
//!   own pico.log shows googlevideo answering 403
//!   `server-refusal` on main formats while other streams serve
//!   200. That is YouTube's trust decision and Pico does not
//!   circumvent it. What Pico controls is the stated fallback —
//!   sign-in must work and persist: content new-window requests
//!   are routed by the pure `core::addr::route_new_window`
//!   (allowlisted http/https → a new tab, everything else denied
//!   and logged, one pico.log line per decision); the profile is
//!   the pinned persistent folder, witnessed at startup by the
//!   `profile: persistent, cookies db present=<bool>` line.
//! - **Root lifecycle (AUDIT M2):** a tab that opens a local folder
//!   *holds* one reference on its server root; the root (and its
//!   file watcher) is released when the last holding tab leaves or
//!   closes. Tabs that merely navigate to a served URL borrow the
//!   root for reload scoping without holding a reference.
//! - **Per-tab failure (AUDIT M6):** a content WebView that fails to
//!   create marks only its own tab failed (the reason rides the
//!   snapshot; Reload retries creation). Only chrome/WebView2-at-
//!   boot failures are fatal.
//! - Background work (localhost detector, file watcher, Shield
//!   compile) reaches the UI thread only through `EventLoopProxy`
//!   user events.
//! - Diagnostics go to `<data dir>\pico.log` (v27: `data\` beside the exe); URLs are logged
//!   through `redact_url` (AUDIT M5) — queries/fragments never land
//!   in the log.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use tao::dpi::LogicalSize as TaoLogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::http::{Request, Response};
use wry::{DragDropEvent, NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder, RGBA};

use pico_core::core::action::{
    dispatch as dispatch_action, kind_label, looks_like_action, ActionSource, Decision, Requester,
};
use pico_core::core::addr::{
    is_allowed_navigation_url, normalize_pico_url, parse_input, route_new_window, submit_route,
    web_origin, NewWindowRoute, SubmitRoute, Target,
};
use pico_core::core::assets;
use pico_core::core::detect::{signature, Listener, ScanOutcome, Service, TitleCache};
use pico_core::core::fileopen::{resolve_open_target, OpenResolveError};
use pico_core::core::history::TabHistory;
use pico_core::core::ipc::{
    looks_like_lite_chosen, looks_like_shield_prune, looks_like_shield_pruned, parse_command,
    parse_lite_chosen, parse_nav_note, parse_shield_prune, parse_shield_pruned,
    userinfo_nav_allowed, Command, Dedupe, NavNote, PruneItem, SocialSite, NAV_OBSERVER_SCRIPT,
};
use pico_core::core::layout;
use pico_core::core::lite::LiteRuntime;
use pico_core::core::media::MediaProbeBudget;
use pico_core::core::redact::redact_url;
use pico_core::core::server::FileServer;
use pico_core::core::shield::ShieldRuntime;
use pico_core::core::state::{self as pstore, LoadOutcome, PersistState, WindowRect};
use pico_core::core::urlstate::{HistEffect, UrlEvent, UrlState};

use crate::log;

/// How long the chrome gets to deliver its `ready` handshake before
/// the watchdog steps in (AUDIT V14-3). Generous against a cold
/// WebView2 start on a slow disk; short enough that a dead shell is
/// surfaced while the user is still looking at the window.
const CHROME_WATCHDOG_DELAY: Duration = Duration::from_millis(2500);

/// Theme surface colors as RGBA, matching `--bg` in
/// assets/style.css. Every content WebView and the window itself get
/// one of these as an explicit background (AUDIT V14-2), so a surface
/// that has not painted yet — or failed to — shows the theme color,
/// never unpainted black. The chrome WebView is the single exception
/// (v16): its controller background is transparent and the window's
/// themed color is what shows through it.
fn theme_bg_rgba(palette_id: &str, mode: &str) -> RGBA {
    // AUDIT V27 (Workstream A): the surface of the ACTIVE
    // palette + mode, read from the one derivation in
    // core::themes — the same value the pages' --surface
    // carries, so the feather melt and any unpainted pixel are
    // seamless with the material in all sixteen themes. (Through
    // v26 this was a hard-coded violet pair.)
    pico_core::core::themes::surface_rgba(palette_id, mode == "dark")
}
/// Hard cap on simultaneous tabs: a page abusing `window.open` must
/// not be able to exhaust memory with WebView instances.
const MAX_TABS: usize = 30;
/// Hard cap on simultaneously watched roots (live reload).
const MAX_WATCHED_ROOTS: usize = 32;
const START_URL: &str = "pico://start";
const CHROME_URL: &str = "pico://chrome";
/// The display version (0.30) lives in core, pinned against
/// the Cargo version by a core unit test; the shell only ever
/// shows this form (badge, banner, snapshot).
const DISPLAY_VERSION: &str = pico_core::core::DISPLAY_VERSION;

/// AUDIT V29 (A5-M3): a persistent state-save failure (a full
/// or read-only data dir) used to log one error line per save
/// — and saves ride title-bearing NavNotes, so a long YouTube
/// session could grow pico.log by thousands of identical
/// lines. The failure itself is already user-visible (state
/// simply does not persist); the log now says so ONCE per
/// session and stays quiet after.
static SAVE_FAIL_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn log_save_failure(e: &std::io::Error) {
    if !SAVE_FAIL_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log::error(&format!(
            "state save failed: {e} (further failures this session are not logged)"
        ));
    }
}

// ---------------------------------------------------------------------------
// Embedded pages (pico:// protocol)
// ---------------------------------------------------------------------------

/// State shared with the protocol handlers (start page initial render).
#[derive(Default)]
struct SharedData {
    services: Vec<Service>,
    /// Listeners that were seen but did not qualify as dev servers
    /// (AUDIT V14-5): rendered as one aggregate line, never as cards.
    other_listeners: usize,
    recents: Vec<String>,
    /// v24: the start page's Recents section starts collapsed.
    recents_hidden: bool,
    theme: String,
    /// v27: the active palette id, mirrored for start renders.
    palette: String,
    /// 0.30: the savings band's figures, mirrored for start
    /// renders (live updates ride `__picoSetStats`).
    stats: pico_core::core::stats::StatsView,
}
type Shared = Arc<Mutex<SharedData>>;

/// Page assembly lives in the core library (AUDIT V14-1) so the exact
/// bytes served here are covered by asset-integrity tests.
fn render_chrome() -> String {
    assets::assemble_chrome_html()
}

fn render_start(shared: &Shared) -> String {
    let data = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let init = serde_json::json!({
        "services": data.services,
        "otherListeners": data.other_listeners,
        "recents": data.recents,
        "recentsHidden": data.recents_hidden,
        "theme": data.theme,
        "palette": data.palette,
        "stats": data.stats,
    });
    let init_json = serde_json::to_string(&init).unwrap_or_else(|_| "null".to_string());
    assets::assemble_start_html(&init_json)
}

/// Handler for the `pico://` custom protocol, registered on every
/// webview. Serves only two documents: the chrome and the start page.
/// (AUDIT L11: it intentionally does not distinguish webviews — the
/// documents carry no secrets, and content navigation to them is
/// denied by the navigation handler.)
fn pico_protocol_handler(
    shared: Shared,
) -> impl Fn(wry::WebViewId<'_>, Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> + 'static {
    move |_id, request| {
        let uri = normalize_pico_url(&request.uri().to_string());
        let route = uri.split(['?', '#']).next().unwrap_or("");
        let route = route.strip_suffix('/').unwrap_or(route);
        let (status, body): (u16, String) = if route == CHROME_URL {
            (200, render_chrome())
        } else if route == START_URL {
            (200, render_start(&shared))
        } else {
            (404, "Pico: unknown internal page".to_string())
        };
        match Response::builder()
            .status(status)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Cow::Owned(body.into_bytes()))
        {
            Ok(resp) => resp,
            Err(_) => Response::new(Cow::Borrowed(b"Pico: internal protocol error".as_slice())),
        }
    }
}

// (The `pico://` ⇄ `https://pico.` fold lives in
// `pico_core::core::addr::normalize_pico_url` — AUDIT V15-2: it now
// folds only Pico's own internal hosts, so real `pico.*` websites
// stay browsable.)
fn is_action_url(u: &str) -> bool {
    // 0.30: delegate to the core classifier. The hand-kept
    // prefix list here had silently fallen behind the
    // dispatch: it never learned the v24 recents-management
    // families (or 0.30's stats-reset), so those gestures'
    // legacy anchor transport dead-ended in the navigation
    // handler instead of reaching the one dispatch. The core
    // classifier is the same one the new-window router uses;
    // the URL here is already normalized, and the core
    // function normalizes idempotently.
    pico_core::core::action::is_action_url(u)
}

fn is_web_url(u: &str) -> bool {
    web_origin(u).is_some()
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Disable WebView2 profile features a local-development browser has
/// no business warehousing: password autosave and general autofill.
/// Reached through the raw WebView2 profile interface (wry exposes
/// the `ICoreWebView2`, but does not surface these two settings).
/// Failures are logged, never fatal: an older runtime may not know
/// the newer profile interface.
///
/// These are the ONLY two profile settings Pico touches (AUDIT
/// V18-1 inventory, re-verified V22-12): nothing here — or anywhere
/// in the tree — restricts or clears cookies, storage, or cache,
/// which is what lets sign-ins persist across restarts.
#[cfg(windows)]
fn harden_webview_profile(view: &WebView, label: &str) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile6, ICoreWebView2_13};
    use windows_core::Interface;
    use wry::WebViewExtWindows;

    let outcome = (|| -> Result<(), String> {
        let core = view
            .webview()
            .cast::<ICoreWebView2_13>()
            .map_err(|e| format!("cast to ICoreWebView2_13: {e}"))?;
        // SAFETY: `Profile()` on the live core returns the
        // profile object for this view's environment; the call
        // is a documented getter with no preconditions beyond a
        // live core, which `view` guarantees.
        let profile = unsafe { core.Profile() }.map_err(|e| format!("Profile(): {e}"))?;
        let profile6 = profile
            .cast::<ICoreWebView2Profile6>()
            .map_err(|e| format!("cast to ICoreWebView2Profile6: {e}"))?;
        // SAFETY: both setters are documented profile methods
        // taking a plain boolean; the profile6 interface was
        // just obtained from the live core above.
        unsafe {
            profile6
                .SetIsPasswordAutosaveEnabled(false)
                .map_err(|e| format!("SetIsPasswordAutosaveEnabled: {e}"))?;
            profile6
                .SetIsGeneralAutofillEnabled(false)
                .map_err(|e| format!("SetIsGeneralAutofillEnabled: {e}"))?;
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => log::info(&format!(
            "webview profile hardened ({label}): password autosave + autofill disabled"
        )),
        Err(why) => log::info(&format!(
            "webview profile hardening unavailable for {label}: {why}"
        )),
    }
}

#[cfg(not(windows))]
fn harden_webview_profile(_view: &WebView, _label: &str) {}

/// The installed WebView2 runtime version (e.g. "141.0.3485.0"),
/// queried from the OS before any environment exists — wry wraps
/// `GetAvailableCoreWebView2BrowserVersionString`. Logged at
/// startup and used to compose the content user agent (AUDIT
/// V18-1/V18-3): codec and EME questions are runtime questions,
/// and pico.log must be able to answer "which runtime was this?"
#[cfg(windows)]
fn webview_runtime_version() -> Option<String> {
    wry::webview_version().ok().filter(|v| !v.trim().is_empty())
}

#[cfg(not(windows))]
fn webview_runtime_version() -> Option<String> {
    None
}

/// The process-wide monotonic clock for the savings ledger
/// (0.30): one epoch, millisecond resolution, shared by the
/// request handler (which notes dispatch times) and the
/// response observer (which completes them). Only DIFFERENCES
/// are ever read, and nothing here is persisted or logged.
fn stats_now_ms() -> u64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Register the response observer on one content WebView: one
/// `WebResourceResponseReceived` handler carrying two pure
/// observation duties.
///
/// 1. The media probe (AUDIT V18-3): for media-stream requests
///    only (`core::media::probe_target`: googlevideo.com
///    hosts, or a `/videoplayback` path anywhere), log the
///    host + HTTP status + classification. Never the URL
///    (signed media URLs carry access tokens in the query),
///    and at most `MEDIA_PROBE_LOG_LIMIT_PER_TAB` lines per
///    tab per session.
/// 2. The savings ledger (0.30): for PUBLIC responses only,
///    feed `core::stats` one sample — the `Content-Length`
///    header value (raw; the core parser decides what is
///    usable) and the `Content-Type`, keyed by the request
///    object's identity token so the core can correlate the
///    response with the dispatch time the request handler
///    noted. The token is the request interface's raw pointer
///    — WebView2 hands the SAME request object to both
///    events; when correlation misses (the object differs, or
///    the entry expired), the core still calibrates bytes by
///    Content-Type and simply records no duration.
///
/// The handler is pure observation of responses the engine
/// has already received — it runs after the fact and cannot
/// delay, alter, or block any request. Pico's own neutered
/// responses cannot poison the ledger: they are delivered
/// for requests the handler never noted (blocked requests are
/// not noted), and they carry no `Content-Length` header at
/// all. Failures to register are logged, never fatal (an
/// older runtime may lack the event; the browser must not die
/// for a statistic).
#[cfg(windows)]
fn attach_media_probe(
    view: &WebView,
    tab: u64,
    budget: Arc<Mutex<MediaProbeBudget>>,
    stats: Arc<pico_core::core::stats::StatsBook>,
) {
    use pico_core::core::media::{classify_media_status, probe_target};
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use webview2_com::WebResourceResponseReceivedEventHandler;
    use windows_core::Interface;
    use wry::WebViewExtWindows;

    let outcome = (|| -> Result<(), String> {
        // The response-received event hangs off ICoreWebView2_2
        // (not the base interface) in the WebView2 API.
        let core = view
            .webview()
            .cast::<ICoreWebView2_2>()
            .map_err(|e| format!("cast to ICoreWebView2_2: {e}"))?;
        let handler =
            WebResourceResponseReceivedEventHandler::create(Box::new(move |_webview, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                // SAFETY: the event args and the request/response
                // views they hand out are valid for the duration of
                // this synchronous callback; every call below is a
                // documented getter on those objects, and the PWSTR
                // returned by `Uri` is CoTaskMem-allocated (freed by
                // `webview2_com::take_pwstr`).
                unsafe {
                    let request = args.Request()?;
                    // The request object's identity token for
                    // the ledger's duration correlation (0.30):
                    // the raw interface pointer, whose pointee
                    // the event args keep alive for this call.
                    let req_token = windows_core::Interface::as_raw(&request) as usize as u64;
                    let mut uri_ptr = windows_core::PWSTR::null();
                    request.Uri(&mut uri_ptr)?;
                    let uri = webview2_com::take_pwstr(uri_ptr);
                    if let Some(target) = probe_target(&uri) {
                        let response = args.Response()?;
                        let mut code: i32 = 0;
                        response.StatusCode(&mut code)?;
                        let allowed = budget
                            .lock()
                            .map(|mut b| b.should_log(tab))
                            .unwrap_or(false);
                        if allowed {
                            crate::log::info(&format!(
                                "media probe: tab={tab} host={} status={code} class={}",
                                target.host,
                                classify_media_status(Some(code)).label()
                            ));
                        }
                    }
                    // The ledger sample (0.30). Public responses
                    // only — the same origin law the pipeline
                    // enforces for blocks (core::stats).
                    if pico_core::core::stats::is_samplable(
                        pico_core::core::filterpipe::classify_origin(&uri),
                    ) {
                        if let Ok(response) = args.Response() {
                            let read_header = |name: &str| -> Option<String> {
                                let headers = response.Headers().ok()?;
                                let wide: Vec<u16> =
                                    name.encode_utf16().chain(std::iter::once(0)).collect();
                                let mut ptr = windows_core::PWSTR::null();
                                headers
                                    .GetHeader(windows_core::PCWSTR(wide.as_ptr()), &mut ptr)
                                    .ok()?;
                                if ptr.is_null() {
                                    return None;
                                }
                                Some(webview2_com::take_pwstr(ptr))
                            };
                            let content_length = pico_core::core::stats::parse_content_length(
                                read_header("Content-Length").as_deref(),
                            );
                            let content_type = read_header("Content-Type");
                            stats.note_response(
                                req_token,
                                content_length,
                                content_type.as_deref(),
                                stats_now_ms(),
                            );
                        }
                    }
                }
                Ok(())
            }));
        let mut token: i64 = 0;
        unsafe {
            core.add_WebResourceResponseReceived(&handler, &mut token)
                .map_err(|e| format!("add_WebResourceResponseReceived: {e}"))?;
        }
        Ok(())
    })();
    if let Err(why) = outcome {
        log::info(&format!("media probe unavailable for tab {tab}: {why}"));
    }
}

#[cfg(not(windows))]
fn attach_media_probe(
    _view: &WebView,
    _tab: u64,
    _budget: Arc<Mutex<MediaProbeBudget>>,
    _stats: Arc<pico_core::core::stats::StatsBook>,
) {
}

/// Byte ceiling for the Shield's injected cosmetic payload
/// (generic selectors + the highest-volume host-specific sets);
/// the compile thread builds the script against this budget.
/// Measured on the shipped lists (AUDIT V21): the generic set
/// alone serializes to ~237 KB, so the generic-first policy
/// needs the ceiling here to also carry the top host sets.
/// v23: raised from 280,000 — with the flag-host JSON honestly
/// counted (AUDIT V23-3), the full 358-host payload measures
/// ~288 KB, and the ceiling governs a governor's approximation,
/// not a hard cap.
/// (AUDIT V26: the value lives in core::shield since v26 —
/// one definition for app, example and tests.)
const COSMETIC_SCRIPT_BUDGET: usize = pico_core::core::shield::COSMETIC_SCRIPT_BUDGET;

/// Byte ceiling for the scriptlet payload (AUDIT V23): the full
/// compiled set on the shipped lists — 2,913 entries on 2,197
/// hosts — serializes to ~187 KB, so this ceiling ships every
/// host with headroom; if the lists grow, the builder's
/// priority order (YouTube family first, then entry volume)
/// decides what fits.
const SCRIPTLET_SCRIPT_BUDGET: usize = pico_core::core::shield::SCRIPTLET_SCRIPT_BUDGET;

/// Build the neutered response for a blocked request (AUDIT V21).
/// Empty bodies go through the typed API with a null stream.
/// The one non-empty body (the 1×1 GIF) needs an `IStream` the
/// typed signature names from the `windows` crate — not a direct
/// dependency of Pico, and not nameable from here — so that one
/// call goes through the environment's vtable directly with the
/// SHCreateMemStream pointer as the ABI value, exactly as the
/// generated wrapper does with `content.param().abi()`.
#[cfg(windows)]
fn make_neutered_response(
    env: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
    rtype: pico_core::core::shield::ResourceType,
) -> Result<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceResponse, String>
{
    use pico_core::core::shield::neutered_body;
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceResponse;
    use windows_core::Interface;

    let (body, ctype) = neutered_body(rtype);
    let headers = format!("Content-Type: {ctype}\r\nCache-Control: no-store\r\n");
    let headers_wide: Vec<u16> = headers.encode_utf16().chain(std::iter::once(0)).collect();
    let headers_pcw = windows_core::PCWSTR(headers_wide.as_ptr());
    // SAFETY: every pointer below is valid for the duration of
    // the call: `env` is the view's live environment, the header
    // buffer outlives the call, the stream (when created) is a
    // valid COM object whose reference we own until the call
    // returns, and the out cell is a plain local.
    unsafe {
        match body {
            Some(bytes) if !bytes.is_empty() => {
                let raw = windows_sys::Win32::UI::Shell::SHCreateMemStream(
                    bytes.as_ptr(),
                    bytes.len() as u32,
                );
                if raw.is_null() {
                    return Err("SHCreateMemStream failed".to_string());
                }
                // We own the stream's initial reference; the
                // environment AddRefs what it keeps.
                let stream_owner = windows_core::IUnknown::from_raw(raw);
                let mut out: *mut core::ffi::c_void = std::ptr::null_mut();
                let hr = (windows_core::Interface::vtable(env).CreateWebResourceResponse)(
                    windows_core::Interface::as_raw(env),
                    stream_owner.as_raw(),
                    200,
                    windows_core::w!("OK"),
                    headers_pcw,
                    &mut out,
                );
                drop(stream_owner);
                hr.ok()
                    .map_err(|e| format!("CreateWebResourceResponse (stream): {e}"))?;
                if out.is_null() {
                    return Err("CreateWebResourceResponse returned null".to_string());
                }
                windows_core::IUnknown::from_raw(out)
                    .cast::<ICoreWebView2WebResourceResponse>()
                    .map_err(|e| format!("cast neutered response: {e}"))
            }
            _ => env
                .CreateWebResourceResponse(None, 200, windows_core::w!("OK"), headers_pcw)
                .map_err(|e| format!("CreateWebResourceResponse: {e}")),
        }
    }
}

/// A WinHTTP handle that closes itself (the getter below
/// juggles session/connect/request triples per redirect hop;
/// RAII keeps every exit path leak-free).
#[cfg(windows)]
struct WinHttpHandle(*mut std::ffi::c_void);

#[cfg(windows)]
impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer is a live WinHTTP handle this
            // struct owns exclusively; it is closed exactly
            // once, here.
            unsafe {
                windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// Split an `https://` URL into (host, port, path+query) for
/// WinHttpConnect/WinHttpOpenRequest. Anything else — plain
/// http, a missing host, a non-numeric port — is refused:
/// the list download is HTTPS-only by construction.
#[cfg(windows)]
fn split_https_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("https://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
            (h, p.parse::<u16>().ok()?)
        }
        _ => (authority, 443),
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port, path.to_string()))
}

/// One HTTPS GET for the Shield's list download (GitHub
/// edition): WinHTTP — the OS stack — so SChannel TLS and the
/// system proxy configuration come from Windows itself and no
/// third-party HTTP code enters the tree. The transport is
/// deliberately narrow: it exists only for the four canonical
/// list URLs in `core::lists`, sends no cookies and no custom
/// headers (the session's plain `Pico/0.31` User-Agent only),
/// follows at most 5 redirects MANUALLY (automatic following
/// is switched off, so the https→https-only policy is enforced
/// here, hop by hop), and enforces the caller's size cap
/// while streaming — a response is never buffered past
/// `max_bytes`. Returns the body or a short log-safe error
/// (status codes and OS error numbers only).
#[cfg(windows)]
fn winhttp_get(url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Networking::WinHttp::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn last_err(what: &str) -> String {
        // SAFETY: GetLastError is a thread-local read with no
        // preconditions; it is called immediately after the
        // failed WinHTTP call, before anything can reset it.
        let code = unsafe { GetLastError() };
        format!("{what} (winhttp error {code})")
    }

    let mut current = url.to_string();
    let mut hops = 0usize;
    loop {
        let (host, port, path) =
            split_https_url(&current).ok_or_else(|| format!("not an https URL: {current}"))?;
        let host_w = wide(&host);
        let path_w = wide(&path);
        let ua_w = wide("Pico/0.31");
        // SAFETY: every handle below is created by WinHTTP and
        // immediately owned by a WinHttpHandle (closed on drop);
        // all string pointers reference the NUL-terminated wide
        // buffers above, which outlive every call in this
        // iteration; null optional parameters are the documented
        // "none/default" values.
        let (status, location, body) = unsafe {
            let session = WinHttpHandle(WinHttpOpen(
                ua_w.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            ));
            if session.0.is_null() {
                return Err(last_err("WinHttpOpen failed"));
            }
            WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 15_000);
            let decomp: u32 = WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
            WinHttpSetOption(
                session.0,
                WINHTTP_OPTION_DECOMPRESSION,
                &decomp as *const u32 as *const std::ffi::c_void,
                4,
            );
            let connect = WinHttpHandle(WinHttpConnect(
                session.0,
                host_w.as_ptr(),
                port,
                0,
            ));
            if connect.0.is_null() {
                return Err(last_err("WinHttpConnect failed"));
            }
            let verb_w = wide("GET");
            let request = WinHttpHandle(WinHttpOpenRequest(
                connect.0,
                verb_w.as_ptr(),
                path_w.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ));
            if request.0.is_null() {
                return Err(last_err("WinHttpOpenRequest failed"));
            }
            // Redirects are followed manually (see the doc
            // comment): switch WinHTTP's own following off.
            let never: u32 = 0; // WINHTTP_OPTION_REDIRECT_POLICY_NEVER
            WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_REDIRECT_POLICY,
                &never as *const u32 as *const std::ffi::c_void,
                4,
            );
            if WinHttpSendRequest(
                request.0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                0,
                0,
            ) == 0
            {
                return Err(last_err("WinHttpSendRequest failed"));
            }
            if WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
                return Err(last_err("WinHttpReceiveResponse failed"));
            }
            let mut status: u32 = 0;
            let mut status_len: u32 = 4;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                &mut status as *mut u32 as *mut std::ffi::c_void,
                &mut status_len,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(last_err("WinHttpQueryHeaders(status) failed"));
            }
            if (300..400).contains(&status) {
                // Two-call sizing for the Location string.
                let mut len: u32 = 0;
                WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_LOCATION,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    &mut len,
                    std::ptr::null_mut(),
                );
                if len == 0 || len > 8192 {
                    return Err(format!(
                        "redirect with an unusable Location (HTTP {status})"
                    ));
                }
                let mut buf = vec![0u16; (len as usize).div_ceil(2)];
                if WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_LOCATION,
                    std::ptr::null(),
                    buf.as_mut_ptr() as *mut std::ffi::c_void,
                    &mut len,
                    std::ptr::null_mut(),
                ) == 0
                {
                    return Err(last_err("WinHttpQueryHeaders(location) failed"));
                }
                let loc = String::from_utf16_lossy(&buf)
                    .trim_end_matches('\0')
                    .to_string();
                (status, Some(loc), Vec::new())
            } else if status == 200 {
                let mut body: Vec<u8> = Vec::new();
                let mut chunk = vec![0u8; 32 * 1024];
                loop {
                    let mut avail: u32 = 0;
                    if WinHttpQueryDataAvailable(request.0, &mut avail) == 0 {
                        return Err(last_err("WinHttpQueryDataAvailable failed"));
                    }
                    if avail == 0 {
                        break;
                    }
                    let want = avail.min(chunk.len() as u32);
                    let mut read: u32 = 0;
                    if WinHttpReadData(
                        request.0,
                        chunk.as_mut_ptr() as *mut std::ffi::c_void,
                        want,
                        &mut read,
                    ) == 0
                    {
                        return Err(last_err("WinHttpReadData failed"));
                    }
                    if read == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..read as usize]);
                    if body.len() > max_bytes {
                        return Err("response exceeds the size cap".to_string());
                    }
                }
                (status, None, body)
            } else {
                (status, None, Vec::new())
            }
        };
        match location {
            Some(loc) => {
                hops += 1;
                if hops > 5 {
                    return Err("too many redirects".to_string());
                }
                // Absolute https targets are followed as-is;
                // a path-relative Location resolves against the
                // current authority. Anything else (notably an
                // http:// downgrade) is refused by
                // split_https_url on the next pass.
                current = if loc.starts_with("https://") {
                    loc
                } else if loc.starts_with('/') {
                    let (h, p, _) = split_https_url(&current)
                        .ok_or_else(|| "redirect from an invalid URL".to_string())?;
                    if p == 443 {
                        format!("https://{h}{loc}")
                    } else {
                        format!("https://{h}:{p}{loc}")
                    }
                } else {
                    return Err(format!("unfollowable redirect (HTTP {status})"));
                };
            }
            None => {
                if status == 200 {
                    return Ok(body);
                }
                return Err(format!("HTTP {status}"));
            }
        }
    }
}

/// Enumerate this PC's unicast IPv4 addresses for Teleport
/// (RESEARCH_v25.md §b): `GetAdaptersAddresses` with the two-call
/// sizing pattern. Only each address and its adapter's up/down
/// state are read — names/descriptions are never touched, and
/// nothing here is logged. The pure choice among candidates is
/// `core::teleport::choose_lan_ip`'s; if enumeration yields
/// nothing usable, the caller falls back to the UDP egress probe.
#[cfg(windows)]
fn enumerate_lan_candidates() -> Vec<pico_core::core::teleport::LanCandidate> {
    use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    const GAA_SKIP_ANYCAST_MULTICAST_DNS: u32 = 0x2 | 0x4 | 0x8;
    let mut out = Vec::new();
    // SAFETY: the sizing call passes a null buffer and a valid
    // size cell, exactly the documented pattern; the walk below
    // stays inside the buffer GetAdaptersAddresses filled (its
    // linked lists are NULL-terminated and wholly contained in
    // the allocation), every pointer is null-checked before
    // dereference, and the buffer outlives the walk.
    unsafe {
        let mut size: u32 = 0;
        let first = GetAdaptersAddresses(
            AF_INET as u32,
            GAA_SKIP_ANYCAST_MULTICAST_DNS,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut size,
        );
        if first != ERROR_BUFFER_OVERFLOW || size == 0 {
            return out;
        }
        // AUDIT 0.31 (A4-F5): the adapter set can GROW between
        // the sizing call and the fill call (a VPN connects, a
        // dock wakes), making the fill itself return
        // ERROR_BUFFER_OVERFLOW with a fresh required size in
        // `size`. Through 0.30 that was a silent empty
        // enumeration — Teleport fell back to the bare egress
        // probe. Retry the fill with the updated size, at most
        // 3 attempts.
        // u64 backing store keeps the buffer 8-byte aligned
        // for the adapter structs.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let mut head: *mut IP_ADAPTER_ADDRESSES_LH = std::ptr::null_mut();
        let mut filled = false;
        for _attempt in 0..3 {
            head = buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
            let rc = GetAdaptersAddresses(
                AF_INET as u32,
                GAA_SKIP_ANYCAST_MULTICAST_DNS,
                std::ptr::null(),
                head,
                &mut size,
            );
            if rc == 0 {
                filled = true;
                break;
            }
            if rc != ERROR_BUFFER_OVERFLOW || size == 0 {
                return out;
            }
            // Reallocate at the freshly-reported size for the
            // next attempt.
            buffer = vec![0u64; (size as usize).div_ceil(8)];
        }
        if !filled {
            return out;
        }
        // IpDadStateDuplicate in the IP_DAD_STATE enumeration
        // (iptypes.h): the address is contested on the LAN.
        const IP_DAD_STATE_DUPLICATE: i32 = 2;
        let mut adapter = head;
        while !adapter.is_null() {
            let up = (*adapter).OperStatus == 1; // IfOperStatusUp
                                                 // AUDIT 0.31 (A4-F2): the adapter kind rides along
                                                 // — the chooser must not let a tunnel outrank a
                                                 // physical adapter.
            let kind = pico_core::core::teleport::lan_kind_for_if_type((*adapter).IfType);
            let mut unicast = (*adapter).FirstUnicastAddress;
            while !unicast.is_null() {
                let next_unicast = (*unicast).Next;
                // AUDIT 0.31 (A4-F2): DAD-duplicate addresses
                // are excluded HERE, at enumeration (the
                // chooser filters again for other sources).
                let dad_duplicate = (*unicast).DadState == IP_DAD_STATE_DUPLICATE;
                if !dad_duplicate {
                    let sockaddr = (*unicast).Address.lpSockaddr
                        as *const windows_sys::Win32::Networking::WinSock::SOCKADDR_IN;
                    if !sockaddr.is_null() && (*sockaddr).sin_family == AF_INET {
                        let raw = (*sockaddr).sin_addr.S_un.S_addr;
                        out.push(pico_core::core::teleport::LanCandidate {
                            ip: std::net::Ipv4Addr::from(raw.to_ne_bytes()),
                            up,
                            kind,
                            dad_duplicate: false,
                        });
                    }
                }
                unicast = next_unicast;
            }
            adapter = (*adapter).Next;
        }
    }
    out
}

#[cfg(not(windows))]
fn enumerate_lan_candidates() -> Vec<pico_core::core::teleport::LanCandidate> {
    // Non-Windows hosts have no shipped Teleport path; the
    // caller's UDP egress fallback still applies.
    Vec::new()
}

/// Register the Shield's network layer on one content WebView
/// (AUDIT V21): a `WebResourceRequested` handler for all resource
/// contexts on all URLs. The handler is a thin fail-open shell
/// around the ONE decision in `core::filterpipe::decide` (pure
/// core; AUDIT V26 D6 — through v25 it composed
/// `ShieldRuntime::should_block` and a Lite consult by hand;
/// that seam was deleted in v28, AUDIT A2-M2):
/// every fallible step is checked, and ANY failure — COM,
/// parsing, a missing engine — leaves the request completely
/// untouched (safety law 3). Blocked requests get the
/// type-appropriate neutered response, synchronously; no
/// deferral is taken.
///
/// v22 (AUDIT V22-9): the block is recorded — counters, the
/// ShieldBlocked shell event, the sampled log line — only AFTER
/// the neutered response has actually been set on the request.
/// v21 recorded first, so a COM failure while building or
/// setting the response left the counters claiming a block that
/// fail-open had not delivered.
#[cfg(windows)]
fn attach_shield(
    view: &WebView,
    tab: u64,
    runtime: Arc<ShieldRuntime>,
    lite: Arc<LiteRuntime>,
    stats: Arc<pico_core::core::stats::StatsBook>,
    proxy: EventLoopProxy<UserEvent>,
) {
    use pico_core::core::shield::{
        host_of, resource_type_from_context, urls_equivalent, ResourceType,
    };
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;
    use webview2_com::WebResourceRequestedEventHandler;
    use wry::WebViewExtWindows;

    let outcome = (|| -> Result<(), String> {
        let core = view.webview();
        let env = view.environment();
        // SAFETY: registration calls on the view's live core and
        // environment; the filter is a static wide string; the
        // token cell is a plain local.
        unsafe {
            core.AddWebResourceRequestedFilter(
                windows_core::w!("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            )
            .map_err(|e| format!("AddWebResourceRequestedFilter: {e}"))?;
        }
        // AUDIT V26 (D7): the navigation-state handlers ride
        // the same registration as the resource handler (this
        // function is the per-view wiring point for everything
        // WebView2 tells us about a content view).
        let src_proxy = proxy.clone();
        let hist_proxy = proxy.clone();
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_wv, args| {
            let Some(args) = args else {
                return Ok(());
            };
            // SAFETY: the args and their request view are valid
            // for this synchronous callback; every getter follows
            // the media probe's established pattern, and every
            // error path returns Ok without touching the request
            // (fail open). The response we build is a fresh COM
            // object owned by the event args once set.
            unsafe {
                let _ = (|| -> Result<(), String> {
                    let request = args.Request().map_err(|e| e.to_string())?;
                    // The request object's identity token for
                    // the savings ledger's duration correlation
                    // (0.30): the raw interface pointer. The
                    // response observer receives the same object
                    // for this request and completes the entry.
                    let req_token = windows_core::Interface::as_raw(&request) as usize as u64;
                    let mut uri_ptr = windows_core::PWSTR::null();
                    request.Uri(&mut uri_ptr).map_err(|e| e.to_string())?;
                    let uri = webview2_com::take_pwstr(uri_ptr);
                    // The HTTP method, for `$method=` rules
                    // (AUDIT V23). A failed read is not fatal:
                    // None means "unknown", and method-scoped
                    // rules fail open on unknown.
                    let method = {
                        let mut method_ptr = windows_core::PWSTR::null();
                        match request.Method(&mut method_ptr) {
                            Ok(()) => Some(webview2_com::take_pwstr(method_ptr)),
                            Err(_) => None,
                        }
                    };
                    let mut ctx =
                        webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT(0);
                    args.ResourceContext(&mut ctx).map_err(|e| e.to_string())?;
                    // AUDIT V28 (A2-L13): the page state is read
                    // ONCE, here, and shared by the main-frame
                    // determination and the pipeline facts —
                    // through v27 the comment below claimed one
                    // read while the code read it twice, and the
                    // two facts could straddle a commit.
                    let (committed, pending) = runtime.page_state(tab);
                    // Main-frame determination: a DOCUMENT-context
                    // request whose URL is the tab's recorded
                    // main-frame target (NavigationStarting) or
                    // its committed URL. Anything else document-
                    // shaped is an iframe (Subdocument). The bias
                    // is deliberate: ties protect the document
                    // (safety law 1).
                    let main_frame = if resource_type_from_context(ctx.0) == ResourceType::Document
                    {
                        urls_equivalent(&uri, &pending) || urls_equivalent(&uri, &committed)
                    } else {
                        false
                    };
                    // AUDIT V26 (D6): ONE decision for both
                    // filters — the unified pipeline in
                    // core::filterpipe. Both toggle flags are
                    // snapshotted ONCE, so the verdict always
                    // describes one real instant of the toggles
                    // (the toggle-race pin). The verdict carries
                    // its attribution: exactly one counter moves.
                    enum Hit {
                        Shield(ResourceType),
                        Lite(ResourceType),
                    }
                    let page_url = if committed.is_empty() {
                        pending
                    } else {
                        committed
                    };
                    let rtype = pico_core::core::shield::effective_rtype(ctx.0, main_frame);
                    // AUDIT V29: Lite v2's two facts — the
                    // page's watch shape (from its live URL) and
                    // the tab's chosen-video designation (set by
                    // the validated lite_chosen report).
                    let page_watch_shaped = pico_core::core::lite::is_watch_shaped_url(&page_url);
                    let tab_chosen = lite.is_chosen(tab);
                    let facts = pico_core::core::filterpipe::RequestFacts {
                        url: uri.clone(),
                        page_url,
                        rtype,
                        method,
                        page_watch_shaped,
                        tab_chosen,
                    };
                    let filter_state = pico_core::core::filterpipe::FilterState {
                        shield_on: runtime.on.load(std::sync::atomic::Ordering::Relaxed),
                        lite_on: lite.on.load(std::sync::atomic::Ordering::Relaxed),
                    };
                    let engine = runtime.engine_snapshot();
                    let hit = match pico_core::core::filterpipe::decide(
                        &facts,
                        filter_state,
                        engine.as_deref(),
                    ) {
                        pico_core::core::filterpipe::Verdict::Allow => {
                            // 0.30: an allowed PUBLIC request
                            // (public page, public target — the
                            // ledger's sampling law) starts a
                            // timing entry the response observer
                            // completes. Blocked requests are
                            // never noted: they have no response
                            // to time, and Pico's neutered
                            // response must never be mistaken for
                            // a real sample.
                            if pico_core::core::stats::is_samplable(
                                pico_core::core::filterpipe::classify_origin(&facts.url),
                            ) && pico_core::core::stats::is_samplable(
                                pico_core::core::filterpipe::classify_origin(&facts.page_url),
                            ) {
                                // AUDIT 0.31 (A2-F9): HEAD
                                // responses carry no body —
                                // the book must not learn
                                // durations/bytes from them.
                                // The book's method-aware entry
                                // point takes the method itself
                                // (stats::note_request_with_method).
                                stats.note_request_with_method(
                                    req_token,
                                    facts.rtype,
                                    stats_now_ms(),
                                    facts.method.as_deref(),
                                );
                            }
                            return Ok(());
                        }
                        pico_core::core::filterpipe::Verdict::ShieldBlock(r) => Hit::Shield(r),
                        pico_core::core::filterpipe::Verdict::LiteBlock(r) => Hit::Lite(r),
                    };
                    let rtype = match hit {
                        Hit::Shield(r) => r,
                        Hit::Lite(r) => r,
                    };
                    // AUDIT V22-9: build and set the neutered
                    // response FIRST; only a delivered neuter may
                    // move the counters, the event, and the log.
                    let response = match make_neutered_response(&env, rtype) {
                        Ok(r) => r,
                        Err(why) => {
                            crate::log::error(&format!(
                                "shield: neuter failed for tab {tab} (request left untouched): {why}"
                            ));
                            return Ok(());
                        }
                    };
                    if let Err(e) = args.SetResponse(&response) {
                        crate::log::error(&format!(
                            "shield: SetResponse failed for tab {tab} (request left untouched): {e}"
                        ));
                        return Ok(());
                    }
                    match hit {
                        Hit::Shield(_) => {
                            let (_, _, log_it) = runtime.record_block(tab);
                            // 0.30: the savings book records
                            // the same delivered block, credited
                            // from the ledger (core::stats).
                            stats.record_block(
                                pico_core::core::stats::Engine::Shield,
                                rtype,
                                facts.method.as_deref(),
                            );
                            let _ = proxy.send_event(UserEvent::ShieldBlocked);
                            if log_it {
                                if let Some(host) = host_of(&uri) {
                                    crate::log::info(&format!(
                                        "shield block: tab={tab} host={host} type={}",
                                        rtype.label()
                                    ));
                                }
                            }
                        }
                        Hit::Lite(_) => {
                            let (_, log_it) = lite.record_block();
                            stats.record_block(
                                pico_core::core::stats::Engine::Lite,
                                rtype,
                                facts.method.as_deref(),
                            );
                            let _ = proxy.send_event(UserEvent::LiteBlocked);
                            if log_it {
                                if let Some(host) = host_of(&uri) {
                                    crate::log::info(&format!(
                                        "lite block: tab={tab} host={host} type={}",
                                        rtype.label()
                                    ));
                                }
                            }
                        }
                    }
                    Ok(())
                })();
            }
            Ok(())
        }));
        // AUDIT V26 (D7): SourceChanged — the platform's live
        // URL signal, never wired before v26. The handler reads
        // the sender's current Source and forwards it; the
        // event loop feeds it to the tab's URL reducer.
        let source_handler =
            webview2_com::SourceChangedEventHandler::create(Box::new(move |sender, _args| {
                let Some(sender) = sender else {
                    return Ok(());
                };
                // SAFETY: the sender is the live core for this
                // synchronous callback; Source writes a PWSTR
                // whose ownership take_pwstr assumes, the same
                // pattern as the resource handler's Uri read.
                unsafe {
                    let mut ptr = windows_core::PWSTR::null();
                    if sender.Source(&mut ptr).is_ok() {
                        let url = webview2_com::take_pwstr(ptr);
                        if !url.is_empty() {
                            let _ = src_proxy.send_event(UserEvent::SourceChanged {
                                tab,
                                url: pico_core::core::addr::normalize_pico_url(&url),
                            });
                        }
                    }
                }
                Ok(())
            }));
        // HistoryChanged: no payload is read (Pico's own
        // history machine is the nav-button authority, AUDIT
        // V26 D7); the event only triggers a snapshot re-push.
        let history_handler =
            webview2_com::HistoryChangedEventHandler::create(Box::new(move |_sender, _args| {
                let _ = hist_proxy.send_event(UserEvent::HistoryChanged { tab });
                Ok(())
            }));
        let mut token: i64 = 0;
        let mut token_src: i64 = 0;
        let mut token_hist: i64 = 0;
        // SAFETY: the handlers are webview2-com callback objects
        // kept alive by the core's event table once registered.
        unsafe {
            core.add_WebResourceRequested(&handler, &mut token)
                .map_err(|e| format!("add_WebResourceRequested: {e}"))?;
            core.add_SourceChanged(&source_handler, &mut token_src)
                .map_err(|e| format!("add_SourceChanged: {e}"))?;
            core.add_HistoryChanged(&history_handler, &mut token_hist)
                .map_err(|e| format!("add_HistoryChanged: {e}"))?;
        }
        Ok(())
    })();
    if let Err(why) = outcome {
        log::info(&format!("shield unavailable for tab {tab}: {why}"));
    }
}

#[cfg(not(windows))]
fn attach_shield(
    _view: &WebView,
    _tab: u64,
    _runtime: Arc<ShieldRuntime>,
    _lite: Arc<LiteRuntime>,
    _stats: Arc<pico_core::core::stats::StatsBook>,
    _proxy: EventLoopProxy<UserEvent>,
) {
}

/// Keep the chrome WebView's container HWND at the top of the
/// sibling z-order (AUDIT V16-1). The v16 feather only works if the
/// chrome *composites over* the content in the 12-DIP overlap rows:
/// the content's own top rows are opaque page, so a content HWND
/// sitting above the chrome would paint straight over the gradient
/// and the blend would silently vanish. Sibling HWNDs are z-ordered
/// by creation (later = higher), the chrome is created *first*, and
/// every new tab would land on top of it — so each layout pass
/// re-asserts the chrome's place. The handle comes from the
/// controller's ParentWindow (wry 0.53.5 exposes no HWND accessor);
/// the raise is SWP_NOACTIVATE and size/position-preserving, and a
/// failure is benign ( worst case the feather is hidden, exactly
/// the pre-guard behavior), so the result is ignored.
#[cfg(windows)]
fn raise_chrome_above_content(view: &WebView) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    use wry::WebViewExtWindows;

    let controller = view.controller();
    let mut hwnd: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: ParentWindow writes the controller's parent HWND (a
    // plain handle, no ownership transfer) into the out-pointer;
    // the pointee type in webview2-com's bindings is the `windows`
    // crate's transparent HWND wrapper over the same raw handle,
    // hence the pointer cast. SetWindowPos then only re-orders
    // that child window among its siblings.
    unsafe {
        if controller
            .ParentWindow((&mut hwnd) as *mut *mut core::ffi::c_void as _)
            .is_ok()
            && !hwnd.is_null()
        {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(windows))]
fn raise_chrome_above_content(_view: &WebView) {}

// ---------------------------------------------------------------------------
// User events (everything async arrives here)
// ---------------------------------------------------------------------------

enum UserEvent {
    /// Raw JSON command string from the chrome page.
    ChromeCmd(String),
    /// SPA navigation note from a content page's observer script.
    NavNote {
        tab: u64,
        note: NavNote,
    },
    /// SourceChanged (AUDIT V26, defect D7): the WebView's live
    /// Source property, read by the platform handler at fire
    /// time. This is the authoritative URL signal the v25 shell
    /// never wired — it is what makes the address bar follow
    /// pushState/replaceState navigations.
    SourceChanged {
        tab: u64,
        url: String,
    },
    /// HistoryChanged (AUDIT V26, defect D7): the page's
    /// session-history shape changed (SPA pushes/traversals).
    /// Carries no payload: Pico's own history machine is the
    /// authority for the nav buttons (it initiates traversals
    /// itself), so this event only re-pushes the snapshot that
    /// recomputes them.
    HistoryChanged {
        tab: u64,
    },
    TitleChanged {
        tab: u64,
        title: String,
    },
    PageLoad {
        tab: u64,
        finished: bool,
        url: String,
    },
    /// window.open / target=_blank from a content webview. The
    /// handler itself decides nothing (AUDIT V22-10/15): the event
    /// carries the target to the event loop, where the pure
    /// routing decision in `core::addr::route_new_window` is taken
    /// and logged.
    NewWindow {
        tab: u64,
        url: String,
    },
    /// A start-page action request, from either transport (AUDIT
    /// V17-1): an IPC message from the start page's script, or a
    /// legacy `pico://…` action navigation. Decided centrally by
    /// `pico_core::core::action::dispatch` in `on_start_action` —
    /// the handlers that produce this event make no decisions.
    StartAction {
        tab: u64,
        source: ActionSource,
    },
    /// A download was requested and denied (Pico has no downloads
    /// manager by design; see SECURITY.md).
    DownloadBlocked {
        tab: u64,
        uri: String,
    },
    DroppedPaths(Vec<PathBuf>),
    ServicesUpdated(ScanOutcome),
    /// Chrome handshake watchdog (AUDIT V14-3): fired ~2.5 s after a
    /// chrome WebView of generation `gen` was created. If that
    /// generation never sent its `ready` command, the chrome is
    /// recreated once; a second miss escalates to a native dialog.
    ChromeWatchdog(u64),
    /// Roots whose files changed. The SSE broadcasts already fired at
    /// `broadcast_at`; tabs that began (re)loading after that moment
    /// reloaded themselves and are skipped by the fallback.
    FilesChanged {
        roots: Vec<u64>,
        broadcast_at: Instant,
    },
    /// The Shield's filter lists finished compiling on the
    /// background thread (AUDIT V21): the engine is installed in
    /// the shared runtime and already deciding requests; this
    /// event only lets the app push the cosmetic script into
    /// pages that loaded before it existed and refresh the shell.
    ShieldReady,
    /// One request was blocked (AUDIT V21): the counters already
    /// moved in the shared runtime; this only refreshes the
    /// shell's chip (signature-gated, so floods cost nothing).
    ShieldBlocked,
    /// One request was Lite-blocked (v25): same deal — counters
    /// moved in the Lite runtime; refresh the shell's Lite chip.
    LiteBlocked,
    /// The scriptlet payload in one tab reported prune counts
    /// (AUDIT V23): a diagnostic hint from page context — the
    /// items feed capped sample log lines only, never counters.
    ShieldPrune {
        tab: u64,
        items: Vec<PruneItem>,
    },
    /// A tab's Lite gate designated its chosen video
    /// (AUDIT V29): a payload-free hint from page context. The
    /// handler validates the sender (a live, public, non-start
    /// content tab with Lite on) before the tab's chosen flag
    /// is set in the Lite runtime; the flag's only effect is
    /// as a `filterpipe` fact.
    LiteChosen {
        tab: u64,
    },
    /// The scriptlet payload in one tab reported MEASURED
    /// pruned payload (0.30): exact serialized bytes of ad
    /// subtrees pruned in-page, plus the subtree count. Unlike
    /// `ShieldPrune` (diagnostic-only), this feeds the savings
    /// book — so the handler validates the sender exactly like
    /// `LiteChosen`'s (Shield on, live public non-start tab)
    /// and the book applies its own rate/bounds gates before
    /// recording.
    ShieldPruned {
        tab: u64,
        bytes: u64,
        events: u64,
    },
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

struct Tab {
    id: u64,
    /// The tab's WebView. `None` when creation failed (AUDIT M6):
    /// the tab exists in the strip with its failure reason and can be
    /// retried via Reload or closed — the rest of the app lives on.
    view: Option<WebView>,
    /// Creation failure reason, when `view` is `None`.
    failed: Option<String>,
    is_start: bool,
    /// The tab's current URL — a MIRROR of `urlstate`'s live
    /// value (AUDIT V26, defect D7). Every write goes through
    /// the reducer (`App::apply_url_event`); the snapshot and
    /// Teleport read this field, the reducer owns it.
    url: String,
    /// The URL state machine (AUDIT V26, defect D7): the single
    /// source of truth for this tab's URL. Commits, SourceChanged
    /// events and observer notes all feed it, in arrival order.
    urlstate: UrlState,
    title: String,
    loading: bool,
    /// Static-server root currently shown, if any (live-reload scope).
    root_id: Option<u64>,
    /// True when this tab *holds* a reference on `root_id` (it opened
    /// the folder); false when it merely borrowed the root by
    /// navigating to a served URL. Only holders release (AUDIT M2).
    root_held: bool,
    /// Reconciled session-history state machine.
    hist: TabHistory,
    /// When the current/last load started (SSE live-reload fallback).
    last_load_start: Option<Instant>,
    /// True when this tab's WebView was built with the Shield's
    /// document script (scriptlets + cosmetics, AUDIT V23) as a
    /// document-start script (AUDIT V21). Tabs built before the
    /// lists finished compiling (or while the Shield was off)
    /// get the script pushed per load instead.
    doc_start_script: bool,
    /// Whether a CommitStart has been applied for the document
    /// currently loading (AUDIT 0.31, A2-F1). Set by
    /// `apply_url_event` on CommitStart, consumed by
    /// CommitFinish: a finish WITH a preceding start must not
    /// clear a Lite chosen designation made during that
    /// document's load; a finish WITHOUT one (orphan) still
    /// clears, as every commit did through 0.30.
    commit_started: bool,
}

impl Tab {
    fn view(&self) -> Option<&WebView> {
        self.view.as_ref()
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    window: Rc<Window>,
    web_ctx: WebContext,
    chrome: WebView,
    tabs: Vec<Tab>,
    active: u64,
    next_tab_id: u64,
    server: FileServer,
    persist: PersistState,
    shared: Shared,
    proxy: EventLoopProxy<UserEvent>,
    watchers: HashMap<u64, notify::RecommendedWatcher>,
    watch_tx: std::sync::mpsc::SyncSender<(u64, Vec<PathBuf>)>,
    dedupe: Dedupe,
    services_count: usize,
    /// Non-qualifying local listeners (AUDIT V14-5 aggregate).
    others_count: usize,
    blocked_downloads: usize,
    download_notice: String,
    /// Signature of the last snapshot actually sent (AUDIT V28
    /// A1-M1): a cheap hash over the snapshot's primitive fields,
    /// computed BEFORE any snapshot construction, so a gated-out
    /// push costs a few hashes — not a themes-catalogue clone, a
    /// QR-matrix clone, and a full JSON serialization.
    last_snap_sig: u64,
    /// Chrome shell health (AUDIT V14-3): generation of the current
    /// chrome WebView (incremented on recreation), whether its
    /// `ready` handshake arrived, whether the first snapshot push was
    /// logged, and whether the single recreation has been spent.
    chrome_gen: u64,
    chrome_ready: bool,
    chrome_snapshot_logged: bool,
    chrome_recreated: bool,
    /// The explicit user agent set on every content WebView
    /// (AUDIT V18-1, form changed V22-14): composed once at startup
    /// from the installed WebView2 runtime's version
    /// (`core::media::compose_user_agent`).
    user_agent: String,
    /// Per-tab media-probe log budgets (AUDIT V18-3), shared with
    /// every content WebView's response-received handler.
    probe_budget: Arc<Mutex<MediaProbeBudget>>,
    /// The Shield runtime (AUDIT V21): shared with every content
    /// WebView's request handler. The engine is installed by the
    /// background compile thread; until then (and whenever the
    /// Shield is off) requests pass untouched.
    shield: Arc<ShieldRuntime>,
    /// The savings book (0.30): exact blocked counts,
    /// calibrated byte/time credits, the response ledger and
    /// the Shield's exact pruned-payload figures, shared with
    /// every content WebView's request handler and response
    /// observer. In-memory authoritative; `save_state` mirrors
    /// its lifetime half into `persist.stats` (and the legacy
    /// `persist.blocked_total` mirror) at the normal save
    /// points.
    stats: Arc<pico_core::core::stats::StatsBook>,
    /// Signature of the last stats view pushed to the start
    /// tabs (0.30): the band re-renders only when a figure
    /// actually moved, so block floods cost one hash.
    last_stats_sig: u64,
    /// Lite Mode runtime (v25): shared with every content
    /// WebView's request handler, consulted strictly after the
    /// Shield's decision.
    lite: Arc<LiteRuntime>,
    /// The latest Teleport resolution for the active tab (v25),
    /// computed on demand by the chrome's `teleport` command and
    /// cleared whenever the active tab changes. Rides the
    /// snapshot; the chrome renders the panel from it.
    teleport: Option<TeleportSnap>,
    /// Whether the Teleport panel is open (v25): while true the
    /// chrome rect grows by the panel height (an overlay — the
    /// content rect never moves; see core::layout).
    teleport_panel_open: bool,
    /// Monotonic counter bumped at every `teleport` assignment
    /// (AUDIT V28 A1-M1): the snapshot signature folds this in
    /// instead of hashing/cloning the QR matrix per push.
    teleport_version: u64,
    /// Whether the theme picker panel is open (v27): same
    /// overlay mechanics as Teleport's panel, and mutually
    /// exclusive with it — opening one closes the other.
    palette_panel_open: bool,
    /// One-shot address-focus hint (AUDIT V26, defect D4): set
    /// when a fresh tab is created via the new-tab paths (the +
    /// button, Ctrl+T, the startup tab, a chrome (re)creation
    /// landing on a start tab); the next snapshot carries it as
    /// `focus_addr` and the chrome focuses + selects the address
    /// input. Consumed by push_state so it fires exactly once.
    pending_focus_addr: bool,
    /// The live set of start-tab ids (AUDIT V26, defect D5),
    /// shared with the WebView navigation handlers so their
    /// start-tab classification follows CONVERSION: when a start
    /// tab converts in place it leaves this set, and its
    /// handler stops treating it as a start tab from that
    /// moment. (Through v25 the handlers captured `is_start` at
    /// build time, which made conversion impossible.)
    live_start_tabs: std::sync::Arc<Mutex<std::collections::HashSet<u64>>>,
    /// Last layout signature logged (AUDIT V26, defect D2): the
    /// layout witness line is written when the computed geometry
    /// changes, so pico.log can answer "what rects did the shell
    /// actually apply" without logging every pass.
    last_layout_sig: String,
    /// One-shot shell navigation expectation (AUDIT 0.31,
    /// A6-F2): `(tab_id, exact URL)` the shell itself most
    /// recently submitted for a tab from USER-TYPED text. A
    /// same-tab navigation to a credentialed (userinfo) URL is
    /// allowed only when it matches this exactly; the
    /// expectation is consumed by the first navigation event
    /// for that tab, matched or not. Page-initiated
    /// credentialed navigations never set it, so they never
    /// match — typed credentialed URLs stay allowed as the
    /// user's own act, page-minted ones are denied. Shared
    /// (like `live_start_tabs`) because the synchronous
    /// navigation handlers — not the event loop — consume it.
    expected_nav: std::sync::Arc<Mutex<Option<(u64, String)>>>,
}

/// The Teleport payload as the chrome consumes it (v25): the
/// resolution kind as a wire label, the URL to show (empty for
/// not-teleportable), the researched panel note, and the QR
/// module matrix as `"1"`/`"0"` row strings (empty when there is
/// no encodable URL or the URL exceeds QR capacity — the note
/// then says so and the URL stays copyable as text).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TeleportSnap {
    url: String,
    note: String,
    matrix: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TabSnap {
    id: u64,
    title: String,
    url: String,
    loading: bool,
    start: bool,
    failed: bool,
}

/// One palette as the chrome's picker consumes it (v27): the
/// id the `theme_set` command sends back, the display name,
/// and the six palette hexes for the card's dots — all from
/// the one table in core::themes.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ThemeSnap {
    id: String,
    name: String,
    slots: Vec<String>,
}

/// The picker catalogue, built once from the static palette
/// table (AUDIT V28 A1-M1) — it cannot change at runtime, so it
/// is shared by reference instead of being rebuilt (and cloned)
/// on every snapshot.
fn themes_snap() -> &'static Vec<ThemeSnap> {
    static THEMES_SNAP: std::sync::OnceLock<Vec<ThemeSnap>> = std::sync::OnceLock::new();
    THEMES_SNAP.get_or_init(|| {
        pico_core::core::themes::PALETTES
            .iter()
            .map(|p| ThemeSnap {
                id: p.id.to_string(),
                name: p.name.to_string(),
                slots: p.slots.iter().map(|s| s.to_string()).collect(),
            })
            .collect()
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snap {
    tabs: Vec<TabSnap>,
    active: u64,
    theme: String,
    /// The active palette id (v27) + the picker catalogue and
    /// the picker's open state.
    palette: String,
    /// The picker catalogue: a process constant behind a
    /// OnceLock, so a snapshot clone is a pointer (AUDIT V28
    /// A1-M1).
    themes: &'static Vec<ThemeSnap>,
    palette_open: bool,
    /// Teleport's open state (AUDIT V28 A1-H1): the chrome syncs
    /// its panel from this exactly like `palette_open`, so a
    /// Rust-side close (navigation, tab switch, conversion) can
    /// no longer leave a stale panel painted in the chrome.
    teleport_open: bool,
    address: String,
    can_back: bool,
    can_forward: bool,
    services: usize,
    blocked_downloads: usize,
    download_notice: String,
    active_error: String,
    /// Shield state (AUDIT V21): on/off, requests blocked this
    /// session, and the active tab's own count.
    shield_on: bool,
    blocked_total: u64,
    blocked_tab: u64,
    /// Lite Mode state (v25): on/off and this session's count.
    lite_on: bool,
    lite_blocked: u64,
    /// Teleport payload (v25) when the chrome asked for one.
    teleport: Option<TeleportSnap>,
    /// Address-focus hint (AUDIT V26, defect D4): true on the
    /// one snapshot that follows a fresh tab's creation — the
    /// chrome focuses the address input and selects its text.
    focus_addr: bool,
    version: &'static str,
}

/// Convert a core device-pixel rect into a wry bounds rect. Physical
/// units end-to-end: what the tiling math computed is what the HWND
/// gets — no second rounding anywhere (AUDIT V14-4).
fn phys_to_rect(r: layout::PhysRect) -> Rect {
    Rect {
        position: PhysicalPosition::new(r.x, r.y).into(),
        size: PhysicalSize::new(r.w.max(1), r.h.max(1)).into(),
    }
}

impl App {
    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.active)
    }
    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The single tiling computation (AUDIT V14-4): device-pixel
    /// rects from `pico_core::core::layout`, the only place logical
    /// chrome height becomes physical pixels.
    fn tiling(&self) -> (layout::PhysRect, layout::PhysRect) {
        let size = self.window.inner_size();
        layout::tile(size.width, size.height, self.window.scale_factor())
    }

    fn content_rect(&self) -> Rect {
        let (_, content) = self.tiling();
        phys_to_rect(content)
    }

    /// Either centered panel (Teleport's or the theme
    /// picker's) extends the chrome rect; they are mutually
    /// exclusive, so one bool covers both (AUDIT V27-A2).
    fn any_panel_open(&self) -> bool {
        self.teleport_panel_open || self.palette_panel_open
    }

    fn chrome_rect(&self) -> Rect {
        let size = self.window.inner_size();
        phys_to_rect(layout::chrome_rect_with_panel(
            size.width,
            size.height,
            self.window.scale_factor(),
            self.any_panel_open(),
        ))
    }

    /// One layout pass: chrome on top, active content below it. The
    /// two rects come from the single device-pixel tiling in
    /// core::layout: they overlap by exactly the feather (12 DIP),
    /// so the content is already in place beneath the chrome's
    /// transparent fade — no unpainted seam can appear.
    fn layout(&mut self) {
        // AUDIT V26 (D2): the layout witness. The v25 black-band
        // defect shipped with no geometry evidence in pico.log;
        // from v26 the applied rects are on record whenever they
        // change, so a recurrence is diagnosable from the log
        // alone: window inner size, scale, chrome rect, content
        // rect, panel state.
        {
            let inner = self.window.inner_size();
            let scale = self.window.scale_factor();
            let cr = layout::chrome_rect_with_panel(
                inner.width,
                inner.height,
                scale,
                self.any_panel_open(),
            );
            let (_, xr) = layout::tile(inner.width, inner.height, scale);
            let sig = format!(
                "layout: win={}x{} scale={:.2} chrome=({},{},{}x{}) content=({},{},{}x{}) panel={}",
                inner.width,
                inner.height,
                scale,
                cr.x,
                cr.y,
                cr.w,
                cr.h,
                xr.x,
                xr.y,
                xr.w,
                xr.h,
                if self.teleport_panel_open {
                    "teleport"
                } else if self.palette_panel_open {
                    "palette"
                } else {
                    "closed"
                },
            );
            if sig != self.last_layout_sig {
                log::info(&sig);
                self.last_layout_sig = sig;
            }
        }
        let _ = self.chrome.set_bounds(self.chrome_rect());
        // The feather overlap only renders if the chrome composites
        // above the content in the shared rows (AUDIT V16-1).
        raise_chrome_above_content(&self.chrome);
        if let Some(tab) = self.active_tab() {
            if let Some(view) = tab.view() {
                let rect = self.content_rect();
                let _ = view.set_bounds(rect);
            }
        }
    }

    /// Build one content WebView for a tab. On failure the error is
    /// returned (AUDIT M6) — creating a tab must never kill the app.
    fn build_content_view(
        &mut self,
        id: u64,
        url: &str,
        // Kept for signature stability with the call sites; the
        // handler's start-tab classification is LIVE since v26
        // (see live_start below), not this build-time fact.
        _is_start: bool,
    ) -> Result<WebView, String> {
        let proxy = self.proxy.clone();
        let nav_proxy = proxy.clone();
        let nav_shield = self.shield.clone();
        let load_proxy = proxy.clone();
        let title_proxy = proxy.clone();
        let drop_proxy = proxy.clone();
        let win_proxy = proxy.clone();
        let note_proxy = proxy.clone();
        let action_proxy = proxy.clone();
        let download_proxy = proxy.clone();
        // AUDIT V26 (D5): the navigation handler classifies
        // "is this the start tab" LIVE from this set, not from
        // the build-time flag — a start tab that converts in
        // place leaves the set and its handler follows.
        let live_start = self.live_start_tabs.clone();
        // AUDIT 0.31 (A6-F2): the one-shot credentialed-nav
        // expectation, consumed by this handler (see the
        // `expected_nav` field on App).
        let expected_nav = self.expected_nav.clone();

        let rect = self.content_rect();
        let protocol = pico_protocol_handler(self.shared.clone());
        let builder = WebViewBuilder::new_with_web_context(&mut self.web_ctx)
            .with_url(url)
            // Explicit desktop-Chrome-form user agent (AUDIT
            // V18-1, form changed by V22-14), composed at startup
            // from the installed runtime's version; no Pico token
            // in it, ever.
            .with_user_agent(self.user_agent.clone())
            // Explicit themed background (AUDIT V14-2): before the
            // page paints — and if it ever fails to — the surface is
            // the theme color, never unpainted black.
            .with_background_color(theme_bg_rgba(&self.persist.theme_id, &self.persist.theme))
            .with_custom_protocol("pico".to_string(), protocol)
            .with_initialization_script_for_main_only(NAV_OBSERVER_SCRIPT, true)
            .with_ipc_handler(move |request: Request<String>| {
                let body = request.body();
                // Start-page actions (AUDIT V17-1) share this IPC
                // channel with navigation notes; the two families
                // are disjoint by their `kind` field. An action
                // body is forwarded undecided — the gate lives in
                // `core::action::dispatch`, applied centrally.
                if looks_like_action(body) {
                    let _ = action_proxy.send_event(UserEvent::StartAction {
                        tab: id,
                        source: ActionSource::Ipc(body.clone()),
                    });
                    return;
                }
                // Shield prune reports (AUDIT V23) share the
                // channel too: disjoint by `kind`, parsed under
                // strict caps in core, and worth only log lines.
                if looks_like_shield_prune(body) {
                    if let Some(items) = parse_shield_prune(body) {
                        if !items.is_empty() {
                            let _ =
                                note_proxy.send_event(UserEvent::ShieldPrune { tab: id, items });
                        }
                    }
                    return;
                }
                // Lite chosen reports (AUDIT V29) share the
                // channel too: disjoint by `kind`, payload-free,
                // validated in core; the app-side handler applies
                // the sender checks before anything is recorded.
                if looks_like_lite_chosen(body) {
                    if parse_lite_chosen(body) {
                        let _ = note_proxy.send_event(UserEvent::LiteChosen { tab: id });
                    }
                    return;
                }
                // Measured prune reports (0.30) share the
                // channel too: disjoint by `kind`, exact-shape
                // validated in core (deny-unknown-fields, hard
                // bounds); the app-side handler applies the
                // sender checks and the book's rate gates
                // before a single figure moves.
                if looks_like_shield_pruned(body) {
                    if let Some(report) = parse_shield_pruned(body) {
                        let _ = note_proxy.send_event(UserEvent::ShieldPruned {
                            tab: id,
                            bytes: report.bytes,
                            events: report.events,
                        });
                    }
                    return;
                }
                if let Some(note) = parse_nav_note(body) {
                    let _ = note_proxy.send_event(UserEvent::NavNote { tab: id, note });
                }
            })
            .with_navigation_handler(move |raw_url: String| -> bool {
                let url = normalize_pico_url(&raw_url);
                let is_start = live_start
                    .lock()
                    .map(|set| set.contains(&id))
                    .unwrap_or(false);
                // Shield page-state (AUDIT V21): WebView2 raises
                // NavigationStarting for the MAIN frame only, so
                // every URL seen here is a main-frame target —
                // record it for the request handler's main-frame
                // determination and party context.
                if url.starts_with("http://") || url.starts_with("https://") {
                    nav_shield.set_pending_main(id, &url);
                }
                // Start-page action links (the legacy transport —
                // AUDIT V17-1: clicks now travel by IPC, anchors
                // remain as the fallback) are forwarded UNDECIDED
                // to the single dispatch in `core::action`. This
                // handler only classifies: an action URL from the
                // start tab becomes a StartAction event; from any
                // other webview it is a flat denial — privileged UI
                // gestures are not web content's business.
                if is_action_url(&url) {
                    if is_start {
                        let _ = nav_proxy.send_event(UserEvent::StartAction {
                            tab: id,
                            source: ActionSource::Url(url),
                        });
                    }
                    return false;
                }
                if is_start {
                    if is_web_url(&url) {
                        // Start-page web links (service cards whose
                        // IPC path was unavailable) go through the
                        // same dispatch; the start page itself never
                        // navigates away.
                        let _ = nav_proxy.send_event(UserEvent::StartAction {
                            tab: id,
                            source: ActionSource::Url(url),
                        });
                        return false;
                    }
                    // The start tab may (re)load only the start page.
                    return url == START_URL || url == "pico://start/";
                }
                // AUDIT 0.31 (A6-F2): same-tab credentialed
                // navigations. Through 0.30 only popup/new-window
                // credentialed URLs were denied — a page could
                // navigate its OWN tab to
                // `https://google.com:443@evil.example/…` and the
                // spoofed prefix landed in the address bar. Now:
                // the tab's one-shot shell expectation is
                // consumed by this first navigation event
                // (matched or not), and a userinfo URL proceeds
                // only on an exact match — the URL the user
                // themselves typed (core::ipc::userinfo_nav_allowed
                // holds the decision table).
                let expected_for_tab = {
                    let mut slot = expected_nav.lock().unwrap_or_else(|p| p.into_inner());
                    match slot.as_ref() {
                        Some((tab_id, _)) if *tab_id == id => slot.take().map(|(_, u)| u),
                        _ => None,
                    }
                };
                if !userinfo_nav_allowed(&url, expected_for_tab.as_deref()) {
                    crate::log::info(&format!(
                        "navigation denied (credentialed URL not typed by the user): tab={id} {}",
                        redact_url(&url)
                    ));
                    return false;
                }
                // Everywhere else: the strict scheme allowlist.
                // javascript:, data:, file:, view-source:, res:, …
                // are all refused as document navigations. Local
                // files reach a tab only as static-server http URLs.
                is_allowed_navigation_url(&url) && !url.starts_with("pico://")
            })
            .with_download_started_handler(move |uri: String, _path: &mut PathBuf| -> bool {
                // Pico has no downloads manager: deny every download
                // (no drive-by file drops) and surface it instead.
                let _ = download_proxy.send_event(UserEvent::DownloadBlocked { tab: id, uri });
                false
            })
            .with_on_page_load_handler(move |event: PageLoadEvent, raw_url: String| {
                let _ = load_proxy.send_event(UserEvent::PageLoad {
                    tab: id,
                    finished: matches!(event, PageLoadEvent::Finished),
                    url: normalize_pico_url(&raw_url),
                });
            })
            .with_document_title_changed_handler(move |title: String| {
                let _ = title_proxy.send_event(UserEvent::TitleChanged { tab: id, title });
            })
            .with_drag_drop_handler(move |ev: DragDropEvent| -> bool {
                if let DragDropEvent::Drop { paths, .. } = ev {
                    if !paths.is_empty() {
                        let _ = drop_proxy.send_event(UserEvent::DroppedPaths(paths));
                    }
                }
                // Claim every phase so WebView2 never navigates to a
                // dropped file:// itself.
                true
            })
            // Sign-in flows and window.open (AUDIT V22-10/15): the
            // handler forwards the target undecided; the event
            // loop takes the pure routing decision in
            // `core::addr::route_new_window`. Deny here only stops
            // WebView2 from spawning its own window — the tab, when
            // the decision allows one, is created by the event arm.
            .with_new_window_req_handler(move |url, _features| {
                let _ = win_proxy.send_event(UserEvent::NewWindow {
                    tab: id,
                    url: normalize_pico_url(&url),
                });
                NewWindowResponse::Deny
            })
            .with_devtools(true)
            .with_bounds(rect)
            .with_visible(false);
        // Pin the WebView2 browser arguments (AUDIT V18-2). The
        // pinned string is byte-identical to the default wry
        // substituted in v17 — the change is that it is now Pico's
        // own tested constant, not a framework default nobody
        // here can see. It must be set on EVERY builder (chrome
        // included): all WebViews share one user-data folder, and
        // environments over one folder must agree on options.
        #[cfg(windows)]
        let builder = {
            use wry::WebViewBuilderExtWindows;
            builder.with_additional_browser_args(pico_core::core::media::PICO_BROWSER_ARGS)
        };
        // Shield document script (AUDIT V21, contents V23):
        // when the compiled script exists and the Shield is on,
        // it rides as a document-start script on every frame of
        // this content view (never the chrome view). The script
        // itself no-ops on non-http(s) and loopback pages. Tabs
        // built before compilation finished get the same script
        // pushed on each load instead (see on_page_load).
        let builder = if self.shield.on.load(std::sync::atomic::Ordering::Relaxed) {
            match self.shield.document_script() {
                Some(script) => {
                    builder.with_initialization_script_for_main_only(script.as_str(), false)
                }
                None => builder,
            }
        } else {
            builder
        };
        // Lite play gate (AUDIT V29, A2-F4): when Lite is on at
        // build, the gate ALSO rides as a document-start script
        // — main frame only (it is a top-document law; subframe
        // players are the documented A2-F10 limitation), chained
        // after the NAV observer so the gate's route hooks wrap
        // the observer's. Through v28 the gate reached a page
        // only via evaluate_script at PageLoad Finished, after
        // the player had already attempted its first play() and
        // its first segment fetches had been decided; the
        // Finished and toggle pushes remain as the re-arm path
        // for tabs built while Lite was off. The runtime itself
        // no-ops on non-http(s) and loopback pages, so start
        // tabs and dev pages are untouched.
        let builder = if self.lite.on.load(std::sync::atomic::Ordering::Relaxed) {
            builder.with_initialization_script_for_main_only(
                pico_core::core::filterpipe::LITE_GATE_RUNTIME,
                true,
            )
        } else {
            builder
        };
        let view = builder
            .build_as_child(&*self.window)
            .map_err(|e| e.to_string())?;
        harden_webview_profile(&view, "content");
        attach_media_probe(&view, id, self.probe_budget.clone(), self.stats.clone());
        attach_shield(
            &view,
            id,
            self.shield.clone(),
            self.lite.clone(),
            self.stats.clone(),
            self.proxy.clone(),
        );
        Ok(view)
    }

    fn add_tab(&mut self, url: &str, is_start: bool, activate: bool) -> u64 {
        if self.tabs.len() >= MAX_TABS {
            log::error(&format!(
                "tab cap ({MAX_TABS}) reached; refusing to open {}",
                redact_url(url)
            ));
            self.push_state(false);
            return 0;
        }
        if !is_allowed_navigation_url(url) {
            log::error(&format!(
                "refusing to open disallowed URL: {}",
                redact_url(url)
            ));
            self.push_state(false);
            return 0;
        }
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        // Was the Shield's cosmetic script available as a
        // document-start script for this build? (Same predicate
        // build_content_view applies; AUDIT V21.)
        let doc_start_script = self.shield.on.load(std::sync::atomic::Ordering::Relaxed)
            && self.shield.document_script().is_some();
        // AUDIT 0.31 (A6-F2): a new tab's initial load is
        // shell-initiated — record the expectation BEFORE the
        // view is built (the navigation handler fires for the
        // initial load during construction). Every caller of
        // add_tab is a shell/user-gesture path (typed input,
        // argv, gated start-page dispatch, constants); the one
        // page-initiated caller — the new-window event — is
        // pre-gated by route_new_window, which already denies
        // credentialed URLs, so this can never bless one.
        self.expect_nav(id, url);
        let (view, failed) = match self.build_content_view(id, url, is_start) {
            Ok(v) => (Some(v), None),
            Err(why) => {
                log::error(&format!(
                    "content webview creation failed for tab {id} ({}): {why}",
                    redact_url(url)
                ));
                (None, Some(why))
            }
        };
        let failed_flag = failed.is_some();
        let title = if is_start {
            "Start".to_string()
        } else if failed_flag {
            "Couldn't load".to_string()
        } else {
            "Loading…".to_string()
        };
        if is_start {
            self.live_start_tabs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(id);
        }
        self.tabs.push(Tab {
            id,
            view,
            failed,
            is_start,
            url: url.to_string(),
            urlstate: UrlState::with_url(url),
            title,
            loading: !failed_flag,
            root_id: self.server.root_id_for_url(url),
            root_held: false,
            hist: TabHistory::new(),
            last_load_start: Some(Instant::now()),
            doc_start_script,
            commit_started: false,
        });
        // Seed the Shield's page-state for this tab: the initial
        // URL is both the pending main-frame target and (until the
        // first commit says otherwise) the page context.
        self.shield.set_pending_main(id, url);
        self.shield.set_page_url(id, url);
        if activate {
            self.activate(id, false);
        }
        self.push_state(false);
        id
    }

    /// Retry creating the WebView of a failed tab (Reload on it).
    fn retry_tab_view(&mut self, id: u64) {
        let Some(idx) = self.tab_index(id) else {
            return;
        };
        if self.tabs[idx].view.is_some() {
            return;
        }
        let url = self.tabs[idx].url.clone();
        let is_start = self.tabs[idx].is_start;
        match self.build_content_view(id, &url, is_start) {
            Ok(v) => {
                let tab = &mut self.tabs[idx];
                tab.view = Some(v);
                tab.failed = None;
                tab.loading = true;
                tab.last_load_start = Some(Instant::now());
                tab.doc_start_script = self.shield.on.load(std::sync::atomic::Ordering::Relaxed)
                    && self.shield.document_script().is_some();
                log::info(&format!("tab {id} webview created on retry"));
            }
            Err(why) => {
                log::error(&format!("tab {id} webview retry failed: {why}"));
                self.tabs[idx].failed = Some(why);
            }
        }
        let is_active = self.active == id;
        if is_active {
            self.layout();
            if let Some(tab) = self.active_tab() {
                if let Some(v) = tab.view() {
                    let _ = v.set_visible(true);
                }
            }
        }
        self.push_state(false);
    }

    /// Move a tab's root pointer, releasing the old root first when
    /// this tab held a reference on it (AUDIT M2). The new root is
    /// *borrowed* (reload scoping only) until explicitly held.
    fn set_tab_root(&mut self, idx: usize, new_root: Option<u64>) {
        let old_root = self.tabs[idx].root_id;
        let old_held = self.tabs[idx].root_held;
        if old_root == new_root {
            return;
        }
        if old_held {
            if let Some(r) = old_root {
                self.server.release_root(r);
                self.note_root_maybe_gone(r);
            }
        }
        self.tabs[idx].root_id = new_root;
        self.tabs[idx].root_held = false;
    }

    /// When a root's last reference is gone it leaves the server; its
    /// file watcher (if any) goes with it.
    fn note_root_maybe_gone(&mut self, root_id: u64) {
        if self.server.root_refs(root_id).is_none() && self.watchers.remove(&root_id).is_some() {
            log::info(&format!(
                "root #{root_id} released by last tab; watcher dropped"
            ));
        }
    }

    fn activate(&mut self, id: u64, focus_content: bool) {
        if !self.tabs.iter().any(|t| t.id == id) {
            return;
        }
        // A Teleport payload belongs to the tab it was computed
        // for (v25): switching tabs invalidates it, and the
        // chrome closes the panel on the active-tab change.
        if id != self.active {
            self.teleport = None;
            self.teleport_version += 1;
            self.teleport_panel_open = false;
            // The picker closes on tab switches too (v27): its
            // local UI dismisses on the active change, and the
            // app state must agree or the chrome rect would stay
            // extended under a closed panel.
            self.palette_panel_open = false;
        }
        self.active = id;
        for t in &self.tabs {
            if let Some(v) = t.view() {
                let _ = v.set_visible(t.id == id);
            }
        }
        self.layout();
        if focus_content {
            if let Some(tab) = self.active_tab() {
                if let Some(v) = tab.view() {
                    let _ = v.focus();
                }
            }
        }
        self.push_state(false);
    }

    fn close_tab(&mut self, id: u64) {
        let Some(idx) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let was_active = self.active == id;
        self.shield.forget_tab(id);
        self.lite.clear_chosen(id);
        self.stats.forget_tab(id);
        self.live_start_tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        let removed = self.tabs.remove(idx); // dropping the WebView closes it
        if removed.root_held {
            if let Some(r) = removed.root_id {
                self.server.release_root(r);
                self.note_root_maybe_gone(r);
            }
        }
        drop(removed);
        if self.tabs.is_empty() {
            self.add_tab(START_URL, true, true);
            return;
        }
        if was_active {
            let next = self.tabs[idx.min(self.tabs.len() - 1)].id;
            self.activate(next, true);
        }
        self.push_state(false);
    }

    fn cycle_tab(&mut self, dir: i32) {
        if self.tabs.len() < 2 {
            return;
        }
        let Some(idx) = self.tabs.iter().position(|t| t.id == self.active) else {
            return;
        };
        let n = self.tabs.len() as i32;
        let next = ((idx as i32 + dir).rem_euclid(n)) as usize;
        let id = self.tabs[next].id;
        self.activate(id, true);
    }

    /// Push the UI state snapshot to the chrome page. Signature-gated:
    /// if the snapshot's content is unchanged, nothing is sent.
    ///
    /// AUDIT V28 A1-M1: the gate used to be the serialized JSON
    /// itself — every call cloned the themes catalogue and the
    /// Teleport QR matrix and serialized the whole snapshot just
    /// to discover nothing changed, and block events call this
    /// per blocked request. The gate is now a cheap hash over the
    /// primitive fields, computed BEFORE any construction; the
    /// snapshot (and its clones) is built only when the push will
    /// actually be sent, when forced, or when the one-shot focus
    /// hint rides it.
    fn push_state(&mut self, force: bool) {
        use std::hash::{Hash, Hasher};
        let (address, can_back, can_forward, active_error) = match self.active_tab() {
            Some(t) => (
                self.display_address(t),
                t.hist.can_go_back(),
                t.hist.can_go_forward(),
                t.failed.clone().unwrap_or_default(),
            ),
            None => (String::new(), false, false, String::new()),
        };
        let shield_on = self.shield.on.load(std::sync::atomic::Ordering::Relaxed);
        let blocked_total = self
            .shield
            .session_blocked
            .load(std::sync::atomic::Ordering::Relaxed);
        let blocked_tab = self.shield.tab_blocked(self.active);
        let lite_on = self.lite.on.load(std::sync::atomic::Ordering::Relaxed);
        let lite_blocked = self
            .lite
            .session_blocked
            .load(std::sync::atomic::Ordering::Relaxed);
        // The cheap signature: every field the snapshot carries,
        // hashed from primitives. The themes catalogue is a
        // process constant (excluded); the Teleport payload is
        // represented by its version counter.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for t in &self.tabs {
            t.id.hash(&mut hasher);
            t.title.hash(&mut hasher);
            t.url.hash(&mut hasher);
            t.loading.hash(&mut hasher);
            t.is_start.hash(&mut hasher);
            t.failed.is_some().hash(&mut hasher);
        }
        self.active.hash(&mut hasher);
        self.persist.theme.hash(&mut hasher);
        self.persist.theme_id.hash(&mut hasher);
        self.palette_panel_open.hash(&mut hasher);
        self.teleport_panel_open.hash(&mut hasher);
        self.teleport_version.hash(&mut hasher);
        address.hash(&mut hasher);
        can_back.hash(&mut hasher);
        can_forward.hash(&mut hasher);
        self.services_count.hash(&mut hasher);
        self.blocked_downloads.hash(&mut hasher);
        self.download_notice.hash(&mut hasher);
        active_error.hash(&mut hasher);
        shield_on.hash(&mut hasher);
        blocked_total.hash(&mut hasher);
        blocked_tab.hash(&mut hasher);
        lite_on.hash(&mut hasher);
        lite_blocked.hash(&mut hasher);
        let sig = hasher.finish();
        // The hint is one-shot (AUDIT V26 D4): it rides exactly
        // one snapshot, and a gated-out push would strand it — so
        // a set hint forces the send, exactly as before.
        let focus_hint = self.pending_focus_addr;
        if !force && !focus_hint && sig == self.last_snap_sig {
            return;
        }
        self.pending_focus_addr = false;
        self.last_snap_sig = sig;
        let snap = Snap {
            tabs: self
                .tabs
                .iter()
                .map(|t| TabSnap {
                    id: t.id,
                    title: t.title.clone(),
                    url: t.url.clone(),
                    loading: t.loading,
                    start: t.is_start,
                    failed: t.failed.is_some(),
                })
                .collect(),
            active: self.active,
            theme: self.persist.theme.clone(),
            palette: self.persist.theme_id.clone(),
            themes: themes_snap(),
            palette_open: self.palette_panel_open,
            teleport_open: self.teleport_panel_open,
            address,
            can_back,
            can_forward,
            services: self.services_count,
            blocked_downloads: self.blocked_downloads,
            download_notice: self.download_notice.clone(),
            active_error,
            shield_on,
            blocked_total,
            blocked_tab,
            lite_on,
            lite_blocked,
            teleport: self.teleport.clone(),
            focus_addr: focus_hint,
            version: DISPLAY_VERSION,
        };
        let json = serde_json::to_string(&snap).unwrap_or_else(|_| "{}".to_string());
        let _ = self
            .chrome
            .evaluate_script(&format!("window.__pico&&window.__pico({json})"));
    }

    /// What the address bar shows for a tab: the file path for locally
    /// served pages (friendlier than the 127.0.0.1 URL), blank for the
    /// start page, the URL otherwise. Derived from the server's
    /// structured URL parse (AUDIT L14), never substring splitting.
    fn display_address(&self, tab: &Tab) -> String {
        if tab.is_start {
            return String::new();
        }
        if let Some((root_id, rel)) = self.server.url_parts(&tab.url) {
            if let Some(root) = self.server.root_path(root_id) {
                if rel.is_empty() {
                    return clean_display_path(&root);
                }
                let full = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                return clean_display_path(&full);
            }
        }
        tab.url.clone()
    }

    fn sync_shared(&self) {
        let mut data = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.theme = self.persist.theme.clone();
        data.palette = self.persist.theme_id.clone();
        data.recents = self.persist.recents.clone();
        data.recents_hidden = self.persist.recents_hidden;
        data.stats = self.stats.view();
    }

    fn push_services_to_start_tabs(&mut self) {
        let json = {
            let data = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            serde_json::to_string(&data.services).unwrap_or_else(|_| "[]".to_string())
        };
        for t in &self.tabs {
            if t.is_start {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoSetServices&&window.__picoSetServices({json})"
                    ));
                }
            }
        }
    }

    /// Push the "other listeners" aggregate to live start tabs
    /// (AUDIT V14-5): one muted count line, never per-port cards.
    fn push_others_to_start_tabs(&mut self) {
        let n = self.others_count;
        for t in &self.tabs {
            if t.is_start {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoSetOthers&&window.__picoSetOthers({n})"
                    ));
                }
            }
        }
    }

    fn push_recents_to_start_tabs(&mut self) {
        let json =
            serde_json::to_string(&self.persist.recents).unwrap_or_else(|_| "[]".to_string());
        for t in &self.tabs {
            if t.is_start {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoSetRecents&&window.__picoSetRecents({json})"
                    ));
                }
            }
        }
    }

    /// Push the Recents section's visibility to live start tabs
    /// (v24): the bool is a literal from our own state, safe to
    /// interpolate into the hook call.
    fn push_recents_hidden_to_start_tabs(&mut self) {
        let hidden = self.persist.recents_hidden;
        for t in &self.tabs {
            if t.is_start {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoSetRecentsHidden&&window.__picoSetRecentsHidden({hidden})"
                    ));
                }
            }
        }
    }

    /// Push the savings band's figures to live start tabs
    /// (0.30). Signature-gated like the chrome snapshot: the
    /// view is hashed first and the JSON is built only when a
    /// figure actually moved, so a block flood costs one hash
    /// per event, not a serialization per request. The band
    /// exists ONLY on the start page — no chrome surface ever
    /// receives these numbers.
    fn push_stats_to_start_tabs(&mut self, force: bool) {
        let sig = self.stats.view_signature();
        if !force && sig == self.last_stats_sig {
            return;
        }
        self.last_stats_sig = sig;
        let view = self.stats.view();
        {
            let mut data = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            data.stats = view;
        }
        let json = serde_json::to_string(&view).unwrap_or_else(|_| "{}".to_string());
        for t in &self.tabs {
            if t.is_start {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoSetStats&&window.__picoSetStats({json})"
                    ));
                }
            }
        }
    }

    fn push_theme_to_start_tabs(&mut self) {
        // Mode and palette both come from closed sets
        // ("light"/"dark", the palette table) — safe to place in
        // a JS string literal directly.
        // AUDIT 0.31 (A3-F10): that safety rests entirely on
        // upstream sanitization, so re-check the closed sets AT
        // the sink: a value outside them (a future state-load
        // regression, a hand-edited state.json that slipped a
        // quote through) must never reach the JS literal.
        let theme = self.persist.theme.clone();
        let palette = self.persist.theme_id.clone();
        if !matches!(theme.as_str(), "dark" | "light")
            || !pico_core::core::themes::PALETTES
                .iter()
                .any(|p| p.id == palette)
        {
            // (The values are only logged — the log writer
            // strips control/format characters — never
            // interpolated anywhere.)
            log::error(&format!(
                "theme push refused: mode/palette outside the closed sets (mode={theme}, palette={palette})"
            ));
            return;
        }
        for t in &self.tabs {
            if t.is_start {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoSetTheme&&window.__picoSetTheme('{theme}','{palette}')"
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Paths, navigation, watcher (App)
// ---------------------------------------------------------------------------

/// File paths go into the address bar as the plain OS path — no file://
/// form, no backslash noise.
fn clean_display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    #[cfg(windows)]
    {
        s.trim_start_matches(r"\\?\").to_string()
    }
    #[cfg(not(windows))]
    {
        s.into_owned()
    }
}

impl App {
    fn save_state(&mut self) {
        // The Shield's persisted fields ride along (AUDIT V21):
        // the toggle mirrors the runtime flag. 0.30: the
        // savings book is the lifetime authority — its figures
        // mirror into `persist.stats`, and the legacy
        // `blocked_total` is kept equal to the book's Shield
        // count so a downgrade-era reader still sees the true
        // lifetime (and a pre-0.30 file's value seeds the book
        // at startup, once).
        self.persist.shield_on = self.shield.on.load(std::sync::atomic::Ordering::Relaxed);
        self.persist.lite_on = self.lite.on.load(std::sync::atomic::Ordering::Relaxed);
        self.persist.stats = self.stats.lifetime();
        self.persist.blocked_total = self.persist.stats.shield.blocked;
        if let Err(e) = pstore::save(&self.persist) {
            log_save_failure(&e);
        }
        self.sync_shared();
    }

    /// The Shield's lists finished compiling (AUDIT V21): pages
    /// that loaded before the cosmetic script existed get it
    /// pushed now (later loads of doc-start-less tabs are handled
    /// in on_page_load), and the shell learns the blocked counts.
    fn on_shield_ready(&mut self) {
        log::info("shield: engine active");
        if self.shield.on.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(script) = self.shield.document_script() {
                for t in &self.tabs {
                    if t.doc_start_script || t.is_start {
                        continue;
                    }
                    if let Some(v) = t.view() {
                        let _ = v.evaluate_script(&script);
                    }
                }
            }
        }
        self.push_state(true);
    }

    /// Scriptlet prune reports (AUDIT V23): log a capped sample
    /// — `shield prune: host=<page host> path=<p> n=<count>` —
    /// so pico.log carries positive proof the scriptlet layer
    /// stripped ad data. The items come from page context and
    /// are otherwise worthless by design: they move no counter
    /// and gate nothing (a page could forge them; the log is a
    /// diagnostic, not a ledger).
    fn on_shield_prune(&mut self, tab: u64, items: &[PruneItem]) {
        let (committed, pending) = self.shield.page_state(tab);
        let page = if committed.is_empty() {
            &pending
        } else {
            &committed
        };
        let host = pico_core::core::shield::host_of(page).unwrap_or_default();
        for item in items {
            if !self.shield.claim_prune_log() {
                break;
            }
            log::info(&format!(
                "shield prune: host={host} path={} n={}",
                item.path, item.count
            ));
        }
    }

    /// A tab's Lite gate designated its chosen video
    /// (AUDIT V29). The report is a payload-free hint from page
    /// context — any page can post it — so it is honored only
    /// when the sender is exactly what the gate runs on: a live
    /// content tab (never the start tab, whose privileged
    /// channel this is not), currently on a PUBLIC origin, with
    /// Lite on. The flag's only effect is as a `filterpipe`
    /// fact; the first designation per document logs one line.
    fn on_lite_chosen(&mut self, tab: u64) {
        if !self.lite.on.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else {
            return; // stale event for a closed tab
        };
        if t.is_start || t.view().is_none() {
            return;
        }
        if pico_core::core::filterpipe::classify_origin(&t.url)
            != pico_core::core::filterpipe::OriginClass::Public
        {
            return;
        }
        if self.lite.mark_chosen(tab) && self.lite.claim_chosen_log() {
            let host = pico_core::core::shield::host_of(&t.url).unwrap_or_default();
            log::info(&format!("lite chosen: tab={tab} host={host}"));
        }
    }

    /// A measured prune report (0.30). The figures come from
    /// page context — a page can post this IPC directly — so
    /// the sender discipline matches `on_lite_chosen` exactly
    /// (a live, public, non-start content tab, the relevant
    /// engine ON), and the book applies its own gates on top:
    /// per-tab rate floor, session byte ceiling, payload
    /// bounds. A report that passes moves ONLY the pruned
    /// figures (never the blocked counts, never the ≈
    /// estimates); accepted reports log a capped sample.
    fn on_shield_pruned(&mut self, tab: u64, bytes: u64, events: u64) {
        if !self.shield.on.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else {
            return; // stale event for a closed tab
        };
        if t.is_start || t.view().is_none() {
            return;
        }
        if pico_core::core::filterpipe::classify_origin(&t.url)
            != pico_core::core::filterpipe::OriginClass::Public
        {
            return;
        }
        if self.stats.record_prune(tab, bytes, events, stats_now_ms()) {
            if self.stats.claim_prune_log() {
                let host = pico_core::core::shield::host_of(&t.url).unwrap_or_default();
                log::info(&format!(
                    "shield pruned: tab={tab} host={host} bytes={bytes} events={events}"
                ));
            }
            self.push_stats_to_start_tabs(false);
        }
    }

    /// Flip the Shield (AUDIT V21). Turning off is immediate for
    /// the network layer (a flag the handlers read per request)
    /// and best-effort for the in-page layers already applied to
    /// live documents: the injected stylesheet is removed and
    /// the scriptlet wrappers are disarmed through the one
    /// atomic in-page flag they all consult (`__picoShieldOff`,
    /// AUDIT V23 — the OFF law covers all three layers). New
    /// documents get no payload at all. The choice persists via
    /// save_state.
    fn toggle_shield(&mut self) {
        let now = !self.shield.on.load(std::sync::atomic::Ordering::Relaxed);
        self.shield
            .on
            .store(now, std::sync::atomic::Ordering::Relaxed);
        log::info(&format!("shield: {}", if now { "on" } else { "off" }));
        // AUDIT V28 (A2-M7): the collapse layer's arming flag is
        // maintained on toggles too, not only at load
        // boundaries — through v27 a mid-session toggle left
        // live documents in the previous posture until the next
        // navigation. both_off is computed AFTER the flip.
        let lite_on = self.lite.on.load(std::sync::atomic::Ordering::Relaxed);
        let both_off = !now && !lite_on;
        if !now {
            for t in &self.tabs {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(
                        "window.__picoShieldOff=1;\
                         var e=document.getElementById('pico-shield-style');\
                         if(e)e.remove();window.__picoShieldCosmetic=0;",
                    );
                    if both_off {
                        let _ = v.evaluate_script("window.__picoCollapseOff=1;");
                    }
                }
            }
        } else if let Some(script) = self.shield.document_script() {
            // Re-arm live documents: clear the OFF flag first
            // (pages that kept their wrappers wake up), then push
            // the document script (idempotent per document; the
            // off path reset the cosmetic guard flag). Future
            // documents of doc-start tabs re-run it automatically.
            for t in &self.tabs {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script("window.__picoShieldOff=0;");
                    let _ = v.evaluate_script("window.__picoCollapseOff=0;");
                    let _ = v.evaluate_script(&script);
                }
            }
        }
        self.save_state();
    }

    /// Flip Lite Mode (v25). The flag is read per request by
    /// the shared handler, so the network flip is immediate for
    /// every tab; already-loaded heavy resources are not
    /// retroactively removed (a reload shows the full effect).
    /// Persists via save_state, like the Shield's flag.
    ///
    /// v28: the DOCUMENT layers follow the live toggle too
    /// (the hover-preview fix's state sync, plus AUDIT A2-M7).
    /// Turning on pushes the combined Lite document script
    /// (collapse layer + play gate) into every live content
    /// view — idempotent per document: pages that already have
    /// the gate re-arm and sweep, pages that never had it
    /// install it. Turning off sets the gate's disarm flag on
    /// live documents (the play() wrapper passes through
    /// untouched from that line on) and, when the Shield is off
    /// too, disarms the collapse layer. The sync is logged —
    /// when a hover preview is reported on-device, pico.log
    /// answers whether the gate was ever pushed.
    fn toggle_lite(&mut self) {
        let now = !self.lite.on.load(std::sync::atomic::Ordering::Relaxed);
        self.lite
            .on
            .store(now, std::sync::atomic::Ordering::Relaxed);
        // AUDIT V29: designations reset on Lite toggle — the
        // gate resets its own state on re-arm, and the app-side
        // flags clear in step, in both directions.
        self.lite.clear_all_chosen();
        let shield_on = self.shield.on.load(std::sync::atomic::Ordering::Relaxed);
        // AUDIT V29 (A6-L8): the sync count names only the
        // views the document layers can actually arm — public
        // content tabs. Through v28 the line also counted the
        // start tab and loopback pages, where both runtimes
        // early-return by law, overstating the sync it reports.
        let syncable = |t: &Tab| {
            !t.is_start
                && pico_core::core::filterpipe::classify_origin(&t.url)
                    == pico_core::core::filterpipe::OriginClass::Public
        };
        let mut synced = 0usize;
        if now {
            let script = pico_core::core::filterpipe::lite_document_script();
            for t in &self.tabs {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script("window.__picoCollapseOff=0;");
                    let _ = v.evaluate_script(&script);
                    if syncable(t) {
                        synced += 1;
                    }
                }
            }
        } else {
            for t in &self.tabs {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script("window.__picoLiteGateOff=1;");
                    if !shield_on {
                        let _ = v.evaluate_script("window.__picoCollapseOff=1;");
                    }
                    if syncable(t) {
                        synced += 1;
                    }
                }
            }
        }
        log::info(&format!(
            "lite: {} (document layers synced to {synced} live views)",
            if now { "on" } else { "off" }
        ));
        self.save_state();
    }

    /// Phone Teleport (v25): resolve the active tab's URL for the
    /// phone and stash the payload for the next snapshot. Pure
    /// resolution lives in `core::teleport`; this method only
    /// supplies the platform facts (the tab URL, whether Pico's
    /// own server serves it, the LAN candidates) and logs the
    /// single mandated line with the host redacted per log laws.
    fn request_teleport(&mut self) {
        use pico_core::core::teleport::{choose_lan_ip_with_egress, resolve};
        // The two centered panels are mutually exclusive (v27):
        // asking for Teleport retires the picker.
        self.palette_panel_open = false;
        let Some(tab) = self.active_tab() else {
            return;
        };
        let url = tab.url.clone();
        let is_start = tab.is_start;
        // Pico-served pages are the ones the built-in server
        // serves. AUDIT 0.31 (A4-F1): detection is the PARSED
        // check in core::server — through 0.30 url_parts
        // string-matched this server's canonical 127.0.0.1
        // prefix, so the same server spelled localhost /
        // userinfo / trailing-dot was not recognized as
        // Pico-served and could be teleported at the phone.
        let is_pico_served = pico_core::core::server::is_served_url(&url, self.server.port());
        let lan_ip = if is_start {
            None
        } else {
            // AUDIT V28 A4-M1: the egress address — the interface
            // this machine actually routes through — is probed
            // ALWAYS and given to the chooser as a hint, not only
            // as a last-resort candidate: a merely-enumerated
            // adapter on the wrong subnet used to win on tier
            // while the phone could never reach it. If enumeration
            // yields nothing usable, the (already usability-
            // filtered) egress is the fallback.
            let egress = pico_core::core::teleport::udp_egress_ip();
            let candidates = enumerate_lan_candidates();
            choose_lan_ip_with_egress(&candidates, egress).or(egress)
        };
        let resolved = resolve(&url, lan_ip, is_pico_served);
        let host_label = pico_core::core::shield::host_of(&url).unwrap_or_default();
        log::info(&format!(
            "teleport: kind={} host={}",
            resolved.kind.label(),
            host_label
        ));
        // The QR encodes the QR-eligible URL only (AUDIT V28
        // A4-L5): for NoNetwork the display URL is a loopback
        // address a phone cannot open, so no matrix is encoded —
        // the text stays copyable for use on this PC. A URL
        // beyond QR capacity keeps the panel honest: no matrix,
        // the note says why, the text stays copyable.
        let mut note = resolved.note.clone();
        let mut matrix: Vec<String> = Vec::new();
        if let Some(target) = resolved.qr_url() {
            match pico_core::core::qr::encode_str(target) {
                Ok(qr) => matrix = qr.rows(),
                Err(pico_core::core::qr::QrError::TooLong { .. }) => {
                    note = format!(
                        "{note} This link is too long for a QR code \u{2014} use Copy link instead."
                    );
                }
                Err(pico_core::core::qr::QrError::Empty) => {}
            }
        }
        self.teleport = Some(TeleportSnap {
            url: resolved.url.clone().unwrap_or_default(),
            note,
            matrix,
        });
        self.teleport_version += 1;
        // Grow the chrome overlay so the panel has somewhere to
        // paint (the content rect does not move).
        if !self.teleport_panel_open {
            self.teleport_panel_open = true;
            self.layout();
        }
        self.push_state(true);
    }

    /// Record the current window geometry into the persisted state
    /// (AUDIT L7). Called on Resized (so the in-memory value is always
    /// fresh) and again before the state file is written on close.
    /// A maximized window does not overwrite the stored restore size —
    /// only the flag moves.
    fn capture_geometry(&mut self) {
        if self.window.is_maximized() {
            if let Some(w) = self.persist.window.as_mut() {
                w.maximized = true;
            } else {
                // No restore rect exists yet; use the current size.
                let sf = self.window.scale_factor().max(0.25);
                let size = self.window.inner_size();
                let w = ((size.width as f64 / sf) as u32).max(760);
                let h = ((size.height as f64 / sf) as u32).max(500);
                self.persist.window = Some(WindowRect {
                    x: 80,
                    y: 80,
                    w,
                    h,
                    maximized: true,
                });
            }
            return;
        }
        let sf = self.window.scale_factor().max(0.25);
        let pos = self.window.outer_position().unwrap_or_default();
        let size = self.window.inner_size();
        self.persist.window = Some(WindowRect {
            x: (pos.x as f64 / sf).round() as i32,
            y: (pos.y as f64 / sf).round() as i32,
            w: (size.width as f64 / sf).round() as u32,
            h: (size.height as f64 / sf).round() as u32,
            maximized: false,
        });
    }

    /// Open a filesystem path (file or directory) in a tab. Directories
    /// resolve to index.html; HTML files are always served through the
    /// static server (never file://), so relative assets and live
    /// reload work. The opening tab holds the root reference (M2).
    ///
    /// Resolution itself — the lexical UNC gate, canonicalization,
    /// and the folder/rel split — is the host-tested core function
    /// `core::fileopen::resolve_open_target` (AUDIT V17-2: the
    /// in-line version checked `canonical.join(&rel)`, which for a
    /// file appends its name to itself and failed EVERY file open
    /// from v13 to v16).
    fn open_path(&mut self, path: &Path) {
        let resolved = match resolve_open_target(path) {
            Ok(r) => r,
            Err(OpenResolveError::UnsafePath) => {
                log::error(&format!(
                    "open_path: refusing network/device path {}",
                    path.display()
                ));
                return;
            }
            Err(OpenResolveError::CannotResolve(why)) => {
                log::error(&format!(
                    "open_path: cannot resolve {}: {why}",
                    path.display()
                ));
                return;
            }
            Err(OpenResolveError::NoParent) => {
                log::error(&format!(
                    "open_path: no parent folder for {}",
                    path.display()
                ));
                return;
            }
            Err(OpenResolveError::MissingFile(target)) => {
                log::error(&format!("open_path: no file at {}", target.display()));
                return;
            }
        };
        let folder = resolved.folder;
        let rel = resolved.rel;
        let canonical = resolved.file;
        // AUDIT V28 A6-4: check the tab cap BEFORE taking a root
        // reference and a watcher — at the cap, add_tab refuses,
        // and the pre-registered root/watcher used to leak for
        // the whole session (and the file was still recorded in
        // Recents as if it had opened).
        if self.tabs.len() >= MAX_TABS {
            log::error(&format!(
                "open_path: tab cap ({MAX_TABS}) reached; refusing to open {}",
                canonical.display()
            ));
            return;
        }
        let root_id = match self.server.register_root(&folder) {
            Ok(id) => id,
            Err(e) => {
                log::error(&format!(
                    "open_path: cannot register root {}: {e}",
                    folder.display()
                ));
                return;
            }
        };
        ensure_watch(&mut self.watchers, &self.watch_tx, &self.server, root_id);
        let url = self.server.url_for(root_id, &rel);
        let id = self.add_tab(&url, false, true);
        if id > 0 {
            if let Some(t) = self.tab_mut(id) {
                // The opening tab owns the reference register_root took.
                t.root_held = t.root_id == Some(root_id);
            }
            pstore::push_recent(&mut self.persist, &canonical.to_string_lossy());
            self.save_state();
            self.push_recents_to_start_tabs();
        } else {
            // Belt-and-braces for any other add_tab refusal: the
            // reference and watcher come back off.
            self.server.release_root(root_id);
            self.note_root_maybe_gone(root_id);
        }
    }

    fn open_target(&mut self, target: Target) {
        match target {
            Target::Url(url) => {
                let is_start = url == START_URL;
                let id = self.add_tab(&url, is_start, true);
                if id > 0 {
                    if let Some(v) = self.active_tab().and_then(|t| t.view()) {
                        let _ = v.focus();
                    }
                }
            }
            Target::File(path) => self.open_path(&path),
        }
    }

    /// Convert the ACTIVE start tab into a content tab in place
    /// (AUDIT V26, defect D5): an address-bar submission on a
    /// fresh start tab navigates THAT tab instead of spawning
    /// another one. The conversion is total and immediate: the
    /// tab leaves the live start set (its navigation handler
    /// follows within the same instant), `is_start` flips (the
    /// action dispatch's Requester gate closes for this webview
    /// from this moment — proven in tests/dispatch_tests.rs),
    /// history and URL state restart at the target, and the
    /// existing webview loads it. Start-PAGE launcher actions
    /// (cards, recents, socials) do NOT come through here —
    /// they keep their open-in-new-tab design.
    fn convert_start_tab(&mut self, url: &str) {
        let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.id == self.active && t.is_start)
        else {
            return;
        };
        let id = self.tabs[idx].id;
        self.live_start_tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        // AUDIT 0.31 (A6-F2): the conversion load below is a
        // shell-initiated load of user-typed text.
        self.expect_nav(id, url);
        // AUDIT V28 A6-11: the conversion retires any Teleport
        // payload/panel naming the start page, immediately — not
        // whenever a later URL event happens to land.
        self.teleport = None;
        self.teleport_version += 1;
        self.teleport_panel_open = false;
        // AUDIT 0.31 (A1-F1): the theme picker panel retires on
        // conversion too, mirroring Teleport (layout() follows).
        self.palette_panel_open = false;
        self.layout();
        {
            let tab = &mut self.tabs[idx];
            tab.is_start = false;
            tab.hist = TabHistory::new();
            tab.urlstate = UrlState::with_url(url);
            tab.url = url.to_string();
            tab.title = String::new();
            // A conversion IS a load start: stale timing from the
            // start page's own load must not leak into the
            // live-reload fallback's comparisons.
            tab.loading = tab.view.is_some();
            tab.last_load_start = if tab.view.is_some() {
                Some(Instant::now())
            } else {
                None
            };
        }
        self.shield.set_page_url(id, url);
        self.shield.set_pending_main(id, url);
        if let Some(v) = self.tabs[idx].view() {
            let _ = v.load_url(url);
            let _ = v.focus();
        } else {
            // AUDIT V29 (A6-L1): a FAILED start tab converts
            // its record but has no view to load into — the
            // A6-3 retry existed only on the content-tab arm.
            // The record is already retargeted above; build
            // the view for the new URL.
            log::info(&format!(
                "start tab {id} converted while failed; retrying with {}",
                redact_url(url)
            ));
            self.retry_tab_view(id);
        }
        log::info(&format!(
            "start tab {id} converted in place: {}",
            redact_url(url)
        ));
        self.push_state(true);
    }

    /// The file-target half of the conversion (AUDIT V26 D5):
    /// resolve + register exactly like `open_path`, then convert
    /// the start tab onto the served URL instead of opening a
    /// new tab.
    fn convert_start_tab_to_file(&mut self, path: &Path) {
        let resolved = match resolve_open_target(path) {
            Ok(r) => r,
            Err(e) => {
                log::error(&format!("convert: cannot open {}: {e:?}", path.display()));
                return;
            }
        };
        let folder = resolved.folder;
        let rel = resolved.rel;
        let canonical = resolved.file;
        let root_id = match self.server.register_root(&folder) {
            Ok(id) => id,
            Err(e) => {
                log::error(&format!(
                    "convert: cannot register root {}: {e}",
                    folder.display()
                ));
                return;
            }
        };
        ensure_watch(&mut self.watchers, &self.watch_tx, &self.server, root_id);
        let url = self.server.url_for(root_id, &rel);
        let active_id = self.active;
        self.convert_start_tab(&url);
        if let Some(idx) = self.tab_index(active_id) {
            // The converted tab owns the reference register_root
            // took (the same fixup open_path applies to a new
            // tab), and it is scoped to the root immediately.
            self.set_tab_root(idx, Some(root_id));
            if let Some(t) = self.tab_mut(active_id) {
                t.root_held = t.root_id == Some(root_id);
            }
        }
        pstore::push_recent(&mut self.persist, &canonical.to_string_lossy());
        self.save_state();
        self.push_recents_to_start_tabs();
    }

    /// Record the one-shot navigation expectation for a
    /// shell-initiated load (AUDIT 0.31, A6-F2): the NEXT
    /// navigation event for `tab` — and only that one — may
    /// carry userinfo if it is exactly `url`. See the
    /// `expected_nav` field.
    fn expect_nav(&self, tab: u64, url: &str) {
        let mut slot = self.expected_nav.lock().unwrap_or_else(|p| p.into_inner());
        *slot = Some((tab, url.to_string()));
    }

    /// Address-bar submission (AUDIT V26, defect D5): routed by
    /// core::addr::submit_route — on a start tab the submission
    /// converts the tab in place; on a content tab a URL loads
    /// in place and a file opens in a new tab (the v12-proven
    /// split for content tabs).
    fn navigate_active(&mut self, input: &str) {
        let active_is_start = self.active_tab().map(|t| t.is_start).unwrap_or(false);
        // AUDIT V29 (A6-M1): the start page's own URL typed
        // into the address bar means "home" — it must never
        // enter the route table. Through v28 it dead-ended on
        // content tabs (the navigation handler refuses pico:
        // loads for them, silently) and CORRUPTED a start tab
        // (conversion flipped is_start and reset the tab, then
        // the same refusal stranded it on an unusable internal
        // URL). Home semantics already exist: go_home().
        if let Some(Target::Url(u)) = parse_input(input) {
            if u == START_URL {
                self.go_home();
                self.push_state(true);
                return;
            }
        }
        match parse_input(input) {
            Some(target) => match submit_route(active_is_start, &target) {
                SubmitRoute::ConvertStart => {
                    if let Target::Url(url) = target {
                        self.convert_start_tab(&url);
                    }
                }
                SubmitRoute::ConvertStartWithFile => {
                    if let Target::File(path) = target {
                        self.convert_start_tab_to_file(&path);
                    }
                }
                SubmitRoute::LoadCurrent => {
                    if let Target::Url(url) = target {
                        if let Some(tab_id) = self.active_tab().map(|t| t.id) {
                            // AUDIT 0.31 (A6-F2): user-typed text
                            // loaded in place — record the
                            // one-shot expectation before loading.
                            self.expect_nav(tab_id, &url);
                            if let Some(tab) = self.tab_mut(tab_id) {
                                // A fresh navigation pre-empts any
                                // pending back/forward intent.
                                tab.hist.cancel_pending();
                            }
                            let has_view = self
                                .tab_mut(tab_id)
                                .map(|t| t.view.is_some())
                                .unwrap_or(false);
                            if has_view {
                                if let Some(tab) = self.tab_mut(tab_id) {
                                    if let Some(v) = tab.view() {
                                        let _ = v.load_url(&url);
                                        let _ = v.focus();
                                    }
                                }
                            } else {
                                // AUDIT V28 A6-3: a FAILED tab has
                                // no view to load into — the typed
                                // URL used to vanish silently.
                                // Retarget the tab record first
                                // (the same write convert_start_tab
                                // makes), then take Reload's retry
                                // path, which builds the view for
                                // the NEW URL.
                                if let Some(tab) = self.tab_mut(tab_id) {
                                    tab.urlstate = UrlState::with_url(&url);
                                    tab.url = url.clone();
                                    tab.title = String::new();
                                    // AUDIT V29 (A6-L2): the
                                    // retarget is total, like
                                    // convert_start_tab's — the
                                    // stale history machine and
                                    // pending-main fact survived
                                    // into the retry in v28 and
                                    // the first commit could
                                    // landing-reconcile the new
                                    // URL as a move inside a dead
                                    // session's entries.
                                    tab.hist = TabHistory::new();
                                }
                                // AUDIT 0.30.1 (A4-2): the
                                // retarget rewrites the tab's URL
                                // directly, and a failed tab has
                                // no view — so no URL event will
                                // ever retire a Teleport panel
                                // naming the old page (the
                                // retirement apply_url_event
                                // performs on adoption, and
                                // convert_start_tab performs on
                                // conversion). Retire it here.
                                if self.teleport_panel_open {
                                    self.teleport = None;
                                    self.teleport_version += 1;
                                    self.teleport_panel_open = false;
                                    self.layout();
                                }
                                self.shield.set_page_url(tab_id, &url);
                                self.shield.set_pending_main(tab_id, &url);
                                log::info(&format!(
                                    "address submission on failed tab {tab_id}; retrying with {}",
                                    redact_url(&url)
                                ));
                                self.retry_tab_view(tab_id);
                            }
                        }
                    }
                }
                SubmitRoute::OpenFileInNewTab => {
                    if let Target::File(path) = target {
                        self.open_path(&path);
                    }
                }
            },
            None => {
                log::error(&format!(
                    "address input rejected: {}",
                    truncate_chars(&redact_url(input), 120)
                ));
                self.push_state(true);
            }
        }
    }

    fn go_home(&mut self) {
        let start_ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.is_start)
            .map(|t| t.id)
            .collect();
        if let Some(&first) = start_ids.first() {
            self.activate(first, true);
            if start_ids.len() > 1 {
                for id in &start_ids[1..] {
                    log::info(&format!(
                        "home: closing duplicate start tab {id} (self-heal)"
                    ));
                    self.close_tab(*id);
                }
            }
        } else {
            self.add_tab(START_URL, true, true);
        }
    }

    /// Back/forward traversal: record the intent in the history
    /// machine (the index moves only when the resulting pop/commit
    /// lands), then ask the *page itself* to traverse — WebView2 has
    /// no native traverse API through wry, and `history.back()` keeps
    /// same-document entries working.
    fn go(&mut self, forward: bool) {
        let Some(t) = self.active_tab() else { return };
        let id = t.id;
        // AUDIT V29 (A6-L4): no view, no intent. A failed tab
        // used to record the pending intent anyway; nothing
        // would ever land it, and the no-stacking guard then
        // blocked every later go() until an unrelated commit
        // happened to cancel it.
        if t.view.is_none() {
            return;
        }
        let ok = if forward {
            t.hist.can_go_forward()
        } else {
            t.hist.can_go_back()
        };
        if !ok {
            return;
        }
        // AUDIT V28 A6-7: never stack a second intent on a
        // pending one — the single pending slot cannot represent
        // both, and the eventual landing would reconcile against
        // the wrong direction's branch.
        // AUDIT 0.30.1 (A6-MED residue): …unless the pending
        // intent is dead. An intent whose landing signal never
        // arrives (the webview's stack and Pico's model diverged
        // far enough that `history.back()` was a silent no-op)
        // used to wedge every later go() behind this guard until
        // an unrelated navigation happened to clear it. Real
        // landings arrive in the same event-loop breath, so an
        // intent older than INTENT_TTL_MS is retired here — the
        // user's re-click IS the expiry event — and this click
        // issues its own intent instead.
        let now_ms = stats_now_ms();
        if t.hist.has_pending() {
            if t.hist.pending_expired(now_ms) {
                log::info(
                    "history: pending back/forward intent expired with no landing signal; re-issuing",
                );
                if let Some(tab) = self.tab_mut(id) {
                    tab.hist.cancel_pending();
                }
            } else {
                return;
            }
        }
        if let Some(tab) = self.tab_mut(id) {
            let target = if forward {
                tab.hist.go_forward(now_ms)
            } else {
                tab.hist.go_back(now_ms)
            };
            if target.is_some() {
                if let Some(v) = tab.view() {
                    let _ = v.evaluate_script(if forward {
                        "history.forward()"
                    } else {
                        "history.back()"
                    });
                }
            }
        }
    }

    fn reload_active(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let id = tab.id;
        if tab.view.is_none() {
            // Failed tab: Reload retries WebView creation (AUDIT M6).
            self.retry_tab_view(id);
            return;
        }
        if let Some(v) = tab.view() {
            let _ = v.reload();
        }
    }

    /// Apply a palette chosen in the picker (v27). Validated
    /// against the core table — an unknown id is logged and
    /// ignored, never persisted. The switch itself is the same
    /// discipline as the mode toggle: persist through the one
    /// save path, repaint every explicit background, push the
    /// theme to start tabs, and let the snapshot re-token the
    /// chrome. Already-open content pages are untouched (the
    /// chrome/start surfaces are the themed ones).
    fn set_palette(&mut self, id: &str) {
        if !pico_core::core::themes::is_known_palette(id) {
            log::error(&format!("theme: unknown palette id refused: {id}"));
            return;
        }
        if self.persist.theme_id == id {
            // AUDIT V28 A3-LOW-2: a same-id re-pick still re-pushes
            // the theme to start tabs — a start page that missed
            // the first push (recreated, not yet ready) heals here
            // instead of staying on the old palette forever.
            self.push_theme_to_start_tabs();
            self.push_state(true);
            return;
        }
        self.persist.theme_id = id.to_string();
        log::info(&format!("theme: palette set to {id}"));
        self.save_state();
        let rgba = theme_bg_rgba(&self.persist.theme_id, &self.persist.theme);
        self.window.set_background_color(Some(rgba));
        for t in &self.tabs {
            if let Some(v) = t.view() {
                let _ = v.set_background_color(rgba);
            }
        }
        self.push_theme_to_start_tabs();
        self.push_state(true);
    }

    fn toggle_theme(&mut self) {
        self.persist.theme = if self.persist.theme == "dark" {
            "light".to_string()
        } else {
            "dark".to_string()
        };
        self.save_state();
        // Keep every surface's explicit background in step with the
        // theme (AUDIT V14-2) — an unpainted frame mid-toggle must
        // show the new surface, not the old one and never black.
        let rgba = theme_bg_rgba(&self.persist.theme_id, &self.persist.theme);
        self.window.set_background_color(Some(rgba));
        // The chrome is deliberately NOT re-backgrounded (AUDIT
        // V16-2): its controller background is transparent, and
        // wry's runtime set_background_color forces alpha to 255 —
        // calling it here would silently turn the chrome opaque and
        // kill the feather on the first theme toggle. The chrome
        // document repaints itself from the theme tokens in the
        // snapshot pushed below; the window's new color is what
        // shows through its transparent regions.
        for t in &self.tabs {
            if let Some(v) = t.view() {
                let _ = v.set_background_color(rgba);
            }
        }
        self.push_theme_to_start_tabs();
        self.push_state(true);
    }

    // -- command processing ------------------------------------------------

    fn on_chrome_cmd(&mut self, raw: &str, control_flow: &mut ControlFlow) {
        let env = match parse_command(raw) {
            Ok(e) => e,
            Err(why) => {
                log::info(&format!(
                    "ipc: rejected chrome command ({why}): {}",
                    truncate_chars(&redact_url(raw), 120)
                ));
                return;
            }
        };
        if !self.dedupe.check(&env) {
            log::info(&format!("ipc: duplicate command dropped (req {})", env.req));
            return; // exact duplicate delivery — already handled
        }
        match env.cmd {
            Command::Ready => {
                // Shell health bookkeeping (AUDIT V14-3): pico.log
                // must always be able to answer "did the chrome come
                // up?" — the handshake and the first snapshot are
                // logged unconditionally.
                if !self.chrome_ready {
                    self.chrome_ready = true;
                    log::info(&format!(
                        "chrome handshake received (generation {})",
                        self.chrome_gen
                    ));
                }
                // AUDIT V26 (D4): landing on a start tab (first
                // boot, or a watchdog chrome recreation) focuses
                // the address bar — the hint rides this very
                // (forced) snapshot, so recreation cannot strand
                // the user's typing focus.
                if self.tabs.iter().any(|t| t.id == self.active && t.is_start) {
                    self.pending_focus_addr = true;
                }
                self.push_state(true);
                if !self.chrome_snapshot_logged {
                    self.chrome_snapshot_logged = true;
                    log::info("first snapshot delivered to chrome");
                }
                return;
            }
            Command::Navigate { text } => self.navigate_active(&text),
            Command::NewTab => {
                let id = self.add_tab(START_URL, true, true);
                if id > 0 {
                    // AUDIT V26 (D4): the fresh tab's snapshot
                    // carries the address-focus hint (add_tab's
                    // own push already ran without it, so push
                    // once more with the hint set).
                    self.pending_focus_addr = true;
                    self.push_state(true);
                    // Focus stays on the chrome so the user can type.
                    let _ = self.chrome.focus();
                }
            }
            Command::CloseTab { tab } => self.close_tab(tab),
            Command::SwitchTab { tab } => self.activate(tab, true),
            Command::Back => self.go(false),
            Command::Forward => self.go(true),
            Command::Reload => self.reload_active(),
            Command::Home => self.go_home(),
            Command::CycleTab { dir } => self.cycle_tab(dir),
            Command::ThemeToggle => self.toggle_theme(),
            Command::ShieldToggle => self.toggle_shield(),
            Command::LiteToggle => self.toggle_lite(),
            Command::Teleport => self.request_teleport(),
            Command::TeleportClose => {
                self.teleport = None;
                self.teleport_version += 1;
                if self.teleport_panel_open {
                    self.teleport_panel_open = false;
                    self.layout();
                }
            }
            Command::TeleportCopy => {
                log::info("teleport: copy");
            }
            Command::Palette => {
                // Mutually exclusive with Teleport (v27): the
                // picker replaces it, payload and all.
                self.teleport = None;
                self.teleport_version += 1;
                self.teleport_panel_open = false;
                if !self.palette_panel_open {
                    self.palette_panel_open = true;
                    self.layout();
                }
                self.push_state(true);
            }
            Command::PaletteClose => {
                if self.palette_panel_open {
                    self.palette_panel_open = false;
                    self.layout();
                }
            }
            Command::ThemeSet { id } => self.set_palette(&id),
            Command::OpenFile => self.open_file_dialog(),
            Command::Social { site } => self.open_social(site),
            Command::WinMin => self.window.set_minimized(true),
            Command::WinMax => self.window.set_maximized(!self.window.is_maximized()),
            Command::WinDrag => {
                let _ = self.window.drag_window();
            }
            Command::WinClose => {
                self.capture_geometry();
                self.save_state();
                *control_flow = ControlFlow::Exit;
            }
        }
        self.push_state(false);
    }

    /// Arm the handshake watchdog for chrome generation `gen`
    /// (AUDIT V14-3): a one-shot timer thread that reports back
    /// through the event loop after [`CHROME_WATCHDOG_DELAY`].
    fn schedule_chrome_watchdog(&self, gen: u64) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(CHROME_WATCHDOG_DELAY);
            let _ = proxy.send_event(UserEvent::ChromeWatchdog(gen));
        });
    }

    /// Watchdog verdict for chrome generation `gen`: a healthy or
    /// stale timer is ignored; a missing handshake spends the single
    /// recreation; a second miss escalates to a native dialog. The
    /// shell's failure is a headline event, never a silent black
    /// rectangle (AUDIT V14-3).
    fn on_chrome_watchdog(&mut self, gen: u64) {
        if gen != self.chrome_gen || self.chrome_ready {
            return;
        }
        if !self.chrome_recreated {
            log::error(&format!(
                "chrome handshake missing (generation {gen}); recreating the chrome webview once"
            ));
            self.chrome_recreated = true;
            // AUDIT V15-1: the destroyed generation must never speak
            // again — retire its boot id so any of its commands still
            // queued in the event loop (including a late `ready`) are
            // dropped instead of being adopted by the new generation.
            self.dedupe.retire_current_boot();
            let bounds = self.chrome_rect();
            match build_chrome(
                &self.window,
                &mut self.web_ctx,
                &self.shared,
                &self.proxy,
                bounds,
            ) {
                Ok(view) => {
                    self.chrome = view;
                    harden_webview_profile(&self.chrome, "chrome");
                    self.chrome_gen += 1;
                    // Force the next snapshot out even if its bytes
                    // match the one the dead chrome never received.
                    self.last_snap_sig = 0;
                    self.layout();
                    log::info(&format!(
                        "chrome webview recreated (generation {}); awaiting handshake",
                        self.chrome_gen
                    ));
                    let gen = self.chrome_gen;
                    self.schedule_chrome_watchdog(gen);
                }
                Err(e) => {
                    log::error(&format!("chrome recreation failed: {e}"));
                    self.escalate_chrome_failure();
                }
            }
        } else {
            log::error("chrome handshake still missing after recreation; the shell cannot start");
            self.escalate_chrome_failure();
        }
    }

    /// Last-resort surfacing for a dead shell: a native dialog
    /// naming the problem and the log path (the message is also in
    /// the log at ERROR).
    fn escalate_chrome_failure(&self) {
        let msg = format!(
            "Pico's browser chrome (the tab bar and address bar) failed to start.\n\n\
             The pages themselves are unaffected, but Pico cannot be controlled without its chrome. \
             Please restart Pico.\n\nDetails were written to the log:\n{}",
            crate::log::log_path().display()
        );
        log::error(&msg);
        #[cfg(windows)]
        show_native_dialog("Pico — chrome failed to start", &msg);
    }

    /// Decide and execute ONE start-page action (AUDIT V17-1). The
    /// decision itself is the pure core function
    /// `core::action::dispatch` over the app's own tab state (the
    /// requester identity and current URL come from `self.tabs`,
    /// never from the page). EVERY dispatch — allowed or denied —
    /// writes exactly one pico.log line naming the action kind and
    /// the decision, so a dead button is always diagnosable from
    /// the log; file paths are logged by basename only, service
    /// URLs redacted.
    fn on_start_action(&mut self, tab_id: u64, source: ActionSource) {
        let kind = kind_label(&source);
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            log::error(&format!(
                "start action: kind={kind} tab={tab_id} decision=deny reason=tab-gone"
            ));
            return;
        };
        let requester = Requester {
            is_start_tab: tab.is_start,
            current_url: tab.url.clone(),
        };
        match dispatch_action(&source, &requester, &self.persist.recents) {
            Decision::OpenRecent(path) => {
                let base = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                log::info(&format!(
                    "start action: kind={kind} tab={tab_id} decision=open-recent file={base}"
                ));
                self.open_path(&path);
            }
            Decision::OpenPicker => {
                log::info(&format!(
                    "start action: kind={kind} tab={tab_id} decision=open-picker"
                ));
                self.open_file_dialog();
            }
            Decision::OpenService(url) => {
                log::info(&format!(
                    "start action: kind={kind} tab={tab_id} decision=open-service url={}",
                    redact_url(&url)
                ));
                let id = self.add_tab(&url, false, true);
                if id > 0 {
                    if let Some(v) = self.active_tab().and_then(|t| t.view()) {
                        let _ = v.focus();
                    }
                }
            }
            Decision::OpenSocial(site) => {
                log::info(&format!(
                    "start action: kind={kind} tab={tab_id} decision=open-social site={}",
                    redact_url(site.url())
                ));
                self.open_social(site);
            }
            Decision::RemoveRecent(idx) => {
                // dispatch range-checked idx against this same
                // list; the guard here keeps the arm total even
                // if that contract is ever revisited.
                if idx < self.persist.recents.len() {
                    let removed = self.persist.recents.remove(idx);
                    let base = Path::new(&removed)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    log::info(&format!(
                        "start action: kind={kind} tab={tab_id} decision=remove-recent index={idx} file={base}"
                    ));
                    self.save_state();
                    self.push_recents_to_start_tabs();
                } else {
                    log::error(&format!(
                        "start action: kind={kind} tab={tab_id} decision=deny reason=recent-out-of-range"
                    ));
                }
            }
            Decision::ClearRecents => {
                let n = self.persist.recents.len();
                self.persist.recents.clear();
                log::info(&format!(
                    "start action: kind={kind} tab={tab_id} decision=clear-recents count={n}"
                ));
                self.save_state();
                self.push_recents_to_start_tabs();
            }
            Decision::SetRecentsHidden(hidden) => {
                self.persist.recents_hidden = hidden;
                log::info(&format!(
                    "start action: kind={kind} tab={tab_id} decision=recents-hidden hidden={hidden}"
                ));
                self.save_state();
                self.push_recents_hidden_to_start_tabs();
            }
            Decision::ResetStats => {
                // 0.30: zero the whole book (lifetime, session,
                // calibration ledger, correlation state) and
                // persist the zero at this same save point, so
                // the reset survives a restart like any other
                // start-page gesture's effect.
                self.stats.reset();
                log::info(&format!(
                    "start action: kind={kind} tab={tab_id} decision=reset-stats"
                ));
                self.save_state();
                self.push_stats_to_start_tabs(true);
            }
            Decision::Deny(reason) => {
                log::error(&format!(
                    "start action: kind={kind} tab={tab_id} decision=deny reason={}",
                    reason.as_str()
                ));
            }
        }
    }

    fn open_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open a local web page")
            .add_filter("Web pages", &["html", "htm", "xhtml"])
            .pick_file()
        {
            self.open_path(&path);
        }
    }

    /// Open one of the author's social pages (v15 branding) in a new
    /// tab. The destination comes from the [`SocialSite`] enum's
    /// constant mapping — no URL ever arrives from a page for this.
    fn open_social(&mut self, site: SocialSite) {
        let url = site.url();
        log::info(&format!("social link opened: {}", redact_url(url)));
        let id = self.add_tab(url, false, true);
        if id > 0 {
            if let Some(v) = self.active_tab().and_then(|t| t.view()) {
                let _ = v.focus();
            }
        }
    }

    // -- user-event processing ----------------------------------------------

    /// Feed one URL event into a tab's state machine and apply
    /// the outcome (AUDIT V26, defect D7): mirror the live URL
    /// into the tab record, record the history effect, keep the
    /// Shield's page state and the live-reload root scope keyed
    /// to the LIVE URL (never a stale commit), and close a
    /// Teleport panel whose payload the navigation just made
    /// stale. This is the ONLY writer of `Tab.url` besides tab
    /// creation/conversion.
    /// Returns whether the event was a refused (stale) observer
    /// note — the only outcome any caller acts on directly.
    fn apply_url_event(&mut self, idx: usize, ev: UrlEvent) -> bool {
        // AUDIT 0.30.1 (A2-C4): the chosen-flag retirement
        // below judges the change from the URL being replaced
        // and the event's kind (a commit is a new document; a
        // Source/Note is the same document changing its URL) —
        // both must be captured before the reducer consumes
        // the event.
        let old_url = self.tabs[idx].url.clone();
        // AUDIT 0.31 (A2-F1): which commit event this is,
        // judged BEFORE the reducer consumes it — CommitStart
        // and CommitFinish now retire the Lite chosen flag
        // under different rules (see the adopted block below).
        let is_commit_start = matches!(ev, UrlEvent::CommitStart(_));
        let is_commit_finish = matches!(ev, UrlEvent::CommitFinish(_));
        let commit_start_url = match &ev {
            UrlEvent::CommitStart(u) => Some(u.clone()),
            _ => None,
        };
        let pending = self.tabs[idx].hist.pending_target().map(|t| t.to_string());
        // AUDIT 0.31 (A6-F1, core half): a commit start is a
        // history-machine fact in its own right — note it
        // before the reducer runs, so the machine can tell a
        // traversal's commit from a fresh navigation's.
        if let Some(u) = &commit_start_url {
            self.tabs[idx].hist.note_commit_start(u);
            self.tabs[idx].commit_started = true;
        }
        // AUDIT V29 (A6-H1): for a Source event, does its URL
        // already live in this tab's history at a non-current
        // entry? That fact is what separates an uninitiated
        // traversal (Pop) from a genuine push in the reducer.
        let in_history = match &ev {
            UrlEvent::Source(u) => {
                let hist = &self.tabs[idx].hist;
                let cur = hist.index();
                hist.entries()
                    .iter()
                    .enumerate()
                    .any(|(j, e)| Some(j) != cur && e == u)
            }
            _ => false,
        };
        let outcome = self.tabs[idx]
            .urlstate
            .apply_landing(ev, pending.as_deref(), in_history);
        let stale_note = outcome.stale_note;
        // AUDIT 0.31 (A2-F1): a CommitFinish consumes the
        // tab's commit_started flag whether or not it adopts
        // a URL. It clears the chosen flag only when it is an
        // ORPHAN finish (no CommitStart was applied for this
        // document) — a designation made DURING a document's
        // load (start seen, then the in-page gate chose its
        // video, then finish) survives its own finish.
        let finish_clears_chosen = if is_commit_finish {
            let orphan = !self.tabs[idx].commit_started;
            self.tabs[idx].commit_started = false;
            orphan
        } else {
            false
        };
        let live = self.tabs[idx].urlstate.url().to_string();
        match outcome.hist {
            // AUDIT 0.31 (A6-F1): record_commit is the
            // started-aware commit — it consumes the
            // note_commit_start recorded above, so a load that
            // started on a history entry under a pending
            // intent lands THERE (the finish URL replacing
            // that entry), never step-replaces the intent's
            // one-step neighbour.
            HistEffect::Commit => self.tabs[idx].hist.record_commit(&live),
            HistEffect::Push => self.tabs[idx].hist.record_push(&live),
            HistEffect::Replace => self.tabs[idx].hist.record_replace(&live),
            HistEffect::Pop => self.tabs[idx].hist.record_pop(&live),
            // AUDIT V28 A6-1: a Traversal outcome is NOT a no-op —
            // the intent only recorded `pending`, so this signal
            // is what moves the history index and consumes it.
            HistEffect::Traversal => self.tabs[idx].hist.record_traversal(&live),
            HistEffect::None => {}
        }
        if let Some(new_url) = outcome.adopted {
            let id = self.tabs[idx].id;
            self.tabs[idx].url = new_url.clone();
            // AUDIT 0.30.1 (A2-C4): retire the tab's
            // chosen-video flag when the designation does not
            // survive this URL change. The document gate resets
            // its designation on every route change; through
            // 0.30 the host flag cleared only at load start, so
            // after an SPA route change away from a watch page
            // it kept blessing preview media/streams on the new
            // page indefinitely. A commit retires it (new
            // document — the same law load start already
            // applied), with ONE 0.31 exception (A2-F1): a
            // CommitFinish that had a CommitStart must not
            // retire a designation made during THIS document's
            // load — the gate designated it, and the host flag
            // is what lets its streams through; a finish
            // WITHOUT a start (orphan) retires as every commit
            // did through 0.30. A same-document change
            // consults the survival matrix in core::lite (the
            // same-video bridge keeps it).
            let clear_for_commit = is_commit_start || finish_clears_chosen;
            if clear_for_commit
                || !pico_core::core::lite::chosen_survives_url_change(&old_url, &new_url)
            {
                self.lite.clear_chosen(id);
            }
            // The Shield's party/main-frame facts and the media
            // probe's context follow the live URL (D7-4).
            self.shield.set_page_url(id, &new_url);
            let root = self.server.root_id_for_url(&new_url);
            self.set_tab_root(idx, root);
            if id == self.active && (self.teleport_panel_open || self.palette_panel_open) {
                // The panel names the page it was computed for;
                // a navigation retires it (AUDIT V26 D3).
                // AUDIT 0.31 (A1-F1): the theme picker follows
                // the same dismissal law — through 0.30 only
                // Teleport was retired here, so the picker (and
                // the grown chrome rect) survived navigation.
                if self.teleport_panel_open {
                    self.teleport = None;
                    self.teleport_version += 1;
                    self.teleport_panel_open = false;
                }
                self.palette_panel_open = false;
                self.layout();
            }
        }
        stale_note
    }

    /// SourceChanged (AUDIT V26, defect D7): the platform's
    /// live URL for a tab. Start tabs are skipped (their URL is
    /// the internal start URL by definition; a converted tab is
    /// no longer start and flows normally from then on).
    fn on_source_changed(&mut self, tab_id: u64, url: String) {
        let Some(idx) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[idx].is_start || self.tabs[idx].view.is_none() {
            return;
        }
        if !is_allowed_navigation_url(&url) {
            return;
        }
        self.apply_url_event(idx, UrlEvent::Source(url));
        self.push_state(false);
    }

    /// A same-document navigation note from the observer script.
    fn on_nav_note(&mut self, tab_id: u64, note: NavNote) {
        let Some(idx) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[idx].is_start {
            return; // the start page manages itself
        }
        // Observer notes are untrusted hints from page script
        // (AUDIT V26 D7): the reducer owns acceptance now. In
        // the pre-Source window it applies the same-origin gate
        // v25 applied here (a page must never spoof another
        // origin into the bar); once Source has settled the
        // document, a disagreeing note is stale and refused —
        // that refusal is what un-wedges the v25 freeze, where
        // one host-form flip (www <-> bare) made every later
        // note "cross-origin" against the frozen stored URL.
        let stale = self.apply_url_event(
            idx,
            UrlEvent::Note {
                how: note.how,
                href: note.href.clone(),
            },
        );
        if stale {
            log::info(&format!(
                "nav note dropped (stale): {} -> {}",
                redact_url(&self.tabs[idx].url),
                redact_url(&note.href)
            ));
            return;
        }
        if !note.title.is_empty() && self.tabs[idx].title != note.title {
            self.tabs[idx].title = note.title.clone();
        }
        self.push_state(false);
    }

    fn on_page_load(&mut self, tab_id: u64, finished: bool, url: String) {
        let Some(idx) = self.tab_index(tab_id) else {
            return;
        };
        let root_id = self.server.root_id_for_url(&url);
        let real = !url.is_empty() && url != "about:blank";
        if real && !is_allowed_navigation_url(&url) {
            // Defense in depth behind the navigation handler: never
            // let state track a document outside the allowlist.
            log::error(&format!(
                "page load outside allowlist ignored: {}",
                redact_url(&url)
            ));
            self.tabs[idx].loading = false;
            self.push_state(false);
            return;
        }
        if !finished {
            // AUDIT V29: a new document retires the old
            // document's chosen-video designation.
            if real {
                self.lite.clear_chosen(tab_id);
            }
            {
                let tab = &mut self.tabs[idx];
                tab.loading = true;
                tab.last_load_start = Some(Instant::now());
                // A load that is not the pending traversal's target
                // pre-empts the intent (user clicked a link while a
                // back/forward was in flight) — AUDIT 0.31
                // (A6-F1): UNLESS the load's start URL is itself
                // an entry in this tab's history. That is a
                // coalesced traversal landing (the webview raced
                // our one-step intent with its own multi-step
                // move): cancelling here forced the finish into
                // a plain push and manufactured a phantom
                // forward branch. The intent survives and the
                // started-aware commit reconciles it.
                if real {
                    if let Some(target) = tab.hist.pending_target() {
                        if target != url.as_str() && !tab.hist.entries().iter().any(|e| e == &url) {
                            tab.hist.cancel_pending();
                        }
                    }
                }
            }
            if real {
                // AUDIT V26 (D7): the commit feeds the reducer —
                // it adopts the URL (a redirect chain's final hop
                // wins) and re-arms note acceptance for the new
                // document.
                self.apply_url_event(idx, UrlEvent::CommitStart(url.clone()));
            }
        } else {
            {
                let tab = &mut self.tabs[idx];
                tab.loading = false;
                if real {
                    if tab.is_start {
                        tab.title = "Start".to_string();
                    } else if tab.title.is_empty() || tab.title == "Loading…" {
                        tab.title = url.clone();
                    }
                }
            }
            if real {
                // The reducer records the history commit (AUDIT
                // V26 D7) — one writer for URL + history.
                self.apply_url_event(idx, UrlEvent::CommitFinish(url.clone()));
            }
        }
        if real {
            self.set_tab_root(idx, root_id);
            // Shield page-state (AUDIT V21): the committed URL is
            // the party/`domain=` context for this tab's requests.
            self.shield.set_page_url(tab_id, &url);
            if !self.shield.on.load(std::sync::atomic::Ordering::Relaxed) {
                // Shield OFF (AUDIT V23): a view built while the
                // Shield was on keeps its document-start script
                // registered (WebView2 cannot unregister one), so
                // every fresh document of such a view starts with
                // the payload installed. Disarm it at load start
                // and again at finish: the in-page wrappers check
                // the flag on every action, so from load-start on
                // the page is untouched; the network layer never
                // needed this (it reads the atomic flag per
                // request). The residual window — document-start
                // to load-start, before any page content runs —
                // is documented in AUDIT v23.
                if let Some(v) = self.tabs[idx].view() {
                    let _ = v.evaluate_script(
                        "window.__picoShieldOff=1;\
                         var e=document.getElementById('pico-shield-style');\
                         if(e)e.remove();window.__picoShieldCosmetic=0;",
                    );
                }
            }
            // AUDIT V26 (D6, interplay item d): the collapse
            // layer's arming flag is maintained with the same
            // discipline — 1 only when BOTH filters are off
            // (byte-identical passthrough), 0 whenever either is
            // on. The Shield-off eval above cannot express the
            // Lite dimension, so it is a separate line, evaluated
            // at every load boundary.
            {
                let lite_on = self.lite.on.load(std::sync::atomic::Ordering::Relaxed);
                let both_off =
                    !self.shield.on.load(std::sync::atomic::Ordering::Relaxed) && !lite_on;
                if let Some(v) = self.tabs[idx].view() {
                    let _ = v.evaluate_script(if both_off {
                        "window.__picoCollapseOff=1;"
                    } else {
                        "window.__picoCollapseOff=0;"
                    });
                    // The Lite play gate's disarm flag is
                    // maintained at the same boundary (v28): a
                    // document born while Lite is off carries
                    // the flag, so a gate pushed later the same
                    // document (toggle) starts from a known
                    // state; the runtime clears it on install.
                    let _ = v.evaluate_script(if lite_on {
                        "window.__picoLiteGateOff=0;"
                    } else {
                        "window.__picoLiteGateOff=1;"
                    });
                }
                // A page on a view that never got the Shield
                // document script (Shield off at build, or lists
                // still compiling) still needs the collapse layer
                // when Lite is on: push it per load, idempotent
                // per document (guarded inside the runtime).
                // v28: with Lite on, the push is the COMBINED
                // Lite document script (collapse + play gate).
                if !both_off && !self.tabs[idx].doc_start_script && finished {
                    if let Some(v) = self.tabs[idx].view() {
                        if lite_on {
                            let _ = v.evaluate_script(
                                &pico_core::core::filterpipe::lite_document_script(),
                            );
                        } else {
                            let _ =
                                v.evaluate_script(pico_core::core::filterpipe::COLLAPSE_RUNTIME);
                        }
                    }
                } else if lite_on && finished {
                    // The Shield's document script (doc-start or
                    // the per-load push below) carries the
                    // collapse layer but never the play gate:
                    // the gate rides separately, always.
                    if let Some(v) = self.tabs[idx].view() {
                        let _ = v.evaluate_script(pico_core::core::filterpipe::LITE_GATE_RUNTIME);
                    }
                }
            }
            if self.shield.on.load(std::sync::atomic::Ordering::Relaxed)
                && finished
                && !self.tabs[idx].doc_start_script
            {
                // This view was built without the document script
                // (lists still compiling, or the Shield was off):
                // push it per load — it is idempotent per document.
                if let Some(script) = self.shield.document_script() {
                    if let Some(v) = self.tabs[idx].view() {
                        let _ = v.evaluate_script(&script);
                    }
                }
            }
        }
        self.push_state(false);
    }
}

/// Watch a root folder for changes and report root ids on the sync
/// channel. Shared with `open_path` and the re-watch pass in `run()`.
/// The watcher callback does zero filtering per event (debounce and
/// ignore-rules live in the drain thread): latency here is pure noise.
fn ensure_watch(
    watchers: &mut HashMap<u64, notify::RecommendedWatcher>,
    tx: &std::sync::mpsc::SyncSender<(u64, Vec<PathBuf>)>,
    server: &FileServer,
    root_id: u64,
) {
    if watchers.contains_key(&root_id) {
        return;
    }
    if watchers.len() >= MAX_WATCHED_ROOTS {
        log::error(&format!(
            "watcher cap ({MAX_WATCHED_ROOTS}) reached; root #{root_id} will not live-reload"
        ));
        return;
    }
    let Some(root) = server.root_path(root_id) else {
        return;
    };
    let tx = tx.clone();
    match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            // Paths ride along so the drain thread can apply the
            // ignore filter before waking any page.
            let _ = tx.try_send((root_id, event.paths));
        }
    }) {
        Ok(mut w) => {
            if w.watch(&root, RecursiveMode::Recursive).is_err() {
                log::error(&format!("watch failed for {}", root.display()));
                return;
            }
            watchers.insert(root_id, w);
        }
        Err(e) => log::error(&format!("watcher create failed: {e}")),
    }
}

/// Should a change under this relative path trigger a reload?
/// Directories that never affect the rendered page are ignored
/// (dependency trees and VCS internals are the watcher-flood sources).
fn is_relevant_change(rel: &Path) -> bool {
    const IGNORED: &[&str] = &["node_modules", ".git"];
    !rel.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        IGNORED.iter().any(|i| *i == name)
    })
}

// ---------------------------------------------------------------------------
// Localhost detector (Windows): TCP listeners -> probes -> start-page cards
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn spawn_detector(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        use pico_core::core::detect::scan_services;
        let our_pid = std::process::id();
        let mut titles = TitleCache::new();
        let mut name_cache: HashMap<u32, String> = HashMap::new();
        let mut prev_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut last_sig = String::new();
        loop {
            let listeners = enum_listeners();
            // AUDIT L2: the PID->name cache forgot nothing in v12, so a
            // recycled PID could wear a dead process's name forever.
            // Evict any PID that is not listening any more, and only
            // look up names for PIDs we have never seen.
            let current_pids: std::collections::HashSet<u32> =
                listeners.iter().map(|l| l.pid).collect();
            name_cache.retain(|pid, _| current_pids.contains(pid));
            for pid in current_pids.difference(&prev_pids) {
                if let Some(name) = process_image_name(*pid) {
                    name_cache.insert(*pid, name);
                }
            }
            prev_pids = current_pids;
            let cache = name_cache.clone();
            let outcome = scan_services(
                &listeners,
                our_pid,
                Duration::from_millis(300),
                &mut titles,
                &move |pid| cache.get(&pid).cloned(),
            );
            let sig = signature(&outcome);
            if sig != last_sig {
                last_sig = sig;
                let ports: Vec<String> = outcome.cards.iter().map(|s| s.port.to_string()).collect();
                log::info(&format!(
                    "detector: {} service card(s) [{}]; {} other listener(s) not shown as cards",
                    outcome.cards.len(),
                    ports.join(", "),
                    outcome.other_listeners
                ));
                if proxy
                    .send_event(UserEvent::ServicesUpdated(outcome))
                    .is_err()
                {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(2500));
        }
    });
}

#[cfg(not(windows))]
fn spawn_detector(_proxy: EventLoopProxy<UserEvent>) {}

/// Fetch the TCP listener table for one address family (4 or 6),
/// retrying the allocate-and-call dance when the required size moves
/// between the two calls (AUDIT L3 — v12 took one shot and a size
/// race blanked the whole scan cycle).
#[cfg(windows)]
fn get_tcp_table(af: u32) -> Option<Vec<u8>> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER,
    };
    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 23;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    let family = if af == 6 { AF_INET6 } else { AF_INET };
    for _ in 0..3 {
        let mut size: u32 = 0;
        // SAFETY: first call passes a null table pointer purely to
        // learn the required buffer size through `size` — the
        // documented sizing idiom for GetExtendedTcpTable.
        unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
        }
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` is exactly the measured size and stays alive
        // for the call; the API writes at most `size` bytes into it
        // and reports the real size back through the same cell.
        let rc = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                &mut size,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if rc == 0 {
            return Some(buf);
        }
        if rc != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        // Size moved under us; loop and re-measure.
    }
    None
}

#[cfg(windows)]
fn enum_listeners() -> Vec<Listener> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
    };
    let mut out = Vec::new();
    // The tables are read as (u32 count, packed rows). The row
    // count is clamped to what the buffer can physically hold, so a
    // lying header can never walk the parse off the end (AUDIT L3).
    if let Some(buf) = get_tcp_table(4) {
        let n = u32::from_ne_bytes(buf[0..4].try_into().unwrap_or([0; 4])) as usize;
        let max_rows = (buf.len().saturating_sub(4)) / std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
        for row in read_rows::<MIB_TCPROW_OWNER_PID>(&buf, n.min(max_rows)) {
            if row.dwState == 2 {
                // MIB_TCP_STATE_LISTEN
                let port = u16::from_be((row.dwLocalPort & 0xffff) as u16);
                let ip = std::net::Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
                out.push(Listener {
                    addr: ip.to_string(),
                    port,
                    pid: row.dwOwningPid,
                });
            }
        }
    }
    if let Some(buf) = get_tcp_table(6) {
        let n = u32::from_ne_bytes(buf[0..4].try_into().unwrap_or([0; 4])) as usize;
        let max_rows = (buf.len().saturating_sub(4)) / std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();
        for row in read_rows::<MIB_TCP6ROW_OWNER_PID>(&buf, n.min(max_rows)) {
            if row.dwState == 2 {
                // MIB_TCP_STATE_LISTEN
                let port = u16::from_be((row.dwLocalPort & 0xffff) as u16);
                let ip = std::net::Ipv6Addr::from(row.ucLocalAddr);
                out.push(Listener {
                    addr: ip.to_string(),
                    port,
                    pid: row.dwOwningPid,
                });
            }
        }
    }
    out
}

#[cfg(windows)]
fn read_rows<T: Copy>(buf: &[u8], count: usize) -> Vec<T> {
    // The tables are packed arrays of POD rows with no padding between
    // entries (Win32 guarantees this layout for these structures).
    let mut out = Vec::with_capacity(count);
    let stride = std::mem::size_of::<T>();
    for i in 0..count {
        let off = 4 + i * stride;
        let Some(slice) = buf.get(off..off + stride) else {
            break;
        };
        // SAFETY: `slice` is exactly size_of::<T>() bytes from the OS
        // table buffer and T is a plain-old-data Win32 row struct.
        let row = unsafe { std::ptr::read_unaligned(slice.as_ptr() as *const T) };
        out.push(row);
    }
    out
}

/// Executable file name for a PID (`QueryFullProcessImageNameW` —
/// needs only PROCESS_QUERY_LIMITED_INFORMATION, so protected
/// processes answer too; v12-proven approach).
#[cfg(windows)]
fn process_image_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    if pid == 0 {
        return None;
    }
    // SAFETY: the handle is checked non-null before use and closed
    // on every path; the buffer is a fixed 512-wide-char array the
    // API bounds through `len` (in/out), and the returned length is
    // clamped to the buffer before slicing.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        let _ = CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]);
        path.rsplit(['\\', '/'])
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Fatal startup errors
// ---------------------------------------------------------------------------

/// Show a fatal-startup explanation and exit. These are the failures
/// where continuing is impossible (no window, no chrome WebView, no
/// file server port). On Windows the user gets a native dialog — the
/// console is compiled out of release builds.
fn fatal_webview(context: &str, err: &str) -> ! {
    log::error(&format!("fatal: {context}: {err}"));
    let msg = format!(
        "Pico could not start: {context}.\n\n{err}\n\nPico needs the Microsoft Edge WebView2 Runtime (it ships with Windows 11 and recent Windows 10). Details were written to pico.log next to the app data."
    );
    #[cfg(windows)]
    {
        show_native_dialog("Pico — cannot start", &msg);
    }
    #[cfg(not(windows))]
    {
        eprintln!("{msg}");
    }
    std::process::exit(1);
}

#[cfg(windows)]
fn show_native_dialog(title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let to_wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let text_w = to_wide(text);
    let title_w = to_wide(title);
    // SAFETY: both buffers are NUL-terminated, outlive the modal
    // call, and stay put while it runs; no parent window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text_w.as_ptr(),
            title_w.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

type WebContext = wry::WebContext;

/// Build the chrome WebView — used at startup and by the watchdog's
/// single recreation (AUDIT V14-3), so both paths construct the
/// shell in exactly the same way.
///
/// The navigation handler is the v14 fix for the v13 black shell
/// (AUDIT V14-1): it allows *exactly* the chrome document (the
/// initial `with_url` load and its protocol-internal rewrite) and
/// denies everything else — v13's deny-all version also cancelled
/// the initial load, because wry applies the handler to WebView2's
/// `NavigationStarting` for every navigation including the first.
///
/// Transparency (AUDIT V16-2): the controller background is
/// transparent (wry's `with_transparent` maps to WebView2's
/// `DefaultBackgroundColor` = transparent in the pinned wry 0.53.5)
/// so the document's feather strip can blend the header into the
/// content beneath it. This is the single deliberate exception to
/// the v14 explicit-background rule, and it is confined to the
/// chrome: the tao window behind it always carries the themed
/// background color, so an unpainted or failed chrome shows the
/// theme surface — never unpainted black. Nothing may call
/// `set_background_color` on this WebView afterwards (wry forces
/// alpha to 255 there, which would silently undo the transparency;
/// see `toggle_theme`).
fn build_chrome(
    window: &Rc<Window>,
    web_ctx: &mut WebContext,
    shared: &Shared,
    proxy: &EventLoopProxy<UserEvent>,
    bounds: Rect,
) -> Result<WebView, String> {
    let proxy_ipc = proxy.clone();
    let proxy_dl = proxy.clone();
    let builder = WebViewBuilder::new_with_web_context(web_ctx)
        .with_url(CHROME_URL)
        // Transparent controller background (AUDIT V16-2, above).
        .with_transparent(true)
        .with_custom_protocol("pico".to_string(), pico_protocol_handler(shared.clone()))
        .with_ipc_handler(move |request: Request<String>| {
            // AUDIT V28 (A1-L5): drop oversize bodies HERE. The
            // 64 KB command cap used to be enforced only in the
            // event loop, after the body had been cloned out of
            // the request and queued as an event — the cap
            // existed, but the allocation it exists to prevent
            // had already happened.
            let body = request.body();
            if body.len() > pico_core::core::ipc::MAX_COMMAND_BODY_BYTES {
                return;
            }
            let _ = proxy_ipc.send_event(UserEvent::ChromeCmd(body.clone()));
        })
        .with_download_started_handler(move |uri: String, _path: &mut PathBuf| -> bool {
            let _ = proxy_dl.send_event(UserEvent::DownloadBlocked { tab: 0, uri });
            false
        })
        .with_navigation_handler(|url: String| {
            let url = normalize_pico_url(&url);
            url == CHROME_URL || url == "pico://chrome/"
        })
        // AUDIT V15-4: the chrome has no window.open surface by
        // design; deny explicitly instead of relying on wry/WebView2
        // defaults, so a future chrome-page change can never turn a
        // popup into a shell escape. (Content views route popups to
        // new tabs through their own handler — AUDIT V22-10.)
        .with_new_window_req_handler(|url, _features| {
            crate::log::info(&format!(
                "chrome new-window request denied: {}",
                redact_url(&normalize_pico_url(&url))
            ));
            NewWindowResponse::Deny
        })
        .with_devtools(true)
        .with_bounds(bounds)
        .with_visible(true);
    // The same pinned browser arguments as every content WebView
    // (AUDIT V18-2): one user-data folder, one options set — see
    // build_content_view. (No user agent and no media probe here:
    // the chrome serves only Pico's internal pages.)
    #[cfg(windows)]
    let builder = {
        use wry::WebViewBuilderExtWindows;
        builder.with_additional_browser_args(pico_core::core::media::PICO_BROWSER_ARGS)
    };
    builder.build_as_child(&**window).map_err(|e| e.to_string())
}

pub fn run() {
    log::init();
    log::install_panic_hook();
    log::info(&format!(
        "Pico {DISPLAY_VERSION} starting (pid {})",
        std::process::id()
    ));
    log::info(&format!("dpi awareness: {}", crate::dpi::outcome()));
    // Portable data (AUDIT V27-C): the data directory was
    // resolved (and probed) when the log opened — pico.log
    // itself lives there. Say where everything lives, then run
    // the one-time appdata migration if the planner calls for
    // it. This MUST precede state loading and any WebView2
    // context creation (both happen in run_with_server), so a
    // migrated profile is the one the first window opens with.
    {
        let choice = pstore::data_dir_choice();
        if choice.portable {
            log::info(&format!("data: portable {}", choice.dir.display()));
        } else {
            log::info(&format!(
                "data: fallback appdata ({}) {}",
                choice.reason,
                choice.dir.display()
            ));
            // AUDIT 0.31 (A5-L11): say plainly when the
            // fallback lives on a network path (a UNC APPDATA
            // on a roaming profile) — state and profile I/O
            // then cross the network, which explains slow or
            // odd startup and must not be a silent surprise.
            if choice.dir.to_string_lossy().starts_with("\\\\") {
                log::info(
                    "data: note — the fallback data directory is on a network (UNC) path; state, log and profile are read/written over the network",
                );
            }
        }
        let appdata = pstore::appdata_pico_dir();
        let portable_has_state = pstore::state_path().is_file();
        if portable_has_state {
            // AUDIT V29 (A5-1): state present means any pending
            // marker is stale (a crash between the state copy
            // and the marker's removal) — retire it, or a later
            // state loss would mistake the live profile for a
            // partial copy and resume over it.
            let _ = std::fs::remove_file(choice.dir.join(pstore::MIGRATION_PENDING_NAME));
        }
        // AUDIT V29 (A5-7): the appdata facts are only STATTED
        // when they can matter. The planner stands down
        // whenever portable state exists, so evaluating the
        // appdata arguments eagerly on every startup touched
        // the appdata tree (a UNC-pointed APPDATA would mean an
        // SMB touch) for a migration that could not happen.
        let (appdata_has_state, appdata_has_profile) = if portable_has_state {
            (false, false)
        } else {
            (
                appdata.join("state.json").is_file(),
                pstore::appdata_webview_dir().is_dir(),
            )
        };
        let plan = pstore::plan_migration(
            choice,
            &appdata,
            portable_has_state,
            pstore::webview_data_dir().is_dir(),
            appdata_has_state,
            appdata_has_profile,
        );
        if let pstore::MigrationPlan::Copy { .. } | pstore::MigrationPlan::CopyStateOnly { .. } =
            plan
        {
            let report = pstore::perform_migration(&plan);
            if report.failed > 0 {
                // AUDIT V29 (A5-1): an incomplete migration is
                // logged as one — the plan re-issues next start.
                log::error(&format!(
                    "data: migration INCOMPLETE ({} failed, {} skipped; state marker withheld, will retry next start; originals untouched in appdata)",
                    report.failed, report.skipped
                ));
            } else {
                log::info(&format!(
                    "data: migrated from appdata (state {}, profile {} files, {} skipped)",
                    if report.state_copied {
                        "copied"
                    } else {
                        "absent"
                    },
                    report.profile_files,
                    report.skipped
                ));
            }
        }
    }

    match FileServer::start() {
        Ok(server) => run_with_server(server),
        Err(e) => {
            fatal_webview(
                "start the built-in file server (no free local port)",
                &e.to_string(),
            );
        }
    }
}

fn run_with_server(server: FileServer) {
    log::info(&format!("file server on 127.0.0.1:{}", server.port()));

    let load_outcome = pstore::load_from_checked(&pstore::state_path());
    let state_was_loaded = matches!(load_outcome, LoadOutcome::Loaded(_));
    let mut persist = match load_outcome {
        LoadOutcome::Loaded(s) => {
            log::info("state loaded");
            *s
        }
        LoadOutcome::Missing => {
            log::info("no state file yet; defaults in use");
            PersistState::default()
        }
        LoadOutcome::Corrupt(why) => {
            log::error(&format!(
                "state file unusable ({why}); defaults in use, file left untouched"
            ));
            PersistState::default()
        }
    };
    // AUDIT V28 (A3-M3): the sanitizer replaced an unknown
    // palette id — say so once, here, because the next save
    // makes the replacement permanent and this line is the
    // only trace of what the file used to say.
    if persist.theme_id_replaced {
        log::info("state: unknown palette id in state file; default palette applied");
        persist.theme_id_replaced = false;
    }
    // Palette migration (AUDIT V27-A2): an empty theme_id is a
    // pre-v27 state (the field did not exist). The user's order
    // was "use this as the new theme" (Forest, the first card
    // of his set), so a migrated install lands on Forest —
    // Violet remains one picker click away. A fresh install
    // already defaults to Forest in PersistState::default.
    if persist.theme_id.is_empty() {
        persist.theme_id = pico_core::core::themes::DEFAULT_PALETTE_ID.to_string();
        if state_was_loaded {
            log::info(
                "theme: pre-v27 state migrated to the forest palette (violet is in the theme picker)",
            );
            if let Err(e) = pstore::save(&persist) {
                log_save_failure(&e);
            }
        }
    }
    log::info(&format!(
        "theme: palette={} mode={}",
        persist.theme_id, persist.theme
    ));

    // The savings book (0.30): load the persisted lifetime,
    // then the one-time seed — a pre-0.30 file carries the
    // Shield's history in the legacy `blocked_total` only, and
    // the count carries over (bytes/time honestly start at
    // zero: they were never measured). From here the book is
    // the authority and `blocked_total` is its mirror.
    let stats_book = Arc::new(pico_core::core::stats::StatsBook::new());
    stats_book.load(persist.stats);
    if persist.stats.shield.blocked == 0 && persist.blocked_total > 0 {
        // AUDIT 0.31 (A5-L4): the seed applies only into a
        // COMPLETELY empty book; the log must say what actually
        // happened — through 0.30 this line claimed "carried
        // over" even when the book's gate refused the seed.
        if stats_book.seed_shield_blocked(persist.blocked_total) {
            log::info(&format!(
                "stats: pre-0.30 state seeded — shield lifetime count {} carried over (byte/time estimates start at zero)",
                persist.blocked_total
            ));
        } else {
            log::info(&format!(
                "stats: pre-0.30 seed NOT applied — the savings book already holds figures, so legacy count {} was left unseeded rather than merged",
                persist.blocked_total
            ));
        }
    }
    {
        let lt = stats_book.lifetime();
        log::info(&format!(
            "stats: shield blocked={} est_bytes={} lite blocked={}",
            lt.shield.blocked, lt.shield.est_bytes, lt.lite.blocked
        ));
    }

    let shared: Shared = Arc::new(Mutex::new(SharedData {
        services: Vec::new(),
        other_listeners: 0,
        recents: persist.recents.clone(),
        recents_hidden: persist.recents_hidden,
        theme: persist.theme.clone(),
        palette: persist.theme_id.clone(),
        stats: stats_book.view(),
    }));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // The runtime window icon (AUDIT V24-1): the Pico mark drawn
    // by scripts/gen_icon.py, handed to tao as raw RGBA at window
    // creation. This is the icon route with ZERO resource risk —
    // it involves no resource section at all — and it is what the
    // taskbar button shows. A malformed asset degrades to the
    // platform default icon plus a log line, never a failed launch.
    let window_icon = match tao::window::Icon::from_rgba(
        assets::ICON_RGBA_64.to_vec(),
        assets::ICON_SIZE,
        assets::ICON_SIZE,
    ) {
        Ok(icon) => Some(icon),
        Err(e) => {
            log::error(&format!(
                "window icon: asset rejected ({e}); default icon in use"
            ));
            None
        }
    };
    let icon_set = window_icon.is_some();

    // Restore the previous window geometry when we have a valid one
    // (AUDIT L7); tao clamps nothing itself, and WindowRect validation
    // at load keeps the values sane.
    let mut builder = WindowBuilder::new()
        .with_title("Pico")
        .with_window_icon(window_icon)
        .with_decorations(false)
        // No-flash startup (AUDIT V14-4): the window stays hidden
        // until the chrome and the first tab exist and are laid out,
        // so the user's first frame is the finished shell — never an
        // empty (black) window mid-construction.
        .with_visible(false)
        .with_min_inner_size(TaoLogicalSize::new(760.0, 500.0));
    let restore_maximized = match &persist.window {
        Some(rect) => {
            builder = builder
                .with_position(tao::dpi::LogicalPosition::new(rect.x as f64, rect.y as f64))
                .with_inner_size(TaoLogicalSize::new(rect.w as f64, rect.h as f64));
            rect.maximized
        }
        None => {
            builder = builder.with_inner_size(TaoLogicalSize::new(1180.0, 780.0));
            false
        }
    };
    let window = match builder.build(&event_loop) {
        Ok(w) => Rc::new(w),
        Err(e) => fatal_webview("create the main window", &e.to_string()),
    };
    if icon_set {
        log::info("window icon: set (runtime)");
    }
    // The window's own background is the theme surface too (AUDIT
    // V14-2): any pixel no webview covers — during the first layout,
    // a resize, a DPI change — shows the theme, not black.
    window.set_background_color(Some(theme_bg_rgba(&persist.theme_id, &persist.theme)));
    if restore_maximized {
        window.set_maximized(true);
    }

    // Media environment diagnostics (AUDIT V18-3): the runtime
    // version, the effective content user agent, and the profile
    // folder are the three facts every media/cookie question
    // starts from — put them on the record before anything loads.
    let runtime_version = webview_runtime_version();
    match &runtime_version {
        Some(v) => log::info(&format!("webview2 runtime version: {v}")),
        None => log::info("webview2 runtime version: unavailable (query failed)"),
    }
    let user_agent = pico_core::core::media::compose_user_agent(runtime_version.as_deref());
    log::info(&format!(
        "content user-agent ({}): {}",
        pico_core::core::media::UA_FORM,
        truncate_chars(&user_agent, 80)
    ));
    // The profile folder (AUDIT V18-3): derived once in core,
    // the same on every launch, never deleted by anything in Pico.
    let data_dir = pstore::webview_data_dir();
    log::info(&format!("webview data dir: {}", data_dir.display()));
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        log::error(&format!(
            "cannot create WebView2 data dir {}: {e}",
            data_dir.display()
        ));
    }
    // The remember-me witness (AUDIT V22-16): the profile above is
    // the one pinned folder every launch reuses; whether a cookies
    // database already exists inside it is the ground truth for
    // "a previous session's sign-ins persisted". Existence only —
    // the database's contents are never read.
    log::info(&format!(
        "profile: persistent, cookies db present={}",
        pico_core::core::media::cookies_db_present(&data_dir)
    ));
    let mut web_ctx = WebContext::new(Some(data_dir));

    // The chrome is the app's only control surface; if it cannot be
    // created there is no app — per-tab resilience (M6) starts below
    // this line, and the handshake watchdog (AUDIT V14-3) guards the
    // softer failure where creation succeeds but the page never
    // comes up.
    let (chrome_phys, _) = {
        let s = window.inner_size();
        layout::tile(s.width, s.height, window.scale_factor())
    };
    let chrome = match build_chrome(
        &window,
        &mut web_ctx,
        &shared,
        &proxy,
        phys_to_rect(chrome_phys),
    ) {
        Ok(v) => v,
        Err(e) => fatal_webview(
            "create the browser chrome (WebView2 runtime may be missing or damaged)",
            &e,
        ),
    };
    harden_webview_profile(&chrome, "chrome");
    log::info("chrome webview created (generation 1); awaiting ready handshake");

    let (watch_tx, watch_rx) = std::sync::mpsc::sync_channel::<(u64, Vec<PathBuf>)>(256);

    // The Shield (AUDIT V21): the runtime exists from the first
    // WebView (handlers consult it per request and fail open
    // while it is empty); the filter lists compile on a
    // background thread so window creation never waits on them.
    let shield = Arc::new(ShieldRuntime::new(persist.shield_on));
    let lite = Arc::new(LiteRuntime::new(persist.lite_on));
    log::info(&format!(
        "lite: {}",
        if persist.lite_on { "on" } else { "off" }
    ));
    log::info(&format!(
        "shield: {} (lists loading in background)",
        if persist.shield_on { "on" } else { "off" }
    ));
    {
        let shield_bg = shield.clone();
        let proxy_bg = proxy.clone();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            // Public build: no list content ships in the
            // binary. The lists come from the data/lists cache
            // or a first-run download of the four canonical
            // sources (core::lists owns the policy; winhttp_get
            // is the transport). One in-session retry after
            // 60 s; after that the Shield stays idle until the
            // next launch — the fail-open law throughout, so a
            // listless Pico is a working Pico.
            let cache_dir = pico_core::core::lists::lists_dir(&pstore::pico_dir());
            let fetch =
                |src: &pico_core::core::lists::ListSource| winhttp_get(src.url, src.max_bytes);
            let mut attempt = 0usize;
            loop {
                attempt += 1;
                match pico_core::core::lists::acquire_lists(&cache_dir, &fetch) {
                    pico_core::core::lists::AcquireOutcome::Ready(engine) => {
                        let engine = *engine;
                        let report = engine.report().clone();
                        let (script, info) = engine
                            .build_document_script(COSMETIC_SCRIPT_BUDGET, SCRIPTLET_SCRIPT_BUDGET);
                        log::info(&format!(
                            "shield: lists compiled — {} network, {} cosmetic ({} generic + {} specific on {} hosts), {} scriptlets ({} entries on {} hosts), {} skipped — in {}ms; document script {} bytes ({} cosmetic hosts, {} scriptlet hosts)",
                            report.network_rules + report.network_exceptions,
                            report.cosmetic_generic + report.cosmetic_specific_rules,
                            report.cosmetic_generic,
                            report.cosmetic_specific_rules,
                            report.cosmetic_specific_hosts,
                            report.scriptlet_entries,
                            report.scriptlet_entries,
                            report.scriptlet_hosts,
                            report.skipped_total(),
                            t0.elapsed().as_millis(),
                            info.bytes,
                            info.cosmetic_hosts,
                            info.scriptlet_hosts,
                        ));
                        shield_bg.install(engine, script);
                        break;
                    }
                    pico_core::core::lists::AcquireOutcome::Unavailable(why) => {
                        log::error(&format!(
                            "shield: lists unavailable ({why}); Shield idle until lists land"
                        ));
                        if attempt >= 2 {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(60));
                    }
                }
            }
            let _ = proxy_bg.send_event(UserEvent::ShieldReady);
        });
    }

    let mut app = App {
        window: window.clone(),
        web_ctx,
        chrome,
        tabs: Vec::new(),
        active: 0,
        next_tab_id: 1,
        server,
        persist,
        shared: shared.clone(),
        proxy: proxy.clone(),
        watchers: HashMap::new(),
        watch_tx: watch_tx.clone(),
        dedupe: Dedupe::new(512),
        services_count: 0,
        others_count: 0,
        blocked_downloads: 0,
        download_notice: String::new(),
        last_snap_sig: 0,
        chrome_gen: 1,
        chrome_ready: false,
        chrome_snapshot_logged: false,
        chrome_recreated: false,
        user_agent,
        probe_budget: Arc::new(Mutex::new(MediaProbeBudget::new())),
        shield,
        stats: stats_book,
        last_stats_sig: 0,
        lite,
        teleport: None,
        teleport_panel_open: false,
        teleport_version: 0,
        palette_panel_open: false,
        pending_focus_addr: false,
        live_start_tabs: std::sync::Arc::new(Mutex::new(std::collections::HashSet::new())),
        last_layout_sig: String::new(),
        expected_nav: std::sync::Arc::new(Mutex::new(None)),
    };

    // Startup tabs: open every command-line argument that resolves —
    // files, directories, URLs, bare ports (AUDIT L4: v12 opened
    // argv[1] only and silently dropped the rest).
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opened_any = false;
    for arg in &args {
        if arg.starts_with('-') {
            continue; // runtime switches, not content
        }
        if let Some(target) = parse_input(arg) {
            app.open_target(target);
            opened_any = true;
        } else {
            // AUDIT V28 (A5-L7): the raw argument went to the
            // log unredacted — a pasted URL's query (OAuth
            // codes, tokens) landed in pico.log verbatim.
            let shown = pico_core::core::redact::redact_url(arg);
            log::info(&format!(
                "startup argument ignored (not openable): {}",
                truncate_chars(&shown, 120)
            ));
        }
    }
    if !opened_any {
        app.add_tab(START_URL, true, true);
        // AUDIT V26 (D4): startup lands on a fresh start tab —
        // the address bar takes focus for immediate typing.
        app.pending_focus_addr = true;
        log::info("no openable startup arguments; start tab opened");
    }
    app.layout();

    // Shell complete: show the window and arm the chrome handshake
    // watchdog (AUDIT V14-3/V14-4). Log order is deliberate — pico.log
    // can reconstruct the whole startup sequence from these lines.
    window.set_visible(true);
    log::info("window shown (chrome + initial tab created and laid out)");
    // v16 layout witness: the merged compact header with the feather
    // overlap, straight from the single source of truth in
    // core::layout (the feather mode is "overlap" — the content
    // starts at SOLID_H, beneath the chrome's transparent strip).
    log::info(&format!(
        "layout: chrome_h={} feather=overlap",
        layout::CHROME_H as u32
    ));
    app.schedule_chrome_watchdog(app.chrome_gen);

    // File-watcher drain: coalesce bursts (editor saves fire dozens of
    // events), drop changes confined to ignored directories
    // (node_modules/.git), skip roots that already left the server
    // (AUDIT M2), and fire the per-root SSE broadcast immediately
    // (AUDIT M1); the UI thread then runs its fallback for tabs that
    // missed it.
    {
        let proxy = proxy.clone();
        let hub = app.server.reload_hub();
        let server = app.server.clone_handle();
        std::thread::spawn(move || loop {
            let first = match watch_rx.recv() {
                Ok(m) => m,
                Err(_) => return,
            };
            std::thread::sleep(Duration::from_millis(120));
            let mut pending: Vec<(u64, Vec<PathBuf>)> = vec![first];
            while let Ok(m) = watch_rx.try_recv() {
                pending.push(m);
            }
            // Group paths by root.
            let mut by_root: HashMap<u64, Vec<PathBuf>> = HashMap::new();
            for (root_id, paths) in pending {
                by_root.entry(root_id).or_default().extend(paths);
            }
            let mut changed = Vec::new();
            for (root_id, paths) in by_root {
                let Some(root_path) = server.root_path(root_id) else {
                    continue; // root left the server (M2 cleanup)
                };
                let relevant = paths.iter().any(|p| match p.strip_prefix(&root_path) {
                    Ok(rel) => is_relevant_change(rel),
                    Err(_) => true, // unknown provenance: reload, don't swallow
                });
                if relevant || paths.is_empty() {
                    changed.push(root_id);
                }
            }
            if changed.is_empty() {
                continue;
            }
            for root_id in &changed {
                hub.broadcast(*root_id);
            }
            let _ = proxy.send_event(UserEvent::FilesChanged {
                roots: changed,
                broadcast_at: Instant::now(),
            });
        });
    }

    spawn_detector(proxy.clone());
    log::info("detector started");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(user_event) => match user_event {
                UserEvent::ChromeCmd(raw) => app.on_chrome_cmd(&raw, control_flow),
                UserEvent::NavNote { tab, note } => app.on_nav_note(tab, note),
                UserEvent::SourceChanged { tab, url } => app.on_source_changed(tab, url),
                UserEvent::HistoryChanged { tab } => {
                    // (The tab id rides along for the log record
                    // Pico keeps of platform signals; the push
                    // itself is shell-global.)
                    let _ = tab;
                    // No state of our own changes — the history
                    // machine is fed by the URL reducer and by
                    // our own traversal calls (AUDIT V26 D7); the
                    // snapshot recomputes the nav buttons from it
                    // and the signature gate suppresses a no-op
                    // push.
                    app.push_state(false);
                }
                UserEvent::TitleChanged { tab, title } => {
                    let clean = truncate_chars(title.trim(), 512);
                    if !clean.is_empty() {
                        if let Some(t) = app.tab_mut(tab) {
                            if !t.is_start && t.title != clean {
                                t.title = clean;
                            }
                        }
                    }
                    app.push_state(false);
                }
                UserEvent::PageLoad { tab, finished, url } => app.on_page_load(tab, finished, url),
                UserEvent::NewWindow { tab, url: raw_url } => {
                    // AUDIT V28 A6-13: the event is keyed by its
                    // source tab like every other URL event — a
                    // popup queued by a tab that has since closed
                    // is dropped, not opened.
                    if app.tab_index(tab).is_none() {
                        log::info(&format!(
                            "new-window: dropped (source tab {tab} closed): {}",
                            redact_url(&raw_url)
                        ));
                    } else {
                        // The routing decision is core's pure function
                        // (AUDIT V22-10/15): allowlisted http/https
                        // opens a new tab (add_tab enforces the 30-tab
                        // cap and logs its own refusal); internal
                        // action URLs and every other scheme are denied
                        // with their reason. One pico.log line per
                        // decision either way, so a popup that "does
                        // nothing" is always explainable from the log.
                        match route_new_window(&raw_url) {
                            NewWindowRoute::OpenTab(url) => {
                                let is_start = url == START_URL;
                                if app.add_tab(&url, is_start, true) > 0 {
                                    log::info(&format!(
                                        "new-window: opened in new tab: {}",
                                        redact_url(&url)
                                    ));
                                } else {
                                    log::info(&format!(
                                        "new-window: popup refused (tab cap or allowlist): {}",
                                        redact_url(&url)
                                    ));
                                }
                            }
                            NewWindowRoute::Deny(reason) => {
                                log::info(&format!(
                                    "new-window: denied reason={reason} url={}",
                                    redact_url(&raw_url)
                                ));
                            }
                        }
                    }
                }
                UserEvent::StartAction { tab, source } => {
                    // The ONLY place start-page actions are decided
                    // (AUDIT V17-1): both transports land here and
                    // go through core::action::dispatch.
                    app.on_start_action(tab, source);
                }
                UserEvent::DownloadBlocked { tab, uri } => {
                    app.blocked_downloads += 1;
                    app.download_notice = truncate_chars(&uri, 80);
                    log::info(&format!(
                        "download denied (tab {tab}): {}",
                        redact_url(&uri)
                    ));
                    app.push_state(false);
                }
                UserEvent::DroppedPaths(paths) => {
                    for p in &paths {
                        app.open_path(p);
                    }
                }
                UserEvent::ServicesUpdated(outcome) => {
                    let other = outcome.other_listeners;
                    app.services_count = outcome.cards.len();
                    app.others_count = other;
                    {
                        let mut data = app
                            .shared
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        data.services = outcome.cards;
                        data.other_listeners = other;
                    }
                    app.push_services_to_start_tabs();
                    app.push_others_to_start_tabs();
                    app.push_state(false);
                }
                UserEvent::ChromeWatchdog(gen) => {
                    app.on_chrome_watchdog(gen);
                }
                UserEvent::ShieldReady => {
                    app.on_shield_ready();
                }
                UserEvent::ShieldBlocked => {
                    app.push_state(false);
                    app.push_stats_to_start_tabs(false);
                }
                UserEvent::LiteBlocked => {
                    app.push_state(false);
                    app.push_stats_to_start_tabs(false);
                }
                UserEvent::ShieldPrune { tab, items } => {
                    app.on_shield_prune(tab, &items);
                }
                UserEvent::ShieldPruned { tab, bytes, events } => {
                    app.on_shield_pruned(tab, bytes, events);
                }
                UserEvent::LiteChosen { tab } => {
                    app.on_lite_chosen(tab);
                }
                UserEvent::FilesChanged {
                    roots,
                    broadcast_at,
                } => {
                    // SSE already told every connected page of these
                    // roots to reload. Fallback for tabs that missed it
                    // (e.g. were mid-load with no SSE socket yet):
                    // reload them here unless they (re)started loading
                    // after the broadcast.
                    for t in &mut app.tabs {
                        let in_root = t.root_id.map(|r| roots.contains(&r)).unwrap_or(false);
                        if !in_root || t.view.is_none() {
                            continue;
                        }
                        if t.last_load_start.map(|s| s > broadcast_at).unwrap_or(false) {
                            continue;
                        }
                        if let Some(v) = t.view() {
                            let _ = v.reload();
                        }
                    }
                }
            },
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::Resized(_) => {
                    app.layout();
                    app.capture_geometry();
                    app.push_state(false);
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    app.layout();
                    app.push_state(false);
                }
                WindowEvent::CloseRequested => {
                    app.capture_geometry();
                    app.save_state();
                    *control_flow = ControlFlow::Exit;
                }
                _ => {}
            },
            _ => {}
        }
    });
}
