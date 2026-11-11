//! Dump the FINAL assembled chrome and start pages (the exact bytes
//! the runtime serves) for the jsdom DOM test in scripts/dom_test.mjs
//! and for manual inspection.
//!
//! Usage: cargo run --example dump_pages -- <out-dir>
//! Writes <out-dir>/chrome.html and <out-dir>/start.html. The start
//! page is rendered with a small representative init object — one
//! service, two recent files (AUDIT V17-1: the DOM test clicks real
//! rendered items and asserts the exact IPC request each click
//! emits, so the fixture must contain clickable items).

use std::path::PathBuf;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).expect("create out dir");
    let chrome = pico_core::core::assets::assemble_chrome_html();
    // 0.35.1 fixture theme: NON-default ember/dark (the default
    // is forest/light), so the DOM suite proves boot-applied INIT
    // theming instead of passing on the pages' static defaults.
    // INIT.downloads carries the 0.35.1 card contract model
    // (state/count/speed_bps/paused/queued/newest_name/unseen),
    // active with the humanizer-pinned speed 1258291 ("1.26 MB").
    let start = pico_core::core::assets::assemble_start_html(
        "{\"services\":[{\"port\":5173,\"pid\":4242,\"process\":\"node.exe\",\"title\":\"Fixture App\",\"label\":\"Fixture App\",\"url\":\"http://localhost:5173\",\"kind\":\"http\"}],\"otherListeners\":0,\"recents\":[\"C:\\\\Users\\\\Fixture\\\\Downloads\\\\First Page.html\",\"C:\\\\Users\\\\Fixture\\\\Desktop\\\\Second.html\"],\"recentsHidden\":false,\"theme\":\"dark\",\"palette\":\"ember\",\"downloads\":{\"state\":\"active\",\"count\":2,\"speed_bps\":1258291,\"paused\":0,\"queued\":0,\"newest_name\":null,\"unseen\":false},\"stats\":{\"shield\":{\"blocked\":1234,\"estBytes\":86400000,\"estDurationMs\":864000,\"prunedEvents\":34,\"prunedBytes\":512000,\"sessionBlocked\":12,\"sessionEstBytes\":640000,\"sessionEstDurationMs\":9600},\"lite\":{\"blocked\":56,\"estBytes\":22400000,\"estDurationMs\":140000,\"prunedEvents\":0,\"prunedBytes\":0,\"sessionBlocked\":3,\"sessionEstBytes\":1200000,\"sessionEstDurationMs\":7500}}}",
    );
    std::fs::write(dir.join("chrome.html"), &chrome).expect("write chrome");
    std::fs::write(dir.join("start.html"), &start).expect("write start");
    println!(
        "chrome.html {} bytes; start.html {} bytes",
        chrome.len(),
        start.len()
    );

    // The Downloads page (0.34): THE snapshot shape the
    // runtime pushes (page init JSON == __picoSetDownloads
    // payload) — one task per status, the failed one carrying
    // its hint + retry_text, the done one its SHA-256, plus
    // active_count / failed_unseen and a live notice, so the
    // DOM test can assert every per-status action set and
    // the visual check renders the full vocabulary.
    let downloads = pico_core::core::assets::assemble_downloads_html(
        "{\"theme\":\"dark\",\"palette\":\"ember\",\"version\":\"0.35\",\"tasks\":[{\"id\":11,\"name\":\"ubuntu-24.04.iso\",\"host\":\"releases.ubuntu.com\",\"url\":\"https://releases.ubuntu.com/ubuntu-24.04.iso\",\"status\":\"downloading\",\"total\":1500000000,\"downloaded\":637500000,\"percent\":42.5,\"size_text\":\"1.4 GB\",\"detail_text\":\"612 MB of 1.4 GB · 3.2 MB/s · 4 min left\",\"connections\":8,\"error\":null,\"hint\":null,\"retry_text\":null,\"retry_pending\":false,\"refreshing\":false,\"sha256\":null,\"added_ms\":1730000000000},{\"id\":12,\"name\":\"lecture.mp4\",\"host\":\"media.example.org\",\"url\":\"https://media.example.org/lecture.mp4\",\"status\":\"paused\",\"total\":251658240,\"downloaded\":50331648,\"percent\":20,\"size_text\":\"240 MB\",\"detail_text\":\"48 MB of 240 MB · Paused\",\"connections\":8,\"error\":null,\"hint\":null,\"retry_text\":null,\"retry_pending\":false,\"refreshing\":false,\"sha256\":null,\"added_ms\":1730003600000},{\"id\":13,\"name\":\"file.rar\",\"host\":\"dl.soft98.ir\",\"url\":\"https://dl.soft98.ir/file.rar?token=abc123\",\"status\":\"failed\",\"total\":89128960,\"downloaded\":26738688,\"percent\":30,\"size_text\":\"85 MB\",\"detail_text\":\"25 MB of 85 MB · Failed\",\"connections\":8,\"error\":\"HTTP 403\",\"hint\":\"the link may have expired — use Refresh link\",\"retry_text\":\"Retrying in 15 s · attempt 1 of 3\",\"retry_pending\":true,\"refreshing\":false,\"sha256\":null,\"added_ms\":1730007200000},{\"id\":14,\"name\":\"manual.pdf\",\"host\":\"docs.example.com\",\"url\":\"https://docs.example.com/manual.pdf\",\"status\":\"done\",\"total\":12582912,\"downloaded\":12582912,\"percent\":100,\"size_text\":\"12.0 MB\",\"detail_text\":\"12.0 MB\",\"connections\":8,\"error\":null,\"hint\":null,\"retry_text\":null,\"retry_pending\":false,\"refreshing\":false,\"sha256\":\"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08\",\"added_ms\":1730010800000},{\"id\":15,\"name\":\"notes.zip\",\"host\":\"files.example.com\",\"url\":\"https://files.example.com/notes.zip\",\"status\":\"queued\",\"total\":5242880,\"downloaded\":0,\"percent\":0,\"size_text\":\"5.0 MB\",\"detail_text\":\"Queued\",\"connections\":8,\"error\":null,\"hint\":null,\"retry_text\":null,\"retry_pending\":false,\"refreshing\":false,\"sha256\":null,\"added_ms\":1730014400000}],\"active_count\":2,\"failed_unseen\":1,\"settings\":{\"default_connections\":8,\"max_concurrent\":3,\"speed_limit_kbps\":0},\"notice\":{\"seq\":7,\"kind\":\"failed\",\"name\":\"file.rar\",\"reason\":\"HTTP 403\",\"open\":true}}",
    );
    std::fs::write(dir.join("downloads.html"), &downloads).expect("write downloads");
    println!("downloads.html {} bytes", downloads.len());

    // The Shield fixture (AUDIT V23): a minimal page whose head
    // installs page-environment stubs FIRST (the "native" fetch /
    // XHR / Response the payload will find) and then the REAL
    // document script built from the synthetic corpus in
    // tests/fixtures (public build: no real lists ship; the
    // corpus's video-site rules are hand-written for this
    // suite and exercise the same payload paths). The DOM test
    // loads it under several URLs (youtube host, other host,
    // localhost) and drives fetch/XHR/JSON.parse/initial-data
    // through the payload exactly as a page would.
    let lists = synthetic_lists();
    let refs: Vec<&str> = lists.iter().map(|s| s.as_str()).collect();
    let engine = pico_core::core::shield::ShieldEngine::compile(&refs);
    // AUDIT V26: budgets from core::shield — one definition
    // for the app, this dump and the tests, so the DOM proofs
    // run against the shipped payload.
    let (doc_script, info) = engine.build_document_script(
        pico_core::core::shield::COSMETIC_SCRIPT_BUDGET,
        pico_core::core::shield::SCRIPTLET_SCRIPT_BUDGET,
    );
    let fixture = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n<script>\n{STUB}\n</script>\n<script>\n{doc_script}\n</script>\n</head><body><div class=\"ad-banner\">fixture ad</div></body></html>\n"
    );
    std::fs::write(dir.join("shield-fixture.html"), &fixture).expect("write shield fixture");
    println!(
        "shield-fixture.html {} bytes (document script {} bytes, {} scriptlet entries)",
        fixture.len(),
        info.bytes,
        info.scriptlet_entries
    );

    // The Lite gate fixture (AUDIT V28, semantics v2 in V29):
    // a media stub installed FIRST (the "native" play/pause the
    // gate will wrap, with a call counter, plus the ipc
    // recorder the chosen report lands in), then the REAL gate
    // runtime from core::filterpipe. The DOM test loads it on a
    // public URL (gate installs), on watch-shaped URLs (the
    // chosen main video must LIVE), drives gesture / no-gesture
    // / route-change play() calls, and on a loopback URL (gate
    // must not install). The raw runtime is also written out so
    // the test can re-evaluate it (double-install proof) and
    // layer it over the Shield fixture (orthogonality proof).
    // #sidebar is the watch-page sidebar preview stand-in.
    let gate = pico_core::core::filterpipe::LITE_GATE_RUNTIME;
    let lite_fixture = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n<script>\n{MEDIA_STUB}\n</script>\n<script>\n{gate}\n</script>\n</head><body><video id=\"v1\"></video><video id=\"v2\"></video><div id=\"wrap\"><video id=\"v3\"></video></div><aside><video id=\"sidebar\"></video></aside></body></html>\n"
    );
    std::fs::write(dir.join("lite-fixture.html"), &lite_fixture).expect("write lite fixture");
    std::fs::write(dir.join("lite-gate.js"), gate).expect("write lite gate");
    println!("lite-fixture.html {} bytes", lite_fixture.len());

    // The cosmetic/scriptlet LAB fixture (AUDIT V28): a tiny
    // hand-written list compiled by the REAL engine, pinning the
    // behaviors the shipped lists exercise only implicitly —
    // a generic cosmetic + a scoped exception against it
    // (A2-M4), an entity-scoped cosmetic (A2-M3), a `set`
    // scriptlet whose pin must yield while the Shield is off
    // (A2-H1), and a scoped generichide flag.
    let lab_list = "\
##.ad-generic
example.com#@#.ad-generic
example.*##.ent-ad
example.com##+js(set, picoTest.pin, 123)
example.com##+js(json-prune, adBanner)
gh.example#$#generichide
";
    let lab_engine = pico_core::core::shield::ShieldEngine::compile(&[lab_list]);
    let (lab_script, _lab_info) = lab_engine.build_document_script(
        pico_core::core::shield::COSMETIC_SCRIPT_BUDGET,
        pico_core::core::shield::SCRIPTLET_SCRIPT_BUDGET,
    );
    let lab_fixture = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n<script>\n{STUB}\nwindow.picoTest = {{}};\n</script>\n<script>\n{lab_script}\n</script>\n</head><body><div class=\"ad-generic\">generic</div><div class=\"ent-ad\">entity</div></body></html>\n"
    );
    std::fs::write(dir.join("lab-fixture.html"), &lab_fixture).expect("write lab fixture");
    println!("lab-fixture.html {} bytes", lab_fixture.len());
}

/// The four synthetic fixture lists (tests/fixtures), in
/// canonical-source order — the same corpus the Rust suites
/// compile, so the DOM proofs run against an identical engine.
fn synthetic_lists() -> Vec<String> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    [
        "easylist-synthetic.txt",
        "easyprivacy-synthetic.txt",
        "ublock-filters-synthetic.txt",
        "ublock-privacy-synthetic.txt",
    ]
    .iter()
    .map(|n| std::fs::read_to_string(base.join(n)).expect("synthetic fixture list"))
    .collect()
}

/// The media stub installed before the Lite gate in its fixture:
/// a counting play() (the "native" implementation the gate
/// wraps) and a silent pause(). `window.__origPlay` keeps the
/// exact function identity for the restoration proof.
const MEDIA_STUB: &str = r#"
window.__playCalls = 0;
window.__ipcSent = [];
window.ipc = { postMessage: function (s) { window.__ipcSent.push(String(s)); } };
window.__origPlay = function () {
  window.__playCalls++;
  this.__picoPlaying = true;
  return Promise.resolve();
};
window.HTMLMediaElement.prototype.play = window.__origPlay;
window.HTMLMediaElement.prototype.pause = function () { this.__picoPlaying = false; };
"#;

/// The page-environment stub installed before the payload in
/// the Shield fixture. Everything the payload might wrap is
/// defined here so the DOM test can assert identity (untouched
/// on loopback / foreign hosts) and drive canned responses.
const STUB: &str = r#"
window.__ipcSent = [];
window.ipc = { postMessage: function (s) { window.__ipcSent.push(String(s)); } };
window.__fetchBody = "";
window.__xhrBody = "";
function FakeResponse(body, init) {
  this._body = body === undefined || body === null ? "" : String(body);
  init = init || {};
  this.status = init.status || 200;
  this.statusText = init.statusText || "";
  this.headers = init.headers || {};
  this.ok = this.status >= 200 && this.status < 300;
  this.url = "";
}
FakeResponse.prototype.clone = function () {
  return new FakeResponse(this._body, { status: this.status, statusText: this.statusText, headers: this.headers });
};
FakeResponse.prototype.text = function () { return Promise.resolve(this._body); };
FakeResponse.prototype.json = function () { return Promise.resolve(JSON.parse(this._body)); };
window.Response = FakeResponse;
window.__nativeFetch = function (input, init) {
  return Promise.resolve(new FakeResponse(window.__fetchBody));
};
window.fetch = window.__nativeFetch;
window.__nativeParse = JSON.parse;
function FakeXHR() {
  this.readyState = 0;
  this.status = 0;
  this.statusText = "";
  this.responseText = "";
  this.response = "";
  this.responseType = "";
  this._listeners = {};
}
FakeXHR.prototype.open = function (method, url) { this.__method = method; this.__url = url; this.readyState = 1; };
FakeXHR.prototype.addEventListener = function (type, fn) {
  (this._listeners[type] = this._listeners[type] || []).push(fn);
};
FakeXHR.prototype.removeEventListener = function () {};
FakeXHR.prototype.dispatchEvent = function (ev) {
  var list = this._listeners[ev.type] || [];
  for (var i = 0; i < list.length; i++) list[i].call(this, ev);
  var h = this["on" + ev.type];
  if (typeof h === "function") h.call(this, ev);
  return true;
};
FakeXHR.prototype.send = function () {
  var self = this;
  setTimeout(function () {
    // The stub's "network stack" writes the backing slots
    // directly — what a native stack does behind the WebIDL
    // getters (and the only way past a getter-only shadow).
    self._responseText = window.__xhrBody;
    self._response = self.responseType === "json" ? JSON.parse(window.__xhrBody) : window.__xhrBody;
    self.readyState = 4;
    self.status = 200;
    self.statusText = "OK";
    self.dispatchEvent({ type: "readystatechange" });
    self.dispatchEvent({ type: "load" });
    self.dispatchEvent({ type: "loadend" });
  }, 0);
};
window.__nativeXhrOpen = FakeXHR.prototype.open;
window.__nativeXhrSend = FakeXHR.prototype.send;
window.XMLHttpRequest = FakeXHR;
// AUDIT V28: responseText/response are ACCESSORS on the
// prototype, as WebIDL defines them on a real browser — the
// payload's getter-first XHR transform (A2-M6) can only shadow
// prototype accessors, and with plain data properties the
// fixture silently exercised only the fallback event hook,
// which is exactly the ordering hole the fix closed.
Object.defineProperty(FakeXHR.prototype, "responseText", {
  configurable: true,
  get: function () { return this._responseText || ""; },
  set: function (v) { this._responseText = v; },
});
Object.defineProperty(FakeXHR.prototype, "response", {
  configurable: true,
  get: function () { return this._response === undefined ? "" : this._response; },
  set: function (v) { this._response = v; },
});
"#;
