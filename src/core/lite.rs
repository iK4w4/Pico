//! Lite Mode (v25): block the heavy resource classes — images,
//! fonts, media — on public pages to save data on slow links.
//!
//! Design laws (AUDIT V25, test-pinned):
//! 1. Lite is a *class* filter, not a site filter: it applies the
//!    same rule to every public page and never special-cases a
//!    host. Predictability is the feature.
//!    AUDIT V29 (Lite semantics v2, user-overruled): the v28
//!    uniform "video never plays" law is REPLACED — the video
//!    the user chose to watch plays (see `filterpipe` and the
//!    play-gate runtime for the chosen-video model); hover
//!    previews and every other unsolicited playback stay dead,
//!    and thumbnail/poster images stay Lite-blocked.
//! 2. Local is sacred: a page that is loopback or Pico-internal
//!    renders completely untouched, and so does any request to a
//!    loopback/internal origin — the user's dev pages are never
//!    degraded by a data-saving mode.
//! 3. Lite runs strictly *after* the Shield in the request
//!    handler: a Shield block is final, is counted by the Shield,
//!    and is never re-counted by Lite.
//! 4. Classification is the Shield's (`resource_type_from_context`
//!    → [`ResourceType`]); there is no parallel mapping to drift.
//!
//! The decision function is pure; the tiny runtime (flag +
//! counter) mirrors the Shield's shape.

use crate::core::shield::ResourceType;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// The resource classes Lite blocks: the bytes-heavy, non-Text
/// payload of a page. Documents, scripts, stylesheets, XHR/fetch
/// and everything else pass — a Lite page still *works*, it is
/// just light.
pub fn is_lite_class(rtype: ResourceType) -> bool {
    matches!(
        rtype,
        ResourceType::Image | ResourceType::Font | ResourceType::Media
    )
}

/// Is this URL shaped like a media stream? (v28 stream
/// backstop.)
///
/// MSE players (YouTube first among them) fetch their segments
/// through fetch/XHR, so the request's resource class is `Xhr`,
/// never `Media` — Lite's class rule cannot see the heaviest
/// bytes on the modern web. This matcher is the backstop's
/// whole vocabulary, and it is deliberately narrow:
///
/// - the URL path (everything before `?`/`#`, lowercased,
///   percent-decoded per segment, with `;` terminating a
///   segment's name — AUDIT 0.31, A2-F12) contains
///   `/videoplayback` (the segmented-stream endpoint
///   shape), or
/// - the path ends in a streaming/media container extension:
///   `.mpd`, `.m3u8`, `.mp4`, `.webm`, `.m4a`, `.m4v`, `.ts`.
///
/// API JSON must never match: the innertube player/bootstrap
/// endpoints (`/youtubei/v1/player`, `/youtubei/v1/browse`, …)
/// end in plain path segments and carry no media extension —
/// the Shield's prune layer owns them, and blocking them
/// would break the pages Lite is trying to lighten. The
/// corpus test in this module pins both directions, including
/// near-misses (`/videoplayback-guide`, `file.mp4.json`,
/// `.ts` in a query string only).
/// Percent-decode one path segment for the stream matcher
/// (AUDIT 0.31, A2-F12): a `%XX` with two hex digits becomes
/// its byte when that byte is ASCII; anything else (a stray
/// `%`, non-hex digits, a non-ASCII byte) is left exactly as
/// written, so only spellings that decode to the matcher's
/// own ASCII vocabulary can change a verdict.
fn percent_decode_segment(seg: &str) -> String {
    let b = seg.as_bytes();
    let mut out = String::with_capacity(seg.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| -> Option<u8> {
                match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2])) {
                let byte = hi * 16 + lo;
                if byte.is_ascii() {
                    out.push(byte as char);
                    i += 3;
                    continue;
                }
            }
        }
        // Not a decodable escape: copy one CHAR (the segment
        // is valid UTF-8; slicing by a byte index that is not
        // a boundary would panic, and this matcher never may).
        let ch_len = seg[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&seg[i..i + ch_len]);
        i += ch_len;
    }
    out
}

pub fn is_media_stream_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let raw_path = lower.split(['?', '#']).next().unwrap_or(lower.as_str());
    // AUDIT 0.31 (A2-F12): the checks below run on the DECODED
    // path, segment by segment. `;` is a path-segment
    // terminator (matrix parameters: `/videoplayback;session=…`
    // is the videoplayback endpoint carrying a parameter), and
    // percent-escapes are decoded before the name/extension
    // checks (`/video%70layback`, `master.m3u8%3Ftok` — the
    // decoded `?` starts the query, ending the path there).
    // Script/WebSocket carriers are deliberately NOT added:
    // the matcher stays a Media/Xhr-shape backstop.
    let mut decoded = String::with_capacity(raw_path.len());
    for (idx, seg) in raw_path.split('/').enumerate() {
        if idx > 0 {
            decoded.push('/');
        }
        let dec = percent_decode_segment(seg);
        if let Some(cut) = dec.find(['?', '#']) {
            decoded.push_str(&dec[..cut]);
            break;
        }
        let seg_part = dec.split(';').next().unwrap_or(dec.as_str());
        decoded.push_str(seg_part);
    }
    let path = decoded.as_str();
    // The endpoint segment, whole: `/videoplayback` alone, or
    // followed by more path — not a longer word that merely
    // begins with it (`/videoplayback-guide` is a page, not a
    // stream). Every occurrence is tried.
    let mut from = 0;
    while let Some(rel) = path[from..].find("/videoplayback") {
        let after = &path[from + rel + "/videoplayback".len()..];
        if after.is_empty() || after.starts_with('/') {
            return true;
        }
        from += rel + 1;
    }
    [".mpd", ".m3u8", ".mp4", ".webm", ".m4a", ".m4v", ".ts"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// Path segments that make a page "watch-shaped" (AUDIT V29):
/// a page whose job is to present ONE main video. The corpus
/// is deliberately small and exact-segment (never substring):
/// `watch`, `video`, `view`, `play`, `episode`, `clip`. The
/// Lite document gate spells the same corpus in JS; both sides
/// are pinned by tests. Watch-shaped matters twice: a fresh
/// load of a watch-shaped page may designate its first large
/// video as chosen without a gesture (the paste-a-watch-link
/// case), and the network backstop never blocks stream shapes
/// on a watch-shaped page (the main video streams through
/// exactly those shapes — blocking them was the v28
/// regression).
pub const WATCH_PATH_SEGMENTS: &[&str] = &["watch", "video", "view", "play", "episode", "clip"];

/// Is this URL's PATH watch-shaped? Only the path is read —
/// never the query, the fragment, or the host — and only whole
/// segments match (`/watchtower` and `/?next=/watch` are not
/// watch pages; `/WATCH` and `/shows/episode/3` are).
pub fn is_watch_shaped_url(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    // AUDIT 0.30.1 (A2-C3): the query/fragment are stripped
    // BEFORE the path is located. Through 0.30 the first `/`
    // was found in the unstripped remainder, so a host-only
    // URL whose QUERY held the first slash
    // (`https://site.example?next=/watch`) read that slash as
    // the path start and called the query watch-shaped —
    // diverging from the document gate, which reads
    // `location.pathname` and structurally cannot see a query
    // at all. The network side then exempted a page the gate
    // had never designated. Neither `?` nor `#` can occur
    // inside an authority, so stripping here cannot eat path.
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let Some(slash) = rest.find('/') else {
        return false;
    };
    rest[slash..]
        .split('/')
        .any(|seg| WATCH_PATH_SEGMENTS.contains(&seg.to_ascii_lowercase().as_str()))
}

/// The identity of a watch-shaped URL, for chosen-flag
/// survival across same-document URL changes (AUDIT 0.30.1,
/// A2-C4): the path, plus the video id — the `v` query
/// parameter when one is present (YouTube's shape; its
/// volatile companions — `t`, `list`, `feature` — change the
/// presentation, not which video the page is), else the
/// whole query string (path-id sites carry the id in the
/// path already, and an unrecognized query shape is treated
/// as part of the identity: churn then clears, which is the
/// safe direction — a watch-shaped page is exempt by shape
/// whether or not the flag stands). The fragment is never
/// part of the identity. `None` for URLs that are not
/// watch-shaped.
pub fn watch_identity(url: &str) -> Option<String> {
    if !is_watch_shaped_url(url) {
        return None;
    }
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let nofrag = rest.split('#').next().unwrap_or(rest);
    let (host_path, query) = match nofrag.split_once('?') {
        Some((h, q)) => (h, q),
        None => (nofrag, ""),
    };
    let path = match host_path.find('/') {
        Some(i) => &host_path[i..],
        None => "",
    };
    let id = query
        .split('&')
        .find_map(|pair| {
            pair.split_once('=')
                .and_then(|(k, v)| (k == "v").then_some(v))
        })
        .unwrap_or(query);
    Some(format!("{path}|{id}"))
}

/// May a tab's chosen-video designation survive a
/// same-document URL change from `old` to `new` (AUDIT
/// 0.30.1, A2-C4)?
///
/// The host flag mirrors the document gate's designation,
/// and the gate resets that designation on EVERY route
/// change — through 0.30 the host cleared its flag only on
/// full navigation / toggle / tab close, so after an SPA
/// route change away from a watch page, preview media and
/// streams stayed network-allowed for the tab indefinitely
/// (playback still gate-refused; the data-saving silently
/// off). The survival matrix, pinned by tests:
///
/// - same URL → survives (not a change at all);
/// - watch → watch, same video identity → survives: this
///   is the designation bridge — the SourceChanged churn
///   around one watch context (host-form flips, `t`/`list`
///   parameter updates) must not starve the chosen video
///   between its designation and its segment fetches;
/// - watch → watch, different video → clears: the
///   designation belonged to the previous video; the new
///   one earns its own through the gate (and its segments
///   are shape-exempt meanwhile, so clearing cannot starve
///   it either);
/// - watch → non-watch → clears (the C4 leak itself);
/// - non-watch → watch → survives ("unaffected": nothing
///   about entering watch territory can strand a flag —
///   shape exempts the page's media regardless, and the
///   flag is re-judged at the next change);
/// - non-watch → different non-watch → clears: the
///   designation belonged to the old page context, exactly
///   as the gate judges it.
///
/// The app consults this only for same-document changes;
/// a full navigation retires the flag unconditionally (a
/// new document, a new designation to earn).
pub fn chosen_survives_url_change(old: &str, new: &str) -> bool {
    if old == new {
        return true;
    }
    match (watch_identity(old), watch_identity(new)) {
        (Some(a), Some(b)) => a == b,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => false,
    }
}

/// AUDIT V28 (A2, dead-code sweep): this module used to carry
/// its own boolean `decide` and `is_local_url` as a "spec
/// seam" beside the production pipeline. Neither had a caller
/// — production decides in `filterpipe::decide` — and the
/// module doc claimed a test pinned the two to the same
/// outcomes; no such test existed, and the pair had already
/// drifted on method handling. Both are deleted: the pipeline
/// IS the spec, and its matrix tests pin Lite's behavior
/// (class rule, stream backstop, local-sacred, toggle states)
/// where it actually runs. What remains here is the layer's
/// vocabulary (`is_lite_class`, `is_media_stream_url`) and its
/// runtime counters.
/// Cap on individually-logged Lite block lines per session
/// (mirrors the Shield's sampling discipline: classify, don't
/// mirror).
pub const LITE_LOG_SAMPLE: usize = 20;

/// Lite runtime shared between the app and every content
/// WebView's request handler.
pub struct LiteRuntime {
    pub on: AtomicBool,
    pub session_blocked: AtomicU64,
    log_sample_left: Mutex<usize>,
    /// Tabs whose page has designated its chosen video
    /// (AUDIT V29): set only by the validated `lite_chosen`
    /// content report, read by the request handler as a
    /// `filterpipe` fact, and cleared on full navigation,
    /// Lite toggle and tab close — and, since AUDIT 0.30.1
    /// (A2-C4), on same-document URL changes the designation
    /// does not survive (see
    /// [`chosen_survives_url_change`]). Runtime state only —
    /// never persisted: a designation belongs to one document
    /// in one session, not to the profile.
    chosen_tabs: Mutex<HashSet<u64>>,
    chosen_log_left: Mutex<usize>,
}

impl LiteRuntime {
    pub fn new(on: bool) -> Self {
        LiteRuntime {
            on: AtomicBool::new(on),
            session_blocked: AtomicU64::new(0),
            log_sample_left: Mutex::new(LITE_LOG_SAMPLE),
            chosen_tabs: Mutex::new(HashSet::new()),
            chosen_log_left: Mutex::new(LITE_LOG_SAMPLE),
        }
    }

    /// Record a tab's chosen-video designation. Returns true
    /// only for the FIRST designation of this tab's current
    /// document (a re-report for the same document is a no-op),
    /// which is what makes the app's one-log-line-per-chosen
    /// discipline and the report rate cap structural rather
    /// than remembered.
    pub fn mark_chosen(&self, tab: u64) -> bool {
        self.chosen_tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(tab)
    }

    /// Has this tab designated a chosen video?
    pub fn is_chosen(&self, tab: u64) -> bool {
        self.chosen_tabs
            .lock()
            .map(|set| set.contains(&tab))
            .unwrap_or(false)
    }

    /// Forget one tab's designation (route/document change,
    /// tab close).
    pub fn clear_chosen(&self, tab: u64) {
        if let Ok(mut set) = self.chosen_tabs.lock() {
            set.remove(&tab);
        }
    }

    /// Forget every designation (Lite toggled either way: the
    /// document gate resets its own state in step, and no
    /// designation may outlive the mode it was made under).
    pub fn clear_all_chosen(&self) {
        if let Ok(mut set) = self.chosen_tabs.lock() {
            set.clear();
        }
    }

    /// Consume one chosen-log sample slot (session-capped like
    /// every other per-event log in the filter layers).
    pub fn claim_chosen_log(&self) -> bool {
        let mut left = self
            .chosen_log_left
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if *left > 0 {
            *left -= 1;
            true
        } else {
            false
        }
    }

    /// Record one Lite block: returns (session total, may-log).
    pub fn record_block(&self) -> (u64, bool) {
        let total = self.session_blocked.fetch_add(1, Ordering::Relaxed) + 1;
        let log_it = {
            let mut left = self
                .log_sample_left
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if *left > 0 {
                *left -= 1;
                true
            } else {
                false
            }
        };
        (total, log_it)
    }
}

impl Default for LiteRuntime {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_url_corpus_true_positives() {
        for url in [
            "https://rr1---sn.example.googlevideo.com/videoplayback?id=x&itag=137",
            "https://rr1.googlevideo.com/videoplayback",
            "https://cdn.example/hls/master.m3u8",
            "https://cdn.example/hls/seg-9.ts?token=1",
            "https://cdn.example/dash/manifest.mpd",
            "https://cdn.example/v/movie.mp4",
            "https://cdn.example/v/movie.webm",
            "https://cdn.example/a/track.m4a",
            "https://cdn.example/a/track.m4v",
            "https://cdn.example/V/MOVIE.MP4",
            "https://cdn.example/x.mp4#t=10",
        ] {
            assert!(is_media_stream_url(url), "{url} must be a stream shape");
        }
    }

    #[test]
    fn stream_url_corpus_semicolon_and_percent_forms() {
        // AUDIT 0.31 (A2-F12): matrix parameters and
        // percent-escapes must not hide a stream shape from
        // the matcher — CDNs spell real segment URLs all
        // three ways.
        for url in [
            // `;param` form: `;` terminates the segment name.
            "https://cdn.example/videoplayback;session=abc123",
            "https://cdn.example/hls/master.m3u8;token=xyz",
            "https://cdn.example/v/movie.mp4;session=abc",
            // %70 = 'p': the endpoint name, escaped.
            "https://rr1.googlevideo.com/video%70layback?id=x&itag=137",
            // %3F = '?': decodes to the query start, so the
            // filename the matcher judges is master.m3u8.
            "https://cdn.example/hls/master.m3u8%3Ftok=abc",
            "https://cdn.example/v/movie.m%70%34",
        ] {
            assert!(is_media_stream_url(url), "{url} must be a stream shape");
        }
        // The near-miss pins survive decoding: an escaped or
        // parameter-carrying NON-stream must stay unmatched.
        for url in [
            "https://cdn.example/file.mp4.json",
            "https://cdn.example/file.mp4%2Ejson",
            "https://cdn.example/app.js?file=movie.mp4",
            "https://api.example/v1/videoplayback-guide",
            "https://api.example/v1/videoplayback%2Dguide",
            "https://cdn.example/notes.ts.txt",
        ] {
            assert!(!is_media_stream_url(url), "{url} must NOT match");
        }
    }

    #[test]
    fn stream_url_corpus_never_api_or_page() {
        // The false-positive side is the load-bearing half:
        // innertube and friends are how the pages Lite lightens
        // still WORK (and how the Shield prunes); a matcher
        // that eats API JSON breaks both.
        for url in [
            "https://www.youtube.com/youtubei/v1/player?key=abc&prettyPrint=false",
            "https://www.youtube.com/youtubei/v1/browse",
            "https://www.youtube.com/youtubei/v1/next",
            "https://api.example/v1/videoplayback-guide",
            "https://cdn.example/watch/videoplayback-guide",
            "https://cdn.example/file.mp4.json",
            "https://cdn.example/app.js?file=movie.mp4",
            "https://cdn.example/style.css",
            "https://cdn.example/image.png",
            "https://cdn.example/notes.ts.txt",
            "https://cdn.example/typescript/guide",
        ] {
            assert!(!is_media_stream_url(url), "{url} must NOT match");
        }
    }

    #[test]
    fn watch_shape_reads_exact_path_segments_only() {
        for url in [
            "https://www.youtube.com/watch?v=abc",
            "https://vimeo.com/video/12345",
            "https://site.example/shows/episode/3",
            "https://site.example/clip/xyz",
            "https://site.example/play",
            "https://site.example/view/item",
            "https://site.example/WATCH",
            "https://site.example/a/Clip/",
        ] {
            assert!(is_watch_shaped_url(url), "{url} must be watch-shaped");
        }
        for url in [
            "https://www.youtube.com/",
            "https://www.youtube.com/results?search_query=watch",
            "https://site.example/?next=/watch",
            "https://site.example/page#watch",
            "https://site.example/watchtower",
            "https://site.example/watch-tower",
            "https://watch.example.com/",
            "https://site.example/videos",
            "https://site.example/playlists",
        ] {
            assert!(!is_watch_shaped_url(url), "{url} must NOT be watch-shaped");
        }
        // AUDIT 0.30.1 (A2-C3): host-only URLs whose query or
        // fragment holds the first slash — through 0.30 these
        // read watch-shaped in Rust while the document gate
        // (location.pathname) never saw a path at all.
        for url in [
            "https://site.example?next=/watch",
            "https://site.example?next=/video/123",
            "https://site.example#/watch",
            "https://site.example:8080?next=/watch",
            "https://site.example?x=1#/clip/9",
        ] {
            assert!(
                !is_watch_shaped_url(url),
                "{url} must NOT be watch-shaped (query/fragment is not a path)"
            );
        }
        // …and a real path in front of such a query still is.
        assert!(is_watch_shaped_url("https://site.example/watch?next=/feed"));
    }

    #[test]
    fn watch_identity_separates_videos_not_presentations() {
        let a = watch_identity("https://www.youtube.com/watch?v=AAA").unwrap();
        // Host-form flips and volatile parameters keep the
        // identity (the designation bridge)…
        assert_eq!(
            a,
            watch_identity("https://youtube.com/watch?v=AAA&t=42&list=PL1").unwrap()
        );
        assert_eq!(
            a,
            watch_identity("https://www.youtube.com/watch?v=AAA#t=10").unwrap()
        );
        // …a different video does not.
        assert_ne!(
            a,
            watch_identity("https://www.youtube.com/watch?v=BBB").unwrap()
        );
        // Path-id sites: the id rides the path; a different
        // path id is a different video…
        assert_ne!(
            watch_identity("https://vimeo.com/video/123").unwrap(),
            watch_identity("https://vimeo.com/video/124").unwrap()
        );
        // …and with no `v` parameter the whole query belongs
        // to the identity, so query churn clears (the
        // documented safe direction for unrecognized shapes).
        assert_ne!(
            watch_identity("https://vimeo.com/video/123").unwrap(),
            watch_identity("https://vimeo.com/video/123?autoplay=0").unwrap()
        );
        // Non-watch-shaped URLs have no identity at all.
        assert_eq!(watch_identity("https://www.youtube.com/"), None);
        assert_eq!(watch_identity("https://site.example?next=/watch"), None);
    }

    #[test]
    fn chosen_survival_matrix() {
        let watch_a = "https://www.youtube.com/watch?v=AAA";
        let watch_a_churn = "https://youtube.com/watch?v=AAA&t=9";
        let watch_b = "https://www.youtube.com/watch?v=BBB";
        let home = "https://www.youtube.com/";
        let feed = "https://www.youtube.com/feed/subscriptions";
        let other = "https://site.example/video/7";
        // Same URL: not a change.
        assert!(chosen_survives_url_change(watch_a, watch_a));
        // The bridge: same video, churned URL.
        assert!(chosen_survives_url_change(watch_a, watch_a_churn));
        assert!(chosen_survives_url_change(watch_a_churn, watch_a));
        // A different video must earn its own designation.
        assert!(!chosen_survives_url_change(watch_a, watch_b));
        // The C4 leak: leaving watch territory clears.
        assert!(!chosen_survives_url_change(watch_a, home));
        assert!(!chosen_survives_url_change(watch_a, feed));
        // Entering watch territory is unaffected.
        assert!(chosen_survives_url_change(home, watch_a));
        assert!(chosen_survives_url_change(feed, other));
        // Non-watch to a different non-watch page clears.
        assert!(!chosen_survives_url_change(home, feed));
        assert!(!chosen_survives_url_change(
            "https://site.example/@chan",
            "https://site.example/@other"
        ));
        // Watch-shaped on another host entirely: different
        // identity (path+id) still governs.
        assert!(chosen_survives_url_change(
            "https://vimeo.com/video/7",
            "https://vimeo.com/video/7"
        ));
    }

    #[test]
    fn chosen_designations_are_per_tab_and_clearable() {
        let rt = LiteRuntime::new(true);
        assert!(!rt.is_chosen(7));
        assert!(rt.mark_chosen(7), "first designation is news");
        assert!(!rt.mark_chosen(7), "a re-report is a no-op");
        assert!(rt.is_chosen(7));
        assert!(!rt.is_chosen(8), "designations are per-tab");
        rt.clear_chosen(7);
        assert!(!rt.is_chosen(7));
        assert!(rt.mark_chosen(7), "after a clear, a new designation counts");
        rt.mark_chosen(9);
        rt.clear_all_chosen();
        assert!(!rt.is_chosen(7) && !rt.is_chosen(9));
    }

    #[test]
    fn lite_class_set_is_exactly_the_heavy_three() {
        use ResourceType::*;
        for rt in [Image, Font, Media] {
            assert!(is_lite_class(rt), "{rt:?}");
        }
        for rt in [Document, Subdocument, Script, Stylesheet, Xhr, Other] {
            assert!(!is_lite_class(rt), "{rt:?}");
        }
    }
}
