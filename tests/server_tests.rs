//! Static-server tests over real TCP sockets, plus the Range parser
//! matrix and the root lifecycle. The server is the security
//! boundary of the whole app, so most of the hostile corpus lives
//! here.

use pico_core::core::server::{
    is_served_url, parse_range, parse_request_head, validate_host_header, FileServer, RangeOutcome,
};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

struct Fixture {
    dir: PathBuf,
    server: FileServer,
    root: u64,
}

fn fixture(tag: &str) -> Fixture {
    let base = std::env::temp_dir().join(format!("pico-v15-srv-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let site = base.join("site");
    std::fs::create_dir_all(site.join("sub")).expect("mkdir site");
    std::fs::write(
        site.join("index.html"),
        "<!DOCTYPE html><title>Fixture</title><p>hello</p>",
    )
    .expect("index");
    std::fs::write(site.join("app.js"), "console.log(1);").expect("js");
    let media: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(site.join("media.bin"), &media).expect("media");
    std::fs::write(site.join("sub").join("page.html"), "<title>Sub</title>").expect("sub page");
    // A secret OUTSIDE the served root: traversal must never reach it.
    std::fs::write(base.join("secret.txt"), "TOP-SECRET").expect("secret");
    let server = FileServer::start().expect("server starts");
    let root = server.register_root(&site).expect("register root");
    Fixture {
        dir: base,
        server,
        root,
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn request(fx: &Fixture, raw: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", fx.server.port())).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(raw.as_bytes()).expect("write request");
    let mut out = String::new();
    // Responses in these tests either close or carry a length; read
    // until EOF or timeout (timeout yields what we have).
    let _ = s.read_to_string(&mut out);
    out
}

fn get(fx: &Fixture, path: &str) -> String {
    request(
        fx,
        &format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            fx.server.port()
        ),
    )
}

fn status_of(resp: &str) -> u16 {
    resp.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn header_of<'a>(resp: &'a str, name: &str) -> Option<&'a str> {
    let head = resp.split("\r\n\r\n").next().unwrap_or("");
    for line in head.split("\r\n").skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim());
            }
        }
    }
    None
}

fn body_of(resp: &str) -> &str {
    resp.split("\r\n\r\n").nth(1).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn serves_file_with_security_headers_and_no_cors() {
    let fx = fixture("basic");
    let path = format!("/r/{}/app.js", fx.root);
    let resp = get(&fx, &path);
    assert_eq!(status_of(&resp), 200);
    assert_eq!(body_of(&resp), "console.log(1);");
    assert_eq!(header_of(&resp, "X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(header_of(&resp, "Cache-Control"), Some("no-store"));
    assert_eq!(header_of(&resp, "Referrer-Policy"), Some("no-referrer"));
    assert!(
        header_of(&resp, "Access-Control-Allow-Origin").is_none(),
        "no CORS: other origins must not read local files"
    );
    assert_eq!(
        header_of(&resp, "Content-Type"),
        Some("text/javascript; charset=utf-8")
    );
}

#[test]
fn html_gets_live_reload_script_for_its_own_root() {
    let fx = fixture("inject");
    let resp = get(&fx, &format!("/r/{}/index.html", fx.root));
    assert_eq!(status_of(&resp), 200);
    let expected = format!("/__pico/events/{}", fx.root);
    assert!(
        body_of(&resp).contains(&expected),
        "injected script must subscribe to this root's stream, got: {}",
        body_of(&resp)
    );
    // Non-HTML is never rewritten.
    let resp = get(&fx, &format!("/r/{}/app.js", fx.root));
    assert!(!body_of(&resp).contains("__pico"));
}

#[test]
fn missing_file_is_404_and_bad_method_is_405() {
    let fx = fixture("status");
    let resp = get(&fx, &format!("/r/{}/nope.txt", fx.root));
    assert_eq!(status_of(&resp), 404);
    let resp = request(
        &fx,
        &format!(
            "POST /r/{}/app.js HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\n\r\n",
            fx.root,
            fx.server.port()
        ),
    );
    assert_eq!(status_of(&resp), 405);
}

#[test]
fn head_request_sends_headers_only() {
    let fx = fixture("head");
    let path = format!("/r/{}/media.bin", fx.root);
    let resp = request(
        &fx,
        &format!(
            "HEAD {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            fx.server.port()
        ),
    );
    assert_eq!(status_of(&resp), 200);
    assert_eq!(header_of(&resp, "Content-Length"), Some("1000"));
    assert_eq!(body_of(&resp), "");
}

// ---------------------------------------------------------------------------
// Host wall (anti DNS-rebinding) and malformed heads
// ---------------------------------------------------------------------------

#[test]
fn host_header_wall() {
    let fx = fixture("hostwall");
    let path = format!("/r/{}/app.js", fx.root);
    let port = fx.server.port();
    for host in [
        format!("evil.example:{port}"),
        format!("127.0.0.1:{}", port + 1),
        format!("user@127.0.0.1:{port}"),
        format!("localhost:{}", port + 1),
        format!("127.0.0.2:{port}"),
    ] {
        let resp = request(
            &fx,
            &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
        );
        assert!(
            [400, 403].contains(&status_of(&resp)),
            "Host {host} must be refused, got {}",
            status_of(&resp)
        );
    }
    // Missing Host on HTTP/1.1: 400.
    let resp = request(
        &fx,
        &format!("GET {path} HTTP/1.1\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(status_of(&resp), 400);
    // localhost IS an accepted name (dev servers print it), ::1 is a
    // loopback name, and a portless loopback Host is tolerated (the
    // wall's job is refusing *foreign* names, which the cases above
    // prove).
    for host in [format!("localhost:{port}"), format!("[::1]:{port}")] {
        let resp = request(
            &fx,
            &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
        );
        assert_eq!(status_of(&resp), 200, "Host {host} must be accepted");
    }
}

#[test]
fn garbage_and_oversized_heads_are_refused() {
    let fx = fixture("garbage");
    let resp = request(&fx, "WAT IS THIS\r\n\r\n");
    assert_eq!(status_of(&resp), 400);
    let big = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Pad: {}\r\n\r\n",
        fx.server.port(),
        "A".repeat(20_000)
    );
    let resp = request(&fx, &big);
    assert_eq!(status_of(&resp), 431, "oversized head -> 431");
    // Absolute-form request targets are not proxies' business here.
    let resp = request(
        &fx,
        &format!(
            "GET http://127.0.0.1:{}/r/{}/app.js HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
            fx.server.port(),
            fx.root,
            fx.server.port()
        ),
    );
    assert_ne!(status_of(&resp), 200);
}

// ---------------------------------------------------------------------------
// Path traversal corpus
// ---------------------------------------------------------------------------

#[test]
fn traversal_corpus_never_escapes_the_root() {
    let fx = fixture("traversal");
    let t = fx.root;
    let attempts = [
        format!("/r/{t}/../secret.txt"),
        format!("/r/{t}/%2e%2e/secret.txt"),
        format!("/r/{t}/%2E%2E%2Fsecret.txt"),
        format!("/r/{t}/sub/../../secret.txt"),
        format!("/r/{t}/..%5csecret.txt"),
        format!("/r/{t}/..\\secret.txt"),
        format!("/r/{t}/%2e%2e%5csecret.txt"),
        format!("/r/{t}//../secret.txt"),
        format!("/r/{t}/....//secret.txt"),
        format!("/r/{t}/index.html%00.png"),
        format!("/r/{t}/app.js::$DATA"),
        format!("/r/{t}/CON.txt"),
        format!("/r/{t}/sub/%2e%2e/%2e%2e/secret.txt"),
    ];
    for path in &attempts {
        let resp = get(&fx, path);
        assert_ne!(status_of(&resp), 200, "{path} must not be served");
        assert!(
            !resp.contains("TOP-SECRET"),
            "{path} leaked the outside secret"
        );
    }
    // And the legitimate nested path still works.
    let resp = get(&fx, &format!("/r/{t}/sub/page.html"));
    assert_eq!(status_of(&resp), 200);
}

// ---------------------------------------------------------------------------
// Keep-alive (AUDIT M4)
// ---------------------------------------------------------------------------

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_response(s: &mut TcpStream) -> Response {
    use std::io::BufReader;
    let mut reader = BufReader::new(s.try_clone().expect("clone stream"));
    let mut head = Vec::new();
    // Read one byte at a time to the header terminator: tests are not
    // the hot path, and this never over-reads into the body.
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = reader.read(&mut byte).expect("read head byte");
        assert!(n == 1, "EOF inside response head");
        head.push(byte[0]);
        assert!(head.len() < 64 * 1024);
    }
    let head_text = String::from_utf8_lossy(&head).into_owned();
    let status: u16 = head_text
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut headers = Vec::new();
    let mut len = 0usize;
    for line in head_text.split("\r\n").skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            if k == "content-length" {
                len = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).expect("read body");
    Response {
        status,
        headers,
        body,
    }
}

fn conn_header(r: &Response) -> Option<&str> {
    r.headers
        .iter()
        .find(|(k, _)| k == "connection")
        .map(|(_, v)| v.as_str())
}

#[test]
fn keep_alive_serves_two_requests_on_one_connection() {
    let fx = fixture("keepalive");
    let port = fx.server.port();
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let path = format!("/r/{}/app.js", fx.root);
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
        .expect("write 1");
    let r1 = read_response(&mut s);
    assert_eq!(r1.status, 200);
    assert_eq!(r1.body, b"console.log(1);");
    assert_eq!(conn_header(&r1), Some("keep-alive"));
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )
    .expect("write 2");
    let r2 = read_response(&mut s);
    assert_eq!(r2.status, 200);
    assert_eq!(conn_header(&r2), Some("close"));
    // Server closes after a close-token response.
    let mut b = [0u8; 1];
    let n = s.read(&mut b).expect("read eof");
    assert_eq!(n, 0, "server must close after Connection: close");
}

#[test]
fn http10_closes_by_default() {
    let fx = fixture("http10");
    let resp = request(
        &fx,
        &format!(
            "GET /r/{}/app.js HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n",
            fx.root,
            fx.server.port()
        ),
    );
    assert_eq!(status_of(&resp), 200);
    assert_eq!(header_of(&resp, "Connection"), Some("close"));
}

#[test]
fn silent_peer_is_cut_by_timeout() {
    let fx = fixture("slowloris");
    let mut s = TcpStream::connect(("127.0.0.1", fx.server.port())).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(8))).ok();
    // Partial head, then silence: the server's header read timeout
    // (3 s) must end the connection, not a slot held forever.
    s.write_all(b"GET /r/ HTTP/1.1\r\nHo")
        .expect("write partial");
    let started = std::time::Instant::now();
    let mut buf = [0u8; 64];
    // EOF (Ok(0)) or a reset both prove the server let go; a read
    // timeout here would mean it did not.
    let closed = loop {
        match s.read(&mut buf) {
            Ok(0) => break true,
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break false,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break false,
            Err(_) => break true,
        }
    };
    assert!(closed, "server never closed the silent connection");
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "server held a silent connection for {:?}",
        started.elapsed()
    );
}

// ---------------------------------------------------------------------------
// Range (AUDIT M3)
// ---------------------------------------------------------------------------

#[test]
fn range_parser_matrix() {
    use RangeOutcome::*;
    assert_eq!(parse_range("bytes=0-99", 1000), Slice(0, 99));
    assert_eq!(parse_range("bytes=100-", 1000), Slice(100, 999));
    assert_eq!(parse_range("bytes=-100", 1000), Slice(900, 999));
    assert_eq!(parse_range("bytes=0-99999", 1000), Slice(0, 999));
    assert_eq!(parse_range("bytes=-2000", 1000), Slice(0, 999));
    assert_eq!(parse_range("bytes=999-999", 1000), Slice(999, 999));
    assert_eq!(parse_range("bytes=1000-", 1000), Unsatisfiable);
    assert_eq!(parse_range("bytes=5-3", 1000), Unsatisfiable);
    assert_eq!(parse_range("bytes=-0", 1000), Unsatisfiable);
    assert_eq!(parse_range("bytes=0-0", 0), Unsatisfiable);
    assert_eq!(parse_range("bytes=0-1,5-6", 1000), Ignore);
    assert_eq!(parse_range("items=0-1", 1000), Ignore);
    assert_eq!(parse_range("bytes=abc-def", 1000), Ignore);
    assert_eq!(parse_range("bytes=-", 1000), Ignore);
    assert_eq!(parse_range("bytes=", 1000), Ignore);
    assert_eq!(parse_range("", 1000), Ignore);
}

#[test]
fn range_requests_over_the_wire() {
    let fx = fixture("rangewire");
    let port = fx.server.port();
    let path = format!("/r/{}/media.bin", fx.root);
    let resp = request(
        &fx,
        &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=10-19\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(status_of(&resp), 206);
    assert_eq!(header_of(&resp, "Content-Range"), Some("bytes 10-19/1000"));
    assert_eq!(header_of(&resp, "Accept-Ranges"), Some("bytes"));
    let body = body_of(&resp);
    assert_eq!(body.len(), 10);
    // media.bin byte i = i % 251, so bytes 10..=19 are 10..=19.
    assert_eq!(body.as_bytes(), (10u8..=19).collect::<Vec<_>>().as_slice());

    let resp = request(
        &fx,
        &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=5000-\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(status_of(&resp), 416);
    assert_eq!(header_of(&resp, "Content-Range"), Some("bytes */1000"));
}

#[test]
fn range_is_suppressed_for_injectable_html() {
    let fx = fixture("rangehtml");
    let port = fx.server.port();
    let resp = request(
        &fx,
        &format!(
            "GET /r/{}/index.html HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=0-4\r\nConnection: close\r\n\r\n",
            fx.root
        ),
    );
    assert_eq!(status_of(&resp), 200, "partial HTML must not be served");
    assert!(body_of(&resp).contains("__pico/events"));
}

// ---------------------------------------------------------------------------
// SSE per-root isolation + lifecycle (AUDIT M1/M2)
// ---------------------------------------------------------------------------

fn sse_connect(fx: &Fixture, root: u64) -> TcpStream {
    let port = fx.server.port();
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect sse");
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    s.write_all(
        format!("GET /__pico/events/{root} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes(),
    )
    .expect("write sse request");
    s
}

fn read_until(s: &mut TcpStream, needle: &str, timeout: Duration) -> String {
    s.set_read_timeout(Some(timeout)).ok();
    let mut out = String::new();
    let mut buf = [0u8; 512];
    let deadline = std::time::Instant::now() + timeout;
    while !out.contains(needle) && std::time::Instant::now() < deadline {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => break,
        }
    }
    out
}

#[test]
fn sse_broadcast_reaches_only_its_own_root() {
    let fx = fixture("sse");
    let other_dir = fx.dir.join("other");
    std::fs::create_dir_all(&other_dir).expect("mkdir other");
    std::fs::write(other_dir.join("x.html"), "<title>x</title>").expect("x");
    let root_b = fx.server.register_root(&other_dir).expect("register b");

    let mut a = sse_connect(&fx, fx.root);
    let hello = read_until(&mut a, "pico live reload", Duration::from_secs(3));
    assert!(hello.contains("200"), "SSE handshake: {hello}");
    let mut b = sse_connect(&fx, root_b);
    let _ = read_until(&mut b, "pico live reload", Duration::from_secs(3));

    fx.server.reload_hub().broadcast(fx.root);
    let got_a = read_until(&mut a, "data: reload", Duration::from_secs(3));
    assert!(got_a.contains("data: reload"), "root A hears its broadcast");
    let got_b = read_until(&mut b, "data: reload", Duration::from_millis(400));
    assert!(
        !got_b.contains("data: reload"),
        "root B must NOT hear root A's broadcast (got {got_b:?})"
    );

    fx.server.reload_hub().broadcast(root_b);
    let got_b = read_until(&mut b, "data: reload", Duration::from_secs(3));
    assert!(got_b.contains("data: reload"), "root B hears its own");
    fx.server.release_root(root_b);
}

#[test]
fn v29_dropped_root_refuses_late_subscribes() {
    // A5-6: a subscribe that lands after its root was dropped
    // must be refused (tombstone), not parked forever in a
    // resurrected map entry.
    let hub = pico_core::core::server::ReloadHub::new();
    let live = hub.try_subscribe(111);
    assert!(live.is_some(), "a live root subscribes normally");
    drop(live);
    hub.drop_root(222);
    assert!(
        hub.try_subscribe(222).is_none(),
        "a dropped root refuses late subscribes"
    );
    assert_eq!(hub.subscriber_count(222), 0);
    // An unrelated root is unaffected by another's tombstone.
    assert!(hub.try_subscribe(333).is_some());
}

#[test]
fn root_lifecycle_register_release_and_refcounts() {
    let fx = fixture("lifecycle");
    let path = format!("/r/{}/app.js", fx.root);
    assert_eq!(status_of(&get(&fx, &path)), 200);

    // Re-registering the same folder dedupes to the same token with
    // a second reference.
    let site = fx.dir.join("site");
    let again = fx.server.register_root(&site).expect("re-register");
    assert_eq!(again, fx.root);
    assert_eq!(fx.server.root_refs(fx.root), Some(2));

    fx.server.release_root(fx.root);
    assert_eq!(status_of(&get(&fx, &path)), 200, "one ref remains");

    fx.server.release_root(fx.root);
    assert_eq!(fx.server.root_refs(fx.root), None);
    assert_eq!(status_of(&get(&fx, &path)), 404, "last release unregisters");
    // The events endpoint dies with the root, too.
    let resp = get(&fx, &format!("/__pico/events/{}", fx.root));
    assert_eq!(status_of(&resp), 404);
    // Releasing a dead token is a no-op, never a panic.
    fx.server.release_root(fx.root);
    fx.server.release_root(123_456_789);
}

#[test]
fn reserved_pico_paths_never_hit_the_filesystem() {
    let fx = fixture("reserved");
    for p in ["/__pico/events", "/__pico/events/abc", "/__pico/whatever"] {
        let resp = get(&fx, p);
        assert_eq!(status_of(&resp), 404, "{p}");
    }
}

#[test]
fn url_parts_handles_r_inside_paths() {
    // AUDIT L14: a file path that itself contains "/r/" segments must
    // parse structurally, not by substring splitting.
    let fx = fixture("urlparts");
    let url = fx.server.url_for(fx.root, "sub/r/page.html");
    assert_eq!(
        fx.server.url_parts(&url),
        Some((fx.root, "sub/r/page.html".to_string()))
    );
    assert_eq!(fx.server.root_id_for_url(&url), Some(fx.root));
    assert_eq!(fx.server.url_parts("http://example.com/r/1/x"), None);
    assert_eq!(
        fx.server
            .url_parts(&format!("http://127.0.0.1:{}/r/abc/x", fx.server.port())),
        None,
        "non-numeric token is not a root URL"
    );
}

#[test]
fn registering_a_missing_folder_fails() {
    let server = FileServer::start().expect("server");
    let missing = std::env::temp_dir().join("pico-v15-definitely-missing-xyz");
    assert!(server.register_root(&missing).is_err());
}

#[test]
fn directory_urls_serve_index_and_queries_are_ignored() {
    let fx = fixture("dirindex");
    let resp = get(&fx, &format!("/r/{}/", fx.root));
    assert_eq!(status_of(&resp), 200);
    assert!(body_of(&resp).contains("hello"));
    // Cache-busting query strings must not break file lookup.
    let resp = get(&fx, &format!("/r/{}/app.js?v=12345&t=1", fx.root));
    assert_eq!(status_of(&resp), 200);
    assert_eq!(body_of(&resp), "console.log(1);");
}

#[test]
fn percent_escaped_names_resolve() {
    let fx = fixture("escaped");
    let site = fx.dir.join("site");
    std::fs::write(site.join("my page.html"), "<title>Spaced</title>").expect("write spaced");
    let resp = get(&fx, &format!("/r/{}/my%20page.html", fx.root));
    assert_eq!(status_of(&resp), 200);
    assert!(body_of(&resp).contains("Spaced"));
    // Double-encoding is decoded ONCE: %252e is a literal "%2e"
    // filename, not a dot — it must not become a traversal.
    let resp = get(&fx, &format!("/r/{}/%252e%252e/secret.txt", fx.root));
    assert_ne!(status_of(&resp), 200);
    assert!(!resp.contains("TOP-SECRET"));
}

#[test]
fn connection_cap_sheds_load_and_recovers() {
    // 64 concurrent connections is the gate. Fill it with silent
    // sockets: a real request must then wait (no response), and once
    // the flood drains the server answers again. The flood sockets
    // also die on their own via the 3 s header timeout — the test
    // just has to act inside that window.
    let fx = fixture("conncap");
    let port = fx.server.port();
    let mut flood = Vec::new();
    for _ in 0..64 {
        flood.push(TcpStream::connect(("127.0.0.1", port)).expect("flood connect"));
    }
    std::thread::sleep(Duration::from_millis(150));
    let mut probe = TcpStream::connect(("127.0.0.1", port)).expect("probe connect");
    probe
        .set_read_timeout(Some(Duration::from_millis(700)))
        .ok();
    probe
        .write_all(
            format!(
                "GET /r/{}/app.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",
                fx.root
            )
            .as_bytes(),
        )
        .expect("probe write");
    // The property under test is "the 65th connection is never
    // SERVED" — not which TCP shape the refusal takes. The server
    // sheds by accepting and immediately dropping, and all three
    // observable outcomes are correct refusals: RST (the drop raced
    // the probe's request bytes, so the close carries unread data),
    // clean EOF (FIN before the bytes landed), or silence until the
    // read timeout (the probe never left the listen backlog). The
    // v15 test asserted `read().is_err()`, which only two of the
    // three produce — it flaked on loaded hosts (AUDIT V16-3).
    // Assert the real invariant instead: not one response byte.
    let mut received = 0usize;
    let mut buf = [0u8; 64];
    loop {
        match probe.read(&mut buf) {
            Ok(0) => break, // EOF: refused
            Ok(n) => {
                received += n;
                if received > 0 {
                    break; // served — the failure case, asserted below
                }
            }
            Err(_) => break, // timeout or reset: refused
        }
    }
    assert_eq!(
        received, 0,
        "65th connection must not be served while capped"
    );
    drop(probe);
    drop(flood);
    // Slots free up: the same request now succeeds.
    std::thread::sleep(Duration::from_millis(200));
    let resp = get(&fx, &format!("/r/{}/app.js", fx.root));
    assert_eq!(status_of(&resp), 200);
}

#[test]
fn sse_subscriber_cap_is_enforced_per_root() {
    let fx = fixture("sse-cap");
    let mut conns = Vec::new();
    for _ in 0..8 {
        let mut s = sse_connect(&fx, fx.root);
        let hello = read_until(&mut s, "pico live reload", Duration::from_secs(3));
        assert!(hello.contains("200"), "subscriber within cap: {hello}");
        conns.push(s);
    }
    // The 9th subscriber to the same root is refused (503), not queued.
    let port = fx.server.port();
    let resp = request(
        &fx,
        &format!(
            "GET /__pico/events/{} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",
            fx.root
        ),
    );
    assert_eq!(status_of(&resp), 503);
    drop(conns);
}

// ---------------------------------------------------------------------------
// Host-wall parser matrix (v15 re-run of the threat model): the pure
// parser is where duplicate/absolute-form/whitespace tricks die.
// ---------------------------------------------------------------------------

#[test]
fn host_wall_parser_matrix() {
    let port = 4321u16;
    // Exactly one Host header: a second one is a smuggling shape.
    let dup = format!(
        "GET /r/1/app.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    );
    assert_eq!(
        parse_request_head(dup.as_bytes(), port),
        Err(400),
        "duplicate Host must be refused"
    );
    // Absolute-form targets are refused even with a perfect Host.
    let abs = format!(
        "GET http://127.0.0.1:{port}/r/1/app.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    );
    assert_eq!(
        parse_request_head(abs.as_bytes(), port),
        Err(400),
        "absolute-form must be refused"
    );
    // Network-path target (//host/...) likewise.
    let netpath = "GET //evil.example/r/1/app.js HTTP/1.1\r\nHost: 127.0.0.1:4321\r\n\r\n";
    assert_eq!(parse_request_head(netpath.as_bytes(), port), Err(400));
    // Header line folding (obs-fold) is refused.
    let folded = "GET /r/1/app.js HTTP/1.1\r\nHost: 127.0.0.1:4321\r\n X-Injected: 1\r\n\r\n";
    assert_eq!(parse_request_head(folded.as_bytes(), port), Err(400));

    // validate_host_header value matrix — accepted spellings.
    for ok in [
        "127.0.0.1:4321",
        "localhost:4321",
        "LOCALHOST:4321",
        "[::1]:4321",
        "127.0.0.1",       // portless loopback tolerated (foreign names are not)
        "localhost",       // portless, same reasoning
        "127.0.0.1:04321", // ports compare numerically after parsing
        "127.0.0.1:4321 ", // the validator trims the value's edges
    ] {
        assert!(validate_host_header(ok, port), "Host {ok:?} must pass");
    }
    // …and refused shapes.
    for no in [
        "evil.example:4321",   // foreign name (the rebinding case)
        "127.0.0.1:4322",      // wrong port
        "user@127.0.0.1:4321", // userinfo shape
        "127.0.0.1 :4321",     // whitespace INSIDE the value
        "127.0.0.1:4321:1",    // two colons outside brackets
        "127.0.0.2:4321",      // not an accepted loopback spelling
        "[::1",                // unterminated bracket
        "[::1]x:4321",         // junk after the bracket
        "[::2]:4321",          // a different IPv6 address entirely
        "",                    // empty
    ] {
        assert!(!validate_host_header(no, port), "Host {no:?} must fail");
    }
}

// ---------------------------------------------------------------------------
// HEAD body discipline (AUDIT V22-1/V22-2): a HEAD response must
// carry headers only, on EVERY path — v21 wrote the 416 body and
// the events-endpoint error bodies even for HEAD.
// ---------------------------------------------------------------------------

#[test]
fn head_range_unsatisfiable_has_no_body() {
    let fx = fixture("head416");
    let path = format!("/r/{}/media.bin", fx.root);
    let resp = request(
        &fx,
        &format!(
            "HEAD {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nRange: bytes=5000-6000\r\nConnection: close\r\n\r\n",
            fx.server.port()
        ),
    );
    assert_eq!(status_of(&resp), 416);
    assert_eq!(
        header_of(&resp, "Content-Range"),
        Some("bytes */1000"),
        "416 must still advertise the real length"
    );
    assert_eq!(body_of(&resp), "", "HEAD 416 must not carry a body");
    // …while the same GET keeps its explanatory body.
    let resp = request(
        &fx,
        &format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nRange: bytes=5000-6000\r\nConnection: close\r\n\r\n",
            fx.server.port()
        ),
    );
    assert_eq!(status_of(&resp), 416);
    assert!(!body_of(&resp).is_empty(), "GET 416 keeps its body");
}

#[test]
fn head_on_events_endpoint_is_headers_only_and_not_a_subscription() {
    let fx = fixture("headevents");
    let path = format!("/__pico/events/{}", fx.root);
    let resp = request(
        &fx,
        &format!(
            "HEAD {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            fx.server.port()
        ),
    );
    assert_eq!(status_of(&resp), 405, "the events endpoint is GET-only");
    assert_eq!(body_of(&resp), "", "HEAD error must not carry a body");
}

#[test]
fn head_responses_match_get_headers_without_bodies() {
    let fx = fixture("headfile");
    let path = format!("/r/{}/app.js", fx.root);
    let get_resp = get(&fx, &path);
    let head_resp = request(
        &fx,
        &format!(
            "HEAD {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            fx.server.port()
        ),
    );
    assert_eq!(status_of(&head_resp), 200);
    assert_eq!(
        header_of(&head_resp, "Content-Length"),
        header_of(&get_resp, "Content-Length"),
        "HEAD must advertise the GET length"
    );
    assert_eq!(body_of(&head_resp), "");
    assert_eq!(body_of(&get_resp), "console.log(1);");
}

// ---------------------------------------------------------------------------
// AUDIT 0.31 (A4-F1): is_served_url — Pico-served detection is
// a PARSE, not a string match against the canonical prefix.
// ---------------------------------------------------------------------------

#[test]
fn v31_is_served_url_alias_matrix() {
    let port = 49152u16;
    // Canonical form.
    assert!(is_served_url(
        &format!("http://127.0.0.1:{port}/r/123/page.html"),
        port
    ));
    // localhost spelling of the same server.
    assert!(is_served_url(
        &format!("http://localhost:{port}/r/123/page.html"),
        port
    ));
    // Userinfo form: the authority's real host is what counts.
    assert!(is_served_url(
        &format!("http://user:secret@127.0.0.1:{port}/r/123/page.html"),
        port
    ));
    // A single trailing dot on the host is the same host.
    assert!(is_served_url(
        &format!("http://127.0.0.1.:{port}/r/123/page.html"),
        port
    ));
    assert!(is_served_url(
        &format!("http://[::1]:{port}/r/123/page.html"),
        port
    ));
    // Wrong port.
    assert!(!is_served_url(
        &format!("http://127.0.0.1:{}/r/123/page.html", port + 1),
        port
    ));
    // https is not the static server's scheme.
    assert!(!is_served_url(
        &format!("https://127.0.0.1:{port}/r/123/page.html"),
        port
    ));
    // A path outside /r/ is not a served root URL (the events
    // endpoint included).
    assert!(!is_served_url(
        &format!("http://127.0.0.1:{port}/__pico/events/123"),
        port
    ));
    assert!(!is_served_url(
        &format!("http://127.0.0.1:{port}/x/r/123/page.html"),
        port
    ));
    // Lookalike host: 127.0.0.1 as a PREFIX of a foreign name.
    assert!(!is_served_url(
        &format!("http://127.0.0.1.evil.com:{port}/r/123/page.html"),
        port
    ));
    // Two trailing dots are not the single-dot alias.
    assert!(!is_served_url(
        &format!("http://127.0.0.1..:{port}/r/123/page.html"),
        port
    ));
    // No port at all means the http default (80), not ours.
    assert!(!is_served_url("http://127.0.0.1/r/123/page.html", port));
    // Against a live server: its own url_for output is served.
    let fx = fixture("servedurl");
    let url = fx.server.url_for(fx.root, "index.html");
    assert!(is_served_url(&url, fx.server.port()));
}

// ---------------------------------------------------------------------------
// AUDIT 0.31 (A5-L1): the hub's dropped-check + subscribe are
// atomic under one lock.
// ---------------------------------------------------------------------------

#[test]
fn v31_reload_hub_drop_is_atomic_against_subscribe() {
    let hub = pico_core::core::server::ReloadHub::new();
    // Subscribe first, then drop: the live subscription is
    // swept (its channel disconnects) and the count is zero.
    let sub = hub.try_subscribe(777);
    assert!(sub.is_some());
    assert_eq!(hub.subscriber_count(777), 1);
    hub.drop_root(777);
    assert_eq!(hub.subscriber_count(777), 0);
    drop(sub);
    // A root dropped BEFORE any subscribe gets no live
    // subscription — the tombstone and the map live under the
    // same lock, so no interleaving can resurrect it.
    hub.drop_root(888);
    assert!(hub.try_subscribe(888).is_none());
    assert_eq!(hub.subscriber_count(888), 0);
    // Broadcast to a dropped root is a harmless no-op.
    hub.broadcast(888);
    hub.broadcast(777);
}
