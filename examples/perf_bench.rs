//! Measured-perf harness for the 0.41.2 perf pass (subagent copy).
//! Run: cargo run --release --locked --no-default-features --example perf_bench
//! Every figure is a median of repeated batch timings on this host.

use pico_core::core::detect::{
    normalize_targets, signature, Listener, ScanOutcome, Service, ServiceKind,
};
use pico_core::core::dlcoord::{card_view, Coordinator, DlBackend, DlSettings, DlTaskView};
use pico_core::core::download::{CookiePair, SpeedMeter};
use pico_core::core::navstore::{
    canonical_nav_key, merge_nav_stores, Bookmark, HistoryEntry, NavStore,
};
use pico_core::core::shield::{ResourceType, ShieldEngine};
use pico_core::core::state::PersistState;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const NOW: u64 = 1_760_000_000_000;

fn median_us(mut samples: Vec<Duration>) -> f64 {
    samples.sort();
    samples[samples.len() / 2].as_secs_f64() * 1e6
}

/// Time `f` in batches; return median microseconds PER CALL.
fn bench<F: FnMut()>(name: &str, batches: usize, iters: usize, mut f: F) {
    // warmup
    f();
    let mut per_call = Vec::new();
    for _ in 0..batches {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        per_call.push(t.elapsed() / iters as u32);
    }
    println!(
        "{name}: {:.2} us/call (median of {batches} batches x {iters})",
        median_us(per_call)
    );
}

fn bench_total<F: FnMut()>(name: &str, batches: usize, mut f: F) {
    f();
    let mut totals = Vec::new();
    for _ in 0..batches {
        let t = Instant::now();
        f();
        totals.push(t.elapsed());
    }
    println!(
        "{name}: {:.2} us total (median of {batches})",
        median_us(totals)
    );
}

// ---------------------------------------------------------------- shield

fn bench_shield() {
    // Public edition: no embedded lists ship — the bench
    // compiles a synthetic list set instead (the dev
    // tree's bench uses the embedded lists; the decide
    // paths measured are the same engine code).
    let lists: Vec<String> = vec!["||ads.example.com^\n||tracker.example.net^\n/banner.js\n\
         @@||good.example^$domain=example.com\n\
         example.com##.ad-banner\n$genericblock\n$generichide"
        .to_string()];
    let refs: Vec<&str> = lists.iter().map(|s| s.as_str()).collect();
    let t = Instant::now();
    let engine = ShieldEngine::compile(&refs);
    println!(
        "shield compile: {:?}; report: network={} exceptions={} always={}",
        t.elapsed(),
        engine.report().network_rules,
        engine.report().network_exceptions,
        engine.report().generic_always_rules
    );

    let page = "https://www.example.com/news/article";
    bench("shield decide typical benign script", 7, 2000, || {
        std::hint::black_box(engine.decide(
            "https://cdn.example.org/assets/site.js",
            page,
            ResourceType::Script,
        ));
    });
    bench("shield decide tracker block", 7, 2000, || {
        std::hint::black_box(engine.decide(
            "https://www.google-analytics.com/analytics.js",
            page,
            ResourceType::Script,
        ));
    });
    bench("shield decide anchored block", 7, 2000, || {
        std::hint::black_box(engine.decide(
            "https://ads.example.net/banner.js",
            page,
            ResourceType::Script,
        ));
    });
    // Long URLs: benign host, path+query padded.
    for len in [1_000usize, 4_000, 16_000, 65_000, 100_000] {
        let pad = "a".repeat(len);
        let url = format!("https://cdn.example.org/assets/{pad}.js");
        bench(
            &format!("shield decide long benign url len={len}"),
            5,
            200,
            || {
                std::hint::black_box(engine.decide(&url, page, ResourceType::Script));
            },
        );
    }
    // Long URL whose generic needle sits early (candidate pattern match over full URL).
    {
        let pad = "a".repeat(65_000);
        let url = format!("https://tracker.example.org/ads.js?x={pad}");
        bench(
            "shield decide long url with early needle len~65k",
            5,
            200,
            || {
                std::hint::black_box(engine.decide(&url, page, ResourceType::Script));
            },
        );
    }
    // Many-token URL: 2000 short path segments / query params.
    {
        let mut url = String::from("https://cdn.example.org");
        for i in 0..500 {
            url.push_str(&format!("/seg{i}"));
        }
        for i in 0..500 {
            url.push_str(&format!("&p{i}=v{i}"));
        }
        println!("many-token url len = {}", url.len());
        bench("shield decide many-token url", 5, 1000, || {
            std::hint::black_box(engine.decide(&url, page, ResourceType::Script));
        });
    }
    // Deep-subdomain host (host suffix walk length).
    bench("shield decide deep host (12 labels)", 7, 2000, || {
        std::hint::black_box(engine.decide(
            "https://a.b.c.d.e.f.g.h.i.j.example.org/x.js",
            page,
            ResourceType::Script,
        ));
    });
    // Uppercase URL (forces the lowercase copy to matter).
    bench("shield decide uppercase url", 7, 2000, || {
        std::hint::black_box(engine.decide(
            "HTTPS://CDN.EXAMPLE.ORG/ASSETS/SITE.JS",
            page,
            ResourceType::Script,
        ));
    });
}

// ---------------------------------------------------------------- navstore

fn full_store() -> NavStore {
    let mut s = NavStore::default();
    for i in 0..2000u64 {
        s.history.push(HistoryEntry {
            url: format!(
                "https://site{:03}.example.com/articles/{}?ref=home",
                i % 250,
                i
            ),
            title: format!("Article number {i} about topic {}", i % 40),
            last_visited: NOW - i * 60_000,
            visits: (i % 17 + 1) as u32,
        });
    }
    for i in 0..200u64 {
        s.bookmarks.push(Bookmark {
            url: format!("https://mark{:03}.example.org/page/{}", i % 100, i),
            title: format!("Saved page {i}"),
            added_at: NOW - i * 120_000,
        });
    }
    s
}

fn bench_nav() {
    let store = full_store();
    println!(
        "nav store: history={} bookmarks={}",
        store.history.len(),
        store.bookmarks.len()
    );

    bench_total(
        "nav canonical_nav_key x2000 (one pass over history)",
        7,
        || {
            let mut n = 0usize;
            for e in &store.history {
                n += canonical_nav_key(&e.url).len();
            }
            std::hint::black_box(n);
        },
    );
    bench("nav suggestions empty query limit 8", 7, 20, || {
        std::hint::black_box(store.suggestions("", NOW, 8));
    });
    bench("nav suggestions 'article' limit 8", 7, 20, || {
        std::hint::black_box(store.suggestions("article", NOW, 8));
    });
    bench("nav suggestions 'site123' limit 8", 7, 20, || {
        std::hint::black_box(store.suggestions("site123", NOW, 8));
    });
    bench("nav suggestions no-match limit 8", 7, 20, || {
        std::hint::black_box(store.suggestions("zzz-no-such-thing", NOW, 8));
    });
    bench("nav search_history 'article'", 7, 20, || {
        std::hint::black_box(store.search_history("article"));
    });
    bench("nav search_history '' (all)", 7, 20, || {
        std::hint::black_box(store.search_history(""));
    });
    // record_visit: existing entry at the TAIL (worst-case linear find).
    {
        let mut s = store.clone();
        let tail_url = s.history.last().unwrap().url.clone();
        bench_total("nav record_visit tail entry x100", 5, || {
            for _ in 0..100 {
                std::hint::black_box(s.record_visit(&tail_url, "T", NOW));
            }
        });
        let mut s2 = store.clone();
        bench_total("nav note_title tail entry x100", 5, || {
            for i in 0..100 {
                std::hint::black_box(s2.note_title(&tail_url, &format!("Title {i}")));
            }
        });
    }
    bench_total("nav sanitized(full store clone)", 5, || {
        std::hint::black_box(store.clone().sanitized(NOW));
    });
    {
        // Diverged disk store: same base + 50 entries only on disk.
        let mut disk = store.clone();
        for i in 0..50u64 {
            disk.history.push(HistoryEntry {
                url: format!("https://other{i}.example.net/x"),
                title: format!("Other {i}"),
                last_visited: NOW - i * 1000,
                visits: 1,
            });
        }
        bench_total("nav merge_nav_stores(ours full, disk full+50)", 5, || {
            std::hint::black_box(merge_nav_stores(&store, &disk));
        });
    }
    // Serialization + save paths.
    bench_total("nav serde pretty serialize (full store)", 7, || {
        std::hint::black_box(serde_json::to_string_pretty(&store).unwrap().len());
    });
    bench_total("nav serde compact serialize (full store)", 7, || {
        std::hint::black_box(serde_json::to_string(&store).unwrap().len());
    });
    let pretty_len = serde_json::to_string_pretty(&store).unwrap().len();
    let compact_len = serde_json::to_string(&store).unwrap().len();
    println!("nav json bytes: pretty={pretty_len} compact={compact_len}");
    let dir = std::env::temp_dir().join(format!("pico-perf-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("nav.json");
    bench_total("nav save_to (serialize+write+fsync+rename)", 7, || {
        pico_core::core::navstore::save_to(&path, &store).unwrap();
    });
    bench_total("nav save_merged (lock+load+merge+sanitize+save)", 5, || {
        pico_core::core::navstore::save_merged(&path, &store, NOW).unwrap();
    });
    // Push-signature JSON builds (app.rs shapes, rebuilt from core data).
    bench_total(
        "nav push history_init_json build (2000 entries -> Value -> String)",
        7,
        || {
            let v = serde_json::json!({ "entries": history_entries_json(&store.history) });
            std::hint::black_box(v.to_string().len());
        },
    );
    bench_total(
        "nav push bookmarks+recent json build (200 marks + 8 recent)",
        7,
        || {
            let b = bookmarks_json(&store.bookmarks).to_string();
            let r = history_entries_json(store.recent(8)).to_string();
            std::hint::black_box(b.len() + r.len());
        },
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// Mirrors of the app.rs JSON shapers (same field names/order of work).
fn history_entry_json(e: &HistoryEntry) -> serde_json::Value {
    serde_json::json!({
        "url": e.url,
        "title": e.title,
        "lastVisited": e.last_visited,
        "visits": e.visits,
    })
}
fn history_entries_json(entries: &[HistoryEntry]) -> serde_json::Value {
    serde_json::Value::Array(entries.iter().map(history_entry_json).collect())
}
fn bookmarks_json(bookmarks: &[Bookmark]) -> serde_json::Value {
    serde_json::Value::Array(
        bookmarks
            .iter()
            .map(|b| serde_json::json!({ "url": b.url, "title": b.title, "addedAt": b.added_at }))
            .collect(),
    )
}

// ---------------------------------------------------------------- state

fn bench_state() {
    let mut st = PersistState::default();
    for i in 0..10 {
        st.recents.push(format!("C:/dev/project{i}/index.html"));
    }
    st.blocked_total = 123_456;
    bench_total("state serde pretty serialize", 9, || {
        std::hint::black_box(serde_json::to_string_pretty(&st).unwrap().len());
    });
    println!(
        "state json bytes: {}",
        serde_json::to_string_pretty(&st).unwrap().len()
    );
    let dir = std::env::temp_dir().join(format!("pico-perf-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.json");
    bench_total("state save_to (serialize+write+fsync+rename)", 9, || {
        pico_core::core::state::save_to(&path, &st).unwrap();
    });
    bench_total("state sanitized(clone)", 9, || {
        std::hint::black_box(st.clone().sanitized());
    });
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- downloads

struct FakeBackend {
    views: Vec<DlTaskView>,
}
impl DlBackend for FakeBackend {
    fn add(
        &self,
        _u: &str,
        _s: Option<&str>,
        _c: Vec<CookiePair>,
        _n: u32,
        _r: Option<String>,
    ) -> Option<u64> {
        None
    }
    fn add_failed(&self, _u: &str, _s: Option<&str>, _r: &str) -> Option<u64> {
        None
    }
    fn pause(&self, _id: u64) {}
    fn resume(&self, _id: u64) {}
    fn cancel(&self, _id: u64) {}
    fn restart(&self, _id: u64) {}
    fn refresh_link(&self, _id: u64, _u: &str) {}
    fn remove(&self, _id: u64) {}
    fn delete_file(&self, _id: u64) {}
    fn clear_completed(&self) {}
    fn clear_all(&self) {}
    fn set_connections(&self, _id: u64, _n: u32) {}
    fn set_default_connections(&self, _n: u32) {}
    fn set_max_concurrent(&self, _n: u32) {}
    fn set_speed_limit(&self, _k: Option<u64>) {}
    fn views(&self) -> Vec<DlTaskView> {
        self.views.clone()
    }
    fn version(&self) -> u64 {
        7
    }
    fn active_count(&self) -> usize {
        1
    }
    fn failed_count(&self) -> usize {
        0
    }
    fn name_of(&self, _id: u64) -> Option<String> {
        None
    }
    fn completed_path(&self, _id: u64) -> Option<PathBuf> {
        None
    }
    fn downloads_dir(&self) -> PathBuf {
        PathBuf::from("C:/Downloads")
    }
}

fn task_views(n: usize) -> Vec<DlTaskView> {
    use pico_core::core::download::TaskStatus;
    (0..n)
        .map(|i| DlTaskView {
            id: i as u64,
            name: format!("file-{i}.zip"),
            host: "example.com".to_string(),
            status: if i % 3 == 0 {
                TaskStatus::Downloading
            } else {
                TaskStatus::Paused
            },
            total: Some(10_000_000),
            downloaded: 5_000_000,
            percent: Some(50.0),
            size_text: "9.5 MB".to_string(),
            detail_text: "4.8 MB of 9.5 MB · 1.2 MB/s · 4 s left".to_string(),
            connections: 8,
            error: None,
            url: format!("https://example.com/file-{i}.zip"),
            added_ms: NOW - i as u64 * 1000,
            sha256: None,
            hint: None,
            retry_text: None,
            retry_pending: false,
            refreshing: false,
            speed_bps: Some(1_200_000),
            eta_secs: Some(4),
        })
        .collect()
}

fn bench_downloads() {
    // SpeedMeter steady state: one record + one rate per 250ms tick.
    {
        let mut m = SpeedMeter::new();
        let mut t = 0u64;
        let mut bytes = 0u64;
        for _ in 0..40 {
            m.record(t, bytes);
            t += 250;
            bytes += 300_000;
        }
        bench("dl SpeedMeter record+rate (steady window)", 9, 2000, || {
            t += 250;
            bytes += 300_000;
            m.record(t, bytes);
            std::hint::black_box(m.rate(t));
        });
        println!("speedmeter samples retained: window 5s / 250ms cadence = ~21");
    }
    for n in [10usize, 100, 1000] {
        let views = task_views(n);
        bench(&format!("dl card_view ({n} tasks)"), 7, 200, || {
            std::hint::black_box(card_view(&views, 0));
        });
        let coord = Coordinator::new(
            FakeBackend {
                views: views.clone(),
            },
            DlSettings {
                default_connections: 8,
                max_concurrent: 3,
                speed_limit_kbps: None,
            },
        );
        bench(
            &format!("dl coordinator snapshot() ({n} tasks)"),
            7,
            200,
            || {
                std::hint::black_box(coord.snapshot());
            },
        );
        bench(
            &format!("dl snapshot json serialize ({n} tasks)"),
            7,
            200,
            || {
                let snap = coord.snapshot();
                std::hint::black_box(serde_json::to_string(&snap).unwrap().len());
            },
        );
    }
}

// ---------------------------------------------------------------- detect

fn bench_detect() {
    for n in [50usize, 200, 1000] {
        let listeners: Vec<Listener> = (0..n)
            .map(|i| Listener {
                addr: if i % 3 == 0 {
                    "0.0.0.0".to_string()
                } else {
                    "127.0.0.1".to_string()
                },
                port: 3000 + (i % 200) as u16,
                pid: 1000 + (i % 50) as u32,
            })
            .collect();
        bench(
            &format!("detect normalize_targets ({n} rows)"),
            7,
            200,
            || {
                std::hint::black_box(normalize_targets(&listeners, 999_999));
            },
        );
    }
    let outcome = ScanOutcome {
        cards: (0..20)
            .map(|i| Service {
                port: 3000 + i,
                pid: Some(100 + i as u32),
                process: Some("node".to_string()),
                title: Some(format!("Dev app {i}")),
                label: format!("Dev app {i}"),
                url: format!("http://localhost:{}", 3000 + i),
                kind: ServiceKind::Http,
            })
            .collect(),
        other_listeners: 7,
    };
    bench("detect signature (20 cards)", 9, 2000, || {
        std::hint::black_box(signature(&outcome));
    });
    // Probe-response parse cost on a 64 KiB HTML body (the probe read cap).
    let body = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nServer: Werkzeug\r\n\r\n<html><head><title>My App</title></head><body>{}</body></html>",
        "x".repeat(60_000)
    );
    bench("detect parse_http_response (64KiB html)", 7, 1000, || {
        std::hint::black_box(pico_core::core::detect::parse_http_response(&body));
    });
}

fn main() {
    println!("== shield ==");
    bench_shield();
    println!("== navstore ==");
    bench_nav();
    println!("== state ==");
    bench_state();
    println!("== downloads ==");
    bench_downloads();
    println!("== detect ==");
    bench_detect();
    let _ = Duration::ZERO;
}
