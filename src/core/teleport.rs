//! Phone Teleport resolution logic (RESEARCH_v25.md §b, §c).
//!
//! Everything here is pure: the app layer enumerates candidate
//! LAN addresses (platform code) and asks this module two
//! questions — which address should represent this PC on the LAN
//! ([`choose_lan_ip`]), and what URL (if any) should the phone be
//! sent to for a given tab URL ([`resolve`]). The QR encoding of
//! the outcome lives in [`crate::core::qr`].

use std::net::Ipv4Addr;

/// What kind of adapter a LAN candidate sits on
/// (AUDIT 0.31, A4-F2). The chooser treats kinds differently:
/// a tunnel/VPN egress must not outrank a real physical
/// adapter the phone can actually reach on the local Wi-Fi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanKind {
    Physical,
    Tunnel,
    Virtual,
    Other,
}

impl LanKind {
    /// Chooser preference rank: physical highest, tunnel
    /// lowest (a tunnel is the least likely to be the LAN the
    /// phone is on).
    fn rank(self) -> u8 {
        match self {
            LanKind::Physical => 3,
            LanKind::Other => 2,
            LanKind::Virtual => 1,
            LanKind::Tunnel => 0,
        }
    }
}

/// Map a Windows `IfType` value to a [`LanKind`]
/// (AUDIT 0.31, A4-F2): 6 (Ethernet) and 71 (IEEE 802.11
/// wireless) are physical; 131 is a tunnel; 53 is
/// proprietary virtual; 24 (loopback) and everything else
/// are Other.
pub fn lan_kind_for_if_type(if_type: u32) -> LanKind {
    match if_type {
        6 | 71 => LanKind::Physical,
        131 => LanKind::Tunnel,
        53 => LanKind::Virtual,
        _ => LanKind::Other,
    }
}

/// One enumerated unicast address plus its adapter's state, as
/// the platform layer reports it. Only address + up/down ever
/// cross into core — adapter names and descriptions stay in the
/// platform layer and are never logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanCandidate {
    pub ip: Ipv4Addr,
    pub up: bool,
    /// Adapter kind (AUDIT 0.31, A4-F2).
    pub kind: LanKind,
    /// The address's DAD state was Duplicate — the address is
    /// contested on the LAN and must never be chosen
    /// (AUDIT 0.31, A4-F2). The platform layer also excludes
    /// these at enumeration; the chooser filters again so the
    /// rule holds for any candidate source.
    pub dad_duplicate: bool,
}

/// RFC1918 preference tier: 192.168/16 beats 10/8 beats
/// 172.16/12 beats anything else (a public/CGNAT address can
/// still be the LAN address on some routers, so it ranks last
/// rather than being dropped).
fn rfc1918_tier(ip: &Ipv4Addr) -> u8 {
    let o = ip.octets();
    if o[0] == 192 && o[1] == 168 {
        3
    } else if o[0] == 10 {
        2
    } else if o[0] == 172 && (16..=31).contains(&o[1]) {
        1
    } else {
        0
    }
}

/// Is this address unusable as "this PC on the LAN"? Loopback,
/// the unspecified address, and link-local APIPA (169.254/16)
/// can never receive the phone's connection in practice.
fn unusable(ip: &Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || ip.is_link_local()
}

/// Choose the single LAN IPv4 that represents this PC, or `None`
/// when no candidate survives filtering.
///
/// AUDIT V28 (A4-M1): through v27 the preference key was
/// (tier, up, enumeration order) — the RFC1918 tier DOMINATED
/// the up flag, so a down/disconnected 192.168 Ethernet adapter
/// beat the live 10.x Wi-Fi the phone could actually reach, and
/// the UDP egress probe (the OS's own route answer) was only
/// consulted when enumeration found nothing at all. Now: an
/// egress address that appears among the usable candidates
/// wins outright — it is the route the OS itself would use —
/// and the remaining key is (up, tier, enumeration order).
pub fn choose_lan_ip_with_egress(
    candidates: &[LanCandidate],
    egress: Option<Ipv4Addr>,
) -> Option<Ipv4Addr> {
    // AUDIT 0.31 (A4-F2): DAD-duplicate candidates are
    // excluded outright, here as well as at enumeration.
    let usable = |c: &LanCandidate| !unusable(&c.ip) && !c.dad_duplicate;
    if let Some(eg) = egress {
        if let Some(egress_cand) = candidates.iter().find(|c| c.ip == eg && usable(c)) {
            // AUDIT 0.31 (A4-F2): a PHYSICAL egress still wins
            // outright. A non-physical egress (above all a
            // tunnel) does NOT beat a physical candidate —
            // fall through to the ranked choice, whose key
            // puts physical adapters first. When no physical
            // candidate exists, the egress keeps its old
            // outright win: it is still the OS's route.
            if egress_cand.kind == LanKind::Physical
                || !candidates
                    .iter()
                    .any(|c| usable(c) && c.kind == LanKind::Physical)
            {
                return Some(eg);
            }
        }
    }
    candidates
        .iter()
        .filter(|c| usable(c))
        .enumerate()
        .max_by_key(|(idx, c)| {
            (
                c.up,
                c.kind.rank(),
                rfc1918_tier(&c.ip),
                // Earlier enumeration wins ties: invert the index
                // so max_by_key prefers the smaller one.
                std::cmp::Reverse(*idx),
            )
        })
        .map(|(_, c)| c.ip)
}

/// [`choose_lan_ip_with_egress`] without an egress hint.
pub fn choose_lan_ip(candidates: &[LanCandidate]) -> Option<Ipv4Addr> {
    choose_lan_ip_with_egress(candidates, None)
}

/// Fallback when adapter enumeration yields nothing: ask the OS
/// which local address it would use to reach the public internet
/// — a UDP "connect" sends no packets, it only resolves the
/// route. Returns `None` when there is no route at all.
pub fn udp_egress_ip() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if !unusable(&v4) => Some(v4),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// URL resolution
// ---------------------------------------------------------------------------

/// What Teleport decided for one tab URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeleportKind {
    /// Public http(s): the phone opens this exact page.
    PublicUrl,
    /// Loopback dev URL rewritten onto the LAN address.
    LanRewrite,
    /// Loopback URL but no LAN address exists.
    NoNetwork,
    /// This page cannot leave the PC (internal/file pages).
    NotTeleportable,
}

impl TeleportKind {
    /// Wire/log label.
    pub fn label(self) -> &'static str {
        match self {
            TeleportKind::PublicUrl => "public",
            TeleportKind::LanRewrite => "lan-rewrite",
            TeleportKind::NoNetwork => "no-network",
            TeleportKind::NotTeleportable => "not-teleportable",
        }
    }
}

/// The full resolution: the outcome kind, the URL to show/encode
/// (`None` only for NotTeleportable, where there is nothing
/// meaningful to put in front of the user), and the panel note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Teleport {
    pub kind: TeleportKind,
    pub url: Option<String>,
    pub note: String,
}

impl Teleport {
    /// The URL that may be encoded into the QR — a stricter
    /// notion than the display URL. AUDIT V28 (A4-L5): through
    /// v27 the NoNetwork case carried the original loopback URL
    /// as its `url`, and the app encoded any `Some` — the panel
    /// showed "no local network found" beside a scannable QR of
    /// the phone's own localhost. Only a LAN rewrite or a
    /// public URL is encodable; the display field may still
    /// show the original for the other kinds.
    pub fn qr_url(&self) -> Option<&str> {
        match self.kind {
            TeleportKind::LanRewrite | TeleportKind::PublicUrl => self.url.as_deref(),
            TeleportKind::NoNetwork | TeleportKind::NotTeleportable => None,
        }
    }
}

const NOTE_PUBLIC: &str = "Opens this exact page on your phone.";
const NOTE_LAN: &str = "Phone must be on the same Wi-Fi. If it won't load, your dev server is bound to this PC only \u{2014} restart it with --host (Vite) / -H 0.0.0.0 (Next) and re-open.";
const NOTE_NO_NETWORK: &str =
    "No local network found \u{2014} this link only works on this PC right now.";
const NOTE_INTERNAL: &str = "Pico's own pages stay on this PC \u{2014} there's nothing to send.";
const NOTE_PICO_SERVED: &str =
    "Files Pico serves itself stay on this PC \u{2014} its built-in server is local-only by design.";
const NOTE_SCHEME: &str = "This kind of page can't be opened on a phone from here.";

/// Is `host` (already extracted, brackets removed) a loopback
/// name/address? The definition itself is the ONE in
/// `shield::is_local_host` — through 0.31 this file kept a
/// private copy, and the two drifted apart and were re-aligned
/// three times (127/8, then 0.0.0.0, then trailing dots). This
/// call site only normalizes first: `split_url` does not
/// lowercase or trim trailing dots the way `shield::host_of`
/// does for the filter layers (`localhost.` and `LOCALHOST`
/// are this machine too).
fn is_loopback_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    let h = h.trim_end_matches('.');
    crate::core::shield::is_local_host(h)
}

/// Split an absolute URL into (scheme, authority, host, rest),
/// where `rest` begins at the first `/`, `?` or `#` after the
/// authority (possibly empty) and `host` excludes userinfo, port
/// and IPv6 brackets. `None` when the shape isn't scheme://…
fn split_url(url: &str) -> Option<(&str, &str, String, &str)> {
    let (scheme, rest0) = url.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let end = rest0
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest0.len());
    let authority = &rest0[..end];
    let rest = &rest0[end..];
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split(']').next().unwrap_or(inner).to_string()
    } else {
        host_port.split(':').next().unwrap_or(host_port).to_string()
    };
    Some((scheme, authority, host, rest))
}

/// Rebuild the URL with its host replaced by `ip`, preserving
/// scheme, port, path, query and fragment byte-for-byte.
/// Userinfo is dropped (a dev URL carrying credentials is not
/// something to bake into a QR on screen).
fn with_lan_host(url: &str, ip: Ipv4Addr) -> Option<String> {
    let (scheme, authority, _host, rest) = split_url(url)?;
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let port = if host_port.starts_with('[') {
        host_port
            .split(']')
            .nth(1)
            .and_then(|tail| tail.strip_prefix(':'))
    } else {
        host_port.split_once(':').map(|(_, p)| p)
    };
    let mut out = format!("{scheme}://{ip}");
    if let Some(port) = port {
        if !port.is_empty() {
            // AUDIT V28 (A4-L3): the port is validated, not
            // preserved verbatim — :0, :99999 and non-numeric
            // ports produced a QR the phone could only fail on;
            // an unrewritable URL is NotTeleportable instead.
            let valid = port.chars().all(|c| c.is_ascii_digit())
                && port
                    .parse::<u32>()
                    .map(|p| (1..=65535).contains(&p))
                    .unwrap_or(false);
            if !valid {
                return None;
            }
            out.push(':');
            out.push_str(port);
        }
    }
    out.push_str(rest);
    Some(out)
}

/// Resolve one tab URL for Teleport. `lan_ip` is the chosen LAN
/// address (if any); `is_pico_served` marks URLs served by Pico's
/// own built-in static server, which stays loopback-only by law
/// and is therefore never rewritten (the app layer knows its own
/// server's origin; the pure resolver only takes the flag).
pub fn resolve(url: &str, lan_ip: Option<Ipv4Addr>, is_pico_served: bool) -> Teleport {
    let not = |note: &str| Teleport {
        kind: TeleportKind::NotTeleportable,
        url: None,
        note: note.to_string(),
    };

    // Pico's internal scheme (start page, chrome, Pico-served
    // action URLs) — nothing leaves the PC.
    if url.starts_with("pico://") || url.starts_with("pico:") {
        return not(NOTE_INTERNAL);
    }
    let Some((scheme, authority, host, rest)) = split_url(url) else {
        // file:, about:, data:, view-source:, malformed — none of
        // them are pages a phone can open from here.
        return not(NOTE_SCHEME);
    };
    let scheme_l = scheme.to_ascii_lowercase();
    if scheme_l != "http" && scheme_l != "https" {
        return not(NOTE_SCHEME);
    }
    if host.is_empty() {
        return not(NOTE_SCHEME);
    }
    if is_pico_served {
        return not(NOTE_PICO_SERVED);
    }
    // AUDIT 0.31 (A4-F4): the display/copy URL with its
    // authority userinfo stripped — shared by the public
    // branch below and the NoNetwork branch; credentials
    // belong in neither a QR nor a copyable link.
    let stripped_url = |url: &str, scheme: &str, authority: &str, rest: &str| -> String {
        if authority.contains('@') {
            let host_port = authority.rsplit('@').next().unwrap_or(authority);
            format!("{scheme}://{host_port}{rest}")
        } else {
            url.to_string()
        }
    };
    if is_loopback_host(&host) {
        return match lan_ip {
            Some(ip) => match with_lan_host(url, ip) {
                Some(rewritten) => Teleport {
                    kind: TeleportKind::LanRewrite,
                    url: Some(rewritten),
                    // AUDIT 0.31 (A4-F6): an https original
                    // rewritten onto a LAN IP is a TLS dead
                    // end on the phone — the certificate
                    // names localhost, not the LAN IP — and
                    // the note must say so.
                    note: if scheme_l == "https" {
                        format!("{NOTE_LAN} Heads up: the original link is https, so the phone will hit a TLS/certificate dead end \u{2014} the certificate names localhost, not this PC's LAN address.")
                    } else {
                        NOTE_LAN.to_string()
                    },
                },
                None => not(NOTE_SCHEME),
            },
            None => Teleport {
                kind: TeleportKind::NoNetwork,
                // AUDIT 0.31 (A4-F4): strip userinfo here
                // too, like both sibling branches.
                url: Some(stripped_url(url, scheme, authority, rest)),
                note: NOTE_NO_NETWORK.to_string(),
            },
        };
    }
    // AUDIT V28 (A4-M4): a public URL is shared WITHOUT its
    // userinfo. The rewrite path has always stripped
    // credentials (they don't belong in a QR on screen); the
    // public path passed the URL through verbatim, password
    // included. Everything else — host, port, path, query,
    // fragment — stays byte-for-byte (an '@' in the path is
    // untouched: only the authority's userinfo is dropped).
    let public_url = stripped_url(url, scheme, authority, rest);
    Teleport {
        kind: TeleportKind::PublicUrl,
        url: Some(public_url),
        note: NOTE_PUBLIC.to_string(),
    }
}
