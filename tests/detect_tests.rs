//! Detector domain-logic tests: title extraction, target
//! normalization, TLS classification against stub listeners, and a
//! real `python3 -m http.server` fixture end to end.

use pico_core::core::detect::{
    guess_service, listener_probe_ips, normalize_targets, parse_title, scan_services, signature,
    Listener, ScanOutcome, Service, ServiceKind, TitleCache,
};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener};
use std::time::Duration;

fn listener(addr: &str, port: u16, pid: u32) -> Listener {
    Listener {
        addr: addr.to_string(),
        port,
        pid,
    }
}

// ---------------------------------------------------------------------------
// parse_title (AUDIT L9/L10 in v12 terms: entities + tag boundaries)
// ---------------------------------------------------------------------------

#[test]
fn titles_extract_and_normalize() {
    assert_eq!(
        parse_title("<title>Hello</title>").as_deref(),
        Some("Hello")
    );
    assert_eq!(
        parse_title("<TITLE>  Spaced\n\t Title  </TITLE>").as_deref(),
        Some("Spaced Title")
    );
    assert_eq!(
        parse_title("<html><head><title lang=\"en\">Attrs</title>").as_deref(),
        Some("Attrs")
    );
    assert_eq!(parse_title("<p>no title here</p>"), None);
    assert_eq!(parse_title("<title>   </title>"), None);
    assert_eq!(parse_title("<title>Unclosed"), None);
}

#[test]
fn titles_skip_comments_and_lookalike_tags() {
    assert_eq!(
        parse_title("<!-- <title>Fake</title> --><title>Real</title>").as_deref(),
        Some("Real")
    );
    assert_eq!(
        parse_title("<titlex>Not</titlex><title>Yes</title>").as_deref(),
        Some("Yes")
    );
    assert_eq!(
        parse_title("<titles>Not</titles><title>Y</title>").as_deref(),
        Some("Y")
    );
}

#[test]
fn titles_decode_named_and_numeric_entities() {
    assert_eq!(
        parse_title("<title>Fish &amp; Chips</title>").as_deref(),
        Some("Fish & Chips")
    );
    assert_eq!(
        parse_title("<title>&lt;b&gt; &quot;x&quot; &#39;y&#39;</title>").as_deref(),
        Some("<b> \"x\" 'y'")
    );
    assert_eq!(
        parse_title("<title>&#72;&#x65;llo &#x1F600;</title>").as_deref(),
        Some("Hello \u{1F600}")
    );
    // Invalid scalar values are dropped, never crash the parse.
    assert!(parse_title("<title>a&#xD800;b</title>").is_some());
    // A bare & that starts no entity survives untouched.
    assert_eq!(
        parse_title("<title>Tom & Jerry</title>").as_deref(),
        Some("Tom & Jerry")
    );
}

// ---------------------------------------------------------------------------
// normalize_targets / probe ips / labels / signature
// ---------------------------------------------------------------------------

#[test]
fn targets_skip_self_and_nonlocal_and_merge_families() {
    let listeners = vec![
        listener("127.0.0.1", 3000, 999),  // our own pid: skipped
        listener("192.168.1.5", 3000, 42), // LAN bind: not probed
        listener("0.0.0.0", 8080, 42),
        listener("::", 8080, 42), // same (port,pid): merged
        listener("::1", 5173, 43),
        listener("127.0.0.1", 0, 44), // port 0: skipped
    ];
    let targets = normalize_targets(&listeners, 999);
    assert_eq!(targets.len(), 2);
    let t8080 = targets
        .iter()
        .find(|t| t.port == 8080)
        .expect("8080 target");
    assert!(t8080.ips.contains(&IpAddr::from([127, 0, 0, 1])));
    assert!(t8080.ips.contains(&IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])));
    let t5173 = targets
        .iter()
        .find(|t| t.port == 5173)
        .expect("5173 target");
    assert_eq!(t5173.ips, vec![IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])]);
    // Sorted by port for deterministic display.
    assert_eq!(targets[0].port, 5173);
}

#[test]
fn probe_ip_rules() {
    assert_eq!(
        listener_probe_ips("127.0.0.5"),
        vec![IpAddr::from([127, 0, 0, 5])]
    );
    assert_eq!(
        listener_probe_ips("0.0.0.0"),
        vec![IpAddr::from([127, 0, 0, 1])]
    );
    assert!(listener_probe_ips("10.0.0.2").is_empty());
    assert!(listener_probe_ips("not-an-ip").is_empty());
    let v6any = listener_probe_ips("::");
    assert_eq!(v6any.len(), 2, "v6 wildcard probes both loopbacks");
}

#[test]
fn labels_and_signature() {
    assert_eq!(guess_service(5173, None), "Vite dev server");
    assert_eq!(guess_service(3000, None), "Web app / dev server");
    assert_eq!(guess_service(4444, Some("My App")), "My App");
    let a = Service {
        port: 3000,
        pid: Some(1),
        process: None,
        title: None,
        label: "x".into(),
        url: "http://127.0.0.1:3000".into(),
        kind: ServiceKind::Http,
    };
    let mut b = a.clone();
    b.url = "https://127.0.0.1:3000".into();
    b.kind = ServiceKind::Https;
    let out_a = ScanOutcome {
        cards: vec![a],
        other_listeners: 0,
    };
    let out_b = ScanOutcome {
        cards: vec![b],
        other_listeners: 0,
    };
    assert_ne!(
        signature(&out_a),
        signature(&out_b),
        "signature must include the URL (AUDIT I5)"
    );
    // The aggregate count is part of the signature too (v14): a
    // listener joining/leaving the "other" bucket must re-push so
    // the muted line on the start page stays true.
    let out_c = ScanOutcome {
        cards: vec![],
        other_listeners: 3,
    };
    let out_d = ScanOutcome {
        cards: vec![],
        other_listeners: 4,
    };
    assert_ne!(signature(&out_c), signature(&out_d));
}

// ---------------------------------------------------------------------------
// Probe classification against stub listeners (AUDIT M8)
// ---------------------------------------------------------------------------

/// A stub that answers every connection with the same first byte(s).
fn stub_responder(reply: &'static [u8]) -> u16 {
    let ln = TcpListener::bind(("127.0.0.1", 0)).expect("bind stub");
    let port = ln.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut s in ln.incoming().flatten() {
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(reply);
            let _ = s.flush();
            std::thread::sleep(Duration::from_millis(60));
        }
    });
    port
}

/// Scan a single stub port with an injected process name. The v14
/// qualification bar depends on the owning process, so tests name
/// it explicitly (via the same `name_of` injection the GUI uses)
/// instead of depending on the host's real process table.
fn scan_one(port: u16, process: Option<&str>) -> ScanOutcome {
    let listeners = vec![listener("127.0.0.1", port, 4242)];
    let mut cache = TitleCache::new();
    let name = process.map(str::to_string);
    scan_services(
        &listeners,
        999_999,
        Duration::from_millis(600),
        &mut cache,
        &move |_| name.clone(),
    )
}

/// Build a complete HTTP reply with a correct Content-Length.
fn http_reply(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// Answer every connection with a runtime-built reply.
fn stub_text(reply: String) -> u16 {
    let leaked: &'static [u8] = Box::leak(reply.into_bytes().into_boxed_slice());
    stub_responder(leaked)
}

#[test]
fn tls_handshake_byte_classifies_as_https() {
    // 0x16 = TLS handshake record. The stub answers the plaintext
    // probe with it too (not HTTP), then the TLS probe sees it.
    // v14: a TLS listener earns a card only via a dev process or a
    // dev port (qualification rule (c)/(d)) — the process name is
    // injected here exactly as the GUI injects real process names.
    let port = stub_responder(&[0x16, 0x03, 0x03, 0x00, 0x2e]);
    let services = scan_one(port, Some("node.exe")).cards;
    assert_eq!(services.len(), 1, "TLS stub must be detected: {services:?}");
    assert_eq!(services[0].kind, ServiceKind::Https);
    assert_eq!(services[0].url, format!("https://localhost:{port}"));
    assert_eq!(services[0].label, "HTTPS service");
}

#[test]
fn tls_alert_byte_also_classifies_as_https() {
    // 0x15 = TLS alert (a server refusing our minimal ClientHello is
    // still a TLS server).
    let port = stub_responder(&[0x15, 0x03, 0x03, 0x00, 0x02]);
    let services = scan_one(port, Some("node.exe")).cards;
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].kind, ServiceKind::Https);
}

#[test]
fn tls_with_unknown_process_is_aggregate_only() {
    // The v13 screenshot's mystery ":19206 HTTPS" card: a TLS
    // listener owned by an unknown process on a random port is NOT
    // a card in v14 — it joins the aggregate count.
    let port = stub_responder(&[0x16, 0x03, 0x03, 0x00, 0x2e]);
    let found = scan_one(port, Some("SomeApp.exe"));
    assert!(found.cards.is_empty(), "anonymous TLS must not card");
    assert_eq!(found.other_listeners, 1);
}

#[test]
fn plaintext_http_classifies_with_title() {
    let port = stub_responder(
        b"HTTP/1.0 200 OK\r\nContent-Type: text/html\r\n\r\n<html><head><title>Stub Title</title></head></html>",
    );
    let services = scan_one(port, None).cards;
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].kind, ServiceKind::Http);
    assert_eq!(services[0].title.as_deref(), Some("Stub Title"));
    assert_eq!(services[0].label, "Stub Title");
}

#[test]
fn fresh_cache_entries_are_not_reprobed() {
    // A counting stub: the first scan probes once (one connection);
    // a second scan inside the TTL must reuse the cached verdict and
    // open NO new connection (AUDIT M7's other half — v12 refetched
    // or never-refetched; v13 caches with a shelf life).
    use std::sync::atomic::{AtomicUsize, Ordering};
    static HITS: AtomicUsize = AtomicUsize::new(0);
    let ln = TcpListener::bind(("127.0.0.1", 0)).expect("bind counting stub");
    let port = ln.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut s in ln.incoming().flatten() {
            HITS.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(
                b"HTTP/1.0 200 OK\r\n\r\n<html><head><title>Counted</title></head></html>",
            );
            let _ = s.flush();
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let listeners = vec![listener("127.0.0.1", port, 4242)];
    let mut cache = TitleCache::new();
    let first = scan_services(
        &listeners,
        999_999,
        Duration::from_millis(600),
        &mut cache,
        &|_| None,
    );
    assert_eq!(first.cards.len(), 1);
    let hits_after_first = HITS.load(Ordering::SeqCst);
    assert!(hits_after_first >= 1);
    let second = scan_services(
        &listeners,
        999_999,
        Duration::from_millis(600),
        &mut cache,
        &|_| None,
    );
    assert_eq!(second.cards.len(), 1);
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        hits_after_first,
        "second scan inside the TTL must not re-probe"
    );
}

#[test]
fn silent_listener_is_not_a_service() {
    // Accepts and says nothing: neither HTTP nor TLS.
    let ln = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = ln.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for s in ln.incoming().flatten() {
            std::thread::sleep(Duration::from_millis(300));
            drop(s);
        }
    });
    let found = scan_one(port, None);
    assert!(found.cards.is_empty(), "silent listener: {found:?}");
    assert_eq!(found.other_listeners, 1, "…but it counts in the aggregate");
}

// ---------------------------------------------------------------------------
// Real fixture: python3 -m http.server
// ---------------------------------------------------------------------------

#[test]
fn python_http_server_fixture_is_detected() {
    let dir = std::env::temp_dir().join(format!("pico-v15-pyfx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    std::fs::write(
        dir.join("index.html"),
        "<!DOCTYPE html><title>PyFixture</title><p>hi</p>",
    )
    .expect("write index");

    // Pick a free port, then hand it to python.
    let port = {
        let ln = TcpListener::bind(("127.0.0.1", 0)).expect("bind probe");
        let p = ln.local_addr().expect("addr").port();
        drop(ln);
        p
    };
    // Prefer python3 (Linux/macOS/CI) but fall back to `python`
    // (Windows), so the fixture runs on either host.
    let python = if std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        "python3"
    } else {
        "python"
    };
    let mut child = std::process::Command::new(python)
        .args([
            "-m",
            "http.server",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
            "--directory",
            &dir.to_string_lossy(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("python must be available for the detector fixture");

    // Wait until the server accepts connections (bounded).
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut up = false;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(up, "python http.server did not come up on {port}");

    let listeners = vec![listener("127.0.0.1", port, 4242)];
    let mut cache = TitleCache::new();
    let services = scan_services(
        &listeners,
        999_999,
        Duration::from_millis(800),
        &mut cache,
        &|_| None,
    )
    .cards;
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(services.len(), 1, "fixture must be detected: {services:?}");
    let s = &services[0];
    assert_eq!(s.kind, ServiceKind::Http);
    assert_eq!(s.title.as_deref(), Some("PyFixture"));
    assert_eq!(s.url, format!("http://localhost:{port}"));
}

// ---------------------------------------------------------------------------
// Qualification bar (AUDIT V14-5): the v13 screenshot's junk cards as
// regression fixtures. Precision over recall — a listener earns a
// card only via a real <title>, a dev-server signature, a dev
// process, or a dev port serving HTML.
// ---------------------------------------------------------------------------

#[test]
fn fixture_plain_text_404_is_not_a_card() {
    // (1) The ":13508 Not Found" card from the screenshot: a plain
    // text 404 body on a random port, unknown process.
    let port = stub_text(http_reply("404 Not Found", "text/plain", "Not Found"));
    let found = scan_one(port, Some("someapp.exe"));
    assert!(found.cards.is_empty(), "plain 404 must not card: {found:?}");
    assert_eq!(found.other_listeners, 1);
}

#[test]
fn fixture_html_404_with_real_title_is_a_card() {
    // (2) Same 404 status, but the body is HTML with a real title.
    let port = stub_text(http_reply(
        "404 Not Found",
        "text/html",
        "<html><head><title>My App</title></head><body>missing</body></html>",
    ));
    let found = scan_one(port, Some("someapp.exe"));
    assert_eq!(found.cards.len(), 1);
    assert_eq!(found.cards[0].title.as_deref(), Some("My App"));
    assert_eq!(found.cards[0].label, "My App");
}

#[test]
fn fixture_generic_error_title_is_not_a_title() {
    // A 404 page whose <title> is the boilerplate "Not Found" (how
    // the screenshot's card got its name) counts as NO title — and
    // with no other qualification the listener stays aggregate-only.
    let port = stub_text(http_reply(
        "404 Not Found",
        "text/html",
        "<html><head><title>Not Found</title></head><body>x</body></html>",
    ));
    let found = scan_one(port, Some("someapp.exe"));
    assert!(found.cards.is_empty(), "boilerplate title must not card");
    assert_eq!(found.other_listeners, 1);
}

#[test]
fn fixture_json_api_with_dev_process_is_a_card() {
    // (4) A JSON API owned by node: card via rule (c), labeled from
    // the process — and crucially with NO invented title.
    let port = stub_text(http_reply("200 OK", "application/json", "{\"ok\":true}"));
    let found = scan_one(port, Some("node.exe"));
    assert_eq!(found.cards.len(), 1);
    assert_eq!(found.cards[0].title, None);
    assert_eq!(found.cards[0].label, "Node.js app");
}

#[test]
fn fixture_discord_rpc_json_is_aggregate_only() {
    // (5) The screenshot's Discord card: a JSON API on a random
    // port owned by a non-dev process (Discord's RPC server).
    let port = stub_text(http_reply(
        "200 OK",
        "application/json",
        "{\"cmd\":\"dispatch\"}",
    ));
    let found = scan_one(port, Some("Discord.exe"));
    assert!(
        found.cards.is_empty(),
        "Discord RPC must not card: {found:?}"
    );
    assert_eq!(found.other_listeners, 1);
}

#[test]
fn fixture_status_line_reason_is_never_a_title() {
    // (6) The reason phrase is not a title source, ever.
    let port = stub_text(http_reply("200 Weird Title", "text/plain", "hi"));
    let found = scan_one(port, Some("someapp.exe"));
    assert!(
        found.cards.is_empty(),
        "status-line text must not become a card: {found:?}"
    );
    assert_eq!(found.other_listeners, 1);
}

#[test]
fn fixture_dev_server_signature_qualifies() {
    // (b) A Werkzeug server header qualifies on its own; the label
    // comes from the signature, not from a guess.
    let body = "<html><body>hello</body></html>";
    let reply = format!(
        "HTTP/1.1 200 OK\r\nServer: Werkzeug/3.0.1 Python/3.12.4\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let port = stub_text(reply);
    let found = scan_one(port, Some("someapp.exe"));
    assert_eq!(found.cards.len(), 1);
    assert_eq!(found.cards[0].label, "Flask / Werkzeug");
    assert_eq!(found.cards[0].title, None);
}

#[test]
fn titled_cards_sort_before_untitled() {
    // Two qualifying listeners: the titled one has the HIGHER port
    // but must still sort first.
    let titled_port = stub_text(http_reply(
        "200 OK",
        "text/html",
        "<html><head><title>Zeta App</title></head></html>",
    ));
    let json_port = stub_text(http_reply("200 OK", "application/json", "{}"));
    let listeners = vec![
        listener("127.0.0.1", titled_port, 4242),
        listener("127.0.0.1", json_port, 4243),
    ];
    let mut cache = TitleCache::new();
    let found = scan_services(
        &listeners,
        999_999,
        Duration::from_millis(600),
        &mut cache,
        &|pid| {
            if pid == 4243 {
                Some("node.exe".to_string())
            } else {
                None
            }
        },
    );
    assert_eq!(found.cards.len(), 2, "{found:?}");
    assert_eq!(found.cards[0].port, titled_port, "titled card first");
    assert_eq!(found.cards[1].port, json_port);
}
