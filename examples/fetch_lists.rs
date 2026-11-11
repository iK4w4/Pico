//! Verification harness for the list download (not shipped logic):
//! drives the REAL `core::lists::acquire_lists` path — URL
//! set, per-file validation, engine compile, atomic cache
//! write, cache reuse — with a fetcher that reads local
//! copies from `PICO_FETCH_SEAM_DIR` (file names = the
//! canonical URLs' file names: easylist.txt, easyprivacy.txt,
//! filters.txt, privacy.txt). The WinHTTP transport that
//! replaces this seam on Windows is compile-verified for the
//! windows target; in this Linux environment the seam stands
//! in for the wire, and a second pass with a dead fetcher
//! proves the cache path compiles with no network at all.
//!
//! Usage:
//!   PICO_FETCH_SEAM_DIR=/path/to/copies \
//!   PICO_LISTS_CACHE=/path/to/cache \
//!     cargo run --example fetch_lists

use pico_core::core::lists::{acquire_lists, AcquireOutcome, ListSource, LIST_SOURCES};
use std::path::PathBuf;

fn main() {
    let seam =
        PathBuf::from(std::env::var("PICO_FETCH_SEAM_DIR").expect("PICO_FETCH_SEAM_DIR set"));
    let cache = PathBuf::from(std::env::var("PICO_LISTS_CACHE").expect("PICO_LISTS_CACHE set"));
    let seam_fetcher = |src: &ListSource| -> Result<Vec<u8>, String> {
        let name = src.url.rsplit('/').next().unwrap_or("list.txt");
        let path = seam.join(name);
        std::fs::read(&path).map_err(|e| format!("seam read {}: {e}", path.display()))
    };
    match acquire_lists(&cache, &seam_fetcher) {
        AcquireOutcome::Ready(engine) => {
            let r = engine.report();
            println!("PASS 1 (seam download): engine compiled");
            println!("  network_rules      {}", r.network_rules);
            println!("  exceptions         {}", r.network_exceptions);
            println!("  cosmetic_generic   {}", r.cosmetic_generic);
            println!("  cosmetic_specific  {}", r.cosmetic_specific_rules);
            println!(
                "  cosmetic total     {}",
                r.cosmetic_generic + r.cosmetic_specific_rules
            );
            println!("  scriptlet_entries  {}", r.scriptlet_entries);
        }
        AcquireOutcome::Unavailable(why) => {
            println!("PASS 1 FAILED: lists unavailable ({why})");
            std::process::exit(1);
        }
    }
    for src in LIST_SOURCES {
        let cached = cache.join(src.file_name);
        println!(
            "  cached {} ({} bytes)",
            src.file_name,
            std::fs::metadata(&cached).map(|m| m.len()).unwrap_or(0)
        );
    }
    // Pass 2: the fetcher is dead — the engine must come from
    // the cache alone.
    let dead = |_src: &ListSource| -> Result<Vec<u8>, String> { Err("network down".to_string()) };
    match acquire_lists(&cache, &dead) {
        AcquireOutcome::Ready(engine) => {
            let r = engine.report();
            println!(
                "PASS 2 (cache only): engine compiled, {} network rules",
                r.network_rules
            );
        }
        AcquireOutcome::Unavailable(why) => {
            println!("PASS 2 FAILED: lists unavailable ({why})");
            std::process::exit(1);
        }
    }
}
