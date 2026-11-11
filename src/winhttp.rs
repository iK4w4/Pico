//! WinHTTP transport for the download manager (0.32).
//!
//! Generalizes the public sibling's `winhttp_get` (the Shield's
//! list download, HTTPS-only and buffered) into the two calls
//! the manager needs:
//!
//! - [`probe`] — HEAD first, with a ranged-GET fallback, that
//!   answers the pure core's `ProbeFacts` (total, range support,
//!   validators, final status and URL, disposition name).
//! - [`fetch_range`] — one ranged (or whole-file) GET whose body
//!   streams through a sink in ≤ 64 KiB chunks, but only after
//!   a PRE-BODY judgment: the final status and Content-Range
//!   are validated before a single byte is read, so an error
//!   page can never be written into a download or counted as
//!   progress (AUDIT 0.32.1, P1-1). The raw final status still
//!   surfaces (200/206/404/416/…) because the CALLER judges
//!   what it means for the task.
//!
//! Discipline, shared with the sibling binding:
//!
//! - WinHTTP — the OS stack — so SChannel TLS and the system
//!   proxy configuration come from Windows itself; no
//!   third-party HTTP code enters the tree.
//! - Redirects are followed MANUALLY (WinHTTP's own following
//!   is switched off — and the switch-off is verified, failing
//!   closed), at most [`MAX_REDIRECTS`] hops, and every
//!   hop must stay on http/https; an https → http downgrade is
//!   refused outright (AUDIT 0.32.1, P1-7/P1-15). A redirect
//!   hop's body is never read at all — the handle is dropped,
//!   which aborts the transfer — so a redirect stub page can
//!   never be spliced into a downloaded file. On every hop the
//!   `Cookie` header is recomputed through the caller's
//!   `cookie_for` closure (host-scoped by
//!   `core::download::cookie_header`) and `Range` is re-sent:
//!   servers redirect to the real file host, so the range must
//!   reach it. `If-Range` is re-sent only to the ORIGIN host —
//!   it is the original resource's validator and means nothing
//!   (and leaks) anywhere else; a validator mismatch after a
//!   redirect simply yields a 200, which the caller honestly
//!   treats as "restart single-stream" (the brief's transport
//!   contract records this decision).
//! - NO decompression option is ever set: range math needs the
//!   raw bytes exactly as the server counts them.
//! - Userinfo in a URL is stripped for the connect and never
//!   forwarded — no `Authorization` is synthesized. Downloads
//!   with embedded credentials are not a feature; the tab's
//!   cookies (via `cookie_for`) are the credential path.
//! - Error strings are short and log-safe: OS error codes and
//!   status numbers only, never a URL, never a header value.
//!
//! Zero `unwrap`/`expect`/`panic!` in this module: every failure
//! is a [`FetchError`].

use pico_core::core::download::{file_name_from_disposition, parse_content_range, ProbeFacts};

/// Redirect hops followed before giving up (brief: cap 10).
const MAX_REDIRECTS: usize = 10;
/// The sink-error sentinel for "the server sent more bytes
/// than the requested range" (AUDIT 0.40.1 byte budget). The
/// value is shared with `src/downloads.rs`'s own sink cap —
/// the two constants must stay equal; the manager matches on
/// it to classify the failure.
pub(crate) const RANGE_OVERFLOW: &str = "pico-range-overflow";
/// Body chunks handed to a sink are at most this large, and
/// `should_stop` is consulted before every read.
const CHUNK_BYTES: usize = 64 * 1024;
/// Stall floor for [`OpenResponse::stream`]: a window this
/// long delivering fewer than [`STALL_MIN_BYTES`] aborts the
/// transfer (AUDIT 0.32.1, P1-14).
const STALL_WINDOW_SECS: u64 = 60;
const STALL_MIN_BYTES: u64 = 1024;
/// The probe's ranged-GET fallback reads at most this much of
/// the (expected 1-byte) body before closing the handle.
const PROBE_DISCARD_CAP: usize = 64 * 1024;
/// A Location / header value longer than this is unusable.
const MAX_HEADER_CHARS: usize = 8192;
/// Per-request RECEIVE timeout for segment fetches (AUDIT
/// 0.32.2, Fix 2): the pause/cancel responsiveness bound. A
/// worker parked in a read notices a stop within this,
/// instead of the session's 30 s; on a healthy link data
/// gaps are far shorter than 5 s, and when the timeout does
/// trip it surfaces as an ordinary retryable transport
/// failure whose retries resume from the honest byte count.
/// Probes keep the session timeout (they pass `None`).
const SEGMENT_RECEIVE_TIMEOUT_MS: u32 = 5_000;

/// The transport's failure shape: the caller's control asked
/// for a stop (pause/cancel — not an error), or the request
/// failed with a short log-safe message.
#[derive(Debug)]
pub enum FetchError {
    Stopped,
    Failed(String),
}

/// One finished request: the FINAL hop's raw status and URL,
/// plus the headers the manager judges by, captured by name:
/// Content-Length, Content-Range, ETag, Last-Modified,
/// Accept-Ranges, Content-Disposition (Location is read on
/// redirect hops and consumed internally).
#[derive(Debug)]
pub struct FetchOutcome {
    pub status: u16,
    /// Contract surface (the brief's transport contract): the
    /// manager currently keeps the task's original URL for
    /// resume identity and does not read this, but the field is
    /// part of the outcome every caller receives.
    #[allow(dead_code)]
    pub final_url: String,
    pub headers: Vec<(String, String)>,
}

impl FetchOutcome {
    /// Case-insensitive header lookup by name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// [`probe`]'s answer: the core's probe facts, the final URL
/// after redirects, and the raw Content-Disposition file name
/// of the final response when it carried one (the caller
/// sanitizes it — `file_name_from_disposition`'s contract is
/// that it returns the raw extracted name).
#[derive(Debug)]
pub struct ProbeResult {
    pub facts: ProbeFacts,
    /// Contract surface, like `FetchOutcome::final_url`.
    #[allow(dead_code)]
    pub final_url: String,
    pub disposition_name: Option<String>,
}

/// Split an http/https URL into (secure, host, port,
/// path+query) for WinHttpConnect/WinHttpOpenRequest.
///
/// - Default ports 80 (http) / 443 (https).
/// - Bracketed IPv6 literals are unbracketed for the connect;
///   an unbracketed authority with more than one colon is read
///   as a bare IPv6 literal on the scheme's default port.
/// - Userinfo (`user[:pass]@`) is stripped and dropped (see the
///   module doc); the path never carries it.
/// - The fragment is dropped: it is never sent on the wire.
/// - Anything that is not http/https, has no host, or has a
///   non-numeric / out-of-range port is refused (`None`).
fn split_url(url: &str) -> Option<(bool, String, u16, String)> {
    // AUDIT 0.40.1: a raw control, space or DEL anywhere in the
    // URL is refused here, at the one choke point every fetch
    // and every redirect hop passes through. Legitimate URLs
    // percent-encode these; a raw CR/LF reaching
    // WinHttpOpenRequest's object name or WinHttpConnect's
    // host could otherwise split the request line.
    if url.bytes().any(|b| b <= 0x20 || b == 0x7f) {
        return None;
    }
    // AUDIT 0.32.1 (P1-16): the scheme comparison is
    // case-insensitive — the core's gate accepts `HTTPS://…`
    // (schemes are case-insensitive per RFC 3986), and a
    // transport that only strips the lowercase spellings
    // refused or mangled such URLs instead of fetching them.
    let (scheme, rest) = url.split_once("://")?;
    let secure = if scheme.eq_ignore_ascii_case("https") {
        true
    } else if scheme.eq_ignore_ascii_case("http") {
        false
    } else {
        return None;
    };
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) => {
            let tail = &rest[i..];
            if tail.starts_with('?') {
                (rest[..i].to_string(), format!("/{tail}"))
            } else {
                (rest[..i].to_string(), tail.to_string())
            }
        }
        None => (rest.to_string(), "/".to_string()),
    };
    if authority.is_empty() {
        return None;
    }
    // Drop userinfo: the LAST '@' separates it (RFC 3986).
    let hostport = authority.rsplit('@').next().unwrap_or("");
    if hostport.is_empty() {
        return None;
    }
    let default_port: u16 = if secure { 443 } else { 80 };
    let (host, port) = if hostport.starts_with('[') {
        // Bracketed IPv6 literal, optional :port after ']'.
        let close = hostport.find(']')?;
        let host = &hostport[1..close];
        if host.is_empty() {
            return None;
        }
        let after = &hostport[close + 1..];
        if after.is_empty() {
            (host.to_string(), default_port)
        } else {
            let p = after.strip_prefix(':')?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (host.to_string(), p.parse::<u16>().ok()?)
        }
    } else if hostport.matches(':').count() > 1 {
        // Bare IPv6 literal without brackets: no port syntax is
        // possible, the whole authority is the host.
        (hostport.to_string(), default_port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                if h.is_empty() {
                    return None;
                }
                (h.to_string(), p.parse::<u16>().ok()?)
            }
            _ => (hostport.to_string(), default_port),
        }
    };
    if host.is_empty() {
        return None;
    }
    Some((secure, host, port, path))
}

/// Resolve a redirect `Location` against the URL that produced
/// it. Absolute http/https targets are taken as-is; scheme-
/// relative (`//host/…`) inherits the scheme; path-absolute
/// (`/…`) and path-relative targets resolve against the current
/// authority / path directory. Anything else (another scheme,
/// a bare query/fragment, an empty value) is refused — the
/// caller turns that into a `Failed` for the request. Scheme
/// enforcement for absolute targets happens on the next hop:
/// `split_url` refuses non-http/https before any connect.
fn resolve_redirect(current: &str, location: &str) -> Option<String> {
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    // Absolute targets: http/https (any case) taken as-is;
    // any OTHER scheme is refused outright — through 0.32 a
    // `FTP://…`-style target fell through to the relative
    // path glue and produced a mangled same-host URL
    // (AUDIT 0.32.1, P1-16).
    if let Some((scheme, _)) = location.split_once("://") {
        if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            return Some(location.to_string());
        }
        return None;
    }
    let (secure, host, port, path) = split_url(current)?;
    let scheme = if secure { "https" } else { "http" };
    let default_port: u16 = if secure { 443 } else { 80 };
    // The authority as it should reappear in a URL: bracket an
    // IPv6 host again, and spell the port only when non-default.
    let authority = if host.contains(':') {
        if port == default_port {
            format!("[{host}]")
        } else {
            format!("[{host}]:{port}")
        }
    } else if port == default_port {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    if let Some(rest) = location.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    if location.starts_with('/') {
        return Some(format!("{scheme}://{authority}{location}"));
    }
    if location.starts_with('?') || location.starts_with('#') {
        return None;
    }
    // Path-relative: resolve against the current path's
    // directory (the current query does not carry over).
    let path_only = path.split(['?', '#']).next().unwrap_or("/");
    let dir = match path_only.rfind('/') {
        Some(i) => &path_only[..=i],
        None => "/",
    };
    Some(format!("{scheme}://{authority}{dir}{location}"))
}

/// Is this status a redirect the manual follower acts on?
/// (304/305/306 are not: 304 is a cache answer the caller must
/// see, 305/306 are historic and unsafe to honor.)
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// The headers captured from every response, by name — queried
/// one by one via WINHTTP_QUERY_CUSTOM (plus Location, read on
/// redirect hops).
const CAPTURED_HEADERS: [&str; 6] = [
    "Content-Length",
    "Content-Range",
    "ETag",
    "Last-Modified",
    "Accept-Ranges",
    "Content-Disposition",
];

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn last_err(what: &str) -> String {
    use windows_sys::Win32::Foundation::GetLastError;
    // SAFETY: GetLastError is a thread-local read with no
    // preconditions; it is called immediately after the failed
    // WinHTTP call, before anything can reset it.
    let code = unsafe { GetLastError() };
    format!("{what} (winhttp error {code})")
}

/// A WinHTTP handle that closes itself. One session is opened
/// per top-level call and reused across its redirect hops;
/// connect/request handles are per hop. RAII keeps every exit
/// path leak-free, including the early `Stopped` returns.
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

/// Read one response header by name (WINHTTP_QUERY_CUSTOM),
/// with the two-call sizing pattern. `None` when the header is
/// absent (absence is the common case, not an error here) or
/// unusably large.
#[cfg(windows)]
fn query_header(request: &WinHttpHandle, name: &str) -> Option<String> {
    use windows_sys::Win32::Networking::WinHttp::{WinHttpQueryHeaders, WINHTTP_QUERY_CUSTOM};
    let name_w = wide(name);
    // SAFETY: every call below passes the live request handle
    // and the NUL-terminated name buffer, which outlives the
    // call; the sizing call's null buffer is the documented
    // pattern, and the fill call's buffer is sized from the
    // length the sizing call reported (in BYTES).
    unsafe {
        let mut len: u32 = 0;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CUSTOM,
            name_w.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
        );
        if len == 0 || (len as usize) / 2 > MAX_HEADER_CHARS {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2)];
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CUSTOM,
            name_w.as_ptr(),
            buf.as_mut_ptr() as *mut std::ffi::c_void,
            &mut len,
            std::ptr::null_mut(),
        ) == 0
        {
            return None;
        }
        let text = String::from_utf16_lossy(&buf)
            .trim_end_matches('\0')
            .trim()
            .to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

/// One hop's response with its body NOT yet read: status, the
/// by-name captured headers, the Location on a redirect, and
/// the live handles. The body is consumed by exactly one of
/// [`OpenResponse::stream`] / [`OpenResponse::discard`], or
/// abandoned by dropping the response (the redirect path).
/// Field order is drop order: the request handle closes before
/// its connect handle.
#[cfg(windows)]
struct OpenResponse {
    status: u16,
    headers: Vec<(String, String)>,
    location: Option<String>,
    request: WinHttpHandle,
    _connect: WinHttpHandle,
}

#[cfg(windows)]
impl OpenResponse {
    /// Stream the body through `sink` in ≤ 64 KiB chunks,
    /// consulting `should_stop` before every read.
    ///
    /// STALL FLOOR (AUDIT 0.32.1, P1-14): the WinHTTP
    /// timeouts are per-OPERATION (30 s receive), so a server
    /// dribbling a byte every ~29 s held a worker forever.
    /// A window of [`STALL_WINDOW_SECS`] that delivers less
    /// than [`STALL_MIN_BYTES`] aborts the transfer as
    /// stalled. The floor (~136 bit/s) sits far below any
    /// real link — the user's own connection is ~300 kbit/s
    /// — so it can only trip on a drip, never on slowness.
    fn stream(
        self,
        should_stop: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), FetchError> {
        use windows_sys::Win32::Networking::WinHttp::{WinHttpQueryDataAvailable, WinHttpReadData};
        let mut chunk = vec![0u8; CHUNK_BYTES];
        let mut window_start = std::time::Instant::now();
        let mut window_bytes: u64 = 0;
        // SAFETY: the request handle is live for the whole loop
        // (owned by `self`, dropped when this method returns);
        // the chunk buffer outlives each read.
        unsafe {
            loop {
                if should_stop() {
                    return Err(FetchError::Stopped);
                }
                let mut avail: u32 = 0;
                if WinHttpQueryDataAvailable(self.request.0, &mut avail) == 0 {
                    return Err(FetchError::Failed(last_err(
                        "WinHttpQueryDataAvailable failed",
                    )));
                }
                if avail == 0 {
                    break;
                }
                let want = avail.min(chunk.len() as u32);
                let mut read: u32 = 0;
                if WinHttpReadData(
                    self.request.0,
                    chunk.as_mut_ptr() as *mut std::ffi::c_void,
                    want,
                    &mut read,
                ) == 0
                {
                    return Err(FetchError::Failed(last_err("WinHttpReadData failed")));
                }
                if read == 0 {
                    break;
                }
                if let Err(e) = sink(&chunk[..read as usize]) {
                    return Err(FetchError::Failed(e));
                }
                window_bytes = window_bytes.saturating_add(u64::from(read));
                if window_start.elapsed() >= std::time::Duration::from_secs(STALL_WINDOW_SECS) {
                    if window_bytes < STALL_MIN_BYTES {
                        return Err(FetchError::Failed(
                            "connection stalled (no useful progress)".to_string(),
                        ));
                    }
                    window_start = std::time::Instant::now();
                    window_bytes = 0;
                }
            }
        }
        Ok(())
    }

    /// Read and throw away at most `cap` bytes, then close
    /// (dropping `self` abandons any remainder). Used by the
    /// probe's ranged fallback, whose body should be 1 byte.
    /// The caller's stop predicate is honored between chunks
    /// (AUDIT 0.32.1, P1-13).
    fn discard(self, cap: usize, should_stop: &dyn Fn() -> bool) -> Result<(), FetchError> {
        let mut seen: usize = 0;
        let mut over = false;
        let result = self.stream(should_stop, &mut |bytes| {
            seen = seen.saturating_add(bytes.len());
            if seen > cap {
                over = true;
                // Returning Err aborts the read early — the
                // handle closes on drop either way; `over`
                // distinguishes the cap from a sink failure
                // (this sink never otherwise fails).
                return Err("discard cap reached".to_string());
            }
            Ok(())
        });
        if over {
            return Ok(());
        }
        result
    }
}

/// Perform ONE request (no redirect following, no body read)
/// on the shared session: connect + open request, automatic
/// redirects switched OFF, the caller's extra headers added,
/// send + receive, then capture status and the by-name headers.
#[cfg(windows)]
fn open_request(
    session: &WinHttpHandle,
    verb: &str,
    url: &str,
    extra_headers: &[(String, String)],
    receive_timeout_ms: Option<u32>,
) -> Result<OpenResponse, FetchError> {
    use windows_sys::Win32::Networking::WinHttp::*;
    let (secure, host, port, path) =
        split_url(url).ok_or_else(|| FetchError::Failed("unsupported URL".to_string()))?;
    let host_w = wide(&host);
    let path_w = wide(&path);
    let verb_w = wide(verb);
    // SAFETY: every handle below is created by WinHTTP and
    // immediately owned by a WinHttpHandle (closed on drop);
    // all string pointers reference the NUL-terminated wide
    // buffers above, which outlive every call in this hop; null
    // optional parameters are the documented "none/default"
    // values.
    unsafe {
        let connect = WinHttpHandle(WinHttpConnect(session.0, host_w.as_ptr(), port, 0));
        if connect.0.is_null() {
            return Err(FetchError::Failed(last_err("WinHttpConnect failed")));
        }
        let flags = if secure { WINHTTP_FLAG_SECURE } else { 0 };
        let request = WinHttpHandle(WinHttpOpenRequest(
            connect.0,
            verb_w.as_ptr(),
            path_w.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            flags,
        ));
        if request.0.is_null() {
            return Err(FetchError::Failed(last_err("WinHttpOpenRequest failed")));
        }
        // Redirects are followed manually (module doc): switch
        // WinHTTP's own following off on every request. The
        // result is CHECKED (AUDIT 0.32.1, P1-15): every
        // per-hop protection in this module — scheme checks,
        // cookie recomputation, the downgrade refusal —
        // depends on automatic following actually being off;
        // if the option ever failed, WinHTTP's own follower
        // would forward the original Cookie header cross-host
        // with none of those checks. Fail closed.
        let never: u32 = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        if WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            &never as *const u32 as *const std::ffi::c_void,
            4,
        ) == 0
        {
            return Err(FetchError::Failed(last_err(
                "WinHttpSetOption(redirect policy) failed",
            )));
        }
        // Segment fetches carry a tighter receive timeout
        // (AUDIT 0.32.2, Fix 2 — see
        // SEGMENT_RECEIVE_TIMEOUT_MS). Checked fail-closed in
        // the P1-15 style, like the redirect policy above: a
        // timeout that silently did not apply would quietly
        // restore the 30 s pause latency this exists to cut.
        if let Some(timeout_ms) = receive_timeout_ms {
            if WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_RECEIVE_TIMEOUT,
                &timeout_ms as *const u32 as *const std::ffi::c_void,
                4,
            ) == 0
            {
                return Err(FetchError::Failed(last_err(
                    "WinHttpSetOption(receive timeout) failed",
                )));
            }
        }
        for (name, value) in extra_headers {
            // Header names/values come from the caller (cookie,
            // range, validator): refuse anything that could
            // smuggle a second header into the request.
            if value.contains(['\r', '\n']) || name.contains(['\r', '\n', ':']) {
                return Err(FetchError::Failed("invalid request header".to_string()));
            }
            let line = wide(&format!("{name}: {value}"));
            // The length is in CHARACTERS excluding the NUL.
            let chars = (line.len() - 1) as u32;
            if WinHttpAddRequestHeaders(
                request.0,
                line.as_ptr(),
                chars,
                WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
            ) == 0
            {
                return Err(FetchError::Failed(last_err(
                    "WinHttpAddRequestHeaders failed",
                )));
            }
        }
        if WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0 {
            return Err(FetchError::Failed(last_err("WinHttpSendRequest failed")));
        }
        if WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
            return Err(FetchError::Failed(last_err(
                "WinHttpReceiveResponse failed",
            )));
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
            return Err(FetchError::Failed(last_err(
                "WinHttpQueryHeaders(status) failed",
            )));
        }
        let status = status as u16;
        let mut headers: Vec<(String, String)> = Vec::new();
        for name in CAPTURED_HEADERS {
            if let Some(value) = query_header(&request, name) {
                headers.push((name.to_string(), value));
            }
        }
        let location = if is_redirect(status) {
            query_header(&request, "Location")
        } else {
            None
        };
        Ok(OpenResponse {
            status,
            headers,
            location,
            request,
            _connect: connect,
        })
    }
}

/// Open one WinHTTP session with Pico's identity and the
/// brief's timeouts (resolve 10 s, connect 15 s, send 30 s,
/// receive 30 s). NO decompression option is set, ever.
#[cfg(windows)]
fn open_session() -> Result<WinHttpHandle, FetchError> {
    use windows_sys::Win32::Networking::WinHttp::*;
    let ua_w = wide("Pico/0.40.2");
    // SAFETY: the agent string buffer outlives the call; null
    // proxy parameters are the documented "default proxy"
    // values, matching the sibling binding.
    let session = unsafe {
        WinHttpHandle(WinHttpOpen(
            ua_w.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ))
    };
    if session.0.is_null() {
        return Err(FetchError::Failed(last_err("WinHttpOpen failed")));
    }
    // SAFETY: the session handle is live (checked above) and
    // the timeout values are plain integers (milliseconds).
    // The result is checked (AUDIT 0.32.1, P1-15): a session
    // whose timeouts silently did not apply could hang a
    // worker far past the documented bounds.
    unsafe {
        if WinHttpSetTimeouts(session.0, 10_000, 15_000, 30_000, 30_000) == 0 {
            return Err(FetchError::Failed(last_err("WinHttpSetTimeouts failed")));
        }
    }
    Ok(session)
}

/// The origin form of a parsed URL (`scheme://host[:port]/`),
/// the cross-origin `Referer` value (AUDIT 0.40.1).
fn origin_form(secure: bool, host: &str, port: u16) -> String {
    let scheme = if secure { "https" } else { "http" };
    let default_port: u16 = if secure { 443 } else { 80 };
    let host_disp = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == default_port {
        format!("{scheme}://{host_disp}/")
    } else {
        format!("{scheme}://{host_disp}:{port}/")
    }
}

/// Scope a `Referer` for one hop (AUDIT 0.40.1): the full
/// value goes only to the referer's own origin (scheme, host,
/// port — the comparison `If-Range` now uses too); any other
/// hop receives the origin form alone, the way browsers behave
/// under strict-origin-when-cross-origin. Through 0.35.1 the
/// full page URL — path and query included — was re-sent to
/// every redirect target and to a refresh-link's new host, so
/// a token carried in the page's query leaked to third-party
/// CDNs. The host still reaches the hop, which is what
/// referer-gated download hosts actually check. An
/// unparseable referer is omitted, never sent raw.
fn scoped_referer(referer: &str, hop_url: &str) -> Option<String> {
    let (r_secure, r_host, r_port, _) = split_url(referer)?;
    match split_url(hop_url) {
        Some((secure, host, port, _))
            if secure == r_secure && port == r_port && host.eq_ignore_ascii_case(&r_host) =>
        {
            Some(referer.to_string())
        }
        _ => Some(origin_form(r_secure, &r_host, r_port)),
    }
}

/// Headers for one hop: Cookie (recomputed for THIS hop's URL),
/// plus Range / If-Range when the request carries them, plus
/// the caller's `Referer`, origin-scoped per hop (0.33 sent it
/// in full on every hop; AUDIT 0.40.1 scoped it — see
/// `scoped_referer`). The referer value is never logged here
/// (or anywhere) — headers are not log data.
fn hop_headers(
    cookie_for: &dyn Fn(&str) -> Option<String>,
    referer: Option<&str>,
    url: &str,
    range: Option<(u64, u64)>,
    if_range: Option<&str>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(cookie) = cookie_for(url) {
        if !cookie.is_empty() {
            out.push(("Cookie".to_string(), cookie));
        }
    }
    if let Some(r) = referer {
        if !r.is_empty() {
            if let Some(scoped) = scoped_referer(r, url) {
                out.push(("Referer".to_string(), scoped));
            }
        }
    }
    if let Some((start, end)) = range {
        if end == u64::MAX {
            out.push(("Range".to_string(), format!("bytes={start}-")));
        } else {
            out.push(("Range".to_string(), format!("bytes={start}-{end}")));
        }
    }
    if let Some(validator) = if_range {
        if !validator.is_empty() {
            out.push(("If-Range".to_string(), validator.to_string()));
        }
    }
    out
}

/// Turn one response's captured headers into probe facts.
/// `ranged` marks that the response answered a ranged GET:
/// only then does a 206 prove range support via Content-Range
/// (the HEAD fast path applies its own stricter rule — it also
/// requires the `Accept-Ranges: bytes` header, which
/// `accept_ranges` already reflects when present).
fn facts_from(status: u16, headers: &[(String, String)], ranged: bool) -> ProbeFacts {
    let get = |name: &str| -> Option<&str> {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let content_range = get("Content-Range").and_then(parse_content_range);
    let total = match content_range {
        Some((_, _, total)) => Some(total),
        None => get("Content-Length").and_then(|v| v.trim().parse::<u64>().ok()),
    };
    let accept_ranges = (ranged && status == 206 && content_range.is_some())
        || get("Accept-Ranges")
            .map(|v| v.trim().eq_ignore_ascii_case("bytes"))
            .unwrap_or(false);
    ProbeFacts {
        status,
        total,
        accept_ranges,
        etag: get("ETag").map(|v| v.to_string()),
        last_modified: get("Last-Modified").map(|v| v.to_string()),
    }
}

fn disposition_of(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("Content-Disposition"))
        .and_then(|(_, v)| file_name_from_disposition(v))
}

/// Follow one hop's redirect: bump the counter (cap enforced),
/// resolve the Location, and return the next URL. An
/// https → http DOWNGRADE is refused (AUDIT 0.32.1, P1-7):
/// through 0.32 the follower took any absolute target as-is,
/// so a secure download could be bounced onto plaintext —
/// non-secure cookies and the file bytes themselves riding
/// the downgrade. Upgrades (http → https) stay welcome.
fn next_hop(
    current: &str,
    location: Option<&str>,
    status: u16,
    hops: &mut usize,
) -> Result<String, FetchError> {
    *hops += 1;
    if *hops > MAX_REDIRECTS {
        return Err(FetchError::Failed("too many redirects".to_string()));
    }
    let next = resolve_redirect(current, location.unwrap_or(""))
        .ok_or_else(|| FetchError::Failed(format!("unfollowable redirect (HTTP {status})")))?;
    if let (Some((cur_secure, _, _, _)), Some((next_secure, _, _, _))) =
        (split_url(current), split_url(&next))
    {
        if cur_secure && !next_secure {
            return Err(FetchError::Failed(
                "redirect from https to http refused".to_string(),
            ));
        }
    }
    Ok(next)
}

/// Probe a URL: HEAD first (no body). The HEAD answer is final
/// when it returns 200 with a Content-Length AND
/// `Accept-Ranges: bytes`. A HEAD status of 405/501, a HEAD
/// answer lacking Content-Length (no usable total), or a
/// transport-level HEAD failure falls back to a ranged GET
/// `bytes=0-0` whose body is read and discarded (≤ 64 KiB, then
/// the handle closes; the Range response should be 1 byte).
/// Facts: total from Content-Range when 206, else
/// Content-Length; range support from a 206 ranged answer or
/// an `Accept-Ranges: bytes` header.
/// A finished HEAD chain: (status, captured headers, final URL).
#[cfg(windows)]
type HeadAnswer = (u16, Vec<(String, String)>, String);

#[cfg(windows)]
pub fn probe(
    url: &str,
    cookie_for: &dyn Fn(&str) -> Option<String>,
    referer: Option<&str>,
    should_stop: &dyn Fn() -> bool,
) -> Result<ProbeResult, FetchError> {
    let session = open_session()?;
    // ---- HEAD, following redirects manually. ----
    let mut current = url.to_string();
    let mut hops = 0usize;
    let mut head: Option<HeadAnswer> = None;
    let mut head_failed = false;
    loop {
        // AUDIT 0.32.1 (P1-13): the probe honors the task's
        // stop control between hops — through 0.32 a pause or
        // cancel during probing waited out the whole redirect
        // chain (up to 10 hops × per-hop timeouts, twice).
        if should_stop() {
            return Err(FetchError::Stopped);
        }
        if split_url(&current).is_none() {
            return Err(FetchError::Failed("unsupported URL".to_string()));
        }
        let headers = hop_headers(cookie_for, referer, &current, None, None);
        match open_request(&session, "HEAD", &current, &headers, None) {
            Ok(resp) => {
                if is_redirect(resp.status) {
                    let status = resp.status;
                    let location = resp.location.clone();
                    drop(resp);
                    current = next_hop(&current, location.as_deref(), status, &mut hops)?;
                    continue;
                }
                head = Some((resp.status, resp.headers, current.clone()));
                break;
            }
            Err(FetchError::Failed(_)) => {
                // Transport-level HEAD failure → the brief's
                // ranged-GET fallback.
                head_failed = true;
                break;
            }
            Err(FetchError::Stopped) => {
                // open_request has no stop channel; unreachable,
                // treated like a transport failure.
                head_failed = true;
                break;
            }
        }
    }
    if !head_failed {
        if let Some((status, headers, final_url)) = head {
            let facts = facts_from(status, &headers, false);
            let has_length = headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("Content-Length"));
            let head_is_final = status == 200 && has_length && facts.accept_ranges;
            // AUDIT 0.32.1 (P1-6): a HEAD claiming
            // Content-Length 0 is NOT final either — a hostile
            // or buggy server answering that for a non-empty
            // resource made the manager "complete" a 0-byte
            // file without ever issuing a GET. The ranged
            // probe cross-checks: a truly empty file answers
            // it consistently (200 with length 0, or 416).
            let head_unusable =
                matches!(status, 405 | 501) || !has_length || facts.total == Some(0);
            if head_is_final || !head_unusable {
                return Ok(ProbeResult {
                    disposition_name: disposition_of(&headers),
                    facts,
                    final_url,
                });
            }
            // Unusable HEAD (405/501, no Content-Length, or a
            // distrusted zero length): fall through to the
            // ranged GET from the ORIGINAL URL — its own
            // redirect chain is the honest one to measure.
        }
    }
    // ---- Ranged GET bytes=0-0, body discarded. ----
    let mut current = url.to_string();
    let mut hops = 0usize;
    loop {
        if should_stop() {
            return Err(FetchError::Stopped);
        }
        if split_url(&current).is_none() {
            return Err(FetchError::Failed("unsupported URL".to_string()));
        }
        let headers = hop_headers(cookie_for, referer, &current, Some((0, 0)), None);
        let resp = open_request(&session, "GET", &current, &headers, None)?;
        if is_redirect(resp.status) {
            let status = resp.status;
            let location = resp.location.clone();
            drop(resp);
            current = next_hop(&current, location.as_deref(), status, &mut hops)?;
            continue;
        }
        let facts = facts_from(resp.status, &resp.headers, true);
        let disposition_name = disposition_of(&resp.headers);
        resp.discard(PROBE_DISCARD_CAP, should_stop)?;
        return Ok(ProbeResult {
            facts,
            final_url: current,
            disposition_name,
        });
    }
}

/// May this response's body be streamed into the file?
/// The pre-body judgment (AUDIT 0.32.1, P1-1/P1-4): the
/// status and headers are known BEFORE a single body byte
/// is read, so they are judged there —
///
/// - ranged request: only a 206 whose `Content-Range`
///   start AND end equal the requested range and whose
///   total equals `expected_total` (when the task knows
///   one). Through 0.32 only the start was ever compared,
///   and only after the body had already been written to
///   disk and counted — an error page could be spliced into
///   a "completed" file, and two versions of a file could
///   be spliced into each other.
/// - whole-file request: only a 200.
///
/// Anything else surfaces as an outcome with NO bytes
/// streamed; the caller judges the status (200-to-ranged =
/// range death, 416, retries) exactly as before, but no
/// unjudged byte can ever reach the file or the progress
/// count.
fn body_may_stream(
    status: u16,
    headers: &[(String, String)],
    range: Option<(u64, u64)>,
    expected_total: Option<u64>,
) -> bool {
    match range {
        Some((start, end)) => {
            if status != 206 {
                return false;
            }
            let content_range = headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("Content-Range"))
                .and_then(|(_, v)| parse_content_range(v));
            match content_range {
                Some((cs, ce, ct)) => {
                    cs == start && ce == end && expected_total.is_none_or(|t| ct == t)
                }
                None => false,
            }
        }
        None => status == 200,
    }
}

/// Fetch one byte range (or the whole file when `range` is
/// `None`), streaming the FINAL hop's body through `sink` in
/// ≤ 64 KiB chunks — but only when the response passes the
/// pre-body judgment ([`body_may_stream`]); a refused body
/// is never read and the outcome still surfaces the raw
/// status for the caller to judge. `should_stop` is
/// consulted before every hop and every read and yields
/// [`FetchError::Stopped`]. `range` is (start, end
/// INCLUSIVE); an end of `u64::MAX` spells an open-ended
/// range (`bytes=start-`). `expected_total` is the task's
/// known total, cross-checked against `Content-Range`.
/// Redirects are followed manually (cap 10, https → http
/// refused) and their bodies are never read; `Range` and a
/// per-hop recomputed `Cookie` are sent on every hop, while
/// `If-Range` — the ORIGINAL resource's validator — is sent
/// only to the origin (AUDIT 0.32.1, P1-7; the comparison
/// became the full scheme/host/port tuple in AUDIT 0.40.1).
#[cfg(windows)]
pub fn fetch_range(
    url: &str,
    range: Option<(u64, u64)>,
    if_range: Option<&str>,
    expected_total: Option<u64>,
    cookie_for: &dyn Fn(&str) -> Option<String>,
    referer: Option<&str>,
    should_stop: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<FetchOutcome, FetchError> {
    let session = open_session()?;
    // AUDIT 0.40.1: If-Range scoping compares the full origin
    // tuple (scheme, host, port). Through 0.35.1 it compared
    // the port-stripped host only, so the original validator
    // could be sent to the same hostname on a different port
    // or scheme — a different service that happens to share
    // the name.
    let origin = split_url(url);
    let mut current = url.to_string();
    let mut hops = 0usize;
    loop {
        if should_stop() {
            return Err(FetchError::Stopped);
        }
        let Some(current_parts) = split_url(&current) else {
            return Err(FetchError::Failed("unsupported URL".to_string()));
        };
        let same_origin = match (&origin, &current_parts) {
            (Some((o_secure, o_host, o_port, _)), (secure, host, port, _)) => {
                secure == o_secure && port == o_port && host.eq_ignore_ascii_case(o_host)
            }
            _ => false,
        };
        let if_range_here = if if_range.is_some() && same_origin {
            if_range
        } else {
            None
        };
        let headers = hop_headers(cookie_for, referer, &current, range, if_range_here);
        let resp = open_request(
            &session,
            "GET",
            &current,
            &headers,
            Some(SEGMENT_RECEIVE_TIMEOUT_MS),
        )?;
        if is_redirect(resp.status) {
            let status = resp.status;
            let location = resp.location.clone();
            drop(resp);
            current = next_hop(&current, location.as_deref(), status, &mut hops)?;
            continue;
        }
        let outcome = FetchOutcome {
            status: resp.status,
            final_url: current.clone(),
            headers: resp.headers.clone(),
        };
        if body_may_stream(resp.status, &resp.headers, range, expected_total) {
            // AUDIT 0.40.1: the stream carries a hard byte
            // budget. The headers were validated against the
            // requested range before streaming, but through
            // 0.35.1 nothing counted the body itself: a server
            // answering 206 with a CORRECT Content-Range and
            // then an endless body streamed forever — and the
            // resume spot-check's sink buffered it all in RAM.
            // A bounded range now aborts the moment the server
            // sends one byte more than it promised, with the
            // same sentinel the manager's own sink cap uses, so
            // failure classification is unchanged.
            match range {
                Some((start, end)) if end != u64::MAX && end >= start => {
                    let budget = end - start + 1;
                    let mut delivered: u64 = 0;
                    let mut budgeted = |chunk: &[u8]| -> Result<(), String> {
                        delivered = delivered.saturating_add(chunk.len() as u64);
                        if delivered > budget {
                            return Err(RANGE_OVERFLOW.to_string());
                        }
                        sink(chunk)
                    };
                    resp.stream(should_stop, &mut budgeted)?;
                }
                _ => {
                    resp.stream(should_stop, sink)?;
                }
            }
        }
        return Ok(outcome);
    }
}

// ---------------------------------------------------------------------------
// Non-Windows stubs: the bin only ever runs on Windows (the
// transport is WinHTTP by definition), but keeping the API
// present lets the module type-check anywhere.
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
pub fn probe(
    _url: &str,
    _cookie_for: &dyn Fn(&str) -> Option<String>,
    _referer: Option<&str>,
    _should_stop: &dyn Fn() -> bool,
) -> Result<ProbeResult, FetchError> {
    Err(FetchError::Failed(
        "winhttp transport unavailable on this platform".to_string(),
    ))
}

#[cfg(not(windows))]
pub fn fetch_range(
    _url: &str,
    _range: Option<(u64, u64)>,
    _if_range: Option<&str>,
    _expected_total: Option<u64>,
    _cookie_for: &dyn Fn(&str) -> Option<String>,
    _referer: Option<&str>,
    _should_stop: &dyn Fn() -> bool,
    _sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<FetchOutcome, FetchError> {
    Err(FetchError::Failed(
        "winhttp transport unavailable on this platform".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{origin_form, scoped_referer, split_url};

    #[test]
    fn split_url_refuses_raw_controls_and_spaces() {
        // AUDIT 0.40.1: a raw CR/LF/space/DEL anywhere in the
        // URL is refused at the transport choke point.
        assert!(split_url("https://example.com/a b").is_none());
        assert!(split_url("https://example.com/a\r\nb").is_none());
        assert!(split_url("https://exa\tmple.com/").is_none());
        assert!(split_url("https://example.com/\u{7f}").is_none());
        assert!(split_url("https://example.com/a%20b").is_some());
        assert!(split_url("https://example.com:8443/f?x=1").is_some());
    }

    #[test]
    fn origin_form_elides_default_ports_and_brackets_ipv6() {
        assert_eq!(
            origin_form(true, "example.com", 443),
            "https://example.com/"
        );
        assert_eq!(origin_form(false, "example.com", 80), "http://example.com/");
        assert_eq!(
            origin_form(true, "example.com", 8443),
            "https://example.com:8443/"
        );
        assert_eq!(origin_form(true, "::1", 443), "https://[::1]/");
    }

    #[test]
    fn referer_is_full_only_on_its_own_origin() {
        // AUDIT 0.40.1: full referer same-origin; origin-only
        // everywhere else (different host, port, or scheme).
        let page = "https://soft98.ir/dl/page?token=SECRET";
        assert_eq!(
            scoped_referer(page, "https://soft98.ir/files/x.zip").as_deref(),
            Some(page)
        );
        assert_eq!(
            scoped_referer(page, "https://cdn.example.net/x.zip").as_deref(),
            Some("https://soft98.ir/")
        );
        assert_eq!(
            scoped_referer(page, "https://soft98.ir:8443/x.zip").as_deref(),
            Some("https://soft98.ir/")
        );
        assert_eq!(
            scoped_referer(page, "http://soft98.ir/x.zip").as_deref(),
            Some("https://soft98.ir/")
        );
        // An unparseable referer is omitted, never sent raw.
        assert_eq!(scoped_referer("not a url", "https://x.test/"), None);
    }
}
