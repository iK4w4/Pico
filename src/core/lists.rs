//! Filter-list acquisition.
//!
//! The private 0.31 embedded the four Shield lists in the
//! executable (`include_bytes!` of gzip assets). The public
//! edition carries NO list content: the lists are downloaded
//! on first run from their canonical sources — the same way
//! uBlock Origin fetches them — and cached as plain text in
//! `<data dir>/lists/`. This module is the whole policy:
//! the closed source table, cache resolution, download
//! orchestration, and the validation every byte passes
//! before the Shield compiler may see it. The transport
//! itself is injected (the app passes a WinHTTP getter);
//! everything here is pure and host-testable.
//!
//! Resolution order (per launch, on the Shield's background
//! thread):
//! 1. A complete, valid cache in `lists/` wins — no network.
//! 2. Otherwise the four lists are downloaded in table
//!    order. All four must arrive and validate; the set is
//!    all-or-nothing, so a half-fetched Shield never runs.
//! 3. A downloaded set is compiled as a final sanity gate
//!    and only then written to the cache (atomically, the
//!    pstore temp+rename discipline), so a failed validation
//!    is never cached and a corrupt page can never become
//!    the standing lists.
//!
//! Failure is fail-open by the Shield's standing law: the
//! caller logs `shield: lists unavailable (…); Shield idle
//! until lists land` and Pico works normally with the
//! Shield's network/cosmetic/scriptlet layers idle.

use std::path::Path;

use super::shield::ShieldEngine;

/// One downloadable list: its cache file name, its canonical
/// URL (the only URLs the downloader will ever be pointed
/// at), and the hard size cap enforced while streaming and
/// re-checked here. Caps carry ~3–4× headroom over the
/// genuine sizes verified on 2026-11-02 (2,043,354 /
/// 1,444,742 / 471,693 / 184,591 raw bytes), so a list can
/// grow substantially before a cap needs a human decision.
pub struct ListSource {
    pub file_name: &'static str,
    pub url: &'static str,
    pub max_bytes: usize,
}

const MIB: usize = 1024 * 1024;

/// The four canonical list sources — EasyList, EasyPrivacy,
/// and uBlock Origin's filters + privacy lists, fetched from
/// the projects' own distribution points (see SOURCES.md).
pub const LIST_SOURCES: &[ListSource] = &[
    ListSource {
        file_name: "easylist.txt",
        url: "https://easylist.to/easylist/easylist.txt",
        max_bytes: 8 * MIB,
    },
    ListSource {
        file_name: "easyprivacy.txt",
        url: "https://easylist.to/easylist/easyprivacy.txt",
        max_bytes: 4 * MIB,
    },
    ListSource {
        file_name: "ublock-filters.txt",
        url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters.txt",
        max_bytes: 2 * MIB,
    },
    ListSource {
        file_name: "ublock-privacy.txt",
        url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/privacy.txt",
        max_bytes: MIB,
    },
];

/// The lists directory inside Pico's data dir.
pub fn lists_dir(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("lists")
}

/// Byte-level gate: size within the source's cap, strict
/// UTF-8 (a leading BOM is stripped — some mirrors add one),
/// then the text shape check. `None` = never compiled, never
/// cached.
pub fn validate_list_bytes(bytes: &[u8], max_bytes: usize) -> Option<String> {
    if bytes.is_empty() || bytes.len() > max_bytes {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if !validate_list_text(text) {
        return None;
    }
    Some(text.to_string())
}

/// The shape of a filter list, as opposed to an HTML error
/// page or a truncated transfer: at least 100 lines, a
/// header/comment line near the top (`[Adblock …]` or a
/// `!`/`#` comment), and — across the first 400 content
/// lines — at least half that look like rules (they carry
/// `##`, `||`, `@@`, a `$` option string, or open with the
/// `/regex/` form). Deliberately loose about everything
/// else: the compiler's own accounting is the strict judge.
pub fn validate_list_text(text: &str) -> bool {
    let mut lines = 0usize;
    let mut header_seen = false;
    let mut sampled = 0usize;
    let mut rule_like = 0usize;
    for (i, raw) in text.lines().enumerate() {
        lines += 1;
        let line = raw.trim();
        if i < 12
            && (line.starts_with("[Adblock") || line.starts_with('!') || line.starts_with('#'))
        {
            header_seen = true;
        }
        if line.is_empty() || line.starts_with('!') {
            continue;
        }
        if sampled < 400 {
            sampled += 1;
            if line.contains("##")
                || line.contains("||")
                || line.starts_with("@@")
                || line.contains('$')
                || line.starts_with('/')
            {
                rule_like += 1;
            }
        }
    }
    lines >= 100 && header_seen && sampled > 0 && rule_like * 2 >= sampled
}

/// The final sanity gate before a set of lists may drive
/// the Shield: the real compiler must account for a real
/// corpus. The floors sit ~20× below the lists' actual
/// size (2026-10: ~200k lines, ~41k network rules), so
/// only garbage — an HTML page that slipped the shape
/// check, a truncated set — can fail them.
fn compile_sane(texts: &[String]) -> Option<ShieldEngine> {
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let engine = ShieldEngine::compile(&refs);
    let r = engine.report();
    if r.total_lines >= 10_000
        && r.network_rules + r.network_exceptions >= 5_000
        && r.cosmetic_generic + r.cosmetic_specific_rules >= 100
    {
        Some(engine)
    } else {
        None
    }
}

/// Read and validate the cached set. All four files must
/// exist and pass; anything less is "no cache".
fn load_cache(dir: &Path) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(LIST_SOURCES.len());
    for src in LIST_SOURCES {
        let bytes = std::fs::read(dir.join(src.file_name)).ok()?;
        out.push(validate_list_bytes(&bytes, src.max_bytes)?);
    }
    Some(out)
}

/// Persist a validated set, one atomic temp+rename per
/// file (the pstore discipline: fsync before rename, a
/// pid-unique temp name). Best-effort: a failed write only
/// means the next launch downloads again.
fn store_cache(dir: &Path, texts: &[String]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    use std::io::Write;
    for (src, text) in LIST_SOURCES.iter().zip(texts.iter()) {
        let target = dir.join(src.file_name);
        let tmp = dir.join(format!(".{}.{}.tmp", src.file_name, std::process::id()));
        let written = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &target)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// The outcome of one acquisition attempt.
pub enum AcquireOutcome {
    /// A compiled engine, ready to install (boxed: the
    /// engine is large and the unavailable path is not).
    Ready(Box<ShieldEngine>),
    /// No usable lists this attempt; the string is a short,
    /// log-safe reason (file names, URLs' file parts and OS
    /// error text only — never response content).
    Unavailable(String),
}

/// One acquisition attempt: cache first, download second,
/// as documented at the head of this module. `fetch` is
/// the transport: given a source it returns the response
/// body or a short error string; it must itself enforce
/// `source.max_bytes` while streaming (this module
/// re-checks the length before validating).
pub fn acquire_lists(
    cache_dir: &Path,
    fetch: &dyn Fn(&ListSource) -> Result<Vec<u8>, String>,
) -> AcquireOutcome {
    if let Some(texts) = load_cache(cache_dir) {
        if let Some(engine) = compile_sane(&texts) {
            return AcquireOutcome::Ready(Box::new(engine));
        }
        // A cache that validates line-by-line but compiles
        // degenerate is treated as no cache: fall through
        // and try the network rather than run a husk.
    }
    let mut texts = Vec::with_capacity(LIST_SOURCES.len());
    for src in LIST_SOURCES {
        let bytes = match fetch(src) {
            Ok(b) => b,
            Err(e) => {
                return AcquireOutcome::Unavailable(format!(
                    "download of {} failed: {e}",
                    src.file_name
                ));
            }
        };
        match validate_list_bytes(&bytes, src.max_bytes) {
            Some(t) => texts.push(t),
            None => {
                return AcquireOutcome::Unavailable(format!(
                    "{} failed validation (size, encoding or list shape)",
                    src.file_name
                ));
            }
        }
    }
    match compile_sane(&texts) {
        Some(engine) => {
            store_cache(cache_dir, &texts);
            AcquireOutcome::Ready(Box::new(engine))
        }
        None => AcquireOutcome::Unavailable(
            "downloaded lists compiled degenerate (sanity floor not met)".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    /// A synthetic list text that passes the shape check:
    /// header lines plus `n` rule lines (all hand-written
    /// fixture content on .invalid hosts).
    fn synthetic_list(n: usize) -> String {
        let mut s = String::from("[Adblock Plus 2.0]\n! Title: Pico synthetic test list\n");
        for i in 0..n {
            s.push_str(&format!("||ads{i}.example-corp.invalid^\n"));
        }
        s
    }

    /// A full four-list set whose COMBINED compile clears
    /// the sanity floors: ~12k lines, ~12k network rules,
    /// and 150 generic cosmetics in the first list.
    fn sane_set() -> Vec<String> {
        let mut first = synthetic_list(6_000);
        for i in 0..150 {
            first.push_str(&format!("##.ad-slot-{i}\n"));
        }
        vec![
            first,
            synthetic_list(2_000),
            synthetic_list(2_000),
            synthetic_list(2_000),
        ]
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("pico-lists-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("create temp dir");
        d
    }

    #[test]
    fn validation_accepts_list_shapes_and_rejects_pages() {
        assert!(validate_list_text(&synthetic_list(150)));
        let html = "<!doctype html>\n<html>\n<head><title>404</title></head>\n<body>not found</body>\n</html>\n".repeat(30);
        assert!(!validate_list_text(&html), "an HTML page is not a list");
        assert!(!validate_list_text("||ads.example^\n"), "too few lines");
        assert!(!validate_list_text(&"x".repeat(500)), "no header, no rules");
    }

    #[test]
    fn validation_bytes_enforces_cap_utf8_and_bom() {
        let good = synthetic_list(150);
        assert!(validate_list_bytes(good.as_bytes(), MIB).is_some());
        assert!(
            validate_list_bytes(good.as_bytes(), 100).is_none(),
            "over the cap"
        );
        assert!(
            validate_list_bytes(&[0xff, 0xfe, 0x00], MIB).is_none(),
            "not UTF-8"
        );
        let bom = format!("\u{feff}{good}");
        assert!(
            validate_list_bytes(bom.as_bytes(), MIB).is_some(),
            "BOM tolerated"
        );
        assert!(validate_list_bytes(&[], MIB).is_none(), "empty");
    }

    #[test]
    fn cache_hit_never_touches_the_network() {
        let dir = temp_dir("hit");
        store_cache(&dir, &sane_set());
        let calls = RefCell::new(0usize);
        let fetch = |_: &ListSource| -> Result<Vec<u8>, String> {
            *calls.borrow_mut() += 1;
            Err("network must not be consulted".to_string())
        };
        match acquire_lists(&dir, &fetch) {
            AcquireOutcome::Ready(_) => {}
            AcquireOutcome::Unavailable(why) => panic!("cache should have served: {why}"),
        }
        assert_eq!(*calls.borrow(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_success_is_cached_for_next_time() {
        let dir = temp_dir("dl");
        let set = sane_set();
        let fetch = |src: &ListSource| -> Result<Vec<u8>, String> {
            let idx = LIST_SOURCES
                .iter()
                .position(|s| s.file_name == src.file_name)
                .expect("known source");
            Ok(set[idx].clone().into_bytes())
        };
        match acquire_lists(&dir, &fetch) {
            AcquireOutcome::Ready(_) => {}
            AcquireOutcome::Unavailable(why) => panic!("download should have served: {why}"),
        }
        for src in LIST_SOURCES {
            assert!(
                dir.join(src.file_name).exists(),
                "validated download must be cached: {}",
                src.file_name
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_bad_download_fails_the_set_and_caches_nothing() {
        let dir = temp_dir("bad");
        let set = sane_set();
        let fetch = |src: &ListSource| -> Result<Vec<u8>, String> {
            if src.file_name == "ublock-privacy.txt" {
                return Ok(b"<html>definitely not a list</html>".to_vec());
            }
            let idx = LIST_SOURCES
                .iter()
                .position(|s| s.file_name == src.file_name)
                .expect("known source");
            Ok(set[idx].clone().into_bytes())
        };
        match acquire_lists(&dir, &fetch) {
            AcquireOutcome::Ready(_) => panic!("a bad file must fail the whole set"),
            AcquireOutcome::Unavailable(why) => assert!(why.contains("ublock-privacy")),
        }
        for src in LIST_SOURCES {
            assert!(
                !dir.join(src.file_name).exists(),
                "a failed set is never cached ({})",
                src.file_name
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn degenerate_compile_is_unavailable_not_installed() {
        let dir = temp_dir("degen");
        // Shape-valid but far below the sanity floors.
        let small = synthetic_list(150);
        let fetch = |_: &ListSource| -> Result<Vec<u8>, String> { Ok(small.clone().into_bytes()) };
        match acquire_lists(&dir, &fetch) {
            AcquireOutcome::Ready(_) => panic!("degenerate set must not install"),
            AcquireOutcome::Unavailable(why) => assert!(why.contains("degenerate")),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_cache_falls_through_to_download() {
        let dir = temp_dir("corrupt");
        store_cache(&dir, &sane_set());
        // Corrupt one cached file after the fact.
        std::fs::write(dir.join("easylist.txt"), b"<html>nope</html>").expect("corrupt");
        let set = sane_set();
        let fetch = |src: &ListSource| -> Result<Vec<u8>, String> {
            let idx = LIST_SOURCES
                .iter()
                .position(|s| s.file_name == src.file_name)
                .expect("known source");
            Ok(set[idx].clone().into_bytes())
        };
        match acquire_lists(&dir, &fetch) {
            AcquireOutcome::Ready(_) => {}
            AcquireOutcome::Unavailable(why) => panic!("download should rescue: {why}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
