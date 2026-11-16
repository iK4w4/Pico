//! 0.41.2 — the favicon pair test: the Rust shell, the
//! core cache and the three rendering surfaces must
//! agree on the icon contract, mechanically.
//!
//! The contract being paired:
//!
//! - the shell registers the native pipeline
//!   (`attach_favicon` → `FaviconChanged` → `GetFavicon`
//!   in PNG form) on every content view and stores
//!   through `core::icons` — never through a third-party
//!   favicon service or a host-side fetch (the privacy
//!   posture is part of the contract, so the service
//!   spellings are pinned ABSENT);
//! - every nav payload builder stamps an `icon` field
//!   (bookmarks, history entries, suggestion items);
//! - every surface renders that field inside a fixed
//!   slot with the letter/glyph fallback intact, and
//!   validates it as a PNG `data:` URI before use;
//! - the transport is the `data:` URI the pages' CSP
//!   already allows (`img-src data:`) — there is NO
//!   icon protocol route and no CSP widening anywhere.

use pico_core::core::assets::{assemble_chrome_html, assemble_history_html, assemble_start_html};
use pico_core::core::icons::{self, IconCache, StoreVerdict};

fn read_source(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// A minimal "PNG" for cache purposes (the cache
/// validates the signature, never the image body).
fn png(len: usize) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.resize(len, 0xAB);
    v
}

#[test]
fn shell_wiring_is_the_native_pipeline() {
    let app = read_source("src/app.rs");
    // The attach exists, is called for every content
    // view, and uses the platform-native route.
    assert!(app.contains("fn attach_favicon("), "attach_favicon missing");
    assert!(
        app.contains("attach_favicon(&view, id, self.proxy.clone());"),
        "build_content_view must attach the favicon pipeline per view"
    );
    assert!(
        app.contains("add_FaviconChanged"),
        "the FaviconChanged registration is the trigger"
    );
    assert!(
        app.contains("COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG"),
        "GetFavicon must ask for the PNG form"
    );
    assert!(
        app.contains("ICoreWebView2_15"),
        "the pipeline hangs off ICoreWebView2_15"
    );
    // The event arm stores through the core cache.
    assert!(
        app.contains("UserEvent::FaviconReceived"),
        "the favicon event must exist and be handled"
    );
    assert!(
        app.contains("app.icons.store_key(&host_key, &bytes)"),
        "the event arm must store through core::icons"
    );
    // The cache lives under the Pico home's icons dir.
    assert!(
        app.contains("pico_dir().join(\"icons\")"),
        "the shell must resolve <home>/icons and pass it to the cache"
    );
}

#[test]
fn no_third_party_favicon_service_anywhere() {
    // The browser itself fetches the icon from the
    // page's own origin; a favicon service would break
    // the privacy posture (every visited host phoned to
    // a third party). The spellings stay absent from
    // the shell, the core module and every page.
    for rel in [
        "src/app.rs",
        "src/core/icons.rs",
        "assets/chrome.html",
        "assets/start.html",
        "assets/history.html",
    ] {
        let text = read_source(rel);
        for service in [
            "s2/favicons",
            "favicons.googleapis",
            "icon.horse",
            "duckduckgo.com/ip3",
            "faviconkit",
            "besticon",
        ] {
            assert!(
                !text.contains(service),
                "{rel} must never name the favicon service {service}"
            );
        }
    }
    // No new protocol route serves icons: the folded
    // internal-host set gained no `icons` host and the
    // core URL folder does not know one either.
    let addr = read_source("src/core/addr.rs");
    assert!(
        !addr.contains("\"icons\""),
        "normalize_pico_url must not learn an icons host (no icon route)"
    );
}

#[test]
fn payload_builders_stamp_the_icon_field() {
    let app = read_source("src/app.rs");
    // Bookmarks + history entries share the one helper.
    assert_eq!(
        app.matches("\"icon\": icon_json(icons, &").count(),
        2,
        "bookmarks_json and history_entry_json must both stamp icon via icon_json"
    );
    // Suggestion items stamp it from the cache directly.
    assert!(
        app.contains("\"icon\": self.icons.cached_uri_for_url(&s.url)"),
        "suggestion items must carry the cached icon"
    );
    // The helper degrades to JSON null, never a missing
    // key the pages would have to guess at.
    assert!(
        app.contains("None => serde_json::Value::Null"),
        "icon_json must emit null when no icon is cached"
    );
}

#[test]
fn surfaces_render_icons_with_fallback_intact() {
    let start = assemble_start_html("{}");
    // Start: both renderers attach through the one
    // validator, the slot class exists, and the letter
    // derivation is still there underneath.
    assert!(start.contains("function attachIcon(slot, v)"));
    assert_eq!(
        start.matches("attachIcon(tile,").count(),
        2,
        "bookmark cards and recent tiles must both attach icons"
    );
    assert!(start.contains("fav-img"), "start icon slot CSS missing");
    assert!(
        start.contains("charAt(0)"),
        "the letter tile derivation must remain as the fallback"
    );
    let history = assemble_history_html("{}");
    assert!(
        history.contains("hist-fav"),
        "history rows need the fixed slot"
    );
    assert!(history.contains("attachIcon(fav, e.icon)"));
    assert!(
        history.contains("charAt(0)"),
        "history letter fallback missing"
    );
    let chrome = assemble_chrome_html();
    assert!(chrome.contains("sug-img"), "suggestion icon CSS missing");
    assert!(
        chrome.contains("sugIconUri(row.icon)"),
        "suggestion rows must validate the icon field"
    );
    // The kind glyphs remain: they are the suggestion
    // fallback when no icon is cached.
    assert!(
        chrome.contains("M12 7v5l3 2"),
        "the history clock glyph must remain as the suggestion fallback"
    );
}

#[test]
fn transport_is_the_csp_the_pages_already_have() {
    // Every internal page keeps `img-src data:` exactly:
    // the data-URI transport needs no widening, and any
    // edit that adds a scheme or host here is a contract
    // change this pin exists to catch.
    for rel in [
        "assets/chrome.html",
        "assets/start.html",
        "assets/history.html",
        "assets/downloads.html",
    ] {
        let text = read_source(rel);
        assert!(
            text.contains("img-src data:;"),
            "{rel} must keep img-src data: as its only image source"
        );
        assert!(
            !text.contains("img-src data: http"),
            "{rel} CSP must not widen img-src"
        );
    }
    // The pages validate the exact data-URI prefix the
    // core emits — the two sides spell it identically.
    for page in [
        assemble_start_html("{}"),
        assemble_history_html("{}"),
        assemble_chrome_html(),
    ] {
        assert!(
            page.contains("data:image/png;base64,"),
            "each surface must validate the core's data URI prefix"
        );
    }
}

#[test]
fn cache_contract_end_to_end() {
    // The integration-level contract over a real temp
    // dir: a stored icon is loadable by every URL on
    // its host, refused hosts store nothing, and the
    // URI map the shell mirrors holds exactly the
    // positive entries.
    let dir = std::env::temp_dir().join(format!("pico-icon-pair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cache = IconCache::new(dir.clone());
    assert_eq!(
        cache.store_url("https://example.com/watch?v=1", &png(40)),
        StoreVerdict::Stored
    );
    let uri = cache.data_uri_for_url("https://example.com/other");
    assert_eq!(uri, icons::data_uri_for(&png(40)));
    assert!(uri
        .as_ref()
        .is_some_and(|u| u.starts_with("data:image/png;base64,")));
    assert_eq!(cache.uri_map().len(), 1);
    assert_eq!(
        cache.store_url("https://user:pass@example.com/", &png(40)),
        StoreVerdict::RefusedHost
    );
    assert_eq!(cache.uri_map().len(), 1, "a refused store changes nothing");
    assert_eq!(icons::MAX_ICONS, 512);
    assert_eq!(icons::MAX_ICON_BYTES, 128 * 1024);
    let _ = std::fs::remove_dir_all(&dir);
}
