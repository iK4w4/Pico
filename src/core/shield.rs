//! The Shield: Pico's ad blocker — network, cosmetic and
//! scriptlet layers, Adblock-Plus rule semantics.
//!
//! The user explicitly ordered the blocker back (2026-10-24),
//! amending the old reset-spec non-goal. This engine is built to
//! the opposite standard of the C#-era "Shield" whose *flattened*
//! domain list caused the v7 catastrophe (google.com blocked as a
//! main document): the real filter lists (EasyList, EasyPrivacy,
//! uBlock filters + privacy, see SOURCES.md) are fetched on
//! first run from their canonical sources and cached beside
//! the exe (`data/lists/`; acquisition policy in
//! [`super::lists`]), then parsed once with Adblock-Plus
//! semantics — anchors, wildcards, separators, exceptions,
//! `$important`, `$badfilter`, party/type/`domain=` options — and
//! every list line is accounted for in a [`CompileReport`]. Lines
//! the engine cannot honor exactly (regex patterns, procedural
//! and scriptlet cosmetics, `$csp`/`$redirect`/`$removeparam` and
//! other unsupported options) are *skipped and counted*, never
//! silently misapplied.
//!
//! Three safety laws are structural, and pinned by tests:
//! 1. A main-frame DOCUMENT is never blocked except by a
//!    domain-anchored rule that explicitly carries the `document`
//!    type (uBO's default no-strict-blocking posture).
//! 2. Localhost / loopback / Pico's internal origins are never
//!    blocked in either layer, whatever the lists say.
//! 3. The engine fails open: decisions come from pure data, the
//!    matcher contains no panicking operations, and any doubt
//!    resolves to Allow.
//!
//! Everything here is pure and host-testable; `app.rs` only wires
//! the compiled engine into WebView2's `WebResourceRequested`.
//!
//! v22 engine fixes (AUDIT V22-4..7 — the shipped lists contain no
//! page-flag rules, so none of these change behavior on the
//! shipped payload; they make the engine correct for the rules it
//! claims to parse):
//! - Anchored matching locates the request host by parsing the
//!   URL authority (`host_span`), not by `str::find` over the
//!   whole URL — a userinfo segment containing the host string
//!   could misalign the pattern remainder, and an IPv6 host's
//!   colons landed the remainder *inside* the host.
//! - `$genericblock` now suppresses generic BLOCK rules only;
//!   generic exceptions still apply (uBO semantics).
//! - The `elemhide` page flag is honored by the cosmetic layer
//!   (it was parsed, counted, and then ignored).
//! - A global (unscoped) `$generichide` now actually suppresses
//!   generic cosmetics everywhere (it never reached the host
//!   list the runtime consults).
//!
//! v23 adds the third layer the v22 audit named as the gap:
//! SCRIPTLETS. YouTube in-stream ads are not network resources —
//! they are `adPlacements` inside the player's own innertube
//! JSON, streamed from the same googlevideo hosts as content —
//! so uBlock Origin kills them with `##+js(...)` rules
//! (json-prune, trusted-replace-*, `set` traps on
//! `ytInitialPlayerResponse`, no-fetch-if / no-xhr-if), exactly
//! the family v21/v22 counted as skipped-unsupported. The
//! compiler now compiles the scriptlet families the shipped
//! lists actually use into a per-host payload; the payload is
//! DATA (name + raw argument tokens, JSON-encoded) interpreted
//! by ONE hand-written, audited JS runtime (`SCRIPTLET_RUNTIME`)
//! through a fixed dispatch table — list content never becomes
//! code. The same rewrite fixes three v22 defects the
//! line-by-line audit found: multi-domain cosmetic scopes were
//! filed under comma-joined pseudo-hosts that can never match
//! (111 of 938 payload keys were dead), `##^` html filters and
//! uBO-extended pseudo selectors (`:has-text(...)` and friends)
//! were shipped as CSS — where one invalid selector silently
//! kills the host's whole stylesheet under CSS error handling —
//! and the option parser rejected the `1p`/`3p`/`xhr`/`doc`/
//! `frame`/`css`/`ghide`/`from=`/`to=`/`method=`/`all`
//! spellings the real lists use (1,960 rules skipped, among
//! them YouTube's `*_ad_` media rule). All three Shield layers
//! — network, cosmetic, scriptlet — share the same laws:
//! loopback never touched, fail-open everywhere, and OFF is one
//! atomic flag (`window.__picoShieldOff` in-page).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// The 1×1 transparent GIF89a served in place of blocked images:
/// the standard neutered body (a real image, so pages lay out as
/// if the ad had been an empty pixel instead of erroring).
/// Document-script payload budgets (bytes), shared by the app,
/// the page-dump example and the tests so the three can never
/// drift apart again (v25 kept a copy in each).
///
/// AUDIT V26 (D6-b), measured on the canonical lists: the full
/// host-specific cosmetic set is 1,152 hosts / 2,128 rules and
/// serializes to 326,923 bytes of script — the v25 budget of
/// 300,000 silently cut the long tail (578 hosts shipped;
/// soft98.ir-class sites got generic cosmetics only). 340,000
/// covers every host in the lists with ~13 KB of headroom for
/// the budget accounting's approximation drift (V26-S3); the
/// coverage test in tests/shield_tests.rs pins hosts-shipped ==
/// hosts-compiled, so a future list refresh that outgrows the
/// budget fails loudly instead of silently dropping sites.
///
/// AUDIT V28: raised to 344,000. The v28 payload carries the
/// new generic-exception channel and a larger runtime, and the
/// accounting now counts the real runtime length; full
/// coverage of the v28 lists measures 341,567 bytes, so the
/// old ceiling cut the last 10 hosts. 344,000 restores the
/// every-host guarantee with ~2.4 KB of headroom.
pub const COSMETIC_SCRIPT_BUDGET: usize = 344_000;
/// Scriptlets already ship every host (2,197) at 186,717 bytes
/// under this budget on the canonical lists (measured, AUDIT V23);
/// unchanged in v26.
pub const SCRIPTLET_SCRIPT_BUDGET: usize = 220_000;

pub const TRANSPARENT_GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
];

// ---------------------------------------------------------------------------
// Resource types
// ---------------------------------------------------------------------------

/// The resource classes ABP type options name. Mapped from
/// WebView2's `COREWEBVIEW2_WEB_RESOURCE_CONTEXT` by
/// [`resource_type_from_context`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Document,
    Subdocument,
    Script,
    Image,
    Stylesheet,
    Xhr,
    Media,
    Font,
    WebSocket,
    Ping,
    Other,
}

impl ResourceType {
    fn bit(self) -> u32 {
        1u32 << (self as u32)
    }
    /// Short label for log lines (`shield block: … type=script`).
    pub fn label(self) -> &'static str {
        match self {
            ResourceType::Document => "document",
            ResourceType::Subdocument => "subdocument",
            ResourceType::Script => "script",
            ResourceType::Image => "image",
            ResourceType::Stylesheet => "stylesheet",
            ResourceType::Xhr => "xmlhttprequest",
            ResourceType::Media => "media",
            ResourceType::Font => "font",
            ResourceType::WebSocket => "websocket",
            ResourceType::Ping => "ping",
            ResourceType::Other => "other",
        }
    }
}

/// Map a WebView2 `COREWEBVIEW2_WEB_RESOURCE_CONTEXT` value to a
/// [`ResourceType`]. Frame documents arrive as DOCUMENT (1) for
/// both main frame and iframes; the app layer reclassifies with
/// its main-frame knowledge (see `app.rs::attach_shield`), so this
/// function maps 1 → Document and the caller downgrades to
/// Subdocument for non-main frames before deciding.
pub fn resource_type_from_context(code: i32) -> ResourceType {
    match code {
        1 => ResourceType::Document,
        2 => ResourceType::Stylesheet,
        3 => ResourceType::Image,
        4 => ResourceType::Media,
        5 => ResourceType::Font,
        6 => ResourceType::Script,
        7 | 8 => ResourceType::Xhr,
        11 => ResourceType::WebSocket,
        14 => ResourceType::Ping,
        _ => ResourceType::Other,
    }
}

/// The resource type a request is decided under: the context
/// mapping, with the main-frame reclassification — a Document
/// that is not the tab's main frame is a Subdocument (an
/// iframe), which is what keeps law 1 (documents are never
/// blocked) from being an iframe-shaped hole. Pure; the
/// request handler and the tests share it (AUDIT V28, A2-M2:
/// it lived inline in the handler and inside the deleted
/// `should_block` seam, in two copies).
pub fn effective_rtype(context_code: i32, main_frame: bool) -> ResourceType {
    let rtype = resource_type_from_context(context_code);
    if rtype == ResourceType::Document && !main_frame {
        ResourceType::Subdocument
    } else {
        rtype
    }
}

fn type_bit_for_name(name: &str) -> Option<u32> {
    let t = match name {
        // v23: the uBO/ABP alias spellings the real lists use
        // (`doc`, `frame`, `css`, `xhr`) map to the same bits.
        "document" | "doc" => ResourceType::Document,
        "subdocument" | "frame" => ResourceType::Subdocument,
        "script" => ResourceType::Script,
        "image" => ResourceType::Image,
        "stylesheet" | "css" => ResourceType::Stylesheet,
        "xmlhttprequest" | "xhr" => ResourceType::Xhr,
        "media" => ResourceType::Media,
        "font" => ResourceType::Font,
        "websocket" => ResourceType::WebSocket,
        "ping" | "beacon" => ResourceType::Ping,
        "other" => ResourceType::Other,
        _ => return None,
    };
    Some(t.bit())
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

/// Lowercased host of an http(s) URL (userinfo/port stripped,
/// IPv6 brackets removed). `None` for non-web or hostless URLs.
pub fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let end = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split(']').next().unwrap_or(inner)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    if host.is_empty() {
        return None;
    }
    // AUDIT V28 (A2-L6): a trailing-dot FQDN (`ads.example.com.`)
    // is the same host; without the trim, every suffix walk
    // misses its anchored rules. All host consumers share this
    // one extraction, so the trim lives here.
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

/// The byte span of the authority's host inside a (lowercased)
/// URL, plus the offset where the post-host remainder begins
/// (the first of `:`, `/`, `?`, `#` at or after the host's end —
/// the port colon belongs to the remainder, exactly as pattern
/// remainders expect). Parsed from the authority structure, so
/// userinfo and path content can never alias the host (AUDIT
/// V22-4). IPv6 literals are bracketed in the URL but bare in
/// `host_of`'s output; the bracket pair is part of the span.
pub fn host_span(url: &str, host: &str) -> Option<(usize, usize)> {
    let auth_start = match url.find("://") {
        Some(i) => i + 3,
        None => 0,
    };
    let rest = &url[auth_start..];
    let auth_len = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    let authority = &rest[..auth_len];
    let userinfo_len = authority.rfind('@').map(|i| i + 1).unwrap_or(0);
    let host_start = auth_start + userinfo_len;
    let host_field = &authority[userinfo_len..];
    let host_len = if host_field.starts_with('[') {
        // Bracketed IPv6: the field runs to the closing bracket.
        host_field.find(']').map(|i| i + 1)?
    } else {
        host.len()
    };
    if host_start + host_len > url.len() {
        return None;
    }
    let after = url[host_start + host_len..]
        .find(|c| ['/', '?', '#', ':'].contains(&c))
        .map(|i| host_start + host_len + i)
        .unwrap_or(url.len());
    Some((host_start, after))
}

/// Loopback / this-machine hosts: the Shield's second safety law.
///
/// AUDIT V26-S1 (D6-e): through v25 this named only 127.0.0.1,
/// while Lite's classifier (and Teleport's) treated the whole
/// 127.0.0.0/8 block as loopback — the two layers disagreed
/// about pages on 127.0.0.2 and friends. RFC 1122 reserves all
/// of 127/8 as loopback, so the widest correct definition wins
/// and is now the ONLY one: `filterpipe::classify_origin` reads
/// this function, Lite delegates to it, and the document
/// runtimes spell the same rule for JS.
/// Teleport delegates as well since 0.31.1.
pub fn is_local_host(host: &str) -> bool {
    // AUDIT V28 (A2-L1): 0.0.0.0 is this machine too (dev servers
    // print it as a URL host; Teleport already treated it as
    // loopback while the filter layers called it public and
    // Lite stripped a local dev page's images).
    if host == "localhost" || host == "::1" || host == "0.0.0.0" || host.ends_with(".localhost") {
        return true;
    }
    // IPv6 literals arrive bracket-free (host_of strips
    // them): ::1 in any spelling is loopback, and an
    // IPv4-mapped address inherits its embedded v4 verdict
    // (0.31.1 — the rule Teleport's private copy always had
    // and this definition lacked).
    if host.contains(':') {
        return match host.parse::<std::net::Ipv6Addr>() {
            Ok(v6) => {
                v6.is_loopback()
                    || v6
                        .to_ipv4_mapped()
                        .map(|v4| v4.octets()[0] == 127)
                        .unwrap_or(false)
            }
            Err(_) => false,
        };
    }
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(v4) => v4.octets()[0] == 127,
        Err(_) => match parse_whatwg_ipv4(host) {
            // AUDIT 0.31 (A2-F15): WHATWG IPv4 number forms
            // that the dotted-quad parser rejects — a browser
            // resolves 2130706433, 0x7f000001, 017700000001,
            // 127.1 and 127.0.1 all to 127.0.0.1, so the filter
            // layers must classify them loopback too, or a page
            // spelled that way escaped the local-sacred law.
            Some(addr) => addr >> 24 == 127,
            None => false,
        },
    }
}

/// Parse a WHATWG IPv4 number-form host into its 32-bit
/// address (AUDIT 0.31, A2-F15). 1–4 dot-separated parts; each
/// part is hex (`0x` prefix), octal (`0` prefix), or decimal;
/// every part but the last is one byte (≤ 255) and the last
/// part spans the remaining bytes. `None` for anything that is
/// not such an address — the caller falls through to name
/// handling, exactly as for an ordinary hostname.
fn parse_whatwg_ipv4(host: &str) -> Option<u32> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut nums: Vec<u64> = Vec::with_capacity(parts.len());
    for part in &parts {
        if part.is_empty() {
            return None;
        }
        let (digits, radix) =
            if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                if hex.is_empty() {
                    return None;
                }
                (hex, 16)
            } else if part.len() > 1 && part.starts_with('0') {
                (&part[1..], 8)
            } else {
                (*part, 10)
            };
        // u64::from_str_radix rejects empty strings, digits
        // outside the radix (an octal part carrying 8/9 is not
        // an address part), signs and overflow.
        nums.push(u64::from_str_radix(digits, radix).ok()?);
    }
    let last = nums.len() - 1;
    for (i, n) in nums.iter().enumerate() {
        if i < last {
            if *n > 255 {
                return None;
            }
        } else if *n >= (1u64 << (8 * (4 - last))) {
            // The last part must fit the bytes it spans:
            // 256^(5 - parts) values, i.e. < 2^(8*(4-last)).
            return None;
        }
    }
    let mut addr: u32 = nums[last] as u32;
    for (i, n) in nums.iter().enumerate().take(last) {
        addr += (*n as u32) << (8 * (3 - i));
    }
    Some(addr)
}

/// Is this URL one of Pico's own (the internal `pico:` scheme or
/// the loopback static-server origin)?
fn is_internal_url(url: &str) -> bool {
    if url.starts_with("pico:") || url.starts_with("pico://") {
        return true;
    }
    match host_of(url) {
        Some(h) => is_local_host(&h),
        None => false,
    }
}

/// Two-level public suffixes common enough to matter for party
/// computation (a pragmatic eTLD table, not the full PSL — the
/// approximation is documented in AUDIT V21: a miss can only
/// misjudge *party*, nudging a rule toward or away from matching;
/// it can never block a document by itself).
const TWO_LEVEL_SUFFIXES: &[&str] = &[
    "co.uk",
    "org.uk",
    "ac.uk",
    "gov.uk",
    "com.au",
    "net.au",
    "org.au",
    "co.jp",
    "ne.jp",
    "or.jp",
    "com.br",
    "com.cn",
    "com.tw",
    "com.hk",
    "co.in",
    "com.in",
    "co.kr",
    "com.mx",
    "com.ar",
    "com.co",
    "com.pe",
    "com.ve",
    "com.sg",
    "com.my",
    "com.ph",
    "com.pk",
    "com.bd",
    "com.ng",
    "com.eg",
    "com.sa",
    "com.ae",
    "com.tr",
    "com.pl",
    "com.ru",
    "com.ua",
    "co.il",
    "com.vn",
    "com.th",
    "co.id",
    "com.np",
    "com.kh",
    "com.mm",
    "co.ke",
    "com.gh",
    "com.et",
    "com.tn",
    "com.dz",
    "com.ma",
    "com.ly",
    "co.za",
    "com.na",
    "com.uy",
    "com.py",
    "com.bo",
    "com.ec",
    "com.gt",
    "com.cu",
    "com.do",
    "com.sv",
    "com.hn",
    "com.ni",
    "com.pa",
    "com.cl",
    // AUDIT V28 (A2-L5): co.nz was missing (two *.co.nz sites
    // computed as one party); github.io is a private suffix —
    // every *.github.io site is a different party.
    "co.nz",
    "github.io",
];

/// Registrable domain (eTLD+1) under the pragmatic suffix table:
/// the last two labels, or the last three when the last two form
/// a known two-level suffix. IP literals are their own domain.
pub fn registrable_domain(host: &str) -> String {
    let host = host.trim_end_matches('.');
    if host.parse::<std::net::IpAddr>().is_ok() || !host.contains('.') {
        return host.to_string();
    }
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    let take = if n >= 3
        && TWO_LEVEL_SUFFIXES.contains(&format!("{}.{}", labels[n - 2], labels[n - 1]).as_str())
    {
        3
    } else {
        2
    };
    labels[n - take.min(n)..].join(".")
}

/// URL equality for main-frame determination: case-insensitive,
/// and trailing slashes past the authority are ignored
/// (`https://example.com` ≡ `https://example.com/`). Used by
/// app.rs to recognize a main-frame document request.
pub fn urls_equivalent(a: &str, b: &str) -> bool {
    fn norm(u: &str) -> String {
        let s = u.trim().to_ascii_lowercase();
        // AUDIT V28 (A2-L8): fragments never distinguish a
        // document, and a spelled-out default port is the same
        // origin — both previously defeated the main-frame
        // equivalence and demoted the document to Subdocument,
        // outside law 1's protection.
        let s = s.split('#').next().unwrap_or("").to_string();
        match s.find("://") {
            Some(i) => {
                let (head, tail) = s.split_at(i + 3);
                let tail = tail.trim_end_matches('/');
                // Only an authority colon counts (before the
                // first '/'); a path ending in ":80" is a path.
                let auth_end = tail.find('/').unwrap_or(tail.len());
                let is_default_port = match (head, tail[..auth_end].rfind(':')) {
                    ("http://", Some(c)) => &tail[c..auth_end] == ":80",
                    ("https://", Some(c)) => &tail[c..auth_end] == ":443",
                    _ => false,
                };
                if is_default_port {
                    let c = tail[..auth_end].rfind(':').unwrap_or(auth_end);
                    format!("{head}{}{}", &tail[..c], &tail[auth_end..])
                } else {
                    format!("{head}{tail}")
                }
            }
            None => s.trim_end_matches('/').to_string(),
        }
    }
    norm(a) == norm(b)
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

/// One piece of a pattern segment: literal bytes, or the `^`
/// separator (any one character that is not a letter, digit, or
/// one of `_-.%` — or the end of the URL).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Lit(String),
    Sep,
}

/// A compiled ABP pattern body (the part after `||` handling and
/// option stripping): segments split by `*`, each a sequence of
/// literal/separator pieces, with optional `|` edge anchors.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    anchor_start: bool,
    anchor_end: bool,
    segments: Vec<Vec<Piece>>,
}

fn is_separator_byte(b: u8) -> bool {
    !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'%'))
}

/// Try to match one segment's pieces starting exactly at `pos`;
/// returns the end position on success.
fn segment_match_at(pieces: &[Piece], url: &[u8], pos: usize) -> Option<usize> {
    let mut p = pos;
    for piece in pieces {
        match piece {
            Piece::Lit(lit) => {
                let lb = lit.as_bytes();
                if p + lb.len() > url.len() || &url[p..p + lb.len()] != lb {
                    return None;
                }
                p += lb.len();
            }
            Piece::Sep => {
                if p == url.len() {
                    // Separator may match end-of-string.
                } else if is_separator_byte(url[p]) {
                    p += 1;
                } else {
                    return None;
                }
            }
        }
    }
    Some(p)
}

/// Find a segment match at or after `from`; returns (start, end).
fn segment_find(pieces: &[Piece], url: &[u8], from: usize) -> Option<(usize, usize)> {
    if pieces.is_empty() {
        return Some((from, from));
    }
    // A segment beginning with literals can only start where its
    // first literal piece could begin after any leading separators
    // (each leading Sep consumes at most one byte, or zero at the
    // end of the string).
    let mut leading_seps = 0usize;
    for piece in pieces {
        if *piece == Piece::Sep {
            leading_seps += 1;
        } else {
            break;
        }
    }
    let mut start = from;
    loop {
        if start > url.len() {
            return None;
        }
        if let Some(end) = segment_match_at(pieces, url, start) {
            return Some((start, end));
        }
        if start == url.len() {
            return None;
        }
        start += 1;
        // Leading separators bound how far the *pieces* can slide:
        // the search itself is over start positions, and
        // segment_match_at handles separator consumption, so no
        // extra pruning is needed beyond the string end.
        let _ = leading_seps;
    }
}

impl Pattern {
    fn compile(body: &str) -> Pattern {
        let mut b = body;
        let mut anchor_start = false;
        let mut anchor_end = false;
        if let Some(rest) = b.strip_prefix('|') {
            anchor_start = true;
            b = rest;
        }
        if b.len() > 1 && b.ends_with('|') {
            anchor_end = true;
            b = &b[..b.len() - 1];
        }
        let mut segments = Vec::new();
        for seg in b.split('*') {
            let mut pieces = Vec::new();
            let mut lit = String::new();
            for ch in seg.chars() {
                if ch == '^' {
                    if !lit.is_empty() {
                        pieces.push(Piece::Lit(std::mem::take(&mut lit)));
                    }
                    pieces.push(Piece::Sep);
                } else {
                    lit.push(ch);
                }
            }
            if !lit.is_empty() {
                pieces.push(Piece::Lit(lit));
            }
            segments.push(pieces);
        }
        Pattern {
            anchor_start,
            anchor_end,
            segments,
        }
    }

    /// Match against `url` (already lowercased). When
    /// `exact_start` is set, the first segment must begin exactly
    /// at `start` (domain-anchored rules: the remainder begins
    /// right after the matched host); otherwise it may begin at
    /// or after `start`.
    fn matches_from(&self, url: &str, start: usize, exact_start: bool) -> bool {
        let bytes = url.as_bytes();
        if bytes.len() < start {
            return false;
        }
        let mut pos = start;
        let n = self.segments.len();
        for (i, seg) in self.segments.iter().enumerate() {
            let last = i + 1 == n;
            if seg.is_empty() {
                continue;
            }
            if i == 0 && (self.anchor_start || exact_start) {
                let begin = if self.anchor_start { 0 } else { start };
                match segment_match_at(seg, bytes, begin) {
                    Some(end) => pos = end,
                    None => return false,
                }
            } else if last && self.anchor_end {
                // The final segment must end exactly at the end.
                // Try every start position from pos; accept one
                // whose match ends at the string end.
                let mut s = pos;
                let mut ok = false;
                loop {
                    if s > bytes.len() {
                        break;
                    }
                    if let Some(end) = segment_match_at(seg, bytes, s) {
                        if end == bytes.len() {
                            ok = true;
                            break;
                        }
                    }
                    if s == bytes.len() {
                        break;
                    }
                    s += 1;
                }
                if !ok {
                    return false;
                }
                pos = bytes.len();
            } else {
                match segment_find(seg, bytes, pos) {
                    Some((_, end)) => pos = end,
                    None => return false,
                }
            }
        }
        !self.anchor_end || pos == bytes.len() || n == 0
    }

    /// Does the pattern body begin with a literal `/`?
    /// (AUDIT 0.31, A2-F10: the anchored matcher needs this to
    /// skip an explicit `:port` segment before matching — see
    /// `decide_with_method`.)
    fn begins_with_slash(&self) -> bool {
        match self.segments.first().and_then(|seg| seg.first()) {
            Some(Piece::Lit(lit)) => lit.starts_with('/'),
            _ => false,
        }
    }

    /// The longest literal piece anywhere in the pattern (the
    /// Aho-Corasick needle candidate for generic rules: a piece
    /// that must appear verbatim in any URL the rule matches).
    fn longest_literal(&self) -> Option<String> {
        let mut best: Option<&str> = None;
        for seg in &self.segments {
            for piece in seg {
                if let Piece::Lit(lit) = piece {
                    if best.map(|b| lit.len() > b.len()).unwrap_or(true) {
                        best = Some(lit.as_str());
                    }
                }
            }
        }
        best.map(|s| s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Aho-Corasick (multi-literal search for generic-rule candidates)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct AcNode {
    next: HashMap<u8, u32>,
    fail: u32,
    out: Vec<u32>,
}

struct AhoCorasick {
    nodes: Vec<AcNode>,
}

impl AhoCorasick {
    fn build(needles: &[String]) -> AhoCorasick {
        let mut nodes: Vec<AcNode> = vec![AcNode::default()];
        for (id, needle) in needles.iter().enumerate() {
            let mut state = 0usize;
            for &b in needle.as_bytes() {
                let nxt = {
                    let node = &nodes[state];
                    node.next.get(&b).copied()
                };
                state = match nxt {
                    Some(s) => s as usize,
                    None => {
                        let s = nodes.len();
                        nodes.push(AcNode::default());
                        nodes[state].next.insert(b, s as u32);
                        s
                    }
                };
            }
            nodes[state].out.push(id as u32);
        }
        // Failure links, breadth-first.
        let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
        let root_children: Vec<u32> = nodes[0].next.values().copied().collect();
        for child in root_children {
            nodes[child as usize].fail = 0;
            queue.push_back(child);
        }
        while let Some(state) = queue.pop_front() {
            let state = state as usize;
            // Propagate outputs along the fail chain lazily by
            // copying the fail node's outputs into this node.
            let fail = nodes[state].fail as usize;
            if fail != state && !nodes[fail].out.is_empty() {
                let outs = nodes[fail].out.clone();
                nodes[state].out.extend(outs);
            }
            let transitions: Vec<(u8, u32)> =
                nodes[state].next.iter().map(|(b, s)| (*b, *s)).collect();
            for (b, child) in transitions {
                let mut f = nodes[state].fail as usize;
                let target = loop {
                    if let Some(&t) = nodes[f].next.get(&b) {
                        break t as usize;
                    }
                    if f == 0 {
                        break 0;
                    }
                    f = nodes[f].fail as usize;
                };
                nodes[child as usize].fail = target as u32;
                queue.push_back(child);
            }
        }
        AhoCorasick { nodes }
    }

    /// Invoke `f(needle_id)` for every needle occurrence in
    /// `text`. Ids may repeat (overlapping occurrences); callers
    /// dedupe.
    fn scan<F: FnMut(u32)>(&self, text: &[u8], mut f: F) {
        if self.nodes.is_empty() {
            return;
        }
        let mut state = 0usize;
        for &b in text {
            loop {
                if let Some(&t) = self.nodes[state].next.get(&b) {
                    state = t as usize;
                    break;
                }
                if state == 0 {
                    break;
                }
                state = self.nodes[state].fail as usize;
            }
            for &id in &self.nodes[state].out {
                f(id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// A `domain=` restriction (also reused for page-flag scoping):
/// applies when the page host matches an include entry (or there
/// are no includes) and no exclude entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DomainSet {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl DomainSet {
    fn parse(value: &str) -> DomainSet {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for part in value.split('|') {
            let part = part.trim().to_ascii_lowercase();
            if let Some(neg) = part.strip_prefix('~') {
                if !neg.is_empty() {
                    exclude.push(neg.to_string());
                }
            } else if !part.is_empty() {
                include.push(part);
            }
        }
        DomainSet { include, exclude }
    }

    fn matches(&self, page_host: &str) -> bool {
        if self.exclude.iter().any(|e| host_matches(page_host, e)) {
            return false;
        }
        self.include.is_empty() || self.include.iter().any(|e| host_matches(page_host, e))
    }

    /// Parse a COSMETIC scope (the part before `##`, `#@#`,
    /// `##+js`, `#$#`): comma-separated domains, `~` negates.
    /// v22 bug, fixed in v23: this used [`DomainSet::parse`]
    /// (pipe-separated, the `domain=` option grammar), so a
    /// multi-domain line like `a.com,b.com##.ad` was filed under
    /// ONE pseudo-host `"a.com,b.com"` that no real hostname can
    /// ever equal — 111 of the 938 "hosts" in the shipped
    /// payload were dead keys of exactly that shape, and every
    /// rule on those lines silently applied nowhere.
    fn parse_cosmetic(scope: &str) -> DomainSet {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for part in scope.split(',') {
            let part = part.trim().to_ascii_lowercase();
            if let Some(neg) = part.strip_prefix('~') {
                if !neg.is_empty() {
                    exclude.push(neg.to_string());
                }
            } else if !part.is_empty() {
                include.push(part);
            }
        }
        DomainSet { include, exclude }
    }
}

/// One `domain=` entry against a page host. Plain entries cover
/// the host and its subdomains. Entries ending in `.*` (89 of
/// them in the shipped lists — `gentside.*`, `ohmymag.*`) are
/// TLD wildcards: they match when the host with its final label
/// stripped equals the stem or ends with `.<stem>`.
fn host_matches(page_host: &str, entry: &str) -> bool {
    if let Some(stem) = entry.strip_suffix(".*") {
        if stem.is_empty() || stem.contains('*') {
            return false;
        }
        let bare = match page_host.rfind('.') {
            Some(i) if i > 0 => &page_host[..i],
            _ => page_host,
        };
        return bare == stem
            || bare
                .strip_suffix(stem)
                .map(|pre| pre.ends_with('.'))
                .unwrap_or(false);
    }
    page_host == entry
        || page_host
            .strip_suffix(entry)
            .map(|pre| pre.ends_with('.'))
            .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageFlag {
    Generichide,
    Genericblock,
    Elemhide,
}

#[derive(Debug, Clone)]
struct NetRule {
    /// The pattern text (lowercased, options stripped) — the
    /// identity used for dedupe and `$badfilter` twinning.
    text: String,
    pattern: Pattern,
    /// `||host` anchor, when present: the rule is indexed under
    /// this host and only applies to it and its subdomains; the
    /// pattern remainder matches right after the host in the URL.
    anchor_host: Option<String>,
    exception: bool,
    important: bool,
    /// Some(true) = `$third-party` only; Some(false) =
    /// `~third-party` (first-party only); None = either.
    party: Option<bool>,
    types_pos: u32,
    types_neg: u32,
    domains: Option<DomainSet>,
    page_flag: Option<PageFlag>,
    /// `$method=` allow/deny lists (lowercased HTTP methods,
    /// v23). Empty = any method. Deny wins, as with types.
    methods_pos: Vec<String>,
    methods_neg: Vec<String>,
    /// `$to=` scope: matched against the REQUEST host (the
    /// mirror image of `domain=`, which scopes by page). v23.
    to: Option<DomainSet>,
}

impl NetRule {
    fn method_allows(&self, method: Option<&str>) -> bool {
        if self.methods_pos.is_empty() && self.methods_neg.is_empty() {
            return true;
        }
        // A caller that cannot name the method cannot satisfy a
        // method-scoped rule: the rule does not apply (fail-open
        // — an uninformed caller never gets blocked by it).
        let Some(m) = method else { return false };
        let m = m.to_ascii_lowercase();
        if self.methods_neg.contains(&m) {
            return false;
        }
        self.methods_pos.is_empty() || self.methods_pos.contains(&m)
    }

    fn types_allow(&self, t: ResourceType) -> bool {
        let bit = t.bit();
        if self.types_neg & bit != 0 {
            return false;
        }
        self.types_pos == 0 || self.types_pos & bit != 0
    }

    /// The dedupe/badfilter identity: pattern text + the full
    /// option set, minus the `badfilter` flag itself.
    ///
    /// AUDIT V28 (A2-M8): the exception bit is part of the
    /// identity. Through v27 it was not — `@@` is stripped
    /// before `text` is stored, so a block rule and its exact
    /// exception twin shared an identity: whichever line came
    /// second was dropped as a "duplicate", and a `$badfilter`
    /// line neutralised its opposite-kind twin. (Zero shipped
    /// collisions — a refreshed list could have created one.)
    fn identity(&self, options_key: &str) -> String {
        format!(
            "{}\u{1}{}\u{1}{}",
            self.text,
            options_key,
            if self.exception { "exc" } else { "blk" }
        )
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompileReport {
    pub total_lines: usize,
    pub comments: usize,
    pub network_rules: usize,
    pub network_exceptions: usize,
    /// Cosmetic hide LINES accepted (generic + host-specific,
    /// before deduplication/subtraction) — the line-accounting
    /// counterpart to the compiled-set counts below.
    pub cosmetic_lines: usize,
    pub cosmetic_generic: usize,
    pub cosmetic_specific_hosts: usize,
    pub cosmetic_specific_rules: usize,
    pub cosmetic_exceptions: usize,
    pub page_flag_rules: usize,
    /// `##+js(...)` LINES compiled into the scriptlet payload
    /// (v23). [`scriptlet_entries`](Self::scriptlet_entries) is
    /// the per-host entry count those lines produced.
    pub scriptlet_rules: usize,
    /// Scriptlet payload entries across all hosts (after
    /// deduplication and exception subtraction).
    pub scriptlet_entries: usize,
    /// Host keys carrying at least one scriptlet entry.
    pub scriptlet_hosts: usize,
    /// `#@#+js(...)` exception LINES compiled (they subtract
    /// matching entries; a line is counted once however many
    /// entries it removes).
    pub scriptlet_exceptions: usize,
    /// Generic (unscoped) scriptlet entries — the shipped
    /// lists define none, but the channel exists.
    pub scriptlet_generic: usize,
    pub skipped_regex: usize,
    pub skipped_procedural: usize,
    pub skipped_scriptlet: usize,
    pub skipped_unsupported_option: usize,
    /// `$replace=` network rules: counted apart (v23) because
    /// the reason is structural — WebView2 offers no response-
    /// body mutation for network requests, and Chromium uBO
    /// lacks `$replace` for the same reason. YouTube's
    /// `$replace` player rules are twinned by the
    /// trusted-replace scriptlets, which v23 DOES implement.
    pub skipped_replace: usize,
    pub skipped_other: usize,
    pub skipped_invalid: usize,
    pub badfilter_neutralized: usize,
    /// `$badfilter` lines that reached the matcher pass (each
    /// disables its twin and never applies itself).
    pub badfilter_rules: usize,
    pub duplicates_removed: usize,
    pub generic_always_rules: usize,
}

impl CompileReport {
    pub fn skipped_total(&self) -> usize {
        self.skipped_regex
            + self.skipped_procedural
            + self.skipped_scriptlet
            + self.skipped_unsupported_option
            + self.skipped_replace
            + self.skipped_other
            + self.skipped_invalid
    }
    /// Network lines that carried a rule (compiled + skipped for
    /// a *known* unsupported feature). The compile-quality ratio
    /// the tests threshold on.
    pub fn network_accounted(&self) -> usize {
        self.network_rules
            + self.network_exceptions
            + self.skipped_regex
            + self.skipped_unsupported_option
            + self.skipped_replace
            + self.skipped_other
    }
}

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

pub struct ShieldEngine {
    rules: Vec<NetRule>,
    /// Domain-anchored rules by anchor host.
    by_host: HashMap<String, Vec<u32>>,
    /// Generic rules reachable through the AC needle index.
    ac: AhoCorasick,
    ac_rules: Vec<Vec<u32>>,
    /// Generic rules with no usable needle (< 3 chars): always
    /// candidates. Kept few by construction; counted in the
    /// report.
    always: Vec<u32>,
    /// Page-level flags (generichide / genericblock / elemhide)
    /// with their domain scoping (None = everywhere).
    page_flags: Vec<(Option<DomainSet>, PageFlag)>,
    generic_cosmetic: Vec<String>,
    specific_cosmetic: HashMap<String, Vec<String>>,
    /// AUDIT V28 (A2-M4): host/entity -> generic-set selectors a
    /// scoped `#@#` exception (or an exclude-only `##` rule, via
    /// its inverted exception) suppresses on that host. Through
    /// v27 those exceptions were filed and then subtracted only
    /// from the SPECIFIC map, so an exception against a generic
    /// selector did nothing — 316 shipped lines.
    generic_exceptions: HashMap<String, Vec<String>>,
    generic_scriptlets: Vec<ScriptletEntry>,
    specific_scriptlets: HashMap<String, Vec<ScriptletEntry>>,
    report: CompileReport,
}

/// Options the parser understands exactly. A rule carrying any
/// other option is skipped-unsupported rather than misapplied —
/// with the exception of the modifiers below, which change the
/// rule's *effect* in ways v21 deliberately does not implement.
const TYPE_OPTION_NAMES: &[&str] = &[
    "document",
    "doc",
    "subdocument",
    "frame",
    "script",
    "image",
    "stylesheet",
    "css",
    "xmlhttprequest",
    "xhr",
    "media",
    "font",
    "websocket",
    "ping",
    "beacon",
    "other",
];
const UNSUPPORTED_OPTIONS: &[&str] = &[
    "csp",
    "redirect",
    "redirect-rule",
    "rewrite",
    "removeparam",
    "match-case",
    "popup",
    "webrtc",
    "sitekey",
    "collapse",
    "donor",
    "urlskip",
    "denyallow",
    "header",
    "permissions",
    "uritransform",
    "popunder",
];

struct ParsedOptions {
    party: Option<bool>,
    types_pos: u32,
    types_neg: u32,
    domains: Option<DomainSet>,
    important: bool,
    badfilter: bool,
    page_flag: Option<PageFlag>,
    methods_pos: Vec<String>,
    methods_neg: Vec<String>,
    to: Option<DomainSet>,
    /// Canonical option string (badfilter removed) for identity.
    key: String,
}

enum OptParse {
    Ok(ParsedOptions),
    /// An option the engine does not implement: skip the rule.
    Unsupported,
    /// A `$replace=` rule: skipped, but counted in its own
    /// report category (see [`CompileReport::skipped_replace`]).
    Replace,
    /// The option tail was not option-shaped at all: the `$` was
    /// part of the pattern.
    NotOptions,
}

/// Parse an option tail (`a,b=c,~d`). `NotOptions` unless every
/// comma part is option-shaped AND at least one is a known name.
fn parse_options(tail: &str) -> OptParse {
    // `$replace=` is intercepted before the comma split: its
    // value is a /regex/replacement/ pair whose commas belong
    // to the value, not to the option grammar. (Without this,
    // a replace rule containing a comma would fall through to
    // NotOptions and be compiled as a never-matching pattern —
    // a silent misapplication.)
    if tail == "replace"
        || tail.starts_with("replace=")
        || tail.contains(",replace=")
        || tail.contains(",replace,")
    {
        return OptParse::Replace;
    }
    let mut out = ParsedOptions {
        party: None,
        types_pos: 0,
        types_neg: 0,
        domains: None,
        important: false,
        badfilter: false,
        page_flag: None,
        methods_pos: Vec::new(),
        methods_neg: Vec::new(),
        to: None,
        key: String::new(),
    };
    let mut key_parts: Vec<String> = Vec::new();
    let mut any_known = false;
    for raw_part in tail.split(',') {
        let part = raw_part.trim();
        if part.is_empty() {
            return OptParse::NotOptions;
        }
        let (neg, name, value) = match part.split_once('=') {
            Some((n, v)) => {
                let (neg, n) = if let Some(stripped) = n.strip_prefix('~') {
                    (true, stripped)
                } else {
                    (false, n)
                };
                (neg, n, Some(v))
            }
            None => {
                if let Some(stripped) = part.strip_prefix('~') {
                    (true, stripped, None)
                } else {
                    (false, part, None)
                }
            }
        };
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return OptParse::NotOptions;
        }
        if UNSUPPORTED_OPTIONS.contains(&name) {
            return OptParse::Unsupported;
        }
        match name {
            "third-party" => {
                out.party = Some(!neg);
                any_known = true;
            }
            // v23: the short party spellings (and uBO's strict
            // variants, treated as their plain counterparts —
            // strict additionally forbids same-registrable-domain
            // subdomain relations a hostname-only engine cannot
            // distinguish further; documented in AUDIT).
            "1p" | "strict1p" => {
                out.party = Some(false);
                any_known = true;
            }
            "3p" | "strict3p" => {
                out.party = Some(true);
                any_known = true;
            }
            // `$all`: every resource type — a known no-op.
            "all" => {
                any_known = true;
            }
            "important" => {
                out.important = !neg;
                any_known = true;
            }
            "badfilter" => {
                out.badfilter = true;
                any_known = true;
            }
            "domain" | "from" => {
                // `from=` is uBO's spelling of `domain=`.
                if neg || value.is_none() {
                    return OptParse::Unsupported;
                }
                out.domains = Some(DomainSet::parse(value.unwrap_or("")));
                any_known = true;
            }
            "to" => {
                if neg || value.is_none() {
                    return OptParse::Unsupported;
                }
                out.to = Some(DomainSet::parse(value.unwrap_or("")));
                any_known = true;
            }
            "method" => {
                if value.is_none() {
                    return OptParse::Unsupported;
                }
                let list = if neg {
                    &mut out.methods_neg
                } else {
                    &mut out.methods_pos
                };
                for m in value.unwrap_or("").split('|') {
                    let m = m.trim().to_ascii_lowercase();
                    if m.is_empty() || !m.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
                        return OptParse::Unsupported;
                    }
                    list.push(m);
                }
                any_known = true;
            }
            "generichide" | "ghide" => {
                out.page_flag = Some(PageFlag::Generichide);
                any_known = true;
            }
            "genericblock" => {
                out.page_flag = Some(PageFlag::Genericblock);
                any_known = true;
            }
            "elemhide" => {
                out.page_flag = Some(PageFlag::Elemhide);
                any_known = true;
            }
            _ if TYPE_OPTION_NAMES.contains(&name) => {
                if let Some(bit) = type_bit_for_name(name) {
                    if neg {
                        out.types_neg |= bit;
                    } else {
                        out.types_pos |= bit;
                    }
                }
                any_known = true;
            }
            _ => return OptParse::Unsupported,
        }
        if name != "badfilter" {
            key_parts.push(part.to_ascii_lowercase());
        }
    }
    if !any_known {
        return OptParse::NotOptions;
    }
    key_parts.sort();
    out.key = key_parts.join(",");
    OptParse::Ok(out)
}

/// Why a rule's option tail stops it from compiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptSkip {
    None,
    Unsupported,
    Replace,
}

/// Split a network rule into (body, options, skip) at the
/// first `$` whose tail parses as options. ABP patterns may
/// contain literal `$` (rare); those survive as pattern text.
fn split_options(rule: &str) -> (&str, Option<ParsedOptions>, OptSkip) {
    let mut search_from = 0;
    while let Some(rel) = rule[search_from..].find('$') {
        let at = search_from + rel;
        match parse_options(&rule[at + 1..]) {
            OptParse::Ok(opts) => return (&rule[..at], Some(opts), OptSkip::None),
            OptParse::Unsupported => return (&rule[..at], None, OptSkip::Unsupported),
            OptParse::Replace => return (&rule[..at], None, OptSkip::Replace),
            OptParse::NotOptions => search_from = at + 1,
        }
    }
    (rule, None, OptSkip::None)
}

fn host_suffix_walk(host: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = host;
    loop {
        out.push(cur.to_string());
        match cur.find('.') {
            Some(i) => cur = &cur[i + 1..],
            None => break,
        }
    }
    out
}

impl ShieldEngine {
    /// Compile the given list texts into an engine. Every line
    /// is classified and counted in the report; nothing panics
    /// on malformed input (worst case: a line is skipped).
    pub fn compile(lists: &[&str]) -> ShieldEngine {
        let mut report = CompileReport::default();
        let mut raw_rules: Vec<(NetRule, String, bool)> = Vec::new(); // (rule, identity, is_badfilter)
        let mut generic_cosmetic: HashSet<String> = HashSet::new();
        let mut cosmetic_exceptions_generic: HashSet<String> = HashSet::new();
        // host -> selectors; and (domains, selector) exceptions to
        // subtract after all lists are read.
        let mut specific: HashMap<String, HashSet<String>> = HashMap::new();
        let mut specific_exceptions: Vec<(DomainSet, String)> = Vec::new();
        let mut page_flags: Vec<(Option<DomainSet>, PageFlag)> = Vec::new();
        // Scriptlets (v23): per-host entries, generic entries,
        // and exceptions to subtract after all lists are read.
        let mut specific_scriptlets: HashMap<String, Vec<ScriptletEntry>> = HashMap::new();
        let mut generic_scriptlets: Vec<ScriptletEntry> = Vec::new();
        let mut scriptlet_exceptions: Vec<(DomainSet, ScriptletEntry)> = Vec::new();

        for list in lists {
            for raw_line in list.lines() {
                let line = raw_line.trim();
                if line.is_empty() {
                    continue;
                }
                report.total_lines += 1;
                if line.starts_with('!') || line.starts_with('[') {
                    report.comments += 1;
                    continue;
                }
                // --- cosmetic family -------------------------------
                if line.contains("#?#") || line.contains("#@?#") {
                    report.skipped_procedural += 1;
                    continue;
                }
                // --- scriptlets (v23) --------------------------
                // `host##+js(name, args...)` compiles into the
                // per-host payload when the name is in the
                // supported dispatch set; `#@#+js` lines are
                // exceptions, subtracted after the pass. Any
                // other scriptlet name is skipped-and-counted,
                // never approximated.
                if let Some(at) = line.find("#@#+js") {
                    let (lhs, rhs) = (&line[..at], line[at + 6..].trim());
                    match parse_scriptlet_call(rhs) {
                        Some(entry) => {
                            report.scriptlet_exceptions += 1;
                            scriptlet_exceptions.push((DomainSet::parse_cosmetic(lhs), entry));
                        }
                        None => report.skipped_invalid += 1,
                    }
                    continue;
                }
                if let Some(at) = line.find("##+js") {
                    let (lhs, rhs) = (&line[..at], line[at + 5..].trim());
                    match parse_scriptlet_call(rhs) {
                        Some(entry) if scriptlet_supported(&entry.name) => {
                            report.scriptlet_rules += 1;
                            let domains = DomainSet::parse_cosmetic(lhs);
                            if domains.include.is_empty() {
                                generic_scriptlets.push(entry);
                            } else {
                                for host in &domains.include {
                                    specific_scriptlets
                                        .entry(host.clone())
                                        .or_default()
                                        .push(entry.clone());
                                }
                                // Negative scope entries (`~host`)
                                // become subtractions, the cosmetic
                                // convention in this engine.
                                if !domains.exclude.is_empty() {
                                    scriptlet_exceptions.push((
                                        DomainSet {
                                            include: domains.exclude.clone(),
                                            exclude: Vec::new(),
                                        },
                                        entry,
                                    ));
                                }
                            }
                        }
                        Some(_) => report.skipped_scriptlet += 1,
                        None => report.skipped_invalid += 1,
                    }
                    continue;
                }
                if line.contains("#$?#") {
                    report.skipped_other += 1;
                    continue;
                }
                // `host#$#generichide` (and genericblock/elemhide)
                // are page-flag declarations, not style rules —
                // the only `#$#` content this engine honors. Other
                // `#$#` lines (style injections) are skipped.
                if let Some(at) = line.find("#$#") {
                    let (lhs, rhs) = (&line[..at], line[at + 3..].trim());
                    let flag = match rhs {
                        "generichide" => Some(PageFlag::Generichide),
                        "genericblock" => Some(PageFlag::Genericblock),
                        "elemhide" => Some(PageFlag::Elemhide),
                        _ => None,
                    };
                    if let Some(flag) = flag {
                        let domains = DomainSet::parse_cosmetic(lhs);
                        let scope = if domains.include.is_empty() && domains.exclude.is_empty() {
                            None
                        } else {
                            Some(domains)
                        };
                        page_flags.push((scope, flag));
                        report.page_flag_rules += 1;
                    } else {
                        report.skipped_other += 1;
                    }
                    continue;
                }
                if let Some(at) = line.find("#@#") {
                    let (lhs, sel) = (&line[..at], line[at + 3..].trim());
                    if !valid_selector(sel) {
                        report.skipped_invalid += 1;
                        continue;
                    }
                    report.cosmetic_exceptions += 1;
                    let domains = DomainSet::parse_cosmetic(lhs);
                    if domains.include.is_empty() && domains.exclude.is_empty() {
                        cosmetic_exceptions_generic.insert(sel.to_string());
                    } else {
                        specific_exceptions.push((domains, sel.to_string()));
                    }
                    continue;
                }
                if let Some(at) = line.find("##") {
                    let (lhs, sel) = (&line[..at], line[at + 2..].trim());
                    if !valid_selector(sel) {
                        report.skipped_invalid += 1;
                        continue;
                    }
                    // v23: uBO ships PROCEDURAL filters under
                    // `##` too — `##^script:has-text(...)` html
                    // filters and selectors built on extended
                    // pseudo-classes (`:has-text`, `:upward`, …).
                    // They are not CSS: shipped in the stylesheet
                    // they are invalid selectors, and CSS drops
                    // the ENTIRE comma-joined rule on the first
                    // invalid selector — so in v21/v22 one such
                    // selector silently disabled every cosmetic
                    // on its host (159 shipped specific selectors
                    // were of this shape). They are classified
                    // procedural and skipped, like their `#?#`
                    // spellings.
                    if is_procedural_selector(sel) {
                        report.skipped_procedural += 1;
                        continue;
                    }
                    report.cosmetic_lines += 1;
                    let domains = DomainSet::parse_cosmetic(lhs);
                    if domains.include.is_empty() {
                        // Generic. AUDIT V28 (A2-L4): exclusions
                        // used to be dropped here (the selector
                        // then hid on the very hosts it was meant
                        // to spare — 2 shipped lines). They are now
                        // recorded as inverted exceptions, exactly
                        // like the include+exclude case below, and
                        // reach the page through the generic-
                        // exception channel (A2-M4).
                        generic_cosmetic.insert(sel.to_string());
                        if !domains.exclude.is_empty() {
                            specific_exceptions.push((
                                DomainSet {
                                    include: domains.exclude.clone(),
                                    exclude: Vec::new(),
                                },
                                sel.to_string(),
                            ));
                        }
                    } else {
                        for host in &domains.include {
                            specific
                                .entry(host.clone())
                                .or_default()
                                .insert(sel.to_string());
                        }
                        // Exclusions are applied below against
                        // concrete host keys.
                        if !domains.exclude.is_empty() {
                            specific_exceptions.push((
                                DomainSet {
                                    include: domains.exclude.clone(),
                                    exclude: Vec::new(),
                                },
                                sel.to_string(),
                            ));
                        }
                    }
                    continue;
                }
                // --- network family --------------------------------
                let exception = line.starts_with("@@");
                let body_src = if exception { &line[2..] } else { line };
                if body_src.is_empty() {
                    report.skipped_invalid += 1;
                    continue;
                }
                if body_src.starts_with('/') && body_src.ends_with('/') && body_src.len() >= 3 {
                    report.skipped_regex += 1;
                    continue;
                }
                let (body_raw, opts, skip) = split_options(body_src);
                match skip {
                    OptSkip::Unsupported => {
                        report.skipped_unsupported_option += 1;
                        continue;
                    }
                    OptSkip::Replace => {
                        report.skipped_replace += 1;
                        continue;
                    }
                    OptSkip::None => {}
                }
                let opts = opts.unwrap_or(ParsedOptions {
                    party: None,
                    types_pos: 0,
                    types_neg: 0,
                    domains: None,
                    important: false,
                    badfilter: false,
                    page_flag: None,
                    methods_pos: Vec::new(),
                    methods_neg: Vec::new(),
                    to: None,
                    key: String::new(),
                });
                let body = body_raw.to_ascii_lowercase();
                // Domain anchor (computed before the page-flag
                // extraction: flag rules scope by it, below).
                let (anchor_host, pattern_body) = if let Some(rest) = body.strip_prefix("||") {
                    let cut = rest
                        .find(|c| ['^', '/', '*', '|'].contains(&c))
                        .unwrap_or(rest.len());
                    let host = &rest[..cut];
                    if host.is_empty() || host.contains(':') {
                        (None, body.as_str())
                    } else {
                        (Some(host.to_string()), &rest[cut..])
                    }
                } else {
                    (None, body.as_str())
                };
                // AUDIT V28 (A2-M5): remember whether that anchor
                // is a PLAIN host. A host cut short by `*`
                // (||torlock.*/…, ||geo*targetly…), ending in `.`
                // or `-` (||adservice.google.^), or carrying a
                // port produced a by_host key no real host suffix
                // can ever equal — ~194 shipped lines were filed,
                // counted, and permanently dead, including @@
                // exceptions. Non-flag rules with such anchors are
                // downgraded to generic patterns below (after the
                // flag branch): a flag's SCOPE keeps the raw
                // anchor, because a scoped-dead flag is safe
                // while a global one is the V23-3 failure shape.
                let anchor_plain = match body.strip_prefix("||") {
                    Some(rest) => {
                        let cut = rest
                            .find(|c| ['^', '/', '*', '|'].contains(&c))
                            .unwrap_or(rest.len());
                        let host = &rest[..cut];
                        !(host.is_empty()
                            || host.contains(':')
                            || rest.as_bytes().get(cut) == Some(&b'*')
                            || host.ends_with('.')
                            || host.ends_with('-'))
                    }
                    None => true,
                };
                // Page-flag rules ($generichide & friends): the
                // flag never enters the matcher. Its scope is the
                // domain= option when present, else the rule's
                // domain anchor — `@@||tvtoday.de^$ghide` is how
                // the lists actually spell it (401 of the 405
                // shipped flag lines). v21–v22 ignored the anchor
                // and filed every such rule as GLOBAL, which (once
                // V22-7 made global flags reach the runtime)
                // suppressed generic cosmetics on EVERY site.
                if let Some(flag) = opts.page_flag {
                    let scope = match (&opts.domains, &anchor_host) {
                        (Some(d), _) => Some(d.clone()),
                        (None, Some(h)) => Some(DomainSet {
                            include: vec![h.clone()],
                            exclude: Vec::new(),
                        }),
                        (None, None) => None,
                    };
                    page_flags.push((scope, flag));
                    report.page_flag_rules += 1;
                    continue;
                }
                // A2-M5 downgrade (see anchor_plain above): an
                // implausible anchor becomes a generic pattern
                // over the text after `||` — the generic matcher
                // handles `*` and `^`, so ||torlock.*/sww.js
                // compiles to segments "torlock." + "/sww.js"
                // instead of a by_host entry that never fires.
                // One spelling needs care: a trailing "^" right
                // after a "." (||adservice.google.^ — a host-
                // PREFIX rule) would demand a separator byte
                // where real URLs have the next domain label, so
                // that final "^" is dropped and the literal
                // prefix does the work.
                let (anchor_host, pattern_body) = if anchor_plain {
                    (anchor_host, pattern_body)
                } else {
                    match body.strip_prefix("||") {
                        Some(rest) => {
                            let rest = if rest.ends_with(".^") {
                                &rest[..rest.len() - 1]
                            } else {
                                rest
                            };
                            (None, rest)
                        }
                        None => (anchor_host, pattern_body),
                    }
                };
                let text = body.clone();
                let rule = NetRule {
                    text,
                    pattern: Pattern::compile(pattern_body),
                    anchor_host,
                    exception,
                    important: opts.important,
                    party: opts.party,
                    types_pos: opts.types_pos,
                    types_neg: opts.types_neg,
                    domains: opts.domains,
                    page_flag: opts.page_flag,
                    methods_pos: opts.methods_pos,
                    methods_neg: opts.methods_neg,
                    to: opts.to,
                };
                let identity = rule.identity(&opts.key);
                raw_rules.push((rule, identity, opts.badfilter));
            }
        }

        // $badfilter: a badfilter rule neutralizes its twin (same
        // identity) and itself never applies.
        let bad_keys: HashSet<String> = raw_rules
            .iter()
            .filter(|(_, _, bad)| *bad)
            .map(|(_, id, _)| id.clone())
            .collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut rules: Vec<NetRule> = Vec::new();
        for (rule, identity, is_bad) in raw_rules {
            if is_bad {
                report.badfilter_rules += 1;
                continue;
            }
            if bad_keys.contains(identity.as_str()) {
                report.badfilter_neutralized += 1;
                continue;
            }
            if !seen.insert(identity) {
                report.duplicates_removed += 1;
                continue;
            }
            if rule.exception {
                report.network_exceptions += 1;
            } else {
                report.network_rules += 1;
            }
            rules.push(rule);
        }

        // Cosmetic exceptions subtract.
        for sel in &cosmetic_exceptions_generic {
            generic_cosmetic.remove(sel);
        }
        for (domains, sel) in &specific_exceptions {
            let hosts: Vec<String> = specific.keys().cloned().collect();
            for host in hosts {
                if domains.matches(&host) {
                    if let Some(set) = specific.get_mut(&host) {
                        set.remove(sel);
                    }
                }
            }
        }
        let specific_cosmetic: HashMap<String, Vec<String>> = specific
            .into_iter()
            .filter(|(_, set)| !set.is_empty())
            .map(|(host, set)| {
                let mut v: Vec<String> = set.into_iter().collect();
                v.sort();
                (host, v)
            })
            .collect();
        // AUDIT V28 (A2-M4 + A2-L4): the generic-exception
        // channel. A scoped exception whose selector lives in the
        // GENERIC set cannot subtract from the specific map (the
        // selector isn't there); it is recorded per host instead
        // and the cosmetic runtime suppresses those selectors
        // from the combined list on matching pages. Scopes with
        // their own exclusions stay specific-only (documented
        // approximation — the runtime walk has no DomainSet).
        let generic_exceptions: HashMap<String, Vec<String>> = {
            let mut map: HashMap<String, HashSet<String>> = HashMap::new();
            for (domains, sel) in &specific_exceptions {
                if !domains.exclude.is_empty() || !generic_cosmetic.contains(sel) {
                    continue;
                }
                for host in &domains.include {
                    map.entry(host.clone()).or_default().insert(sel.clone());
                }
            }
            map.into_iter()
                .map(|(host, set)| {
                    let mut v: Vec<String> = set.into_iter().collect();
                    v.sort();
                    (host, v)
                })
                .collect()
        };
        report.cosmetic_generic = generic_cosmetic.len();
        report.cosmetic_specific_hosts = specific_cosmetic.len();
        report.cosmetic_specific_rules = specific_cosmetic.values().map(|v| v.len()).sum();

        // Scriptlet exceptions subtract (v23): an exception
        // removes entries with the same canonical name and the
        // same normalized argument vector on every host its
        // scope matches; an exception with NO arguments removes
        // every entry of that name in scope. Dedupe then leaves
        // each host's entries in first-seen order.
        let scriptlet_excepted = |host: &str, e: &ScriptletEntry| -> bool {
            scriptlet_exceptions.iter().any(|(domains, x)| {
                x.name == e.name && (x.args.is_empty() || x.args == e.args) && domains.matches(host)
            })
        };
        // Generic entries are reachable only by a fully global
        // exception (no include, no exclude): a scoped one names
        // concrete hosts by construction.
        generic_scriptlets.retain(|e| {
            !scriptlet_exceptions.iter().any(|(d, x)| {
                d.include.is_empty()
                    && d.exclude.is_empty()
                    && x.name == e.name
                    && (x.args.is_empty() || x.args == e.args)
            })
        });
        let specific_scriptlets: HashMap<String, Vec<ScriptletEntry>> = specific_scriptlets
            .into_iter()
            .map(|(host, entries)| {
                let mut out: Vec<ScriptletEntry> = Vec::new();
                for e in entries {
                    if scriptlet_excepted(&host, &e) {
                        continue;
                    }
                    if !out.contains(&e) {
                        out.push(e);
                    }
                }
                (host, out)
            })
            .filter(|(_, v)| !v.is_empty())
            .collect();
        {
            let mut gseen: HashSet<ScriptletEntry> = HashSet::new();
            generic_scriptlets.retain(|e| gseen.insert(e.clone()));
        }
        report.scriptlet_generic = generic_scriptlets.len();
        report.scriptlet_hosts = specific_scriptlets.len();
        report.scriptlet_entries =
            specific_scriptlets.values().map(|v| v.len()).sum::<usize>() + generic_scriptlets.len();

        // Index network rules.
        let mut by_host: HashMap<String, Vec<u32>> = HashMap::new();
        let mut needles: Vec<String> = Vec::new();
        let mut needle_rules: Vec<Vec<u32>> = Vec::new();
        let mut needle_index: HashMap<String, usize> = HashMap::new();
        let mut always: Vec<u32> = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            let i = i as u32;
            match &rule.anchor_host {
                Some(host) => by_host.entry(host.clone()).or_default().push(i),
                None => {
                    if rule.page_flag.is_some()
                        && rule.pattern.segments.iter().all(|s| s.is_empty())
                    {
                        continue; // patternless flag rule: no network effect
                    }
                    match rule.pattern.longest_literal() {
                        Some(lit) if lit.len() >= 3 => {
                            let id = *needle_index.entry(lit.clone()).or_insert_with(|| {
                                needles.push(lit);
                                needle_rules.push(Vec::new());
                                needles.len() - 1
                            });
                            needle_rules[id].push(i);
                        }
                        _ => {
                            always.push(i);
                            report.generic_always_rules += 1;
                        }
                    }
                }
            }
        }
        let ac = AhoCorasick::build(&needles);

        let mut generic_cosmetic_vec: Vec<String> = generic_cosmetic.into_iter().collect();
        generic_cosmetic_vec.sort();

        ShieldEngine {
            rules,
            by_host,
            ac,
            ac_rules: needle_rules,
            always,
            page_flags,
            generic_cosmetic: generic_cosmetic_vec,
            specific_cosmetic,
            generic_exceptions,
            generic_scriptlets,
            specific_scriptlets,
            report,
        }
    }

    pub fn report(&self) -> &CompileReport {
        &self.report
    }

    fn page_flag_active(&self, flag: PageFlag, page_host: &str) -> bool {
        self.page_flags.iter().any(|(scope, f)| {
            *f == flag && scope.as_ref().map(|s| s.matches(page_host)).unwrap_or(true)
        })
    }

    /// Decide one request. Pure, total, panic-free; every doubt
    /// resolves to Allow (safety law 3).
    pub fn decide(&self, url: &str, page_url: &str, rtype: ResourceType) -> Decision {
        self.decide_with_method(url, page_url, rtype, None)
    }

    /// [`decide`](Self::decide) with the request's HTTP method,
    /// for `$method=` rules (v23). `None` = method unknown:
    /// method-scoped rules then do not apply (fail-open).
    pub fn decide_with_method(
        &self,
        url: &str,
        page_url: &str,
        rtype: ResourceType,
        method: Option<&str>,
    ) -> Decision {
        let Some(req_host) = host_of(url) else {
            return Decision::Allow;
        };
        // Safety law 2: loopback and internal origins, both ways.
        if is_local_host(&req_host) || is_internal_url(url) {
            return Decision::Allow;
        }
        let page_host = match host_of(page_url) {
            Some(h) => h,
            None => return Decision::Allow, // internal/start page: nothing to protect from itself
        };
        if is_local_host(&page_host) {
            return Decision::Allow;
        }
        let url_lower = url.to_ascii_lowercase();
        let third_party = registrable_domain(&req_host) != registrable_domain(&page_host);
        let generic_blocked = self.page_flag_active(PageFlag::Genericblock, &page_host);

        let mut any_block = false;
        let mut any_exception = false;
        let mut any_important_block = false;
        let mut any_important_exception = false;

        let mut consider = |rule: &NetRule, url: &str, start: usize, exact: bool| {
            if rule.page_flag.is_some() {
                return;
            }
            // $genericblock suppresses generic BLOCK rules only
            // (AUDIT V22-5): a generic exception is not a generic
            // block rule, and uBO keeps it in force.
            if rule.anchor_host.is_none() && generic_blocked && !rule.exception {
                return;
            }
            // Safety law 1: main-frame documents.
            if rtype == ResourceType::Document {
                let ok = rule.anchor_host.is_some()
                    && rule.types_pos & ResourceType::Document.bit() != 0;
                if !ok {
                    return;
                }
            }
            if let Some(want_tp) = rule.party {
                if want_tp != third_party {
                    return;
                }
            }
            if !rule.types_allow(rtype) {
                return;
            }
            if !rule.method_allows(method) {
                return;
            }
            if let Some(domains) = &rule.domains {
                if !domains.matches(&page_host) {
                    return;
                }
            }
            // `$to=` scopes by the REQUEST host (v23).
            if let Some(to) = &rule.to {
                if !to.matches(&req_host) {
                    return;
                }
            }
            if !rule.pattern.matches_from(url, start, exact) {
                return;
            }
            match (rule.exception, rule.important) {
                (false, true) => any_important_block = true,
                (true, true) => any_important_exception = true,
                (false, false) => any_block = true,
                (true, false) => any_exception = true,
            }
        };

        // Domain-anchored candidates: walk the request host's
        // suffixes; the pattern remainder begins right after the
        // host in the URL. The host's span comes from parsing the
        // authority (AUDIT V22-4) — a `str::find` for the host
        // string over the whole URL can land inside userinfo or
        // the path when those contain the same text, and for an
        // IPv6 literal the first ':' sits inside the brackets.
        let after_host = host_span(&url_lower, &req_host)
            .map(|(_, after)| after)
            .unwrap_or(url_lower.len());
        for suffix in host_suffix_walk(&req_host) {
            if let Some(idxs) = self.by_host.get(&suffix) {
                for &idx in idxs {
                    let rule = &self.rules[idx as usize];
                    // AUDIT 0.31 (A2-F10): an anchored pattern
                    // body that begins with '/' describes the
                    // PATH — but with an explicit port the
                    // remainder after the host begins with ':',
                    // so the slash form could never match a
                    // ported URL (`||ads.example.com/banner.js`
                    // vs `https://ads.example.com:8443/banner.js`).
                    // Skip the `:port` segment (through the next
                    // '/') before matching those rules only;
                    // `||host^` and friends keep matching at
                    // `after_host`, where ':' is the separator
                    // they expect.
                    let mut start = after_host;
                    if rule.pattern.begins_with_slash() && url_lower[start..].starts_with(':') {
                        start = match url_lower[start..].find('/') {
                            Some(rel) => start + rel,
                            None => url_lower.len(),
                        };
                    }
                    consider(rule, &url_lower, start, true);
                }
            }
        }

        // Generic candidates via the AC needle index.
        let mut candidates: Vec<u32> = Vec::new();
        self.ac.scan(url_lower.as_bytes(), |nid| {
            if let Some(rs) = self.ac_rules.get(nid as usize) {
                candidates.extend(rs.iter().copied());
            }
        });
        candidates.extend(self.always.iter().copied());
        candidates.sort_unstable();
        candidates.dedup();
        for idx in candidates {
            let rule = &self.rules[idx as usize];
            consider(rule, &url_lower, 0, false);
        }

        if any_important_block && !any_important_exception {
            return Decision::Block;
        }
        if any_important_exception {
            return Decision::Allow;
        }
        if any_exception {
            return Decision::Allow;
        }
        if any_block {
            return Decision::Block;
        }
        Decision::Allow
    }

    // -- cosmetic layer ------------------------------------------------------

    pub fn generic_cosmetic(&self) -> &[String] {
        &self.generic_cosmetic
    }

    pub fn specific_cosmetic(&self) -> &HashMap<String, Vec<String>> {
        &self.specific_cosmetic
    }

    /// Build the document-start cosmetic script: a self-contained
    /// runtime carrying the generic selectors plus host-specific
    /// selectors for the highest-volume hosts that fit the byte
    /// budget. Returns (script, hosts_included, generic_included).
    /// The script is a no-op on non-http(s) and loopback pages
    /// (safety law 2, cosmetic half) and cannot throw into the
    /// page.
    pub fn build_cosmetic_script(&self, byte_budget: usize) -> (String, usize, usize) {
        // Page-flag cosmetics data (AUDIT V22-6/7): generichide
        // domains (generic set suppressed there), elemhide
        // domains (ALL cosmetics suppressed there), and the two
        // global forms (scope None = everywhere). v21 collected
        // only scoped generichide hosts: elemhide was parsed but
        // never honored, and a global generichide reached no
        // list at all.
        let mut gh_hosts: Vec<String> = Vec::new();
        let mut gh_all = false;
        let mut eh_hosts: Vec<String> = Vec::new();
        let mut eh_all = false;
        for (scope, flag) in &self.page_flags {
            let (hosts, all) = match flag {
                PageFlag::Generichide => (&mut gh_hosts, &mut gh_all),
                PageFlag::Elemhide => (&mut eh_hosts, &mut eh_all),
                PageFlag::Genericblock => continue,
            };
            match scope {
                Some(set) => {
                    for h in &set.include {
                        hosts.push(h.clone());
                    }
                }
                None => *all = true,
            }
        }
        // Generic selectors are the core value and always ship in
        // full. Host-specific sets join in rule-volume order until
        // the byte budget is spent (the script's payload ceiling
        // is a hard number, measured in BUILD_NOTES).
        let generic_json =
            serde_json::to_string(&self.generic_cosmetic).unwrap_or_else(|_| "[]".to_string());
        let mut ordered: Vec<(&String, &Vec<String>)> = self.specific_cosmetic.iter().collect();
        ordered.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
        let mut map = serde_json::Map::new();
        let mut count = 0usize;
        // v23: the flag-host JSON (generichide/elemhide lists)
        // is counted too — v22's allowance ignored it, so the
        // shipped script drifted ~8 KB over the nominal budget
        // once the anchor-scoped flags (V23-3) were carried.
        // AUDIT V28: the accounting now starts from the REAL
        // runtime length. Through v27 a flat 512-byte allowance
        // stood in for it, so the "budget" never quite meant
        // script bytes; the runtime grew (entity walk, the ge
        // suppression block) and the fiction started cutting
        // real hosts while the script still fit.
        let mut len = COSMETIC_RUNTIME.len()
            + generic_json.len()
            + serde_json::to_string(&gh_hosts)
                .map(|s| s.len())
                .unwrap_or(0)
            + serde_json::to_string(&eh_hosts)
                .map(|s| s.len())
                .unwrap_or(0)
            + serde_json::to_string(&self.generic_exceptions)
                .map(|s| s.len())
                .unwrap_or(0)
            + 256; // JSON braces/keys overhead allowance
        for (host, sels) in ordered {
            let approx = host.len() + sels.iter().map(|s| s.len() + 3).sum::<usize>() + 8;
            if count > 0 && len + approx > byte_budget {
                continue;
            }
            len += approx;
            count += 1;
            map.insert(
                host.clone(),
                serde_json::Value::Array(
                    sels.iter()
                        .map(|s| serde_json::Value::String(s.clone()))
                        .collect(),
                ),
            );
        }
        let specific_json = serde_json::to_string(&serde_json::Value::Object(map))
            .unwrap_or_else(|_| "{}".to_string());
        let gh_json = serde_json::to_string(&gh_hosts).unwrap_or_else(|_| "[]".to_string());
        let eh_json = serde_json::to_string(&eh_hosts).unwrap_or_else(|_| "[]".to_string());
        // AUDIT V28 (A2-M4): the generic-exception channel ships
        // in full (it is small: exceptions against the generic
        // set, keyed by host/entity) and counts toward the same
        // budget accounting.
        let ge_json =
            serde_json::to_string(&self.generic_exceptions).unwrap_or_else(|_| "{}".to_string());
        let data = format!(
            "{{\"g\":{},\"s\":{},\"ge\":{},\"gh\":{},\"ghAll\":{},\"eh\":{},\"ehAll\":{}}}",
            generic_json, specific_json, ge_json, gh_json, gh_all, eh_json, eh_all
        );
        let data = data.replace("</", "<\\/");
        let script = COSMETIC_RUNTIME.replace("__PICO_SHIELD_DATA__", &data);
        (script, count, self.generic_cosmetic.len())
    }
}

fn valid_selector(sel: &str) -> bool {
    !sel.is_empty()
        && sel.len() <= 1024
        && !sel.contains('{')
        && !sel.contains('}')
        && !sel.contains('<')
        && !sel.contains('\n')
}

/// uBO-extended pseudo-classes: selectors using them are
/// procedural filters, not CSS (see the compile pass). `:has(`
/// itself is standard CSS and deliberately NOT in this list;
/// `:has-text(` is the uBO invention.
const EXTENDED_PSEUDOS: &[&str] = &[
    ":has-text(",
    ":contains(",
    ":matches-css",
    ":matches-attr(",
    ":matches-prop(",
    ":upward(",
    ":xpath(",
    ":others(",
    ":watch-attr(",
    ":min-text-length(",
    ":has-display",
    // AUDIT V28 (A2-H3): the v23 denylist was incomplete —
    // these uBO actions shipped as plain CSS selectors, and
    // because the cosmetic runtime concatenates a host's
    // selectors into ONE rule, one invalid selector killed the
    // whole rule (generic set included) on ~90 :style( hosts.
    // They are actions this runtime cannot perform (it only
    // hides), so they are procedural skips, counted as such.
    ":style(",
    ":remove(",
    ":matches-path(",
    ":remove-attr(",
    ":remove-class(",
];

fn is_procedural_selector(sel: &str) -> bool {
    sel.starts_with('^') || EXTENDED_PSEUDOS.iter().any(|p| sel.contains(p))
}

/// The cosmetic runtime (document-start). Deliberately boring:
/// build one stylesheet of `display:none!important` rules from
/// the generic set plus the rules for this host (parent-walked),
/// unless the host carries a generichide flag. A stylesheet —
/// not per-element JS — so late-inserted nodes are covered by
/// the CSS engine itself; no MutationObserver needed.
///
/// AUDIT 0.31 (A2-F4+F5): the runtime normalizes its hostname
/// ONCE at acquisition — `.replace(/\.+$/, "")` strips
/// trailing dots like the Rust layer's `host_of` — so a
/// trailing-dot FQDN (www.youtube.com.) hits its host-specific
/// cosmetics and `foo.localhost.` / `127.0.0.1.` classify
/// loopback in JS too. (The note lives HERE, not inside the
/// string: the runtime's length is the cosmetic budget's
/// starting term, so in-string comments cost real hosts.)
const COSMETIC_RUNTIME: &str = r#"(function () {
  try {
    if (window.__picoShieldCosmetic) return;
    window.__picoShieldCosmetic = 1;
    if (location.protocol !== "http:" && location.protocol !== "https:") return;
    var host = (location.hostname || "").toLowerCase().replace(/\.+$/, "");
    // AUDIT V28 (A2-L2): the JS spelling of the loopback law now
    // matches the Rust one exactly — 127.0.0.0/8 as a numeric
    // IPv4 literal. The old prefix test also exempted public
    // names like 127.example.com, which Rust treats as public.
    if (!host || host === "localhost" || host === "localhost." || host === "::1" || host === "0.0.0.0" || host.endsWith(".localhost") || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)) return;
    var D = __PICO_SHIELD_DATA__;
    function walk(obj, h) {
      var cur = h;
      for (;;) {
        if (Object.prototype.hasOwnProperty.call(obj, cur)) return obj[cur];
        var i = cur.indexOf(".");
        if (i < 0) return null;
        cur = cur.slice(i + 1);
      }
    }
    // AUDIT V28 (A2-M3): entity scopes (example.*) compiled into
    // the payload but the walk never synthesised the key, so
    // they could never match. The entity of a host is its
    // registrable domain + ".*", with the two-level-suffix
    // adjustment the scriptlet runtime also uses (A2-L12).
    function entityKey(h) {
      var labels = h.split(".");
      if (labels.length < 2) return null;
      var take = 1;
      if (labels.length >= 3 && labels[labels.length - 1].length === 2 &&
          /^(co|com|org|net|ac|gov|edu|or|ne)$/.test(labels[labels.length - 2])) take = 2;
      return labels[labels.length - 1 - take] + ".*";
    }
    // elemhide (AUDIT V22-6): no cosmetics at all on flagged
    // pages — global, or this host (or a parent of it).
    if (D.ehAll) return;
    for (var k2 = 0; k2 < D.eh.length; k2++) {
      var e2 = D.eh[k2];
      if (host === e2 || host.endsWith("." + e2)) return;
    }
    var sels = [];
    var gh = D.ghAll === true;
    for (var k = 0; k < D.gh.length; k++) {
      var g = D.gh[k];
      if (host === g || host.endsWith("." + g)) { gh = true; break; }
    }
    if (!gh) sels = sels.concat(D.g);
    var sp = walk(D.s, host);
    if (sp) sels = sels.concat(sp);
    var ek = entityKey(host);
    if (ek && D.s[ek]) sels = sels.concat(D.s[ek]);
    // AUDIT V28 (A2-M4): suppress generic-set selectors a scoped
    // exception lifts on this host. Exceptions are collected at
    // EVERY suffix level plus the entity key — an exception wins
    // wherever it is scoped.
    if (D.ge) {
      var banned = null;
      var cur = host;
      for (;;) {
        if (D.ge[cur]) { banned = banned || {}; for (var bi = 0; bi < D.ge[cur].length; bi++) banned[D.ge[cur][bi]] = 1; }
        var ci = cur.indexOf(".");
        if (ci < 0) break;
        cur = cur.slice(ci + 1);
      }
      if (ek && D.ge[ek]) { banned = banned || {}; for (var bj = 0; bj < D.ge[ek].length; bj++) banned[D.ge[ek][bj]] = 1; }
      if (banned) sels = sels.filter(function (s) { return !banned[s]; });
    }
    if (!sels.length) return;
    var css = sels.join(",\n") + " { display: none !important; }";
    function apply() {
      try {
        var st = document.getElementById("pico-shield-style");
        if (!st) {
          st = document.createElement("style");
          st.id = "pico-shield-style";
          (document.head || document.documentElement).appendChild(st);
        }
        if (st.textContent !== css) st.textContent = css;
      } catch (e) { /* page without a usable root: stay silent */ }
    }
    apply();
    document.addEventListener("DOMContentLoaded", apply);
  } catch (e) { /* never break the page */ }
})();
"#;

// ---------------------------------------------------------------------------
// Scriptlets (v23) — the `##+js(...)` layer
// ---------------------------------------------------------------------------

/// One compiled scriptlet invocation: a canonical dispatch
/// name plus its RAW argument tokens (quotes and `/regex/`
/// literals preserved exactly as written in the list). The
/// tokens cross into the page as JSON data and are interpreted
/// by the hand-written [`SCRIPTLET_RUNTIME`]; list content is
/// never interpolated into code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScriptletEntry {
    pub name: String,
    pub args: Vec<String>,
}

/// Canonicalize a scriptlet name: uBO lists use both the short
/// aliases (`aopr`) and the long forms (`set-constant`), and
/// two spellings of the fetch-response replacer. The runtime's
/// dispatch table keys on the canonical names.
fn canonical_scriptlet_name(raw: &str) -> String {
    let n = raw.trim().to_ascii_lowercase();
    match n.as_str() {
        "set-constant" => "set".to_string(),
        "abort-on-property-read" => "aopr".to_string(),
        "abort-on-property-write" => "aopw".to_string(),
        "abort-current-inline-script" | "abort-current-script" => "acs".to_string(),
        "remove-attr" => "ra".to_string(),
        "trusted-replace-fetch-response" => "trusted-rpfr".to_string(),
        "prevent-xhr" => "no-xhr-if".to_string(),
        _ => n,
    }
}

/// The dispatch set the runtime implements — chosen from the
/// Phase 0 census of the shipped lists (AUDIT v23): these
/// names cover every YouTube rule and the highest-value
/// families overall. Any other name (aeld, nostif, json-edit,
/// procedural traps, …) is skipped-and-counted, never faked.
fn scriptlet_supported(name: &str) -> bool {
    matches!(
        name,
        "set"
            | "json-prune"
            | "json-prune-fetch-response"
            | "json-prune-xhr-response"
            | "trusted-rpfr"
            | "trusted-replace-xhr-response"
            | "no-fetch-if"
            | "no-xhr-if"
            | "aopr"
            | "aopw"
            | "acs"
            | "ra"
            | "set-attr"
    )
}

/// Parse a scriptlet call — the text after `##+js` /
/// `#@#+js`, shaped `(name, arg, arg...)` — into an entry.
/// The argument tokenizer follows uBO's grammar: each argument
/// is a `'...'` or `"..."` string, a `/regex/flags` literal, or
/// a bare token, and commas separate arguments only at the top
/// level (YouTube's own rules rely on this: `...\}{2\,4}...`
/// carries an escaped comma inside a regex, and the set-attr
/// rule carries quotes inside a bare selector). Like uBO's own
/// parser it is tolerant of the lists' sloppy edges: an
/// unterminated string or a `/regex` with no closing slash at
/// end-of-input degrades to a bare token rather than rejecting
/// the line. Returns None only when the call shape itself is
/// wrong (no parentheses, empty/oversized name, absurd arity).
fn parse_scriptlet_call(rhs: &str) -> Option<ScriptletEntry> {
    let rhs = rhs.trim();
    if !rhs.starts_with('(') || !rhs.ends_with(')') {
        return None;
    }
    let inner = &rhs[1..rhs.len() - 1];
    let b = inner.as_bytes();
    let mut args: Vec<String> = Vec::new();
    let mut i = 0usize;
    // Scan one token starting at `i` (first non-space byte);
    // returns the index just past the token.
    fn scan_quoted(b: &[u8], mut i: usize, q: u8) -> usize {
        i += 1; // opening quote
        while i < b.len() {
            if b[i] == b'\\' {
                i += 2;
                continue;
            }
            if b[i] == q {
                return i + 1;
            }
            i += 1;
        }
        b.len() // unterminated: token runs to the end (tolerated)
    }
    fn scan_regex(b: &[u8], i: usize) -> Option<usize> {
        // From the opening '/', find an unescaped closing '/'
        // outside a [...] class, then flag letters, then require
        // ',' or end — otherwise this is not a regex token.
        let mut j = i + 1;
        let mut in_class = false;
        while j < b.len() {
            match b[j] {
                b'\\' => {
                    j += 2;
                    continue;
                }
                b'[' => in_class = true,
                b']' => in_class = false,
                b'/' if !in_class => {
                    let mut k = j + 1;
                    while k < b.len() && b[k].is_ascii_lowercase() {
                        k += 1;
                    }
                    if k >= b.len() || b[k] == b',' {
                        return Some(k);
                    }
                    return None;
                }
                _ => {}
            }
            j += 1;
        }
        None
    }
    fn scan_bare(b: &[u8], mut i: usize) -> usize {
        // Bare token: ends at the next top-level comma; quoted
        // sections inside it (a selector's [attr="a,b"]) do not
        // split.
        let mut quote: u8 = 0;
        while i < b.len() {
            let c = b[i];
            if quote != 0 {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == quote {
                    quote = 0;
                }
                i += 1;
                continue;
            }
            match c {
                b',' => return i,
                b'\'' | b'"' => {
                    quote = c;
                    i += 1;
                }
                b'\\' => i += 2,
                _ => i += 1,
            }
        }
        b.len()
    }
    loop {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        // A quote or slash opens a string/regex token only if
        // the token is properly terminated by ',' or the end
        // (a bare JSON-ish argument like
        // `"adBlockWallEnabled":true` merely STARTS with a
        // quote — it is one bare token, quotes and all).
        let end = match b[i] {
            b'\'' | b'"' => {
                let e = scan_quoted(b, i, b[i]);
                let mut k = e;
                while k < b.len() && b[k] == b' ' {
                    k += 1;
                }
                if k >= b.len() || b[k] == b',' {
                    e
                } else {
                    scan_bare(b, i)
                }
            }
            b'/' => scan_regex(b, i).unwrap_or_else(|| scan_bare(b, i)),
            _ => scan_bare(b, i),
        };
        args.push(inner[i..end].trim().to_string());
        i = end;
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] != b',' {
            return None; // junk between tokens
        }
        i += 1;
        if i >= b.len() {
            args.push(String::new()); // trailing comma: empty arg
            break;
        }
    }
    let mut it = args.into_iter();
    let name_raw = it.next().unwrap_or_default();
    if name_raw.is_empty() || name_raw.len() > 64 {
        return None;
    }
    let name = canonical_scriptlet_name(&name_raw);
    let args: Vec<String> = it.collect();
    if args.len() > 16 || args.iter().any(|a| a.len() > 4096) {
        return None;
    }
    Some(ScriptletEntry { name, args })
}

impl ShieldEngine {
    pub fn generic_scriptlets(&self) -> &[ScriptletEntry] {
        &self.generic_scriptlets
    }

    pub fn specific_scriptlets(&self) -> &HashMap<String, Vec<ScriptletEntry>> {
        &self.specific_scriptlets
    }

    /// Build the scriptlet payload script: the audited runtime
    /// plus, as JSON data, the generic entries and the per-host
    /// entries for the highest-volume hosts fitting the byte
    /// budget. The YouTube family — this layer's reason to
    /// exist — is always included first, regardless of volume
    /// order. Returns (script, hosts_included, entries_included).
    pub fn build_scriptlet_script(&self, byte_budget: usize) -> (String, usize, usize) {
        const PRIORITY: &[&str] = &[
            "www.youtube.com",
            "youtube.com",
            "m.youtube.com",
            "music.youtube.com",
            "tv.youtube.com",
            "youtubekids.com",
            "youtube-nocookie.com",
        ];
        let entry_json = |e: &ScriptletEntry| -> serde_json::Value {
            let mut m = serde_json::Map::new();
            m.insert("n".to_string(), serde_json::Value::String(e.name.clone()));
            m.insert(
                "a".to_string(),
                serde_json::Value::Array(
                    e.args
                        .iter()
                        .map(|a| serde_json::Value::String(a.clone()))
                        .collect(),
                ),
            );
            serde_json::Value::Object(m)
        };
        let generic_json = serde_json::to_string(&serde_json::Value::Array(
            self.generic_scriptlets.iter().map(entry_json).collect(),
        ))
        .unwrap_or_else(|_| "[]".to_string());
        let mut ordered: Vec<(&String, &Vec<ScriptletEntry>)> =
            self.specific_scriptlets.iter().collect();
        ordered.sort_by(|a, b| {
            let pa = PRIORITY.contains(&a.0.as_str());
            let pb = PRIORITY.contains(&b.0.as_str());
            pb.cmp(&pa)
                .then(b.1.len().cmp(&a.1.len()))
                .then(a.0.cmp(b.0))
        });
        let mut map = serde_json::Map::new();
        let mut hosts = 0usize;
        let mut entries = self.generic_scriptlets.len();
        let mut len = generic_json.len() + 512;
        for (host, list) in ordered {
            let approx = host.len()
                + list
                    .iter()
                    .map(|e| e.name.len() + e.args.iter().map(|a| a.len() + 3).sum::<usize>() + 10)
                    .sum::<usize>()
                + 8;
            if hosts > 0 && len + approx > byte_budget {
                continue;
            }
            len += approx;
            hosts += 1;
            entries += list.len();
            map.insert(
                host.clone(),
                serde_json::Value::Array(list.iter().map(entry_json).collect()),
            );
        }
        let specific_json = serde_json::to_string(&serde_json::Value::Object(map))
            .unwrap_or_else(|_| "{}".to_string());
        let data = format!("{{\"g\":{},\"s\":{}}}", generic_json, specific_json);
        let data = data.replace("</", "<\\/");
        let script = SCRIPTLET_RUNTIME.replace("__PICO_SCRIPTLET_DATA__", &data);
        (script, hosts, entries)
    }

    /// The ONE document-start script the app injects: scriptlet
    /// runtime first (its traps must exist before any page
    /// script runs), then the cosmetic runtime, then the
    /// filter pipeline's ghost-box collapse layer (AUDIT V26
    /// D6-a — it must be registered before page images can
    /// fail). All three are independently guarded, so a page
    /// that already received one is never double-wrapped.
    pub fn build_document_script(
        &self,
        cosmetic_budget: usize,
        scriptlet_budget: usize,
    ) -> (String, DocumentScriptInfo) {
        let (scriptlet_script, scriptlet_hosts, scriptlet_entries) =
            self.build_scriptlet_script(scriptlet_budget);
        let (cosmetic_script, cosmetic_hosts, _) = self.build_cosmetic_script(cosmetic_budget);
        let mut script = String::with_capacity(
            scriptlet_script.len()
                + cosmetic_script.len()
                + crate::core::filterpipe::COLLAPSE_RUNTIME.len()
                + 2,
        );
        script.push_str(&scriptlet_script);
        script.push('\n');
        script.push_str(&cosmetic_script);
        script.push('\n');
        script.push_str(crate::core::filterpipe::COLLAPSE_RUNTIME);
        let info = DocumentScriptInfo {
            bytes: script.len(),
            cosmetic_hosts,
            scriptlet_hosts,
            scriptlet_entries,
        };
        (script, info)
    }
}

/// What a built document script carries (for the startup log
/// line and the tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentScriptInfo {
    pub bytes: usize,
    pub cosmetic_hosts: usize,
    pub scriptlet_hosts: usize,
    pub scriptlet_entries: usize,
}

/// The scriptlet runtime (document-start, v23). ONE hand-written
/// implementation for every supported scriptlet; the compiled
/// list rules arrive as DATA (`__PICO_SCRIPTLET_DATA__`: name +
/// raw argument tokens) and are interpreted through the fixed
/// dispatch at the bottom — list content is never evaluated as
/// code. Discipline, mirroring the cosmetic runtime and the
/// engine's laws: no-op on non-http(s) and loopback pages
/// (checked BEFORE any wrapper is installed, so on a localhost
/// page `window.fetch` keeps its exact original identity),
/// every entry and every wrapper is individually try/caught
/// (any failure degrades to unmodified browsing), and the one
/// atomic OFF flag (`window.__picoShieldOff`, set by the app's
/// toggle) disarms every wrapper dynamically.
const SCRIPTLET_RUNTIME: &str = r#"(function () {
  "use strict";
  try {
    if (window.__picoShieldScriptlets) return;
    if (location.protocol !== "http:" && location.protocol !== "https:") return;
    // AUDIT 0.31 (A2-F4+F5): normalize the hostname ONCE at
    // acquisition — strip trailing dots, like the Rust layer's
    // host_of already does — so a trailing-dot FQDN
    // (www.youtube.com.) hits its host-specific scriptlets and
    // foo.localhost. / 127.0.0.1. classify loopback here too.
    var host = (location.hostname || "").toLowerCase().replace(/\.+$/, "");
    if (!host || host === "localhost" || host === "localhost." || host === "::1" || host === "[::1]" || host === "0.0.0.0" || host.endsWith(".localhost") || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)) return;
    var D = __PICO_SCRIPTLET_DATA__;
    // ---- collect this host's entries: generic, exact host,
    // every parent domain (uBO applies a domain's scriptlets to
    // its subdomains), and the entity key `<sld>.*`.
    var entries = [];
    var seenKeys = {};
    function addAll(list) {
      if (!list) return;
      for (var i = 0; i < list.length; i++) {
        var e = list[i];
        var k = e.n + "\u0001" + (e.a || []).join("\u0001");
        if (seenKeys[k]) continue;
        seenKeys[k] = 1;
        entries.push(e);
      }
    }
    addAll(D.g);
    var cur = host;
    for (;;) {
      addAll(D.s[cur]);
      var di = cur.indexOf(".");
      if (di < 0) break;
      cur = cur.slice(di + 1);
    }
    var labels = host.split(".");
    // AUDIT V28 (A2-L12): the entity key is the registrable
    // domain + ".*"; on two-level suffixes (foo.co.uk) the old
    // labels[len-2] produced "co.*" — entity scriptlets were
    // missed there entirely.
    if (labels.length >= 2) {
      var etake = 1;
      if (labels.length >= 3 && labels[labels.length - 1].length === 2 &&
          /^(co|com|org|net|ac|gov|edu|or|ne)$/.test(labels[labels.length - 2])) etake = 2;
      addAll(D.s[labels[labels.length - 1 - etake] + ".*"]);
    }
    window.__picoShieldScriptlets = 1;
    if (!entries.length) return;

    function off() { return !!window.__picoShieldOff; }

    // ---- prune accounting: per-path counters, flushed to the
    // host through the same window.ipc channel the nav notes
    // use (kind "shield_prune"), throttled and capped. The live
    // object is also exposed for tests/diagnostics.
    var counts = {};
    var flushTimer = 0;
    window.__picoShieldPruneCounts = counts;
    function hit(p) {
      try {
        counts[p] = (counts[p] || 0) + 1;
        if (!flushTimer) flushTimer = setTimeout(flushCounts, 2500);
      } catch (e) {}
    }
    // 0.30: the MEASURED half of prune accounting — the exact
    // serialized byte size of every subtree prunePath deletes,
    // accumulated by notePruned() and flushed as
    // {kind:"shield_pruned", bytes, events} on the same
    // throttle. The host validates the shape, rate-caps per
    // tab, and keeps the figure OUT of the estimated-savings
    // totals (these bytes were downloaded inside the JSON;
    // they are a separate, exact line — see core::stats).
    var prunedBytes = 0;
    var prunedEvents = 0;
    window.__picoShieldPruned = { bytes: 0, events: 0 };
    function notePruned(size) {
      prunedBytes += size;
      prunedEvents += 1;
      window.__picoShieldPruned.bytes = prunedBytes;
      window.__picoShieldPruned.events = prunedEvents;
      if (!flushTimer) flushTimer = setTimeout(flushCounts, 2500);
    }
    function flushCounts() {
      flushTimer = 0;
      try {
        var items = [];
        for (var k in counts) {
          // AUDIT 0.31 (A2-F16): delete ONLY the keys actually
          // included in this 16-item payload — through 0.30.1
          // every key was deleted, so counts past the cap were
          // silently discarded instead of riding the next flush.
          if (items.length < 16) { items.push({ p: k, n: Math.min(counts[k], 100000) }); delete counts[k]; }
        }
        if (items.length && window.ipc && window.ipc.postMessage) {
          window.ipc.postMessage(JSON.stringify({ kind: "shield_prune", items: items }));
        }
        if ((prunedBytes > 0 || prunedEvents > 0) && window.ipc && window.ipc.postMessage) {
          // One report carries at most the host's per-report
          // ceilings (bytes AND events — AUDIT 0.30 A2: the
          // first cut capped only bytes, so a >100k-event
          // flush was rejected whole by the host AFTER the
          // runtime had discarded it); any remainder of
          // either rides the next flush.
          var sendBytes = Math.min(prunedBytes, 16777216);
          var sendEvents = Math.min(prunedEvents, 100000);
          window.ipc.postMessage(JSON.stringify({ kind: "shield_pruned", bytes: sendBytes, events: sendEvents }));
          prunedBytes -= sendBytes;
          prunedEvents -= sendEvents;
          window.__picoShieldPruned.bytes = prunedBytes;
          window.__picoShieldPruned.events = prunedEvents;
          if (prunedBytes > 0 || prunedEvents > 0) flushTimer = setTimeout(flushCounts, 2500);
        }
      } catch (e) {}
    }
    try { window.addEventListener("pagehide", flushCounts); } catch (e) {}

    // ---- argument decoding. Tokens arrive raw: `'quoted'`,
    // `"quoted"`, `/regex/flags`, or bare.
    function unq(a) {
      if (typeof a !== "string" || a.length < 2) return a;
      var c = a.charAt(0);
      if (c !== "'" && c !== '"') return a;
      if (a.charAt(a.length - 1) === c) {
        var body = a.slice(1, -1);
        if (c === '"') { try { return JSON.parse(a); } catch (e) {} }
        return body.replace(/\\(.)/g, "$1");
      }
      // Unterminated string (the lists contain a few, e.g.
      // `acs, Math, '0x`): strip the lone leading quote.
      return a.slice(1).replace(/\\(.)/g, "$1");
    }
    function asRegex(a) {
      if (typeof a !== "string" || a.charAt(0) !== "/") return null;
      var m = /^\/(.*)\/([a-z]*)$/.exec(a);
      if (!m) return null;
      try { return new RegExp(m[1], m[2]); } catch (e) { return null; }
    }
    function patOf(a) {
      // A URL condition: /regex/flags, or a substring.
      var re = asRegex(a);
      if (re) return { re: re };
      var s = unq(a);
      return s ? { str: s } : null;
    }
    function patMatch(p, url) {
      if (!p) return false;
      if (p.re) { p.re.lastIndex = 0; return p.re.test(url); }
      return url.indexOf(p.str) !== -1;
    }
    function condsOf(args) {
      // propsToMatch marker tokens are uBO bookkeeping; the
      // conditions are the remaining tokens (`url:<pat>`, a
      // bare /regex/, or a bare substring).
      var pats = [];
      for (var i = 0; i < args.length; i++) {
        var a = args[i];
        if (a === "propsToMatch") continue;
        if (a.indexOf("url:") === 0) a = a.slice(4);
        var p = patOf(a);
        if (p) pats.push(p);
      }
      return pats;
    }
    function condsMatch(pats, url) {
      if (!pats || !pats.length) return true;
      for (var i = 0; i < pats.length; i++) if (patMatch(pats[i], url)) return true;
      return false;
    }

    // ---- property-path engine. Segments: plain keys, numeric
    // indices, `[]` (any array element), `[-]` (last element),
    // `*` (any key). The `..` recursive-descent and `*[?...]`
    // filter forms belong to the json-edit family, which this
    // runtime deliberately does not implement (AUDIT v23).
    // The exact serialized size (UTF-8 bytes) of a value
    // about to be pruned — measured BEFORE deletion, counted
    // only when the deletion lands (see prunePath's leaf).
    // JSON.stringify of an already-parsed subtree cannot
    // cycle (JSON.parse output is a tree); a hostile getter
    // could still throw, which the guard turns into 0.
    function byteSizeOf(v) {
      try {
        var s = JSON.stringify(v);
        if (typeof s !== "string") return 0;
        var n = 0;
        for (var i = 0; i < s.length; i++) {
          var c = s.charCodeAt(i);
          if (c < 0x80) n += 1;
          else if (c < 0x800) n += 2;
          else if (c >= 0xD800 && c <= 0xDBFF) { n += 4; i++; }
          else n += 3;
        }
        return n;
      } catch (e) { return 0; }
    }
    function prunePath(obj, segs) {
      if (obj === null || typeof obj !== "object" || !segs.length) return false;
      var head = segs[0], rest = segs.slice(1), i, k;
      if (!rest.length) {
        if (head === "*") {
          var didAll = false;
          for (k in obj) {
            var szAll = byteSizeOf(obj[k]);
            if (del(obj, k)) { notePruned(szAll); didAll = true; }
          }
          return didAll;
        }
        var sz = byteSizeOf(obj[head]);
        if (del(obj, head)) { notePruned(sz); return true; }
        return false;
      }
      if (head === "[]") {
        if (!Array.isArray(obj)) return false;
        var didAny = false;
        for (i = 0; i < obj.length; i++) if (prunePath(obj[i], rest)) didAny = true;
        return didAny;
      }
      if (head === "[-]") {
        if (!Array.isArray(obj) || !obj.length) return false;
        return prunePath(obj[obj.length - 1], rest);
      }
      if (head === "*") {
        var didStar = false;
        for (k in obj) if (prunePath(obj[k], rest)) didStar = true;
        return didStar;
      }
      var next = obj[head];
      if (next === null || typeof next !== "object") return false;
      return prunePath(next, rest);
    }
    function del(obj, key) {
      if (!Object.prototype.hasOwnProperty.call(obj, key)) return false;
      try { delete obj[key]; return true; } catch (e) {}
      try { obj[key] = undefined; return true; } catch (e) {}
      return false;
    }
    function pathExists(obj, segs) {
      if (!segs.length) return true;
      if (obj === null || typeof obj !== "object") return false;
      var head = segs[0], rest = segs.slice(1), i, k;
      if (head === "[]") {
        if (!Array.isArray(obj)) return false;
        for (i = 0; i < obj.length; i++) if (pathExists(obj[i], rest)) return true;
        return false;
      }
      if (head === "[-]") {
        return Array.isArray(obj) && obj.length > 0 && pathExists(obj[obj.length - 1], rest);
      }
      if (head === "*") {
        for (k in obj) if (pathExists(obj[k], rest)) return true;
        return false;
      }
      if (!Object.prototype.hasOwnProperty.call(obj, head)) return false;
      return pathExists(obj[head], rest);
    }
    // Apply a prune rule {req, paths} to a parsed object (in
    // place); returns true when something was deleted.
    function applyPrune(obj, rule) {
      var i;
      for (i = 0; i < rule.req.length; i++) {
        if (!pathExists(obj, rule.req[i])) return false;
      }
      var did = false;
      for (i = 0; i < rule.paths.length; i++) {
        if (prunePath(obj, rule.paths[i])) { hit(rule.labels[i]); did = true; }
      }
      return did;
    }
    function parsePathList(arg) {
      var toks = String(arg || "").split(/\s+/), out = [];
      for (var i = 0; i < toks.length; i++) {
        var t = toks[i];
        if (!t || t === "important") continue;
        out.push(t);
      }
      return out;
    }
    // json-prune argument shape: arg0 = paths (or required
    // paths when arg1 is present), arg1 = paths to prune,
    // later args = needle (substring the raw JSON must
    // contain). Fetch/XHR variants: arg0 = prune paths,
    // arg1 = needle, then conditions.
    function pruneRule(args, fetchStyle) {
      var a0 = parsePathList(args[0]);
      var rule = { req: [], paths: [], labels: [], needle: null, pats: null };
      var i;
      if (fetchStyle) {
        for (i = 0; i < a0.length; i++) { rule.paths.push(a0[i].split(".")); rule.labels.push(a0[i]); }
        if (args.length > 1) { var nd = unq(args[1]); rule.needle = nd === "" ? null : nd; }
        rule.pats = condsOf(args.slice(2));
      } else if (args.length > 1 && args[1] !== undefined && args[1] !== "" && looksLikePaths(args[1])) {
        var a1 = parsePathList(args[1]);
        for (i = 0; i < a0.length; i++) rule.req.push(a0[i].split("."));
        for (i = 0; i < a1.length; i++) { rule.paths.push(a1[i].split(".")); rule.labels.push(a1[i]); }
        for (i = 2; i < args.length; i++) if (args[i] !== "important" && rule.needle === null) rule.needle = unq(args[i]);
      } else {
        for (i = 0; i < a0.length; i++) { rule.paths.push(a0[i].split(".")); rule.labels.push(a0[i]); }
        for (i = 1; i < args.length; i++) if (args[i] !== "important" && rule.needle === null) rule.needle = unq(args[i]);
      }
      return rule;
    }
    function looksLikePaths(arg) {
      var toks = String(arg || "").split(/\s+/);
      for (var i = 0; i < toks.length; i++) {
        var t = toks[i];
        if (!t || t === "important") continue;
        if (!/^[-\w$.*[\]]+$/.test(t)) return false;
      }
      return true;
    }

    // ---- window-root traps: one accessor per global name,
    // shared by every `set` / trap scriptlet rooted there. The
    // handler runs on the value already present AND on every
    // later assignment — including `var name = ...` in page
    // scripts, whose assignment fires the setter because the
    // accessor pre-exists (the mechanism uBO's set-constant
    // relies on). Traps never restore themselves away: SPA
    // navigations reassign these globals per video.
    var rootHandlers = {};
    function addRootHandler(root, fn) {
      if (rootHandlers[root]) { rootHandlers[root].push(fn); return; }
      var fns = [fn];
      rootHandlers[root] = fns;
      var cur, has = false;
      try { cur = window[root]; has = cur !== undefined; } catch (e) {}
      try {
        Object.defineProperty(window, root, {
          configurable: true, enumerable: true,
          get: function () { return cur; },
          set: function (v) {
            cur = v; has = true;
            for (var i = 0; i < fns.length; i++) { try { fns[i](v); } catch (e) {} }
          }
        });
      } catch (e) { return; }
      if (has) { try { fn(cur); } catch (e) {} }
    }
    function walkTo(obj, segs, create) {
      var cur = obj;
      for (var i = 0; i < segs.length; i++) {
        if (cur === null || (typeof cur !== "object" && typeof cur !== "function")) return null;
        var nxt = cur[segs[i]];
        if (nxt === null || (typeof nxt !== "object" && typeof nxt !== "function")) {
          if (!create) return null;
          nxt = {};
          try { cur[segs[i]] = nxt; } catch (e) { return null; }
        }
        cur = nxt;
      }
      return cur;
    }
    // AUDIT V28 (A2-H1): the pin must honor the OFF law. The
    // accessor tracks the page's own assigned value; the getter
    // serves the pinned constant only while Shield is on, and
    // the real value while off — so toggling Shield off
    // restores the property's behavior exactly, and back on
    // re-pins it, with no reload. Through v27 the `set` family
    // never consulted off() at all: pinned globals stayed
    // pinned for the document's life after toggle-off.
    //
    // The pin state lives in a shared record (WeakMap-keyed by
    // object) because applySet legitimately applies a chain
    // TWICE at install — once for the already-present root and
    // once through the root handler's initial call. A naive
    // per-call capture reads the CURRENT value on the second
    // application, which by then is the first pin's own value:
    // the record of "real" would be poisoned with the constant
    // and the pin could never yield. Re-pinning an already-
    // pinned property therefore only updates the constant in
    // the existing record; the original real value stands.
    var pinRecords = new WeakMap();
    function defineConst(obj, key, value) {
      try {
        var byKey = pinRecords.get(obj);
        if (!byKey) { byKey = new Map(); pinRecords.set(obj, byKey); }
        var rec = byKey.get(key);
        if (rec) { rec.value = value; return; }
        rec = { real: obj[key], value: value };
        Object.defineProperty(obj, key, {
          configurable: true, enumerable: true,
          get: function () { return off() ? rec.real : rec.value; },
          set: function (v) { rec.real = v; }
        });
        byKey.set(key, rec);
        return;
      } catch (e) {}
      try { obj[key] = value; } catch (e) {}
    }
    function constValue(tok) {
      switch (tok) {
        case "undefined": return undefined;
        case "null": return null;
        case "true": return true;
        case "false": return false;
        case "noopFunc": return function () {};
        case "trueFunc": return function () { return true; };
        case "falseFunc": return function () { return false; };
        case "noopPromise": return Promise.resolve();
        case "''": case '""': return "";
        case "{}": return {};
        case "[]": return [];
      }
      if (/^-?\d+(\.\d+)?$/.test(tok)) return Number(tok);
      return unq(tok);
    }
    var CHAIN_OK = /^[A-Za-z_$][\w$-]*(\.[A-Za-z_$][\w$-]*|\[\d+\])*$/;
    function chainSegs(chain) {
      return String(chain || "").replace(/\[(\d+)\]/g, ".$1").split(".");
    }
    function applySet(chain, value) {
      if (!CHAIN_OK.test(chain)) return;
      var segs = chainSegs(chain);
      var leaf = segs[segs.length - 1];
      if (segs.length === 1) { defineConst(window, leaf, value); return; }
      var root = segs[0], mid = segs.slice(1, -1);
      var applyTo = function (rootObj) {
        // A2-H1: while off, do not even create the intermediate
        // objects — installing structure is a side effect too.
        if (off()) return;
        var parent = mid.length ? walkTo(rootObj, mid, true) : rootObj;
        if (parent) defineConst(parent, leaf, value);
      };
      if (root === "window") { applyTo(window); return; }
      // Apply now if the root exists, and re-apply on every
      // (re)assignment of the root global.
      try { var existing = window[root]; if (existing !== undefined) applyTo(existing); } catch (e) {}
      addRootHandler(root, applyTo);
    }
    function applyTrap(chain, kind, needle) {
      // kind: "aopr" (getter throws), "aopw" (setter throws),
      // "acs" (getter throws when an inline script reads).
      if (!CHAIN_OK.test(chain)) return;
      var segs = chainSegs(chain);
      var leaf = segs[segs.length - 1];
      var install = function (parent) {
        if (!parent) return;
        var orig;
        try { orig = parent[leaf]; } catch (e) { orig = undefined; }
        var shouldThrow = function () {
          if (off()) return false;
          if (kind === "acs") {
            try {
              var cs = document.currentScript;
              if (!cs || cs.src) return false;
              if (!needle) return true;
              return String(cs.textContent || "").indexOf(needle) !== -1;
            } catch (e) { return false; }
          }
          if (kind === "aopr" && needle) {
            try { return String(new Error().stack || "").indexOf(needle) !== -1; } catch (e) { return false; }
          }
          return true;
        };
        try {
          if (kind === "aopw") {
            Object.defineProperty(parent, leaf, {
              configurable: true, enumerable: true,
              get: function () { return orig; },
              set: function () { if (shouldThrow()) throw new Error("pico shield"); }
            });
          } else {
            Object.defineProperty(parent, leaf, {
              configurable: true, enumerable: true,
              get: function () { if (shouldThrow()) throw new Error("pico shield"); return orig; },
              set: function (v) { orig = v; }
            });
          }
        } catch (e) {}
      };
      if (segs.length === 1) { install(window); return; }
      var root = segs[0], mid = segs.slice(1, -1);
      var applyTo = function (rootObj) { install(mid.length ? walkTo(rootObj, mid, false) : rootObj); };
      if (root === "window") { applyTo(window); return; }
      try { var existing = window[root]; if (existing !== undefined) applyTo(existing); } catch (e) {}
      addRootHandler(root, applyTo);
    }

    // ---- text replace (trusted-rpfr / trusted-replace-xhr):
    // needle is a /regex/flags (its `$1` backrefs work through
    // String.replace), a quoted literal (split/join), or a bare
    // token treated as a regex source (uBO's `player?` form).
    function replaceRule(args) {
      var needle = args[0] || "", repl = unq(args[1] || "");
      if (!needle) return { re: null, lit: "", repl: repl, pats: [], label: "replace:" };
      var re = asRegex(needle), lit = null;
      if (!re) {
        if (needle.length >= 2 && (needle.charAt(0) === "'" || needle.charAt(0) === '"')) lit = unq(needle);
        else { try { re = new RegExp(needle, "g"); } catch (e) { lit = needle; } }
      }
      return { re: re, lit: lit, repl: repl, pats: condsOf(args.slice(2)), label: "replace:" + unq(needle).slice(0, 48) };
    }
    function applyReplace(text, rule) {
      var out = text;
      if (rule.re) { rule.re.lastIndex = 0; out = text.replace(rule.re, rule.repl); }
      else if (rule.lit !== null && rule.lit !== "") out = text.split(rule.lit).join(rule.repl);
      if (out !== text) hit(rule.label);
      return out;
    }
    // A no-*-if pattern: /regex/flags | substring, with an
    // optional trailing ` method:POST` condition.
    function blockRule(arg) {
      var a = String(arg || ""), method = null;
      var mi = a.lastIndexOf(" method:");
      if (mi > 0) { method = a.slice(mi + 8).trim().toUpperCase(); a = a.slice(0, mi); }
      var p = patOf(a);
      return { pat: p, method: method };
    }
    function blockMatch(rule, url, method) {
      if (!rule.pat) return false;
      if (rule.method && method && rule.method !== method) return false;
      return patMatch(rule.pat, url);
    }

    // ---- dispatch: sort the entries into rule lists, install
    // traps immediately, and wrap fetch/XHR/JSON.parse once.
    var jPrune = [], fPrune = [], xPrune = [], fReplace = [], xReplace = [];
    var fBlock = [], xBlock = [], domJobs = [];
    for (var ei = 0; ei < entries.length; ei++) {
      var en = entries[ei], ea = en.a || [];
      try {
        switch (en.n) {
          case "set": applySet(ea[0], constValue(ea[1] === undefined ? "undefined" : ea[1])); break;
          case "aopr": applyTrap(ea[0], "aopr", ea[1] !== undefined ? unq(ea[1]) : null); break;
          case "aopw": applyTrap(ea[0], "aopw", null); break;
          case "acs": applyTrap(ea[0], "acs", ea[1] !== undefined ? unq(ea[1]) : null); break;
          case "json-prune": jPrune.push(pruneRule(ea, false)); break;
          case "json-prune-fetch-response": fPrune.push(pruneRule(ea, true)); break;
          case "json-prune-xhr-response": xPrune.push(pruneRule(ea, true)); break;
          case "trusted-rpfr": fReplace.push(replaceRule(ea)); break;
          case "trusted-replace-xhr-response": xReplace.push(replaceRule(ea)); break;
          case "no-fetch-if": fBlock.push(blockRule(ea[0])); break;
          case "no-xhr-if": xBlock.push(blockRule(ea[0])); break;
          case "ra": domJobs.push({ attrs: String(ea[0] || "").split("|"), sel: ea[1] ? unq(ea[1]) : null, val: null }); break;
          case "set-attr": if (ea[1]) domJobs.push({ attrs: [unq(ea[1])], sel: ea[0] ? unq(ea[0]) : null, val: ea[2] !== undefined ? unq(ea[2]) : "" }); break;
        }
      } catch (e) { /* one bad entry never stops the rest */ }
    }

    // json-prune: intercept JSON.parse.
    if (jPrune.length && typeof JSON !== "undefined" && JSON.parse) {
      var origParse = JSON.parse;
      JSON.parse = function (text) {
        var out = origParse.apply(this, arguments);
        try {
          if (!off() && typeof text === "string" && out !== null && typeof out === "object") {
            for (var i = 0; i < jPrune.length; i++) {
              var r = jPrune[i];
              if (r.needle !== null && text.indexOf(r.needle) === -1) continue;
              try { applyPrune(out, r); } catch (e) {}
            }
          }
        } catch (e) {}
        return out;
      };
    }

    // fetch: block-listed requests resolve to an empty 200;
    // matching responses are pruned/replaced on a CLONE (the
    // original stream is never consumed by the wrapper), and
    // any failure returns the original response untouched.
    function fetchUrl(input) {
      try {
        if (typeof input === "string") return input;
        if (input && typeof input.url === "string") return input.url;
      } catch (e) {}
      return "";
    }
    function fetchMethod(input, init) {
      try {
        var m = (init && init.method) || (input && input.method) || "GET";
        return String(m).toUpperCase();
      } catch (e) { return "GET"; }
    }
    function transformText(text, url, prunes, replaces) {
      var out = text, i, r;
      for (i = 0; i < replaces.length; i++) {
        r = replaces[i];
        if (condsMatch(r.pats, url)) { try { out = applyReplace(out, r); } catch (e) {} }
      }
      for (i = 0; i < prunes.length; i++) {
        r = prunes[i];
        if (!condsMatch(r.pats, url)) continue;
        if (r.needle !== null && out.indexOf(r.needle) === -1) continue;
        try {
          // AUDIT V28 (A2-L11): parse with the ORIGINAL
          // JSON.parse. Calling the wrapped one re-ran every
          // host json-prune rule inside each fetch/XHR
          // transform — idempotent deletes, but every hit was
          // counted twice in the prune telemetry.
          var rawParse = (typeof origParse === "function") ? origParse : JSON.parse;
          var obj = rawParse(out);
          if (obj !== null && typeof obj === "object" && applyPrune(obj, r)) out = JSON.stringify(obj);
        } catch (e) {}
      }
      return out;
    }
    if ((fPrune.length || fReplace.length || fBlock.length) && typeof window.fetch === "function") {
      var origFetch = window.fetch;
      window.fetch = function (input, init) {
        try {
          if (!off()) {
            var url = fetchUrl(input), method = fetchMethod(input, init), i;
            for (i = 0; i < fBlock.length; i++) {
              if (blockMatch(fBlock[i], url, method)) {
                if (typeof Response === "function") {
                  return Promise.resolve(new Response("", { status: 200, statusText: "OK", headers: { "Content-Type": "text/plain" } }));
                }
                break;
              }
            }
            if (fPrune.length || fReplace.length) {
              var p = origFetch.apply(this, arguments);
              return Promise.resolve(p).then(function (resp) {
                try {
                  // AUDIT V28 (A2-L10): re-check the OFF flag at
                  // completion — a response that was in flight
                  // when the user toggled Shield off is delivered
                  // untouched.
                  if (off()) return resp;
                  var relevant = false, j;
                  for (j = 0; j < fReplace.length && !relevant; j++) relevant = condsMatch(fReplace[j].pats, url);
                  for (j = 0; j < fPrune.length && !relevant; j++) relevant = condsMatch(fPrune[j].pats, url);
                  if (!relevant || !resp || typeof resp.clone !== "function") return resp;
                  var cl = resp.clone();
                  return cl.text().then(function (text) {
                    try {
                      if (off()) return resp;
                      var out = transformText(text, url, fPrune, fReplace);
                      if (out !== text && typeof Response === "function") {
                        // AUDIT V28 (A2-L9): never inherit the
                        // original Content-Length — the prune
                        // changed the body's length. (The
                        // replacement's `url` is empty and
                        // cannot be set from script; that part
                        // is a platform limit, documented.)
                        var hdrs = resp.headers;
                        try {
                          var nh = new Headers(resp.headers);
                          nh.delete("content-length");
                          hdrs = nh;
                        } catch (e) {}
                        return new Response(out, { status: resp.status, statusText: resp.statusText, headers: hdrs });
                      }
                    } catch (e) {}
                    return resp;
                  }, function () { return resp; });
                } catch (e) { return resp; }
              });
            }
            return origFetch.apply(this, arguments);
          }
        } catch (e) {}
        return origFetch.apply(this, arguments);
      };
    }

    // XMLHttpRequest: open() records url+method; send() arms a
    // completion hook for matching requests. Text responses are
    // transformed and shadowed as own properties; `json`
    // responses are pruned in place (same object the page will
    // read). Block-listed requests never reach the network:
    // the instance is completed synthetically, empty.
    if ((xPrune.length || xReplace.length || xBlock.length) && typeof window.XMLHttpRequest === "function") {
      var XP = window.XMLHttpRequest.prototype;
      var origOpen = XP.open, origSend = XP.send;
      XP.open = function (method, url) {
        // AUDIT 0.30.1 (A2-C2): open() begins a NEW request on
        // this instance — retire whatever a previous request
        // installed on it (the A2-M6 getter gate, the fallback
        // path's shadowed response properties, a blocked
        // request's synthetic state) and bump the generation
        // the fallback hooks check. Through 0.30 the gate was
        // never removed: a page reusing one XHR object for a
        // second, non-relevant request kept reading the FIRST
        // response's transformed text through the stale
        // per-instance getters, whose cache is keyed to the
        // first request's URL.
        try {
          resetXhrInstance(this);
          this.__picoXhrGen = (this.__picoXhrGen || 0) + 1;
        } catch (e) {}
        try { this.__picoXhrUrl = String(url || ""); this.__picoXhrMethod = String(method || "GET").toUpperCase(); } catch (e) {}
        return origOpen.apply(this, arguments);
      };
      XP.send = function () {
        var self = this;
        try {
          if (!off()) {
            var url = self.__picoXhrUrl || "", method = self.__picoXhrMethod || "GET", i;
            for (i = 0; i < xBlock.length; i++) {
              if (blockMatch(xBlock[i], url, method)) { fakeEmptyXhr(self); return undefined; }
            }
            var relevant = false, j;
            for (j = 0; j < xReplace.length && !relevant; j++) relevant = condsMatch(xReplace[j].pats, url);
            for (j = 0; j < xPrune.length && !relevant; j++) relevant = condsMatch(xPrune[j].pats, url);
            if (relevant) {
              // AUDIT V28 (A2-M6): the transform must be visible
              // to page handlers registered BEFORE send() — a
              // readystatechange hook registered inside send()
              // runs after them (dispatch is registration
              // ordered), so the classic jQuery/raw-XHR pattern
              // read the untransformed body. The gate therefore
              // intercepts the READS: per-instance getters over
              // responseText/response transform lazily on first
              // read at readyState 4, which is after ANY handler
              // ordering. The event hook remains only as a
              // fallback for hosts where the getters cannot be
              // installed.
              if (!installXhrGate(self, url)) {
                var doneFlag = false;
                // AUDIT 0.30.1 (A2-C2): the fallback hook is
                // armed for THIS request only — open() bumps
                // the instance generation, and a hook left
                // over from a previous request on a reused
                // instance must never transform (or shadow)
                // the new response under the old URL.
                var hookGen = 0;
                try { hookGen = self.__picoXhrGen || 0; } catch (e) {}
                var hook = function () {
                  if (doneFlag) return;
                  try { if ((self.__picoXhrGen || 0) !== hookGen) return; } catch (e) {}
                  var rs = 0;
                  try { rs = self.readyState; } catch (e) {}
                  if (rs !== 4) return;
                  doneFlag = true;
                  try { transformXhr(self, url); } catch (e) {}
                };
                // AUDIT 0.31 (A2-F7): keep the fallback listener
                // refs on the instance so resetXhrInstance (from
                // the next open()) can removeEventListener them —
                // through 0.30.1 they leaked onto reused instances
                // and were disarmed only by their generation check.
                try { self.__picoFbHooks = { rs: hook, load: hook }; } catch (e) {}
                try { self.addEventListener("readystatechange", hook); self.addEventListener("load", hook); } catch (e) {}
              }
            }
          }
        } catch (e) {}
        return origSend.apply(this, arguments);
      };
      function shadow(xhr, prop, val) {
        try { xhr.__picoXhrTouched = 1; } catch (e) {}
        try { Object.defineProperty(xhr, prop, { configurable: true, enumerable: true, writable: true, value: val }); } catch (e) {}
      }
      // AUDIT 0.30.1 (A2-C2): remove everything a previous
      // request installed on this instance — the getter
      // gate's own accessors and every shadow()ed property.
      // All of Pico's own properties are defined
      // configurable, so `delete` restores the prototype's
      // native accessors; a property Pico never touched is
      // not the instance's to lose (the touched marker is
      // set by shadow() and installXhrGate, the only two
      // installers). Called from open(), where the platform
      // itself resets the response state.
      function resetXhrInstance(xhr) {
        try {
          // AUDIT 0.31 (A2-F7): the fallback hooks come off
          // FIRST and unconditionally — installing them never
          // set the touched marker, so the old early return
          // below would have skipped their removal entirely.
          try {
            var fb = xhr.__picoFbHooks;
            if (fb) {
              if (fb.rs) { try { xhr.removeEventListener("readystatechange", fb.rs); } catch (e) {} }
              if (fb.load) { try { xhr.removeEventListener("load", fb.load); } catch (e) {} }
              try { delete xhr.__picoFbHooks; } catch (e) { try { xhr.__picoFbHooks = undefined; } catch (e) {} }
            }
          } catch (e) {}
          if (!xhr.__picoXhrTouched) return;
          var props = ["responseText", "response", "responseXML", "readyState", "status", "statusText"];
          for (var i = 0; i < props.length; i++) { try { delete xhr[props[i]]; } catch (e) {} }
          xhr.__picoXhrTouched = 0;
        } catch (e) {}
      }
      // A2-M6 getter gate (see the send() call site). Returns
      // false when the prototype exposes no accessor to shadow
      // (exotic hosts); the caller then falls back to the event
      // hook. The transform is computed once per request and
      // cached on the instance's closure.
      function installXhrGate(xhr, url) {
        var dText, dResp;
        try {
          dText = Object.getOwnPropertyDescriptor(XP, "responseText");
          dResp = Object.getOwnPropertyDescriptor(XP, "response");
        } catch (e) { return false; }
        if (!dText || !dText.get || !dResp || !dResp.get) return false;
        var cache = { done: false, text: null, jsonDone: false };
        function gatedText(raw) {
          if (typeof raw !== "string") return raw;
          if (!cache.done) { cache.text = transformText(raw, url, xPrune, xReplace); cache.done = true; }
          return cache.text;
        }
        try {
          Object.defineProperty(xhr, "responseText", {
            configurable: true, enumerable: true,
            get: function () {
              var raw = dText.get.call(xhr);
              try {
                if (off() || xhr.readyState !== 4) return raw;
                var rt = xhr.responseType || "";
                if (rt !== "" && rt !== "text") return raw;
                return gatedText(raw);
              } catch (e) { return raw; }
            }
          });
          // AUDIT 0.31 (A2-F6): mark the instance touched
          // IMMEDIATELY after the FIRST defineProperty succeeds
          // — if the second define throws, the partial gate
          // must still be tear-down-able by resetXhrInstance,
          // which keys entirely off this marker.
          try { xhr.__picoXhrTouched = 1; } catch (e) {}
          Object.defineProperty(xhr, "response", {
            configurable: true, enumerable: true,
            get: function () {
              var rt = "";
              try { rt = xhr.responseType || ""; } catch (e) {}
              if (rt === "json") {
                var obj = dResp.get.call(xhr);
                try {
                  if (!off() && xhr.readyState === 4 && obj !== null && typeof obj === "object" && !cache.jsonDone) {
                    cache.jsonDone = true;
                    for (var i = 0; i < xPrune.length; i++) {
                      var r = xPrune[i];
                      if (!condsMatch(r.pats, url)) continue;
                      if (r.needle !== null) { try { if (JSON.stringify(obj).indexOf(r.needle) === -1) continue; } catch (e) {} }
                      try { applyPrune(obj, r); } catch (e) {}
                    }
                  }
                } catch (e) {}
                return obj;
              }
              var raw = dResp.get.call(xhr);
              try {
                if (off() || xhr.readyState !== 4) return raw;
                if (rt !== "" && rt !== "text") return raw;
                return gatedText(raw);
              } catch (e) { return raw; }
            }
          });
          try { xhr.__picoXhrTouched = 1; } catch (e) {}
          return true;
        } catch (e) { return false; }
      }
      function transformXhr(xhr, url) {
        // A2-L10: the fallback path also honors a toggle that
        // landed while the request was in flight.
        if (off()) return;
        var rt = "";
        try { rt = xhr.responseType || ""; } catch (e) {}
        if (rt === "" || rt === "text") {
          var text;
          try { text = xhr.responseText; } catch (e) { return; }
          if (typeof text !== "string") return;
          var out = transformText(text, url, xPrune, xReplace);
          if (out !== text) { shadow(xhr, "responseText", out); shadow(xhr, "response", out); }
        } else if (rt === "json") {
          var obj;
          try { obj = xhr.response; } catch (e) { return; }
          if (obj !== null && typeof obj === "object") {
            for (var i = 0; i < xPrune.length; i++) {
              var r = xPrune[i];
              if (!condsMatch(r.pats, url)) continue;
              if (r.needle !== null) { try { if (JSON.stringify(obj).indexOf(r.needle) === -1) continue; } catch (e) {} }
              try { applyPrune(obj, r); } catch (e) {}
            }
          }
        }
      }
      function fakeEmptyXhr(xhr) {
        try {
          shadow(xhr, "readyState", 4);
          shadow(xhr, "status", 200);
          shadow(xhr, "statusText", "OK");
          shadow(xhr, "responseText", "");
          shadow(xhr, "response", "");
          shadow(xhr, "responseXML", null);
          var fire = function (type) {
            try {
              var ev = null;
              try { ev = new Event(type); } catch (e) { ev = { type: type }; }
              if (typeof xhr.dispatchEvent === "function") xhr.dispatchEvent(ev);
              var h = xhr["on" + type];
              if (typeof h === "function") h.call(xhr, ev);
            } catch (e) {}
          };
          // AUDIT 0.31 (A2-F7): capture the instance generation
          // at scheduling and re-check it in the callback —
          // mirror of the fallback hook's check. If open() has
          // reused this instance for a new request in the
          // meantime, the synthetic completion of the BLOCKED
          // request must not fire into the new one.
          var fakeGen = 0;
          try { fakeGen = xhr.__picoXhrGen || 0; } catch (e) {}
          setTimeout(function () {
            try { if ((xhr.__picoXhrGen || 0) !== fakeGen) return; } catch (e) {}
            fire("readystatechange"); fire("load"); fire("loadend");
          }, 0);
        } catch (e) {}
      }
    }

    // remove-attr / set-attr: applied now, at DOMContentLoaded,
    // and on DOM mutations (guarded so our own writes cannot
    // loop: a write happens only when the value differs).
    if (domJobs.length) {
      var runDomJobs = function () {
        if (off()) return;
        for (var i = 0; i < domJobs.length; i++) {
          var job = domJobs[i];
          try {
            for (var a = 0; a < job.attrs.length; a++) {
              var attr = job.attrs[a];
              var els = job.sel ? document.querySelectorAll(job.sel) : document.querySelectorAll("[" + attr + "]");
              for (var j = 0; j < els.length; j++) {
                var el = els[j];
                if (job.val === null) { if (el.hasAttribute(attr)) el.removeAttribute(attr); }
                else if (el.getAttribute(attr) !== job.val) el.setAttribute(attr, job.val);
              }
            }
          } catch (e) {}
        }
      };
      runDomJobs();
      try {
        document.addEventListener("DOMContentLoaded", runDomJobs);
        if (typeof MutationObserver === "function" && document.documentElement) {
          var domTimer = 0;
          var mo = new MutationObserver(function () {
            if (domTimer) return;
            domTimer = setTimeout(function () { domTimer = 0; runDomJobs(); }, 250);
          });
          mo.observe(document.documentElement, { childList: true, subtree: true, attributes: true });
        }
      } catch (e) {}
    }
  } catch (e) { /* never break the page */ }
})();
"#;

// ---------------------------------------------------------------------------
// Decisions + shared runtime state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
}

impl Decision {
    pub fn is_block(self) -> bool {
        matches!(self, Decision::Block)
    }
}

/// The neutered response for a blocked request: images get the
/// transparent GIF; everything else an empty body (scripts get
/// an empty script). Never an error page for subresources.
pub fn neutered_body(rtype: ResourceType) -> (Option<&'static [u8]>, &'static str) {
    match rtype {
        ResourceType::Image => (Some(TRANSPARENT_GIF), "image/gif"),
        ResourceType::Script => (None, "text/javascript"),
        ResourceType::Stylesheet => (None, "text/css"),
        _ => (None, "text/plain"),
    }
}

/// Cap on individually-logged block lines per session (mirrors
/// the media probe's discipline: classify, don't mirror).
pub const SHIELD_LOG_SAMPLE: usize = 20;

/// Cap on individually-logged prune lines per session (v23):
/// the scriptlet payload reports per-path prune counts; the
/// app logs a sample, never a mirror.
pub const SHIELD_PRUNE_LOG_SAMPLE: usize = 20;

/// Runtime state shared between the app and every content
/// WebView's request handler. Handlers read it lock-cheaply and
/// never wait on compilation: until the engine lands (and
/// whenever the Shield is off) the answer is Allow — safety
/// law 3 at the wiring level.
pub struct ShieldRuntime {
    engine: RwLock<Option<std::sync::Arc<ShieldEngine>>>,
    script: RwLock<Option<std::sync::Arc<String>>>,
    pub on: AtomicBool,
    pub session_blocked: AtomicU64,
    per_tab: Mutex<HashMap<u64, u64>>,
    log_sample_left: Mutex<usize>,
    prune_log_left: Mutex<usize>,
    /// Per-tab page state for party/`domain=` context and the
    /// main-frame determination: the last committed URL and the
    /// last main-frame navigation target (from NavigationStarting,
    /// which WebView2 raises for the main frame only).
    page_urls: Mutex<HashMap<u64, (String, String)>>,
}

impl Default for ShieldRuntime {
    fn default() -> Self {
        Self::new(true)
    }
}

impl ShieldRuntime {
    pub fn new(on: bool) -> Self {
        ShieldRuntime {
            engine: RwLock::new(None),
            script: RwLock::new(None),
            on: AtomicBool::new(on),
            session_blocked: AtomicU64::new(0),
            per_tab: Mutex::new(HashMap::new()),
            log_sample_left: Mutex::new(SHIELD_LOG_SAMPLE),
            prune_log_left: Mutex::new(SHIELD_PRUNE_LOG_SAMPLE),
            page_urls: Mutex::new(HashMap::new()),
        }
    }

    pub fn install(&self, engine: ShieldEngine, script: String) {
        if let Ok(mut slot) = self.script.write() {
            *slot = Some(std::sync::Arc::new(script));
        }
        if let Ok(mut slot) = self.engine.write() {
            *slot = Some(std::sync::Arc::new(engine));
        }
    }

    /// The combined document-start script (scriptlets +
    /// cosmetics), built once at compile time.
    pub fn document_script(&self) -> Option<std::sync::Arc<String>> {
        self.script.read().ok().and_then(|s| s.clone())
    }

    /// A snapshot of the compiled engine, when it exists
    /// (AUDIT V26, D6): the unified filter pipeline
    /// (`core::filterpipe`) takes the engine as a value so the
    /// one decision site owns the Shield/Lite composition;
    /// through v25 the runtime's `should_block` hid the engine
    /// and the handler composed Lite around its answer.
    pub fn engine_snapshot(&self) -> Option<std::sync::Arc<ShieldEngine>> {
        self.engine.read().ok().and_then(|s| s.clone())
    }

    /// Consume one prune-log sample slot. The app logs a
    /// `shield prune:` line only while this returns true.
    pub fn claim_prune_log(&self) -> bool {
        let mut left = self
            .prune_log_left
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if *left > 0 {
            *left -= 1;
            true
        } else {
            false
        }
    }

    // AUDIT V28 (A2-M2): the v25-era `should_block` one-call
    // seam was deleted. The unified pipeline (filterpipe) has
    // owned the handler's decision since v26, leaving
    // should_block with zero production callers — while its
    // tests kept asserting behavior (POST blocking) the real
    // browser never performed. Decisions live in
    // filterpipe::decide; the main-frame reclassification it
    // used to bundle is the pure `effective_rtype` below, used
    // by the handler itself.

    /// Record a block: bump session + per-tab counters, and say
    /// whether this block may consume a log-sample slot.
    pub fn record_block(&self, tab: u64) -> (u64, u64, bool) {
        let total = self.session_blocked.fetch_add(1, Ordering::Relaxed) + 1;
        let tab_count = {
            let mut map = self.per_tab.lock().unwrap_or_else(|p| p.into_inner());
            let e = map.entry(tab).or_insert(0);
            *e += 1;
            *e
        };
        let log_it = {
            let mut left = self
                .log_sample_left
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if *left > 0 {
                *left -= 1;
                true
            } else {
                false
            }
        };
        (total, tab_count, log_it)
    }

    pub fn tab_blocked(&self, tab: u64) -> u64 {
        self.per_tab
            .lock()
            .map(|m| m.get(&tab).copied().unwrap_or(0))
            .unwrap_or(0)
    }

    pub fn set_page_url(&self, tab: u64, committed: &str) {
        if let Ok(mut map) = self.page_urls.lock() {
            let e = map
                .entry(tab)
                .or_insert_with(|| (String::new(), String::new()));
            e.0 = committed.to_string();
        }
    }

    pub fn set_pending_main(&self, tab: u64, url: &str) {
        if let Ok(mut map) = self.page_urls.lock() {
            let e = map
                .entry(tab)
                .or_insert_with(|| (String::new(), String::new()));
            e.1 = url.to_string();
        }
    }

    /// (committed page URL, pending main-frame target) for a tab.
    pub fn page_state(&self, tab: u64) -> (String, String) {
        self.page_urls
            .lock()
            .map(|m| m.get(&tab).cloned().unwrap_or_default())
            .unwrap_or_else(|_| (String::new(), String::new()))
    }

    pub fn forget_tab(&self, tab: u64) {
        if let Ok(mut map) = self.page_urls.lock() {
            map.remove(&tab);
        }
        if let Ok(mut map) = self.per_tab.lock() {
            map.remove(&tab);
        }
    }
}
