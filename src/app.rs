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
use pico_core::core::dlcmd::DOWNLOADS_URL;
use pico_core::core::dlcoord::{Coordinator, DlSettings};
use pico_core::core::fileopen::{resolve_open_target, OpenResolveError};
use pico_core::core::history::TabHistory;
use pico_core::core::ipc::{
    looks_like_lite_chosen, looks_like_shield_prune, looks_like_shield_pruned, parse_command,
    parse_lite_chosen, parse_nav_note, parse_shield_prune, parse_shield_pruned, url_has_userinfo,
    userinfo_adoption_allowed, userinfo_nav_allowed, Command, Dedupe, Envelope, NavNote, PruneItem,
    SocialSite, NAV_OBSERVER_SCRIPT,
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
    /// 0.33: the Downloads page's init JSON (the one page-data
    /// shape from the 0.33 contract), mirrored for downloads
    /// renders. `render_downloads` assembles the page from
    /// this string; `push_downloads_page` refreshes it
    /// whenever it rebuilds the JSON, so a fresh page load
    /// renders current data. 0.35.1: this JSON carries NO
    /// theme — an internal page has exactly two theme
    /// inputs, the INIT bake `render_downloads` merges in
    /// at serve time and the live `__picoTheme` push.
    downloads_init: String,
    /// 0.35.1: the start page's Downloads card model (the
    /// coordinator snapshot's `card`, [`DlCardView`]),
    /// mirrored whenever the app pushes state, so
    /// `render_start`'s INIT JSON carries the live card.
    /// Live updates ride `__picoSetDownloadsCard` pushes
    /// from `push_state`.
    downloads_card: pico_core::core::dlcoord::DlCardView,
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
        "downloads": data.downloads_card.clone(),
    });
    let init_json = serde_json::to_string(&init).unwrap_or_else(|_| "null".to_string());
    assets::assemble_start_html(&init_json)
}

/// The Downloads page's initial render (0.33): mirrors
/// `render_start` — the shared init JSON (already the one
/// page-data shape) is assembled into the page document.
/// 0.35.1: the CURRENT theme/palette are merged into the
/// INIT here, at assembly time, from the shared mirror
/// (`sync_shared` keeps them current) — exactly like
/// `render_start` bakes them. This serve-time bake and
/// the live `__picoTheme` push are the ONLY two theme
/// inputs an internal page has; the stored init JSON and
/// every data push carry no theme at all. A stored init
/// that fails to parse is served raw (never a panic).
fn render_downloads(shared: &Shared) -> String {
    let data = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let init_json = match serde_json::from_str::<serde_json::Value>(&data.downloads_init) {
        Ok(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "theme".to_string(),
                    serde_json::Value::String(data.theme.clone()),
                );
                obj.insert(
                    "palette".to_string(),
                    serde_json::Value::String(data.palette.clone()),
                );
            }
            serde_json::to_string(&v).unwrap_or_else(|_| data.downloads_init.clone())
        }
        Err(_) => data.downloads_init.clone(),
    };
    assets::assemble_downloads_html(&init_json)
}

/// Handler for the `pico://` custom protocol, registered on every
/// webview. Serves only three documents: the chrome, the start
/// page and (0.33) the Downloads page.
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
        } else if route == DOWNLOADS_URL {
            (200, render_downloads(&shared))
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

/// Register the outside-click panel dismissal on one content
/// view (0.32.4, Fix A): when the view's controller gains
/// focus, the user has clicked (or tabbed) INTO the page —
/// every chrome panel closes. This is the one signal that
/// can see that click: the panels live in the CHROME
/// webview's DOM, a plain click into the page goes to the
/// CONTENT webview (a separate HWND) and produces no
/// navigation, so no chrome-DOM handler or navigation event
/// ever observes it. The handler only forwards the fact as
/// [`UserEvent::ContentFocus`]; the event arm is the funnel.
/// Failures are logged, never fatal (the media probe's
/// outcome style): an older runtime may lack the event, and
/// the browser must not die for a panel law.
#[cfg(windows)]
fn attach_panel_focus_close(view: &WebView, tab: u64, proxy: EventLoopProxy<UserEvent>) {
    use webview2_com::FocusChangedEventHandler;
    use wry::WebViewExtWindows;

    let outcome = (|| -> Result<(), String> {
        let controller = view.controller();
        let handler = FocusChangedEventHandler::create(Box::new(move |_controller, _args| {
            let _ = proxy.send_event(UserEvent::ContentFocus { tab });
            Ok(())
        }));
        let mut token: i64 = 0;
        // SAFETY: the handler is a webview2-com callback
        // object kept alive by the controller's event table
        // once registered; the token cell is a plain local.
        unsafe {
            controller
                .add_GotFocus(&handler, &mut token)
                .map_err(|e| format!("GotFocus registration: {e}"))?;
        }
        Ok(())
    })();
    if let Err(why) = outcome {
        log::info(&format!(
            "panel focus close unavailable for tab {tab}: {why}"
        ));
    }
}

#[cfg(not(windows))]
fn attach_panel_focus_close(_view: &WebView, _tab: u64, _proxy: EventLoopProxy<UserEvent>) {}

/// Register the Ctrl+J downloads accelerator on one content
/// view (0.35, WS5): the controller's AcceleratorKeyPressed
/// event is the one signal that sees a key chord while the
/// PAGE holds focus — through 0.34 Ctrl+J lived only in the
/// chrome document's own key handler, so the chord did
/// nothing from content focus. The handler matches exactly
/// one chord (a KEY_DOWN with virtual key 0x4A — VK_J —
/// while Ctrl is held), marks it handled so the page never
/// sees it, and forwards [`UserEvent::DownloadsPageShortcut`];
/// key-up and system-key kinds are ignored, and every other
/// key passes through untouched. Registered ONLY on content
/// views (build_content_view) — the chrome keeps its own
/// handler, so the chord cannot double-fire. Failures are
/// logged, never fatal (the focus-close attach's outcome
/// style): an older runtime may lack the event, and the
/// browser must not die for a shortcut.
#[cfg(windows)]
fn attach_downloads_shortcut(view: &WebView, proxy: EventLoopProxy<UserEvent>) {
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    use wry::WebViewExtWindows;

    let outcome = (|| -> Result<(), String> {
        let controller = view.controller();
        let handler =
            AcceleratorKeyPressedEventHandler::create(Box::new(move |_controller, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                // SAFETY: the event args are valid for the
                // duration of this synchronous callback; every
                // call below is a documented getter/setter on
                // them, and a failed read simply leaves the key
                // untouched — the page keeps every chord this
                // handler does not positively identify as
                // Ctrl+J.
                unsafe {
                    let _ = (|| -> Result<(), String> {
                        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND(0);
                        args.KeyEventKind(&mut kind)
                            .map_err(|e| format!("KeyEventKind(): {e}"))?;
                        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN {
                            return Ok(()); // key-up / system-key: not ours
                        }
                        let mut vk: u32 = 0;
                        args.VirtualKey(&mut vk)
                            .map_err(|e| format!("VirtualKey(): {e}"))?;
                        if vk != 0x4A {
                            return Ok(()); // not VK_J
                        }
                        // Ctrl held reads as a negative i16 (the
                        // high bit is the "down" bit).
                        if GetKeyState(i32::from(VK_CONTROL)) >= 0 {
                            return Ok(());
                        }
                        // AUDIT 0.40.1: the chord is Ctrl+J
                        // exactly — with Shift or Alt also down
                        // it is a different chord (a page or
                        // DevTools shortcut) and stays the page's.
                        if GetKeyState(i32::from(VK_SHIFT)) < 0
                            || GetKeyState(i32::from(VK_MENU)) < 0
                        {
                            return Ok(());
                        }
                        args.SetHandled(true)
                            .map_err(|e| format!("SetHandled(): {e}"))?;
                        let _ = proxy.send_event(UserEvent::DownloadsPageShortcut);
                        Ok(())
                    })();
                }
                Ok(())
            }));
        let mut token: i64 = 0;
        // SAFETY: the handler is a webview2-com callback
        // object kept alive by the controller's event table
        // once registered; the token cell is a plain local.
        unsafe {
            controller
                .add_AcceleratorKeyPressed(&handler, &mut token)
                .map_err(|e| format!("AcceleratorKeyPressed registration: {e}"))?;
        }
        Ok(())
    })();
    if let Err(why) = outcome {
        log::info(&format!("downloads shortcut unavailable: {why}"));
    }
}

#[cfg(not(windows))]
fn attach_downloads_shortcut(_view: &WebView, _proxy: EventLoopProxy<UserEvent>) {}

/// Register Pico's own context menu on one content view
/// (0.34.1): WebView2's `ContextMenuRequested` is intercepted
/// and the stock menu is ALWAYS suppressed (`SetHandled`
/// first, unconditionally — even a failed fact read must
/// never fall back to the unthemed stock menu with its
/// "Save as" / "Print" verbs). The handler only reads the
/// target facts (kind, page/link/source URLs, selection,
/// editable, clipboard-text presence) and forwards them as
/// [`UserEvent::ContextMenu`]; the event loop owns the menu
/// (core::ctxmenu decides the items, the chrome draws them,
/// picks return as `ctx_pick` commands). Failures are
/// logged, never fatal (the focus-close attach's outcome
/// style): an older runtime may lack the event, and the
/// browser must not die for a menu.
#[cfg(windows)]
fn attach_context_menu(view: &WebView, tab: u64, proxy: EventLoopProxy<UserEvent>) {
    use pico_core::core::ctxmenu::{CtxKind, CtxTarget};
    use webview2_com::ContextMenuRequestedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_11, COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND,
    };
    use windows_core::Interface;
    use wry::WebViewExtWindows;

    let outcome = (|| -> Result<(), String> {
        // The ContextMenuRequested event hangs off
        // ICoreWebView2_11 (not the base interface) in the
        // WebView2 API.
        let core = view
            .webview()
            .cast::<ICoreWebView2_11>()
            .map_err(|e| format!("cast to ICoreWebView2_11: {e}"))?;
        let handler = ContextMenuRequestedEventHandler::create(Box::new(move |_sender, args| {
            let Some(args) = args else {
                return Ok(());
            };
            // SAFETY: the event args and the target object they
            // hand out are valid for the duration of this
            // synchronous callback; every PWSTR getter follows
            // the established pattern (`take_pwstr` takes
            // ownership of the CoTaskMem string), and a failed
            // read degrades to an empty/absent fact rather than
            // an error — the stock menu stays handled no matter
            // what happens after it.
            unsafe {
                // FIRST, unconditionally: the stock menu must
                // never render, even if every read below fails.
                let _ = args.SetHandled(true);
                let _ = (|| -> Result<(), String> {
                    let target = args
                        .ContextMenuTarget()
                        .map_err(|e| format!("ContextMenuTarget(): {e}"))?;
                    let mut kind_raw = COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND(0);
                    target
                        .Kind(&mut kind_raw)
                        .map_err(|e| format!("Kind(): {e}"))?;
                    // One PWSTR getter, the established pattern
                    // (see fetch_download_cookies): a failed/null
                    // read is an empty string, never fatal.
                    macro_rules! target_str {
                        ($getter:ident) => {{
                            let mut ptr = windows_core::PWSTR::null();
                            match target.$getter(&mut ptr) {
                                Ok(()) if !ptr.is_null() => webview2_com::take_pwstr(ptr),
                                _ => String::new(),
                            }
                        }};
                    }
                    // One BOOL getter, same discipline: a failed
                    // read is false.
                    macro_rules! target_bool {
                        ($getter:ident) => {{
                            let mut v = windows_core::BOOL::default();
                            match target.$getter(&mut v) {
                                Ok(()) => v.as_bool(),
                                Err(_) => false,
                            }
                        }};
                    }
                    let has_link = target_bool!(HasLinkUri);
                    let has_source = target_bool!(HasSourceUri);
                    let mut pt = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
                    // The Location out-pointer is typed in the
                    // `windows` crate's POINT; the windows-sys
                    // POINT above is the same repr(C) {x, y} pair,
                    // so the cast only reinterprets that layout.
                    args.Location((&mut pt as *mut windows_sys::Win32::Foundation::POINT).cast())
                        .map_err(|e| format!("Location(): {e}"))?;
                    let facts = CtxTarget {
                        kind: CtxKind::from_webview2(kind_raw.0),
                        page_url: target_str!(PageUri),
                        link_url: if has_link {
                            Some(target_str!(LinkUri))
                        } else {
                            None
                        },
                        source_url: if has_source {
                            Some(target_str!(SourceUri))
                        } else {
                            None
                        },
                        has_selection: target_bool!(HasSelection),
                        is_editable: target_bool!(IsEditable),
                        has_clipboard_text:
                            windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable(
                                13, // CF_UNICODETEXT
                            ) != 0,
                    };
                    let _ = proxy.send_event(UserEvent::ContextMenu {
                        tab,
                        target: facts,
                        location: (pt.x, pt.y),
                    });
                    Ok(())
                })();
            }
            Ok(())
        }));
        let mut token: i64 = 0;
        // SAFETY: the handler is a webview2-com callback object
        // kept alive by the core's event table once registered;
        // the token cell is a plain local.
        unsafe {
            core.add_ContextMenuRequested(&handler, &mut token)
                .map_err(|e| format!("ContextMenuRequested registration: {e}"))?;
        }
        Ok(())
    })();
    if let Err(why) = outcome {
        log::info(&format!("context menu unavailable for tab {tab}: {why}"));
    }
}

#[cfg(not(windows))]
fn attach_context_menu(_view: &WebView, _tab: u64, _proxy: EventLoopProxy<UserEvent>) {}

/// Set the system clipboard's Unicode text (0.34.1): the
/// context menu's Copy Link / Copy Image Address verbs. The
/// Win32 sequence is the documented one — open, empty, hand
/// over a moveable global allocation holding the UTF-16 text
/// plus its NUL. Once `SetClipboardData` succeeds the SYSTEM
/// owns the handle and it must NOT be freed here; every
/// failure path before that still owns it and frees it.
/// Outcome-logged like harden_webview_profile; the text
/// itself never reaches the log (a copied link can carry a
/// query) — only its length does.
#[cfg(windows)]
fn clipboard_set_text(text: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };

    const CF_UNICODETEXT: u32 = 13;
    let outcome = (|| -> Result<(), String> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: every handle is null-checked before use; the
        // clipboard is closed on every path past a successful
        // open; the allocation is freed on every failure path
        // and deliberately NOT freed once SetClipboardData
        // succeeds (ownership moves to the system with it).
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return Err("OpenClipboard failed".to_string());
            }
            let result = (|| -> Result<(), String> {
                if EmptyClipboard() == 0 {
                    return Err("EmptyClipboard failed".to_string());
                }
                let handle = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
                if handle.is_null() {
                    return Err("GlobalAlloc failed".to_string());
                }
                let lock = GlobalLock(handle) as *mut u16;
                if lock.is_null() {
                    GlobalFree(handle);
                    return Err("GlobalLock failed".to_string());
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), lock, wide.len());
                GlobalUnlock(handle);
                if SetClipboardData(CF_UNICODETEXT, handle).is_null() {
                    GlobalFree(handle);
                    return Err("SetClipboardData failed".to_string());
                }
                Ok(())
            })();
            CloseClipboard();
            result
        }
    })();
    match outcome {
        Ok(()) => {
            log::info(&format!(
                "clipboard: text set ({} chars)",
                text.chars().count()
            ));
            Ok(())
        }
        Err(why) => {
            log::info(&format!("clipboard set unavailable: {why}"));
            Err(why)
        }
    }
}

#[cfg(not(windows))]
fn clipboard_set_text(_text: &str) -> Result<(), String> {
    Err("clipboard is Windows-only".to_string())
}

/// Read the system clipboard's Unicode text (0.34.1): the
/// context menu's Paste verb. `None` for "no text format", a
/// failed open, or an unreadable handle alike — Paste simply
/// does nothing then. The text is page-bound input the
/// caller inserts through a JSON-escaped script argument,
/// and it never reaches the log.
#[cfg(windows)]
fn clipboard_get_text() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;
    // SAFETY: the format gate runs before the open; every
    // handle is null-checked before use; the lock is released
    // and the clipboard closed on every path past a successful
    // open; and the read is bounded by the allocation's own
    // size (plus a hard cap), never by trusting the buffer to
    // be NUL-terminated — though CF_UNICODETEXT data is by
    // contract.
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
            return None;
        }
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            log::info("clipboard get unavailable: OpenClipboard failed");
            return None;
        }
        let result = (|| -> Option<String> {
            let handle = GetClipboardData(CF_UNICODETEXT);
            if handle.is_null() {
                return None;
            }
            let lock = GlobalLock(handle) as *const u16;
            if lock.is_null() {
                return None;
            }
            let cap = (GlobalSize(handle) / 2).min(1_000_000);
            let mut len = 0usize;
            while len < cap && *lock.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(lock, len));
            GlobalUnlock(handle);
            Some(text)
        })();
        CloseClipboard();
        result
    }
}

#[cfg(not(windows))]
fn clipboard_get_text() -> Option<String> {
    None
}

/// The cursor's position in window DIP (0.34.1): the ground
/// truth `core::ctxmenu::resolve_anchor` resolves the
/// WebView2 location against. Screen-physical cursor minus
/// the client area's screen origin, divided by the scale —
/// exactly ScreenToClient's translation, computed from tao's
/// `inner_position` (the client area's own screen origin)
/// because windows-sys 0.59 gates ScreenToClient itself
/// behind the not-enabled Win32_Graphics_Gdi feature.
/// `None` on any failure (no cursor read, no window
/// position, a degenerate scale): the anchor resolution
/// then falls back to its documented default reading.
#[cfg(windows)]
fn cursor_client_dip(window: &Window, scale: f64) -> Option<(f64, f64)> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let mut pt = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    // SAFETY: `pt` is a plain local out-cell — the documented
    // GetCursorPos pattern, nothing else is touched.
    if unsafe { GetCursorPos(&mut pt) } == 0 {
        return None;
    }
    let origin = window.inner_position().ok()?;
    Some((
        (pt.x - origin.x) as f64 / scale,
        (pt.y - origin.y) as f64 / scale,
    ))
}

#[cfg(not(windows))]
fn cursor_client_dip(_window: &Window, _scale: f64) -> Option<(f64, f64)> {
    None
}

/// Fetch a captured download's cookies from its tab's
/// WebView (0.32; token shape 0.34): the page's session
/// cookies for the download URL are what let logged-in
/// downloads work. The chain is the profile route —
/// `view.webview()` → `ICoreWebView2_13::Profile()` →
/// `ICoreWebView2Profile5::CookieManager()` → `GetCookies` —
/// with webview2-com's own `GetCookiesCompletedHandler` wrapper
/// (the crate ships one, so no hand-rolled `implement` is
/// needed). The handler normalizes each cookie into a core
/// `CookiePair` (domain: leading `.` stripped, lowercased;
/// cookies with an empty name skipped) and posts it back as
/// [`UserEvent::DownloadCookies`], carrying the capture
/// `token` the coordinator minted and the source `tab` (the
/// landing revalidation needs the tab's current facts).
/// `host_only` is derived from
/// the RAW domain: WebView2 exposes no host-only getter, and a
/// domain cookie is the one whose raw domain carries the
/// leading dot (a domain cookie is sent to subdomains; a
/// host-only cookie's domain is the bare host). Cookie VALUES
/// never touch the log — this function's errors name only the
/// failed step.
///
/// Any failure returns `Err` and the caller lands the capture
/// immediately with no cookies: a download must never hang on
/// its session fetch.
#[cfg(windows)]
fn fetch_download_cookies(
    view: &WebView,
    uri: &str,
    tab: u64,
    token: u64,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<(), String> {
    use webview2_com::GetCookiesCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile5, ICoreWebView2_13};
    use windows_core::Interface;
    use wry::WebViewExtWindows;

    let core = view
        .webview()
        .cast::<ICoreWebView2_13>()
        .map_err(|e| format!("cast to ICoreWebView2_13: {e}"))?;
    // SAFETY: `Profile()` on the live core returns the profile
    // object for this view's environment (same getter the
    // profile-hardening path above uses).
    let profile = unsafe { core.Profile() }.map_err(|e| format!("Profile(): {e}"))?;
    let profile5 = profile
        .cast::<ICoreWebView2Profile5>()
        .map_err(|e| format!("cast to ICoreWebView2Profile5: {e}"))?;
    // SAFETY: `CookieManager()` is a documented getter on the
    // live profile just obtained from the core.
    let manager =
        unsafe { profile5.CookieManager() }.map_err(|e| format!("CookieManager(): {e}"))?;
    let handler = GetCookiesCompletedHandler::create(Box::new(move |error_code, cookies| {
        let mut out: Vec<pico_core::core::download::CookiePair> = Vec::new();
        if error_code.is_ok() {
            if let Some(list) = cookies {
                // SAFETY: the cookie list and its entries are
                // valid for the duration of this synchronous
                // completion callback; every getter follows the
                // established PWSTR pattern (`take_pwstr` takes
                // ownership of the CoTaskMem string).
                unsafe {
                    let mut count: u32 = 0;
                    if list.Count(&mut count).is_ok() {
                        for i in 0..count {
                            let Ok(cookie) = list.GetValueAtIndex(i) else {
                                continue;
                            };
                            // One PWSTR getter, the established
                            // pattern: a failed/null read is an
                            // empty string, never a fatal error.
                            macro_rules! cookie_str {
                                ($getter:ident) => {{
                                    let mut ptr = windows_core::PWSTR::null();
                                    match cookie.$getter(&mut ptr) {
                                        Ok(()) if !ptr.is_null() => webview2_com::take_pwstr(ptr),
                                        _ => String::new(),
                                    }
                                }};
                            }
                            let name = cookie_str!(Name);
                            if name.is_empty() {
                                continue;
                            }
                            let value = cookie_str!(Value);
                            let raw_domain = cookie_str!(Domain);
                            let path = cookie_str!(Path);
                            let mut secure = windows_core::BOOL::default();
                            let _ = cookie.IsSecure(&mut secure);
                            // AUDIT 0.32.1 (P2-4): the expiry
                            // rides along (OLE DATE → unix ms in
                            // core) so an already-deleted cookie
                            // (Expires in the past is how servers
                            // delete) is never replayed by the
                            // download engine.
                            let mut expires_raw: f64 = 0.0;
                            let _ = cookie.Expires(&mut expires_raw);
                            out.push(pico_core::core::download::CookiePair {
                                name,
                                value,
                                domain: raw_domain
                                    .strip_prefix('.')
                                    .unwrap_or(&raw_domain)
                                    .to_lowercase(),
                                host_only: !raw_domain.starts_with('.'),
                                secure: secure.as_bool(),
                                path,
                                expires_unix_ms: pico_core::core::download::cookie_expiry_unix_ms(
                                    expires_raw,
                                ),
                            });
                        }
                    }
                }
            }
        }
        let _ = proxy.send_event(UserEvent::DownloadCookies {
            tab,
            token,
            cookies: out,
        });
        Ok(())
    }));
    let uri_wide: Vec<u16> = uri.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the URI buffer outlives the call, the handler is
    // the crate's own completed-handler object, and the manager
    // is the live profile's cookie manager obtained above.
    unsafe {
        manager
            .GetCookies(windows_core::PCWSTR(uri_wide.as_ptr()), &handler)
            .map_err(|e| format!("GetCookies: {e}"))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn fetch_download_cookies(
    _view: &WebView,
    _uri: &str,
    _tab: u64,
    _token: u64,
    _proxy: EventLoopProxy<UserEvent>,
) -> Result<(), String> {
    Err("cookie fetch is Windows-only".to_string())
}

/// Prepare one canonical downloads body for
/// `core::dlcmd::parse_dl_body` (0.34). ONE transport
/// adaptation lives here, in the app layer only — the core
/// parser stays frozen:
///
/// - The chrome's `send()` wraps every body in the IPC
///   envelope (`v` / `boot` / `req` ride alongside
///   `kind`/`action`). Those three keys are chrome
///   TRANSPORT framing, not downloads payload, so they are
///   stripped before the strict parse (which denies
///   unknown fields). Downloads-page bodies carry no
///   envelope; stripping is a no-op for them.
///
/// The connection-count wire name is `conn` on every
/// surface (the JS emitters' spelling; `core::dlcmd`'s
/// parser expects exactly it since the 0.34 integration
/// settlement) — no field renaming happens here.
///
/// A body that is not a JSON object passes through
/// unchanged — the strict parser denies it there.
fn normalize_dl_body(raw: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_string();
    };
    let Some(obj) = v.as_object_mut() else {
        return raw.to_string();
    };
    obj.remove("v");
    obj.remove("boot");
    obj.remove("req");
    serde_json::to_string(&v).unwrap_or_else(|_| raw.to_string())
}

/// The chrome envelope identity (`boot`, `req`) of one raw
/// chrome body, when it carries both: dl bodies ride the
/// chrome transport, whose double-fire protection is the
/// `(boot, req)` dedupe — the app checks it for dl bodies
/// exactly as for `Command` envelopes (see
/// `App::on_chrome_dl_body`).
fn dl_envelope_id(raw: &str) -> Option<(String, u64)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let boot = v.get("boot")?.as_str()?.to_string();
    let req = v.get("req")?.as_u64()?;
    Some((boot, req))
}

/// Open the downloads FOLDER in Explorer (0.32): ShellExecuteW
/// with the "open" verb on the folder path only — never a file,
/// never a URL (the `download_open_folder` command carries no
/// path; the folder is the manager's own resolved directory).
#[cfg(windows)]
fn open_folder_in_explorer(dir: &Path) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let verb = wide("open");
    let file = wide(&dir.to_string_lossy());
    // SAFETY: both buffers are NUL-terminated and outlive the
    // call; no parent window, no parameters, no working
    // directory override. (windows-sys 0.59 spells PCWSTR as a
    // raw `*const u16`, so the buffers pass as plain pointers.)
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW reports success as a value above 32.
    if (rc as isize) <= 32 {
        log::error(&format!(
            "downloads: folder open failed (ShellExecuteW rc {})",
            rc as isize
        ));
    }
}

#[cfg(not(windows))]
fn open_folder_in_explorer(_dir: &Path) {}

/// Open one completed download FILE (0.33): ShellExecuteW with
/// the "open" verb on the file path only. The path is the
/// manager's own `completed_path` for a task id — never a
/// page-supplied path, never a URL.
#[cfg(windows)]
fn open_file_in_explorer(file: &Path) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let verb = wide("open");
    let file_w = wide(&file.to_string_lossy());
    // SAFETY: both buffers are NUL-terminated and outlive the
    // call; no parent window, no parameters, no working
    // directory override.
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW reports success as a value above 32.
    if (rc as isize) <= 32 {
        log::error(&format!(
            "downloads: file open failed (ShellExecuteW rc {})",
            rc as isize
        ));
    }
}

#[cfg(not(windows))]
fn open_file_in_explorer(_file: &Path) {}

/// Probe: can Pico's downloads live beside the exe? Mirrors
/// `core::state::probe_exe_dir`'s idea for the `Downloads`
/// subfolder (0.32): create it, then round-trip a temp file
/// including a RENAME (state saves and `.part` completion are
/// rename-shaped work). Any failure is simply "not writable" —
/// `core::download::resolve_downloads_dir` then falls back to
/// the user's Downloads folder.
fn probe_downloads_dir(exe_dir: &Path) -> bool {
    let dir = exe_dir.join("Downloads");
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".pico-probe-{}", std::process::id()));
    let probe2 = dir.join(format!(".pico-probe-{}-renamed", std::process::id()));
    let ok = std::fs::write(&probe, b"pico").is_ok() && std::fs::rename(&probe, &probe2).is_ok();
    let _ = std::fs::remove_file(&probe);
    let _ = std::fs::remove_file(&probe2);
    ok
}

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
/// headers (the session's plain `Pico/0.40.1` User-Agent only),
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
        let ua_w = wide("Pico/0.40.1");
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
            let connect = WinHttpHandle(WinHttpConnect(session.0, host_w.as_ptr(), port, 0));
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
            if WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0 {
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
    /// A Downloads-page action request (0.33; body shape
    /// 0.34), from the page's IPC channel: the raw canonical
    /// `{"kind":"dl", ...}` body, forwarded undecided.
    /// Decided centrally by `core::dlcmd::parse_dl_body` +
    /// the coordinator in `on_downloads_page_body` — the
    /// handler that produces this event makes no decisions.
    DownloadsAction {
        tab: u64,
        body: String,
    },
    /// A download was requested by the CHROME webview and denied
    /// (0.32: content tabs route to the download manager via
    /// [`UserEvent::DownloadStarting`] instead; the chrome serves
    /// only Pico's internal pages, so a download attempt there is
    /// still a flat denial — its event arm now only logs. No tab
    /// field: the chrome is not a tab, and the pre-0.32 dummy
    /// `tab: 0` carried no information).
    DownloadBlocked {
        uri: String,
    },
    /// A content tab asked WebView2 to download (0.32): the wry
    /// handler cancelled it (returning false always) and carries
    /// the URI plus the suggested file name (the handler path's
    /// file name, lossy; empty when it had none). The event arm
    /// validates the URL, registers a pending download, and
    /// starts the cookie fetch from that tab's webview.
    DownloadStarting {
        tab: u64,
        uri: String,
        suggested: String,
    },
    /// A content webview gained focus (0.32.4): the user
    /// clicked (or tabbed) into the page — every chrome
    /// panel closes. Sent by the GotFocus registration in
    /// build_content_view; the arm is the panel law's
    /// outside-interaction funnel.
    ContentFocus {
        tab: u64,
    },
    /// Ctrl+J was pressed while a CONTENT webview held
    /// focus (0.35, WS5): sent by the AcceleratorKeyPressed
    /// registration in build_content_view — through 0.34
    /// the chord only worked when the CHROME document had
    /// focus (its own key handler), so Ctrl+J from the page
    /// did nothing. The arm opens the downloads page
    /// through the same `App::open_downloads_page` the
    /// coordinator's OpenDownloadsPage effect uses.
    DownloadsPageShortcut,
    /// A content page raised WebView2's context-menu request
    /// (0.34.1): the registration handler already suppressed
    /// the stock menu (SetHandled, unconditionally) and
    /// carries the target facts plus the event's raw location
    /// here, undecided — the event arm computes the anchor
    /// (`core::ctxmenu::resolve_anchor`), the items
    /// (`core::ctxmenu::menu_items`) and the card position
    /// (`core::layout::ctx_menu_pos`), and stores the one
    /// open menu in App state.
    ContextMenu {
        tab: u64,
        target: pico_core::core::ctxmenu::CtxTarget,
        location: (i32, i32),
    },
    /// A captured download's cookies arrived from its tab's
    /// WebView2 cookie manager (0.32; token shape 0.34).
    /// Whichever of this and
    /// [`UserEvent::DownloadCookiesTimeout`] arrives first
    /// lands the capture in the coordinator; the loser is
    /// logged and ignored there. `tab` rides along because
    /// the coordinator's landing revalidation needs the
    /// tab's CURRENT facts (alive / navigation epoch /
    /// only-tab), which only the app can supply.
    DownloadCookies {
        tab: u64,
        token: u64,
        cookies: Vec<pico_core::core::download::CookiePair>,
    },
    /// A captured download's cookie fetch did not answer in
    /// time (0.32; token shape 0.34): the task is created
    /// with no cookies rather than not at all.
    DownloadCookiesTimeout {
        tab: u64,
        token: u64,
    },
    /// The download manager's visible state changed (0.32):
    /// emitted by the manager on every transition and (throttled)
    /// on progress; the arm only re-pushes the snapshot — the
    /// signature gate decides whether anything is sent.
    DownloadsChanged,
    /// The download notice's display window ended (0.32.3;
    /// coordinator-owned 0.34): fired by the one-shot timer
    /// thread the app arms when a coordinator notice opens
    /// (the cookie timeout's pattern). Carries the notice's
    /// sequence number — a newer notice is never retired by
    /// an older notice's timer (the coordinator checks).
    DownloadNoticeExpired {
        seq: u64,
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
    /// True when this tab is the Downloads page (0.33):
    /// `pico://downloads`, classified live through
    /// `live_downloads_tabs` like `is_start` is through its
    /// set.
    is_downloads: bool,
    /// The tab's navigation epoch (0.34): the count of real
    /// page commits this tab has applied, bumped ONLY at
    /// the single commit point in `apply_url_event`
    /// (`HistEffect::Commit`) — never for a swallowed
    /// download finish, which returns before the reducer
    /// runs. A download capture records the epoch at
    /// capture time; the coordinator revalidates the tab's
    /// fate against it at landing — a tab whose epoch moved
    /// has committed a real page since and is the user's
    /// tab now. (This is 0.33's `commits` counter under its
    /// coordinator name; the 0.33 navigation guard and
    /// finish-swallow marker it worked beside are gone —
    /// the coordinator's capture records own that state
    /// now.)
    nav_epoch: u64,
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
    /// True when this tab was opened by a PAGE (the
    /// new-window/popup adoption path), not by the user
    /// (0.32.4). Runtime-only: tabs are rebuilt every
    /// session, nothing persists it. Cleared the moment
    /// the tab commits a real page of its own — from then
    /// on it is a normal tab.
    adopted: bool,
    /// The current unsettled navigation's URL chain
    /// (0.35, WS2): every URL the in-flight navigation
    /// has occupied — its initial target (the
    /// `CommitStart` URL) plus each redirect hop observed
    /// through SourceChanged while the tab is loading.
    /// Maintained in `apply_url_event` (see its chain
    /// block for the event-by-event rule): a CommitStart
    /// RESETS it to that Start's URL, a Source URL
    /// observed while loading APPENDS (consecutive
    /// duplicates deduped), and a real CommitFinish
    /// CLEARS it — the navigation settled into a page, so
    /// the record must hold nothing from it for the next
    /// navigation. A download capture assembles its
    /// `CaptureFacts.chain` from this record: through
    /// 0.34 the capture carried only the post-redirect
    /// URI while the tab's live URL stayed on the
    /// pre-redirect hop, and the fate gate's exact match
    /// between the two could never pass for a redirected
    /// navigation.
    pending_chain: Vec<String>,
    /// The tab's `adopted` flag AS OF the pending
    /// navigation's start (0.35, WS2): snapshotted when
    /// the chain resets (and when its first Source hop is
    /// recorded). A real Commit CLEARS `adopted`, so a
    /// captured navigation whose own Finished landed
    /// BEFORE the capture (the WebView2Feedback-#5172
    /// shape) has already cleared the live flag when the
    /// capture reads its facts — this snapshot is where
    /// `on_download_starting` recovers the origin the
    /// navigation actually started with. A CommitFinish
    /// clears `pending_chain` but deliberately NOT this
    /// snapshot: it still describes the navigation that
    /// just settled, which is exactly the one a
    /// just-late capture belongs to. The next
    /// navigation's start overwrites it.
    pending_adopted: bool,
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
    /// The downloads coordinator (0.34): the pure core state
    /// machine that owns downloads coordination — capture
    /// triage and tab fate, the cookie race, the notice
    /// machine, command dispatch — over the real manager
    /// through the `dlbackend` adapter. The app feeds it
    /// events and executes the `DlEffect`s it returns; the
    /// 0.32–0.33 app-side patch stack it replaces (pending
    /// downloads, the notice fields, the status-diff map)
    /// is gone. Constructed once at startup with the
    /// resolved downloads folder (inside the manager) and
    /// the persisted downloads settings.
    dl: Coordinator<crate::dlbackend::ManagerBackend>,
    /// Whether the Downloads panel is open (0.32): same overlay
    /// mechanics as Teleport's and the picker's panels, and
    /// mutually exclusive with them — opening one closes the
    /// others. App-owned UI state; the coordinator only
    /// REQUESTS opens/closes through its effects.
    downloads_panel_open: bool,
    /// The coordinator notice sequence the app last armed an
    /// expiry timer for (0.34): a notice is armed exactly
    /// once per sequence, when it is first seen open after
    /// executing effects — an older timer can never retire
    /// a newer notice because the coordinator seq-checks
    /// expiry itself.
    dl_notice_armed_seq: u64,
    /// Signature of the last Downloads-page push (0.33):
    /// (snapshot version, settings tuple) hashed — a
    /// gated-out push costs one hash, like the chrome
    /// snapshot's. (0.35.1: theme left this hash — data
    /// pushes no longer carry theme at all.)
    last_dl_page_sig: u64,
    /// Signature of the last Downloads-card push to the
    /// start tabs (0.35.1): a hash of the serialized card
    /// JSON, so `push_state` re-sends the card only when
    /// the card itself changed — speed ticks move it at
    /// the popover's cadence, idle pushes cost one hash.
    last_dl_card_sig: u64,
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
    /// Pico's own context menu (0.34.1): the one open menu's
    /// stored state, or None. An overlay like the panels —
    /// while it is open the chrome rect grows to contain the
    /// card (see chrome_phys) — and mutually exclusive with
    /// them: opening it closes every panel through the one
    /// funnel, and opening any panel clears it.
    context_menu: Option<CtxMenuState>,
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
    /// The live set of Downloads-page tab ids (0.33): the
    /// mirror of `live_start_tabs` for `pico://downloads`,
    /// shared with the WebView navigation / IPC handlers so
    /// their downloads classification is live, not the
    /// build-time flag.
    live_downloads_tabs: std::sync::Arc<Mutex<std::collections::HashSet<u64>>>,
    /// Last layout signature logged (AUDIT V26, defect D2): the
    /// layout witness line is written when the computed geometry
    /// changes, so pico.log can answer "what rects did the shell
    /// actually apply" without logging every pass.
    last_layout_sig: String,
    /// One-shot shell navigation expectations (AUDIT 0.31,
    /// A6-F2), keyed PER TAB (AUDIT 0.32.1, P3-3: through
    /// 0.32 a single global slot meant arming tab B silently
    /// disarmed tab A): `tab_id → exact URL` the shell itself
    /// most recently submitted for that tab from USER-TYPED
    /// text. A same-tab navigation to a credentialed
    /// (userinfo) URL is allowed only when it matches this
    /// exactly; the expectation is consumed by the first
    /// navigation event for that tab, matched or not.
    /// Page-initiated credentialed navigations never set it,
    /// so they never match — typed credentialed URLs stay
    /// allowed as the user's own act, page-minted ones are
    /// denied. Shared (like `live_start_tabs`) because the
    /// synchronous navigation handlers — not the event loop —
    /// consume it.
    expected_nav: std::sync::Arc<Mutex<std::collections::HashMap<u64, String>>>,
    /// Credentialed-URL ADOPTION markers (AUDIT 0.32.1,
    /// P3-1): `tab_id → exact URL` set by the navigation
    /// handler at the moment it ALLOWS a userinfo navigation
    /// on the expectation match. `on_page_load` re-checks
    /// only the scheme allowlist, so without this marker a
    /// credentialed URL arriving via a server redirect
    /// (which may not re-run the navigation handler as a
    /// fresh user gesture) would be ADOPTED into tab state /
    /// the address bar without ever passing the expectation
    /// gate. A load carrying userinfo is adopted only when
    /// it equals the marker; a load without userinfo clears
    /// the tab's marker.
    userinfo_adopt: std::sync::Arc<Mutex<std::collections::HashMap<u64, String>>>,
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

/// Pico's own context menu state (0.34.1): the ONE open
/// menu — the tab it was raised on, the target facts it was
/// built from (its picks execute against these stored facts,
/// not against whatever the page did since), its window-DIP
/// top-left as resolved by `core::layout::ctx_menu_pos`, and
/// the item rows `core::ctxmenu::menu_items` decided.
/// App-owned UI state like the panels': it lives in exactly
/// one place, closes through the one panel funnel, and any
/// panel opening supersedes it (as it supersedes them).
struct CtxMenuState {
    tab: u64,
    target: pico_core::core::ctxmenu::CtxTarget,
    x: f64,
    y: f64,
    items: Vec<pico_core::core::ctxmenu::CtxItem>,
}

/// The context menu as the chrome consumes it (0.34.1): the
/// card's window-DIP position plus the item rows — the
/// chrome draws exactly this and reports picks back by id.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CtxMenuSnap {
    x: f64,
    y: f64,
    items: Vec<CtxItemSnap>,
}

/// One context-menu row as the chrome consumes it (0.34.1):
/// the pick id (empty for a separator), the frozen label
/// and shortcut from core::ctxmenu's table, the enabled
/// gate, and the separator flag.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CtxItemSnap {
    id: String,
    label: String,
    shortcut: Option<String>,
    enabled: bool,
    sep: bool,
}

/// The context menu card's height for a stored item vec
/// (0.34.1): item rows and separator rows counted separately
/// — core::layout prices them differently.
fn ctx_menu_h_for(items: &[pico_core::core::ctxmenu::CtxItem]) -> f64 {
    let rows = items.iter().filter(|i| !i.sep).count();
    let seps = items.iter().filter(|i| i.sep).count();
    layout::ctx_menu_h(rows, seps)
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
    /// THE downloads data shape (0.34): the coordinator's
    /// snapshot, serialized with its frozen field names
    /// (`{version, tasks, active_count, failed_unseen,
    /// settings, notice, card}` — `card` added 0.35.1) —
    /// the same shape the downloads page's init/push JSON
    /// carries. Plus the panel's open state, which stays
    /// app-owned like the other panels'.
    downloads: pico_core::core::dlcoord::DlSnapshot,
    downloads_open: bool,
    /// Pico's own context menu (0.34.1), when one is open:
    /// the chrome draws the card from this and reports picks
    /// back as ctx_pick / ctx_close commands.
    context_menu: Option<CtxMenuSnap>,
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

    /// Any centered panel (Teleport's, the theme picker's, or
    /// Downloads') extends the chrome rect; they are mutually
    /// exclusive, so one bool covers all three (AUDIT V27-A2).
    fn any_panel_open(&self) -> bool {
        self.teleport_panel_open || self.palette_panel_open || self.downloads_panel_open
    }

    /// Whether the coordinator's current download notice is
    /// open (0.34): the toast's open state lives in the
    /// coordinator's snapshot now — this is its one reader
    /// for the layout/geometry paths.
    fn dl_notice_open(&self) -> bool {
        self.dl
            .snapshot()
            .notice
            .as_ref()
            .map(|n| n.open)
            .unwrap_or(false)
    }

    /// Retire the download notice WITHOUT the toast-click
    /// routing, GUARDED (0.35, WS5): the coordinator's
    /// `retire_notice_unless_failed` — panel open/close
    /// gestures (close_all_panels, the Teleport and Palette
    /// arms) are not the user looking at a failure, so an
    /// open Failed notice survives them. Genuine exits keep
    /// the coordinator's hard retire and never come through
    /// here: the notice click and the `DismissNotice`
    /// dispatch (`notice_clicked` / `Coordinator::dispatch`),
    /// the downloads panel opening (`panel_opened`), the
    /// downloads page visit (`page_visited`) and expiry
    /// (`notice_expired`).
    fn retire_notice_unless_failed(&mut self) {
        let effects = self.dl.retire_notice_unless_failed();
        self.exec_dl_effects(effects);
    }

    /// Close every chrome overlay (0.32.4): the one funnel
    /// every outside-interaction signal uses — a content
    /// webview gaining focus, the window losing focus.
    /// Mirrors each panel's own Close command semantics
    /// (Teleport's payload retires with its panel), retires
    /// the download toast, and pushes only when something
    /// actually changed, so a focus event with nothing open
    /// is a true no-op.
    fn close_all_panels(&mut self) {
        let mut changed = false;
        if self.teleport_panel_open {
            self.teleport = None;
            self.teleport_version += 1;
            self.teleport_panel_open = false;
            changed = true;
        }
        if self.palette_panel_open {
            self.palette_panel_open = false;
            changed = true;
        }
        if self.downloads_panel_open {
            self.downloads_panel_open = false;
            changed = true;
        }
        // Pico's own context menu (0.34.1) rides the same
        // funnel: a click into the page or a window focus
        // loss dismisses it with the panels.
        if self.context_menu.take().is_some() {
            changed = true;
        }
        let notice_open = self.dl_notice_open();
        if changed {
            self.layout();
        }
        self.retire_notice_unless_failed();
        if changed || notice_open {
            self.push_state(false);
        }
    }

    /// A content page's context-menu request arrived
    /// (0.34.1): the stock menu is already suppressed by the
    /// registration handler; here the event loop owns the
    /// replacement. Every open panel (and any existing menu)
    /// closes first through the one funnel — the menu
    /// supersedes them, as they supersede it. The item model,
    /// the anchor resolution and the card position are core's
    /// pure functions; this arm only supplies the live facts
    /// (the tab's history gates, the scale, the window size,
    /// the real cursor position) and then focuses the chrome,
    /// so the menu's keyboard dismissal engages
    /// deterministically (Escape is the chrome's, and a click
    /// back into the page reaches the GotFocus close funnel).
    fn on_context_menu(
        &mut self,
        tab: u64,
        target: pico_core::core::ctxmenu::CtxTarget,
        location: (i32, i32),
    ) {
        let Some(t) = self.tabs.iter().find(|t| t.id == tab) else {
            return; // the tab closed while the event was queued
        };
        let can_back = t.hist.can_go_back();
        let can_forward = t.hist.can_go_forward();
        self.close_all_panels();
        let scale = self.window.scale_factor();
        let cursor = cursor_client_dip(&self.window, scale);
        let anchor = pico_core::core::ctxmenu::resolve_anchor(
            (location.0 as f64, location.1 as f64),
            scale,
            layout::SOLID_H,
            cursor,
        );
        let items = pico_core::core::ctxmenu::menu_items(&target, can_back, can_forward);
        let menu_h = ctx_menu_h_for(&items);
        let inner = self.window.inner_size();
        let (x, y) = layout::ctx_menu_pos(
            inner.width as f64 / scale,
            inner.height as f64 / scale,
            anchor,
            menu_h,
        );
        // One log line: the kind, the item count, and the
        // page in redacted form only (redaction law) — the
        // link/source URLs never reach the log at all.
        log::info(&format!(
            "context menu: tab={tab} kind={:?} items={} page={}",
            target.kind,
            items.len(),
            redact_url(&target.page_url)
        ));
        self.context_menu = Some(CtxMenuState {
            tab,
            target,
            x,
            y,
            items,
        });
        self.layout();
        self.push_state(true);
        let _ = self.chrome.focus();
    }

    /// Execute one context-menu pick (0.34.1): the menu
    /// closes first (its state is taken, the chrome rect
    /// restored by the layout pass), then the id executes
    /// against the STORED tab and target — the facts the
    /// menu was built from. The nav verbs act on the active
    /// tab, which is the right-clicked tab by construction
    /// (only the active tab's view is visible, so only it
    /// could raise the menu); everything else acts on the
    /// stored tab explicitly, and a pick whose tab has since
    /// closed stops after the close. The pick is logged by
    /// id only — URLs never ride the log raw (redaction law).
    fn on_ctx_pick(&mut self, id: &str) {
        let Some(state) = self.context_menu.take() else {
            return;
        };
        self.layout();
        log::info(&format!("context menu: pick id={id} tab={}", state.tab));
        if self.tab_index(state.tab).is_none() {
            self.push_state(false);
            return;
        }
        // AUDIT 0.40.1: a pick may only name an item this menu
        // actually offered AND enabled. The IPC gate checks the
        // global whitelist; without this second check a pick
        // naming a whitelisted-but-unoffered (or greyed) item —
        // e.g. Paste with no clipboard text, Back with no
        // history — would still execute.
        if !state.items.iter().any(|i| i.id == id && i.enabled) {
            log::info(&format!(
                "context menu: pick id={id} not offered/enabled; ignored"
            ));
            self.push_state(false);
            return;
        }
        fn find_view(tabs: &[Tab], id: u64) -> Option<&WebView> {
            tabs.iter().find(|t| t.id == id).and_then(|t| t.view())
        }
        let stored_tab = state.tab;
        match id {
            "back" => self.go(false),
            "forward" => self.go(true),
            "reload" => self.reload_active(),
            "open_link" => {
                if let Some(u) = &state.target.link_url {
                    if is_web_url(u) {
                        let u = u.clone();
                        self.add_tab(&u, false, true);
                    }
                }
            }
            "open_image" => {
                if let Some(u) = &state.target.source_url {
                    if is_web_url(u) {
                        let u = u.clone();
                        self.add_tab(&u, false, true);
                    }
                }
            }
            "copy_link" => {
                if let Some(u) = &state.target.link_url {
                    let _ = clipboard_set_text(u);
                }
            }
            "copy_image" => {
                if let Some(u) = &state.target.source_url {
                    let _ = clipboard_set_text(u);
                }
            }
            "download_link" => {
                // The proven capture entry, verbatim: the
                // coordinator reads the tab's LIVE url (the
                // page, not the file URL) as the capture's
                // live_url, so this capture's tab fate is None
                // by construction, and its referer, cookies
                // and host-scoping come from the same triage
                // as a browser-initiated download. 0.35: the
                // capture is flagged as a context-menu
                // capture, so its chain is the file URL alone
                // (see on_download_starting) — the None fate
                // survives the chain gate for the same
                // reason: the page being read is not in it.
                if let Some(u) = state.target.link_url.clone() {
                    self.on_download_starting(state.tab, u, String::new(), true);
                }
            }
            "download_image" => {
                if let Some(u) = state.target.source_url.clone() {
                    self.on_download_starting(state.tab, u, String::new(), true);
                }
            }
            "copy" => {
                if let Some(v) = find_view(&self.tabs, stored_tab) {
                    let _ = v.evaluate_script("document.execCommand('copy')");
                }
            }
            "cut" => {
                if let Some(v) = find_view(&self.tabs, stored_tab) {
                    let _ = v.evaluate_script("document.execCommand('cut')");
                }
            }
            "select_all" => {
                if let Some(v) = find_view(&self.tabs, stored_tab) {
                    let _ = v.evaluate_script("document.execCommand('selectAll')");
                }
            }
            "paste" => {
                if let Some(text) = clipboard_get_text().filter(|s| !s.is_empty()) {
                    if let Some(v) = find_view(&self.tabs, stored_tab) {
                        // Clipboard text is page-bound input: it
                        // crosses as a JSON string literal, never
                        // interpolated raw.
                        let arg = serde_json::to_string(&text).unwrap_or_default();
                        let _ = v.evaluate_script(&format!(
                            "document.execCommand('insertText', false, {arg})"
                        ));
                    }
                }
            }
            "inspect" => {
                if let Some(v) = find_view(&self.tabs, stored_tab) {
                    v.open_devtools();
                }
            }
            _ => {
                log::info(&format!("context menu: unknown pick id ignored: {id}"));
            }
        }
        self.push_state(false);
    }

    /// The chrome rect in device pixels: the downloads popover
    /// (0.32.2) sizes its extension from the live task count —
    /// a compact dropdown needs far less than the centered
    /// panels' fixed extension — while Teleport/palette keep
    /// the fixed panel extension and the closed state is the
    /// plain tiling.
    fn chrome_phys(&self) -> layout::PhysRect {
        let size = self.window.inner_size();
        let scale = self.window.scale_factor();
        if let Some(state) = &self.context_menu {
            // Pico's own context menu (0.34.1) is the FIRST
            // claim on the chrome rect: the menu supersedes
            // every panel (opening it closed them all), and
            // its card hangs from its stored position — the
            // extension is exactly what the card's bottom
            // needs past the chrome's own height.
            let menu_h = ctx_menu_h_for(&state.items);
            layout::chrome_rect_with_extra(
                size.width,
                size.height,
                scale,
                layout::ctx_menu_extra(state.y, menu_h),
            )
        } else if self.downloads_panel_open {
            layout::chrome_rect_with_extra(size.width, size.height, scale, self.dl_popover_h())
        } else if self.any_panel_open() {
            layout::chrome_rect_with_panel(size.width, size.height, scale, self.any_panel_open())
        } else if self.dl_notice_open() {
            // The download toast (0.32.3): a pill under the
            // toolbar needs the same kind of extension as a
            // panel — without it the pill would paint below
            // the chrome HWND and be clipped away entirely.
            // Panels are checked first: their extension is
            // the larger claim and wins while one is open
            // (a notice is also never born open over a panel,
            // and opening a panel retires it).
            layout::chrome_rect_with_extra(
                size.width,
                size.height,
                scale,
                layout::downloads_toast_h(),
            )
        } else {
            layout::tile(size.width, size.height, scale).0
        }
    }

    fn chrome_rect(&self) -> Rect {
        phys_to_rect(self.chrome_phys())
    }

    /// The popover extension the CURRENT snapshot implies:
    /// the single pricing path for the downloads branch of
    /// `chrome_phys`, factored out so `exec_dl_effects` can
    /// notice the price moving while the panel is open
    /// (AUDIT 0.34.2: task/error-row changes used to leave
    /// the chrome rect at the old height indefinitely when
    /// no notice open-state flip happened in the same batch).
    fn dl_popover_h(&self) -> f64 {
        let views = self.dl.snapshot().tasks;
        let rows = views.len();
        let errs = views.iter().filter(|v| v.error.is_some()).count();
        layout::downloads_popover_h(rows, errs)
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
            let cr = self.chrome_phys();
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
                if self.context_menu.is_some() {
                    "contextmenu"
                } else if self.teleport_panel_open {
                    "teleport"
                } else if self.palette_panel_open {
                    "palette"
                } else if self.downloads_panel_open {
                    "downloads"
                } else if self.dl_notice_open() {
                    "toast"
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
        // 0.33: the same live classification for the
        // Downloads page (see `live_downloads_tabs` on App).
        let live_downloads = self.live_downloads_tabs.clone();
        // AUDIT 0.31 (A6-F2): the one-shot credentialed-nav
        // expectation, consumed by this handler (see the
        // `expected_nav` field on App).
        let expected_nav = self.expected_nav.clone();
        // AUDIT 0.32.1 (P3-1): the adoption marker this
        // handler sets when it allows a credentialed URL (see
        // the `userinfo_adopt` field on App).
        let userinfo_adopt = self.userinfo_adopt.clone();

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
                // AUDIT 0.40.1 (the A1-L5 symmetry fix): drop
                // oversize bodies HERE, like the chrome handler
                // does. Every parser on this channel caps at or
                // below MAX_NAV_BODY_BYTES (16 KB), so a larger
                // body can only be waste — pre-0.40.1 it was
                // cloned and queued before any cap saw it.
                if body.len() > pico_core::core::ipc::MAX_NAV_BODY_BYTES {
                    return;
                }
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
                // Downloads-page actions (0.33; canonical
                // body 0.34) share this channel too: disjoint
                // by `kind` ("dl"), forwarded undecided — the
                // gate is `core::dlcmd::page_gate` + the
                // frozen parser, applied centrally in
                // `on_downloads_page_body`.
                if pico_core::core::dlcmd::looks_like_dl_body(body) {
                    let _ = action_proxy.send_event(UserEvent::DownloadsAction {
                        tab: id,
                        body: body.clone(),
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
                let is_downloads = live_downloads
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
                if is_downloads {
                    // The Downloads tab may (re)load only the
                    // Downloads page itself (0.33) — the
                    // mirror of the start arm below. (Action
                    // URLs from a downloads tab already
                    // returned false above: `is_start` is
                    // false there, so they never dispatch.)
                    // The `https://pico.downloads` spellings
                    // are wry's unfolded rewrite of the custom
                    // scheme: `normalize_pico_url` does not
                    // fold the downloads host yet (flagged in
                    // the 0.33 report), so they are admitted
                    // here by name rather than falling through
                    // to the web allowlist.
                    return url == DOWNLOADS_URL
                        || url == "pico://downloads/"
                        || url == "https://pico.downloads"
                        || url == "https://pico.downloads/";
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
                    slot.remove(&id)
                };
                if !userinfo_nav_allowed(&url, expected_for_tab.as_deref()) {
                    crate::log::info(&format!(
                        "navigation denied (credentialed URL not typed by the user): tab={id} {}",
                        redact_url(&url)
                    ));
                    return false;
                }
                // Allowed. When the URL carries userinfo, this
                // exact URL is the one credentialed load this
                // tab may adopt (P3-1): record the marker the
                // page-load gate checks.
                if url_has_userinfo(&url) {
                    let mut markers = userinfo_adopt.lock().unwrap_or_else(|p| p.into_inner());
                    markers.insert(id, url.clone());
                }
                // Everywhere else: the strict scheme allowlist.
                // javascript:, data:, file:, view-source:, res:, …
                // are all refused as document navigations. Local
                // files reach a tab only as static-server http URLs.
                is_allowed_navigation_url(&url) && !url.starts_with("pico://")
            })
            .with_download_started_handler(move |uri: String, path: &mut PathBuf| -> bool {
                // 0.32: WebView2's own download never runs — the
                // attempt is cancelled here and routed to Pico's
                // download manager by the event arm, which owns
                // the URL validation, the cookie fetch and the
                // task creation. The suggested name is the path
                // argument's file name (lossy; empty when it has
                // none). Returning false ALWAYS: if the manager
                // path fails, the download is simply denied, as
                // it was through 0.31.
                let suggested = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let _ = download_proxy.send_event(UserEvent::DownloadStarting {
                    tab: id,
                    uri,
                    suggested,
                });
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
        attach_panel_focus_close(&view, id, self.proxy.clone());
        attach_downloads_shortcut(&view, self.proxy.clone());
        attach_context_menu(&view, id, self.proxy.clone());
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
        // AUDIT 0.33: `is_allowed_navigation_url` (core::addr)
        // pattern-matches the start URL specifically and does
        // not know `pico://downloads` yet — the Downloads page
        // is admitted here, in the shell, under the same
        // exact-spelling discipline. (Flagged for the core
        // allowlist to learn it; see the 0.33 report.)
        if !is_allowed_navigation_url(url) && url != DOWNLOADS_URL && url != "pico://downloads/" {
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
            is_downloads: false,
            nav_epoch: 0,
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
            adopted: false,
            pending_chain: Vec::new(),
            pending_adopted: false,
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
        // The Downloads PAGE tab becoming the active page
        // (0.34): the user has SEEN the failed tasks — the
        // coordinator clears the unseen-failed count and any
        // open notice. Idempotent: a no-change visit returns
        // no effects; the activation's own snapshot push
        // below still runs either way.
        if self.tabs.iter().any(|t| t.id == id && t.is_downloads) {
            let effects = self.dl.page_visited();
            self.exec_dl_effects(effects);
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
        // AUDIT 0.32.1 (P3-5): the media probe budget's per-tab
        // entry was the one per-tab map never forgotten here —
        // closed tab ids accumulated for the app's lifetime.
        self.probe_budget
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .forget_tab(id);
        // AUDIT 0.32.1 (P3-1/P3-3): the per-tab navigation
        // expectation and adoption marker die with the tab.
        self.expected_nav
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        self.userinfo_adopt
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        self.live_start_tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        self.live_downloads_tabs
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
        // Downloads (0.32; coordinator 0.34): the panel's open
        // state, the manager's version counter (it bumps on
        // every visible change, so the task list itself needs
        // no hashing) and the persisted settings trio. Notice
        // changes and failed-badge clears always arrive with
        // a FORCED push from the coordinator's effects, so
        // they need no hash here.
        self.downloads_panel_open.hash(&mut hasher);
        // Context menu (0.34.1): its presence, its tab and
        // its item count. (An open always arrives with a
        // forced push from its event arm; this keeps a
        // later unforced push honest about a menu that
        // closed or changed without one.)
        self.context_menu.is_some().hash(&mut hasher);
        if let Some(m) = &self.context_menu {
            m.tab.hash(&mut hasher);
            m.items.len().hash(&mut hasher);
        }
        self.dl.backend().0.version().hash(&mut hasher);
        self.persist.download_connections.hash(&mut hasher);
        self.persist.downloads_max_concurrent.hash(&mut hasher);
        self.persist.downloads_speed_limit_kbps.hash(&mut hasher);
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
        // The ONE downloads snapshot (0.34): the coordinator's
        // view of tasks, settings, the unseen-failed count
        // and the notice — the chrome's `downloads` key, and
        // (0.35.1, via its `card`) the start page's Downloads
        // card, which the shared state mirrors below for the
        // next start-page render and this push re-sends live
        // after the chrome eval.
        let dl_snapshot = self.dl.snapshot();
        let dl_card = dl_snapshot.card.clone();
        {
            let mut data = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            data.downloads_card = dl_snapshot.card.clone();
        }
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
            downloads: dl_snapshot,
            downloads_open: self.downloads_panel_open,
            context_menu: self.context_menu.as_ref().map(|m| CtxMenuSnap {
                x: m.x,
                y: m.y,
                items: m
                    .items
                    .iter()
                    .map(|i| CtxItemSnap {
                        id: i.id.to_string(),
                        label: i.label.to_string(),
                        shortcut: i.shortcut.map(|s| s.to_string()),
                        enabled: i.enabled,
                        sep: i.sep,
                    })
                    .collect(),
            }),
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
        // The start page's Downloads card (0.35.1): pushed
        // live to every start tab, gated on a hash of the
        // serialized card so an unchanged card costs one
        // hash and no eval. This sits in push_state's
        // non-gated region (past the signature early-return):
        // the gate above hashes the dl backend version, so
        // download ticks — speed changes included — pass it
        // and the card updates at the popover's cadence,
        // while a fully idle push_state never reaches here.
        // The card JSON is never logged: `newest_name` is
        // the user's own download name on his own start
        // page — page content, not log content.
        {
            use std::hash::{Hash, Hasher};
            let card_json = serde_json::to_string(&dl_card).unwrap_or_else(|_| "{}".to_string());
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            card_json.hash(&mut hasher);
            let card_sig = hasher.finish();
            if card_sig != self.last_dl_card_sig {
                self.last_dl_card_sig = card_sig;
                // Neutralize the script-breakout sequence
                // exactly as the neighbouring pushes do.
                let safe = card_json.replace("</", "<\\/");
                for t in &self.tabs {
                    if t.is_start {
                        if let Some(v) = t.view() {
                            let _ = v.evaluate_script(&format!(
                                "window.__picoSetDownloadsCard&&window.__picoSetDownloadsCard({safe})"
                            ));
                        }
                    }
                }
            }
        }
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

    /// The ONE live theme channel (0.35.1): every internal
    /// page (start + downloads) receives theme changes ONLY
    /// through this push — the versioned payload
    /// `{"v":1,"mode":"dark"|"light","palette":"<id>"}`
    /// handed to `window.__picoTheme`. (Through 0.35 the
    /// push reached start tabs only, as `__picoSetTheme`,
    /// and an open Downloads page kept the old theme until
    /// its next data push happened to carry one.) A page's
    /// only other theme input is the INIT bake at serve
    /// time (`render_start` / `render_downloads`).
    fn push_theme_to_internal_tabs(&mut self) {
        // Mode and palette both come from closed sets
        // ("light"/"dark", the palette table).
        // AUDIT 0.31 (A3-F10): that safety rests entirely on
        // upstream sanitization, so re-check the closed sets AT
        // the sink: a value outside them (a future state-load
        // regression, a hand-edited state.json) must never
        // reach the page — the push is refused whole, before
        // any eval.
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
        let payload = serde_json::json!({
            "v": 1,
            "mode": theme,
            "palette": palette,
        });
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        for t in &self.tabs {
            if t.is_start || t.is_downloads {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoTheme&&window.__picoTheme({json})"
                    ));
                }
            }
        }
    }

    /// The Downloads page's one data shape (0.34 contract):
    /// the coordinator's snapshot JSON — the frozen
    /// `DlSnapshot` fields verbatim. The same JSON is the
    /// page's stored init data and every
    /// `__picoSetDownloads` push. 0.35.1: it carries NO
    /// theme — an internal page's only theme inputs are
    /// the INIT bake at serve time (`render_downloads`
    /// merges the current theme/palette in) and the live
    /// `__picoTheme` push.
    fn downloads_snapshot_json(&self) -> String {
        let snap = self.dl.snapshot();
        serde_json::to_string(&snap).unwrap_or_else(|_| "{}".to_string())
    }

    /// Push the Downloads page's data to every live downloads
    /// tab (0.33; payload 0.34). Signature-gated on (snapshot
    /// version, settings tuple) unless forced, like the
    /// start pages' pushes. (0.35.1: theme/palette left the
    /// gate — data pushes carry no theme; theme changes
    /// ride `push_theme_to_internal_tabs`.) Whenever the
    /// JSON is rebuilt the shared init data is refreshed
    /// with it, so a fresh page load renders current data
    /// — never a stale first paint.
    fn push_downloads_page(&mut self, force: bool) {
        use std::hash::{Hash, Hasher};
        let snap = self.dl.snapshot();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        snap.version.hash(&mut hasher);
        snap.settings.default_connections.hash(&mut hasher);
        snap.settings.max_concurrent.hash(&mut hasher);
        snap.settings.speed_limit_kbps.hash(&mut hasher);
        let sig = hasher.finish();
        if !force && sig == self.last_dl_page_sig {
            return;
        }
        self.last_dl_page_sig = sig;
        let json = self.downloads_snapshot_json();
        {
            let mut data = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            data.downloads_init = json.clone();
        }
        // The JSON rides a <script>-adjacent evaluate, so the
        // script-breakout sequence is neutralized exactly as
        // the asset assembly neutralizes init data.
        let safe = json.replace("</", "<\\/");
        for t in &self.tabs {
            if t.is_downloads {
                if let Some(v) = t.view() {
                    let _ = v.evaluate_script(&format!(
                        "window.__picoSetDownloads&&window.__picoSetDownloads({safe})"
                    ));
                }
            }
        }
    }

    /// Open the Downloads page (0.33): focus its tab when one
    /// exists, otherwise create it. Opening the page marks
    /// every failed task as seen and retires an open notice
    /// (0.34: the coordinator's `page_visited` owns that
    /// state now — the page IS the failure list).
    fn open_downloads_page(&mut self) {
        let effects = self.dl.page_visited();
        self.exec_dl_effects(effects);
        let existing = self.tabs.iter().find(|t| t.is_downloads).map(|t| t.id);
        match existing {
            Some(id) => {
                self.activate(id, true);
                self.push_state(true);
            }
            None => {
                let id = self.add_tab(DOWNLOADS_URL, false, true);
                if id > 0 {
                    if let Some(t) = self.tab_mut(id) {
                        t.is_downloads = true;
                        t.title = "Downloads".to_string();
                    }
                    self.live_downloads_tabs
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(id);
                }
            }
        }
        self.push_downloads_page(true);
    }

    /// The coordinator's landing facts for one tab RIGHT
    /// NOW (0.34): whether the capture's tab still exists,
    /// its current navigation epoch (0 when the tab is
    /// gone — the coordinator's `tab_alive: false`
    /// dominates), and whether it is the only tab. These
    /// are the app's live answers to the questions the
    /// coordinator's deferred fate decision asks.
    fn landing_facts_now(&self, tab_id: u64) -> pico_core::core::dlcoord::LandingFacts {
        match self.tabs.iter().find(|t| t.id == tab_id) {
            Some(t) => pico_core::core::dlcoord::LandingFacts {
                tab_alive: true,
                nav_epoch: t.nav_epoch,
                only_tab: self.tabs.len() <= 1,
            },
            None => pico_core::core::dlcoord::LandingFacts {
                tab_alive: false,
                nav_epoch: 0,
                only_tab: self.tabs.len() <= 1,
            },
        }
    }

    /// Execute the coordinator's effects (0.34): the ONE
    /// funnel every coordinator call's output runs through.
    /// Effects are executed in order; an effect that itself
    /// produces effects (opening the panel reports back
    /// into the coordinator) queues behind the rest. The
    /// notice timer is armed at the end, from the resulting
    /// snapshot.
    fn exec_dl_effects(&mut self, effects: Vec<pico_core::core::dlcoord::DlEffect>) {
        use pico_core::core::dlcoord::DlEffect;
        let notice_was_open = self.dl_notice_open();
        let popover_h_before = self.dl_popover_h();
        let mut queue: std::collections::VecDeque<DlEffect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            match effect {
                DlEffect::Log(msg) => log::info(&msg),
                DlEffect::LogError(msg) => log::error(&msg),
                DlEffect::ExecuteFate { tab, fate } => {
                    self.exec_dl_fate(tab, fate);
                }
                DlEffect::PushChrome => self.push_state(true),
                DlEffect::PushPage => self.push_downloads_page(false),
                DlEffect::OpenDownloadsPage => self.open_downloads_page(),
                DlEffect::OpenPanel => {
                    // Mirror the old chrome Downloads arm:
                    // the panels are mutually exclusive.
                    self.teleport = None;
                    self.teleport_version += 1;
                    self.teleport_panel_open = false;
                    self.palette_panel_open = false;
                    // A panel opening supersedes the context
                    // menu (0.34.1), as the menu supersedes it.
                    self.context_menu = None;
                    self.downloads_panel_open = true;
                    self.layout();
                    self.push_state(true);
                    // The funnel law: opening the panel
                    // (any route) reports back.
                    queue.extend(self.dl.panel_opened());
                }
                DlEffect::ClosePanel => {
                    self.downloads_panel_open = false;
                    self.layout();
                    self.push_state(true);
                }
                DlEffect::ShellOpenFolder(path) => {
                    open_folder_in_explorer(&path);
                }
                DlEffect::ShellOpenFile(path) => {
                    open_file_in_explorer(&path);
                }
                DlEffect::PersistDownloadsSettings(settings) => {
                    self.persist.download_connections = settings.default_connections;
                    self.persist.downloads_max_concurrent = settings.max_concurrent;
                    self.persist.downloads_speed_limit_kbps =
                        settings.speed_limit_kbps.unwrap_or(0);
                    self.save_state();
                }
            }
        }
        // The toast changes the chrome's height claim: if
        // its open state flipped during this batch, relayout
        // once (the per-effect paths already relayout for
        // panel changes; the notice had no such hook in
        // 0.33's field writes either — its show/retire
        // helpers called layout() directly).
        // AUDIT 0.34.2: the popover's height claim moves with
        // the task list too — a task added/removed or an
        // error row appearing while the panel is open changes
        // the implied extension without any notice flip, and
        // through 0.34.1 nothing relayouted, so the rect sat
        // at the stale height (indefinitely under a Failed
        // notice, which has no expiry timer). Relayout when
        // the price moved during this batch.
        if self.dl_notice_open() != notice_was_open
            || (self.downloads_panel_open && self.dl_popover_h() != popover_h_before)
        {
            self.layout();
        }
        self.arm_dl_notice_timer();
    }

    /// Execute ONE tab fate (0.34): the tab-side action a
    /// landed capture decided. The restore/go-back bodies
    /// are 0.33's capture-unwind EXECUTION code, reused
    /// verbatim minus the decision (the coordinator owns
    /// the decision now; `capture_started` already applied
    /// the fate's state half where the capture is
    /// concerned — these run the view/history half).
    ///
    /// AUDIT 0.34.1 (field addition): each executed fate
    /// logs exactly one `download tab fate: …` line, at
    /// the point of commitment inside its arm (see the
    /// RestoreTo / GoBack notes) — through 0.34 a fate
    /// executed with no log line at all, so a field log
    /// could not say what happened to the capturing tab.
    fn exec_dl_fate(&mut self, tab_id: u64, fate: pico_core::core::dlcoord::TabFate) {
        use pico_core::core::dlcoord::TabFate;
        match fate {
            TabFate::None => {}
            TabFate::RestoreTo(url) => {
                // AUDIT 0.34.1 (field addition): the fate
                // log line sits AFTER this early return, at
                // the point of commitment below — a missing
                // tab restores nothing, so it must not claim
                // a restore. (fate_step already revalidated
                // the tab alive, so this path is defensive
                // only.)
                let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) else {
                    return;
                };
                // The restore target is host data; if it
                // carries userinfo, register the navigation
                // expectation first (AUDIT 0.32.1 P2-3: an
                // unmarked expectation-free load of a
                // credentialed URL is denied by the
                // navigation handler).
                if pico_core::core::ipc::url_has_userinfo(&url) {
                    self.expect_nav(tab_id, &url);
                }
                let tab = &mut self.tabs[idx];
                // The finish of the file navigation is
                // swallowed by the coordinator
                // (`navigation_finished`) — no marker is
                // needed here.
                tab.urlstate = UrlState::with_url(&url);
                tab.url = url.clone();
                if let Some(view) = tab.view() {
                    let _ = view.load_url(&url);
                }
                // 0.34.1: exactly one fate line per
                // execution, after the point of commitment.
                let redacted = redact_url(&url);
                log::info(&format!(
                    "download tab fate: restored tab {tab_id} to {redacted}"
                ));
            }
            TabFate::GoBack => {
                // `App::go`'s back half, verbatim in shape:
                // record the traversal intent in the history
                // machine (it returns the target), let the
                // webview perform the move, and re-arm the
                // one-shot expectation if the target is a
                // credentialed entry (AUDIT 0.32.1 P3-2).
                // The landing reconciles through the reducer
                // like any traversal.
                let now_ms = stats_now_ms();
                let mut target_url: Option<String> = None;
                if let Some(tab) = self.tab_mut(tab_id) {
                    if let Some(target) = tab.hist.go_back(now_ms) {
                        target_url = Some(target.to_string());
                        if let Some(v) = tab.view() {
                            let _ = v.evaluate_script("history.back()");
                        }
                        // 0.34.1: the fate line fires only
                        // here — go_back returning a target
                        // is the point of commitment; with
                        // no target (or no tab) no traversal
                        // happened and no line is claimed.
                        log::info(&format!("download tab fate: tab {tab_id} back"));
                    }
                }
                if let Some(target) = &target_url {
                    if pico_core::core::ipc::url_has_userinfo(target) {
                        self.expect_nav(tab_id, target);
                    }
                }
            }
            TabFate::ConvertToStart => {
                self.convert_tab_to_start(tab_id);
                // 0.34.1: exactly one fate line per
                // execution (fate_step revalidated the tab
                // alive, so the conversion has a tab).
                log::info(&format!(
                    "download tab fate: converted tab {tab_id} to start"
                ));
            }
            TabFate::CloseTab => {
                self.close_tab(tab_id);
                log::info(&format!("download tab fate: closed tab {tab_id}"));
            }
        }
        if self.tabs.iter().any(|t| t.id == tab_id) {
            self.push_state(true);
        }
    }

    /// Arm the notice expiry timer (0.34): after effects
    /// execute, if the coordinator's notice is open and no
    /// timer was armed for its sequence yet, spawn the
    /// one-shot sleep thread (the cookie timeout's pattern)
    /// that posts `DownloadNoticeExpired`. Durations are
    /// 0.33's, preserved exactly: started 4 s, blocked 4 s,
    /// finished 5 s, failed NEVER (a failed notice retires
    /// only by click or by opening the downloads page).
    fn arm_dl_notice_timer(&mut self) {
        use pico_core::core::dlcoord::NoticeKind;
        let snap = self.dl.snapshot();
        let Some(notice) = snap.notice else {
            return;
        };
        if !notice.open || notice.seq == self.dl_notice_armed_seq {
            return;
        }
        let ms = match notice.kind {
            NoticeKind::Started => 4000,
            NoticeKind::Blocked => 4000,
            NoticeKind::Finished => 5000,
            NoticeKind::Failed => return,
        };
        self.dl_notice_armed_seq = notice.seq;
        let seq = notice.seq;
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let _ = proxy.send_event(UserEvent::DownloadNoticeExpired { seq });
        });
    }

    /// One downloads body from the CHROME (0.34): the dl
    /// bodies ride the chrome transport, so the (boot, req)
    /// dedupe guards them exactly like `Command` envelopes
    /// (a transport double-fire must not double-apply a
    /// downloads verb). Parse via the frozen dlcmd parser
    /// over the normalized body; a denial writes exactly
    /// one log line — no silent drop.
    fn on_chrome_dl_body(&mut self, raw: &str) {
        // AUDIT 0.40.1: the envelope is mandatory on this
        // transport. Through 0.35.1 a body whose boot/req were
        // absent or mistyped skipped the dedupe — and with it
        // the stale/retired-boot rejection — and dispatched
        // anyway. Pico's own chrome JS always sends a valid
        // envelope (`send()` sets v/boot/req after the payload
        // merge), so an envelope-less body is dropped like a
        // parse failure.
        let Some((boot, req)) = dl_envelope_id(raw) else {
            log::info("downloads command dropped (missing chrome envelope)");
            return;
        };
        {
            // The dedupe consults the command only for its
            // Ready-ness (boot adoption); a non-Ready
            // placeholder preserves the strict rule that a
            // fresh boot's count starts at its first Ready.
            // `check` returns true for a FIRST delivery (the
            // pair was unseen and is now recorded) — the same
            // polarity as the command path in `on_chrome_cmd`.
            // AUDIT 0.34.2: through 0.34.1 this guard was
            // inverted (it returned early on a variable
            // holding the check's "accepted"), which dropped every
            // chrome-surface downloads verb on its normal
            // single delivery — the toolbar button, the
            // popover's rows, Add URL, the toast's dismiss —
            // while re-deliveries and stale-boot bodies (for
            // which `check` returns false) were dispatched,
            // repeatedly. The downloads PAGE surface has no
            // dedupe and kept working, which is why the dead
            // chrome surface shipped unnoticed.
            if !self.dedupe.check(&Envelope {
                boot,
                req,
                cmd: Command::WinMin,
            }) {
                log::info(&format!(
                    "downloads command dropped (duplicate or stale boot, req {req})"
                ));
                return;
            }
        }
        let body = normalize_dl_body(raw);
        match pico_core::core::dlcmd::parse_dl_body(&body) {
            Ok(cmd) => {
                let name = cmd.name();
                // AUDIT 0.40.1: the per-verb surface table is a
                // gate, not documentation — a verb the chrome
                // surface never emits is denied here, so a
                // compromised chrome context cannot reach
                // page-only verbs (delete-file, refresh, …).
                if !pico_core::core::dlcmd::spec_for(name)
                    .is_some_and(|s| s.allows(pico_core::core::dlcmd::DlSurface::Chrome))
                {
                    log::error(&format!(
                        "downloads command: surface=chrome action={name} decision=deny reason=surface"
                    ));
                    return;
                }
                let effects = if cmd == pico_core::core::dlcmd::DlCommand::DismissNotice {
                    self.dl.notice_clicked()
                } else {
                    self.dl
                        .dispatch(cmd, pico_core::core::dlcoord::DlSource::Chrome)
                };
                log::info(&format!(
                    "downloads command: surface=chrome action={name} decision=ok"
                ));
                self.exec_dl_effects(effects);
            }
            Err(deny) => {
                log::error(&format!(
                    "downloads command: surface=chrome decision=deny reason={}",
                    deny.label()
                ));
            }
        }
    }

    /// One downloads body from the DOWNLOADS PAGE (0.34):
    /// the page's action bridge, gated by the frozen
    /// `core::dlcmd::page_gate` over the app's own tab
    /// state, parsed by the same frozen parser, dispatched
    /// as `DlSource::Page`. EVERY body — allowed or denied
    /// — writes exactly one pico.log line naming the action
    /// and the decision, mirroring `on_start_action`.
    fn on_downloads_page_body(&mut self, tab_id: u64, raw: &str) {
        let label = pico_core::core::dlcmd::action_label(raw);
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            log::error(&format!(
                "downloads action: kind={label} tab={tab_id} decision=deny reason=tab-gone"
            ));
            return;
        };
        let gate = pico_core::core::dlcmd::page_gate(
            tab.is_downloads,
            pico_core::core::dlcmd::is_downloads_url(&tab.url),
        );
        if !gate {
            log::error(&format!(
                "downloads action: kind={label} tab={tab_id} decision=deny reason=not-on-downloads-page"
            ));
            return;
        }
        let body = normalize_dl_body(raw);
        match pico_core::core::dlcmd::parse_dl_body(&body) {
            Ok(cmd) => {
                // AUDIT 0.40.1: same surface gate as the chrome
                // transport — the page may only send page verbs,
                // never chrome-only ones (set-connections,
                // set-default-connections, panel verbs, …).
                if !pico_core::core::dlcmd::spec_for(cmd.name())
                    .is_some_and(|s| s.allows(pico_core::core::dlcmd::DlSurface::Page))
                {
                    log::error(&format!(
                        "downloads action: kind={label} tab={tab_id} decision=deny reason=surface"
                    ));
                    return;
                }
                log::info(&format!(
                    "downloads action: kind={label} tab={tab_id} decision=ok cmd={}",
                    cmd.name()
                ));
                let effects = if cmd == pico_core::core::dlcmd::DlCommand::DismissNotice {
                    self.dl.notice_clicked()
                } else {
                    self.dl
                        .dispatch(cmd, pico_core::core::dlcoord::DlSource::Page)
                };
                self.exec_dl_effects(effects);
            }
            Err(deny) => {
                log::error(&format!(
                    "downloads action: kind={label} tab={tab_id} decision=deny reason={}",
                    deny.label()
                ));
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

    /// A content tab's download attempt (0.34): the thin
    /// app side of a capture. The facts come from the tab
    /// as it stands at capture time; the coordinator
    /// triages, decides the tab's fate and (on acceptance)
    /// holds the capture open under a token until the
    /// cookie leg lands it. The app's remaining jobs: run
    /// the returned effects, then run the cookie leg for an
    /// ACCEPTED capture (acceptance is pre-triaged here
    /// with the core's pure `triage_download_uri` — the
    /// coordinator re-triages internally for its own
    /// decision; the double triage is pure and cheap).
    /// Only the host is ever logged — never the URL, never
    /// a cookie.
    ///
    /// 0.35, WS2: the facts carry the captured navigation's
    /// URL CHAIN (see the assembly in the body), and
    /// `from_context_menu` marks the context menu's
    /// Download Link / Download Image captures, which are
    /// NOT navigations: their chain is the file URL alone,
    /// so the page being read (their live URL) is outside
    /// it and their fate stays None, exactly as through
    /// 0.34.
    fn on_download_starting(
        &mut self,
        tab: u64,
        uri: String,
        suggested: String,
        from_context_menu: bool,
    ) {
        use pico_core::core::dlcoord::CaptureFacts;
        // App-side accept pre-triage, for the cookie leg
        // only (see the doc above).
        let accepted = matches!(
            pico_core::core::download::triage_download_uri(&uri),
            pico_core::core::download::DownloadTriage::Accept
        );
        // Capture facts from the tab's CURRENT state. A
        // gone tab still gets a capture call (its facts are
        // the gone-tab shape); the landing revalidation
        // dominates the outcome there.
        let (facts, referer) = match self.tabs.iter().find(|t| t.id == tab) {
            Some(t) => {
                let committed = t.hist.current().map(|s| s.to_string());
                let prev = committed.as_ref().and_then(|_| {
                    let entries = t.hist.entries();
                    let idx = t.hist.index()?;
                    if idx > 0 {
                        entries.get(idx - 1).cloned()
                    } else {
                        None
                    }
                });
                // The capture's URL chain (0.35, WS2),
                // deduped, order preserved. A navigation
                // capture assembles it from the tab's
                // pending-navigation record (initial target
                // + redirect hops) plus the live URL plus
                // the captured URI — the live URL is a
                // pre-redirect hop the record may not have
                // seen (e.g. no Source event fired for it),
                // and the captured URI is the final hop the
                // record cannot have seen (the handler hands
                // it over post-redirect). A context-menu
                // capture is not a navigation: its chain is
                // the file URL alone, so the page being read
                // (its live URL) stays outside the chain and
                // the fate gate refuses, as through 0.34.
                let mut chain: Vec<String> = Vec::new();
                let push_unique = |chain: &mut Vec<String>, u: &str| {
                    if !chain.iter().any(|c| c == u) {
                        chain.push(u.to_string());
                    }
                };
                if from_context_menu {
                    push_unique(&mut chain, &uri);
                } else {
                    for hop in &t.pending_chain {
                        let hop = hop.clone();
                        push_unique(&mut chain, &hop);
                    }
                    push_unique(&mut chain, &t.url.clone());
                    push_unique(&mut chain, &uri);
                }
                let in_chain = |u: &str| chain.iter().any(|c| c == u);
                let committed_in_chain = committed.as_ref().is_some_and(|c| in_chain(c));
                let prev_is_real = prev.as_ref().is_some_and(|p| !in_chain(p));
                // Pre-capture-commit defense (0.35, WS2,
                // bounded): a committed entry that is IN
                // the chain is the captured navigation's own
                // artifact — a download renders no page, so
                // that "commit" is the WebView2Feedback-#5172
                // shape, a Finished for the file that landed
                // before this capture and polluted the
                // history (and, being a Commit, cleared the
                // tab's live `adopted`). When NO real page
                // sits behind it (the previous entry is
                // absent or itself a chain hop), the artifact
                // is presented to the matrix as NO committed
                // page, and the origin is recovered from the
                // pending record's adopted-at-start snapshot
                // — the download-only tab then closes as it
                // should instead of converting. When a real
                // previous page DOES sit behind it, the
                // committed entry is left in place on
                // purpose: the chain matrix judges a
                // chain-committed entry with a real previous
                // entry as GoBack to that page, origin
                // regardless, and presenting None here would
                // destroy exactly that row.
                let (committed_url, origin) =
                    if !from_context_menu && committed_in_chain && !prev_is_real {
                        (
                            None,
                            if t.pending_adopted {
                                pico_core::core::dlcoord::CaptureOrigin::Adopted
                            } else {
                                pico_core::core::dlcoord::CaptureOrigin::User
                            },
                        )
                    } else {
                        (
                            committed.clone(),
                            if t.adopted {
                                pico_core::core::dlcoord::CaptureOrigin::Adopted
                            } else {
                                pico_core::core::dlcoord::CaptureOrigin::User
                            },
                        )
                    };
                // The 0.33 referer rule: the page the
                // download was captured from (the committed
                // page when it differs from the download
                // URL); the manager/coordinator sanitizes
                // (userinfo-stripped http/https) downstream.
                // 0.35: an in-chain committed entry is the
                // artifact above, not a referring page — the
                // real page behind it (when there is one)
                // refers instead.
                let referer = if !from_context_menu && committed_in_chain {
                    if prev_is_real {
                        prev.clone().filter(|c| c != &uri)
                    } else {
                        None
                    }
                } else {
                    committed.clone().filter(|c| c != &uri)
                };
                (
                    CaptureFacts {
                        live_url: t.url.clone(),
                        captured_url: uri.clone(),
                        chain,
                        committed_url,
                        prev_url: prev,
                        origin,
                        only_tab: self.tabs.len() <= 1,
                        traversal_pending: t.hist.has_pending(),
                        nav_epoch: t.nav_epoch,
                    },
                    referer,
                )
            }
            None => (
                CaptureFacts {
                    live_url: uri.clone(),
                    captured_url: uri.clone(),
                    chain: vec![uri.clone()],
                    committed_url: None,
                    prev_url: None,
                    origin: pico_core::core::dlcoord::CaptureOrigin::User,
                    only_tab: self.tabs.len() <= 1,
                    traversal_pending: false,
                    nav_epoch: 0,
                },
                None,
            ),
        };
        let (token, effects) =
            self.dl
                .capture_started(tab, uri.clone(), suggested, referer.clone(), facts);
        self.exec_dl_effects(effects);
        if !accepted {
            // A rejected capture is complete: the effects
            // carried the fate and the blocked notice. No
            // cookie leg, no token to land.
            return;
        }
        // The cookie leg (0.32's plumbing, token-shaped):
        // the page's session cookies ride the task when the
        // capture crosses within the page's own site. Any
        // shortcut — no tab, no view, cross-site, COM error
        // — lands the capture immediately with no cookies:
        // a download must never hang on its session fetch.
        let page_url = {
            let t = self.tabs.iter().find(|t| t.id == tab);
            match (t, &referer) {
                (Some(_), Some(r)) => Some(r.clone()),
                (Some(t), None) => Some(t.url.clone()),
                _ => None,
            }
        };
        let has_view = self
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.view())
            .is_some();
        let (true, Some(page_url)) = (has_view, page_url) else {
            let effects = self
                .dl
                .cookies_arrived(token, Vec::new(), self.landing_facts_now(tab));
            self.exec_dl_effects(effects);
            return;
        };
        if !pico_core::core::download::cookie_capture_allowed(&page_url, &uri) {
            // P2-1 (AUDIT 0.32.1), preserved: the page's
            // cookies are withheld when the download host is
            // not host-related to the page's host (neither the
            // same host nor a subdomain of it — the predicate
            // is host-relatedness only; AUDIT 0.40.1 corrected
            // this comment, which used to claim scheme/port
            // checks the predicate never made) — a hostile page
            // must not pass its session to a third-party host by
            // pointing the browser at it, and the third party
            // must not receive the page's cookies either. The
            // jar slice itself is WebView2's, taken for the
            // download URI, and the send predicate re-checks
            // host-only/secure/path/expiry per request.
            log::info(&format!(
                "download cookies skipped (cross-site): page host {} download host {}",
                pico_core::core::download::url_host(&page_url)
                    .unwrap_or_else(|| "unknown host".to_string()),
                pico_core::core::download::url_host(&uri)
                    .unwrap_or_else(|| "unknown host".to_string())
            ));
            let effects = self
                .dl
                .cookies_arrived(token, Vec::new(), self.landing_facts_now(tab));
            self.exec_dl_effects(effects);
            return;
        }
        // The 1500 ms cookie timeout (0.32): whichever of
        // the answer and the timeout arrives first lands
        // the capture; the loser is ignored by the
        // coordinator (unknown token).
        let timeout_proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let _ = timeout_proxy.send_event(UserEvent::DownloadCookiesTimeout { tab, token });
        });
        let fetch_result = {
            // AUDIT 0.40.1: was an `expect` on an invariant the
            // guard above establishes — under panic="abort" a
            // broken invariant would kill the browser, so this
            // now takes the same graceful path as a failed
            // fetch instead of panicking.
            let Some(view) = self
                .tabs
                .iter()
                .find(|t| t.id == tab)
                .and_then(|t| t.view())
            else {
                log::error(&format!(
                    "download cookie fetch skipped for tab {tab}: view gone (continuing without cookies)"
                ));
                let effects =
                    self.dl
                        .cookies_arrived(token, Vec::new(), self.landing_facts_now(tab));
                self.exec_dl_effects(effects);
                return;
            };
            fetch_download_cookies(view, &uri, tab, token, self.proxy.clone())
        };
        if let Err(why) = fetch_result {
            log::error(&format!(
                "download cookie fetch failed for tab {tab}: {why} (continuing without cookies)"
            ));
            let effects = self
                .dl
                .cookies_arrived(token, Vec::new(), self.landing_facts_now(tab));
            self.exec_dl_effects(effects);
        }
    }

    /// A capture's cookies arrived (0.34): land it in the
    /// coordinator with the tab's CURRENT landing facts —
    /// the coordinator revalidates the deferred fate
    /// against them (a tab that committed a real page
    /// since capture keeps it).
    fn on_download_cookies(
        &mut self,
        tab: u64,
        token: u64,
        cookies: Vec<pico_core::core::download::CookiePair>,
    ) {
        let effects = self
            .dl
            .cookies_arrived(token, cookies, self.landing_facts_now(tab));
        self.exec_dl_effects(effects);
    }

    /// A capture's cookie fetch timed out (0.34): land it
    /// with no cookies rather than not at all.
    fn on_download_cookies_timeout(&mut self, tab: u64, token: u64) {
        let effects = self.dl.cookies_timeout(token, self.landing_facts_now(tab));
        self.exec_dl_effects(effects);
    }

    /// Convert a content tab into the start tab in place
    /// (0.32.3): the mirror image of `convert_start_tab`.
    /// Used by the download unwind when the tab has no
    /// committed page to return to — a fresh tab or a popup
    /// adoption whose first navigation was the captured
    /// file. The start classification is LIVE (the shared
    /// set the navigation handler reads), so the existing
    /// view follows the flip immediately; the record reset
    /// mirrors the start half of `add_tab`'s seeding.
    fn convert_tab_to_start(&mut self, tab_id: u64) {
        let Some(idx) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[idx].is_start {
            return;
        }
        // The tab may hold or borrow a static-server root
        // (it was showing a local page): release through
        // the one path that owns root bookkeeping.
        self.set_tab_root(idx, None);
        // 0.33 hygiene: a downloads tab converting to start
        // leaves the downloads classification too.
        self.live_downloads_tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&tab_id);
        self.live_start_tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(tab_id);
        let was_active = self.active == tab_id;
        if was_active {
            // The page the panels name is gone: retire them
            // exactly as a start conversion does.
            self.teleport = None;
            self.teleport_version += 1;
            self.teleport_panel_open = false;
            self.palette_panel_open = false;
            self.downloads_panel_open = false;
        }
        {
            let tab = &mut self.tabs[idx];
            tab.is_start = true;
            tab.is_downloads = false;
            tab.hist = TabHistory::new();
            tab.urlstate = UrlState::with_url(START_URL);
            tab.url = START_URL.to_string();
            tab.title = "Start".to_string();
            tab.failed = None;
            tab.commit_started = false;
            tab.pending_chain.clear();
            tab.pending_adopted = false;
            tab.loading = tab.view.is_some();
            tab.last_load_start = if tab.view.is_some() {
                Some(Instant::now())
            } else {
                None
            };
        }
        self.shield.set_page_url(tab_id, START_URL);
        self.shield.set_pending_main(tab_id, START_URL);
        self.lite.clear_chosen(tab_id);
        if let Some(v) = self.tabs[idx].view() {
            let _ = v.load_url(START_URL);
            if was_active {
                let _ = v.focus();
            }
        } else {
            // A failed view: the record is already retargeted;
            // rebuild it for the start page (the A6-3 retry).
            self.retry_tab_view(tab_id);
        }
        if was_active {
            self.layout();
        }
        self.push_state(true);
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
        // The centered panels are mutually exclusive (v27,
        // Downloads added 0.32): asking for Teleport retires
        // the picker and the Downloads panel.
        self.palette_panel_open = false;
        self.downloads_panel_open = false;
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
            // A panel opening supersedes the context menu
            // (0.34.1), as the menu supersedes it.
            self.context_menu = None;
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
            tab.pending_chain.clear();
            tab.pending_adopted = tab.adopted;
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
        slot.insert(tab, url.to_string());
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
        let mut target_url: Option<String> = None;
        if let Some(tab) = self.tab_mut(id) {
            let target = if forward {
                tab.hist.go_forward(now_ms)
            } else {
                tab.hist.go_back(now_ms)
            };
            if target.is_some() {
                target_url = target.map(str::to_string);
                if let Some(v) = tab.view() {
                    let _ = v.evaluate_script(if forward {
                        "history.forward()"
                    } else {
                        "history.back()"
                    });
                }
            }
        }
        // AUDIT 0.32.1 (P3-2): a traversal landing on a
        // credentialed history entry must re-arm the one-shot
        // expectation, exactly as the original typed
        // submission did — through 0.32 reload/traversal never
        // armed it, so a legitimately typed credentialed page
        // could never be reloaded or gone back to (the
        // navigation handler denied its own user's page).
        if let Some(target) = &target_url {
            if url_has_userinfo(target) {
                self.expect_nav(id, target);
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
        // AUDIT 0.32.1 (P3-2): reloading a credentialed page
        // re-arms the one-shot expectation for its exact URL
        // (see go()) — the navigation the reload triggers is
        // the user's own act on their own typed URL.
        let current = tab.url.clone();
        if url_has_userinfo(&current) {
            self.expect_nav(id, &current);
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
    /// theme to the internal tabs, and let the snapshot
    /// re-token the chrome. The theme law (0.35.1): internal
    /// pages (start + downloads) are themed live through the
    /// one `__picoTheme` channel; regular web pages are
    /// never touched.
    fn set_palette(&mut self, id: &str) {
        if !pico_core::core::themes::is_known_palette(id) {
            log::error(&format!("theme: unknown palette id refused: {id}"));
            return;
        }
        if self.persist.theme_id == id {
            // AUDIT V28 A3-LOW-2: a same-id re-pick still re-pushes
            // the theme to internal tabs — an internal page that
            // missed the first push (recreated, not yet ready)
            // heals here instead of staying on the old palette
            // forever.
            self.push_theme_to_internal_tabs();
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
        self.push_theme_to_internal_tabs();
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
        self.push_theme_to_internal_tabs();
        self.push_state(true);
    }

    // -- command processing ------------------------------------------------

    fn on_chrome_cmd(&mut self, raw: &str, control_flow: &mut ControlFlow) {
        // Downloads bodies (0.34) ride the chrome transport
        // as the canonical `{"kind":"dl", ...}` shape — they
        // are NOT chrome `Command`s (the downloads family
        // left `core::ipc` in 0.34). Route them to the
        // coordinator before the command parser sees the
        // body; the dl handler applies the envelope dedupe
        // itself.
        if pico_core::core::dlcmd::looks_like_dl_body(raw) {
            self.on_chrome_dl_body(raw);
            self.push_state(false);
            return;
        }
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
            Command::Teleport => {
                // The download toast retires when any panel
                // opens (0.32.3), Teleport included — guarded
                // since 0.35 (WS5): an open Failed notice
                // survives panel gestures.
                self.retire_notice_unless_failed();
                self.request_teleport();
            }
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
                // Mutually exclusive with Teleport (v27) and
                // Downloads (0.32): the picker replaces them,
                // payload and all. The download toast retires
                // with the other panels' opening (0.32.3) —
                // guarded since 0.35 (WS5), like Teleport's.
                self.retire_notice_unless_failed();
                self.teleport = None;
                self.teleport_version += 1;
                self.teleport_panel_open = false;
                self.downloads_panel_open = false;
                // A panel opening supersedes the context menu
                // (0.34.1), as the menu supersedes it.
                self.context_menu = None;
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
            Command::CtxClose => {
                // Context menu dismissed without a pick
                // (0.34.1): drop the stored state and restore
                // the plain chrome bounds; a close with no
                // menu open is a no-op.
                if self.context_menu.take().is_some() {
                    self.layout();
                    self.push_state(false);
                }
            }
            Command::CtxPick { id } => self.on_ctx_pick(&id),
            Command::ThemeSet { id } => self.set_palette(&id),
            Command::OpenFile => self.open_file_dialog(),
            Command::Social { site } => self.open_social(site),
            Command::WinMin => self.window.set_minimized(true),
            Command::WinMax => self.window.set_maximized(!self.window.is_maximized()),
            Command::WinDrag => {
                let _ = self.window.drag_window();
            }
            Command::WinClose => {
                // Downloads (0.32): pause every running task and
                // persist its live progress BEFORE teardown, so
                // a close never loses downloaded bytes' record.
                self.dl.backend().0.pause_all_and_persist();
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
        // The pending-navigation chain (0.35, WS2), maintained
        // event by event BEFORE the reducer consumes the event
        // (the chain describes the navigation the event belongs
        // to, judged from the state that navigation started
        // in):
        //
        // - CommitStart(url): a full navigation is starting.
        //   Every CommitStart begins a NEW navigation (the
        //   previous one either settled at its Finish or is
        //   being pre-empted), so the chain RESETS to
        //   `[url]` and the record snapshots the tab's
        //   CURRENT `adopted` — the origin this navigation
        //   starts with, before any commit of its own can
        //   clear the live flag. (The brief's "a Start
        //   arriving while the tab is not loading resets"
        //   is this same moment seen from the load flags:
        //   `on_page_load` has already set `loading` for a
        //   Start by the time this runs, so the reset is
        //   unconditional here rather than flag-gated.)
        // - Source(url): while the tab is loading (a load
        //   in flight: `loading` or `commit_started`), the
        //   URL is a hop of the in-flight navigation — a
        //   redirect target, or the final URL the Source
        //   settled on — and APPENDS, consecutive
        //   duplicates deduped. Recording the FIRST hop of
        //   an as-yet empty chain snapshots `adopted` the
        //   same way a Start does (a Source can precede its
        //   document's CommitStart, AUDIT V28 A6-9; the
        //   Start that follows resets to the same URL and
        //   re-snapshots the same flag). A Source observed
        //   while NOT loading is a same-document change of
        //   an already settled page — not a navigation hop
        //   — and never touches the chain.
        // - CommitFinish: the navigation settled into a
        //   page, so the chain CLEARS — it must hold
        //   nothing from a settled navigation. The
        //   `pending_adopted` snapshot deliberately
        //   survives (see the field): a download capture
        //   that arrives just after this Finish can still
        //   recover the settled navigation's origin.
        // - Note: an observer hint about the CURRENT
        //   document, never a navigation hop — ignored.
        //
        // Unusable URLs (empty / about:blank / non-web, the
        // reducer's `usable` gate) never enter the chain:
        // the reducer refuses them, so the record must not
        // carry a URL no event could have adopted.
        match &ev {
            UrlEvent::CommitStart(u)
                if !u.is_empty()
                    && u != "about:blank"
                    && pico_core::core::addr::web_origin(u).is_some() =>
            {
                let tab = &mut self.tabs[idx];
                tab.pending_adopted = tab.adopted;
                tab.pending_chain.clear();
                tab.pending_chain.push(u.clone());
            }
            UrlEvent::Source(u)
                if !u.is_empty()
                    && u != "about:blank"
                    && pico_core::core::addr::web_origin(u).is_some() =>
            {
                let tab = &mut self.tabs[idx];
                if tab.loading || tab.commit_started {
                    if tab.pending_chain.is_empty() {
                        tab.pending_adopted = tab.adopted;
                    }
                    if tab.pending_chain.last().map(|s| s.as_str()) != Some(u.as_str()) {
                        tab.pending_chain.push(u.clone());
                    }
                }
            }
            UrlEvent::CommitFinish(_) => {
                self.tabs[idx].pending_chain.clear();
            }
            _ => {}
        }
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
        if matches!(outcome.hist, HistEffect::Commit) {
            // 0.32.4: a real page landing in this tab's
            // history makes the tab its own — an adopted
            // (site-opened) tab that commits a page is a
            // normal tab from then on, and a later download
            // capture in it must never close it.
            // AUDIT 0.33 (WS1): the clear is gated on COMMIT
            // only. The pre-0.33 rule cleared on ANY landing
            // (Push/Replace included), and a download file's
            // SourceChanged — classified Push — cleared it
            // before the capture ever decided, converting a
            // download-only tab's close into a start-page
            // conversion. Nothing is lost by the narrower
            // rule: a same-document landing presupposes an
            // earlier Commit of that document, which already
            // cleared the flag. The same Commit also bumps
            // the tab's NAVIGATION EPOCH (0.34) — the SINGLE
            // point where the epoch moves: a real commit
            // landed. A swallowed download finish returns
            // before the reducer runs, so it never reaches
            // here; the coordinator's capture revalidation
            // reads the epoch as "has this tab committed a
            // real page since the capture".
            self.tabs[idx].adopted = false;
            self.tabs[idx].nav_epoch += 1;
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
            if id == self.active
                && (self.teleport_panel_open
                    || self.palette_panel_open
                    || self.downloads_panel_open)
            {
                // The panel names the page it was computed for;
                // a navigation retires it (AUDIT V26 D3).
                // AUDIT 0.31 (A1-F1): the theme picker follows
                // the same dismissal law — through 0.30 only
                // Teleport was retired here, so the picker (and
                // the grown chrome rect) survived navigation.
                // 0.32.2: the downloads popover joins the same
                // law — a committed navigation in the active tab
                // is a content interaction and retires it too.
                if self.teleport_panel_open {
                    self.teleport = None;
                    self.teleport_version += 1;
                    self.teleport_panel_open = false;
                }
                self.palette_panel_open = false;
                self.downloads_panel_open = false;
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
        // The download-finish swallow (0.33 WS1; coordinator
        // 0.34), FIRST — before any URL application, exactly
        // where 0.33 checked its marker. A captured download's
        // navigation completion (WebView2 reports it as a
        // Finished load for the file URL) is dropped whole:
        // no CommitFinish (so no navigation-epoch bump), no
        // history record of the file as a page, no error
        // state, no Shield/Lite tail. The coordinator owns
        // the pending-swallow records now; this asks it
        // whether THIS finish is one.
        if finished && self.dl.navigation_finished(tab_id, &url) {
            let tab = &mut self.tabs[idx];
            tab.loading = false;
            tab.commit_started = false;
            // 0.35, WS2: the captured navigation has settled
            // (as a download, not a page) — its pending chain
            // clears here exactly as a real CommitFinish
            // clears it in apply_url_event; the coordinator
            // keeps its own copy of the chain for the landing.
            tab.pending_chain.clear();
            log::info(&format!("download finish swallowed: tab={tab_id}"));
            self.push_state(true);
            return;
        }
        let root_id = self.server.root_id_for_url(&url);
        let real = !url.is_empty() && url != "about:blank";
        // AUDIT 0.33: the Downloads page's own load is exempt
        // from the allowlist check for its tab, under the
        // same exact-spelling discipline as add_tab's
        // admission (core's allowlist does not know
        // `pico://downloads` yet — flagged in the 0.33
        // report).
        let is_downloads_load =
            self.tabs[idx].is_downloads && (url == DOWNLOADS_URL || url == "pico://downloads/");
        if real && !is_allowed_navigation_url(&url) && !is_downloads_load {
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
        if real {
            // AUDIT 0.32.1 (P3-1): the same defense for
            // credentialed URLs. The layer above checks only
            // the scheme, so a userinfo URL that reached the
            // load stage without the navigation handler's
            // expectation match (e.g. adopted through a
            // redirect) would land in tab state as if typed.
            // Adoption now requires the exact marker the
            // handler set when it allowed the navigation; a
            // plain (non-credentialed) load retires the tab's
            // marker.
            if url_has_userinfo(&url) {
                let marker = {
                    let markers = self
                        .userinfo_adopt
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    markers.get(&tab_id).cloned()
                };
                if !userinfo_adoption_allowed(&url, marker.as_deref()) {
                    log::info(&format!(
                        "page load with credentialed URL not adopted: tab={tab_id} {}",
                        redact_url(&url)
                    ));
                    self.tabs[idx].loading = false;
                    self.push_state(false);
                    return;
                }
            } else {
                let mut markers = self
                    .userinfo_adopt
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                markers.remove(&tab_id);
            }
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
                // V26 D7) — one writer for URL + history. (A
                // swallowed download finish never reaches here;
                // it returned at the top.)
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
        // AUDIT 0.33: a downloads tab's own load finish pushes
        // the page data (forced — the freshly loaded page must
        // render current data even when the signature gate
        // would call it unchanged). 0.34: the committed
        // downloads page also counts as VISITED for the
        // coordinator (failed tasks seen, open notice
        // retired) — the same hook point 0.33 used to know a
        // downloads tab committed.
        if finished && self.tabs[idx].is_downloads {
            let effects = self.dl.page_visited();
            self.exec_dl_effects(effects);
            self.push_downloads_page(true);
        }
        // Heal on load (0.35.1): an internal page (start or
        // downloads) that finished loading re-receives the
        // current theme through the one live channel — a
        // page that loaded before (or while) a theme change
        // landed converges instead of keeping the theme its
        // INIT was baked with. Unconditional but cheap: the
        // push is two small evals at most (one per internal
        // tab).
        if finished && (self.tabs[idx].is_start || self.tabs[idx].is_downloads) {
            self.push_theme_to_internal_tabs();
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
            let _ = proxy_dl.send_event(UserEvent::DownloadBlocked { uri });
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
            // AUDIT 0.34.1 (field addition): the itemized
            // failures and pure skips log BEFORE the summary
            // line — relative paths only, never contents,
            // so the summary's counts are always preceded by
            // the items that make them up.
            for detail in &report.details {
                match &detail.error {
                    Some(error) => log::error(&format!(
                        "data: migration item failed: {} {} — {}",
                        detail.kind, detail.path, error
                    )),
                    None => log::info(&format!(
                        "data: migration item skipped: {} {}",
                        detail.kind, detail.path
                    )),
                }
            }
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
        // 0.34: the Downloads page's initial data — an
        // empty-but-valid snapshot JSON (the frozen
        // `DlSnapshot` shape, seeded from the persisted
        // settings). 0.35.1: no theme in it — `render_downloads`
        // bakes the current theme/palette in at serve time.
        // `push_downloads_page` refreshes it with live data
        // on the first push (which run() makes once the App
        // exists). The start page's card starts empty and is
        // mirrored by the first snapshot push.
        downloads_init: serde_json::json!({
            "version": 0,
            "tasks": [],
            "active_count": 0,
            "failed_unseen": 0,
            "settings": {
                "default_connections": persist.download_connections,
                "max_concurrent": persist.downloads_max_concurrent,
                "speed_limit_kbps": if persist.downloads_speed_limit_kbps == 0 {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(persist.downloads_speed_limit_kbps)
                },
            },
            "notice": null,
            "card": pico_core::core::dlcoord::DlCardView::empty(),
        })
        .to_string(),
        downloads_card: pico_core::core::dlcoord::DlCardView::empty(),
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

    // Downloads (0.32): resolve the downloads folder ONCE, the
    // same way the data dir is resolved — probed facts in, one
    // pure decision out (`core::download::resolve_downloads_dir`).
    // Beside the exe when its Downloads subfolder passes the
    // write+rename probe; else the user's own Downloads folder
    // (USERPROFILE\Downloads, when the variable is set); else
    // beside the exe anyway as the last resort (the manager's
    // writes then fail per-task, visibly, never silently).
    // Task state lives under the data dir's `downloads` folder.
    let downloads_manager = {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let exe_writable = probe_downloads_dir(&exe_dir);
        let user_downloads = std::env::var("USERPROFILE")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|home| PathBuf::from(home).join("Downloads"));
        let used_user_fallback = !exe_writable && user_downloads.is_some();
        let dir = pico_core::core::download::resolve_downloads_dir(
            &exe_dir,
            user_downloads.as_deref(),
            exe_writable,
        );
        if used_user_fallback {
            log::info(&format!(
                "downloads: folder {} (fallback: user Downloads)",
                dir.display()
            ));
        } else {
            log::info(&format!("downloads: folder {}", dir.display()));
        }
        // W2's constructor note: `UserEvent` is private to this
        // module, so the manager takes the proxy through its
        // generic `new_with_proxy` adapter with the event
        // factory — the brief's `new(proxy, …)` shape, one
        // indirection removed from app types.
        let manager = crate::downloads::DownloadManager::new_with_proxy(
            proxy.clone(),
            || UserEvent::DownloadsChanged,
            dir,
            pstore::pico_dir().join("downloads"),
            persist.download_connections,
            persist.downloads_max_concurrent,
            persist.downloads_speed_limit_kbps,
        );
        // 0.34: the coordinator owns the manager from here,
        // through the adapter; its settings seed from the
        // same persisted trio (the persisted speed limit is
        // stored as 0 = off; the coordinator's shape is
        // `Option`).
        Coordinator::new(
            crate::dlbackend::ManagerBackend::new(manager),
            DlSettings {
                default_connections: persist.download_connections,
                max_concurrent: persist.downloads_max_concurrent,
                speed_limit_kbps: if persist.downloads_speed_limit_kbps == 0 {
                    None
                } else {
                    Some(persist.downloads_speed_limit_kbps)
                },
            },
        )
    };

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
        dl: downloads_manager,
        downloads_panel_open: false,
        dl_notice_armed_seq: 0,
        last_dl_page_sig: 0,
        last_dl_card_sig: 0,
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
        context_menu: None,
        pending_focus_addr: false,
        live_start_tabs: std::sync::Arc::new(Mutex::new(std::collections::HashSet::new())),
        live_downloads_tabs: std::sync::Arc::new(Mutex::new(std::collections::HashSet::new())),
        last_layout_sig: String::new(),
        expected_nav: std::sync::Arc::new(Mutex::new(std::collections::HashMap::new())),
        userinfo_adopt: std::sync::Arc::new(Mutex::new(std::collections::HashMap::new())),
    };

    // 0.34: seed the shared Downloads-page init data with the
    // coordinator's REAL snapshot (restored tasks included)
    // before any tab can render the page.
    app.push_downloads_page(true);

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
                                let new_id = app.add_tab(&url, is_start, true);
                                if new_id > 0 {
                                    // 0.32.4: a tab a PAGE opened
                                    // is adopted, not the user's
                                    // — the download unwind's
                                    // CloseTab decision keys off
                                    // this (a start tab is Pico's
                                    // own page, never adopted).
                                    if !is_start {
                                        if let Some(t) = app.tab_mut(new_id) {
                                            t.adopted = true;
                                        }
                                    }
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
                UserEvent::DownloadBlocked { uri } => {
                    // The chrome webview's downloads stay denied
                    // (0.32): the line is the whole handling —
                    // no counter, no notice, no snapshot field.
                    log::info(&format!("chrome download denied: {}", redact_url(&uri)));
                }
                UserEvent::ContentFocus { tab } => {
                    // (The tab id rides along for symmetry with
                    // the other per-tab events; the funnel is
                    // shell-global, like HistoryChanged's push.)
                    let _ = tab;
                    app.close_all_panels();
                }
                UserEvent::DownloadsPageShortcut => {
                    // Ctrl+J from content focus (0.35, WS5):
                    // open the downloads page through the
                    // same code the coordinator's
                    // OpenDownloadsPage effect uses (which
                    // also performs page_visited).
                    log::info("downloads page opened (ctrl+j)");
                    app.open_downloads_page();
                }
                UserEvent::ContextMenu {
                    tab,
                    target,
                    location,
                } => {
                    // Pico's own context menu (0.34.1): the
                    // registration handler suppressed the stock
                    // menu and forwarded the target facts; the
                    // app computes the menu and stores it.
                    app.on_context_menu(tab, target, location);
                }
                UserEvent::DownloadStarting {
                    tab,
                    uri,
                    suggested,
                } => {
                    app.on_download_starting(tab, uri, suggested, false);
                }
                UserEvent::DownloadCookies {
                    tab,
                    token,
                    cookies,
                } => {
                    app.on_download_cookies(tab, token, cookies);
                }
                UserEvent::DownloadCookiesTimeout { tab, token } => {
                    app.on_download_cookies_timeout(tab, token);
                }
                UserEvent::DownloadsChanged => {
                    // The manager's state moved (0.34): the
                    // coordinator diffs task statuses into
                    // transition notices itself; its effects
                    // re-push the surfaces through the
                    // signature gates.
                    let effects = app.dl.backend_changed();
                    app.exec_dl_effects(effects);
                }
                UserEvent::DownloadsAction { tab, body } => {
                    // The ONLY place Downloads-page actions are
                    // decided (0.33; dlcmd 0.34): the IPC handler
                    // forwards the raw body undecided;
                    // `core::dlcmd::page_gate` + the parser
                    // decide centrally.
                    app.on_downloads_page_body(tab, &body);
                }
                UserEvent::DownloadNoticeExpired { seq } => {
                    // The notice's display window ended
                    // (0.32.3; coordinator 0.34): the
                    // coordinator retires it only if it is
                    // still THIS notice — a newer notice
                    // re-arms its own timer and must not be
                    // retired by this one.
                    let effects = app.dl.notice_expired(seq);
                    app.exec_dl_effects(effects);
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
                    // Same pre-teardown download pause as the
                    // WinClose command arm (0.32).
                    app.dl.backend().0.pause_all_and_persist();
                    app.capture_geometry();
                    app.save_state();
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::Focused(false) => {
                    // The user left the window entirely
                    // (0.32.4): panels close, same law as a
                    // click into the page.
                    app.close_all_panels();
                }
                _ => {}
            },
            _ => {}
        }
    });
}
