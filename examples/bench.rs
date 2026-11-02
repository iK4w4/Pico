//! Repeatable benchmark harness for BUILD_NOTES (Linux host numbers;
//! the Windows binary shares this exact core code).
//!
//! Run: `cargo run --release --locked --no-default-features --example bench`
//!
//! Measures:
//! 1. static-server small-file requests — sequential on keep-alive
//!    connections (latency) and 8-way parallel (throughput);
//! 2. one 32 MiB streaming response (plus a 1 MiB Range slice);
//! 3. detector scan time against a real `python3 -m http.server`.

use pico_core::core::detect::{scan_services, Listener, TitleCache};
use pico_core::core::server::FileServer;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn get_once(port: u16, path: &str, range: Option<&str>) -> (u16, usize) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let range_line = range.map(|r| format!("Range: {r}\r\n")).unwrap_or_default();
    s.write_all(
        format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{range_line}Connection: close\r\n\r\n"
        )
        .as_bytes(),
    )
    .expect("write");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).expect("read");
    let status: u16 = String::from_utf8_lossy(&buf[..buf.len().min(64)])
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    (status, buf.len())
}

/// Drive `n` sequential requests over one keep-alive connection,
/// reading each response exactly (head, then Content-Length bytes).
fn bench_conn(s: &mut TcpStream, port: u16, path: &str, n: usize) {
    for _ in 0..n {
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
            .expect("write ka");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            s.read_exact(&mut byte).expect("read head");
            head.push(byte[0]);
        }
        let head_text = String::from_utf8_lossy(&head).into_owned();
        let len: usize = head_text
            .split("\r\n")
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse().unwrap_or(0))
            })
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).expect("read body");
    }
}

fn main() {
    let base = std::env::temp_dir().join(format!("pico-v17-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("mkdir");
    std::fs::write(base.join("app.js"), "console.log('pico bench');\n").expect("small file");
    let big: Vec<u8> = (0..32 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(base.join("big.bin"), &big).expect("big file");
    std::fs::write(base.join("index.html"), "<title>Bench</title>").expect("index");

    let server = FileServer::start().expect("server");
    let root = server.register_root(&base).expect("root");
    let port = server.port();
    let url_path = |u: String| u[u.find("/r/").expect("path")..].to_string();
    let small_path = url_path(server.url_for(root, "app.js"));
    let big_path = url_path(server.url_for(root, "big.bin"));

    // --- 1a. sequential keep-alive latency --------------------------
    // The server caps one connection at 100 requests (documented DoS
    // bound), so the harness reconnects per 100 — like a browser pool.
    let n = 2000usize;
    let started = Instant::now();
    for _ in 0..(n / 100) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect ka");
        s.set_read_timeout(Some(Duration::from_secs(30))).ok();
        bench_conn(&mut s, port, &small_path, 100);
    }
    let seq = started.elapsed();
    println!(
        "small-file keep-alive sequential: {n} requests in {seq:?} = {:.0} req/s, {:.3} ms/req",
        n as f64 / seq.as_secs_f64(),
        seq.as_secs_f64() * 1000.0 / n as f64
    );

    // --- 1b. parallel throughput ------------------------------------
    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let path = small_path.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..250 {
                let (status, _) = get_once(port, &path, None);
                assert_eq!(status, 200);
            }
        }));
    }
    for h in handles {
        h.join().expect("worker");
    }
    let par = started.elapsed();
    println!(
        "small-file parallel (8 conns): 2000 requests in {par:?} = {:.0} req/s",
        2000.0 / par.as_secs_f64()
    );

    // --- 2. large-file streaming -------------------------------------
    let started = Instant::now();
    let (status, bytes) = get_once(port, &big_path, None);
    let stream = started.elapsed();
    assert_eq!(status, 200);
    println!(
        "large-file stream: {bytes} bytes in {stream:?} = {:.1} MiB/s",
        (bytes as f64 / (1024.0 * 1024.0)) / stream.as_secs_f64()
    );
    let started = Instant::now();
    let (status, bytes) = get_once(port, &big_path, Some("bytes=1048576-2097151"));
    let slice = started.elapsed();
    assert_eq!(status, 206);
    println!("range slice (1 MiB of 32 MiB): response {bytes} bytes in {slice:?}");

    // --- 3. detector scan over a python fixture ----------------------
    let py_port = {
        let ln = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let p = ln.local_addr().expect("addr").port();
        drop(ln);
        p
    };
    let mut child = std::process::Command::new("python3")
        .args([
            "-m",
            "http.server",
            &py_port.to_string(),
            "--bind",
            "127.0.0.1",
            "--directory",
            &base.to_string_lossy(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("python3 fixture");
    let deadline = Instant::now() + Duration::from_secs(8);
    while TcpStream::connect(("127.0.0.1", py_port)).is_err() {
        assert!(Instant::now() < deadline, "python server never came up");
        std::thread::sleep(Duration::from_millis(50));
    }
    let listeners = vec![Listener {
        addr: "127.0.0.1".to_string(),
        port: py_port,
        pid: 4242,
    }];
    let mut cold = Vec::new();
    for _ in 0..10 {
        let mut cache = TitleCache::new();
        let t = Instant::now();
        let services = scan_services(
            &listeners,
            999_999,
            Duration::from_millis(800),
            &mut cache,
            &|_| None,
        );
        cold.push(t.elapsed());
        assert_eq!(services.cards.len(), 1);
    }
    let mut cache = TitleCache::new();
    let _ = scan_services(
        &listeners,
        999_999,
        Duration::from_millis(800),
        &mut cache,
        &|_| None,
    );
    let mut warm = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let services = scan_services(
            &listeners,
            999_999,
            Duration::from_millis(800),
            &mut cache,
            &|_| None,
        );
        warm.push(t.elapsed());
        assert_eq!(services.cards.len(), 1);
    }
    let _ = child.kill();
    let _ = child.wait();
    let avg = |v: &[Duration]| v.iter().sum::<Duration>() / (v.len() as u32);
    println!(
        "detector scan (1 python service): cold avg {:?}, warm (cached) avg {:?}",
        avg(&cold),
        avg(&warm)
    );

    let _ = std::fs::remove_dir_all(&base);

    bench_shield();
}

/// Shield decision latency (AUDIT V21): compile the synthetic
/// corpus (tests/fixtures — the public build carries no real
/// lists to compile) once, then time 10k mixed decisions (ad
/// URLs, benign URLs, first/third party, several resource
/// types) and report p50/p95. Target: p95 < 50 µs per decision.
/// The corpus is small, so absolute numbers flatter the
/// engine versus the real lists; the bench tracks regressions
/// of the matcher, not list scale.
fn bench_shield() {
    use pico_core::core::shield::{ResourceType, ShieldEngine};
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let lists: Vec<String> = [
        "easylist-synthetic.txt",
        "easyprivacy-synthetic.txt",
        "ublock-filters-synthetic.txt",
        "ublock-privacy-synthetic.txt",
    ]
    .iter()
    .map(|n| std::fs::read_to_string(base.join(n)).expect("synthetic fixture list"))
    .collect();
    let refs: Vec<&str> = lists.iter().map(|s| s.as_str()).collect();
    let t = Instant::now();
    let engine = ShieldEngine::compile(&refs);
    println!("shield compile: {:?}", t.elapsed());
    let cases: &[(&str, &str, ResourceType)] = &[
        (
            "https://ads.example-corp.invalid/banner.js",
            "https://news.example/",
            ResourceType::Script,
        ),
        (
            "https://analytics.example-corp.invalid/analytics.js",
            "https://www.example.com/",
            ResourceType::Script,
        ),
        (
            "https://www.example.com/assets/site.css",
            "https://www.example.com/",
            ResourceType::Stylesheet,
        ),
        (
            "https://cdn.example.org/img/hero.png",
            "https://www.example.com/",
            ResourceType::Image,
        ),
        (
            "https://ads.example.net/banner.js",
            "https://blog.example.com/",
            ResourceType::Script,
        ),
        (
            "https://www.youtube.com/embed/abc",
            "https://www.example.com/",
            ResourceType::Subdocument,
        ),
        (
            "https://fonts.googleapis.com/css2?family=X",
            "https://www.example.com/",
            ResourceType::Stylesheet,
        ),
        (
            "https://api.example.com/v1/data",
            "https://app.example.com/",
            ResourceType::Xhr,
        ),
    ];
    let mut samples: Vec<u128> = Vec::with_capacity(10_000);
    let mut blocked = 0usize;
    for i in 0..10_000 {
        let (u, p, rt) = cases[i % cases.len()];
        let t = Instant::now();
        if engine.decide(u, p, rt).is_block() {
            blocked += 1;
        }
        samples.push(t.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let pct = |q: f64| samples[((samples.len() as f64 * q) as usize).min(samples.len() - 1)];
    println!(
        "shield decide: 10k mixed, {blocked} blocked, p50 {} ns, p95 {} ns, max {} ns",
        pct(0.50),
        pct(0.95),
        samples[samples.len() - 1]
    );
}
