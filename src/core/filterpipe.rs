//! The unified request-filter pipeline (AUDIT V26, defect D6).
//!
//! v25 composed its two filters as an afterthought in the COM
//! handler: the Shield decided inside its runtime, then the
//! handler read Lite's flag separately and ran a second,
//! parallel classification with a *different* local-origin test
//! (Lite: the whole 127.0.0.0/8 block; Shield: only 127.0.0.1).
//! A page on 127.0.0.2 was "public" to one layer and "local" to
//! the other; the two layers read their on/off atomics at
//! different moments, so a toggle landing mid-handler could
//! produce a decision neither state described; and Lite-era
//! pages got no document layer at all when the Shield was off.
//!
//! v26 decides every request exactly once, here:
//!
//! * [`classify_origin`] is the ONE origin classifier — page
//!   and request, Shield and Lite all read it. Loopback means
//!   the full 127.0.0.0/8 block, `localhost`, `*.localhost` and
//!   `::1` (the definition Lite and Teleport already used; the
//!   Shield's host test is widened to match, see
//!   `shield::is_local_host`).
//! * [`decide`] takes the two on/off flags as *values* — the
//!   handler snapshots both atomics once per request, so the
//!   decision always describes one real instant of the toggles
//!   (the toggle-race pin).
//! * The verdict is a single enum: a request is allowed, or it
//!   is blocked by exactly one layer, which is the layer that
//!   counts it. Double-counting is impossible by construction.
//!
//! The Shield's ABP rule semantics are untouched: the pipeline
//! calls the same engine entry point the v25 runtime called.
//! What changed is composition, classification and counting —
//! the layering — per the D6 order.
//!
//! AUDIT V29 (Lite semantics v2) extends the composition with
//! two facts per request — whether the page is watch-shaped
//! and whether the tab has designated a chosen video. On a
//! watch-shaped page, or in a chosen tab, Media-class and
//! stream-shaped requests are never Lite-blocked: blocking
//! them was the v28 regression (the main video streams
//! through exactly those shapes). Images and fonts stay
//! Lite-blocked everywhere, so thumbnails and posters still
//! die; the document play gate remains the guarantee that
//! nothing unchosen can actually play.

use crate::core::shield::{host_of, is_local_host, ResourceType, ShieldEngine};

/// Where a URL lives, for filtering purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginClass {
    /// Loopback: this machine. Never filtered, either layer,
    /// in either direction (the local-sacred law).
    Local,
    /// Pico-internal (`pico:` scheme): the shell's own pages.
    Internal,
    /// The public web.
    Public,
}

/// The one origin classifier. A URL with no extractable host
/// (data:, blob:, a malformed string) is `Local` — not because
/// it is local, but because there is no network origin to
/// filter and both layers fail open on the unclassifiable.
pub fn classify_origin(url: &str) -> OriginClass {
    if url.starts_with("pico:") {
        return OriginClass::Internal;
    }
    match host_of(url) {
        Some(host) => {
            if is_local_host(&host) {
                OriginClass::Local
            } else {
                OriginClass::Public
            }
        }
        None => OriginClass::Local,
    }
}

/// The toggle states, snapshotted once per request by the
/// handler (AUDIT V26 D6-f: a decision must describe one real
/// instant of the two flags, never a blend of two reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterState {
    pub shield_on: bool,
    pub lite_on: bool,
}

/// The facts of one resource request, prepared by the handler:
/// the request URL, the page's live URL (the D7 single source
/// of truth via the runtime's page state), the resource type
/// *after* the main-frame reclassification (Document →
/// Subdocument for iframes), and the HTTP method for
/// `$method=` rules. The last two facts are Lite v2's
/// (AUDIT V29): `page_watch_shaped` derives from the page URL
/// via [`crate::core::lite::is_watch_shaped_url`], and
/// `tab_chosen` is the tab's chosen-video designation in the
/// shared Lite runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestFacts {
    pub url: String,
    pub page_url: String,
    pub rtype: ResourceType,
    pub method: Option<String>,
    pub page_watch_shaped: bool,
    pub tab_chosen: bool,
}

/// One request, one verdict. The Block variants carry the
/// resource type so the handler can build the neutered response
/// and the log line; attribution travels with the verdict, so
/// exactly one counter can move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    ShieldBlock(ResourceType),
    LiteBlock(ResourceType),
}

/// Decide one request.
///
/// Order (each step is a law, pinned by the matrix tests):
/// 1. Local/internal on EITHER side (page or request) → Allow.
///    The local-sacred law, one classifier, both layers.
/// 2. Shield on + engine says block → ShieldBlock. The
///    Shield's verdict is final: Lite never sees the request.
/// 3. Lite on (semantics v2, AUDIT V29) → LiteBlock, where:
///    image/font classes are blocked always; the media class
///    and XHR/other media-stream shapes are blocked ONLY when
///    the page is not watch-shaped AND the tab has no chosen
///    video — on a watch page or in a chosen tab they are the
///    chosen video's own bytes and pass.
/// 4. Allow.
pub fn decide(facts: &RequestFacts, state: FilterState, engine: Option<&ShieldEngine>) -> Verdict {
    if classify_origin(&facts.page_url) != OriginClass::Public
        || classify_origin(&facts.url) != OriginClass::Public
    {
        return Verdict::Allow;
    }
    if state.shield_on {
        // Only GET and HEAD requests are ever Shield-blocked
        // (v22 law; the pipeline owns it): neutering a non-GET
        // response can break form posts and API writes, so the
        // Shield step is skipped entirely for them. HEAD carries
        // no body to corrupt, so it stands with GET (AUDIT V28,
        // A2-M2 — it was silently exempt through v27). An
        // unnamed method proceeds. AUDIT V28 also removed the
        // v25-era `ShieldRuntime::should_block` seam: it had no
        // production callers left, yet its tests asserted POST
        // blocking the real browser never performed; the
        // pipeline's matrix tests now pin the law where it
        // actually runs.
        let method_allows = facts
            .method
            .as_deref()
            .map(|m| m.eq_ignore_ascii_case("GET") || m.eq_ignore_ascii_case("HEAD"))
            .unwrap_or(true);
        if method_allows {
            if let Some(engine) = engine {
                if engine
                    .decide_with_method(
                        &facts.url,
                        &facts.page_url,
                        facts.rtype,
                        facts.method.as_deref(),
                    )
                    .is_block()
                {
                    return Verdict::ShieldBlock(facts.rtype);
                }
            }
        }
    }
    if state.lite_on {
        // The chosen video's exemption (AUDIT V29): the page
        // being watch-shaped, or the tab having designated a
        // chosen video, means Media-class bytes and stream
        // shapes are presumed to be the video the user picked
        // until the play gate says otherwise — the gate, not
        // the network, is what can tell the chosen element
        // from a sidebar preview, so the network's job here is
        // only to not starve it. Images and fonts get no
        // exemption: thumbnails and posters stay dead even on
        // watch pages (and a poster is not needed to play).
        let chosen_context = facts.page_watch_shaped || facts.tab_chosen;
        if crate::core::lite::is_lite_class(facts.rtype) {
            if facts.rtype == ResourceType::Media && chosen_context {
                return Verdict::Allow;
            }
            return Verdict::LiteBlock(facts.rtype);
        }
        // Lite stream backstop (v28, narrowed v29): MSE
        // streaming fetches its segments through fetch/XHR,
        // whose context classifies as Xhr — never a Lite class
        // — so streamed video (YouTube hover previews first
        // among them) sailed through Lite while direct <video>
        // loads were blocked. A fetch/XHR-shaped request whose
        // URL is a media-stream shape is Lite's too — but ONLY
        // outside a chosen context: the main video streams
        // through exactly these shapes, and v28 blocked them
        // unconditionally, which is why the chosen video
        // starved. Default-deny stands on home/channel/search
        // pages (their stream fetches are previews). The
        // matcher is deliberately narrow
        // (core::lite::is_media_stream_url): API JSON —
        // including the innertube player responses the Shield
        // prunes — is not a stream shape and never matches.
        if !chosen_context
            && matches!(facts.rtype, ResourceType::Xhr | ResourceType::Other)
            && crate::core::lite::is_media_stream_url(&facts.url)
        {
            return Verdict::LiteBlock(facts.rtype);
        }
    }
    Verdict::Allow
}

// ---------------------------------------------------------------------------
// The ghost-box collapse layer (AUDIT V26, defect D6-a)
// ---------------------------------------------------------------------------

/// Document-layer companion to the network pipeline: when a
/// filter blocks an image, the network answer is a 1×1 GIF or an
/// empty body — the *element* keeps whatever box the page's CSS
/// gave it, and an image that fails outright keeps a
/// broken-image placeholder. On long-tail sites without
/// host-specific cosmetics (a long-tail-site screenshot:
/// ad sidebars of broken-image boxes in fully visible
/// containers) nothing ever collapses those boxes. This runtime
/// does: it listens for image load failures in the capture
/// phase (resource errors do not bubble) and sweeps for
/// already-broken images at DOMContentLoaded/load, collapsing
/// the `<img>` (or its `<picture>` parent) with an inline
/// `display:none !important`.
///
/// Laws, mirroring the other runtimes:
/// - No-op on non-http(s) and loopback pages (the classifier's
///   definition, spelled for JS: localhost, *.localhost, ::1,
///   0.0.0.0, 127.0.0.0/8 as a numeric literal — AUDIT V28
///   A2-L2 aligned the spelling with the Rust classifier).
/// - Dynamically disarmed by `window.__picoCollapseOff`, which
///   the app maintains on every load AND on every toggle from
///   the live flags (AUDIT V28 A2-M7: through v27 only loads
///   maintained it, so a mid-session toggle left the layer in
///   the previous state's posture until the next navigation).
/// - Never throws into the page; every step is guarded.
///
/// AUDIT V28 (A2-H2) narrowed WHAT it collapses. Through v27
/// any image error — and any `<source>` error — collapsed the
/// element immediately, which broke two legitimate patterns:
/// a `<picture>` whose first candidate 404s (the source error
/// collapsed the whole picture before the fallback candidate
/// was tried) and the classic onerror fallback-src swap (the
/// capture listener hid the element before the page's own
/// handler could recover it). Now: `<source>` errors never
/// collapse (candidate fallthrough is the element's job; the
/// `<img>` errors only when every candidate is dead), and an
/// `<img>` error schedules a deferred revalidation — the
/// collapse lands only if the element is still broken, with
/// the same src, a moment later. Pico's own blocked images
/// never error at all (they are answered with a 1×1 GIF); the
/// layer's real targets are images that fail outright — dead
/// ad slots and filter-killed embeds — which stay broken
/// through the revalidation window and still collapse.
///
/// It rides inside the Shield's document script (third segment)
/// and is also injected standalone on Lite-only pages, where no
/// Shield script exists. Its own guard flag makes the two
/// paths mutually exclusive.
pub const COLLAPSE_RUNTIME: &str = r#"(function () {
  try {
    if (window.__picoCollapse) return;
    window.__picoCollapse = 1;
    if (location.protocol !== "http:" && location.protocol !== "https:") return;
    // AUDIT 0.31 (A2-F4+F5): normalize the hostname ONCE at
    // acquisition — strip trailing dots, like the Rust layer's
    // host_of already does — so foo.localhost. / 127.0.0.1.
    // classify loopback here exactly as they do in Rust.
    var host = (location.hostname || "").toLowerCase().replace(/\.+$/, "");
    if (!host || host === "localhost" || host === "localhost." || host === "::1" || host === "[::1]" || host === "0.0.0.0" ||
        host.endsWith(".localhost") || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)) return;
    function off() { return window.__picoCollapseOff === 1; }
    function collapse(el) {
      try {
        var t = el;
        if (el.closest && el.parentNode && el.parentNode.tagName === "PICTURE") {
          t = el.closest("picture") || el;
        }
        if (t.getAttribute && t.getAttribute("data-pico-collapsed")) return;
        if (t.setAttribute) t.setAttribute("data-pico-collapsed", "1");
        if (t.style) t.style.setProperty("display", "none", "important");
      } catch (e) { /* a hostile element must not break the layer */ }
    }
    window.addEventListener("error", function (ev) {
      if (off()) return;
      var t = ev && ev.target;
      // A2-H2: SOURCE errors are candidate fallthrough, never
      // a collapse reason.
      if (!t || t.tagName !== "IMG") return;
      var srcAtError = "";
      try { srcAtError = t.getAttribute("src") || ""; } catch (e) {}
      // Deferred revalidation: collapse only if the element is
      // still there, still on the same src, and still broken —
      // a fallback swap or a late success rescues it.
      setTimeout(function () {
        try {
          if (off()) return;
          if (!t.isConnected) return;
          if ((t.getAttribute("src") || "") !== srcAtError) return;
          if (t.complete && t.naturalWidth === 0) collapse(t);
        } catch (e) { /* element gone or hostile: nothing to do */ }
      }, 350);
    }, true);
    function sweep() {
      if (off()) return;
      var imgs = document.getElementsByTagName("img");
      for (var i = 0; i < imgs.length; i++) {
        var im = imgs[i];
        try {
          if (im.complete && im.naturalWidth === 0 && im.getAttribute("src")) collapse(im);
        } catch (e) { /* keep sweeping */ }
      }
    }
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", sweep);
    } else {
      sweep();
    }
    window.addEventListener("load", sweep);
  } catch (e) { /* never break the page */ }
})();
"#;

// ---------------------------------------------------------------------------
// The Lite play gate (v28 — the hover-preview fix, document half)
// ---------------------------------------------------------------------------

/// Why this exists (the user's reports, 2026-11-01): v28's
/// gate fixed the first report — with Lite on, YouTube still
/// played inline previews when a thumbnail was hovered — and
/// caused the second: the video the user actually wanted to
/// watch no longer loaded at all. The network layer never sees
/// preview bytes as media — MSE streaming fetches segments
/// through fetch/XHR (WebView2 context: Xhr), and the preview
/// element plays from a `blob:` URL — but the MAIN video
/// streams through exactly the same shapes, and its player
/// calls `play()` programmatically after navigation with no
/// per-element gesture. v28 refused both. The user overruled
/// v28's "no watch-page exception, uniform law" decision, and
/// v29's law is the CHOSEN-VIDEO MODEL:
///
/// - Each page context designates at most ONE chosen media
///   element. The chosen element's `play()` always passes —
///   including the player's own repeated retry calls. Every
///   other element's `play()` is refused with a
///   `NotAllowedError` rejection — the exact shape a browser's
///   own autoplay policy produces, so pages handle it
///   routinely — and the element stays paused.
/// - Designation happens when: (i) the user taps the element
///   or its controls container (a pointerdown/click/keydown
///   marks the element, its media ancestors, and the media of
///   any small container around the tap — a feed-sized
///   container marks nothing); (ii) a route change
///   (pushState/replaceState/popstate, watched by the gate
///   itself) follows a pointerdown anywhere within 2 s — the
///   thumbnail-tap case — and the next LARGE video (≥ 320 CSS
///   px wide, or ≥ 40% of the viewport) to ask becomes
///   chosen; (iii) a fresh load lands on a watch-shaped URL
///   (the [`crate::core::lite`] segment corpus, spelled again
///   here) — the paste-a-watch-link case — and the first
///   LARGE video to ask becomes chosen with no gesture (a
///   size-less element falls back to first-video-in-DOM).
/// - Hover NEVER designates: hover events are simply not
///   listened to, and v29 drops v28's transient-activation
///   pass — activation blesses the document, not an element,
///   and under the chosen model it let previews through after
///   any click.
/// - Designation resets on route change and on Lite toggle
///   (re-arm calls `reset()`); once chosen, other elements
///   stay refused. On designation the gate reports
///   `{kind:"lite_chosen"}` over `window.ipc` — once per
///   context, payload-free — so the app can set the tab's
///   chosen flag for the network facts (see [`decide`]).
/// - A capture-phase `play`-event belt handles playback that
///   bypasses the wrapper (attribute autoplay): an unchosen
///   element that starts is paused, unless it is eligible to
///   become chosen (a watch page's autoplaying main video),
///   in which case the belt designates it. Installing or
///   re-arming sweeps already-playing media the same way.
/// - Disarm is the app's job: toggling Lite off sets
///   `window.__picoLiteGateOff = 1` on live documents (the
///   wrapper then passes through untouched) — and
///   `__picoLiteGate.restore()` puts the exact original
///   prototype method back, unwraps the gate's history hooks
///   and removes the listeners, for a full, provable
///   restoration. The guard object makes a second install a
///   re-arm, never a double wrap. The gate wraps `play()` and
///   the two history methods only; the Shield scriptlet
///   runtime wraps fetch/XHR/JSON — orthogonal surfaces.
/// - Loopback/internal pages are exempt, like every Lite law:
///   the runtime returns before touching the prototype.
pub const LITE_GATE_RUNTIME: &str = r#"(function () {
  try {
    if (location.protocol !== "http:" && location.protocol !== "https:") return;
    // AUDIT 0.31 (A2-F4+F5): normalize the hostname ONCE at
    // acquisition — strip trailing dots, like the Rust layer's
    // host_of already does — so foo.localhost. / 127.0.0.1.
    // classify loopback here exactly as they do in Rust.
    var host = (location.hostname || "").toLowerCase().replace(/\.+$/, "");
    if (!host || host === "localhost" || host === "localhost." || host === "::1" || host === "[::1]" || host === "0.0.0.0" ||
        host.endsWith(".localhost") || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)) return;
    if (window.__picoLiteGate && window.__picoLiteGate.installed) {
      // Re-arm (Lite toggled back on for a live document):
      // clear the disarm flag, reset the designation (the
      // toggle law) and stop anything unchosen that is
      // playing right now.
      window.__picoLiteGateOff = 0;
      try { window.__picoLiteGate.reset(); } catch (e) {}
      try { window.__picoLiteGate.sweep(); } catch (e) {}
      return;
    }
    var proto = window.HTMLMediaElement && window.HTMLMediaElement.prototype;
    if (!proto || typeof proto.play !== "function") return;
    var original = proto.play;
    var marks = new WeakMap();
    var INTENT_WINDOW_MS = 2000;
    var ROUTE_INTENT_TTL_MS = 30000;
    var WATCH_SEGMENTS = { watch: 1, video: 1, view: 1, play: 1, episode: 1, clip: 1 };
    var chosen = null;
    var reported = false;
    // AUDIT 0.31 (A2-F2): route intent is gesture intent, not
    // pointer-only intent — a keyboard user who tabs to a
    // thumbnail and presses Enter produces no pointerdown at
    // all, so `lastGestureAt` is stamped by pointerdown AND
    // by any keydown, and onRoute reads the most recent one.
    var lastGestureAt = 0;
    var routeIntentUntil = 0;
    var routeHref = "";
    var freshWatch = false;
    function watchShaped() {
      try {
        var parts = (location.pathname || "").split("/");
        for (var i = 0; i < parts.length; i++) {
          if (WATCH_SEGMENTS[parts[i].toLowerCase()]) return true;
        }
      } catch (e) {}
      return false;
    }
    function resetChoice() {
      chosen = null;
      reported = false;
      routeIntentUntil = 0;
      freshWatch = watchShaped();
    }
    // AUDIT 0.31 (A2-F3): the watch identity of a URL, a
    // faithful JS port of core::lite::watch_identity — the
    // pathname if it is watch-shaped (same segment corpus),
    // plus the video id: the `v` query parameter when one is
    // present, else the whole query string; the fragment is
    // never part of it. null for non-watch-shaped URLs.
    function watchIdentityOf(href) {
      try {
        var u = new URL(href);
        var parts = (u.pathname || "").split("/");
        var shaped = false;
        for (var i = 0; i < parts.length; i++) {
          if (WATCH_SEGMENTS[parts[i].toLowerCase()]) { shaped = true; break; }
        }
        if (!shaped) return null;
        var query = (u.search || "").replace(/^\?/, "");
        var id = query;
        var pairs = query.split("&");
        for (var j = 0; j < pairs.length; j++) {
          var eq = pairs[j].indexOf("=");
          if (eq >= 0 && pairs[j].slice(0, eq) === "v") { id = pairs[j].slice(eq + 1); break; }
        }
        return (u.pathname || "") + "|" + id;
      } catch (e) { return null; }
    }
    function onRoute(ev) {
      try {
        var h = location.href;
        if (h === routeHref) return;
        var prevHref = routeHref;
        routeHref = h;
        // AUDIT 0.31 (A2-F3): same-video churn is not a new
        // page — a watch URL whose watch identity equals the
        // previous URL's (host-form flips, `t`/`list` updates
        // around the SAME video) keeps its designation,
        // mirroring core::lite::chosen_survives_url_change.
        // A genuinely new watch page falls through to the
        // fresh designation logic below.
        var prevId = watchIdentityOf(prevHref);
        var newId = watchIdentityOf(h);
        if (prevId !== null && newId !== null && prevId === newId) return;
        // AUDIT 0.31 (A2-F2): intent follows the most recent
        // GESTURE (pointer or keyboard); a popstate-driven
        // route change (Back/Forward — the user demonstrably
        // navigating) counts as intent itself for a short
        // window, even with no fresh gesture stamp.
        var followedGesture = lastGestureAt > 0 && (Date.now() - lastGestureAt) <= INTENT_WINDOW_MS;
        var viaPopstate = !!(ev && ev.type === "popstate");
        chosen = null;
        reported = false;
        freshWatch = false;
        routeIntentUntil = followedGesture ? Date.now() + ROUTE_INTENT_TTL_MS : (viaPopstate ? Date.now() + 2000 : 0);
      } catch (e) {}
    }
    function wrapHistory(name) {
      try {
        var orig = history[name];
        if (typeof orig !== "function" || orig.__picoLiteWrapped) return;
        var wrapped = function () {
          var r = orig.apply(this, arguments);
          onRoute();
          return r;
        };
        wrapped.__picoLiteWrapped = true;
        wrapped.__picoLiteOriginal = orig;
        history[name] = wrapped;
      } catch (e) {}
    }
    function markTree(target, now) {
      try {
        var node = target, hops = 0;
        while (node && hops < 8) {
          if (node instanceof window.HTMLMediaElement) marks.set(node, now);
          // A container holding a handful of media is a
          // player (the element, its poster, its controls);
          // a container holding a feed is not — marking its
          // media would bless every preview on the page.
          // AUDIT 0.31 (A2-F13): the handful rule blessed ALL
          // of a container's media on any tap inside it, so
          // clicking TEXT in a two/three-video container
          // designated whichever of them asked first. Now: a
          // container holding exactly ONE media element keeps
          // the old behavior (a tap anywhere on the player
          // marks its video); a container holding MORE THAN
          // ONE marks only the media elements that contain
          // the event target (or are the target) — text in a
          // multi-media container blesses nothing.
          if (node.querySelectorAll) {
            var found = node.querySelectorAll("video, audio");
            if (found.length === 1) {
              marks.set(found[0], now);
            } else if (found.length > 1) {
              for (var i = 0; i < found.length; i++) {
                var m = found[i];
                try {
                  if (m === target || (target && m.contains && m.contains(target))) marks.set(m, now);
                } catch (e) {}
              }
            }
          }
          node = node.parentElement || node.parentNode;
          hops++;
        }
      } catch (e) {}
    }
    function onGesture(ev) {
      try {
        var now = Date.now();
        // AUDIT 0.31 (A2-F2): any keydown is a user gesture
        // for route-intent purposes, exactly as pointerdown
        // is (Enter/Space keyboard navigation included).
        if (ev && (ev.type === "pointerdown" || ev.type === "keydown")) lastGestureAt = now;
        markTree(ev && ev.target, now);
        // Keyboard intent lands on the focused element, which
        // is not always the event target chain in every
        // engine — but a keypress with the body focused (a
        // scroll space, say) must not bless a whole player.
        try {
          var ae = document.activeElement;
          if (ev && ev.type === "keydown" && ae && ae !== document.body) markTree(ae, now);
        } catch (e) {}
      } catch (e) {}
    }
    function marked(el) {
      try {
        var t = marks.get(el);
        return !!(t && (Date.now() - t) < INTENT_WINDOW_MS);
      } catch (e) { return false; }
    }
    function renderedWidth(el) {
      try {
        if (el.getBoundingClientRect) {
          var w = el.getBoundingClientRect().width;
          if (typeof w === "number" && w > 0) return w;
        }
        if (el.clientWidth) return el.clientWidth;
      } catch (e) {}
      return 0;
    }
    function isLarge(el) {
      var w = renderedWidth(el);
      if (w >= 320) return true;
      try {
        var vw = window.innerWidth || 0;
        if (vw > 0 && w >= vw * 0.4) return true;
      } catch (e) {}
      return false;
    }
    function isFirstVideo(el) {
      try { return el.tagName === "VIDEO" && document.querySelector("video") === el; } catch (e) { return false; }
    }
    function canChoose(el) {
      // (i) A direct tap on the element or its controls
      // container chooses it, on any page, at any size.
      if (marked(el)) return true;
      try { if (!el || el.tagName !== "VIDEO") return false; } catch (e) { return false; }
      // (ii) A route change that followed a tap anywhere:
      // the next LARGE video to ask becomes chosen.
      if (routeIntentUntil && Date.now() <= routeIntentUntil && isLarge(el)) return true;
      // (iii) A fresh watch-shaped load: the first LARGE
      // video to ask becomes chosen with no gesture at all;
      // if it has no layout size yet, the first video in DOM
      // order stands in for it.
      if (freshWatch && (isLarge(el) || (renderedWidth(el) === 0 && isFirstVideo(el)))) return true;
      return false;
    }
    function designate(el) {
      chosen = el;
      freshWatch = false;
      routeIntentUntil = 0;
      if (!reported) {
        reported = true;
        try {
          if (window.ipc && typeof window.ipc.postMessage === "function") {
            window.ipc.postMessage(JSON.stringify({ kind: "lite_chosen" }));
          }
        } catch (e) {}
      }
    }
    function refusal() {
      try {
        return Promise.reject(new DOMException("play() refused by Pico Lite: not the chosen video", "NotAllowedError"));
      } catch (e) {
        var err = new Error("play() refused by Pico Lite: not the chosen video");
        err.name = "NotAllowedError";
        return Promise.reject(err);
      }
    }
    function gatedPlay() {
      try {
        if (window.__picoLiteGateOff === 1) return original.apply(this, arguments);
        if (this === chosen) return original.apply(this, arguments);
        if (!chosen && canChoose(this)) {
          designate(this);
          return original.apply(this, arguments);
        }
        try { this.pause(); } catch (e) {}
        return refusal();
      } catch (e) {
        return original.apply(this, arguments);
      }
    }
    function onPlayEvent(ev) {
      try {
        if (window.__picoLiteGateOff === 1) return;
        var el = ev && ev.target;
        if (!el || !(el instanceof window.HTMLMediaElement)) return;
        if (el === chosen) return;
        if (!chosen && canChoose(el)) { designate(el); return; }
        try { el.pause(); } catch (e) {}
      } catch (e) {}
    }
    function sweep() {
      try {
        var ms = document.querySelectorAll("video, audio");
        for (var i = 0; i < ms.length; i++) {
          var el = ms[i];
          try {
            if (el.paused || el === chosen) continue;
            if (!chosen && canChoose(el)) { designate(el); continue; }
            el.pause();
          } catch (e) {}
        }
      } catch (e) {}
    }
    function restore() {
      try { proto.play = original; } catch (e) {}
      try {
        if (history.pushState && history.pushState.__picoLiteWrapped) history.pushState = history.pushState.__picoLiteOriginal;
        if (history.replaceState && history.replaceState.__picoLiteWrapped) history.replaceState = history.replaceState.__picoLiteOriginal;
      } catch (e) {}
      try {
        document.removeEventListener("pointerdown", onGesture, true);
        document.removeEventListener("click", onGesture, true);
        document.removeEventListener("keydown", onGesture, true);
        document.removeEventListener("play", onPlayEvent, true);
        window.removeEventListener("popstate", onRoute);
        window.removeEventListener("hashchange", onRoute);
      } catch (e) {}
      try { delete window.__picoLiteGate; } catch (e) { window.__picoLiteGate = undefined; }
    }
    try { routeHref = location.href; } catch (e) {}
    freshWatch = watchShaped();
    wrapHistory("pushState");
    wrapHistory("replaceState");
    proto.play = gatedPlay;
    document.addEventListener("pointerdown", onGesture, true);
    document.addEventListener("click", onGesture, true);
    document.addEventListener("keydown", onGesture, true);
    document.addEventListener("play", onPlayEvent, true);
    window.addEventListener("popstate", onRoute);
    window.addEventListener("hashchange", onRoute);
    window.__picoLiteGateOff = 0;
    window.__picoLiteGate = { installed: true, sweep: sweep, restore: restore, reset: function () {
      try { routeHref = location.href; } catch (e) {}
      resetChoice();
    } };
    sweep();
  } catch (e) { /* never break the page */ }
})();
"#;

/// The standalone Lite document script: collapse layer + play
/// gate, in that order, as ONE script — what the app injects
/// when Lite must reach a page the Shield's document script
/// does not (or no longer) cover. Each runtime carries its own
/// guard flag, so a page that already has either piece simply
/// re-arms it.
pub fn lite_document_script() -> String {
    let mut s = String::with_capacity(COLLAPSE_RUNTIME.len() + LITE_GATE_RUNTIME.len() + 1);
    s.push_str(COLLAPSE_RUNTIME);
    s.push('\n');
    s.push_str(LITE_GATE_RUNTIME);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::shield::ResourceType::*;

    fn facts(url: &str, page: &str, rtype: ResourceType) -> RequestFacts {
        RequestFacts {
            url: url.to_string(),
            page_url: page.to_string(),
            rtype,
            method: Some("GET".to_string()),
            page_watch_shaped: crate::core::lite::is_watch_shaped_url(page),
            tab_chosen: false,
        }
    }

    fn engine() -> ShieldEngine {
        // A tiny hand-fed list is enough to exercise the
        // composition: one anchored ad rule + one generic rule.
        ShieldEngine::compile(&["||ads.example.com^", "/banner-ad.", "---not-a-rule---"])
    }

    const ON_ON: FilterState = FilterState {
        shield_on: true,
        lite_on: true,
    };

    #[test]
    fn classifier_agrees_with_itself_everywhere() {
        assert_eq!(
            classify_origin("http://127.0.0.2:8080/x"),
            OriginClass::Local
        );
        assert_eq!(classify_origin("http://127.0.0.1/x"), OriginClass::Local);
        assert_eq!(classify_origin("http://dev.localhost/"), OriginClass::Local);
        assert_eq!(classify_origin("http://[::1]:3000/"), OriginClass::Local);
        assert_eq!(classify_origin("pico://start"), OriginClass::Internal);
        assert_eq!(classify_origin("https://downloads.example.net/"), OriginClass::Public);
        assert_eq!(
            classify_origin("data:image/png;base64,x"),
            OriginClass::Local
        );
    }

    #[test]
    fn matrix_shield_hit_wins_over_lite() {
        let e = engine();
        // An image the Shield blocks is the Shield's, even with
        // Lite also on (both layers want it; attribution is the
        // Shield's and Lite never counts it).
        let f = facts(
            "https://ads.example.com/banner.png",
            "https://news.example/",
            Image,
        );
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::ShieldBlock(Image));
        // Shield off → the same image becomes Lite's.
        let f2 = facts(
            "https://ads.example.com/banner.png",
            "https://news.example/",
            Image,
        );
        assert_eq!(
            decide(
                &f2,
                FilterState {
                    shield_on: false,
                    lite_on: true
                },
                Some(&e)
            ),
            Verdict::LiteBlock(Image)
        );
    }

    #[test]
    fn matrix_classes_and_toggles() {
        let e = engine();
        let page = "https://news.example/";
        // Lite classes blocked only by Lite; non-Lite classes
        // pass both layers when no rule hits.
        for rt in [Image, Font, Media] {
            let f = facts("https://cdn.example/file", page, rt);
            assert_eq!(
                decide(&f, ON_ON, Some(&e)),
                Verdict::LiteBlock(rt),
                "{rt:?}"
            );
        }
        for rt in [Script, Stylesheet, Xhr, Document, Other] {
            let f = facts("https://cdn.example/file", page, rt);
            assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow, "{rt:?}");
        }
        // Both off: even a rule-hit image is allowed (the
        // engine is never consulted with shield_on=false, and
        // the toggle snapshot is what decides).
        let f = facts("https://ads.example.com/banner.png", page, Image);
        assert_eq!(
            decide(
                &f,
                FilterState {
                    shield_on: false,
                    lite_on: false
                },
                Some(&e)
            ),
            Verdict::Allow
        );
        // A generic-rule script hit is the Shield's with Lite on.
        let f = facts("https://cdn.example/banner-ad.js", page, Script);
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::ShieldBlock(Script));
        // No engine yet (still compiling): fail open for the
        // Shield; Lite still works (it needs no engine).
        let f = facts("https://cdn.example/pic.png", page, Image);
        assert_eq!(decide(&f, ON_ON, None), Verdict::LiteBlock(Image));
        let f = facts("https://ads.example.com/banner.png", page, Image);
        assert_eq!(decide(&f, ON_ON, None), Verdict::LiteBlock(Image));
    }

    #[test]
    fn matrix_local_is_sacred_both_directions() {
        let e = engine();
        // A 127.0.0.2 page (the v25 classifier disagreement):
        // both layers now agree it is local.
        let f = facts(
            "https://ads.example.com/banner.png",
            "http://127.0.0.2:8080/app",
            Image,
        );
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow);
        // A local REQUEST on a public page is likewise sacred.
        let f = facts(
            "http://localhost:9000/banner-ad.js",
            "https://news.example/",
            Script,
        );
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow);
        // Internal page: nothing is filtered.
        let f = facts("https://ads.example.com/x.png", "pico://start", Image);
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow);
    }

    #[test]
    fn collapse_runtime_carries_its_laws() {
        // The JS is data here; pin the load-bearing spellings so
        // an edit cannot quietly drop a law.
        assert!(COLLAPSE_RUNTIME.contains("window.__picoCollapseOff"));
        // A2-L2: the loopback spelling is the exact numeric
        // 127/8 literal test, shared with the other runtimes.
        assert!(COLLAPSE_RUNTIME.contains("/^127\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}$/"));
        assert!(COLLAPSE_RUNTIME.contains("endsWith(\".localhost\")"));
        assert!(COLLAPSE_RUNTIME.contains("data-pico-collapsed"));
        assert!(COLLAPSE_RUNTIME.contains("naturalWidth === 0"));
        assert!(
            COLLAPSE_RUNTIME.contains("}, true);"),
            "capture-phase listener"
        );
        // A2-H2: SOURCE errors never collapse (candidate
        // fallthrough), and IMG collapses are deferred until
        // revalidation — the two spellings that carry the fix.
        assert!(COLLAPSE_RUNTIME.contains("t.tagName !== \"IMG\""));
        assert!(COLLAPSE_RUNTIME.contains("setTimeout"));
        assert!(COLLAPSE_RUNTIME.contains("srcAtError"));
    }

    #[test]
    fn lite_gate_runtime_carries_its_laws() {
        // The chosen-video gate's document half: pin the
        // spellings that carry its laws (gesture marking on
        // real intent events only, the designation report, the
        // route watcher, the watch corpus, the size test, the
        // NotAllowedError refusal shape, the disarm flag, the
        // exact-restore handle, the double-wrap guard).
        assert!(LITE_GATE_RUNTIME.contains("window.__picoLiteGate"));
        assert!(LITE_GATE_RUNTIME.contains("window.__picoLiteGateOff"));
        assert!(LITE_GATE_RUNTIME.contains("\"pointerdown\""));
        assert!(LITE_GATE_RUNTIME.contains("\"click\""));
        assert!(LITE_GATE_RUNTIME.contains("\"keydown\""));
        assert!(
            !LITE_GATE_RUNTIME.contains("mouseenter"),
            "hover must never mark intent"
        );
        assert!(
            !LITE_GATE_RUNTIME.contains("mouseover"),
            "hover must never mark intent"
        );
        // V29 dropped v28's transient-activation pass: it
        // blessed the document, not an element, and let
        // previews play after any click.
        assert!(
            !LITE_GATE_RUNTIME.contains("userActivation"),
            "no document-wide activation pass under the chosen model"
        );
        assert!(LITE_GATE_RUNTIME.contains("NotAllowedError"));
        assert!(LITE_GATE_RUNTIME.contains("restore"));
        assert!(LITE_GATE_RUNTIME.contains("reset"));
        assert!(LITE_GATE_RUNTIME.contains("/^127\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}$/"));
        // The chosen model, spelled: the report, the route
        // hooks, the corpus, the LARGE test.
        assert!(LITE_GATE_RUNTIME.contains("lite_chosen"));
        // AUDIT 0.31 (A2-F2/F3): gesture-based route intent
        // (pointer + keyboard), the popstate intent window,
        // and the JS watch-identity port that lets same-video
        // churn keep its designation.
        assert!(LITE_GATE_RUNTIME.contains("lastGestureAt"));
        assert!(LITE_GATE_RUNTIME.contains("watchIdentityOf"));
        assert!(LITE_GATE_RUNTIME.contains("prevId === newId"));
        assert!(LITE_GATE_RUNTIME.contains("viaPopstate"));
        // AUDIT 0.31 (A2-F13): multi-media containers mark
        // only the media containing the event target.
        assert!(LITE_GATE_RUNTIME.contains("m.contains(target)"));
        assert!(LITE_GATE_RUNTIME.contains("found.length === 1"));
        // AUDIT 0.31 (A2-F4+F5): trailing-dot host
        // normalization at acquisition, in both filterpipe
        // runtimes.
        assert!(LITE_GATE_RUNTIME.contains("replace(/\\.+$/, \"\")"));
        assert!(COLLAPSE_RUNTIME.contains("replace(/\\.+$/, \"\")"));
        assert!(LITE_GATE_RUNTIME.contains("pushState"));
        assert!(LITE_GATE_RUNTIME.contains("replaceState"));
        assert!(LITE_GATE_RUNTIME.contains("popstate"));
        assert!(LITE_GATE_RUNTIME.contains("getBoundingClientRect"));
        for seg in crate::core::lite::WATCH_PATH_SEGMENTS {
            assert!(
                LITE_GATE_RUNTIME.contains(&format!("{seg}: 1")),
                "JS watch corpus must spell {seg}"
            );
        }
        // The gate wraps play() and the two history methods —
        // fetch/XHR belong to the Shield scriptlet runtime
        // (orthogonal surfaces).
        assert!(LITE_GATE_RUNTIME.contains("proto.play = gatedPlay"));
        assert!(!LITE_GATE_RUNTIME.contains("window.fetch"));
        assert!(!LITE_GATE_RUNTIME.contains("XMLHttpRequest"));
    }

    #[test]
    fn lite_document_script_is_collapse_then_gate_once_each() {
        let s = lite_document_script();
        let c = s.find("window.__picoCollapse = 1").unwrap_or(usize::MAX);
        let g = s.find("window.__picoLiteGate = {").unwrap_or(usize::MAX);
        assert!(c < g, "collapse installs before the gate");
        assert_eq!(s.matches("__picoLiteGate = {").count(), 1);
    }

    #[test]
    fn matrix_lite_stream_backstop() {
        let e = engine();
        let page = "https://www.youtube.com/";
        let lite_only = FilterState {
            shield_on: false,
            lite_on: true,
        };
        // MSE segment shapes through fetch/XHR: Lite's now.
        for url in [
            "https://rr1.googlevideo.com/videoplayback?id=abc&itag=137",
            "https://cdn.example/hls/master.m3u8",
            "https://cdn.example/dash/manifest.mpd",
            "https://cdn.example/v/segment-0042.ts",
            "https://cdn.example/media/movie.mp4?token=zzz",
            "https://cdn.example/audio/track.m4a",
        ] {
            let f = facts(url, page, Xhr);
            assert_eq!(
                decide(&f, lite_only, Some(&e)),
                Verdict::LiteBlock(Xhr),
                "{url}"
            );
        }
        // The innertube API is NOT a stream shape: the Shield's
        // prune path owns it; the backstop must never touch it.
        for url in [
            "https://www.youtube.com/youtubei/v1/player?key=abc",
            "https://www.youtube.com/youtubei/v1/browse?key=abc",
            "https://api.example/v1/data.json",
            "https://cdn.example/app.js",
            "https://cdn.example/style.css",
        ] {
            let f = facts(url, page, Xhr);
            assert_eq!(decide(&f, lite_only, Some(&e)), Verdict::Allow, "{url}");
        }
        // Lite off: the backstop is off with it.
        let f = facts(
            "https://rr1.googlevideo.com/videoplayback?id=abc",
            page,
            Xhr,
        );
        assert_eq!(
            decide(
                &f,
                FilterState {
                    shield_on: false,
                    lite_on: false
                },
                Some(&e)
            ),
            Verdict::Allow
        );
        // A stream shape on a LOCAL page stays sacred.
        let f = facts(
            "https://cdn.example/hls/master.m3u8",
            "http://127.0.0.1:5173/",
            Xhr,
        );
        assert_eq!(decide(&f, lite_only, Some(&e)), Verdict::Allow);
    }

    #[test]
    fn regression_watch_page_main_video_lives_under_lite() {
        // THE v29 regression guard (network half; the play
        // half is the DOM watch fixture): on a watch-shaped
        // page with Lite ON, everything the main video needs
        // passes the real decision — its MSE segment fetches,
        // its direct media loads — while its poster/thumbnail
        // images stay Lite-blocked. v28 blocked the segments
        // and starved the video; this matrix is the witness
        // inversion, pinned.
        let e = engine();
        let page = "https://www.youtube.com/watch?v=fixture1";
        let lite_only = FilterState {
            shield_on: false,
            lite_on: true,
        };
        for (url, rt) in [
            (
                "https://rr1.googlevideo.com/videoplayback?id=abc&itag=137",
                Xhr,
            ),
            (
                "https://rr1.googlevideo.com/videoplayback?id=abc&itag=140",
                Other,
            ),
            ("https://cdn.example/hls/master.m3u8", Xhr),
            ("https://cdn.example/media/movie.mp4", Media),
            ("https://cdn.example/media/movie.mp4", Other),
        ] {
            let f = facts(url, page, rt);
            assert_eq!(
                decide(&f, lite_only, Some(&e)),
                Verdict::Allow,
                "{url} {rt:?}"
            );
        }
        // Thumbnails/posters/fonts on the same page: still dead.
        for rt in [Image, Font] {
            let f = facts("https://i.ytimg.com/vi/abc/hqdefault.jpg", page, rt);
            assert_eq!(
                decide(&f, lite_only, Some(&e)),
                Verdict::LiteBlock(rt),
                "{rt:?}"
            );
        }
        // The innertube player response still passes (the
        // Shield prune path owns it; Lite never touches it).
        let f = facts(
            "https://www.youtube.com/youtubei/v1/player?key=abc",
            page,
            Xhr,
        );
        assert_eq!(decide(&f, lite_only, Some(&e)), Verdict::Allow);
    }

    #[test]
    fn matrix_lite_chosen_tab_exemption_is_media_only() {
        // A tab that designated a chosen video on a NON-watch
        // page (the tapped embedded player): its media and
        // stream shapes pass, its images/fonts still die, and
        // without the designation the same requests are
        // Lite-blocked (default-deny preserved).
        let e = engine();
        let page = "https://news.example/story";
        let lite_only = FilterState {
            shield_on: false,
            lite_on: true,
        };
        let mut chosen = facts("https://cdn.example/media/movie.mp4", page, Media);
        chosen.tab_chosen = true;
        assert_eq!(decide(&chosen, lite_only, Some(&e)), Verdict::Allow);
        let mut chosen_stream = facts("https://cdn.example/hls/master.m3u8", page, Xhr);
        chosen_stream.tab_chosen = true;
        assert_eq!(decide(&chosen_stream, lite_only, Some(&e)), Verdict::Allow);
        let mut chosen_img = facts("https://cdn.example/pic.png", page, Image);
        chosen_img.tab_chosen = true;
        assert_eq!(
            decide(&chosen_img, lite_only, Some(&e)),
            Verdict::LiteBlock(Image),
            "the chosen exemption never covers images"
        );
        let unchosen = facts("https://cdn.example/media/movie.mp4", page, Media);
        assert_eq!(
            decide(&unchosen, lite_only, Some(&e)),
            Verdict::LiteBlock(Media)
        );
        // Watch-shape is read from the path only: a search
        // page whose QUERY mentions /watch is not a watch page.
        let f = facts(
            "https://cdn.example/hls/master.m3u8",
            "https://site.example/results?next=/watch",
            Xhr,
        );
        assert_eq!(decide(&f, lite_only, Some(&e)), Verdict::LiteBlock(Xhr));
        // Shield still wins inside a chosen context: a
        // rule-hit stream is the Shield's, not Lite's pass.
        let mut ad = facts(
            "https://ads.example.com/banner-ad.mp4",
            "https://www.youtube.com/watch?v=fixture1",
            Media,
        );
        ad.tab_chosen = true;
        assert_eq!(
            decide(&ad, ON_ON, Some(&e)),
            Verdict::ShieldBlock(Media),
            "generic /banner-ad. rule must still hit before Lite's exemption"
        );
    }

    #[test]
    fn matrix_method_gate_head_and_post_law() {
        // A2-M2: the production method law, pinned where it
        // runs. HEAD stands with GET; POST is never the
        // Shield's — not even for a $method=post rule (those
        // rules compile into the engine but the pipeline's
        // gate is the law the browser actually lives by).
        let e = ShieldEngine::compile(&["||ads.example.com^", "||post.example^$method=post"]);
        let page = "https://news.example/";
        let mut f = facts("https://ads.example.com/x.js", page, Script);
        f.method = Some("HEAD".to_string());
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::ShieldBlock(Script));
        let mut f = facts("https://post.example/x", page, Script);
        f.method = Some("POST".to_string());
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow);
        // The engine itself still honors $method= (the seam the
        // pipeline gates): with the method threaded, POST hits.
        assert!(e
            .decide_with_method("https://post.example/x", page, Script, Some("POST"))
            .is_block());
    }
    #[test]
    fn matrix_origin_dimension() {
        let e = engine();
        // A local PAGE is fully immune: even a rule-hit ad image
        // on a loopback page is allowed, with both filters on.
        // (Through v25 the two layers spelled "local" two ways
        // and a 127.0.0.2 page fell between them.)
        for page in [
            "http://127.0.0.2:8080/app",
            "http://localhost:5173/",
            "http://dev.localhost/",
            "http://[::1]:3000/",
        ] {
            let f = facts("https://ads.example.com/banner.png", page, Image);
            assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow, "{page}");
        }
        // A local REQUEST is immune even on a public page (a
        // dev page's own assets, a LAN image CDN).
        let f = facts(
            "http://127.0.0.1:9000/pic.png",
            "https://news.example/",
            Image,
        );
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow);
        // Internal pages never reach the engine at all.
        let f = facts("https://ads.example.com/banner.png", "pico://start", Image);
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::Allow);
        // Shield on, Lite off: a non-rule image passes (Lite is
        // the only image layer besides rules).
        let f = facts(
            "https://cdn.example/pic.png",
            "https://news.example/",
            Image,
        );
        assert_eq!(
            decide(
                &f,
                FilterState {
                    shield_on: true,
                    lite_on: false
                },
                Some(&e)
            ),
            Verdict::Allow
        );
        // A POST is never the Shield's (only GET is ever
        // blocked, v22 law) — the Shield step composes its
        // method gate inside, so the request falls through to
        // Lite, which has no method dimension.
        let mut f = facts(
            "https://ads.example.com/banner.png",
            "https://news.example/",
            Image,
        );
        f.method = Some("POST".to_string());
        assert_eq!(decide(&f, ON_ON, Some(&e)), Verdict::LiteBlock(Image));
    }
}
