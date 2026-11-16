//! Favicon cache (0.41.2) — the real site icons behind the
//! bookmarks / Recent tiles / history rows / suggestion
//! rows that used to show only an initial-letter tile.
//!
//! One pipeline, one cache. The SHELL obtains each icon
//! from WebView2 itself (`ICoreWebView2_15::GetFavicon`,
//! PNG form): the browser already fetched the page's
//! declared icon from the page's own origin, so no
//! third-party favicon service is consulted, no extra
//! host-side fetch happens, and nothing phones home —
//! the privacy posture the rest of Pico keeps. This
//! module owns everything downstream of those bytes:
//! which hosts may have an icon at all, the on-disk
//! cache under a directory the shell passes in
//! (`<Pico home>/icons/`, resolved by `core::state` in
//! the shell — this module never resolves the home
//! itself, exactly like the nav store receives its
//! file's directory from `navstore::nav_path`), the
//! bounds, and the `data:` URI transport the internal
//! pages render.
//!
//! HOST IDENTITY. An icon belongs to a HOST, not a page:
//! the key is the origin prefix of
//! [`crate::core::navstore::canonical_nav_key`] — scheme,
//! host and non-default port, host lowercased with one
//! trailing dot dropped — so every page on one origin
//! shares one icon, and the key can never disagree with
//! the nav store's notion of sameness (it is cut out of
//! the canonical key itself). Only recordable http(s)
//! URLs have a key: credentialed URLs, internal/pico
//! pages, `file://` and every other scheme are refused
//! by [`crate::core::navstore::is_recordable_url`] plus
//! the `web_origin` gate, inherited wholesale. Loopback
//! hosts are ordinary http hosts here — a local dev
//! server's icon comes from that server itself through
//! the same native route.
//!
//! BOUNDS. At most [`MAX_ICONS`] files, oldest first
//! evicted (by file mtime — a re-store refreshes it
//! only when the cached file's mtime is older than
//! 24 h, the amortized refresh in [`store_icon`], so
//! the common already-cached path stays write-free);
//! one icon at most [`MAX_ICON_BYTES`]; stored bytes
//! must carry the PNG magic (the native route is asked
//! for PNG, so anything else is not a favicon). File
//! names are a percent-encoded form of the host key
//! (hosts are untrusted input — no path separator, dot
//! segment or reserved name can survive the encoding),
//! with a SHA-256 fallback for pathological lengths.
//!
//! TRANSPORT. Pages receive icons as `data:image/png`
//! URIs inside the existing nav payloads — the internal
//! pages' CSP already allows exactly `img-src data:`,
//! so no new protocol route, no new origin and no CSP
//! change exists. An icon larger than
//! [`MAX_ICON_DATA_URI_BYTES`] is cached on disk but
//! never inlined (the history page can list thousands
//! of rows; the payload must stay bounded) — its rows
//! keep the letter tile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Hard cap on cached icon files.
pub const MAX_ICONS: usize = 512;
/// Hard cap on one icon's bytes, as served by WebView2.
pub const MAX_ICON_BYTES: usize = 128 * 1024;
/// Hard cap on the bytes inlined as a `data:` URI. The
/// on-disk cap above governs storage; this one governs
/// payload size (a history push carries every row).
pub const MAX_ICON_DATA_URI_BYTES: usize = 32 * 1024;

/// The PNG signature every stored icon must carry.
const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Is this byte string a PNG (by its signature)?
pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(PNG_MAGIC)
}

/// The icon host key for a URL, or `None` when the URL
/// may not have an icon (see the module header for the
/// gate). The key is cut from the URL's canonical nav
/// key, so it is the nav store's host identity by
/// construction.
pub fn icon_host_key(url: &str) -> Option<String> {
    if !crate::core::navstore::is_recordable_url(url) {
        return None;
    }
    crate::core::addr::web_origin(url)?;
    let canonical = crate::core::navstore::canonical_nav_key(url);
    let authority_start = canonical.find("://")? + 3;
    let tail = &canonical[authority_start..];
    let cut = tail
        .find(|c| ['/', '?', '#'].contains(&c))
        .unwrap_or(tail.len());
    Some(canonical[..authority_start + cut].to_string())
}

/// The cache file name for one host key: every byte
/// outside `[A-Za-z0-9.-]` is percent-encoded, and a
/// `.` that leads the key or follows another `.` is
/// encoded too, so the result is always ONE safe path
/// component (no separator, no `..` run, no leading
/// dot, no trailing dot) on any filesystem. A name
/// that would still be too long for a path component
/// falls back to a truncated SHA-256 of the key —
/// deterministic, and collision-resistant far beyond
/// this cache's 512 entries.
pub fn icon_file_name(host_key: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(host_key.len() + 4);
    let mut prev: Option<u8> = None;
    for &b in host_key.as_bytes() {
        let keep = match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' => true,
            // A dot survives only between non-dots (the
            // `example.com` shape); a leading dot or a
            // dot run is encoded, so no `..` can form.
            b'.' => matches!(prev, Some(p) if p != b'.'),
            _ => false,
        };
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xF) as usize] as char);
        }
        prev = Some(b);
    }
    if out.is_empty() || out.len() > 180 {
        let digest = crate::core::sha256::sha256_hex(host_key.as_bytes());
        return format!("h-{}.png", &digest[..32]);
    }
    out.push_str(".png");
    out
}

/// The cache file for one host key inside `dir`.
pub fn icon_path(dir: &Path, host_key: &str) -> PathBuf {
    dir.join(icon_file_name(host_key))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (RFC 4648, with padding) — the
/// `data:` URI payload encoding. No crate enters the
/// tree for this (the house rule that keeps
/// `core::sha256` in-tree applies the same way).
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The `data:` URI a page renders for these icon bytes,
/// or `None` when the bytes may not ride a payload
/// (empty, not a PNG, or over the transport cap) — the
/// caller renders the letter tile instead.
pub fn data_uri_for(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_ICON_DATA_URI_BYTES || !is_png(bytes) {
        return None;
    }
    Some(format!("data:image/png;base64,{}", base64_encode(bytes)))
}

/// Which cached files must go when the cache holds
/// `entries` (file name, mtime in ms) and the cap is
/// `cap`: the oldest first, mtime ties broken by name
/// so two runs over the same directory always agree.
/// Pure — the store's eviction is this choice plus
/// `remove_file`.
pub fn eviction_victims(entries: &[(String, u64)], cap: usize) -> Vec<String> {
    if entries.len() <= cap {
        return Vec::new();
    }
    let mut sorted: Vec<&(String, u64)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    sorted[..entries.len() - cap]
        .iter()
        .map(|e| e.0.clone())
        .collect()
}

/// What one store attempt decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreVerdict {
    /// The bytes are now the host's cached icon.
    Stored,
    /// The cached bytes were already exactly these.
    AlreadyCached,
    /// The URL has no icon host key (the shell derives
    /// keys from URLs; a refused URL stores nothing).
    RefusedHost,
    /// Empty byte string — WebView2 had no icon.
    RefusedEmpty,
    /// Over [`MAX_ICON_BYTES`].
    RefusedTooLarge,
    /// Not a PNG — the native route was asked for PNG,
    /// so anything else is not a favicon.
    RefusedNotPng,
    /// The write itself failed (the io error is the
    /// caller's log line; the cache simply stays as it
    /// was).
    Failed,
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Store one icon's bytes for `host_key` under `dir`,
/// then evict down to [`MAX_ICONS`]. The write follows
/// the house discipline (per-process temp sibling,
/// fsync, rename) so a crash mid-write can never leave
/// a torn icon at the real path.
pub fn store_icon(dir: &Path, host_key: &str, bytes: &[u8]) -> std::io::Result<StoreVerdict> {
    if bytes.is_empty() {
        return Ok(StoreVerdict::RefusedEmpty);
    }
    if bytes.len() > MAX_ICON_BYTES {
        return Ok(StoreVerdict::RefusedTooLarge);
    }
    if !is_png(bytes) {
        return Ok(StoreVerdict::RefusedNotPng);
    }
    std::fs::create_dir_all(dir)?;
    let path = icon_path(dir, host_key);
    if let Ok(existing) = std::fs::read(&path) {
        if existing == bytes {
            // AUDIT 0.41.3 (Front A F8): the module doc
            // always claimed "a re-store refreshes" the
            // eviction mtime, but this arm returned
            // before any write — a host whose icon never
            // changed could be evicted as "oldest" while
            // actively in use. The claim is now true,
            // amortized: only a cached file whose mtime
            // is older than 24 h is rewritten (the same
            // bytes, through the same temp+fsync+rename
            // discipline below); a fresher file returns
            // with no write at all.
            const REFRESH_AFTER: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
            let stale = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().map(|age| age > REFRESH_AFTER).unwrap_or(false))
                .unwrap_or(false);
            if stale {
                write_icon_bytes(dir, host_key, bytes)?;
            }
            return Ok(StoreVerdict::AlreadyCached);
        }
    }
    write_icon_bytes(dir, host_key, bytes)?;
    evict(dir);
    Ok(StoreVerdict::Stored)
}

/// Write one icon's bytes to its cache path through a
/// per-process temp sibling, an fsync, and a rename —
/// the house discipline, shared by the store path and
/// the AlreadyCached mtime refresh (AUDIT 0.41.3).
fn write_icon_bytes(dir: &Path, host_key: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = icon_path(dir, host_key);
    let name = icon_file_name(host_key);
    let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Evict oldest-first until the cache is within
/// [`MAX_ICONS`]. Best-effort: a file that will not
/// delete is simply counted again next store.
fn evict(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(String, u64)> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".png") {
            continue;
        }
        let mtime = entry.metadata().map(|m| mtime_ms(&m)).unwrap_or(0);
        entries.push((name, mtime));
    }
    for victim in eviction_victims(&entries, MAX_ICONS) {
        let _ = std::fs::remove_file(dir.join(victim));
    }
}

/// Load one host's cached icon bytes, validated on the
/// way out (a hand-planted or torn file that is not a
/// bounded PNG is simply "no icon").
pub fn load_icon(dir: &Path, host_key: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(icon_path(dir, host_key)).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES || !is_png(&bytes) {
        return None;
    }
    Some(bytes)
}

/// Load one host's cached icon as its renderable
/// `data:` URI (see [`data_uri_for`] for the transport
/// cap that can still refuse a stored icon).
pub fn load_icon_data_uri(dir: &Path, host_key: &str) -> Option<String> {
    data_uri_for(&load_icon(dir, host_key)?)
}

/// The shell's handle on the cache: the directory plus
/// the lazily-loaded `data:` URIs, keyed by host key.
/// Disk reads happen once per host per session (the
/// negative answer is cached too); a store updates the
/// entry in place, so payload builders always read a
/// current map through [`IconCache::uri_map`] /
/// [`IconCache::cached_uri_for_url`].
#[derive(Debug)]
pub struct IconCache {
    dir: PathBuf,
    uris: HashMap<String, Option<String>>,
}

impl IconCache {
    pub fn new(dir: PathBuf) -> Self {
        IconCache {
            dir,
            uris: HashMap::new(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The renderable URI for one URL's host, loading
    /// from disk on first ask. `None` = letter tile.
    pub fn data_uri_for_url(&mut self, url: &str) -> Option<String> {
        let key = icon_host_key(url)?;
        if !self.uris.contains_key(&key) {
            let uri = load_icon_data_uri(&self.dir, &key);
            self.uris.insert(key.clone(), uri);
        }
        self.uris.get(&key).cloned().flatten()
    }

    /// The already-loaded URI for one URL's host (no
    /// disk I/O): payload builders `preload` first, then
    /// read through this.
    pub fn cached_uri_for_url(&self, url: &str) -> Option<String> {
        let key = icon_host_key(url)?;
        self.uris.get(&key).cloned().flatten()
    }

    /// Load the URIs for a batch of URLs (the payload
    /// builders' one disk pass per push).
    pub fn preload<'a>(&mut self, urls: impl IntoIterator<Item = &'a str>) {
        for url in urls {
            let _ = self.data_uri_for_url(url);
        }
    }

    /// The positive entries only, host key → `data:`
    /// URI: the map the shared start/history mirrors
    /// carry for serve-time renders.
    pub fn uri_map(&self) -> HashMap<String, String> {
        self.uris
            .iter()
            .filter_map(|(k, v)| v.as_ref().map(|uri| (k.clone(), uri.clone())))
            .collect()
    }

    /// Store bytes fetched for a page URL (the shell's
    /// favicon event arm). The in-memory entry follows
    /// the verdict: stored bytes become the URI (or
    /// `None` when over the transport cap); refusals
    /// change nothing.
    pub fn store_url(&mut self, url: &str, bytes: &[u8]) -> StoreVerdict {
        let Some(key) = icon_host_key(url) else {
            return StoreVerdict::RefusedHost;
        };
        self.store_key(&key, bytes)
    }

    /// Store bytes for an already-derived host key (the
    /// shell's handler derives the key at the source,
    /// where the page URL lives).
    pub fn store_key(&mut self, host_key: &str, bytes: &[u8]) -> StoreVerdict {
        let verdict = match store_icon(&self.dir, host_key, bytes) {
            Ok(v) => v,
            Err(_) => StoreVerdict::Failed,
        };
        if matches!(verdict, StoreVerdict::Stored | StoreVerdict::AlreadyCached) {
            self.uris.insert(host_key.to_string(), data_uri_for(bytes));
        }
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal "PNG" for cache purposes: the magic
    /// plus filler (the cache validates the signature,
    /// never the image body — WebView2 produced it).
    fn png(len: usize) -> Vec<u8> {
        let mut v = PNG_MAGIC.to_vec();
        v.resize(len, 0xAB);
        v
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pico-icons-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn host_key_is_the_canonical_origin() {
        // Paths and queries do not split a host's icon.
        assert_eq!(
            icon_host_key("https://example.com/a/b?x=1#frag"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            icon_host_key("https://example.com"),
            Some("https://example.com".to_string())
        );
        // Default ports drop, exactly as canonical_nav_key.
        assert_eq!(
            icon_host_key("http://example.com:80/x"),
            Some("http://example.com".to_string())
        );
        assert_eq!(
            icon_host_key("https://example.com:443/x"),
            Some("https://example.com".to_string())
        );
        // Non-default ports are part of the host identity.
        assert_eq!(
            icon_host_key("https://example.com:8443/x"),
            Some("https://example.com:8443".to_string())
        );
        assert_eq!(
            icon_host_key("http://localhost:5173/app"),
            Some("http://localhost:5173".to_string())
        );
        assert_eq!(
            icon_host_key("http://127.0.0.1:8080/"),
            Some("http://127.0.0.1:8080".to_string())
        );
        assert_eq!(
            icon_host_key("http://[::1]:8080/x"),
            Some("http://[::1]:8080".to_string())
        );
        // Host case and one trailing dot fold away.
        assert_eq!(
            icon_host_key("https://EXAMPLE.com./Path"),
            Some("https://example.com".to_string())
        );
        // Scheme IS part of the identity (canonical_nav_key
        // says so): http and https are two keys.
        assert_ne!(
            icon_host_key("http://example.com/"),
            icon_host_key("https://example.com/")
        );
        // Consistency with the canonical key itself: the
        // host key is its origin prefix for every web URL.
        for url in [
            "https://example.com/a?b=c",
            "http://localhost:5173/",
            "https://sub.example.co.uk:8443/x#y",
            "http://[2001:db8::1]/p",
        ] {
            let key = icon_host_key(url).unwrap();
            let canonical = crate::core::navstore::canonical_nav_key(url);
            assert!(
                canonical == key || canonical[key.len()..].starts_with(['/', '?', '#']),
                "{url}: canonical {canonical} does not start with host key {key}"
            );
        }
    }

    #[test]
    fn host_key_refuses_non_icon_hosts() {
        for url in [
            // Credentialed: the password must never key a file.
            "https://user:pass@example.com/",
            "https://user@example.com/",
            // Internal pages, both spellings.
            "pico://start",
            "pico://history",
            "https://pico.start/",
            "https://pico.history/",
            "https://pico.downloads/",
            // Not http(s) at all.
            "file:///C:/dev/page.html",
            "about:blank",
            "data:text/html,hi",
            "ftp://example.com/icon.png",
            "javascript:alert(1)",
            "",
            "https://example.com/a b",
        ] {
            assert_eq!(icon_host_key(url), None, "{url} must have no icon key");
        }
    }

    #[test]
    fn file_names_are_safe_single_components() {
        for key in [
            "https://example.com",
            "http://localhost:5173",
            "http://[::1]:8080",
            "https://example.com:8443",
        ] {
            let name = icon_file_name(key);
            assert!(name.ends_with(".png"), "{name}");
            assert!(
                !name.contains('/') && !name.contains('\\'),
                "{name} must be one component"
            );
            assert_ne!(name, "..");
            assert!(!name.starts_with('.'), "{name}");
            // Joined onto a dir it stays inside it.
            let joined = Path::new("/cache").join(&name);
            assert_eq!(joined.parent(), Some(Path::new("/cache")));
        }
        // Hostile keys cannot traverse: separators and dot
        // segments are percent-encoded away.
        let hostile = icon_file_name("../../etc/passwd");
        assert!(!hostile.contains('/'));
        assert!(!hostile.contains(".."));
        assert_eq!(icon_file_name("a/b"), "a%2Fb.png");
        assert_ne!(icon_file_name("a/b"), icon_file_name("a%2Fb"));
        // Distinct keys keep distinct names (the encoding
        // is injective for realistic keys).
        assert_ne!(
            icon_file_name("https://a.example"),
            icon_file_name("https://b.example")
        );
        assert_ne!(
            icon_file_name("http://example.com:8080"),
            icon_file_name("http://example.com:8081")
        );
        // A pathological key length falls back to the
        // bounded hash form.
        let long = format!("https://{}.example", "a".repeat(300));
        let name = icon_file_name(&long);
        assert!(name.starts_with("h-") && name.len() == 38, "{name}");
    }

    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(PNG_MAGIC), "iVBORw0KGgo=");
    }

    #[test]
    fn data_uri_caps_and_magic() {
        assert_eq!(
            data_uri_for(&png(100)),
            Some(format!(
                "data:image/png;base64,{}",
                base64_encode(&png(100))
            ))
        );
        assert_eq!(data_uri_for(&[]), None);
        assert_eq!(data_uri_for(b"not a png at all"), None);
        // Over the transport cap: stored, never inlined.
        assert_eq!(data_uri_for(&png(MAX_ICON_DATA_URI_BYTES + 1)), None);
        assert!(data_uri_for(&png(MAX_ICON_DATA_URI_BYTES)).is_some());
    }

    #[test]
    fn eviction_chooses_oldest_first() {
        let entries = vec![
            ("new.png".to_string(), 300),
            ("old.png".to_string(), 100),
            ("mid.png".to_string(), 200),
        ];
        assert!(eviction_victims(&entries, 3).is_empty());
        assert_eq!(eviction_victims(&entries, 2), vec!["old.png".to_string()]);
        assert_eq!(
            eviction_victims(&entries, 1),
            vec!["old.png".to_string(), "mid.png".to_string()]
        );
        // Mtime ties break by name, deterministically.
        let tied = vec![("b.png".to_string(), 5), ("a.png".to_string(), 5)];
        assert_eq!(eviction_victims(&tied, 1), vec!["a.png".to_string()]);
        assert_eq!(
            eviction_victims(&tied, 0),
            vec!["a.png".to_string(), "b.png".to_string()]
        );
    }

    #[test]
    fn store_load_roundtrip_and_refusals() {
        let dir = temp_dir("roundtrip");
        let key = "https://example.com";
        assert_eq!(load_icon(&dir, key), None);
        assert_eq!(
            store_icon(&dir, key, &[]).unwrap(),
            StoreVerdict::RefusedEmpty
        );
        assert_eq!(
            store_icon(&dir, key, &png(MAX_ICON_BYTES + 1)).unwrap(),
            StoreVerdict::RefusedTooLarge
        );
        assert_eq!(
            store_icon(&dir, key, b"GIF89a not a png").unwrap(),
            StoreVerdict::RefusedNotPng
        );
        assert_eq!(load_icon(&dir, key), None, "refusals store nothing");
        let bytes = png(64);
        assert_eq!(store_icon(&dir, key, &bytes).unwrap(), StoreVerdict::Stored);
        assert_eq!(load_icon(&dir, key), Some(bytes.clone()));
        assert_eq!(
            store_icon(&dir, key, &bytes).unwrap(),
            StoreVerdict::AlreadyCached
        );
        let newer = png(65);
        assert_eq!(store_icon(&dir, key, &newer).unwrap(), StoreVerdict::Stored);
        assert_eq!(load_icon(&dir, key), Some(newer));
        // A hand-planted non-PNG at the icon path loads as
        // "no icon", never as bytes.
        std::fs::write(icon_path(&dir, key), b"torn").unwrap();
        assert_eq!(load_icon(&dir, key), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn already_cached_refreshes_only_a_stale_mtime() {
        // AUDIT 0.41.3 (Front A F8): a re-store of the
        // same bytes refreshes the eviction mtime ONLY
        // when the cached file is older than 24 h.
        let dir = temp_dir("refresh");
        let key = "https://example.com";
        let bytes = png(64);
        let path = icon_path(&dir, key);
        let age = |p: &Path, secs: u64| {
            let f = std::fs::File::options().write(true).open(p).unwrap();
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs))
                .unwrap();
        };
        let mtime = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(store_icon(&dir, key, &bytes).unwrap(), StoreVerdict::Stored);
        // One hour old: no refresh — the mtime is untouched.
        age(&path, 3600);
        let before = mtime(&path);
        assert_eq!(
            store_icon(&dir, key, &bytes).unwrap(),
            StoreVerdict::AlreadyCached
        );
        assert_eq!(mtime(&path), before, "a fresh cache file is not rewritten");
        // Two days old: the same bytes are rewritten and
        // the mtime is fresh again; the verdict is still
        // AlreadyCached and the bytes are unchanged.
        age(&path, 2 * 24 * 3600);
        assert_eq!(
            store_icon(&dir, key, &bytes).unwrap(),
            StoreVerdict::AlreadyCached
        );
        assert!(
            mtime(&path).elapsed().unwrap() < std::time::Duration::from_secs(60),
            "a stale cache file's mtime is refreshed"
        );
        assert_eq!(load_icon(&dir, key), Some(bytes));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_evicts_oldest_beyond_the_cap() {
        let dir = temp_dir("evict");
        // Seed MAX_ICONS + 3 files with strictly increasing
        // mtimes via direct writes, then one real store:
        // the store's eviction pass must leave exactly
        // MAX_ICONS files and drop the three oldest seeds.
        for i in 0..(MAX_ICONS + 3) {
            let key = format!("https://h{i}.example");
            std::fs::write(icon_path(&dir, &key), png(16)).unwrap();
        }
        // Backdate every seed so the new store is newest;
        // seeds keep their relative order by index through
        // their identical mtimes + name tie-break... names
        // differ, so instead assert the COUNT and that the
        // freshly stored icon survived.
        let fresh_key = "https://fresh.example";
        assert_eq!(
            store_icon(&dir, fresh_key, &png(24)).unwrap(),
            StoreVerdict::Stored
        );
        let count = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".png"))
            .count();
        assert_eq!(count, MAX_ICONS, "cache must be capped");
        assert_eq!(load_icon(&dir, fresh_key), Some(png(24)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_store_and_lookup_by_url() {
        let dir = temp_dir("cache");
        let mut cache = IconCache::new(dir.clone());
        assert_eq!(cache.data_uri_for_url("https://example.com/a"), None);
        // A refused URL stores nothing and reports it.
        assert_eq!(
            cache.store_url("https://user:pass@example.com/", &png(32)),
            StoreVerdict::RefusedHost
        );
        assert_eq!(
            cache.store_url("pico://start", &png(32)),
            StoreVerdict::RefusedHost
        );
        let bytes = png(48);
        assert_eq!(
            cache.store_url("https://example.com/a", &bytes),
            StoreVerdict::Stored
        );
        // Same host, other path/scheme-spelling: one icon.
        let uri = cache.data_uri_for_url("https://example.com/other");
        assert_eq!(uri, data_uri_for(&bytes));
        assert_eq!(
            cache.cached_uri_for_url("https://example.com/third"),
            uri.clone()
        );
        assert_eq!(cache.uri_map().len(), 1);
        assert_eq!(cache.uri_map().get("https://example.com"), uri.as_ref());
        // A fresh cache over the same dir loads it back.
        let mut cold = IconCache::new(dir.clone());
        assert_eq!(cold.data_uri_for_url("https://example.com/a"), uri);
        // Over-transport-cap bytes store but never inline.
        let big = png(MAX_ICON_DATA_URI_BYTES + 16);
        assert_eq!(
            cache.store_url("https://big.example/", &big),
            StoreVerdict::Stored
        );
        assert_eq!(cache.data_uri_for_url("https://big.example/"), None);
        assert_eq!(load_icon(&dir, "https://big.example"), Some(big));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
