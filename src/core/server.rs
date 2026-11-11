//! Tiny built-in static file server (std-only) with per-root SSE
//! live reload, HTTP/1.1 keep-alive, and single-range support.
//!
//! Opened local files/folders are registered as *roots* and
//! served over `http://127.0.0.1:<random port>/r/<root-token>/<rel>`
//! so that relative assets, `fetch()` and ES modules work — they
//! all break on `file://`. One server instance serves every root
//! for the app's lifetime. Root tokens are unguessable 64-bit
//! values, not sequence numbers, so another local process that
//! can see the listening port cannot enumerate the user's opened
//! folders by guessing `1, 2, 3…`.
//!
//! Lifecycle (AUDIT M2): roots are reference-counted by the tabs
//! that show them. [`FileServer::register_root`] takes a
//! reference; [`FileServer::release_root`] drops one; at zero the
//! root stops being served and its reload subscribers are
//! dropped.
//!
//! Live reload (AUDIT M1): reload events are scoped per root. A
//! page served from root R subscribes to `/__pico/events/<R>`
//! and is only reloaded when R changes.
//!
//! Security posture (see SECURITY.md for the full register):
//! - Every request must carry a `Host` header naming this
//!   loopback server (127.0.0.1 / localhost / [::1], with our
//!   port when a port is present). A DNS-rebinding site resolves
//!   its own name to 127.0.0.1 but cannot forge the Host header
//!   from a browser, so rebinding dies here with 403.
//! - No CORS headers are emitted, ever.
//! - Connections, header bytes, header count and SSE subscribers
//!   are all capped; header reads have a per-read timeout *and*
//!   a total deadline (slow-loris).
//! - Paths are percent-decoded exactly once, then normalized as
//!   strings (backslashes, NULs, `:` segments, Windows device
//!   names, trailing dot/space tricks refused) *before* the
//!   filesystem is touched, and the canonical target must
//!   remain inside the canonical root.
//!
//! v22 protocol fix (AUDIT V22-1/2): HEAD responses never carry
//! a body — the 416 responder and the events-branch error
//! responses now thread `send_body` through like every other
//! responder. (The malformed-head error responses keep their
//! body: a request whose head cannot be parsed has no knowable
//! method, and a conforming HEAD client never reaches them.)

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::addr::is_safe_local_path;

/// URL prefix of the live-reload event endpoint. The full path
/// is `/__pico/events/<root-token>` (AUDIT M1: per-root events).
pub const EVENTS_PREFIX: &str = "/__pico/events/";

/// Total request-head budget (request line + all headers).
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Maximum simultaneous connections (files and SSE combined).
pub const MAX_CONNECTIONS: usize = 64;
/// Maximum simultaneous live-reload subscribers per root.
pub const MAX_SSE_SUBSCRIBERS_PER_ROOT: usize = 8;
/// Maximum requests served on one keep-alive connection.
pub const MAX_REQUESTS_PER_CONNECTION: usize = 100;
/// HTML larger than this is streamed without script injection.
pub const MAX_INJECT_HTML_BYTES: u64 = 8 * 1024 * 1024;

const HEADER_TIMEOUT: Duration = Duration::from_secs(3);
const HEAD_TOTAL_DEADLINE: Duration = Duration::from_secs(5);
const KEEPALIVE_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// The script injected into served HTML for root `root_id`. It
/// connects an `EventSource` to *that root's* reload endpoint
/// and reloads the page on a reload event. `EventSource`
/// reconnects automatically with backoff when the connection
/// drops, so no client-side retry loop is needed.
pub fn reload_script(root_id: u64) -> String {
    format!(
        "<script>(function(){{try{{var es=new EventSource(\"{EVENTS_PREFIX}{root_id}\");\
         es.onmessage=function(ev){{if(ev.data===\"reload\"){{location.reload();}}}};\
         }}catch(e){{}})();</script>"
    )
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn rw_read<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(|e| e.into_inner())
}
fn rw_write<T>(m: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    m.write().unwrap_or_else(|e| e.into_inner())
}

/// Find the last occurrence of `needle` in `haystack`, comparing
/// ASCII case-insensitively. Byte-based, so offsets are always
/// exact (a Unicode lowercase pass could shift them).
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

/// Inject the live-reload script for `root_id` into an HTML
/// document: immediately before the last `</body>`
/// (case-insensitive) when present, appended at the end
/// otherwise. Idempotent — a document that already references
/// the events endpoint is returned unchanged.
pub fn inject_reload_script(html: &[u8], root_id: u64) -> Vec<u8> {
    if rfind_ascii_ci(html, EVENTS_PREFIX.as_bytes()).is_some() {
        return html.to_vec();
    }
    let script = reload_script(root_id);
    if let Some(pos) = rfind_ascii_ci(html, b"</body>") {
        let mut out = Vec::with_capacity(html.len() + script.len());
        out.extend_from_slice(&html[..pos]);
        out.extend_from_slice(script.as_bytes());
        out.extend_from_slice(&html[pos..]);
        out
    } else {
        let mut out = html.to_vec();
        out.extend_from_slice(script.as_bytes());
        out
    }
}

// ---------------------------------------------------------------------------
// Live-reload hub (per-root channels — AUDIT M1)
// ---------------------------------------------------------------------------

/// One SSE subscription. Dropping it removes the sender from the
/// hub, so disconnected clients free their slot immediately (not
/// just at the next broadcast).
pub struct ReloadSubscription {
    hub: Arc<ReloadHub>,
    root_id: u64,
    id: u64,
    rx: std::sync::mpsc::Receiver<()>,
}

impl ReloadSubscription {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<(), std::sync::mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }
}

impl Drop for ReloadSubscription {
    fn drop(&mut self) {
        self.hub.remove(self.root_id, self.id);
    }
}

/// Fan-out hub for live-reload events, keyed by root: the
/// watcher for root R broadcasts only to R's subscribers.
/// The hub's mutable state under ONE lock (AUDIT 0.31,
/// A5-L1): the subscriber map and the dropped-root tombstone
/// set together, so a subscribe's dropped-check and its
/// insertion are atomic against a concurrent `drop_root` —
/// with two locks the check could pass, the drop could run
/// to completion, and the insertion could still land after
/// both, resurrecting a dead root's entry.
struct HubState {
    subs: HashMap<u64, HashMap<u64, std::sync::mpsc::Sender<()>>>,
    /// Tombstones of dropped roots (AUDIT V29, A5-6). Root ids
    /// are random 64-bit tokens that are never reused, so a
    /// tombstone is permanent and the set is session-bounded.
    dropped: std::collections::HashSet<u64>,
}

pub struct ReloadHub {
    state: Mutex<HubState>,
    next_id: AtomicU64,
}

impl ReloadHub {
    pub fn new() -> Arc<ReloadHub> {
        Arc::new(ReloadHub {
            state: Mutex::new(HubState {
                subs: HashMap::new(),
                dropped: std::collections::HashSet::new(),
            }),
            next_id: AtomicU64::new(1),
        })
    }

    /// Subscribe to one root's events, unless that root's
    /// subscriber cap is already reached — or the root is
    /// already gone. AUDIT V29 (A5-6): through v28 a subscribe
    /// racing `drop_root` landed AFTER the drop had swept the
    /// map, resurrecting an entry nobody would ever remove:
    /// the subscription hung forever, holding one of the
    /// root's 8 slots and a connection-gate permit. The
    /// tombstone makes the drop stick.
    pub fn try_subscribe(self: &Arc<Self>, root_id: u64) -> Option<ReloadSubscription> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel();
        {
            // AUDIT 0.31 (A5-L1): the dropped check and the
            // insertion happen under the SAME lock acquisition
            // that `drop_root` takes — no window between them.
            let mut state = lock(&self.state);
            if state.dropped.contains(&root_id) {
                return None;
            }
            let root_subs = state.subs.entry(root_id).or_default();
            if root_subs.len() >= MAX_SSE_SUBSCRIBERS_PER_ROOT {
                return None;
            }
            root_subs.insert(id, tx);
        }
        Some(ReloadSubscription {
            hub: Arc::clone(self),
            root_id,
            id,
            rx,
        })
    }

    fn remove(&self, root_id: u64, id: u64) {
        let mut state = lock(&self.state);
        if let Some(root_subs) = state.subs.get_mut(&root_id) {
            root_subs.remove(&id);
            if root_subs.is_empty() {
                state.subs.remove(&root_id);
            }
        }
    }

    /// Drop every subscriber of a root (the root went away; their
    /// SSE handlers see the channel disconnect and close) and
    /// tombstone it against late subscribes (A5-6) — atomically,
    /// under the one lock (AUDIT 0.31, A5-L1): a subscribe
    /// racing the drop either lands before it (and is swept)
    /// or after it (and sees the tombstone) — never between.
    pub fn drop_root(&self, root_id: u64) {
        let mut state = lock(&self.state);
        state.dropped.insert(root_id);
        state.subs.remove(&root_id);
    }

    /// Notify every subscriber of one root. Dead subscribers are
    /// dropped.
    pub fn broadcast(&self, root_id: u64) {
        let mut state = lock(&self.state);
        if let Some(root_subs) = state.subs.get_mut(&root_id) {
            root_subs.retain(|_, tx| tx.send(()).is_ok());
        }
    }

    pub fn subscriber_count(&self, root_id: u64) -> usize {
        lock(&self.state)
            .subs
            .get(&root_id)
            .map(|m| m.len())
            .unwrap_or(0)
    }
}

/// MIME type for a file, by extension.
pub fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" | "md" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

fn is_html(path: &Path) -> bool {
    mime_for(path).starts_with("text/html")
}

// ---------------------------------------------------------------------------
// Connection gate (a counter + RAII permit — AUDIT L8)
// ---------------------------------------------------------------------------

struct ConnectionGate {
    count: Mutex<usize>,
    max: usize,
}

struct ConnectionPermit {
    gate: Arc<ConnectionGate>,
}

impl ConnectionGate {
    fn new(max: usize) -> Arc<Self> {
        Arc::new(ConnectionGate {
            count: Mutex::new(0),
            max,
        })
    }

    fn try_acquire(self: &Arc<Self>) -> Option<ConnectionPermit> {
        let mut count = lock(&self.count);
        if *count >= self.max {
            return None;
        }
        *count += 1;
        Some(ConnectionPermit {
            gate: Arc::clone(self),
        })
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut count = lock(&self.gate.count);
        *count = count.saturating_sub(1);
    }
}

// ---------------------------------------------------------------------------
// File server
// ---------------------------------------------------------------------------

/// One registered servable root and its outstanding tab
/// references.
struct RootEntry {
    path: PathBuf,
    refs: usize,
}

/// A running static file server bound to 127.0.0.1 on a random
/// free port.
pub struct FileServer {
    port: u16,
    roots: Arc<RwLock<HashMap<u64, RootEntry>>>,
    hub: Arc<ReloadHub>,
    gate: Arc<ConnectionGate>,
}

fn random_root_token() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    // Fresh RandomState keys are seeded from the OS by std;
    // combined with the time, pid and a monotonic counter
    // through a splitmix finalizer, the result is a 64-bit token
    // that cannot be enumerated by a process that merely knows
    // the server is up.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    std::process::id().hash(&mut hasher);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    COUNTER
        .fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::SeqCst)
        .hash(&mut hasher);
    let mut x = hasher.finish();
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    if x == 0 {
        1
    } else {
        x
    }
}

impl FileServer {
    /// Bind 127.0.0.1:0 (OS picks a free port) and start
    /// accepting.
    pub fn start() -> std::io::Result<FileServer> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let roots: Arc<RwLock<HashMap<u64, RootEntry>>> = Arc::new(RwLock::new(HashMap::new()));
        let hub = ReloadHub::new();
        let gate = ConnectionGate::new(MAX_CONNECTIONS);
        let thread_roots = Arc::clone(&roots);
        let thread_hub = Arc::clone(&hub);
        let thread_gate = Arc::clone(&gate);
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => {
                        let Some(permit) = thread_gate.try_acquire() else {
                            // At capacity: refuse immediately
                            // rather than queueing threads.
                            drop(s);
                            continue;
                        };
                        let r = Arc::clone(&thread_roots);
                        let h = Arc::clone(&thread_hub);
                        thread::spawn(move || {
                            let _permit = permit;
                            handle_conn(s, port, r, h);
                        });
                    }
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Ok(FileServer {
            port,
            roots,
            hub,
            gate,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The live-reload hub: broadcast on it (per root) to reload
    /// the pages served from that root.
    pub fn reload_hub(&self) -> Arc<ReloadHub> {
        Arc::clone(&self.hub)
    }

    /// Register a directory as a servable root and take one
    /// reference on it. Returns its token; registering the same
    /// directory again returns the existing token with the
    /// reference count bumped. Non-local (UNC/device-namespace)
    /// roots are refused before and after canonicalisation.
    pub fn register_root(&self, root: &Path) -> std::io::Result<u64> {
        if !is_safe_local_path(root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "refusing to serve a network or device-namespace path",
            ));
        }
        let canon = root.canonicalize()?;
        if !is_safe_local_path(&canon) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path resolves to a network location; refusing to serve it",
            ));
        }
        let mut map = rw_write(&self.roots);
        for (id, entry) in map.iter_mut() {
            if entry.path == canon {
                entry.refs += 1;
                return Ok(*id);
            }
        }
        let mut token = random_root_token();
        while map.contains_key(&token) {
            token = random_root_token();
        }
        map.insert(
            token,
            RootEntry {
                path: canon,
                refs: 1,
            },
        );
        Ok(token)
    }

    /// Drop one reference on a root. When the last reference
    /// goes, the root is unregistered: its URLs start returning
    /// 404 and its reload subscribers are disconnected.
    /// Releasing an unknown or already-released token is a
    /// no-op.
    pub fn release_root(&self, id: u64) {
        let gone = {
            let mut map = rw_write(&self.roots);
            match map.get_mut(&id) {
                Some(entry) => {
                    entry.refs = entry.refs.saturating_sub(1);
                    if entry.refs == 0 {
                        map.remove(&id);
                        true
                    } else {
                        false
                    }
                }
                None => false,
            }
        };
        if gone {
            self.hub.drop_root(id);
        }
    }

    /// A second handle onto the same server — roots, hub and
    /// gate are shared `Arc`s, so handles are interchangeable.
    /// For background threads that need root lookups after the
    /// app took ownership.
    pub fn clone_handle(&self) -> FileServer {
        FileServer {
            port: self.port,
            roots: self.roots.clone(),
            hub: self.hub.clone(),
            gate: self.gate.clone(),
        }
    }

    /// Outstanding references on a root (diagnostics/tests).
    pub fn root_refs(&self, id: u64) -> Option<usize> {
        rw_read(&self.roots).get(&id).map(|e| e.refs)
    }

    pub fn root_path(&self, id: u64) -> Option<PathBuf> {
        rw_read(&self.roots).get(&id).map(|e| e.path.clone())
    }

    /// Build the URL for a file inside a root. `rel` uses `/`
    /// separators.
    pub fn url_for(&self, root_id: u64, rel: &str) -> String {
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            format!("{}/r/{}/", self.base(), root_id)
        } else {
            format!("{}/r/{}/{}", self.base(), root_id, percent_encode_path(rel))
        }
    }

    /// If `url` is served by this server, return the root token
    /// it targets.
    pub fn root_id_for_url(&self, url: &str) -> Option<u64> {
        self.url_parts(url).map(|(id, _)| id)
    }

    /// Structured parse of a Pico server URL (AUDIT L14): the
    /// root token and the percent-decoded relative path, or
    /// `None` when the URL is not one of this server's
    /// `/r/<token>/…` URLs. Positional parsing — no substring
    /// splitting on `/r/`, which a page path could itself
    /// contain.
    pub fn url_parts(&self, url: &str) -> Option<(u64, String)> {
        let base = self.base();
        let rest = url.strip_prefix(&base)?.strip_prefix("/r/")?;
        let (id_str, rel_raw) = match rest.split_once('/') {
            Some((a, b)) => (a, b),
            None => (rest, ""),
        };
        if id_str.is_empty() || !id_str.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let id: u64 = id_str.parse().ok()?;
        // The query string is not part of the file path.
        let rel_raw = rel_raw.split(['?', '#']).next().unwrap_or("");
        let rel = percent_decode(rel_raw)?;
        Some((id, rel))
    }
}

/// Is `url` a URL served by a Pico static server on `port`?
/// (AUDIT 0.31, A4-F1.) PARSED, not string-matched: through
/// 0.30 the app compared against its own canonical
/// `http://127.0.0.1:<port>` prefix, so the same server
/// reached as `localhost`, with userinfo, or with a
/// trailing-dot host was not recognized as Pico-served (and
/// Teleport would happily rewrite/send it). True iff the
/// scheme is http, the authority's host — userinfo stripped,
/// one trailing dot allowed — is 127.0.0.1, localhost or
/// [::1], the port equals `port` (an absent port means the
/// http default, 80), and the path begins with `/r/`.
pub fn is_served_url(url: &str, port: u16) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    let auth_end = rest
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    let path = &rest[auth_end..];
    if !path.starts_with("/r/") {
        return false;
    }
    // Strip userinfo: everything up to the LAST '@'.
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port_part) = if let Some(inner) = host_port.strip_prefix('[') {
        let Some(close) = inner.find(']') else {
            return false;
        };
        let tail = &inner[close + 1..];
        let p = if tail.is_empty() {
            None
        } else if let Some(p) = tail.strip_prefix(':') {
            Some(p)
        } else {
            return false;
        };
        (&inner[..close], p)
    } else {
        match host_port.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (host_port, None),
        }
    };
    // A single trailing dot is the same host; two are not.
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.ends_with('.') {
        return false;
    }
    let host_ok = host.eq_ignore_ascii_case("127.0.0.1")
        || host.eq_ignore_ascii_case("localhost")
        || host.eq_ignore_ascii_case("::1");
    if !host_ok {
        return false;
    }
    let url_port = match port_part {
        None => 80,
        Some(p) => {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            match p.parse::<u16>() {
                Ok(n) => n,
                Err(_) => return false,
            }
        }
    };
    url_port == port
}

fn percent_encode_path(rel: &str) -> String {
    let mut out = String::with_capacity(rel.len());
    for b in rel.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'/'
            | b'('
            | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decode a percent-encoded URL path segment string. Decoding
/// happens exactly once: `%252e` becomes the literal text
/// `%2e`, never `.`.
pub fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Request parsing (pure — host-testable without a socket)
// ---------------------------------------------------------------------------

/// A parsed, validated request head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRequest {
    pub method: String,
    /// Origin-form target (`/path?query`), exactly as sent.
    pub target: String,
    pub host: String,
    /// "HTTP/1.0" or "HTTP/1.1".
    pub version: String,
    /// Whether the connection may be reused for another request:
    /// HTTP/1.1 unless `Connection: close`, HTTP/1.0 only with
    /// `Connection: keep-alive`, and never when the request
    /// declares a body (bodies are not consumed, so reuse could
    /// desync).
    pub keep_alive: bool,
    /// Raw `Range` header value, if any (parsed later, when the
    /// target file's length is known).
    pub range: Option<String>,
}

/// Validate a `Host` header value against this server. Only the
/// loopback spellings are accepted, and an explicit port must be
/// ours. This is the DNS-rebinding wall: a rebinding hostname is
/// neither `localhost` nor a loopback literal, so it fails here.
pub fn validate_host_header(value: &str, expected_port: u16) -> bool {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty()
        || v.len() > 255
        || v.chars().any(|c: char| c.is_control() || c.is_whitespace())
        || v.contains('@')
    {
        return false;
    }
    let (host, port) = if let Some(rest) = v.strip_prefix('[') {
        let Some(close) = rest.find(']') else {
            return false;
        };
        let host = &rest[..close];
        let tail = &rest[close + 1..];
        let port = if tail.is_empty() {
            None
        } else if let Some(p) = tail.strip_prefix(':') {
            match parse_port(p) {
                Some(p) => Some(p),
                None => return false,
            }
        } else {
            return false;
        };
        (host.to_string(), port)
    } else {
        if v.matches(':').count() > 1 {
            return false;
        }
        match v.split_once(':') {
            Some((h, p)) => match parse_port(p) {
                Some(p) => (h.to_string(), Some(p)),
                None => return false,
            },
            None => (v.clone(), None),
        }
    };
    if let Some(p) = port {
        if p != expected_port {
            return false;
        }
    }
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

fn parse_port(text: &str) -> Option<u16> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Parse and validate a complete request head (request line,
/// headers, terminating blank line). Errors are HTTP status
/// codes — 400 when malformed, 403 when the Host is not this
/// loopback server, 431 when the head is oversized.
pub fn parse_request_head(block: &[u8], expected_port: u16) -> Result<ParsedRequest, u16> {
    if block.len() > MAX_HEADER_BYTES {
        return Err(431);
    }
    let text = std::str::from_utf8(block).map_err(|_| 400u16)?;
    if !text.ends_with("\r\n\r\n") && !text.ends_with("\n\n") {
        return Err(400); // head must be properly terminated
    }
    let mut lines = text.split('\n');
    let request_line = lines.next().ok_or(400u16)?.trim_end_matches('\r');
    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or(400u16)?;
    let target = parts.next().ok_or(400u16)?;
    let version = parts.next().ok_or(400u16)?;
    if parts.next().is_some()
        || method.is_empty()
        || !method.bytes().all(|b| b.is_ascii_uppercase())
        || (version != "HTTP/1.0" && version != "HTTP/1.1")
    {
        return Err(400);
    }
    // Origin-form only: absolute-form URIs (`http://other/...`)
    // and network-path targets (`//other/...`) are refused.
    if !target.starts_with('/')
        || target.starts_with("//")
        || target.contains('\\')
        || target.contains('#')
        || target.chars().any(|c: char| c.is_control())
    {
        return Err(400);
    }

    let mut host: Option<String> = None;
    let mut connection_close = false;
    let mut connection_keep_alive = false;
    let mut has_body = false;
    let mut range: Option<String> = None;
    let mut header_count = 0usize;
    let mut terminated = false;
    for raw_line in lines {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            terminated = true;
            break;
        }
        header_count += 1;
        if header_count > 64 {
            return Err(431);
        }
        // No obsolete line folding, no nameless headers.
        if line.starts_with([' ', '\t']) {
            return Err(400);
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(400);
        };
        if name.is_empty() || !name.bytes().all(is_token_char) {
            return Err(400);
        }
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            if host.is_some() {
                return Err(400); // exactly one Host header
            }
            host = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("connection") {
            for tok in value.split(',') {
                let tok = tok.trim();
                if tok.eq_ignore_ascii_case("close") {
                    connection_close = true;
                } else if tok.eq_ignore_ascii_case("keep-alive") {
                    connection_keep_alive = true;
                }
            }
        } else if name.eq_ignore_ascii_case("content-length") {
            // Any declared body forbids connection reuse (bodies
            // are never consumed, so reuse could desync).
            match value.parse::<u64>() {
                Ok(0) => {}
                Ok(_) => has_body = true,
                Err(_) => return Err(400),
            }
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            has_body = true;
        } else if name.eq_ignore_ascii_case("range") && range.is_none() {
            range = Some(value.to_string());
        }
    }
    if !terminated {
        return Err(400);
    }
    let host = host.ok_or(400u16)?;
    if !validate_host_header(&host, expected_port) {
        return Err(403);
    }
    let keep_alive = !has_body
        && if version == "HTTP/1.1" {
            !connection_close
        } else {
            connection_keep_alive
        };
    Ok(ParsedRequest {
        method: method.to_string(),
        target: target.to_string(),
        host,
        version: version.to_string(),
        keep_alive,
        range,
    })
}

/// Outcome of parsing a `Range` header value against a body
/// length (AUDIT M3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOutcome {
    /// Malformed, multi-range, or a unit we do not speak: ignore
    /// the header and serve the full body (RFC-sanctioned).
    Ignore,
    /// Satisfiable single range, inclusive byte offsets: 206.
    Slice(u64, u64),
    /// Well-formed but unsatisfiable for this length: 416.
    Unsatisfiable,
}

pub fn parse_range(value: &str, len: u64) -> RangeOutcome {
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return RangeOutcome::Ignore; // a range unit we do not speak
    };
    if spec.contains(',') {
        return RangeOutcome::Ignore; // multi-range (RFC 9110 allows ignoring)
    }
    let Some((a, b)) = spec.split_once('-') else {
        return RangeOutcome::Ignore;
    };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() && b.is_empty() {
        return RangeOutcome::Ignore;
    }
    if (!a.is_empty() && !a.bytes().all(|c| c.is_ascii_digit()))
        || (!b.is_empty() && !b.bytes().all(|c| c.is_ascii_digit()))
    {
        return RangeOutcome::Ignore;
    }
    if len == 0 {
        return RangeOutcome::Unsatisfiable;
    }
    if a.is_empty() {
        // Suffix range: the last b bytes. The digit check above
        // means this parse can only fail on u64 overflow — a
        // suffix longer than any file — so clamp to whole.
        let n: u64 = b.parse().unwrap_or(u64::MAX);
        if n == 0 {
            return RangeOutcome::Unsatisfiable;
        }
        if n >= len {
            return RangeOutcome::Slice(0, len - 1);
        }
        return RangeOutcome::Slice(len - n, len - 1);
    }
    let start: u64 = match a.parse() {
        Ok(n) => n,
        Err(_) => return RangeOutcome::Ignore, // overflow: nonsense value
    };
    if start >= len {
        return RangeOutcome::Unsatisfiable;
    }
    if b.is_empty() {
        return RangeOutcome::Slice(start, len - 1);
    }
    let end: u64 = match b.parse() {
        Ok(n) => n,
        Err(_) => return RangeOutcome::Ignore,
    };
    if end < start {
        return RangeOutcome::Unsatisfiable;
    }
    RangeOutcome::Slice(start, end.min(len - 1))
}

// ---------------------------------------------------------------------------
// Connection handling
// ---------------------------------------------------------------------------

/// Read one header line (through `\n`) without ever buffering
/// more than the global head budget. 431 when the budget is
/// exceeded, 400 on EOF before the line completes or when the
/// total head deadline passes mid-line (a byte-dribbling peer
/// must not stretch one line past the deadline — AUDIT L1).
fn read_header_line(
    reader: &mut impl BufRead,
    budget: &mut usize,
    deadline: Instant,
) -> Result<Vec<u8>, u16> {
    let mut line = Vec::new();
    loop {
        if Instant::now() > deadline {
            return Err(408);
        }
        let available = reader.fill_buf().map_err(|_| 400u16)?;
        if available.is_empty() {
            return Err(400);
        }
        match available.iter().position(|b| *b == b'\n') {
            Some(pos) => {
                let take = pos + 1;
                if take > *budget {
                    return Err(431);
                }
                line.extend_from_slice(&available[..take]);
                reader.consume(take);
                *budget -= take;
                return Ok(line);
            }
            None => {
                if available.len() > *budget {
                    return Err(431);
                }
                line.extend_from_slice(available);
                let n = available.len();
                reader.consume(n);
                *budget -= n;
            }
        }
    }
}

/// Outcome of trying to read one request head.
enum HeadRead {
    Head(Vec<u8>),
    /// Peer closed (or idled out) before a complete head arrived.
    Closed,
    /// Head exceeded the byte budget.
    TooLarge,
    /// Total deadline hit mid-head (slow-loris — AUDIT L1).
    TimedOut,
}

fn read_head(reader: &mut impl BufRead, deadline: Instant) -> HeadRead {
    let mut block: Vec<u8> = Vec::with_capacity(1024);
    let mut budget = MAX_HEADER_BYTES;
    let mut lines = 0usize;
    loop {
        if Instant::now() > deadline {
            return HeadRead::TimedOut;
        }
        match read_header_line(reader, &mut budget, deadline) {
            Ok(line) => {
                block.extend_from_slice(&line);
                if line == b"\r\n" || line == b"\n" {
                    return HeadRead::Head(block);
                }
                lines += 1;
                if lines > 65 {
                    return HeadRead::TooLarge;
                }
            }
            Err(431) => return HeadRead::TooLarge,
            Err(408) => return HeadRead::TimedOut,
            Err(_) => return HeadRead::Closed,
        }
    }
}

fn handle_conn(
    stream: TcpStream,
    port: u16,
    roots: Arc<RwLock<HashMap<u64, RootEntry>>>,
    hub: Arc<ReloadHub>,
) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    // Loopback latency matters more than packet coalescing here:
    // without NODELAY, the head/body two-write pattern stalls
    // ~40 ms per response against the peer's delayed ACK.
    let _ = stream.set_nodelay(true);
    let read_half = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::with_capacity(4096, read_half);
    let mut writer = stream;
    let mut served = 0usize;

    loop {
        // The first head gets the strict header timeout;
        // subsequent heads on a keep-alive connection get the
        // idle timeout. Both are bounded by a total deadline (L1).
        let _ = writer.set_read_timeout(Some(if served == 0 {
            HEADER_TIMEOUT
        } else {
            KEEPALIVE_IDLE_TIMEOUT
        }));
        let deadline = Instant::now() + HEAD_TOTAL_DEADLINE.max(KEEPALIVE_IDLE_TIMEOUT);
        let block = match read_head(&mut reader, deadline) {
            HeadRead::Head(b) => b,
            HeadRead::TooLarge => {
                // Malformed-head error: the method is unknowable,
                // so this response keeps its body (see module doc).
                let _ = respond_bytes(
                    &mut writer,
                    431,
                    "text/plain; charset=utf-8",
                    status_message(431),
                    true,
                    true,
                );
                return;
            }
            HeadRead::Closed | HeadRead::TimedOut => return,
        };

        let req = match parse_request_head(&block, port) {
            Ok(r) => r,
            Err(status) => {
                let _ = respond_bytes(
                    &mut writer,
                    status,
                    "text/plain; charset=utf-8",
                    status_message(status),
                    true,
                    true,
                );
                return;
            }
        };
        served += 1;
        let close_after = !req.keep_alive || served >= MAX_REQUESTS_PER_CONNECTION;
        // AUDIT 0.40.2: the body is suppressed for HEAD only.
        // This used to be `method == "GET"`, which made the
        // error arms ABOVE the method gate (events, the
        // reserved prefix) advertise a Content-Length they
        // never sent for POST/PUT/DELETE — a truncated
        // response. Every consumer below the method gate only
        // ever sees GET or HEAD, so their behaviour is
        // unchanged.
        let send_body = req.method != "HEAD";

        let path = req.target.split(['?', '#']).next().unwrap_or("/");

        // Live-reload event stream for one root (GET only, held
        // open; the connection ends with the stream). v22: the
        // error arms honor send_body, so a HEAD request here gets
        // headers only (AUDIT V22-2).
        if let Some(id_str) = path.strip_prefix(EVENTS_PREFIX) {
            match id_str.parse::<u64>() {
                Ok(root_id) if req.method == "GET" && rw_read(&roots).contains_key(&root_id) => {
                    serve_events(&mut writer, &hub, root_id);
                }
                Ok(_) if req.method == "GET" => {
                    let _ = respond_bytes(
                        &mut writer,
                        404,
                        "text/plain; charset=utf-8",
                        b"404 Not Found",
                        send_body,
                        true,
                    );
                }
                Ok(_) => {
                    // Non-GET on the events endpoint: 405; for
                    // HEAD, send_body is false, so headers only.
                    let _ = respond_bytes(
                        &mut writer,
                        405,
                        "text/plain; charset=utf-8",
                        status_message(405),
                        send_body,
                        true,
                    );
                }
                Err(_) => {
                    let _ = respond_bytes(
                        &mut writer,
                        404,
                        "text/plain; charset=utf-8",
                        b"404 Not Found",
                        send_body,
                        true,
                    );
                }
            }
            return;
        }
        if path == "/__pico/events" || path.starts_with("/__pico/") {
            // The v12 global endpoint is gone (M1); anything else
            // under the reserved prefix is a 404, never a lookup.
            let _ = respond_bytes(
                &mut writer,
                404,
                "text/plain; charset=utf-8",
                b"404 Not Found",
                send_body,
                true,
            );
            return;
        }

        if req.method != "GET" && req.method != "HEAD" {
            let _ = respond_bytes(
                &mut writer,
                405,
                "text/plain; charset=utf-8",
                b"Method Not Allowed",
                true,
                true,
            );
            return;
        }
        match resolve_file(path, &roots) {
            Ok((file, root_id)) => {
                let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                // Range applies to streamed bodies. HTML small
                // enough for injection is never range-served: a
                // partial HTML body must not ship without its
                // reload-script logic being whole-document.
                let want_range = if is_html(&file) && len <= MAX_INJECT_HTML_BYTES {
                    None
                } else {
                    req.range.as_deref()
                };
                match want_range.map(|r| parse_range(r, len)) {
                    Some(RangeOutcome::Unsatisfiable) => {
                        // AUDIT V22-1: send_body threaded through —
                        // a HEAD 416 no longer carries a body.
                        // AUDIT 0.40.2 (A5-3): a failed write here
                        // closes the connection like every other
                        // response site — never loop keep-alive on
                        // a desynced stream.
                        if respond_range_unsatisfiable(&mut writer, len, close_after, send_body)
                            .is_err()
                        {
                            return;
                        }
                    }
                    Some(RangeOutcome::Slice(start, end)) => {
                        // AUDIT V29 (A5-3): a mid-stream error
                        // (above all: the file shrank under the
                        // promised Content-Length) closes the
                        // connection — never loop keep-alive on
                        // a desynced stream.
                        if respond_file(
                            &mut writer,
                            &file,
                            len,
                            Some((start, end)),
                            send_body,
                            close_after,
                            root_id,
                        )
                        .is_err()
                        {
                            return;
                        }
                    }
                    Some(RangeOutcome::Ignore) | None => {
                        if respond_file(
                            &mut writer,
                            &file,
                            len,
                            None,
                            send_body,
                            close_after,
                            root_id,
                        )
                        .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            Err(status) => {
                // AUDIT 0.40.2 (A5-3): same rule as the 416 arm
                // above — a failed write closes the connection
                // instead of looping keep-alive on a desynced
                // stream.
                if respond_bytes(
                    &mut writer,
                    status,
                    "text/plain; charset=utf-8",
                    status_message(status),
                    send_body,
                    close_after,
                )
                .is_err()
                {
                    return;
                }
            }
        }
        if close_after {
            return;
        }
        // Keep-alive: loop for the next request. Any buffered
        // pipelined bytes stay in `reader` — correct.
    }
}

fn status_message(status: u16) -> &'static [u8] {
    match status {
        400 => b"400 Bad Request",
        403 => b"403 Forbidden",
        404 => b"404 Not Found",
        405 => b"405 Method Not Allowed",
        416 => b"416 Range Not Satisfiable",
        431 => b"431 Request Header Fields Too Large",
        _ => b"404 Not Found",
    }
}

/// Hold the connection open as an SSE stream for one root until
/// the client goes away or the root is unregistered (its
/// subscribers are dropped by the hub, which disconnects this
/// handler). Sends a comment heartbeat every 15 s.
fn serve_events(writer: &mut TcpStream, hub: &Arc<ReloadHub>, root_id: u64) {
    let Some(rx) = hub.try_subscribe(root_id) else {
        let _ = respond_bytes(
            writer,
            503,
            "text/plain; charset=utf-8",
            b"Too Many Subscribers",
            true,
            true,
        );
        return;
    };
    let _ = writer.set_read_timeout(None);
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nX-Accel-Buffering: no\r\n\r\n: pico live reload\n\n";
    if writer.write_all(head.as_bytes()).is_err() {
        return;
    }
    let _ = writer.flush();
    loop {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(()) => {
                if writer.write_all(b"data: reload\n\n").is_err() {
                    return;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if writer.write_all(b": ping\n\n").is_err() {
                    return;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if writer.flush().is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

const WINDOWS_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    // AUDIT 0.40.1: the console device aliases are reserved too.
    "CONIN$", "CONOUT$",
];

/// Normalize a once-percent-decoded relative path into safe
/// segments, as a pure string operation (host-testable, runs
/// before any filesystem access). Refuses: NULs and other
/// controls, backslashes, `..` segments, `:`-bearing segments
/// (drive/ADS tricks), segments ending in `.` or space (Windows
/// silently strips those, which is how `.. ` smuggles a
/// traversal past naive checks), and Windows device names in
/// any segment.
pub fn normalize_relative_path(decoded: &str) -> Result<Vec<String>, u16> {
    if decoded.chars().any(|c: char| c.is_control()) || decoded.contains('\\') {
        return Err(400);
    }
    let mut out = Vec::new();
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Err(404),
            s => {
                if s.contains(':') || s.ends_with(['.', ' ']) {
                    return Err(400);
                }
                let stem = s.split('.').next().unwrap_or(s).to_ascii_uppercase();
                if WINDOWS_DEVICE_NAMES.contains(&stem.as_str()) {
                    return Err(400);
                }
                out.push(s.to_string());
            }
        }
    }
    Ok(out)
}

/// Resolve a request path to a regular file inside a registered
/// root, plus the root token it belongs to (for reload-script
/// injection). Err(status) is 400 for undecodable/unsafe paths,
/// 404 for everything else (missing files, traversal attempts,
/// directories without index, unregistered roots).
fn resolve_file(
    path: &str,
    roots: &RwLock<HashMap<u64, RootEntry>>,
) -> Result<(PathBuf, u64), u16> {
    let rest = path.strip_prefix("/r/").ok_or(404u16)?;
    let (id_str, rel_raw) = match rest.split_once('/') {
        Some((a, b)) => (a, b),
        None => (rest, ""),
    };
    let id: u64 = id_str.parse().map_err(|_| 404u16)?;
    let root = rw_read(roots)
        .get(&id)
        .map(|e| e.path.clone())
        .ok_or(404u16)?;
    let rel_decoded = percent_decode(rel_raw).ok_or(400u16)?;
    let segments = normalize_relative_path(&rel_decoded)?;
    let mut target = root.clone();
    for seg in &segments {
        target.push(seg);
    }
    let canon = target.canonicalize().map_err(|_| 404u16)?;
    if !canon.starts_with(&root) {
        return Err(404);
    }
    if canon.is_dir() {
        let index = canon.join("index.html");
        match index.canonicalize() {
            Ok(c) if c.starts_with(&root) && c.is_file() => Ok((c, id)),
            _ => Err(404),
        }
    } else if canon.is_file() {
        Ok((canon, id))
    } else {
        Err(404)
    }
}

fn security_headers() -> &'static str {
    "X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n"
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn connection_token(close_after: bool) -> &'static str {
    if close_after {
        "close"
    } else {
        "keep-alive"
    }
}

fn respond_bytes(
    stream: &mut TcpStream,
    status: u16,
    ctype: &str,
    body: &[u8],
    send_body: bool,
    close_after: bool,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{}Connection: {}\r\n\r\n",
        reason_phrase(status),
        body.len(),
        security_headers(),
        connection_token(close_after)
    );
    stream.write_all(head.as_bytes())?;
    if send_body {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn respond_range_unsatisfiable(
    stream: &mut TcpStream,
    len: u64,
    close_after: bool,
    send_body: bool,
) -> std::io::Result<()> {
    let body = status_message(416);
    let head = format!(
        "HTTP/1.1 416 {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nContent-Range: bytes */{len}\r\nCache-Control: no-store\r\n{}Connection: {}\r\n\r\n",
        reason_phrase(416),
        body.len(),
        security_headers(),
        connection_token(close_after)
    );
    stream.write_all(head.as_bytes())?;
    // AUDIT V22-1: headers advertise the body a GET would have
    // received; a HEAD response must not carry the bytes.
    if send_body {
        stream.write_all(body)?;
    }
    stream.flush()
}

/// Serve a resolved file: injected-in-memory for small HTML,
/// streamed (with optional single-range window) for everything
/// else.
fn respond_file(
    stream: &mut TcpStream,
    file_path: &Path,
    len: u64,
    range: Option<(u64, u64)>,
    send_body: bool,
    close_after: bool,
    root_id: u64,
) -> std::io::Result<()> {
    if range.is_none() && is_html(file_path) && len <= MAX_INJECT_HTML_BYTES {
        match std::fs::read(file_path) {
            Ok(bytes) => {
                let body = inject_reload_script(&bytes, root_id);
                return respond_bytes(
                    stream,
                    200,
                    mime_for(file_path),
                    &body,
                    send_body,
                    close_after,
                );
            }
            Err(_) => {
                return respond_bytes(
                    stream,
                    404,
                    "text/plain; charset=utf-8",
                    b"404 Not Found",
                    send_body,
                    close_after,
                );
            }
        }
    }
    // AUDIT V28 (A5-L1): an open failure here (the file lost a
    // canonicalize→open race, an AV lock, a vanished temp) must
    // still produce a RESPONSE. Through v27 the error
    // propagated and both call sites discarded it — zero bytes
    // on a keep-alive connection, which the client reads as a
    // hang or as the NEXT response's bytes. A plain 404 is the
    // honest answer: as far as this request can tell, the file
    // is not there.
    let mut file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => {
            return respond_bytes(
                stream,
                404,
                "text/plain; charset=utf-8",
                b"404 Not Found",
                send_body,
                close_after,
            );
        }
    };
    // AUDIT V29 (A5-3): the length that governs the head is
    // the OPENED handle's, not the caller's pre-open metadata
    // — a file that shrinks between the two (the live-edit
    // workload exactly) otherwise gets a head promising bytes
    // that never arrive, and the keep-alive loop then serves
    // the NEXT response into the gap (protocol desync). The
    // range clamps to the real length; a range that starts
    // past the shrunken end degrades to a full 200 of what
    // exists now. Both are honest answers; a short body on a
    // reusable connection is not (see the read loop).
    let len = file.metadata().map(|m| m.len()).unwrap_or(len);
    let (start, end, status) = match range {
        Some((s, e)) if s < len => (s, e.min(len - 1), 206u16),
        Some(_) if len > 0 => (0, len - 1, 200u16),
        Some(_) => (0, 0, 200u16),
        None => (0, len.saturating_sub(1), 200u16),
    };
    let body_len = if len == 0 { 0 } else { end - start + 1 };
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {}\r\nContent-Length: {body_len}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\n{}Connection: {}\r\n",
        reason_phrase(status),
        mime_for(file_path),
        security_headers(),
        connection_token(close_after)
    );
    if status == 206 {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if send_body && body_len > 0 {
        file.seek(SeekFrom::Start(start))?;
        let mut remaining = body_len;
        let mut buf = [0u8; 64 * 1024];
        while remaining > 0 {
            let want = (remaining as usize).min(buf.len());
            let n = file.read(&mut buf[..want])?;
            if n == 0 {
                // AUDIT V29 (A5-3): EOF before the promised
                // length (the file shrank mid-stream). The
                // head already promised body_len bytes, so the
                // only honest move left is to CLOSE: returning
                // an error makes the caller drop the
                // connection instead of looping keep-alive
                // into a desynced stream.
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "file shrank mid-stream",
                ));
            }
            stream.write_all(&buf[..n])?;
            remaining -= n as u64;
        }
    }
    stream.flush()
}
