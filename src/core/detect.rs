//! Localhost service detection — the platform-independent core.
//!
//! Platform enumeration (which sockets are listening, owned by which
//! PID) lives in the app layer: `GetExtendedTcpTable` for AF_INET and
//! AF_INET6 on Windows. This module normalizes those raw listeners
//! into deduplicated probe targets, probes each target with a short
//! HTTP request on the *caller's* thread (never the UI thread),
//! extracts `<title>`, and produces the card data for the start page.
//!
//! v13 changes vs v12.1 (see AUDIT.md):
//! - **M7:** probe-cache entries carry timestamps. `NotHttp` expires
//!   after [`NOT_HTTP_TTL`], `Http` titles refresh after
//!   [`HTTP_TITLE_TTL`] — a service that was still booting (or whose
//!   title changed after boot) no longer stays wrong forever, while
//!   in-TTL services are still never refetched.
//! - **M8:** when the plaintext probe fails, a TLS ClientHello probe
//!   classifies TLS-only listeners (mkcert / dev-cert HTTPS dev
//!   servers) as HTTPS services. No certificate is fetched or
//!   trusted; Pico stores no title for them — clicking the card opens
//!   the page in WebView2, whose own TLS validation applies.
//! - **L9/L10:** numeric character references decode; `<title`
//!   matching requires a real tag boundary and skips HTML comments.
//!
//! v14 changes vs v13 (see AUDIT.md, V14-5 — "detector precision"):
//! v13 carded *every* listener that spoke HTTP or TLS, so the start
//! page filled with junk (Discord's RPC JSON API, an unidentified TLS
//! port, a 404 boilerplate page titled "Not Found"). v14 adds a
//! **qualification bar**: a listener becomes a card only with real
//! evidence it is a dev server — an HTML page title, a known
//! dev-server signature, a dev-runtime process speaking HTTP, or a
//! curated dev port serving HTML (TLS: dev process or dev port only).
//! Everything else is counted in one aggregate (`other_listeners`).
//! Titles are additionally cleaned here: control characters stripped,
//! capped at 80 chars, and boilerplate error titles ("Not Found",
//! "404 Not Found", "Internal Server Error", …) treated as *no
//! title* — an error page's furniture is not an app's name.
//!
//! v22 (AUDIT V22-3): `parse_http_response` no longer computes the
//! body offset by accumulating line lengths — the v21 arithmetic
//! drifted one byte short on CRLF responses (the status line was
//! measured after trimming its `\r`). The head/body split is now
//! found directly at the header terminator. Nothing else in this
//! module changed: qualification, probes, TTLs and signatures were
//! re-verified line by line and carried.

use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// Upper bound on simultaneous probes during one scan cycle.
pub const MAX_PARALLEL_PROBES: usize = 8;

/// How long a "does not speak HTTP(S)" result is trusted before the
/// listener is probed again (covers servers still booting).
pub const NOT_HTTP_TTL: Duration = Duration::from_secs(15);
/// How long a fetched page title is trusted before a refresh probe.
pub const HTTP_TITLE_TTL: Duration = Duration::from_secs(120);

/// One listening TCP socket as reported by the OS. `addr` is the bind
/// address in textual form (`"127.0.0.1"`, `"0.0.0.0"`, `"::1"`, `"::"`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub addr: String,
    pub port: u16,
    pub pid: u32,
}

/// Whether a detected service speaks plaintext HTTP or TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceKind {
    Http,
    Https,
}

/// A detected local service, ready to render as a start-page card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Service {
    pub port: u16,
    pub pid: Option<u32>,
    pub process: Option<String>,
    pub title: Option<String>,
    /// Display label: the page title, or a guess based on the port.
    pub label: String,
    pub url: String,
    /// http or https — drives the card's scheme tag.
    pub kind: ServiceKind,
}

/// Which local IPs are worth probing for a listener bound to `addr`.
///
/// - IPv4 loopback bind → that exact address (127.0.0.0/8 is all local).
/// - IPv4 wildcard `0.0.0.0` → 127.0.0.1.
/// - IPv6 loopback `::1` → ::1.
/// - IPv6 wildcard `::` → both loopbacks (dual-stack sockets accept
///   IPv4 too, but a v6-only socket does not, so try each).
/// - Anything else (LAN addresses, link-local) → not probed; the brief
///   covers loopback + unspecified listeners only.
pub fn listener_probe_ips(addr: &str) -> Vec<IpAddr> {
    match addr.trim().trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            if v4.is_loopback() {
                vec![IpAddr::V4(v4)]
            } else if v4.is_unspecified() {
                vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
            } else {
                Vec::new()
            }
        }
        Ok(IpAddr::V6(v6)) => {
            if v6.is_loopback() {
                vec![IpAddr::V6(v6)]
            } else if v6.is_unspecified() {
                vec![
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(Ipv6Addr::LOCALHOST),
                ]
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    }
}

/// A deduplicated thing to probe: one `(port, pid)` plus every local
/// IP it might answer on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    pub port: u16,
    pub pid: u32,
    pub ips: Vec<IpAddr>,
}

/// Normalize raw OS listeners into probe targets: skip our own PID,
/// skip non-local binds, merge rows that share `(port, pid)` across
/// address families (a dual-stack service appears as several rows).
pub fn normalize_targets(listeners: &[Listener], our_pid: u32) -> Vec<ProbeTarget> {
    let mut out: Vec<ProbeTarget> = Vec::new();
    for l in listeners {
        if l.pid == our_pid || l.port == 0 {
            continue;
        }
        let ips = listener_probe_ips(&l.addr);
        if ips.is_empty() {
            continue;
        }
        match out.iter_mut().find(|t| t.port == l.port && t.pid == l.pid) {
            Some(t) => {
                for ip in ips {
                    if !t.ips.contains(&ip) {
                        t.ips.push(ip);
                    }
                }
            }
            None => out.push(ProbeTarget {
                port: l.port,
                pid: l.pid,
                ips,
            }),
        }
    }
    out.sort_by_key(|t| t.port);
    out
}

/// Find `needle` in `haystack`, comparing ASCII case-insensitively.
/// Byte-based: offsets are always exact, even when the document
/// contains characters whose Unicode lowercase form has a different
/// byte length (the failure mode of the RS-era `to_lowercase`
/// implementation — it could slice at a non-char-boundary and panic
/// on a hostile page title).
fn find_ascii_ci_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() || from > haystack.len() - needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| {
        haystack[i..i + needle.len()]
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// Extract and clean the `<title>` of an HTML document.
///
/// The match must be a real tag: `<title` followed by whitespace,
/// `>`, or `/` (so `<titlex>` does not count), and candidates inside
/// `<!-- -->` comments are skipped. Byte-based throughout.
pub fn parse_title(html: &str) -> Option<String> {
    let bytes = html.as_bytes();
    let mut search_from = 0usize;
    let start = loop {
        let cand = find_ascii_ci_from(bytes, b"<title", search_from)?;
        // Skip a candidate that sits inside an HTML comment.
        if let Some(cstart) = rfind_ascii_ci(&bytes[..cand], b"<!--") {
            let closed = find_ascii_ci_from(bytes, b"-->", cstart).map(|e| e + 3 <= cand);
            if closed != Some(true) {
                // Unterminated or still-open comment: keep looking
                // after this candidate (a real title may follow).
                search_from = cand + 1;
                continue;
            }
        }
        match bytes.get(cand + 6) {
            Some(b) if b.is_ascii_whitespace() || *b == b'>' || *b == b'/' => break cand,
            _ => {
                search_from = cand + 1;
                continue;
            }
        }
    };
    let gt = find_ascii_ci_from(bytes, b">", start)? + 1;
    let end = find_ascii_ci_from(bytes, b"</title", gt)?;
    // `gt`/`end` sit on ASCII delimiters, so these are char boundaries.
    let raw = &html[gt..end];
    let cleaned = decode_entities(raw);
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed.chars().take(512).collect())
    }
}

/// Find the last occurrence of `needle` before the given end offset.
fn rfind_ascii_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).rev().find(|&i| {
        haystack[i..i + needle.len()]
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// Decode HTML entities: the common named set plus numeric character
/// references (`&#8212;`, `&#x2014;`). Invalid or out-of-range numeric
/// references are dropped (not echoed back as raw markup).
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            let rest = &s[i..];
            // Named references (longest first among the supported set).
            let named: &[(&str, &str)] = &[
                ("&quot;", "\""),
                ("&apos;", "'"),
                ("&nbsp;", " "),
                ("&amp;", "&"),
                ("&lt;", "<"),
                ("&gt;", ">"),
                ("&#39;", "'"),
            ];
            if let Some((src, dst)) = named.iter().find(|(src, _)| rest.starts_with(src)) {
                out.push_str(dst);
                i += src.len();
                continue;
            }
            // Numeric character reference: &#123; or &#x1F600;
            if let Some(semi) = rest.find(';') {
                if semi <= 10 && rest.starts_with("&#") {
                    let body = &rest[2..semi];
                    let parsed = if let Some(hex) =
                        body.strip_prefix('x').or_else(|| body.strip_prefix('X'))
                    {
                        u32::from_str_radix(hex, 16).ok()
                    } else if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) {
                        body.parse::<u32>().ok()
                    } else {
                        None
                    };
                    if let Some(n) = parsed {
                        if let Some(c) = char::from_u32(n) {
                            out.push(c);
                        }
                        i += semi + 1;
                        continue;
                    }
                }
            }
            out.push('&');
            i += 1;
        } else {
            // Copy the whole UTF-8 sequence for this char.
            let ch_len = utf8_len(bytes[i]);
            let end = (i + ch_len).min(bytes.len());
            out.push_str(&s[i..end]);
            i = end;
        }
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Maximum length of a displayed page title.
pub const MAX_TITLE_CHARS: usize = 80;

/// Boilerplate titles that carry no information about the app: the
/// standard reason phrases frameworks stamp onto error pages. A page
/// whose `<title>` is one of these is treated as *untitled* (the
/// v13 screenshot's "Not Found" card is exactly this furniture).
const GENERIC_TITLES: &[&str] = &[
    "ok",
    "error",
    "not found",
    "bad request",
    "unauthorized",
    "forbidden",
    "internal server error",
    "not implemented",
    "bad gateway",
    "service unavailable",
    "gateway timeout",
    "moved permanently",
    "moved temporarily",
    "found",
    "see other",
    "request timeout",
    "method not allowed",
    "page not found",
    "404 page",
    "error 404",
];

/// Clean a raw extracted `<title>` for display and qualification:
/// strip control characters, cap at [`MAX_TITLE_CHARS`], and reject
/// boilerplate error titles (with or without a leading status code,
/// e.g. "404 Not Found" / "404 - Not Found"). Returns `None` when
/// nothing worth showing remains.
pub fn clean_title(raw: &str) -> Option<String> {
    let no_ctrl: String = raw.chars().filter(|c| !c.is_control()).collect();
    let collapsed = no_ctrl.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let lower = collapsed.to_lowercase();
    // Strip a leading status code ("404 ", "404 - ", "404: ") so the
    // bare-phrase comparison below sees the furniture itself.
    let phrase = match lower.as_bytes().get(0..3) {
        Some([a, b, c]) if a.is_ascii_digit() && b.is_ascii_digit() && c.is_ascii_digit() => {
            lower[3..].trim_start_matches([' ', '-', ':', '|']).trim()
        }
        _ => lower.trim(),
    };
    if phrase.len() <= 3 && phrase.bytes().all(|b| b.is_ascii_digit()) {
        return None; // the title is just a status code
    }
    if GENERIC_TITLES.contains(&phrase) {
        return None;
    }
    Some(collapsed.chars().take(MAX_TITLE_CHARS).collect())
}

/// Normalize an OS process image name for comparison: lowercase,
/// `.exe` suffix removed ("Node.EXE" → "node").
pub fn normalize_process_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// Process names that mark a listener as development tooling
/// (qualification rule (c)): JS/other runtimes, language servers'
/// hosts, static-site generators, live-reload proxies.
pub fn is_dev_process(process: Option<&str>) -> bool {
    let Some(name) = process else { return false };
    matches!(
        normalize_process_name(name).as_str(),
        "node"
            | "python"
            | "python3"
            | "pythonw"
            | "deno"
            | "bun"
            | "php"
            | "ruby"
            | "dotnet"
            | "java"
            | "hugo"
            | "air"
            | "cargo"
    )
}

/// A friendly card label derived from a dev process name.
pub fn process_label(process: &str) -> String {
    match normalize_process_name(process).as_str() {
        "node" => "Node.js app".to_string(),
        "python" | "python3" | "pythonw" => "Python app".to_string(),
        "deno" => "Deno app".to_string(),
        "bun" => "Bun app".to_string(),
        "php" => "PHP app".to_string(),
        "ruby" => "Ruby app".to_string(),
        "dotnet" => ".NET app".to_string(),
        "java" => "Java app".to_string(),
        "hugo" => "Hugo site".to_string(),
        "air" => "Go app (air)".to_string(),
        "cargo" => "Rust app".to_string(),
        _ => "Dev server".to_string(),
    }
}

/// Ports curated as development-server ports (qualification rule
/// (d)): the defaults of the common dev servers and live-reload
/// setups, plus the Chrome DevTools port.
pub fn is_dev_port(port: u16) -> bool {
    matches!(
        port,
        1234 | 1313
            | 3000
            | 3001
            | 4000
            | 4173
            | 4200
            | 4321
            | 5000
            | 5173
            | 5174
            | 5500
            | 8000
            | 8080
            | 8888
            | 9000
            | 9229
    )
}

/// Recognize a known dev server from its response `Server` header
/// and/or body markers. Returns the display name for the card label.
pub fn match_dev_signature(server: Option<&str>, body: &str) -> Option<&'static str> {
    let server = server.unwrap_or("");
    if server.starts_with("Werkzeug") {
        return Some("Flask / Werkzeug");
    }
    if server.starts_with("PHP") {
        return Some("PHP built-in server");
    }
    if server.contains("SimpleHTTP") {
        return Some("Python http.server");
    }
    if server.contains("WSGIServer") {
        return Some("Django dev server");
    }
    let markers: &[(&str, &str)] = &[
        ("/@vite/client", "Vite dev server"),
        ("__NEXT_DATA__", "Next.js dev server"),
        ("Code injected by live-server", "Live Server"),
        ("__sveltekit", "SvelteKit dev server"),
        ("name=\"generator\" content=\"Hugo", "Hugo dev server"),
        ("csrfmiddlewaretoken", "Django dev server"),
        ("Directory listing for", "Python http.server"),
        ("webpack-dev-server", "webpack dev server"),
        ("sockjs-node", "webpack dev server"),
        ("astro-island", "Astro dev server"),
        ("ng-version", "Angular dev server"),
    ];
    markers
        .iter()
        .find(|(marker, _)| body.contains(marker))
        .map(|(_, name)| *name)
}

/// Guess what a port is running when the page gives no title.
pub fn guess_service(port: u16, title: Option<&str>) -> String {
    if let Some(t) = title {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    match port {
        5173 | 5174 => "Vite dev server".to_string(),
        4200 => "Angular dev server".to_string(),
        1313 => "Hugo dev server".to_string(),
        1234 => "Parcel dev server".to_string(),
        8888 => "Jupyter".to_string(),
        3000 | 3001 | 4000 | 5000 | 8000 | 8080 | 9000 => "Web app / dev server".to_string(),
        80 => "Web server".to_string(),
        _ => "HTTP service".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Probe cache (TTL — AUDIT M7)
// ---------------------------------------------------------------------------

/// Cached outcome of probing one `(port, pid)`, with the moment it was
/// learned. Entries expire by kind (see the TTL constants): a cached
/// outcome is *evidence with a shelf life*, not a permanent verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CachedProbe {
    /// The port spoke neither HTTP nor TLS when last probed.
    NotHttp { at: Instant },
    /// It spoke HTTP; the facts qualification needs (cleaned title,
    /// dev-server signature, whether the response was HTML) and the
    /// IP that answered.
    Http {
        title: Option<String>,
        signature: Option<&'static str>,
        html: bool,
        ip: IpAddr,
        at: Instant,
    },
    /// It answered a TLS ClientHello with a TLS handshake record.
    Https { ip: IpAddr, at: Instant },
}

impl CachedProbe {
    fn fresh(&self, now: Instant) -> bool {
        match self {
            CachedProbe::NotHttp { at } => now.duration_since(*at) < NOT_HTTP_TTL,
            CachedProbe::Http { at, .. } | CachedProbe::Https { at, .. } => {
                now.duration_since(*at) < HTTP_TITLE_TTL
            }
        }
    }
}

/// Probe-result cache keyed by `(port, pid)`. Fresh entries are reused
/// verbatim — no refetching page titles every cycle. Stale entries are
/// re-probed lazily by [`scan_services`]. Entries for listeners that
/// disappeared are evicted at the start of each scan.
#[derive(Debug, Default)]
pub struct TitleCache {
    map: HashMap<(u16, u32), CachedProbe>,
}

impl TitleCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn retain_targets(&mut self, targets: &[ProbeTarget]) {
        self.map
            .retain(|(port, pid), _| targets.iter().any(|t| t.port == *port && t.pid == *pid));
    }
}

// ---------------------------------------------------------------------------
// Probes
// ---------------------------------------------------------------------------

/// The facts one HTTP probe learned about a listener — everything
/// the qualification bar (see module docs) is allowed to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// HTTP status code from the status line.
    pub status: u16,
    /// Cleaned page title — only ever from a real `<title>` element
    /// in an HTML response; boilerplate error titles already removed.
    pub title: Option<String>,
    /// Known dev-server signature matched on headers/body, if any.
    pub signature: Option<&'static str>,
    /// Whether the response presented as HTML (Content-Type or a
    /// body that opens with an HTML doctype/element).
    pub html: bool,
}

/// Parse a raw HTTP response (status line + headers + body) into
/// probe facts. Pure: the fixture tests drive this directly, and the
/// socket probe below is a thin shell around it. Returns `None` when
/// the bytes are not a syntactically valid HTTP response — titles
/// NEVER come from the status line or reason phrase; the only title
/// source is a `<title>` element in an HTML body.
pub fn parse_http_response(text: &str) -> Option<ProbeResult> {
    // Split head from body at the header terminator, exactly.
    // (AUDIT V22-3: the v21 code accumulated per-line lengths to
    // find this offset and drifted one byte short on CRLF
    // responses, because the status line was measured after its
    // `\r` was trimmed. Detection tolerated the stray byte; the
    // arithmetic is gone anyway.)
    let (head, body) = match text.find("\r\n\r\n") {
        Some(i) => (&text[..i], &text[i + 4..]),
        None => match text.find("\n\n") {
            Some(i) => (&text[..i], &text[i + 2..]),
            None => (text, ""),
        },
    };
    let mut lines = head.split('\n');
    let status_line = lines.next()?.trim_end_matches('\r');
    let mut parts = status_line.split_whitespace();
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    let status: u16 = parts.next()?.parse().ok()?;
    if !(100..=599).contains(&status) {
        return None;
    }
    let mut content_type: Option<String> = None;
    let mut server: Option<String> = None;
    for line in lines {
        let clean = line.trim_end_matches('\r');
        if let Some((name, value)) = clean.split_once(':') {
            let value = value.trim().to_string();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-type" => content_type = Some(value),
                "server" => server = Some(value),
                _ => {}
            }
        }
    }
    let html = match &content_type {
        Some(ct) => ct.to_ascii_lowercase().starts_with("text/html"),
        None => false,
    } || body_opens_as_html(body);
    let signature = match_dev_signature(server.as_deref(), body);
    let title = if html {
        parse_title(body).and_then(|t| clean_title(&t))
    } else {
        None
    };
    Some(ProbeResult {
        status,
        title,
        signature,
        html,
    })
}

/// Whether a body opens like an HTML document (leading whitespace,
/// then a doctype or an `<html` element).
fn body_opens_as_html(body: &str) -> bool {
    let head = body.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let head: String = head.chars().take(16).collect::<String>().to_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

/// Probe `ip:port` with a minimal HTTP/1.1 GET. Returns `None` when
/// the address does not speak HTTP (connect refused, timeout, or the
/// reply is not a valid HTTP response).
pub fn probe_http(ip: IpAddr, port: u16, timeout: Duration) -> Option<ProbeResult> {
    let sa = SocketAddr::new(ip, port);
    let mut stream = TcpStream::connect_timeout(&sa, timeout).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: Pico\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    // No redirect following: a manual request either speaks HTTP here
    // or it doesn't. The read is capped at 64 KiB *and* by a total
    // deadline, so a service that trickles one byte per read-timeout
    // cannot hold the probe (and its thread) open indefinitely.
    let deadline = Instant::now() + timeout.max(Duration::from_millis(50)) * 3;
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 4096];
    loop {
        if Instant::now() >= deadline {
            break;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() >= 64 * 1024 {
                    buf.truncate(64 * 1024);
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if buf.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&buf);
    parse_http_response(&text)
}

/// A minimal, byte-exact TLS ClientHello (TLS 1.2, one cipher suite,
/// no extensions, no SNI — this is a *classifier*, not a TLS stack: it
/// asks "do you speak TLS?" and nothing more). The "random" is zeros;
/// a classifier hello needs no entropy — servers answer the handshake
/// message type regardless of its contents.
///
/// Layout (50 bytes total): record header `16 03 01 00 2D`, handshake
/// header `01 00 00 29`, then the 41-byte body: client version
/// `03 03`, 32 random bytes, session-id length `00`, cipher-suites
/// length `00 02` + suite `00 2F`, compression length `01` + `00`.
/// Every declared length equals the bytes actually present.
fn client_hello() -> [u8; 50] {
    let mut h = [0u8; 50];
    h[0] = 0x16; // content type: handshake
    h[1] = 0x03;
    h[2] = 0x01; // record version (TLS 1.0 compat)
    h[3] = 0x00;
    h[4] = 0x2D; // record length: 45
    h[5] = 0x01; // handshake type: ClientHello
    h[6] = 0x00;
    h[7] = 0x00;
    h[8] = 0x29; // handshake body length: 41
    h[9] = 0x03;
    h[10] = 0x03; // client version: TLS 1.2
                  // bytes 11..43: random (zeros)
    h[43] = 0x00; // session id length: 0
    h[44] = 0x00;
    h[45] = 0x02; // cipher suites length: 2
    h[46] = 0x00;
    h[47] = 0x2F; // TLS_RSA_WITH_AES_128_CBC_SHA
    h[48] = 0x01; // compression methods length: 1
    h[49] = 0x00; // null compression
    h
}

/// Classify `ip:port` as TLS-speaking: send a ClientHello and check
/// the reply begins with a TLS record — content type 0x16 (handshake,
/// i.e. a ServerHello) or 0x15 (alert: the server understood TLS well
/// enough to refuse this minimal hello, which is still proof of
/// protocol — e.g. TLS-1.3-only servers rejecting a 1.2 hello).
/// A plaintext HTTP server answers with `HTTP…` bytes instead and is
/// rejected; anything else (silence, garbage, reset) is rejected too.
/// This never validates a certificate — it only recognizes the
/// protocol, so it cannot weaken TLS validation anywhere.
pub fn probe_tls(ip: IpAddr, port: u16, timeout: Duration) -> bool {
    let sa = SocketAddr::new(ip, port);
    let Ok(mut stream) = TcpStream::connect_timeout(&sa, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if stream.write_all(&client_hello()).is_err() {
        return false;
    }
    let mut first = [0u8; 1];
    match stream.read_exact(&mut first) {
        Ok(()) => first[0] == 0x16 || first[0] == 0x15,
        Err(_) => false,
    }
}

fn probe_target(target: &ProbeTarget, timeout: Duration) -> CachedProbe {
    let now = Instant::now();
    for ip in &target.ips {
        if let Some(res) = probe_http(*ip, target.port, timeout) {
            return CachedProbe::Http {
                title: res.title,
                signature: res.signature,
                html: res.html,
                ip: *ip,
                at: now,
            };
        }
    }
    // Plaintext failed on every IP: maybe it only speaks TLS.
    for ip in &target.ips {
        if probe_tls(*ip, target.port, timeout) {
            return CachedProbe::Https { ip: *ip, at: now };
        }
    }
    CachedProbe::NotHttp { at: now }
}

fn service_url(ip: IpAddr, port: u16, kind: ServiceKind) -> String {
    let scheme = match kind {
        ServiceKind::Http => "http",
        ServiceKind::Https => "https",
    };
    match ip {
        IpAddr::V4(_) => format!("{scheme}://localhost:{port}"),
        IpAddr::V6(v6) => format!("{scheme}://[{v6}]:{port}"),
    }
}

/// The outcome of one scan: the listeners that earned a start-page
/// card, and how many other local listeners were seen but did not
/// qualify (rendered as one aggregate line, never as cards).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    pub cards: Vec<Service>,
    pub other_listeners: usize,
}

/// Decide whether one probed listener earns a card, and build it.
///
/// The qualification bar (module docs, V14-5) — HTTP qualifies via
/// (a) a real page title, (b) a dev-server signature, (c) a
/// dev-runtime process (any valid HTTP response is enough), or
/// (d) a curated dev port serving HTML. TLS qualifies via (c) or (d)
/// only and never carries a title. Everything else returns `None`
/// and is counted as an "other listener" by the caller.
fn qualify(target: &ProbeTarget, probe: &CachedProbe, process: Option<String>) -> Option<Service> {
    match probe {
        CachedProbe::Http {
            title,
            signature,
            html,
            ip,
            ..
        } => {
            let dev_proc = is_dev_process(process.as_deref());
            let qualified = title.is_some()
                || signature.is_some()
                || dev_proc
                || (is_dev_port(target.port) && *html);
            if !qualified {
                return None;
            }
            let label = if let Some(t) = title {
                t.clone()
            } else if let Some(sig) = signature {
                (*sig).to_string()
            } else if dev_proc {
                process_label(process.as_deref().unwrap_or(""))
            } else {
                guess_service(target.port, None)
            };
            Some(Service {
                port: target.port,
                pid: Some(target.pid),
                process,
                title: title.clone(),
                label,
                url: service_url(*ip, target.port, ServiceKind::Http),
                kind: ServiceKind::Http,
            })
        }
        CachedProbe::Https { ip, .. } => {
            let dev_proc = is_dev_process(process.as_deref());
            if !dev_proc && !is_dev_port(target.port) {
                return None;
            }
            Some(Service {
                port: target.port,
                pid: Some(target.pid),
                process,
                title: None,
                label: "HTTPS service".to_string(),
                url: service_url(*ip, target.port, ServiceKind::Https),
                kind: ServiceKind::Https,
            })
        }
        CachedProbe::NotHttp { .. } => None,
    }
}

/// Scan OS listeners: probe what needs probing (TTL cache, parallel
/// chunks), then apply the qualification bar to every known target.
///
/// Fresh cached `(port, pid)` results are reused without probing;
/// missing or stale targets are probed in parallel chunks of at most
/// [`MAX_PARALLEL_PROBES`], each probe bounded by `probe_timeout`.
pub fn scan_services(
    listeners: &[Listener],
    our_pid: u32,
    probe_timeout: Duration,
    cache: &mut TitleCache,
    name_of: &dyn Fn(u32) -> Option<String>,
) -> ScanOutcome {
    let targets = normalize_targets(listeners, our_pid);
    cache.retain_targets(&targets);
    let now = Instant::now();

    // Probe only what the cache doesn't know *freshly*, in chunks.
    let unknown: Vec<ProbeTarget> = targets
        .iter()
        .filter(|t| match cache.map.get(&(t.port, t.pid)) {
            Some(entry) => !entry.fresh(now),
            None => true,
        })
        .cloned()
        .collect();
    for chunk in unknown.chunks(MAX_PARALLEL_PROBES) {
        let probed: Vec<CachedProbe> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|t| scope.spawn(move || probe_target(t, probe_timeout)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or(CachedProbe::NotHttp { at: Instant::now() })
                })
                .collect()
        });
        for (target, outcome) in chunk.iter().zip(probed) {
            cache.map.insert((target.port, target.pid), outcome);
        }
    }

    let mut outcome = ScanOutcome::default();
    for t in &targets {
        let process = name_of(t.pid);
        match cache.map.get(&(t.port, t.pid)) {
            Some(probe) => match qualify(t, probe, process) {
                Some(card) => outcome.cards.push(card),
                None => outcome.other_listeners += 1,
            },
            None => outcome.other_listeners += 1,
        }
    }
    // Titled cards first (they carry the most signal), each group by
    // ascending port.
    outcome.cards.sort_by_key(|s| (s.title.is_none(), s.port));
    outcome
}

/// Stable signature of a scan outcome, for change-gated UI pushes.
/// Includes the URL (AUDIT I5: an answering-IP flip must re-push) and
/// the aggregate count, so a listener appearing/vanishing in the
/// "other" bucket still refreshes the start page's muted line.
pub fn signature(outcome: &ScanOutcome) -> String {
    let mut sig = String::new();
    for s in &outcome.cards {
        sig.push_str(&format!(
            "{}|{}|{}|{}|{};",
            s.port,
            s.pid.unwrap_or(0),
            s.title.as_deref().unwrap_or(""),
            s.process.as_deref().unwrap_or(""),
            s.url
        ));
    }
    sig.push_str(&format!("#other={}", outcome.other_listeners));
    sig
}

// ---------------------------------------------------------------------------
// Tests (in-module: they need the private cache types and backdated
// timestamps that integration tests cannot fabricate)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }

    #[test]
    fn not_http_entries_expire_after_short_ttl() {
        let now = Instant::now();
        let fresh = CachedProbe::NotHttp { at: now };
        assert!(fresh.fresh(now));
        let stale = CachedProbe::NotHttp {
            at: now - NOT_HTTP_TTL - Duration::from_secs(1),
        };
        assert!(!stale.fresh(now), "booting service must be re-probed");
    }

    #[test]
    fn http_entries_live_for_the_long_ttl() {
        let now = Instant::now();
        let mid = CachedProbe::Http {
            title: Some("App".into()),
            signature: None,
            html: true,
            ip: ip(),
            at: now - NOT_HTTP_TTL - Duration::from_secs(5),
        };
        assert!(mid.fresh(now), "Http outlives the NotHttp TTL");
        let stale = CachedProbe::Http {
            title: None,
            signature: None,
            html: false,
            ip: ip(),
            at: now - HTTP_TITLE_TTL - Duration::from_secs(1),
        };
        assert!(!stale.fresh(now));
    }

    #[test]
    fn https_entries_share_the_long_ttl() {
        let now = Instant::now();
        let e = CachedProbe::Https {
            ip: ip(),
            at: now - HTTP_TITLE_TTL + Duration::from_secs(1),
        };
        assert!(e.fresh(now));
        let stale = CachedProbe::Https {
            ip: ip(),
            at: now - HTTP_TITLE_TTL - Duration::from_secs(1),
        };
        assert!(!stale.fresh(now));
    }

    #[test]
    fn cache_evicts_targets_that_disappeared() {
        let mut cache = TitleCache::new();
        cache.map.insert(
            (3000, 42),
            CachedProbe::Http {
                title: None,
                signature: None,
                html: false,
                ip: ip(),
                at: Instant::now(),
            },
        );
        cache
            .map
            .insert((3000, 43), CachedProbe::NotHttp { at: Instant::now() });
        let targets = vec![ProbeTarget {
            port: 3000,
            pid: 42,
            ips: vec![ip()],
        }];
        cache.retain_targets(&targets);
        assert_eq!(cache.len(), 1);
        assert!(cache.map.contains_key(&(3000, 42)));
    }
}
