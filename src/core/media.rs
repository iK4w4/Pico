//! Media environment core: the user-agent composer, the pinned
//! WebView2 browser-argument string, the media-response probe
//! classification, and the profile-persistence witnesses.
//!
//! History: v18 pinned the UA (previously the runtime default
//! went out unpinned and unlogged), pinned the browser args in
//! source (previously whatever wry substituted invisibly), and
//! added the response probe so a "Video unavailable" player
//! leaves Pico-side evidence. v22 changes the UA's FORM:
//!
//! **The v22 UA change (AUDIT V22-14).** v18–v21 composed the
//! Edge-token form (`… Chrome/{v} Safari/537.36 Edg/{v}`). The
//! user's pico.log proved anonymous YouTube playback is refused
//! server-side on his egress (403 `server-refusal` on main
//! formats) — that part is YouTube's trust decision and Pico
//! does not circumvent it. But the same fingerprint class also
//! breaks the fallback the user asked for — *logging in*: Google
//! sign-in refuses browser contexts it identifies as embedded or
//! non-standard ("This browser or app may not be secure"), and
//! an `Edg/`-token UA emitted by a WebView2 host is precisely
//! such a signal. v22 therefore composes the standard **desktop
//! Chrome** form with the runtime's major version
//! (`Chrome/{major}.0.0.0` — Chrome's own UA freezes the minor
//! components to zero). This is a compatibility statement, not a
//! disguise: the string is what a stock desktop Chrome sends,
//! it carries no Pico token, and it is logged at startup.
//!
//! Everything here is pure and host-testable; the GUI layer only
//! wires the results into wry/webview2-com calls.

use std::collections::HashMap;
use std::path::Path;

/// Chromium version used for the UA when the installed WebView2
/// runtime version cannot be obtained. Any plausible recent
/// version is better than a hole in the UA; the composed string
/// stays a standard desktop Chrome UA either way.
pub const FALLBACK_BROWSER_VERSION: &str = "140.0.0.0";

/// The UA form identifier written to the startup log line, so a
/// pico.log always states which compatibility form is in use.
pub const UA_FORM: &str = "desktop-chrome";

/// Sanitize a runtime-supplied version string into something
/// safe to embed in a UA: digits and dots only, leading digit,
/// at least one dot, bounded length. Anything else → `None`
/// (the caller falls back to [`FALLBACK_BROWSER_VERSION`]).
/// This is a trust boundary: the UA is later placed into HTTP
/// headers by the engine, so no whitespace or header-shaped
/// content may survive.
fn sanitize_version(raw: &str) -> Option<String> {
    let v: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if v.len() < 3 || v.len() > 24 {
        return None;
    }
    if !v.starts_with(|c: char| c.is_ascii_digit()) || !v.contains('.') {
        return None;
    }
    if v.contains("..") || v.ends_with('.') {
        return None;
    }
    Some(v)
}

/// Compose the explicit user agent for content WebViews: the
/// standard desktop Chrome UA. The major version is taken from
/// the installed WebView2 runtime when known (so the UA ages
/// with the runtime instead of fossilizing), rendered as
/// `Chrome/{major}.0.0.0` exactly as stock Chrome freezes it;
/// [`FALLBACK_BROWSER_VERSION`] otherwise. The string
/// deliberately contains **no Pico/app token and no `Edg/`
/// token** — a UA that names a niche wrapper, or an Edge token
/// coming from a non-Edge host, is exactly what sign-in
/// fingerprinting looks for (AUDIT V22-14).
pub fn compose_user_agent(runtime_version: Option<&str>) -> String {
    let version = runtime_version
        .and_then(sanitize_version)
        .unwrap_or_else(|| FALLBACK_BROWSER_VERSION.to_string());
    let major = version.split('.').next().unwrap_or("140");
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
    )
}

/// The additional browser arguments Pico passes to WebView2,
/// pinned in source (AUDIT V18-2). Before v18 this string was
/// whatever wry 0.53.5 substituted when the app set none; the
/// pinned value is byte-identical to that substituted default —
/// pinning changes *visibility*, not behavior — and
/// [`args_are_media_safe`] keeps any future edit inside the
/// media-safe allowlist.
///
/// The SAME string must be set on every WebView builder: all of
/// Pico's WebViews share one user-data folder, and WebView2
/// environments created over one folder must agree on options
/// (wry's `with_additional_browser_args` warning).
pub const PICO_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// Features that may be named in a `--disable-features=` list:
/// Edge UI extras only (mini menu, PDF UI, SmartScreen). No
/// media, codec, EME, GPU, or transport feature may ever join
/// this list.
const ALLOWED_DISABLED_FEATURES: &[&str] = &["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection"];

/// Is an additional-browser-arguments string media-safe? Every
/// whitespace-separated flag must be on the allowlist:
/// `--disable-features=<only Edge UI extras>` and the
/// gesture-free autoplay policy. Anything else — any other flag,
/// any other disable-features entry, an `--enable-features`, a
/// proxy, a media/GPU/QUIC/WebRTC switch of any spelling — is
/// refused. An empty string is safe (it sets nothing).
pub fn args_are_media_safe(args: &str) -> bool {
    args.split_whitespace().all(|flag| {
        if flag == "--autoplay-policy=no-user-gesture-required" {
            return true;
        }
        if let Some(list) = flag.strip_prefix("--disable-features=") {
            return !list.is_empty()
                && list
                    .split(',')
                    .all(|f| ALLOWED_DISABLED_FEATURES.contains(&f));
        }
        false
    })
}

/// The profile-persistence witness (v22, AUDIT V22-16): does the
/// pinned WebView2 profile folder already hold a cookies
/// database? Existence ONLY — the file's contents are never
/// read, parsed, or logged. WebView2 keeps cookies at
/// `Default/Network/Cookies` (current) or `Default/Cookies`
/// (older runtimes); either location counts. The startup log
/// line built from this (`profile: persistent, cookies db
/// present=<bool>`) is how a pico.log proves, on the user's own
/// machine, that a previous session's sign-in state had
/// somewhere to persist to.
pub fn cookies_db_present(profile_dir: &Path) -> bool {
    profile_dir
        .join("Default")
        .join("Network")
        .join("Cookies")
        .exists()
        || profile_dir.join("Default").join("Cookies").exists()
}

/// A media response the probe should log: the request targets a
/// media stream. Carries the request's host (no port, no
/// userinfo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaProbeTarget {
    pub host: String,
}

/// Does this request URI belong to the media probe? Two shapes:
/// any host at or under `googlevideo.com` (YouTube's media CDN),
/// or a `/videoplayback` path on any host (the player endpoint
/// shape, wherever it is served from). The query string never
/// participates (signed URLs carry tokens there — and the probe
/// logs hosts and statuses only, never the URI).
pub fn probe_target(uri: &str) -> Option<MediaProbeTarget> {
    let (_, rest) = uri.split_once("://")?;
    let authority_end = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    // Drop userinfo before reading the host (`a@b` — the host is b).
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        // IPv6 literal: host runs to the closing bracket.
        stripped.split(']').next().unwrap_or(stripped)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    // The path is what follows the authority, minus query/fragment.
    let tail = &rest[authority_end..];
    let path_end = tail.find(|c| ['?', '#'].contains(&c)).unwrap_or(tail.len());
    let path = &tail[..path_end];
    let is_googlevideo = host == "googlevideo.com" || host.ends_with(".googlevideo.com");
    if is_googlevideo || path.contains("/videoplayback") {
        Some(MediaProbeTarget { host })
    } else {
        None
    }
}

/// What a media response's HTTP status says about a "Video
/// unavailable" report — the classification pico.log records so
/// the next report is evidence instead of a screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaClass {
    /// 200/206: the stream was served. A player that still says
    /// "unavailable" is then a player/config-side problem.
    StreamServed,
    /// 401/403: the server refused the stream (the bot-mitigation /
    /// region / sign-in class — decided on the server's side).
    ServerRefusal,
    /// No HTTP status at all: transport.
    TransportFailure,
    /// Any other status: recorded verbatim for diagnosis.
    Unexpected(i32),
}

impl MediaClass {
    pub fn label(self) -> &'static str {
        match self {
            MediaClass::StreamServed => "served",
            MediaClass::ServerRefusal => "server-refusal",
            MediaClass::TransportFailure => "transport",
            MediaClass::Unexpected(_) => "unexpected",
        }
    }
}

pub fn classify_media_status(status: Option<i32>) -> MediaClass {
    match status {
        Some(200) | Some(206) => MediaClass::StreamServed,
        Some(401) | Some(403) => MediaClass::ServerRefusal,
        Some(0) | None => MediaClass::TransportFailure,
        Some(other) => MediaClass::Unexpected(other),
    }
}

/// Per-tab cap on media-probe log lines per session. The probe
/// exists to classify a failure, not to mirror traffic: a
/// playing video fires hundreds of segment responses, and
/// pico.log must stay a diagnostic file, not a firehose.
pub const MEDIA_PROBE_LOG_LIMIT_PER_TAB: usize = 20;

/// Counting budget for probe lines: the first
/// [`MEDIA_PROBE_LOG_LIMIT_PER_TAB`] targets of each tab may
/// log; everything after that is dropped silently.
#[derive(Debug, Default)]
pub struct MediaProbeBudget {
    used: HashMap<u64, usize>,
}

impl MediaProbeBudget {
    pub fn new() -> Self {
        MediaProbeBudget {
            used: HashMap::new(),
        }
    }

    /// Forget a closed tab's budget (AUDIT 0.32.1, P3-5):
    /// through 0.32 nothing ever removed an entry, so the map
    /// grew by one per tab ever opened, for the whole
    /// session — and a recycled tab id inherited a stranger's
    /// spent budget.
    pub fn forget_tab(&mut self, tab: u64) {
        self.used.remove(&tab);
    }

    /// May one more probe line be logged for `tab`? Consumes the
    /// slot when it answers yes.
    pub fn should_log(&mut self, tab: u64) -> bool {
        let n = self.used.entry(tab).or_insert(0);
        if *n >= MEDIA_PROBE_LOG_LIMIT_PER_TAB {
            return false;
        }
        *n += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ua_is_chrome_form_with_runtime_major() {
        let ua = compose_user_agent(Some("151.0.4129.78"));
        assert_eq!(
            ua,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36"
        );
        assert!(!ua.contains("Edg/"));
        assert!(!ua.contains("Pico"));
    }

    #[test]
    fn cookies_db_witness_checks_both_locations() {
        let dir = std::env::temp_dir().join(format!("pico-v23-media-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!cookies_db_present(&dir));
        let net = dir.join("Default").join("Network");
        std::fs::create_dir_all(&net).unwrap();
        std::fs::write(net.join("Cookies"), b"").unwrap();
        assert!(cookies_db_present(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
