//! Address-bar input parsing and navigation-target validation.
//!
//! Accepts full HTTP(S) URLs, `localhost:5173`, bare ports (`5173`,
//! `5173/app`), plain domains (`example.com`), local Windows paths
//! (`C:\dev\page.html`) and local `file://` URLs, and decides whether
//! the target is a web URL or a local file to serve through the
//! static server.
//!
//! Security rules enforced here (see SECURITY.md):
//! - Only `http:`, `https:`, Pico's exact internal start URL, and
//!   local `file:` targets can survive parsing. Every other scheme
//!   is rejected, including whitespace/control-character
//!   obfuscations.
//! - UNC / network-namespace paths are rejected before any
//!   filesystem call can touch them, so Pico never triggers SMB
//!   authentication to a potentially hostile server (NTLM
//!   disclosure) by "opening" one.
//! - A `file:` URL must have an empty or `localhost` authority; a
//!   remote authority is a network path in disguise.
//!
//! v22 addition: [`route_new_window`] — the pure routing decision
//! for `window.open` / `target=_blank` requests (AUDIT V22-10/15).
//! It lives here, with the rest of the URL policy, so the shell's
//! handler is a one-line forward and the whole matrix is unit
//! tested without a window.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Url(String),
    File(PathBuf),
}

pub const START_URL: &str = "pico://start";

fn clean_input(raw: &str) -> Option<String> {
    // Trim whitespace only. Any control character that survives
    // (including non-whitespace controls like \x01 at the edges)
    // is rejected outright below rather than silently laundered.
    // AUDIT 0.31 (A6-F8): invisible format characters (the
    // shared set in core::redact — zero-width marks, the BOM,
    // the Unicode Tags range, …) are rejected the same way: a
    // URL carrying one renders identically to one without it,
    // so it must never be navigated to as if it were the
    // address the user sees.
    let s = raw.trim_matches(|c: char| c.is_whitespace());
    if s.is_empty()
        || s.chars()
            .any(|c: char| c.is_control() || crate::core::redact::is_invisible_format(c))
    {
        return None;
    }
    Some(s.to_string())
}

fn scheme_prefix(s: &str) -> Option<&str> {
    let colon = s.find(':')?;
    let prefix = &s[..colon];
    if prefix.is_empty() || prefix.len() > 32 {
        return None;
    }
    let mut chars = prefix.chars();
    if !chars
        .next()
        .map(|c| c.is_ascii_alphabetic())
        .unwrap_or(false)
    {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some(prefix)
}

fn has_whitespace(s: &str) -> bool {
    s.chars().any(|c: char| c.is_whitespace())
}

/// True for address strings that name a Windows network location
/// or device namespace. Purely lexical: callers must run it
/// *before* `exists`, `metadata`, or `canonicalize`, because
/// merely resolving a UNC path can make Windows contact the
/// named server.
pub fn is_unc_address(s: &str) -> bool {
    let t = s.trim_start_matches(|c: char| c.is_control() || c.is_whitespace());
    t.starts_with("\\\\") || t.starts_with("//")
}

/// Whether `path` is safe to hand to filesystem APIs as a *local*
/// path. Allows ordinary absolute/relative paths and the local
/// verbatim-drive form Windows canonicalisation produces
/// (`\\?\C:\...`); rejects UNC, verbatim-UNC, device-namespace
/// and NT-namespace spellings.
pub fn is_safe_local_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.chars().any(|c: char| c.is_control()) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("\\??\\")
        || lower.starts_with("\\device\\")
        || lower.starts_with("\\globalroot\\")
    {
        return false;
    }
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        // Only \\?\X:\ or \\?\X:/ (a local drive) is acceptable.
        let b = rest.as_bytes();
        return b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b.len() == 2 || b[2] == b'\\' || b[2] == b'/');
    }
    if s.starts_with("\\\\") || s.starts_with("//") {
        return false;
    }
    true
}

fn valid_port(text: &str) -> Option<u16> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match text.parse::<u16>() {
        Ok(0) | Err(_) => None,
        Ok(p) => Some(p),
    }
}

fn parse_host_port(s: &str) -> Option<Target> {
    // Bracketed IPv6, with optional port.
    if s.starts_with('[') {
        let close = s.find(']')?;
        let host = &s[1..close];
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return None;
        }
        let rest = &s[close + 1..];
        if rest.is_empty() || matches!(rest.chars().next(), Some('/' | '?' | '#')) {
            // No port: bare host, or host + path/query/fragment
            // (AUDIT V28 A6-6 — the suffix forms used to fall
            // through to a Google search).
            return Some(Target::Url(format!("http://{s}")));
        }
        let port_and_tail = rest.strip_prefix(':')?;
        let tail_at = port_and_tail
            .find(|c| ['/', '?', '#'].contains(&c))
            .unwrap_or(port_and_tail.len());
        valid_port(&port_and_tail[..tail_at])?;
        return Some(Target::Url(format!("http://{s}")));
    }

    let (host, rest) = s.split_once(':')?;
    if host.is_empty() || has_whitespace(host) || host.contains(['/', '?', '#', '\\', '@']) {
        return None;
    }
    let tail_at = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    valid_port(&rest[..tail_at])?;
    // A digits-and-dots host is an IPv4 claim: if it does not
    // parse as one (`256.256.256.256:8080`), it is not a public
    // hostname either — refuse rather than navigate to a
    // dotted-quad-shaped https host (AUDIT V28 A6-12).
    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return if host.parse::<std::net::Ipv4Addr>().is_ok() {
            Some(Target::Url(format!("http://{s}")))
        } else {
            None
        };
    }
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some(Target::Url(format!("http://{s}")));
    }
    // A dotted or single-label host with an explicit port: keep
    // the historical https interpretation for public hosts.
    if host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    {
        return Some(Target::Url(format!("https://{s}")));
    }
    None
}

/// True when the input is an explicit host:port (or bare :port)
/// whose port is present but impossible — `0`, over 65535, or
/// non-numeric (AUDIT V28 A6-5). `parse_host_port` returning
/// `None` is ambiguous ("not this shape" vs "this shape, bad
/// port"); without this check the bad-port numeric forms fell
/// through the whole parser into a Google search, while the
/// alphabetic forms were refused by the scheme backstop — the
/// matrix now refuses every explicit bad port uniformly.
fn explicit_port_malformed(s: &str) -> bool {
    let head = s.split(['/', '?', '#']).next().unwrap_or_default();
    if let Some(bracket) = head.strip_prefix('[') {
        if let Some((_, rest)) = bracket.split_once(']') {
            if rest.is_empty() {
                return false;
            }
            if let Some(port) = rest.strip_prefix(':') {
                return valid_port(port).is_none();
            }
            // AUDIT 0.31 (A6-F7): a bracketed host followed by
            // junk that is not ':' (a port), '/', '?' or '#'
            // (already split off into `head` above) — e.g.
            // `[::1]8080`, `[::1]xyz` — is a malformed
            // host:port claim, not a search term. Through
            // 0.30.1 it escaped this refusal and fell through
            // the whole parser into a Google search.
            return true;
        }
        return false;
    }
    let Some((host, port)) = head.split_once(':') else {
        return false;
    };
    if port.contains(':') {
        return false; // unbracketed multi-colon: not a host:port form
    }
    if host.is_empty() {
        return valid_port(port).is_none(); // bare :port
    }
    // A digits-and-dots host that is not a valid IPv4 address is
    // a malformed host:port claim whatever the port (`256.256…`).
    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && host.parse::<std::net::Ipv4Addr>().is_err()
    {
        return true;
    }
    let host_eligible = host.eq_ignore_ascii_case("localhost")
        || host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
    host_eligible && valid_port(port).is_none()
}

/// True when an http(s) URL carries userinfo (`user:pass@host`).
/// Typed credentials are the user's own act and stay untouched;
/// PAGE-initiated credentialed URLs are another matter (see
/// `route_new_window`).
fn has_userinfo(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority.contains('@')
}

fn is_drive_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// Parse the origin-relevant parts of an HTTP(S) URL without a URL
/// crate: `(scheme, lowercase host, effective port)`. `None` for
/// anything malformed or carrying raw whitespace, controls, or
/// backslashes (which browsers may reinterpret).
pub fn web_origin(url: &str) -> Option<(String, String, u16)> {
    // AUDIT 0.31 (A6-F8): invisible format characters (the
    // shared set in core::redact) refuse the URL outright,
    // exactly like controls — a host containing U+FEFF or a
    // tag character is not the host it renders as, and must
    // not be treated as the same origin (or as local).
    if url.chars().any(|c: char| {
        c.is_control() || c.is_whitespace() || crate::core::redact::is_invisible_format(c)
    }) || url.contains('\\')
    {
        return None;
    }
    let (scheme_raw, rest) = url.split_once("://")?;
    let scheme = scheme_raw.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority_end = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return None;
    }
    // Userinfo, when present, is not part of the origin.
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = if let Some(stripped) = host_port.strip_prefix('[') {
        let close = stripped.find(']')?;
        let host = &stripped[..close];
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return None;
        }
        let tail = &stripped[close + 1..];
        let port = if tail.is_empty() {
            None
        } else {
            Some(valid_port(tail.strip_prefix(':')?)?)
        };
        (host.to_ascii_lowercase(), port)
    } else {
        if host_port.matches(':').count() > 1 {
            return None;
        }
        match host_port.split_once(':') {
            Some((h, p)) => (h.to_ascii_lowercase(), Some(valid_port(p)?)),
            None => (host_port.to_ascii_lowercase(), None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let default_port = if scheme == "http" { 80 } else { 443 };
    Some((scheme, host, port.unwrap_or(default_port)))
}

/// The navigation allowlist used for typed input, argv,
/// new-window requests and in-page navigations alike: HTTP(S),
/// or Pico's exact internal start URL. Nothing else may ever be
/// loaded as a document.
pub fn is_allowed_navigation_url(url: &str) -> bool {
    url == START_URL || url == "pico://start/" || web_origin(url).is_some()
}

/// Fold wry's internal rewrite of Pico's custom scheme back to
/// the canonical `pico://` form. On Windows, WebView2 cannot host
/// a `pico://` scheme directly, so wry rewrites `pico://<host>/...`
/// to `https://pico.<host>/...` before the URL reaches Pico's
/// handlers; both spellings must compare equal for the internal
/// pages.
///
/// AUDIT V15-2: the fold applies ONLY to Pico's own internal
/// hosts (`chrome`, `start`, `recent`, `social`, `pick-*`, and
/// v24's `recent-remove` / `recents-clear` / `recents-visibility`
/// — single DNS labels with no dot). v14 folded *any*
/// `https://pico.*` URL, which made real websites on `pico.*`
/// domains (e.g. `https://pico.dev/`) unbrowsable. A host
/// containing a dot, or not in the internal set, is a real web
/// address and is returned unchanged.
///
/// AUDIT 0.30 (A1/A4): `stats-reset` joins the list — the
/// savings band's Reset anchor (`pico://stats-reset`) needs
/// the same fold as the v24 recents actions, or its legacy
/// transport dead-ends on the unfolded rewrite.
pub fn normalize_pico_url(u: &str) -> String {
    for prefix in ["https://pico.", "http://pico."] {
        if let Some(rest) = u.strip_prefix(prefix) {
            let host_end = rest
                .find(|c| ['/', '?', '#'].contains(&c))
                .unwrap_or(rest.len());
            let host = &rest[..host_end];
            let internal = !host.contains('.')
                && (host == "chrome"
                    || host == "start"
                    || host == "recent"
                    || host == "social"
                    || host == "recent-remove"
                    || host == "recents-clear"
                    || host == "recents-visibility"
                    || host == "stats-reset"
                    || host.starts_with("pick-"));
            if internal {
                return format!("pico://{rest}");
            }
            return u.to_string();
        }
    }
    u.to_string()
}

/// True when two URLs share scheme, host and effective port.
pub fn same_origin(a: &str, b: &str) -> bool {
    match (web_origin(a), web_origin(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// What the shell should do with a content webview's new-window
/// request (`window.open`, `target=_blank`, middle-click on some
/// flows). Decided by [`route_new_window`]; the shell only
/// executes the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewWindowRoute {
    /// Open the (normalized) URL in a fresh tab. The shell still
    /// applies the 30-tab cap at execution time — a cap refusal
    /// degrades the route to a deny, never to a bypass.
    OpenTab(String),
    /// Refuse the request. The payload is a short static reason
    /// for the pico.log line.
    Deny(&'static str),
}

/// The new-window routing decision (v22, AUDIT V22-10/15).
///
/// Rules, in order:
/// 1. The URL is folded through [`normalize_pico_url`] first, so
///    wry's rewrite of an internal URL cannot smuggle it past —
///    or into — the web rules.
/// 2. Internal ACTION URLs (`pico://recent/…`, `pico://social/…`,
///    `pico://pick-file`) are denied: actions may only be decided
///    by the gated start-page dispatch, never by a popup. (The
///    start page itself is not an action; `pico://start` opens
///    as a tab like Home does.)
/// 3. Anything on the navigation allowlist (http/https, or the
///    start page) opens in a new tab — this is what makes
///    full-page OAuth sign-in flows and ordinary `target=_blank`
///    links work.
/// 4. Everything else (`javascript:`, `data:`, `file:`,
///    `about:blank` script-popups, malformed) is denied.
pub fn route_new_window(raw_url: &str) -> NewWindowRoute {
    let url = normalize_pico_url(raw_url);
    if crate::core::action::is_action_url(&url) {
        return NewWindowRoute::Deny("internal-action-url");
    }
    // AUDIT V28 A6-12: a page must not mint a tab whose address
    // bar leads with a trusted-looking credentialed prefix
    // (`https://google.com:…@evil…`). Typed credentialed URLs
    // are the user's own act and are unaffected — this gate is
    // only on the page-initiated path.
    if has_userinfo(&url) {
        return NewWindowRoute::Deny("credentialed-url");
    }
    if is_allowed_navigation_url(&url) {
        // Canonicalize the start page's trailing-slash spelling
        // so the shell's `url == START_URL` check recognizes it.
        if url == "pico://start/" {
            return NewWindowRoute::OpenTab(START_URL.to_string());
        }
        return NewWindowRoute::OpenTab(url);
    }
    NewWindowRoute::Deny("scheme-not-allowed")
}

/// Where an address-bar submission goes (AUDIT V26, defect D5).
///
/// Through v25, submitting the address bar while the start tab
/// was active opened ANOTHER tab and left the start tab sitting
/// there — the user's words: "it should search there instead of
/// making another tab for no reason." v26 converts the start
/// tab in place: it becomes the content tab for the submitted
/// target. Start-page *cards* (services, recents, socials) are
/// launcher actions, not address-bar submissions, and keep
/// their open-in-new-tab behavior — the distinction is pinned
/// here and in tests/action_tests.rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitRoute {
    /// Start tab + web target: convert the start tab into a
    /// content tab showing the target.
    ConvertStart,
    /// Start tab + file target: convert the start tab into the
    /// content tab serving the file (via the static server).
    ConvertStartWithFile,
    /// Content tab + web target: navigate the current tab (the
    /// long-standing behavior).
    LoadCurrent,
    /// Content tab + file target: open the file in a new tab
    /// (the long-standing `open_path` behavior; only the start
    /// tab's submissions changed in v26).
    OpenFileInNewTab,
}

/// The routing decision for one parsed address-bar submission.
pub fn submit_route(active_is_start: bool, target: &Target) -> SubmitRoute {
    match (active_is_start, target) {
        (true, Target::Url(_)) => SubmitRoute::ConvertStart,
        (true, Target::File(_)) => SubmitRoute::ConvertStartWithFile,
        (false, Target::Url(_)) => SubmitRoute::LoadCurrent,
        (false, Target::File(_)) => SubmitRoute::OpenFileInNewTab,
    }
}

/// True when `candidate` names the start page once its
/// query/fragment and trailing slashes are stripped (AUDIT
/// 0.31, A6-F6): the action dispatch has always compared
/// start-page URLs that way, but `parse_input`'s comparisons
/// were exact, so typing `pico://start?x=1`, `pico://start#x`
/// or `pico://start//` missed the start page and fell through
/// to the `pico:` refusal. The comparison itself stays exact
/// after the strip — case differences and trailing-dot forms
/// (`pico://start.`) are still NOT the start page.
fn is_start_spelling(candidate: &str) -> bool {
    let base = candidate.split(['?', '#']).next().unwrap_or_default();
    base.trim_end_matches('/') == START_URL
}

/// Parse raw address-bar text. `None` for empty or unsafe input.
pub fn parse_input(raw: &str) -> Option<Target> {
    let s = clean_input(raw)?;
    let lower = s.to_ascii_lowercase();

    // Network locations are refused before anything can resolve.
    if is_unc_address(&s) {
        return None;
    }

    // AUDIT 0.30.1 (A6-M1 residue): wry's folded spelling of
    // the start URL, typed by hand — `https://pico.start/` —
    // is the start page, not a website. Fold it BEFORE the
    // http branch below can claim it as a web URL: through
    // 0.30 it routed as one, dead-ending on content tabs and
    // stranding a converted start tab, and the shell's M1
    // go-home guard never saw it because it compares against
    // the canonical form. The fold only knows Pico's internal
    // hosts, so real `pico.*` sites are untouched.
    {
        let folded = normalize_pico_url(&s);
        // AUDIT 0.31 (A6-F6): compare through
        // `is_start_spelling`, so the folded form carrying a
        // query, fragment or extra trailing slashes is the
        // start page too.
        if is_start_spelling(&folded) {
            return Some(Target::Url(START_URL.to_string()));
        }
    }

    if lower.starts_with("http://") || lower.starts_with("https://") {
        return if web_origin(&s).is_some() {
            Some(Target::Url(s.clone()))
        } else {
            None
        };
    }
    // AUDIT 0.31 (A6-F6): same stripped comparison for the
    // direct `pico://start` spelling.
    if is_start_spelling(&lower) {
        return Some(Target::Url(START_URL.to_string()));
    }
    if lower.starts_with("pico:") {
        return None;
    }
    if lower.starts_with("file:") {
        return file_url_to_path(&s).map(Target::File);
    }
    if is_drive_path(&s) {
        return Some(Target::File(PathBuf::from(&s)));
    }
    // `localhost` with no port.
    if lower == "localhost"
        || lower.starts_with("localhost/")
        || lower.starts_with("localhost?")
        || lower.starts_with("localhost#")
    {
        return Some(Target::Url(format!("http://{s}")));
    }
    // An explicit host:port with an impossible port is refused
    // uniformly, whatever the host's shape (AUDIT V28 A6-5).
    if explicit_port_malformed(&s) {
        return None;
    }
    // host:port forms (localhost, IPv4, [IPv6], domains).
    if let Some(t) = parse_host_port(&s) {
        return Some(t);
    }
    // Bare port, optionally with a path: "5173", "5173/app", ":5173"
    {
        let t = s.strip_prefix(':').unwrap_or(&s);
        let head: String = t
            .chars()
            .take_while(|c| !matches!(c, '/' | '?' | '#'))
            .collect();
        if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) {
            // A bare number that cannot be a port (`0`, `65536`,
            // `99999`) is a refused port claim, not a search term
            // (AUDIT V28 A6-5) — the `?` refuses the whole input.
            let port = valid_port(&head)?;
            let rest = &t[head.len()..];
            let rest = if rest.is_empty() { "/" } else { rest };
            if !has_whitespace(rest) {
                return Some(Target::Url(format!("http://127.0.0.1:{port}{rest}")));
            }
        }
    }
    // Any remaining explicit scheme is not on the allowlist —
    // including a scheme-SHAPED word ("recipe: …"), which is
    // refused even when prose follows the colon: a single clean
    // token before a colon is a scheme claim, not a sentence.
    // A colon whose prefix contains whitespace AND whose payload
    // is a single token is an obfuscated scheme ("java
    // script:alert(1)") — refuse it too. But a MULTI-WORD prefix
    // with whitespace after the colon as well ("how to: cook
    // rice") is prose, not a scheme; v24 lets it fall through
    // to the search tail below.
    if let Some(colon) = s.find(':') {
        let prefix = &s[..colon];
        if scheme_prefix(&s).is_some()
            || (has_whitespace(prefix) && !has_whitespace(&s[colon + 1..]))
        {
            return None;
        }
    }
    // An existing local file (absolute or relative). The safety
    // check runs before `exists`, by design.
    let p = PathBuf::from(&s);
    if is_safe_local_path(&p) && p.exists() {
        return Some(Target::File(p));
    }
    // v24 tail (AUDIT V24-4): what is left is one of —
    //   * a plausible bare host ("example.com",
    //     "sub.example.co.uk/path", a bare IPv4) → navigate;
    //   * a file-name-shaped token ("LLM.html") → the FILE
    //     logic's legacy tail (the https fallback it has always
    //     had) — a file name is never turned into a search;
    //   * anything else — a single label ("youtube") or any
    //     prose with spaces ("how to cook rice") → a Google
    //     search for exactly what was typed.
    // Backslash-bearing remnants stay refused, as before.
    if s.contains('\\') {
        return None;
    }
    if has_whitespace(&s) {
        return Some(Target::Url(google_search_url(&s)));
    }
    // Bare loopback / unspecified IPv4 with no port goes over
    // plain HTTP, like its with-port form (AUDIT V28 A6-12):
    // typing `127.0.0.1` for a dev server must not attempt TLS
    // to 443. (A bare PRIVATE address keeps the historical
    // https interpretation — that half is pinned by test.)
    {
        let hostpart = s.split(['/', '?', '#']).next().unwrap_or_default();
        if let Ok(ip) = hostpart.parse::<std::net::Ipv4Addr>() {
            if ip.is_loopback() || ip.is_unspecified() {
                return Some(Target::Url(format!("http://{s}")));
            }
        }
    }
    if is_plausible_bare_host(&s) || has_file_extension(&s) {
        return Some(Target::Url(format!("https://{s}")));
    }
    Some(Target::Url(google_search_url(&s)))
}

/// The one search URL shape Pico builds (v24, AUDIT V24-4): the
/// user's own typed text, percent-encoded, as the `q` of a
/// Google search. Nothing else is ever appended — no client id,
/// no referral parameters.
pub fn google_search_url(query: &str) -> String {
    format!(
        "https://www.google.com/search?q={}",
        percent_encode_query(query)
    )
}

/// RFC 3986 percent-encoding for a query value: unreserved
/// characters pass through, everything else (spaces included)
/// becomes `%XX` over the UTF-8 bytes.
fn percent_encode_query(q: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(q.len());
    for b in q.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xF) as usize] as char);
            }
        }
    }
    out
}

/// Is this (whitespace-free, backslash-free) input shaped like a
/// host a user would navigate to? A bare IPv4, or ≥2 well-formed
/// DNS labels whose final label is an alphabetic TLD of ≥2
/// characters. A single label ("youtube") is NOT a host — v24
/// searches it instead.
fn is_plausible_bare_host(s: &str) -> bool {
    let host = s.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let well_formed = labels.iter().all(|l| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    if !well_formed {
        return false;
    }
    let tld = labels[labels.len() - 1];
    tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic())
}

/// File-name extensions the address bar treats as FILE-family
/// tokens, never as search terms (v24, AUDIT V24-4): a typed
/// "LLM.html" belongs to the file logic — the existing-file
/// check above first, the legacy https tail after — exactly as
/// it behaved before search existed.
const FILE_EXTENSIONS: &[&str] = &[
    "html", "htm", "xhtml", "shtml", "css", "js", "mjs", "json", "txt", "md", "pdf", "png", "jpg",
    "jpeg", "gif", "svg", "webp", "ico", "xml", "csv", "zip", "mp3", "mp4", "wav",
];

/// Does the input's final path segment end in a known file
/// extension (case-insensitive)?
fn has_file_extension(s: &str) -> bool {
    let last = s.rsplit(['/', '\\']).next().unwrap_or(s);
    match last.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty() && FILE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
        }
        None => false,
    }
}

/// Convert a local `file://` URL to a path. `None` for remote
/// authorities, malformed percent-encoding, embedded controls, or
/// backslash tricks.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("file://") {
        return None;
    }
    let after = &url["file://".len()..];
    // file:////server/share is a UNC path wearing a URL costume.
    if after.starts_with("//") {
        return None;
    }
    let (authority, path_part) = match after.find('/') {
        Some(0) => ("", after),
        Some(i) => (&after[..i], &after[i..]),
        None => return None,
    };
    let combined: String = if authority.len() == 2
        && authority.as_bytes()[0].is_ascii_alphabetic()
        && authority.as_bytes()[1] == b':'
    {
        format!("/{authority}{path_part}")
    } else {
        let a = authority.to_ascii_lowercase();
        if !a.is_empty() && a != "localhost" {
            return None;
        }
        path_part.to_string()
    };
    let decoded = crate::core::server::percent_decode(&combined)?;
    if decoded.chars().any(|c: char| c.is_control()) || decoded.contains('\\') {
        // Backslashes have no business in a URL path. (The
        // drive-form conversion below introduces them itself,
        // from '/'.)
        return None;
    }
    if decoded.starts_with("//") {
        return None;
    }
    let bytes = decoded.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && (bytes[3] == b'/' || bytes[3] == b'\\')
    {
        #[cfg(windows)]
        {
            return Some(PathBuf::from(decoded[1..].replace('/', "\\")));
        }
        #[cfg(not(windows))]
        {
            return Some(PathBuf::from(&decoded[1..]));
        }
    }
    Some(PathBuf::from(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_window_matrix() {
        assert_eq!(
            route_new_window("https://accounts.google.com/signin"),
            NewWindowRoute::OpenTab("https://accounts.google.com/signin".to_string())
        );
        assert_eq!(
            route_new_window("http://localhost:5173/app"),
            NewWindowRoute::OpenTab("http://localhost:5173/app".to_string())
        );
        assert!(matches!(
            route_new_window("pico://social/telegram"),
            NewWindowRoute::Deny(_)
        ));
        assert!(matches!(
            route_new_window("pico://recent/0"),
            NewWindowRoute::Deny(_)
        ));
        assert!(matches!(
            route_new_window("pico://pick-file"),
            NewWindowRoute::Deny(_)
        ));
        assert!(matches!(
            route_new_window("javascript:alert(1)"),
            NewWindowRoute::Deny(_)
        ));
        assert!(matches!(
            route_new_window("data:text/html,hi"),
            NewWindowRoute::Deny(_)
        ));
        assert!(matches!(
            route_new_window("file:///C:/x.html"),
            NewWindowRoute::Deny(_)
        ));
        assert!(matches!(
            route_new_window("about:blank"),
            NewWindowRoute::Deny(_)
        ));
        // wry's folded spelling of an action URL is still an action.
        assert!(matches!(
            route_new_window("https://pico.social/telegram"),
            NewWindowRoute::Deny(_)
        ));
        // The start page itself is a page, not an action.
        assert_eq!(
            route_new_window("pico://start"),
            NewWindowRoute::OpenTab(START_URL.to_string())
        );
    }

    #[test]
    fn start_spellings_with_query_fragment_or_extra_slashes_go_home() {
        // AUDIT 0.31 (A6-F6): the three newly accepted
        // spellings — query, fragment and doubled trailing
        // slash — go home like the action dispatch's
        // comparison already allowed.
        for spelling in ["pico://start?x=1", "pico://start#x", "pico://start//"] {
            assert_eq!(
                parse_input(spelling),
                Some(Target::Url(START_URL.to_string())),
                "{spelling} is the start page"
            );
        }
        // The folded spelling carrying a query goes home too.
        assert_eq!(
            parse_input("https://pico.start/?x=1"),
            Some(Target::Url(START_URL.to_string()))
        );
        // Deliberate pins stay: a trailing-dot form is NOT
        // the start page and stays refused.
        assert_eq!(parse_input("pico://start."), None);
        assert_eq!(parse_input("pico://start./"), None);
    }

    #[test]
    fn bracketed_host_followed_by_junk_is_refused() {
        // AUDIT 0.31 (A6-F7): the C8 bad-port matrix grows
        // the junk-after-bracket forms — a bracketed host
        // followed by anything that is not ':', '/', '?' or
        // '#' is malformed, not a Google search.
        for bad in ["[::1]8080", "[::1]xyz"] {
            assert_eq!(parse_input(bad), None, "{bad} must be refused");
        }
        // The valid neighbours still navigate.
        assert_eq!(
            parse_input("[::1]:8080"),
            Some(Target::Url("http://[::1]:8080".to_string()))
        );
        assert_eq!(
            parse_input("[::1]/path"),
            Some(Target::Url("http://[::1]/path".to_string()))
        );
    }

    #[test]
    fn invisible_format_characters_are_refused_in_typed_input() {
        // AUDIT 0.31 (A6-F8): zero-width and tag characters
        // in typed input are refused by clean_input, and a
        // host carrying one has no web origin.
        assert_eq!(parse_input("https://example.com\u{200B}/"), None);
        assert_eq!(parse_input("https://exa\u{FEFF}mple.com/"), None);
        assert_eq!(parse_input("https://exa\u{E0041}mple.com/"), None);
        assert_eq!(web_origin("https://exa\u{E0041}mple.com/"), None);
        assert_eq!(web_origin("https://example.com\u{200B}/"), None);
        // The same addresses without the invisible guest
        // are unaffected.
        assert_eq!(
            parse_input("https://example.com/"),
            Some(Target::Url("https://example.com/".to_string()))
        );
    }
}
