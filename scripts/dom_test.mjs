// Pico v27 DOM test (AUDIT V14-1; v15 branding; v16 merged
// header; v17 action transport; v18 media env; v19 alignment;
// v20 neumorphic material): load the FINAL assembled chrome
// and start pages in jsdom, drive them the way the Rust shell
// does, and assert the rendering contract — including that a
// poisoned tab title stays inert text, that the shell
// self-diagnosis strip appears when no snapshot ever arrives,
// that the branding controls emit only enum-named commands, that
// the header is the v16 merged single row (brand block before
// the tabs, window controls in the same row, no separate
// titlebar row, feather strip present), and — the v17 seam —
// that clicking each start-page control emits the EXACT IPC
// action request the Rust dispatch consumes (AUDIT V17-1: the
// v16 test asserted only the anchors' href strings, which is
// how a dead click path shipped unnoticed).
//
// Run:
//   cargo run --locked --no-default-features --example dump_pages -- /tmp/pico-pages
//   mkdir -p /tmp/picodom && cd /tmp/picodom && npm install jsdom
//   JSDOM_DIR=/tmp/picodom node scripts/dom_test.mjs /tmp/pico-pages
//
// (JSDOM_DIR points at a directory containing node_modules/jsdom;
// a plain `import "jsdom"` is tried first.)
// Exits non-zero when any check fails.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

let JSDOM;
try {
  ({ JSDOM } = await import("jsdom"));
} catch {
  const base = pathToFileURL(`${process.env.JSDOM_DIR || "."}/loader.js`);
  ({ JSDOM } = createRequire(base)("jsdom"));
}

const dir = process.argv[2];
if (!dir) {
  console.error("usage: node scripts/dom_test.mjs <pages-dir>");
  process.exit(2);
}
const chromeHtml = readFileSync(`${dir}/chrome.html`, "utf8");
const startHtml = readFileSync(`${dir}/start.html`, "utf8");
const downloadsHtml = readFileSync(`${dir}/downloads.html`, "utf8");

let failures = 0;
function check(name, cond, extra = "") {
  if (cond) {
    console.log(`ok   ${name}`);
  } else {
    failures++;
    console.log(`FAIL ${name} ${extra}`);
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ------------------------------------------------------------------
// Chrome: healthy path — a canned snapshot renders the shell.
// ------------------------------------------------------------------
{
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
  });
  const { window } = dom;
  const doc = window.document;

  // Static skeleton exists before any Rust message.
  check("skeleton: topbar present", !!doc.getElementById("topbar"));
  check("skeleton: NO separate titlebar row",
    doc.getElementById("titlebar") === null);
  check("skeleton: addr input present", !!doc.getElementById("addr"));
  check("skeleton: win controls present",
    ["win-min", "win-max", "win-close"].every((id) => !!doc.getElementById(id)));
  check("skeleton: win controls live in the top row",
    ["win-min", "win-max", "win-close"].every(
      (id) => !!doc.getElementById(id).closest("#topbar")));
  check("skeleton: feather strip present", !!doc.getElementById("feather"));
  check("skeleton: NO savings band in the chrome (0.30: start page only)",
    doc.getElementById("stat-shield") === null &&
    doc.getElementById("stats-grid") === null);
  check("skeleton: version badge is 0.40.1",
    doc.getElementById("ver").textContent === "0.40.1",
    doc.getElementById("ver").textContent);
  check("skeleton: brand byline present",
    doc.getElementById("brand-by").textContent.includes("@iK4w4"),
    doc.getElementById("brand-by").textContent);

  // A3-F7: the full 16-palette catalogue, each with its OWN
  // six slots — the same entries the THEMES16 fixture below
  // uses (the v27 fixture listed 9 palettes all wearing
  // Forest's slots, so dot colors were untestable).
  const THEMES = [
    ["forest", "Forest", ["#0F1F17", "#1E3A2E", "#3E5A3F", "#6B7F67", "#C7CBB8", "#3D4337"]],
    ["ember", "Ember Navy", ["#0B132B", "#1C2541", "#3A506B", "#5BC0DE", "#F4D6B0", "#D97706"]],
    ["sage", "Sage Gold", ["#0B0F0D", "#1F2A1F", "#3E5A3F", "#8CA98C", "#D9C7A3", "#B08D57"]],
    ["lagoon", "Lagoon", ["#0A0F14", "#0F3D3E", "#2E6F73", "#7FB3B8", "#EADCC8", "#C6601F"]],
    ["ultraviolet", "Ultraviolet", ["#1A0F2E", "#3B1B4F", "#7A2E6F", "#D6598F", "#F4D7D4", "#FF6B6B"]],
    ["indigo", "Indigo Amber", ["#1A1A1F", "#2D2B55", "#6A5ACD", "#A678B9", "#F0D9B5", "#F4A62A"]],
    ["graphite", "Graphite Pink", ["#121212", "#1F2937", "#334155", "#64748B", "#F1F5F9", "#EC4899"]],
    ["violet", "Violet", ["#0E0B1D", "#2A1E4D", "#5A3B8C", "#8B7CC7", "#C9C2E3", "#3A3A52"]],
    ["bronze", "Bronze", ["#1A120E", "#3A2418", "#6B4A2E", "#A88462", "#D5C4AE", "#4A4A46"]],
    ["teal", "Deep Teal", ["#0B3D3B", "#0E6860", "#14B8A6", "#99F6E4", "#E6FFFB", "#C0FBF0"]],
    ["mist", "Mist Slate", ["#191D23", "#2F3A41", "#57707A", "#7B919C", "#989DAA", "#C5BAC4"]],
    ["lavender", "Lavender", ["#0D0B2D", "#282177", "#5B4BFF", "#A78BFA", "#E6E0FF", "#C7B6FD"]],
    ["royal", "Royal Blue", ["#0B1F3B", "#143779", "#2563EB", "#60A5FA", "#DBEAFE", "#9EC8FC"]],
    ["tangerine", "Tangerine", ["#1F1D2B", "#6D3E1C", "#FF7A00", "#FFA94D", "#FFE4C7", "#FFC78A"]],
    ["amethyst", "Amethyst", ["#5B21B6", "#6C36CC", "#8B5CF6", "#C7B3FF", "#EDE9FE", "#DACEFF"]],
    ["rose", "Rose", ["#6D0D3F", "#99225F", "#EC4899", "#F9A8D4", "#FCE7F3", "#FBC8E4"]],
  ].map(([id, name, slots]) => ({ id, name, slots }));

  const poison = '<img src=x onerror="window.__pwned=1">';
  // The page focuses the address bar at load, and the renderer
  // (correctly) never overwrites it while focused — blur first,
  // like a user who is not typing.
  doc.activeElement.blur();
  window.__pico({
    tabs: [
      { id: 1, title: "Docs", url: "http://localhost:5173/", loading: false, start: false, failed: false },
      { id: 2, title: poison, url: "http://localhost:3000/", loading: true, start: false, failed: false },
    ],
    active: 2,
    theme: "dark",
    address: "http://localhost:3000/",
    canBack: true,
    canForward: false,
    maximized: false,
    services: 3,
    otherListeners: 5,
    downloads: [], downloadsOpen: false, downloadsActive: 0, downloadsDefaultConn: 8,
    activeError: "",
    blockedTotal: 7,
    blockedTab: 3,
    shieldOn: true,
    palette: "violet",
    themes: THEMES,
    paletteOpen: false,
    liteOn: false,
    focusAddr: false,
    version: "0.40.1",
  });

  const tabs = doc.querySelectorAll("#tabstrip .tab");
  check("snapshot: two tabs rendered", tabs.length === 2, `got ${tabs.length}`);
  // v16 merged header: the brand block sits in the top row BEFORE
  // the first tab in DOM order, and the feather strip carries a
  // gradient background.
  const brand = doc.querySelector("#topbar .brand");
  check("header: brand block precedes the first tab in DOM order",
    !!brand && !!tabs[0] &&
    (brand.compareDocumentPosition(tabs[0]) & window.Node.DOCUMENT_POSITION_FOLLOWING) !== 0);
  check("header: tabs render inside the top row",
    !!tabs[0] && !!tabs[0].closest("#topbar"));
  // jsdom's cssstyle engine cannot resolve var() inside the
  // multi-layer `background` shorthand and reports "none" for the
  // computed value even though the rule is present and valid (real
  // Chromium applies it). Accept either the computed gradient, or
  // the #feather rule in the parsed stylesheet carrying it — the
  // Rust asset test separately pins the exact gradient bytes.
  let featherBg = window.getComputedStyle(doc.getElementById("feather")).backgroundImage;
  if (!(typeof featherBg === "string" && featherBg.includes("gradient"))) {
    for (const sheet of doc.styleSheets) {
      for (const rule of sheet.cssRules || []) {
        if (rule.selectorText === "#feather" && rule.cssText.includes("linear-gradient")) {
          featherBg = rule.cssText;
        }
      }
    }
  }
  check("header: feather strip has a gradient background",
    typeof featherBg === "string" && featherBg.includes("gradient"),
    String(featherBg).slice(0, 80));
  const shellCs = window.getComputedStyle(doc.getElementById("shell"));
  check("header: shell is the full 86 DIP chrome slot",
    shellCs.height === "86px", shellCs.height);
  check("header: feather strip is 12 DIP tall",
    window.getComputedStyle(doc.getElementById("feather")).height === "12px",
    window.getComputedStyle(doc.getElementById("feather")).height);
  const warnCs = window.getComputedStyle(doc.getElementById("shellwarn"));
  check("header: shellwarn overlays the top of the chrome",
    warnCs.position === "fixed" && (warnCs.top === "0px" || warnCs.top === "0"),
    `${warnCs.position}/${warnCs.top}`);
  // v19 alignment system (AUDIT D1–D4): one 12 DIP page inset
  // starts both rows, caption buttons share the 38 DIP width,
  // and the neon focus hoop is gone from the shipped bytes —
  // the address wrap pre-allocates a 1px transparent border and
  // focuses into a thin border + --focus-glow halo instead.
  const topCs = window.getComputedStyle(doc.getElementById("topbar"));
  const navCs = window.getComputedStyle(doc.getElementById("navbar"));
  check("v19: top row starts on the 12 DIP page inset",
    topCs.paddingLeft === "12px", topCs.paddingLeft);
  check("v19: address row starts on the same 12 DIP inset",
    navCs.paddingLeft === "12px", navCs.paddingLeft);
  check("v19: caption buttons are uniformly 38 DIP wide",
    window.getComputedStyle(doc.getElementById("win-close")).width === "38px",
    window.getComputedStyle(doc.getElementById("win-close")).width);
  check("v19: nav buttons are square 30 DIP icon buttons",
    window.getComputedStyle(doc.getElementById("back")).width === "30px" &&
    window.getComputedStyle(doc.getElementById("back")).height === "30px",
    window.getComputedStyle(doc.getElementById("back")).width);
  check("v19: neon focus hoop is gone from the shipped page",
    !chromeHtml.includes("outline: 2px solid var(--accent)"));
  // v20 neumorphic material (AUDIT v20): no border draws the
  // address well, focus is a shadow-only glow, the header is one
  // slab of var(--surface), and the active tab's shadow is
  // clipped at its bottom edge so it grows out of the toolbar.
  check("v20: the v19 pre-allocated focus border is gone",
    !chromeHtml.includes("border: 1px solid transparent"));
  check("v20: address focus layers the --neu-focus glow over the inset well",
    chromeHtml.includes("box-shadow: var(--neu-inset), var(--neu-focus)"));
  check("v20: neu shadow vocabulary is defined in the shipped page",
    chromeHtml.includes("--neu-raised:") && chromeHtml.includes("--neu-inset:") &&
    chromeHtml.includes("--neu-pressed:"));
  check("v20: tab shadow is clipped at the tab's bottom edge (attachment)",
    chromeHtml.includes("clip-path: inset(-18px -18px 0 -18px)"));
  check("v20: active tab carries the concave fillet gradients",
    chromeHtml.includes("radial-gradient(circle at 0 0") &&
    chromeHtml.includes("radial-gradient(circle at 100% 0"));
  check("snapshot: address bar shows active URL",
    doc.getElementById("addr").value === "http://localhost:3000/",
    doc.getElementById("addr").value);
  check("snapshot: theme applied",
    doc.documentElement.getAttribute("data-theme") === "dark");
  check("snapshot: version badge stays 0.32.1 (display version)",
    doc.getElementById("ver").textContent === "0.40.1",
    doc.getElementById("ver").textContent);
  check("snapshot: services badge shows count",
    doc.getElementById("svc-badge").textContent.includes("3"),
    doc.getElementById("svc-badge").textContent);
  // v22 Shield: the toggle reflects the snapshot's shieldOn and
  // the chip carries the session total (+ per-tab in its title).
  const shieldBtn = doc.getElementById("shield");
  check("shield: toggle present in the address row",
    !!shieldBtn && !!shieldBtn.closest("#navbar"));
  check("shield: aria-pressed reflects shieldOn=true",
    shieldBtn.getAttribute("aria-pressed") === "true",
    shieldBtn.getAttribute("aria-pressed"));
  const shieldBadge = doc.getElementById("shield-badge");
  check("shield: chip shows the session blocked total",
    shieldBadge.textContent === "7 blocked", shieldBadge.textContent);
  check("shield: chip title carries the per-tab count",
    shieldBadge.title.includes("3 blocked in this tab"), shieldBadge.title);
  window.__pico({
    tabs: [], active: 0, theme: "dark", address: "", canBack: false,
    canForward: false, maximized: false, services: 0, otherListeners: 0,
    downloads: [], downloadsOpen: false, downloadsActive: 0, downloadsDefaultConn: 8, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: false, version: "0.40.1",
  });
  check("shield: aria-pressed follows shieldOn=false",
    shieldBtn.getAttribute("aria-pressed") === "false",
    shieldBtn.getAttribute("aria-pressed"));
  check("shield: chip hides when nothing is blocked",
    shieldBadge.textContent === "", JSON.stringify(shieldBadge.textContent));
  check("poison: title stayed inert text",
    tabs[1].textContent.includes(poison) && !window.__pwned,
    `tab text: ${tabs[1].textContent}`);
  check("poison: no <img> element was created",
    doc.querySelectorAll("#tabstrip img").length === 0);
  check("health: snapshot flag set", window.__picoGotSnapshot === true);
  await sleep(2200);
  check("health: warn strip stays hidden after a snapshot",
    !doc.getElementById("shellwarn").classList.contains("on"));
  window.close();
}

// ------------------------------------------------------------------
// Chrome branding (v15): clicking the titlebar byline emits ONLY
// the enum social command — no URL ever crosses IPC.
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  const byBtn = doc.getElementById("brand-by");
  check("branding: byline is a button with the Telegram tooltip",
    !!byBtn && byBtn.title === "Telegram — @iK4w4",
    byBtn ? byBtn.title : "missing");
  byBtn.click();
  const social = sent.filter((m) => m.t === "social");
  check("branding: byline click emits exactly one social command",
    social.length === 1, `got ${social.length}`);
  check("branding: social command carries the telegram enum only",
    social.length === 1 && social[0].site === "telegram" &&
    !JSON.stringify(social[0]).includes("http"),
    JSON.stringify(social[0] || null));
  // v22 Shield: the toggle click emits exactly {t:"shield_toggle"}.
  doc.getElementById("shield").click();
  const shieldCmds = sent.filter((m) => m.t === "shield_toggle");
  check("shield: click emits exactly one shield_toggle command",
    shieldCmds.length === 1, `got ${shieldCmds.length}`);
  // v16 window drag: the merged top row drags from its non-control
  // areas (empty tab strip, brand text) and never from a control.
  const strip = doc.getElementById("tabstrip");
  const pd = (target) => target.dispatchEvent(
    new window.MouseEvent("pointerdown", { button: 0, bubbles: true }));
  pd(strip);
  check("drag: pointerdown on empty strip emits win_drag",
    sent.filter((m) => m.t === "win_drag").length === 1,
    JSON.stringify(sent.map((m) => m.t)));
  pd(byBtn);
  check("drag: pointerdown on a control emits no win_drag",
    sent.filter((m) => m.t === "win_drag").length === 1);
  strip.dispatchEvent(new window.MouseEvent("dblclick", { bubbles: true }));
  check("drag: dblclick on empty strip emits win_max",
    sent.filter((m) => m.t === "win_max").length === 1);
  window.close();
}

// ------------------------------------------------------------------
// Chrome v25/v26: Lite toggle + Teleport panel. The toggle emits
// exactly {t:"lite_toggle"}; the panel renders the snapshot's
// teleport payload (SVG module count == dark cells in the
// matrix), and every dismissal path emits {t:"teleport_close"}.
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  const baseSnap = {
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 1,
    otherListeners: 0, downloads: [], downloadsOpen: false, downloadsActive: 0, downloadsDefaultConn: 8, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, teleport: null,
  };
  window.__pico({ ...baseSnap });
  // Lite toggle.
  const liteBtn = doc.getElementById("lite");
  check("v25 lite: toggle present beside the shield",
    !!liteBtn && !!liteBtn.closest("#navbar") && !!doc.getElementById("shield"));
  check("v29 lite: tooltip carries the chosen-video law",
    liteBtn.title.includes("The video you choose to watch still plays"), liteBtn.title);
  liteBtn.click();
  check("v25 lite: click emits exactly one lite_toggle command",
    sent.filter((m) => m.t === "lite_toggle").length === 1,
    JSON.stringify(sent.map((m) => m.t)));
  window.__pico({ ...baseSnap, liteOn: true, liteBlocked: 5 });
  check("v25 lite: aria-pressed follows liteOn=true",
    liteBtn.getAttribute("aria-pressed") === "true");
  const liteBadge = doc.getElementById("lite-badge");
  check("v25 lite: chip shows the session lite count",
    liteBadge.textContent === "5 lite-blocked", liteBadge.textContent);
  window.__pico({ ...baseSnap, liteOn: false, liteBlocked: 5 });
  check("v25 lite: chip hides while Lite is off",
    liteBadge.textContent === "", liteBadge.textContent);
  // Teleport: click asks Rust; the answering snapshot opens the panel.
  const tpBtn = doc.getElementById("teleport");
  const panel = doc.getElementById("tpanel");
  check("v26 teleport: button present in the address row",
    !!tpBtn && !!tpBtn.closest("#navbar"));
  check("v26 teleport: panel starts closed", !panel.classList.contains("on"));
  tpBtn.click();
  check("v26 teleport: click emits exactly one teleport command",
    sent.filter((m) => m.t === "teleport").length === 1,
    JSON.stringify(sent.map((m) => m.t)));
  const matrix = [
    "1111111",
    "1000001",
    "1011101",
    "1011101",
    "1011101",
    "1000001",
    "1111111",
  ];
  const darkCells = matrix.join("").split("").filter((c) => c === "1").length;
  window.__pico({
    ...baseSnap, teleportOpen: true,
    teleport: {
      state: "lan-rewrite",
      url: "http://192.168.1.20:5173/",
      note: "Phone must be on the same Wi-Fi.",
      matrix,
    },
  });
  check("v26 teleport: panel opens on the answering snapshot",
    panel.classList.contains("on"));
  check("v26 teleport: button reports expanded",
    tpBtn.getAttribute("aria-expanded") === "true");
  const svg = doc.querySelector("#tp-qr svg");
  check("v26 teleport: QR svg rendered", !!svg);
  const pathD = svg ? svg.querySelector("path").getAttribute("d") : "";
  check("v26 teleport: svg module count == matrix dark cells",
    (pathD.match(/M/g) || []).length === darkCells,
    `${(pathD.match(/M/g) || []).length} vs ${darkCells}`);
  check("v26 teleport: url input carries the resolved URL",
    doc.getElementById("tp-url").value === "http://192.168.1.20:5173/",
    doc.getElementById("tp-url").value);
  // AUDIT V26 (D3): the QR renders at the 264px floor, and the
  // card is centered by its stylesheet (jsdom has no layout;
  // the declarations + the backdrop element are the contract,
  // the visual centering is render-verified).
  check("v26 teleport: QR svg is at least 264px",
    !!svg && svg.getAttribute("width") === "264" && svg.getAttribute("height") === "264",
    svg ? svg.getAttribute("width") : "no svg");
  check("v26 teleport: card is centered in the stylesheet",
    /#tpanel\s*\{[^}]*left:\s*50%/.test(chromeHtml) &&
    /#tpanel\s*\{[^}]*translateX\(-50%\)/.test(chromeHtml) &&
    /#tpanel\s*\{[^}]*width:\s*400px/.test(chromeHtml));
  check("v26 teleport: backdrop element exists and follows the panel",
    !!doc.getElementById("tp-backdrop") &&
    doc.getElementById("tp-backdrop").classList.contains("on"));
  check("v26 teleport: backdrop is token-tinted in the stylesheet",
    // v27: one shared rule tints both centered panels' backdrops.
    /#tp-backdrop[^{]*\{[^}]*var\(--backdrop\)/.test(chromeHtml));
  // AUDIT V26 (D2): the feather + error bar are top-anchored —
  // a bottom anchor is what painted the black band when the
  // panel grew the chrome rect.
  check("v26 chrome: feather is top-anchored at the header's rows",
    /#feather\s*\{[^}]*top:\s*74px/.test(chromeHtml) &&
    !/#feather\s*\{[^}]*bottom:/.test(chromeHtml));
  check("v26 chrome: error bar is top-anchored",
    /#errbar\s*\{[^}]*top:\s*92px/.test(chromeHtml) &&
    !/#errbar\s*\{[^}]*bottom:/.test(chromeHtml));
  // AUDIT V26 (D1): the URL field carries the full string in
  // its tooltip and starts scrolled HOME.
  const tpUrl = doc.getElementById("tp-url");
  check("v26 teleport: url field tooltip carries the full URL",
    tpUrl.title === "http://192.168.1.20:5173/", tpUrl.title);
  check("v26 teleport: url field is scrolled home, never to the end",
    tpUrl.scrollLeft === 0, String(tpUrl.scrollLeft));
  tpUrl.click();
  check("v26 teleport: clicking the url field selects it all",
    tpUrl.selectionStart === 0 &&
    tpUrl.selectionEnd === tpUrl.value.length,
    `${tpUrl.selectionStart}..${tpUrl.selectionEnd}`);
  check("v26 teleport: note rendered as text",
    doc.getElementById("tp-note").textContent === "Phone must be on the same Wi-Fi.",
    doc.getElementById("tp-note").textContent);
  // Copy reports honestly (AUDIT V28 A1-L7): with a working
  // fallback the command fires and the button says Copied;
  // with the fallback failing, NO command fires and the button
  // admits the failure instead.
  doc.execCommand = () => true;
  doc.getElementById("tp-copy").click();
  check("v26 teleport: copy emits teleport_copy",
    sent.filter((m) => m.t === "teleport_copy").length === 1);
  check("v28 teleport: successful copy says Copied",
    doc.getElementById("tp-copy").textContent === "Copied",
    doc.getElementById("tp-copy").textContent);
  doc.execCommand = () => false;
  doc.getElementById("tp-copy").click();
  check("v28 teleport: failed copy says so and emits nothing",
    doc.getElementById("tp-copy").textContent === "Copy failed" &&
    sent.filter((m) => m.t === "teleport_copy").length === 1,
    doc.getElementById("tp-copy").textContent);
  delete doc.execCommand;
  // Esc closes and tells Rust.
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  check("v26 teleport: Esc closes the panel", !panel.classList.contains("on"));
  check("v26 teleport: Esc emits teleport_close",
    sent.filter((m) => m.t === "teleport_close").length === 1);
  // Reopen, then a tab switch closes it too.
  tpBtn.click();
  window.__pico({
    ...baseSnap, teleportOpen: true,
    teleport: { state: "public", url: "https://example.com/", note: "Opens this exact page on your phone.", matrix },
  });
  check("v26 teleport: panel reopens", panel.classList.contains("on"));
  window.__pico({ ...baseSnap, active: 2, teleportOpen: false, teleport: null, tabs: [...baseSnap.tabs, { id: 2, title: "Two", url: "https://example.com/", loading: false, start: false, failed: false }] });
  check("v26 teleport: tab switch closes the panel", !panel.classList.contains("on"));
  check("v28 teleport: a Rust-side close is mirrored, not echoed back",
    sent.filter((m) => m.t === "teleport_close").length === 1,
    JSON.stringify(sent.filter((m) => m.t === "teleport_close")));
  // Not-teleportable: no QR, no URL row, note only.
  tpBtn.click();
  window.__pico({
    ...baseSnap, active: 2, teleportOpen: true,
    teleport: { state: "not-teleportable", url: "", note: "Pico's own pages stay on this PC.", matrix: [] },
  });
  check("v26 teleport: not-teleportable still opens the panel",
    panel.classList.contains("on"));
  check("v26 teleport: not-teleportable hides the QR well",
    doc.getElementById("tp-qr").style.display === "none");
  check("v26 teleport: not-teleportable hides the copy button",
    doc.getElementById("tp-copy").style.display === "none");
  check("v26 teleport: not-teleportable shows its reason",
    doc.getElementById("tp-note").textContent.includes("stay on this PC"));
  doc.getElementById("tp-close").click();
  check("v26 teleport: x button closes + notifies",
    !panel.classList.contains("on") &&
    sent.filter((m) => m.t === "teleport_close").length === 2);
  // Backdrop click dismisses too (v26).
  tpBtn.click();
  window.__pico({
    ...baseSnap, active: 2, teleportOpen: true,
    teleport: { state: "public", url: "https://example.com/", note: "Opens this exact page on your phone.", matrix },
  });
  check("v26 teleport: panel reopens for the backdrop test",
    panel.classList.contains("on"));
  doc.getElementById("tp-backdrop").click();
  check("v26 teleport: backdrop click closes + notifies",
    !panel.classList.contains("on") &&
    !doc.getElementById("tp-backdrop").classList.contains("on") &&
    sent.filter((m) => m.t === "teleport_close").length === 3);
  // TooLong (v26): over-capacity URL — no QR, the URL row
  // stays copyable, and the note explains (never a blank
  // panel).
  tpBtn.click();
  window.__pico({
    ...baseSnap, active: 2, teleportOpen: true,
    teleport: {
      state: "public",
      url: "https://example.com/very/long",
      note: "Opens this exact page on your phone. This link is too long for a QR code — use Copy link instead.",
      matrix: [],
    },
  });
  check("v26 teleport: too-long still opens the panel with the URL row",
    panel.classList.contains("on") &&
    doc.getElementById("tp-url").style.display === "" &&
    doc.getElementById("tp-url").value === "https://example.com/very/long");
  check("v26 teleport: too-long hides the QR and explains itself",
    doc.getElementById("tp-qr").style.display === "none" &&
    doc.getElementById("tp-note").textContent.includes("too long for a QR code"),
    doc.getElementById("tp-note").textContent);
  doc.getElementById("tp-close").click();
  // AUDIT V28 A1-H1/I2: the two centered panels are mutually
  // exclusive THROUGH the snapshot, and a Rust-side Teleport
  // close (navigation) closes the chrome panel without the
  // next click being swallowed as a phantom close.
  const ppanel = doc.getElementById("ppanel");
  window.__pico({ ...baseSnap, active: 2, paletteOpen: true, teleportOpen: false, teleport: null });
  check("v28 panels: picker opens from its snapshot",
    ppanel.classList.contains("on") && !panel.classList.contains("on"));
  tpBtn.click();
  window.__pico({
    ...baseSnap, active: 2, paletteOpen: false, teleportOpen: true,
    teleport: { state: "public", url: "https://example.com/", note: "Opens this exact page on your phone.", matrix },
  });
  check("v28 panels: Teleport and picker are never both open",
    panel.classList.contains("on") && !ppanel.classList.contains("on"));
  const closesBefore = sent.filter((m) => m.t === "teleport_close").length;
  window.__pico({ ...baseSnap, active: 2, paletteOpen: false, teleportOpen: false, teleport: null });
  check("v28 panels: a Rust-side close closes the chrome panel silently",
    !panel.classList.contains("on") &&
    sent.filter((m) => m.t === "teleport_close").length === closesBefore);
  const tpReqsBefore = sent.filter((m) => m.t === "teleport").length;
  tpBtn.click();
  check("v28 panels: the click after a Rust-side close is a fresh request",
    sent.filter((m) => m.t === "teleport").length === tpReqsBefore + 1);
  window.__pico({
    ...baseSnap, active: 2, teleportOpen: true,
    teleport: { state: "public", url: "https://example.com/", note: "Opens this exact page on your phone.", matrix },
  });
  check("v28 panels: and it opens the panel again",
    panel.classList.contains("on"));
  // Escape cancels a PENDING opening too (A1-L2).
  doc.getElementById("tp-close").click();
  tpBtn.click();
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  window.__pico({ ...baseSnap, active: 2, teleportOpen: false, teleport: null });
  check("v28 teleport: Esc cancels a pending opening",
    !panel.classList.contains("on"));
  window.close();
}

// ------------------------------------------------------------------
// Chrome v27: button clusters + theme picker (Workstreams A/B).
// The navbar stands in the v27 cluster order; the palette
// button emits {t:"palette"}; an answering snapshot with
// paletteOpen renders one card per theme (6 dots each, the
// active theme pressed); a card click emits {t:"theme_set"};
// Esc and the backdrop emit {t:"palette_close"}; the root's
// data-palette follows the snapshot.
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  // AUDIT V28 A3-LOW-1: each palette carries its OWN six slots
  // (the v27 fixture gave every theme Forest's hexes, so dot
  // colors were untestable and identical by construction).
  const THEMES16 = [
    ["forest", "Forest", ["#0F1F17", "#1E3A2E", "#3E5A3F", "#6B7F67", "#C7CBB8", "#3D4337"]],
    ["ember", "Ember Navy", ["#0B132B", "#1C2541", "#3A506B", "#5BC0DE", "#F4D6B0", "#D97706"]],
    ["sage", "Sage Gold", ["#0B0F0D", "#1F2A1F", "#3E5A3F", "#8CA98C", "#D9C7A3", "#B08D57"]],
    ["lagoon", "Lagoon", ["#0A0F14", "#0F3D3E", "#2E6F73", "#7FB3B8", "#EADCC8", "#C6601F"]],
    ["ultraviolet", "Ultraviolet", ["#1A0F2E", "#3B1B4F", "#7A2E6F", "#D6598F", "#F4D7D4", "#FF6B6B"]],
    ["indigo", "Indigo Amber", ["#1A1A1F", "#2D2B55", "#6A5ACD", "#A678B9", "#F0D9B5", "#F4A62A"]],
    ["graphite", "Graphite Pink", ["#121212", "#1F2937", "#334155", "#64748B", "#F1F5F9", "#EC4899"]],
    ["violet", "Violet", ["#0E0B1D", "#2A1E4D", "#5A3B8C", "#8B7CC7", "#C9C2E3", "#3A3A52"]],
    ["bronze", "Bronze", ["#1A120E", "#3A2418", "#6B4A2E", "#A88462", "#D5C4AE", "#4A4A46"]],
    ["teal", "Deep Teal", ["#0B3D3B", "#0E6860", "#14B8A6", "#99F6E4", "#E6FFFB", "#C0FBF0"]],
    ["mist", "Mist Slate", ["#191D23", "#2F3A41", "#57707A", "#7B919C", "#989DAA", "#C5BAC4"]],
    ["lavender", "Lavender", ["#0D0B2D", "#282177", "#5B4BFF", "#A78BFA", "#E6E0FF", "#C7B6FD"]],
    ["royal", "Royal Blue", ["#0B1F3B", "#143779", "#2563EB", "#60A5FA", "#DBEAFE", "#9EC8FC"]],
    ["tangerine", "Tangerine", ["#1F1D2B", "#6D3E1C", "#FF7A00", "#FFA94D", "#FFE4C7", "#FFC78A"]],
    ["amethyst", "Amethyst", ["#5B21B6", "#6C36CC", "#8B5CF6", "#C7B3FF", "#EDE9FE", "#DACEFF"]],
    ["rose", "Rose", ["#6D0D3F", "#99225F", "#EC4899", "#F9A8D4", "#FCE7F3", "#FBC8E4"]],
  ].map(([id, name, slots]) => ({ id, name, slots }));
  const baseSnap = {
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "dark", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 1,
    otherListeners: 0, downloads: [], downloadsOpen: false, downloadsActive: 0, downloadsDefaultConn: 8, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, teleport: null,
    palette: "forest", themes: THEMES16, paletteOpen: false,
  };
  window.__pico({ ...baseSnap });
  check("v27 clusters: root carries the snapshot palette",
    doc.documentElement.getAttribute("data-palette") === "forest",
    doc.documentElement.getAttribute("data-palette"));
  const navIds = [...doc.querySelectorAll("#navbar button, #navbar input")]
    .map((el) => el.id).filter(Boolean);
  const want = ["back", "fwd", "reload", "home", "openfile", "addr", "shield", "lite", "downloads", "teleport", "palette", "theme"];
  check("v27 clusters: navbar controls stand in cluster order",
    JSON.stringify(navIds) === JSON.stringify(want), navIds.join(","));
  const palBtn = doc.getElementById("palette");
  check("v27 picker: palette button present in the send cluster",
    !!palBtn && !!palBtn.closest("#navbar"));
  const panel = doc.getElementById("ppanel");
  check("v27 picker: panel starts closed", !panel.classList.contains("on"));
  palBtn.click();
  check("v27 picker: click emits exactly one palette command",
    sent.filter((m) => m.t === "palette").length === 1,
    JSON.stringify(sent.map((m) => m.t)));
  window.__pico({ ...baseSnap, paletteOpen: true });
  check("v27 picker: panel opens on the answering snapshot",
    panel.classList.contains("on"));
  check("v27 picker: button reports expanded",
    palBtn.getAttribute("aria-expanded") === "true");
  const cards = doc.querySelectorAll("#pp-grid .pcard");
  check("v27 picker: one card per theme", cards.length === 16, `got ${cards.length}`);
  check("picker: cards stand in catalogue order",
    JSON.stringify([...cards].map((c) => c.dataset.id)) ===
    JSON.stringify(THEMES16.map((t) => t.id)),
    [...cards].map((c) => c.dataset.id).join(","));
  check("picker: panel scrolls its grid (16 cards overflow the sheet)",
    window.getComputedStyle(panel).overflowY === "auto",
    window.getComputedStyle(panel).overflowY);
  check("v27 picker: every card shows six palette dots",
    [...cards].every((c) => c.querySelectorAll(".pdot").length === 6));
  check("v27 picker: the active theme is the pressed card",
    doc.querySelector('.pcard[data-id="forest"]').getAttribute("aria-pressed") === "true" &&
    doc.querySelector('.pcard[data-id="ember"]').getAttribute("aria-pressed") === "false");
  check("v28 picker: dot colors are the palette's own, per card",
    doc.querySelector('.pcard[data-id="forest"] .pdot').style.background !==
    doc.querySelector('.pcard[data-id="ember"] .pdot').style.background &&
    doc.querySelector('.pcard[data-id="violet"] .pdot').style.background !==
    doc.querySelector('.pcard[data-id="bronze"] .pdot').style.background);
  const emberCard = doc.querySelector('.pcard[data-id="ember"]');
  emberCard.focus();
  emberCard.click();
  const setMsgs = sent.filter((m) => m.t === "theme_set");
  check("v27 picker: card click emits theme_set with the card id",
    setMsgs.length === 1 && setMsgs[0].id === "ember", JSON.stringify(setMsgs));
  window.__pico({ ...baseSnap, palette: "ember", paletteOpen: true });
  check("v27 picker: palette attribute follows a theme switch",
    doc.documentElement.getAttribute("data-palette") === "ember");
  check("v28 picker: the pressed card moves with the switch",
    doc.querySelector('.pcard[data-id="ember"]').getAttribute("aria-pressed") === "true" &&
    doc.querySelector('.pcard[data-id="forest"]').getAttribute("aria-pressed") === "false");
  check("v28 picker: a selection change does not rebuild the focused card",
    doc.activeElement === emberCard,
    doc.activeElement && doc.activeElement.className);
  // 0.31: the LAST catalogue card (below the panel's fold —
  // it is reached by the panel's own scroll) is a real,
  // focusable button whose click selects its theme.
  const roseCard = doc.querySelector('.pcard[data-id="rose"]');
  check("picker: the last card (rose) is present with six dots",
    !!roseCard && roseCard.querySelectorAll(".pdot").length === 6);
  roseCard.focus();
  roseCard.click();
  const roseMsgs = sent.filter((m) => m.t === "theme_set" && m.id === "rose");
  check("picker: last-card click emits theme_set rose",
    roseMsgs.length === 1, JSON.stringify(roseMsgs));
  window.__pico({ ...baseSnap, palette: "rose", paletteOpen: true });
  check("picker: pressed state reaches the last card",
    doc.querySelector('.pcard[data-id="rose"]').getAttribute("aria-pressed") === "true" &&
    doc.querySelector('.pcard[data-id="forest"]').getAttribute("aria-pressed") === "false");
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  check("v27 picker: Esc emits palette_close",
    sent.filter((m) => m.t === "palette_close").length === 1,
    JSON.stringify(sent.map((m) => m.t)));
  window.__pico({ ...baseSnap, paletteOpen: true });
  doc.getElementById("pp-backdrop").click();
  check("v27 picker: backdrop click emits palette_close",
    sent.filter((m) => m.t === "palette_close").length === 2,
    JSON.stringify(sent.map((m) => m.t)));
  window.close();
}

// ------------------------------------------------------------------
// Chrome v26: address focus + editing guard (defects D4/D7).
// A snapshot carrying focusAddr focuses the input; while the
// user edits, snapshots never stomp the draft; blur re-syncs;
// a plain tab-switch snapshot never yanks focus.
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  const addr = doc.getElementById("addr");
  const snap = (over) => ({
    tabs: [{ id: 1, title: "Start", url: "pico://start", loading: false, start: true, failed: false }],
    active: 1, theme: "dark", address: "",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, downloads: [], downloadsOpen: false, downloadsActive: 0, downloadsDefaultConn: 8, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, teleport: null, focusAddr: false,
    ...over,
  });
  // D4: the focus hint lands focus in the address input.
  window.__pico(snap({ focusAddr: true }));
  check("v26 focus: focusAddr snapshot focuses the address input",
    doc.activeElement === addr, doc.activeElement && doc.activeElement.id);
  // D7 guard: type a draft, then a navigation snapshot lands —
  // the draft survives; the stored value waits for blur.
  addr.value = "half typed query";
  addr.dispatchEvent(new window.Event("input", { bubbles: true }));
  window.__pico(snap({ address: "https://www.youtube.com/watch?v=live1" }));
  check("v26 guard: dirty draft is never stomped by a snapshot",
    addr.value === "half typed query", addr.value);
  addr.blur();
  check("v26 guard: blur re-syncs the input to the live URL",
    addr.value === "https://www.youtube.com/watch?v=live1", addr.value);
  // A clean focused input follows navigation (SPA case).
  addr.focus();
  window.__pico(snap({ address: "https://www.youtube.com/watch?v=live2" }));
  check("v26 guard: clean focused input follows the live URL",
    addr.value === "https://www.youtube.com/watch?v=live2", addr.value);
  // No yank: a plain snapshot never moves focus off a control.
  const reloadBtn = doc.getElementById("reload");
  reloadBtn.focus();
  window.__pico(snap({ address: "https://example.com/" }));
  check("v26 focus: snapshot without the hint leaves focus alone",
    doc.activeElement === reloadBtn, doc.activeElement && doc.activeElement.id);
  window.close();
}

// ------------------------------------------------------------------
// Chrome 0.31 fixes: A1-F2 (tabstrip scroll survives ordinary
// snapshots), A1-F3 (address Escape does not dismiss panels),
// A1-F4 (focusAddr forces a resync over a dirty draft),
// A1-F6 (focus moves when the focused tab is closed), A6-F9
// (a dirty draft belongs to its tab), A3-F2 (picker dot ring).
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  const addr = doc.getElementById("addr");
  const strip = doc.getElementById("tabstrip");
  // jsdom gives elements no scrollIntoView — install the spy
  // the A1-F2 fixture calls for before any snapshot renders.
  let sivCalls = 0;
  window.Element.prototype.scrollIntoView = function () { sivCalls++; };
  const TABS3 = [
    { id: 1, title: "One", url: "http://one.example/", loading: false, start: false, failed: false },
    { id: 2, title: "Two", url: "http://two.example/", loading: false, start: false, failed: false },
    { id: 3, title: "Three", url: "http://three.example/", loading: false, start: false, failed: false },
  ];
  const snap = (over) => ({
    tabs: TABS3, active: 1, theme: "light", address: "http://one.example/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, downloads: [], downloadsOpen: false, downloadsActive: 0, downloadsDefaultConn: 8, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, focusAddr: false,
    paletteOpen: false, teleportOpen: false, teleport: null,
    ...over,
  });
  // --- A1-F2: scroll position survives a same-active snapshot.
  window.__pico(snap({}));
  check("0.31 A1-F2: first render scrolls the active tab into view",
    sivCalls === 1, `siv=${sivCalls}`);
  strip.scrollLeft = 42;
  sivCalls = 0;
  window.__pico(snap({ blockedTotal: 7, blockedTab: 2 }));
  check("0.31 A1-F2: same-active snapshot does not call scrollIntoView",
    sivCalls === 0, `siv=${sivCalls}`);
  check("0.31 A1-F2: manual tabstrip scrollLeft survives the snapshot",
    strip.scrollLeft === 42, String(strip.scrollLeft));
  check("0.31 A1-F2: the changed badge still rendered",
    doc.getElementById("shield-badge").textContent === "7 blocked",
    doc.getElementById("shield-badge").textContent);
  window.__pico(snap({ active: 2, address: "http://two.example/" }));
  check("0.31 A1-F2: an active-tab change scrolls into view again",
    sivCalls === 1, `siv=${sivCalls}`);
  // --- A1-F6: closing the focused (inactive) tab moves focus
  // to the tab now at the same strip index (clamped).
  const tab3 = strip.querySelector('[data-tab-id="3"]');
  tab3.focus();
  check("0.31 A1-F6: fixture focused the inactive third tab",
    doc.activeElement === tab3, doc.activeElement && doc.activeElement.className);
  window.__pico(snap({ tabs: TABS3.slice(0, 2), active: 2, address: "http://two.example/" }));
  check("0.31 A1-F6: focus lands on a tab element, not <body>",
    !!doc.activeElement && doc.activeElement.classList.contains("tab"),
    doc.activeElement ? doc.activeElement.tagName : "none");
  check("0.31 A1-F6: focus clamped to the last remaining tab",
    doc.activeElement && doc.activeElement.dataset.tabId === "2",
    doc.activeElement && doc.activeElement.dataset.tabId);
  // --- A6-F9: a dirty draft typed on tab A must not follow
  // the user to tab B.
  window.__pico(snap({ tabs: TABS3.slice(0, 2), active: 1, address: "http://one.example/" }));
  addr.focus();
  addr.value = "draft meant for tab one";
  addr.dispatchEvent(new window.Event("input", { bubbles: true }));
  window.__pico(snap({ tabs: TABS3.slice(0, 2), active: 2, address: "http://two.example/" }));
  check("0.31 A6-F9: switching tabs resyncs the input to the new tab",
    addr.value === "http://two.example/", addr.value);
  window.__pico(snap({ tabs: TABS3.slice(0, 2), active: 2, address: "http://two.example/next" }));
  check("0.31 A6-F9: dirty is cleared (clean input follows navigation)",
    addr.value === "http://two.example/next", addr.value);
  // --- A1-F4: already focused + dirty + focusAddr forces the
  // resync, focus and selection.
  addr.value = "stale draft before new tab";
  addr.dispatchEvent(new window.Event("input", { bubbles: true }));
  window.__pico(snap({
    tabs: TABS3.slice(0, 2), active: 2, address: "http://fresh.example/",
    focusAddr: true,
  }));
  check("0.31 A1-F4: focusAddr resyncs over a dirty draft",
    addr.value === "http://fresh.example/", addr.value);
  check("0.31 A1-F4: the input keeps focus and the new text is selected",
    doc.activeElement === addr &&
    addr.selectionStart === 0 && addr.selectionEnd === addr.value.length,
    `${doc.activeElement && doc.activeElement.id} ${addr.selectionStart}..${addr.selectionEnd}`);
  // --- A1-F3: one Escape in the address input reverts the
  // draft ONLY; a second Escape (at the document) closes the
  // open panel.
  window.__pico(snap({
    tabs: TABS3.slice(0, 2), active: 2, address: "http://fresh.example/",
    paletteOpen: true,
  }));
  const ppanel = doc.getElementById("ppanel");
  check("0.31 A1-F3: fixture opened the palette panel",
    ppanel.classList.contains("on"));
  addr.focus();
  addr.value = "another dirty draft";
  addr.dispatchEvent(new window.Event("input", { bubbles: true }));
  addr.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  check("0.31 A1-F3: input Escape reverts the draft",
    addr.value === "http://fresh.example/", addr.value);
  check("0.31 A1-F3: the same Escape leaves the panel open",
    ppanel.classList.contains("on") &&
    sent.filter((m) => m.t === "palette_close").length === 0,
    JSON.stringify(sent.map((m) => m.t)));
  doc.body.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  check("0.31 A1-F3: a second, document-level Escape closes the panel",
    !ppanel.classList.contains("on") &&
    sent.filter((m) => m.t === "palette_close").length === 1,
    JSON.stringify(sent.map((m) => m.t)));
  // --- A3-F2: the picker dots carry the neutral inset ring.
  check("0.31 A3-F2: palette dots carry the neutral inset ring",
    chromeHtml.includes("rgba(128,128,128,.45)") &&
    chromeHtml.includes("--neu-pdot-ring"),
    "ring token missing from the shipped chrome page");
  window.close();
}

// ------------------------------------------------------------------
// Chrome: dead-shell path — no snapshot ever arrives, the page
// itself must explain the failure (never a silent black void).
// ------------------------------------------------------------------
{
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
  });
  const { window } = dom;
  const doc = window.document;
  await sleep(2200);
  check("dead shell: warn strip appears without a snapshot",
    doc.getElementById("shellwarn").classList.contains("on"));
  check("dead shell: strip names pico.log",
    doc.getElementById("shellwarn").textContent.includes("pico.log"),
    doc.getElementById("shellwarn").textContent);
  window.close();
}

// ------------------------------------------------------------------
// Start page: services render; the "other listeners" line is one
// muted aggregate, driven by INIT and by the live hook. AND the
// v17 seam: window.ipc is stubbed exactly as wry provides it, and
// every control's click must emit the exact action request the
// Rust dispatch parses (tests/action_tests.rs pins the other end
// of the same strings).
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(startHtml, {
    runScripts: "dangerously",
    url: "https://pico.start/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  // Track whether clicks are default-prevented (the page's IPC
  // path ran) via a bubble-phase listener, which observes the
  // event after the target's own handlers.
  const prevented = [];
  doc.addEventListener("click", (ev) => {
    if (ev.defaultPrevented) {
      const el = ev.target.closest("a");
      if (el) prevented.push(el.className);
    }
  });
  const actionsOf = (name) => sent.filter((m) => m.kind === "action" && m.action === name);
  // 0.35.1: the fixture INIT carries the NON-default ember/dark
  // theme (dump_pages.rs), so boot-applied INIT theming is
  // proven against values the static <html> defaults do NOT
  // carry. The theme hook is the object-payload
  // window.__picoTheme({v:1, mode, palette}); the old
  // window.__picoSetTheme is deleted.
  check("start: fixture INIT applies ember/dark (boot theme)",
    doc.documentElement.getAttribute("data-palette") === "ember" &&
    doc.documentElement.getAttribute("data-theme") === "dark",
    `${doc.documentElement.getAttribute("data-palette")}/${doc.documentElement.getAttribute("data-theme")}`);
  check("start: __picoTheme hook present, old __picoSetTheme deleted",
    typeof window.__picoTheme === "function" &&
    typeof window.__picoSetTheme === "undefined",
    `__picoTheme=${typeof window.__picoTheme} __picoSetTheme=${typeof window.__picoSetTheme}`);
  if (typeof window.__picoTheme === "function") window.__picoTheme({ v: 1, mode: "dark", palette: "ember" });
  check("start: __picoTheme sets mode and palette together",
    doc.documentElement.getAttribute("data-theme") === "dark" &&
    doc.documentElement.getAttribute("data-palette") === "ember");
  // The old one-argument mode-only push has no counterpart in
  // the new contract (a payload always carries both fields):
  // its intent — change the mode, leave the palette — is
  // preserved by sending the page's CURRENT palette (ember).
  if (typeof window.__picoTheme === "function") window.__picoTheme({ v: 1, mode: "light", palette: "ember" });
  check("start: __picoTheme mode update (current palette carried) sets the mode only in effect",
    doc.documentElement.getAttribute("data-theme") === "light" &&
    doc.documentElement.getAttribute("data-palette") === "ember");
  check("start: others line hidden at 0",
    doc.getElementById("others").hidden === true);
  window.__picoSetOthers(4);
  const others = doc.getElementById("others");
  check("start: others line appears with count",
    others.hidden === false && others.textContent.includes("4"),
    others.textContent);
  check("start: others line is aggregate-only (no per-port cards)",
    others.querySelectorAll("a").length === 0);
  // Fixture INIT (examples/dump_pages.rs) renders real items:
  // two recents and one service card — clickable, like production.
  const fixtureRecents = doc.querySelectorAll("#recents .recent");
  check("start: fixture renders two recent entries", fixtureRecents.length === 2,
    `got ${fixtureRecents.length}`);
  check("start: fixture recent shows its file name",
    fixtureRecents.length === 2 && fixtureRecents[0].textContent.includes("First Page.html"),
    fixtureRecents.length ? fixtureRecents[0].textContent : "missing");
  check("start: fixture renders one service card",
    doc.querySelectorAll("#services .card").length === 1);
  window.__picoSetServices([
    { port: 5173, pid: 1, process: "node.exe", title: "My App", label: "My App", url: "http://localhost:5173", kind: "http" },
  ]);
  const cards = doc.querySelectorAll("#services .card");
  check("start: one service card rendered", cards.length === 1);
  check("start: card shows title and port",
    cards[0].textContent.includes("My App") && cards[0].textContent.includes(":5173"),
    cards[0].textContent);
  // Branding (v15): byline row + enum-wired chips + footer line.
  const byText = doc.querySelector(".byline .by-text");
  check("start: byline names the author",
    !!byText && byText.textContent === "by @iK4w4",
    byText ? byText.textContent : "missing");
  const chips = doc.querySelectorAll(".byline .chip");
  check("start: two social chips rendered", chips.length === 2, `got ${chips.length}`);
  check("start: chips carry enum names only (no URLs)",
    chips.length === 2 &&
    chips[0].getAttribute("data-social") === "telegram" &&
    chips[0].getAttribute("href") === "pico://social/telegram" &&
    chips[1].getAttribute("data-social") === "youtube" &&
    chips[1].getAttribute("href") === "pico://social/youtube",
    chips.length ? chips[0].getAttribute("href") : "missing");
  check("start: chip glyphs are inline SVG (no <img>)",
    doc.querySelectorAll(".byline .chip svg").length === 2 &&
    doc.querySelectorAll(".byline img").length === 0);
  check("start: footer reads 'Pico 0.40.1 · by @iK4w4'",
    doc.querySelector(".footer").textContent.includes("Pico 0.40.1 · by @iK4w4"));
  // Clicking a chip must target its action URL as the fallback
  // (the Rust nav handler turns that into a NEW tab — the start
  // tab never navigates). Capture the click before jsdom follows.
  const clicked = [];
  doc.addEventListener("click", (ev) => {
    const a = ev.target.closest("a.chip");
    if (a) { ev.preventDefault(); clicked.push(a.getAttribute("href")); }
  }, true);
  chips[0].click();
  chips[1].click();
  check("start: chip clicks target the social action URLs",
    clicked.join("|") === "pico://social/telegram|pico://social/youtube",
    clicked.join("|"));
  // THE v17 SEAM — each control's click emits exactly one IPC
  // action request of the exact shape Rust parses.
  check("start: chip clicks emit enum-only social actions over IPC",
    actionsOf("social").length === 2 &&
    actionsOf("social")[0].site === "telegram" &&
    actionsOf("social")[1].site === "youtube" &&
    !JSON.stringify(actionsOf("social")).includes("http"),
    JSON.stringify(actionsOf("social")));
  // v24: a recent row is a container div; the opening anchor is
  // its .recent-main child (the row's other anchor is the ×).
  fixtureRecents[1].querySelector(".recent-main").click();
  check("start: recent click emits {action:recent, index:1} over IPC",
    actionsOf("recent").length === 1 && actionsOf("recent")[0].index === 1,
    JSON.stringify(actionsOf("recent")));
  check("start: recent click was intercepted (default prevented)",
    prevented.some((c) => c.includes("recent")), prevented.join(","));
  doc.getElementById("pick-file").click();
  check("start: pick-file click emits {action:pick-file} over IPC",
    actionsOf("pick-file").length === 1,
    JSON.stringify(actionsOf("pick-file")));
  check("start: pick-file click was intercepted (default prevented)",
    prevented.some((c) => c.includes("action")), prevented.join(","));
  cards[0].click();
  check("start: service card click emits its URL over IPC",
    actionsOf("service").length === 1 &&
    actionsOf("service")[0].url === "http://localhost:5173",
    JSON.stringify(actionsOf("service")));
  check("start: service click was intercepted (default prevented)",
    prevented.some((c) => c.includes("card")), prevented.join(","));
  // ------------------------------------------------------------
  // v24 recents management (AUDIT V24-2): the × on a row, Clear
  // all, and Hide/Show all emit their exact action shapes; the
  // hidden state Rust pushes back collapses the section to the
  // header line and persists through the render cycle.
  // ------------------------------------------------------------
  const xs = doc.querySelectorAll("#recents .recent-x");
  check("v24: each recent row carries a remove (x) anchor",
    xs.length === 2 &&
    xs[0].getAttribute("href") === "pico://recent-remove/0" &&
    xs[1].getAttribute("href") === "pico://recent-remove/1",
    `got ${xs.length}`);
  const recentsHead = doc.getElementById("recents-head");
  check("v24: recents header is visible with its count",
    recentsHead.hidden === false &&
    doc.getElementById("recents-count").textContent.includes("2"),
    doc.getElementById("recents-count").textContent);
  xs[0].click();
  check("v24: x click emits {action:recent-remove, index:0} over IPC",
    actionsOf("recent-remove").length === 1 &&
    actionsOf("recent-remove")[0].index === 0,
    JSON.stringify(actionsOf("recent-remove")));
  check("v24: x click was intercepted (default prevented)",
    prevented.some((c) => c.includes("recent-x")), prevented.join(","));
  doc.getElementById("recents-clear").click();
  check("v24: Clear all emits {action:recents-clear} over IPC",
    actionsOf("recents-clear").length === 1,
    JSON.stringify(actionsOf("recents-clear")));
  const visBtn = doc.getElementById("recents-vis");
  check("v24: visibility toggle offers Hide while visible",
    visBtn.textContent === "Hide" &&
    visBtn.getAttribute("href") === "pico://recents-visibility/hide",
    visBtn.textContent);
  visBtn.click();
  check("v24: Hide click emits {action:recents-visibility, hidden:true}",
    actionsOf("recents-visibility").length === 1 &&
    actionsOf("recents-visibility")[0].hidden === true,
    JSON.stringify(actionsOf("recents-visibility")));
  // Rust persists the flag and pushes it back; the section must
  // collapse to the single header line "RECENT FILES — hidden".
  window.__picoSetRecentsHidden(true);
  check("v24: hidden state collapses the list, header stays",
    doc.getElementById("recents").style.display === "none" &&
    recentsHead.hidden === false &&
    doc.getElementById("recents-count").textContent.includes("hidden") &&
    visBtn.textContent === "Show" &&
    visBtn.getAttribute("href") === "pico://recents-visibility/show",
    doc.getElementById("recents-count").textContent);
  check("v24: Clear all is not offered while hidden",
    doc.getElementById("recents-clear").style.display === "none");
  visBtn.click();
  check("v24: Show click emits {action:recents-visibility, hidden:false}",
    actionsOf("recents-visibility").length === 2 &&
    actionsOf("recents-visibility")[1].hidden === false,
    JSON.stringify(actionsOf("recents-visibility")));
  window.__picoSetRecentsHidden(false);
  check("v24: shown state restores the list and the Hide offer",
    doc.getElementById("recents").style.display === "" &&
    visBtn.textContent === "Hide" &&
    doc.getElementById("recents-clear").style.display === "");
  // After a removal, Rust pushes the shortened list; the rows
  // (and their × indexes) re-render from it — the page never
  // edits the list itself.
  window.__picoSetRecents(["C:\\Users\\Fixture\\Desktop\\Second.html"]);
  const afterRemove = doc.querySelectorAll("#recents .recent");
  check("v24: pushed list re-renders rows with fresh indexes",
    afterRemove.length === 1 &&
    afterRemove[0].querySelector(".recent-main").getAttribute("href") === "pico://recent/0" &&
    afterRemove[0].querySelector(".recent-x").getAttribute("href") === "pico://recent-remove/0" &&
    doc.getElementById("recents-count").textContent.includes("1"),
    `rows=${afterRemove.length}`);
  window.__picoSetRecents([]);
  check("v24: empty list hides the whole recents header",
    recentsHead.hidden === true && doc.querySelectorAll("#recents .recent").length === 0);
  // ------------------------------------------------------------
  // 0.30: the savings band (AUDIT 0.30). The fixture INIT
  // carries a populated book (examples/dump_pages.rs); the
  // tiles must render it faithfully — counts exact and
  // grouped, bytes/time humanized with the ≈ of an estimate,
  // the pruned line present only on the Shield tile — and
  // the zero state must replace the band wholesale.
  // ------------------------------------------------------------
  const bandPos = doc.querySelector(".hero")
    .compareDocumentPosition(doc.getElementById("stats-grid"));
  check("0.30: the band sits in the hero zone (after the hero, before the services)",
    (bandPos & window.Node.DOCUMENT_POSITION_FOLLOWING) !== 0 &&
    (doc.getElementById("stats-grid")
      .compareDocumentPosition(doc.getElementById("services")) &
      window.Node.DOCUMENT_POSITION_FOLLOWING) !== 0);
  check("0.30: shield tile renders the fixture book",
    doc.getElementById("stat-shield-bytes").textContent === "≈ 86.4 MB" &&
    doc.getElementById("stat-shield-count").textContent === "1,234 requests blocked" &&
    doc.getElementById("stat-shield-pruned").textContent ===
      "+ 34 ad payloads stripped in-page · ≈ 512 KB of removed values, counted per removal" &&
    doc.getElementById("stat-shield-session").textContent ===
      "12 blocked this session · ≈ 640 KB",
    doc.getElementById("stat-shield-bytes").textContent + " | " +
    doc.getElementById("stat-shield-count").textContent + " | " +
    doc.getElementById("stat-shield-pruned").textContent + " | " +
    doc.getElementById("stat-shield-session").textContent);
  check("0.30: lite tile renders its own book (no pruned line exists there)",
    doc.getElementById("stat-lite-bytes").textContent === "≈ 22.4 MB" &&
    doc.getElementById("stat-lite-count").textContent === "56 requests blocked" &&
    doc.getElementById("stat-lite-session").textContent ===
      "3 blocked this session · ≈ 1.2 MB" &&
    doc.getElementById("stat-lite-pruned") === null,
    doc.getElementById("stat-lite-bytes").textContent);
  check("0.30: time line combines both engines, session alongside",
    doc.getElementById("stat-time").textContent ===
      "≈ 16.7 min of loading prevented · ≈ 17.1 s this session",
    doc.getElementById("stat-time").textContent);
  check("0.30: populated band hides the zero state",
    doc.getElementById("stat-zero").hidden === true &&
    doc.getElementById("stats-grid").style.display === "");
  // Zero state: wholesale swap, no stale figures.
  window.__picoSetStats({
    shield: { blocked: 0, estBytes: 0, estDurationMs: 0, prunedEvents: 0, prunedBytes: 0, sessionBlocked: 0, sessionEstBytes: 0, sessionEstDurationMs: 0 },
    lite: { blocked: 0, estBytes: 0, estDurationMs: 0, prunedEvents: 0, prunedBytes: 0, sessionBlocked: 0, sessionEstBytes: 0, sessionEstDurationMs: 0 },
  });
  check("0.30: zero book shows the zero state and hides grid + time",
    doc.getElementById("stat-zero").hidden === false &&
    doc.getElementById("stats-grid").style.display === "none" &&
    doc.getElementById("stat-time").style.display === "none");
  // A single blocked request: singular grammar, session line
  // present, time line back.
  window.__picoSetStats({
    shield: { blocked: 1, estBytes: 50000, estDurationMs: 800, prunedEvents: 0, prunedBytes: 0, sessionBlocked: 1, sessionEstBytes: 50000, sessionEstDurationMs: 800 },
    lite: { blocked: 0, estBytes: 0, estDurationMs: 0, prunedEvents: 0, prunedBytes: 0, sessionBlocked: 0, sessionEstBytes: 0, sessionEstDurationMs: 0 },
  });
  check("0.30: one block renders singular and revives the band",
    doc.getElementById("stat-shield-count").textContent === "1 request blocked" &&
    doc.getElementById("stat-shield-bytes").textContent === "≈ 50 KB" &&
    doc.getElementById("stat-time").textContent ===
      "≈ 0.8 s of loading prevented · ≈ 0.8 s this session" &&
    doc.getElementById("stat-zero").hidden === true,
    doc.getElementById("stat-shield-count").textContent + " | " +
    doc.getElementById("stat-time").textContent);
  // The humanizer mirrors: the SAME fixture cases the Rust
  // unit tests pin (core::stats) — one fixture set, two
  // implementations.
  const byteCases = [
    [0, "0 B"], [999, "999 B"], [1000, "1 KB"], [1500, "1.5 KB"],
    [12300, "12.3 KB"], [999499, "999 KB"], [1000000, "1 MB"],
    [86400000, "86.4 MB"], [1250000000, "1.25 GB"], [999950000, "1 GB"],
    [5500000000000, "5.5 TB"], [42, "42 B"],
    // Exact rounding ties: half-up in both implementations.
    [12250, "12.3 KB"], [10250, "10.3 KB"],
    // A5-L13: the EB cap — 2^62 bytes humanizes in exabytes,
    // matching the Rust humanizer's EB unit.
    [4611686018427387904, "4.61 EB"],
  ];
  check("0.30: JS humanizeBytes matches the Rust fixtures",
    byteCases.every(([n, want]) => window.humanizeBytes(n) === want),
    byteCases.map(([n]) => window.humanizeBytes(n)).join(","));
  const durCases = [
    [0, "0 s"], [400, "0.4 s"], [45000, "45 s"], [59000, "59 s"],
    [90000, "1.5 min"], [3600000, "1 h"], [5400000, "1.5 h"],
    [86400000, "1 day"], [216000000, "2.5 days"], [1000, "1 s"],
    [1125, "1.13 s"], [12250, "12.3 s"],
  ];
  check("0.30: JS humanizeDuration matches the Rust fixtures",
    durCases.every(([n, want]) => window.humanizeDuration(n) === want),
    durCases.map(([n]) => window.humanizeDuration(n)).join(","));
  // Reset: the control rides the action dispatch like every
  // other privileged gesture, with the anchor fallback.
  const resetEl = doc.getElementById("stats-reset");
  check("0.30: reset control carries the pico://stats-reset fallback",
    resetEl.getAttribute("href") === "pico://stats-reset",
    resetEl.getAttribute("href"));
  resetEl.click();
  check("0.30: reset click emits {action:stats-reset} over IPC",
    actionsOf("stats-reset").length === 1,
    JSON.stringify(actionsOf("stats-reset")));
  window.close();
}

// ---------------------------------------------------------------------
// 0.35.1 theme live-apply (the Defect 1 proof): on BOTH internal
// pages, window.__picoTheme({v:1, mode, palette}) retokens an
// ALREADY-RENDERED page in place — attributes change on the same
// window/document (no reload), and the computed --bg follows to
// the new palette+mode's derived token. Token values below are
// core::themes::derive() for those palettes (themes.rs):
//   ember/dark   --bg #0B132B (slot 1 verbatim)
//   forest/light --bg #D8DBCD (slot 5 #C7CBB8 mixed to white 0.30)
//   royal/dark   --bg #0B1F3B (slot 1 verbatim)
// jsdom in this harness DOES resolve custom properties from the
// [data-palette][data-theme] rules (verified against the dumped
// pages), so the computed-style assertions are real; each block
// ALSO asserts the stylesheet text carries the target block, so
// the attributes are proven to select real tokens either way.
// Defensive contract: invalid payloads change nothing and never
// throw; where the hook returns a boolean it must return false
// (asserted conditionally — a non-boolean return is checked
// state-only, per the contract handoff).
// ---------------------------------------------------------------------
{
  // [label, payload] — every entry must be a no-op.
  const INVALID_THEMES = [
    ["undefined", undefined],
    ["null", null],
    ["bare string", "dark"],
    ["wrong v", { v: 2, mode: "dark", palette: "forest" }],
    ["bad mode", { v: 1, mode: "blue", palette: "forest" }],
    ["unknown palette", { v: 1, mode: "dark", palette: "not-a-palette" }],
    ["missing palette", { v: 1, mode: "dark" }],
  ];
  const callTheme = (win, payload) => {
    let threw = false, ret;
    try { ret = win.__picoTheme(payload); } catch (e) { threw = true; }
    return { threw, ret };
  };
  const defensiveChecks = (label, win, doc) => {
    for (const [name, payload] of INVALID_THEMES) {
      const beforeTheme = doc.documentElement.getAttribute("data-theme");
      const beforePalette = doc.documentElement.getAttribute("data-palette");
      const { threw, ret } = callTheme(win, payload);
      check(`0.35.1 theme defensive (${label}): ${name} changes nothing, never throws`,
        typeof win.__picoTheme === "function" && !threw &&
        (typeof ret !== "boolean" || ret === false) &&
        doc.documentElement.getAttribute("data-theme") === beforeTheme &&
        doc.documentElement.getAttribute("data-palette") === beforePalette,
        `threw=${threw} ret=${String(ret)} attrs=${doc.documentElement.getAttribute("data-palette")}/${doc.documentElement.getAttribute("data-theme")}`);
    }
  };
  const liveApplyChecks = (label, html, url) => {
    const dom = new JSDOM(html, {
      runScripts: "dangerously",
      url,
      beforeParse(window) {
        window.ipc = { postMessage: () => {} };
      },
    });
    const win = dom.window;
    const doc = win.document;
    const docBefore = doc;
    check(`0.35.1 theme (${label}): __picoTheme hook present, old __picoSetTheme deleted`,
      typeof win.__picoTheme === "function" &&
      typeof win.__picoSetTheme === "undefined",
      `__picoTheme=${typeof win.__picoTheme} __picoSetTheme=${typeof win.__picoSetTheme}`);
    check(`0.35.1 theme (${label}): fixture INIT applies ember/dark (boot theme)`,
      doc.documentElement.getAttribute("data-palette") === "ember" &&
      doc.documentElement.getAttribute("data-theme") === "dark",
      `${doc.documentElement.getAttribute("data-palette")}/${doc.documentElement.getAttribute("data-theme")}`);
    const bgBefore = win.getComputedStyle(doc.documentElement).getPropertyValue("--bg").trim();
    check(`0.35.1 theme (${label}): computed --bg is the ember/dark token before the push`,
      bgBefore === "#0B132B", bgBefore);
    // Live-apply: DIFFERENT palette (ember -> forest) + flipped
    // mode (dark -> light) on the already-rendered page.
    const { threw } = callTheme(win, { v: 1, mode: "light", palette: "forest" });
    check(`0.35.1 theme (${label}): live-apply changes data-palette/data-theme in place, no throw`,
      !threw &&
      doc.documentElement.getAttribute("data-palette") === "forest" &&
      doc.documentElement.getAttribute("data-theme") === "light",
      `threw=${threw} attrs=${doc.documentElement.getAttribute("data-palette")}/${doc.documentElement.getAttribute("data-theme")}`);
    check(`0.35.1 theme (${label}): live-apply happened WITHOUT a reload (same window/document)`,
      win.document === docBefore && doc === docBefore);
    const bgAfter = win.getComputedStyle(doc.documentElement).getPropertyValue("--bg").trim();
    check(`0.35.1 theme (${label}): computed --bg changed to the forest/light token`,
      bgAfter === "#D8DBCD" && bgAfter !== bgBefore,
      `${bgBefore} -> ${bgAfter}`);
    check(`0.35.1 theme (${label}): stylesheet carries the [data-palette="forest"][data-theme="light"] block the attributes now select`,
      html.includes('[data-palette="forest"][data-theme="light"]'));
    // Park the page on a known valid state, then the
    // defensive battery must leave exactly that state standing.
    callTheme(win, { v: 1, mode: "dark", palette: "royal" });
    check(`0.35.1 theme (${label}): setup for defensive checks lands on royal/dark`,
      doc.documentElement.getAttribute("data-palette") === "royal" &&
      doc.documentElement.getAttribute("data-theme") === "dark" &&
      win.getComputedStyle(doc.documentElement).getPropertyValue("--bg").trim() === "#0B1F3B",
      `${doc.documentElement.getAttribute("data-palette")}/${doc.documentElement.getAttribute("data-theme")}`);
    defensiveChecks(label, win, doc);
    win.close();
  };
  liveApplyChecks("downloads page", downloadsHtml, "https://pico.downloads/");
  liveApplyChecks("start page", startHtml, "https://pico.start/");
}

// ---------------------------------------------------------------------
// v23 Shield scriptlet layer: the REAL document script (built by
// dump_pages from the shipped lists) runs in the fixture page;
// the tests drive fetch / XMLHttpRequest / JSON.parse / the
// initial-data globals exactly as a YouTube page would.
// ---------------------------------------------------------------------
{
  const fixtureHtml = readFileSync(`${dir}/shield-fixture.html`, "utf8");
  const PLAYER = JSON.stringify({
    adPlacements: [{ adPlacementRenderer: { id: "pre" } }],
    adSlots: [],
    playerAds: [{ id: "pad" }],
    streamingData: { formats: [{ itag: 22, url: "https://video.example/v" }] },
    videoDetails: { videoId: "fixture1", title: "Fixture" },
  });
  const REEL = JSON.stringify({
    reelWatchSequenceResponse: {
      entries: [{ command: { reelWatchEndpoint: { adClientParams: { isAd: true, keep: 7 } } } }],
    },
    entries: [{ command: { reelWatchEndpoint: { adClientParams: { isAd: true } } } }],
    other: "intact",
  });
  const shieldDom = (url) =>
    new JSDOM(fixtureHtml, { url, runScripts: "dangerously", pretendToBeVisual: true });

  // (1) fetch: the trusted-replace-fetch rule renames
  // adPlacements out of the innertube player response before
  // the page's consumer parses it; content survives untouched.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__fetchBody = PLAYER;
    const resp = await w.fetch("https://www.youtube.com/youtubei/v1/player?key=k");
    const data = JSON.parse(await resp.text());
    check("shield fetch: adPlacements stripped from player response",
      data.adPlacements === undefined, JSON.stringify(Object.keys(data)));
    check("shield fetch: streamingData + videoDetails intact",
      data.streamingData.formats[0].itag === 22 && data.videoDetails.videoId === "fixture1");
    // Prune accounting reached the host channel.
    w.dispatchEvent(new w.Event("pagehide"));
    const pruneMsgs = w.__ipcSent.map((s) => { try { return JSON.parse(s); } catch { return null; } })
      .filter((m) => m && m.kind === "shield_prune");
    check("shield fetch: prune report posted over window.ipc",
      pruneMsgs.length >= 1 && pruneMsgs[0].items.some((i) => i.p.startsWith("replace:")),
      JSON.stringify(w.__ipcSent));
    w.close();
  }

  // (1b) fetch: json-prune-fetch-response deletes the reel ad
  // flags at BOTH compiled paths; siblings survive.
  {
    const dom = shieldDom("https://www.youtube.com/shorts/fixture");
    const w = dom.window;
    w.__fetchBody = REEL;
    const resp = await w.fetch("https://www.youtube.com/youtubei/v1/reel/reel_watch_sequence?x=1");
    const data = JSON.parse(await resp.text());
    const deep = data.reelWatchSequenceResponse.entries[0].command.reelWatchEndpoint.adClientParams;
    const top = data.entries[0].command.reelWatchEndpoint.adClientParams;
    check("shield fetch: reel isAd pruned at both paths",
      deep.isAd === undefined && top.isAd === undefined && deep.keep === 7 && data.other === "intact",
      JSON.stringify(data).slice(0, 200));
    w.close();
  }

  // (2) XMLHttpRequest: the trusted-replace-xhr rules cut the
  // adPlacements span out of the raw text before the page reads
  // responseText.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__xhrBody = PLAYER;
    const text = await new Promise((resolve, reject) => {
      const xhr = new w.XMLHttpRequest();
      xhr.open("POST", "https://www.youtube.com/youtubei/v1/player?key=k");
      xhr.addEventListener("load", () => resolve(xhr.responseText));
      xhr.addEventListener("error", reject);
      xhr.send();
    });
    const data = JSON.parse(text);
    check("shield xhr: adPlacements stripped from player response",
      data.adPlacements === undefined, text.slice(0, 160));
    check("shield xhr: streamingData intact",
      data.streamingData.formats[0].itag === 22);
    w.close();
  }

  // (2c) AUDIT V28 A2-M6: a page readystatechange handler
  // registered BEFORE send() must see the TRANSFORMED text —
  // the v27 listener-hook model let it read the raw body
  // (Pico's listener was registered inside send, after the
  // page's). The getter-first transform makes the first read
  // at readyState 4 the transformed one, whoever asks first.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__xhrBody = PLAYER;
    const seen = await new Promise((resolve, reject) => {
      const xhr = new w.XMLHttpRequest();
      let captured = null;
      xhr.addEventListener("readystatechange", () => {
        if (xhr.readyState === 4) captured = xhr.responseText;
      });
      xhr.open("POST", "https://www.youtube.com/youtubei/v1/player?key=k");
      xhr.addEventListener("load", () => resolve(captured));
      xhr.addEventListener("error", reject);
      xhr.send();
    });
    const data = JSON.parse(seen);
    check("v28 xhr ordering: pre-registered page handler sees pruned text",
      data.adPlacements === undefined && data.streamingData.formats[0].itag === 22,
      String(seen).slice(0, 120));
    w.close();
  }

  // (2d) AUDIT 0.30.1 (A2-C2): ONE XHR object reused for a
  // second, non-relevant request must serve the SECOND
  // response — through 0.30 the first request's getter gate
  // was never removed on open(), so the reused instance kept
  // answering reads with the first response's transformed
  // text (its cache is keyed to the first request's URL).
  // The discriminating assertion holds in every correct
  // world (raw second body, or a fresh gate's transform of
  // it): the second read carries the second body's marker
  // and none of the first body's content.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    const xhr = new w.XMLHttpRequest();
    const round = (url) => new Promise((resolve, reject) => {
      xhr.addEventListener("load", () => resolve(xhr.responseText), { once: true });
      xhr.addEventListener("error", reject);
      xhr.open("POST", url);
      xhr.send();
    });
    w.__xhrBody = PLAYER;
    const first = await round("https://www.youtube.com/youtubei/v1/player?key=k");
    check("0.30.1 xhr reuse: first request transformed as usual",
      JSON.parse(first).adPlacements === undefined &&
        JSON.parse(first).streamingData.formats[0].itag === 22,
      String(first).slice(0, 120));
    w.__xhrBody = JSON.stringify({ second: true, adPlacements: [{ id: "s" }] });
    const second = await round("https://www.youtube.com/robots.txt");
    check("0.30.1 xhr reuse: second request serves the second response, not the stale gate's cache",
      second.includes('"second":true') && !second.includes("streamingData") && second !== first,
      String(second).slice(0, 160));
    w.close();
  }

  // (2b) JSON.parse itself is pruned (the reel json-prune rule
  // applies to every parse on the host).
  {
    const dom = shieldDom("https://www.youtube.com/shorts/fixture");
    const w = dom.window;
    const data = w.JSON.parse(REEL);
    const top = data.entries[0].command.reelWatchEndpoint.adClientParams;
    check("shield json.parse: reel isAd pruned, payload otherwise intact",
      top.isAd === undefined && data.other === "intact");
    check("shield json.parse: wrapper actually installed on youtube",
      w.JSON.parse !== w.__nativeParse);
    w.close();
  }

  // (3) Initial data: assigning ytInitialPlayerResponse after
  // the payload installed has the ad properties stripped by
  // the `set` traps before any reader sees the object.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.ytInitialPlayerResponse = JSON.parse(PLAYER);
    const got = w.ytInitialPlayerResponse;
    check("shield trap: ytInitialPlayerResponse ad props stripped at assignment",
      got.adPlacements === undefined && got.playerAds === undefined && got.adSlots === undefined,
      JSON.stringify(Object.keys(got)));
    check("shield trap: streamingData survives the trap",
      got.streamingData.formats[0].itag === 22);
    w.close();
  }

  // (4) Negative controls: no youtube scriptlets on other
  // hosts; NOTHING installed on loopback (wrapper identities
  // are the exact stub functions; no counters object exists).
  {
    const dom = shieldDom("https://example.com/page");
    const w = dom.window;
    check("shield control: foreign host keeps native fetch + JSON.parse",
      w.fetch === w.__nativeFetch && w.JSON.parse === w.__nativeParse);
    w.ytInitialPlayerResponse = { adPlacements: [1] };
    check("shield control: foreign host has no initial-data trap",
      Array.isArray(w.ytInitialPlayerResponse.adPlacements));
    w.close();
  }
  {
    const dom = shieldDom("http://localhost:5173/");
    const w = dom.window;
    check("shield control: localhost fetch/XHR/JSON.parse untouched",
      w.fetch === w.__nativeFetch &&
      w.JSON.parse === w.__nativeParse &&
      w.XMLHttpRequest.prototype.open === w.__nativeXhrOpen &&
      w.XMLHttpRequest.prototype.send === w.__nativeXhrSend);
    check("shield control: localhost has no prune counters and no cosmetic style",
      w.__picoShieldPruneCounts === undefined && !w.document.getElementById("pico-shield-style"));
    w.close();
  }

  // (4b) Cosmetic half of the same document script still works
  // on the youtube host (one combined payload, both layers).
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    const st = w.document.getElementById("pico-shield-style");
    check("shield cosmetic: style element present on youtube with generic selectors",
      !!st && st.textContent.includes(".ad-banner"),
      st ? st.textContent.slice(0, 80) : "no style element");
    w.close();
  }

  // (7) AUDIT V26 (D6-a): the collapse layer — an image whose
  // load fails on a filtered page is collapsed with the
  // marker; the sweep catches already-broken images at load.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    const img = w.document.createElement("img");
    img.setAttribute("src", "https://ads.example/banner.png");
    // jsdom never settles image loads (complete stays false);
    // a real errored image is complete with naturalWidth 0 —
    // model exactly that, as the sweep check below already did.
    Object.defineProperty(img, "complete", { value: true });
    w.document.body.appendChild(img);
    img.dispatchEvent(new w.Event("error"));
    // AUDIT V28 A2-H2: the collapse is DEFERRED — an immediate
    // check must find the element still standing (a fallback
    // swap gets its chance first)…
    check("v28 collapse: no instant collapse on error",
      img.getAttribute("data-pico-collapsed") === null);
    await sleep(500);
    // …and a still-broken image collapses after revalidation.
    check("v26 collapse: image error collapses the element",
      img.getAttribute("data-pico-collapsed") === "1" && img.style.display === "none",
      `${img.getAttribute("data-pico-collapsed")} / ${img.style.display}`);
    const img2 = w.document.createElement("img");
    img2.setAttribute("src", "https://ads.example/late.png");
    Object.defineProperty(img2, "complete", { value: true });
    Object.defineProperty(img2, "naturalWidth", { value: 0 });
    w.document.body.appendChild(img2);
    w.dispatchEvent(new w.Event("load"));
    check("v26 collapse: load sweep collapses an already-broken image",
      img2.getAttribute("data-pico-collapsed") === "1",
      String(img2.getAttribute("data-pico-collapsed")));
    w.close();
  }

  // (7b) AUDIT V28 A2-H2: <source> errors are candidate
  // fallthrough (never a collapse), and an <img> whose src is
  // swapped by a fallback handler before revalidation is
  // rescued, not collapsed.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    const pic = w.document.createElement("picture");
    const src = w.document.createElement("source");
    src.setAttribute("srcset", "https://ads.example/dead.webp");
    const img = w.document.createElement("img");
    img.setAttribute("src", "https://cdn.example/real.png");
    // A healthy image for sweep purposes: only the <source>
    // error is under test here.
    Object.defineProperty(img, "naturalWidth", { value: 640 });
    pic.appendChild(src);
    pic.appendChild(img);
    w.document.body.appendChild(pic);
    src.dispatchEvent(new w.Event("error"));
    await sleep(500);
    check("v28 collapse: source error never collapses the picture",
      pic.getAttribute("data-pico-collapsed") === null &&
      img.getAttribute("data-pico-collapsed") === null);
    const img2 = w.document.createElement("img");
    img2.setAttribute("src", "https://ads.example/dead.png");
    Object.defineProperty(img2, "naturalWidth", { value: 640 });
    w.document.body.appendChild(img2);
    img2.dispatchEvent(new w.Event("error"));
    img2.setAttribute("src", "https://cdn.example/fallback.png");
    await sleep(500);
    check("v28 collapse: a fallback src swap rescues the image",
      img2.getAttribute("data-pico-collapsed") === null && img2.style.display === "",
      `${img2.getAttribute("data-pico-collapsed")} / ${img2.style.display}`);
    w.close();
  }

  // (8) Both filters OFF: the collapse layer is disarmed by
  // its flag — the same error leaves the element untouched.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__picoCollapseOff = 1;
    const img = w.document.createElement("img");
    img.setAttribute("src", "https://ads.example/banner.png");
    w.document.body.appendChild(img);
    img.dispatchEvent(new w.Event("error"));
    check("v26 collapse: disarmed when both filters are off",
      img.getAttribute("data-pico-collapsed") === null && img.style.display === "",
      `${img.getAttribute("data-pico-collapsed")} / ${img.style.display}`);
    w.close();
  }

  // (9) Lite-only dimension (D6-d): the Shield layer disarmed
  // (its OFF flag) while the collapse layer stays armed — an
  // image error still collapses. This is the state a Lite-on /
  // Shield-off page's per-load collapse push produces.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__picoShieldOff = true;
    const img = w.document.createElement("img");
    img.setAttribute("src", "https://cdn.example/photo.png");
    Object.defineProperty(img, "complete", { value: true });
    w.document.body.appendChild(img);
    img.dispatchEvent(new w.Event("error"));
    await sleep(500);
    check("v26 collapse: armed with Shield off + Lite on",
      img.getAttribute("data-pico-collapsed") === "1");
    w.close();
  }

  // (10) Loopback immunity: the runtime refuses to install on
  // a local page at all — image errors stay untouched.
  {
    const dom = shieldDom("http://127.0.0.2:8080/app");
    const w = dom.window;
    const img = w.document.createElement("img");
    img.setAttribute("src", "http://127.0.0.2:8080/pic.png");
    w.document.body.appendChild(img);
    img.dispatchEvent(new w.Event("error"));
    check("v26 collapse: loopback page is immune",
      img.getAttribute("data-pico-collapsed") === null);
    w.close();
  }

  // (11) The scriptlet layer is undisturbed by the collapse
  // segment riding in the same document script: the innertube
  // prune still fires with the collapse layer armed.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__fetchBody = PLAYER;
    const resp = await w.fetch("https://www.youtube.com/youtubei/v1/player?key=k");
    const data = JSON.parse(await resp.text());
    check("v26 interplay: innertube prune intact alongside collapse",
      data.adPlacements === undefined && data.streamingData.formats[0].itag === 22);
    w.close();
  }

  // (5) OFF state: the app's atomic in-page flag disarms the
  // wrappers — the same fetch returns the payload byte-intact.
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__picoShieldOff = true;
    w.__fetchBody = PLAYER;
    const resp = await w.fetch("https://www.youtube.com/youtubei/v1/player?key=k");
    check("shield off: player response passes through byte-identical",
      (await resp.text()) === PLAYER);
    w.close();
  }

  // (6) Fail-open: a malformed JSON body on a matching URL is
  // delivered byte-identical (never an exception, never a
  // half-transformed response).
  {
    const dom = shieldDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    w.__fetchBody = "this is not json{{{";
    const resp = await w.fetch("https://www.youtube.com/youtubei/v1/player?key=k");
    check("shield fail-open: malformed body passes through byte-identical",
      (await resp.text()) === "this is not json{{{");
    w.close();
  }
}


// ---------------------------------------------------------------------
// Lite play gate (v28 hover fix, REBUILT as v29's chosen-video
// model): the REAL gate runtime over a stubbed media
// prototype. The v29 matrix: hover refused; a tapped element
// is chosen and its repeats pass; once chosen, every other
// element stays refused (even with its own gesture); a fresh
// watch-shaped load lets its main video play with NO gesture
// while the sidebar preview dies; a tap followed by an SPA
// route change crowns the next large video and the old
// designation is gone after the next route change; the OFF
// flag passes through; restore() puts the exact original
// back; loopback never installs; and the gate coexists with
// the Shield's innertube prune (orthogonal surfaces).
// ---------------------------------------------------------------------
{
  const liteHtml = readFileSync(`${dir}/lite-fixture.html`, "utf8");
  const gateJs = readFileSync(`${dir}/lite-gate.js`, "utf8");
  const liteDom = (url) =>
    new JSDOM(liteHtml, { url, runScripts: "dangerously", pretendToBeVisual: true });

  {
    const dom = liteDom("https://www.youtube.com/");
    const w = dom.window;
    const proto = w.HTMLMediaElement.prototype;
    check("v28 lite gate: installs on a public page",
      !!w.__picoLiteGate && proto.play !== w.__origPlay);
    const v1 = w.document.getElementById("v1");
    let err = null;
    await v1.play().catch((e) => { err = e; });
    check("v28 lite gate: play() with no gesture is refused",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 0,
      err ? String(err.name) : "no rejection");
    // Hover-family events must NOT create intent.
    v1.dispatchEvent(new w.MouseEvent("mouseover", { bubbles: true }));
    v1.dispatchEvent(new w.MouseEvent("mouseenter", { bubbles: true }));
    v1.dispatchEvent(new w.MouseEvent("mousemove", { bubbles: true }));
    err = null;
    await v1.play().catch((e) => { err = e; });
    check("v28 lite gate: hover is not intent (still refused)",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 0);
    // AUDIT V29 (A2-F7): a gesture on an UNRELATED element
    // must not bless this one. v28's gate also passed any
    // element while the document held transient activation —
    // click anything (a Like button, a cookie banner) and a
    // programmatic preview could play inside the window.
    const stray = w.document.getElementById("sidebar");
    stray.dispatchEvent(new w.MouseEvent("pointerdown", { bubbles: true }));
    err = null;
    await v1.play().catch((e) => { err = e; });
    check("v29 lite gate: an unrelated click does not bless another element",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 0);
    // A genuine gesture on the element designates it CHOSEN
    // (the network layer's business is separate, pinned in
    // Rust): its play() passes, and so do its own repeats —
    // real players retry play() on the chosen element.
    const v2 = w.document.getElementById("v2");
    v2.dispatchEvent(new w.MouseEvent("pointerdown", { bubbles: true }));
    await v2.play();
    check("v29 lite gate: play() after pointerdown passes the gate",
      w.__playCalls === 1, String(w.__playCalls));
    await v2.play();
    check("v29 lite gate: the chosen element's repeated play() keeps passing",
      w.__playCalls === 2, String(w.__playCalls));
    check("v29 lite gate: designation reported exactly once (lite_chosen)",
      w.__ipcSent.filter((s) => s.includes("lite_chosen")).length === 1,
      JSON.stringify(w.__ipcSent));
    // Once chosen, another element stays refused — even with
    // a genuine gesture of its own on its container (through
    // v28 this passed; the chosen model overrules it).
    const v3 = w.document.getElementById("v3");
    w.document.getElementById("wrap").dispatchEvent(new w.MouseEvent("pointerdown", { bubbles: true }));
    err = null;
    await v3.play().catch((e) => { err = e; });
    check("v29 lite gate: once chosen, another element stays refused",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 2);
    // OFF flag: byte-exact passthrough to the original.
    w.__picoLiteGateOff = 1;
    await v1.play();
    check("v28 lite gate: the OFF flag passes through to the original",
      w.__playCalls === 3, String(w.__playCalls));
    // Re-evaluating the runtime re-arms instead of double-wrapping.
    w.__picoLiteGateOff = 0;
    const wrapped = proto.play;
    w.eval(gateJs);
    check("v28 lite gate: a second install does not double-wrap",
      proto.play === wrapped && !!w.__picoLiteGate);
    err = null;
    const vFresh = w.document.createElement("video");
    w.document.body.appendChild(vFresh);
    await vFresh.play().catch((e) => { err = e; });
    check("v28 lite gate: still gating after re-arm",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 3);
    // restore(): the exact original method returns.
    w.__picoLiteGate.restore();
    check("v28 lite gate: restore() returns the exact original play()",
      proto.play === w.__origPlay && w.__picoLiteGate === undefined);
    await v1.play();
    check("v28 lite gate: play() after restore is the page's own",
      w.__playCalls === 4, String(w.__playCalls));
    w.close();
  }

  // A container gesture designates the media inside it (its
  // own fresh page: designation is one-per-context).
  {
    const dom = liteDom("https://news.example/story");
    const w = dom.window;
    const v3 = w.document.getElementById("v3");
    w.document.getElementById("wrap").dispatchEvent(new w.MouseEvent("pointerdown", { bubbles: true }));
    await v3.play();
    check("v29 lite gate: a container gesture designates its media",
      w.__playCalls === 1, String(w.__playCalls));
    w.close();
  }

  // THE v29 WITNESS — a fresh watch-shaped load (the
  // paste-a-watch-link case): the main video plays with NO
  // gesture at all, its repeats pass, the report fires once,
  // and the sidebar preview is refused. (jsdom gives media no
  // layout, so the main element stands in via the gate's
  // first-video-on-a-watch-page fallback — the large-size
  // path is exercised by the route-change block below.)
  {
    const dom = liteDom("https://www.youtube.com/watch?v=fixture1");
    const w = dom.window;
    check("v29 lite gate: installs on a watch page", !!w.__picoLiteGate);
    const main = w.document.getElementById("v1");
    await main.play();
    check("v29 witness: the chosen (main) video PLAYS on a watch page under Lite",
      w.__playCalls === 1, String(w.__playCalls));
    await main.play();
    check("v29 witness: the main video's retry play() passes too",
      w.__playCalls === 2, String(w.__playCalls));
    check("v29 witness: the watch designation was reported once",
      w.__ipcSent.filter((s) => s.includes("lite_chosen")).length === 1,
      JSON.stringify(w.__ipcSent));
    const side = w.document.getElementById("sidebar");
    side.dispatchEvent(new w.MouseEvent("mouseover", { bubbles: true }));
    let err = null;
    await side.play().catch((e) => { err = e; });
    check("v29 witness: the sidebar preview stays refused on the watch page",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 2);
    w.close();
  }

  // The thumbnail-tap case: a pointerdown anywhere, then an
  // SPA route change — the next LARGE video to ask is chosen;
  // navigating on retires the designation.
  {
    const dom = liteDom("https://www.youtube.com/");
    const w = dom.window;
    const card = w.document.getElementById("v2");
    card.dispatchEvent(new w.MouseEvent("pointerdown", { bubbles: true }));
    w.history.pushState({}, "", "/watch?v=route1");
    const main = w.document.getElementById("v1");
    main.getBoundingClientRect = () => ({
      width: 640, height: 360, top: 0, left: 0, right: 640, bottom: 360, x: 0, y: 0,
    });
    await main.play();
    check("v29 lite gate: tap + route change crowns the next large video",
      w.__playCalls === 1, String(w.__playCalls));
    let err = null;
    await card.play().catch((e) => { err = e; });
    check("v29 lite gate: the tapped card itself is not the chosen video",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 1);
    // Route on: the designation is retired. The main element
    // loses its layout size (back on the home grid it is just
    // another unchosen element) and its play() is refused.
    w.history.pushState({}, "", "/");
    main.getBoundingClientRect = () => ({
      width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0, x: 0, y: 0,
    });
    err = null;
    await main.play().catch((e) => { err = e; });
    check("v29 lite gate: a route change retires the designation",
      !!err && err.name === "NotAllowedError" && w.__playCalls === 1);
    w.close();
  }

  // Loopback: the runtime returns before touching the prototype.
  {
    const dom = liteDom("http://127.0.0.1:8080/dev");
    const w = dom.window;
    check("v28 lite gate: loopback page is exempt (prototype untouched)",
      w.__picoLiteGate === undefined &&
      w.HTMLMediaElement.prototype.play === w.__origPlay);
    w.close();
  }

  // Orthogonality: gate + Shield document script on one page —
  // the innertube prune still fires AND the gate still gates.
  {
    const fixtureHtml = readFileSync(`${dir}/shield-fixture.html`, "utf8");
    const dom = new JSDOM(fixtureHtml, {
      url: "https://www.youtube.com/watch?v=fixture1",
      runScripts: "dangerously", pretendToBeVisual: true,
    });
    const w = dom.window;
    w.eval(gateJs);
    check("v28 lite gate: installs alongside the Shield payload",
      !!w.__picoLiteGate);
    const PLAYER = JSON.stringify({
      adPlacements: [{ adPlacementRenderer: { id: "pre" } }],
      streamingData: { formats: [{ itag: 22, url: "https://video.example/v" }] },
      videoDetails: { videoId: "fixture1", title: "Fixture" },
    });
    w.__fetchBody = PLAYER;
    const resp = await w.fetch("https://www.youtube.com/youtubei/v1/player?key=k");
    const data = JSON.parse(await resp.text());
    check("v28 interplay: innertube prune stays green with Lite ON",
      data.adPlacements === undefined && data.streamingData.formats[0].itag === 22);
    // AUDIT V29: on this watch page the Shield payload's own
    // media stub is NOT installed (the shield fixture predates
    // the gate), so assert the gate's half with the DOM's
    // native stub: play() must not be REFUSED by the gate —
    // jsdom's native play() is a not-implemented stub that
    // returns undefined rather than a NotAllowedError
    // rejection, so a resolved/undefined result is the pass
    // signal; a gate refusal would reject NotAllowedError.
    const vid = w.document.createElement("video");
    w.document.body.appendChild(vid);
    let refused = null;
    try {
      const r = vid.play();
      if (r && typeof r.catch === "function") await r.catch((e) => { refused = e; });
    } catch (e) { refused = e; }
    check("v29 interplay: the watch page's main video is not gate-refused (Shield + Lite)",
      !refused || refused.name !== "NotAllowedError",
      refused ? String(refused.name) : "passed");
    check("v29 interplay: the designation reached the app channel",
      w.__ipcSent.some((s) => s.includes("lite_chosen")),
      JSON.stringify(w.__ipcSent));
    w.close();
  }
}

// ---------------------------------------------------------------------
// Lite gate 0.31 (A2): the NEW route / marking behaviors, written
// to spec ahead of the Rust-side runtime change — keyboard and
// popstate routes into a watch page, same-video route churn,
// and markTree's multi-media container rule. Fixtures follow
// the v29 gate pattern above (the REAL runtime over the stubbed
// media prototype, one fresh page per scenario). Against the
// pre-0.31 runtime some of these FAIL by design; the
// coordinator re-runs this suite once the runtime lands.
// ---------------------------------------------------------------------
{
  const liteHtml = readFileSync(`${dir}/lite-fixture.html`, "utf8");
  const liteDom = (url) =>
    new JSDOM(liteHtml, { url, runScripts: "dangerously", pretendToBeVisual: true });
  const makeLarge = (el) => {
    el.getBoundingClientRect = () => ({
      width: 640, height: 360, top: 0, left: 0, right: 640, bottom: 360, x: 0, y: 0,
    });
  };
  const refusedName = async (el) => {
    let err = null;
    await el.play().catch((e) => { err = e; });
    return err ? err.name : null;
  };

  // (a) Keyboard route: a keydown (NO pointerdown) is a real
  // user gesture — a route change to a watch URL on its heels
  // carries route intent, and the large video may play.
  {
    const dom = liteDom("https://www.youtube.com/");
    const w = dom.window;
    check("0.31 gate (a): installs for the keyboard-route scenario", !!w.__picoLiteGate);
    w.document.body.dispatchEvent(
      new w.KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    w.history.pushState({}, "", "/watch?v=kb1");
    const main = w.document.getElementById("v1");
    makeLarge(main);
    await main.play();
    check("0.31 gate (a): keydown + route to /watch lets the large video play",
      w.__playCalls === 1, String(w.__playCalls));
    w.close();
  }

  // (b) Popstate route: browser traversal lands on a watch
  // URL (the URL changes first, the popstate event follows —
  // simulated here by moving the URL through the gate's own
  // unwrapped pushState, then dispatching popstate). The
  // watch page's large video may play with no gesture at all.
  {
    const dom = liteDom("https://www.youtube.com/");
    const w = dom.window;
    check("0.31 gate (b): installs for the popstate scenario", !!w.__picoLiteGate);
    const rawPush = w.history.pushState.__picoLiteOriginal || w.history.pushState;
    rawPush.call(w.history, {}, "", "/watch?v=pop1");
    w.dispatchEvent(new w.Event("popstate"));
    const main = w.document.getElementById("v1");
    makeLarge(main);
    await main.play();
    check("0.31 gate (b): popstate to a watch URL lets the large video play",
      w.__playCalls === 1, String(w.__playCalls));
    w.close();
  }

  // (c) Same-video churn: on a watch page the chosen element
  // keeps its designation when the route changes only in a
  // presentation parameter (&t=42 — same video, new timestamp);
  // its repeated play() stays allowed.
  {
    const dom = liteDom("https://www.youtube.com/watch?v=churn1");
    const w = dom.window;
    const main = w.document.getElementById("v1");
    await main.play();
    check("0.31 gate (c): the watch page's main video is chosen first",
      w.__playCalls === 1, String(w.__playCalls));
    w.history.pushState({}, "", "/watch?v=churn1&t=42");
    await main.play();
    check("0.31 gate (c): designation survives same-video churn (&t=42)",
      w.__playCalls === 2, String(w.__playCalls));
    w.close();
  }

  // (d) markTree, part 1: a container holding TWO media
  // elements is not a single player — clicking its non-media
  // child (the headline) blesses NEITHER element.
  {
    const dom = liteDom("https://news.example/story");
    const w = dom.window;
    const box = w.document.createElement("div");
    const head = w.document.createElement("span");
    head.textContent = "Headline";
    const mA = w.document.createElement("video");
    const mB = w.document.createElement("video");
    box.appendChild(head); box.appendChild(mA); box.appendChild(mB);
    w.document.body.appendChild(box);
    head.dispatchEvent(new w.MouseEvent("pointerdown", { bubbles: true }));
    const errA = await refusedName(mA);
    const errB = await refusedName(mB);
    check("0.31 gate (d): a headline click in a two-media container blesses neither",
      errA === "NotAllowedError" && errB === "NotAllowedError" && w.__playCalls === 0,
      `${errA}/${errB} calls=${w.__playCalls}`);
    w.close();
  }

  // (d) markTree, part 2: clicking directly ON one media
  // element of the pair blesses THAT one only — the sibling
  // is refused even when it asks first, and the clicked
  // element then plays.
  {
    const dom = liteDom("https://news.example/story");
    const w = dom.window;
    const box = w.document.createElement("div");
    const head = w.document.createElement("span");
    head.textContent = "Headline";
    const mA = w.document.createElement("video");
    const mB = w.document.createElement("video");
    box.appendChild(head); box.appendChild(mA); box.appendChild(mB);
    w.document.body.appendChild(box);
    mB.dispatchEvent(new w.MouseEvent("pointerdown", { bubbles: true }));
    const errA = await refusedName(mA);
    check("0.31 gate (d): the unclicked sibling stays refused",
      errA === "NotAllowedError" && w.__playCalls === 0,
      `${errA} calls=${w.__playCalls}`);
    await mB.play();
    check("0.31 gate (d): the directly clicked element plays",
      w.__playCalls === 1, String(w.__playCalls));
    w.close();
  }
}

// ---------------------------------------------------------------------
// v28 lab fixture: a tiny list through the REAL engine — scoped
// cosmetic exceptions against generic selectors (A2-M4),
// entity-scoped cosmetics (A2-M3), the `set` scriptlet's pin
// yielding while the Shield is off (A2-H1), and generichide.
// ---------------------------------------------------------------------
{
  const labHtml = readFileSync(`${dir}/lab-fixture.html`, "utf8");
  const labDom = (url) =>
    new JSDOM(labHtml, { url, runScripts: "dangerously", pretendToBeVisual: true });
  const display = (w, sel) =>
    w.getComputedStyle(w.document.querySelector(sel)).display;

  {
    const dom = labDom("https://example.com/page");
    const w = dom.window;
    check("v28 lab: a scoped exception un-hides the generic selector on its host",
      display(w, ".ad-generic") !== "none", display(w, ".ad-generic"));
    check("v28 lab: the entity-scoped cosmetic hides on the entity host",
      display(w, ".ent-ad") === "none", display(w, ".ent-ad"));
    check("v28 lab: the set scriptlet pins while the Shield is on",
      w.picoTest && w.picoTest.pin === 123,
      w.picoTest ? String(w.picoTest.pin) : "no picoTest");
    w.__picoShieldOff = 1;
    check("v28 lab: the pin yields while the Shield is off",
      w.picoTest.pin === undefined, String(w.picoTest.pin));
    w.picoTest.pin = 7;
    check("v28 lab: assignments land (and read back) while off",
      w.picoTest.pin === 7, String(w.picoTest.pin));
    w.__picoShieldOff = 0;
    check("v28 lab: the pin returns when the Shield comes back",
      w.picoTest.pin === 123, String(w.picoTest.pin));
    w.close();
  }
  {
    const dom = labDom("https://www.example.com/page");
    const w = dom.window;
    check("v28 lab: the entity cosmetic matches subdomains via the entity key",
      display(w, ".ent-ad") === "none", display(w, ".ent-ad"));
    w.close();
  }
  {
    const dom = labDom("https://other.org/page");
    const w = dom.window;
    check("v28 lab: the generic cosmetic hides where no exception scopes",
      display(w, ".ad-generic") === "none", display(w, ".ad-generic"));
    check("v28 lab: the entity cosmetic does not leak to foreign hosts",
      display(w, ".ent-ad") !== "none", display(w, ".ent-ad"));
    check("v28 lab: the set scriptlet does not run on foreign hosts",
      !!w.picoTest && w.picoTest.pin === undefined,
      w.picoTest ? String(w.picoTest.pin) : "stub object missing");
    w.close();
  }
  {
    const dom = labDom("https://gh.example/page");
    const w = dom.window;
    check("v28 lab: generichide suppresses generic cosmetics on its host",
      display(w, ".ad-generic") !== "none", display(w, ".ad-generic"));
    w.close();
  }
  // 0.30: the measured prune report. The lab list's
  // json-prune rule deletes the adBanner subtree from every
  // JSON.parse on example.com; the runtime must measure the
  // subtree's EXACT serialized size — UTF-8 bytes, not chars
  // (the € pair proves it) — and flush it as
  // {kind:"shield_pruned"} on the shared throttle.
  {
    const dom = labDom("https://example.com/page");
    const w = dom.window;
    const payload = JSON.stringify({ adBanner: { x: "€€" }, keep: 1 });
    const expected = Buffer.byteLength(JSON.stringify({ x: "€€" }), "utf8");
    check("0.30 lab: the € subtree is 14 UTF-8 bytes (fixture sanity)",
      expected === 14, String(expected));
    const data = w.JSON.parse(payload);
    check("0.30 lab: adBanner pruned, sibling intact",
      data.adBanner === undefined && data.keep === 1, JSON.stringify(data));
    check("0.30 lab: the accumulator holds the exact measurement",
      !!w.__picoShieldPruned && w.__picoShieldPruned.bytes === expected &&
      w.__picoShieldPruned.events === 1,
      JSON.stringify(w.__picoShieldPruned));
    w.dispatchEvent(new w.Event("pagehide"));
    const measured = w.__ipcSent.map((s) => { try { return JSON.parse(s); } catch { return null; } })
      .filter((m) => m && m.kind === "shield_pruned");
    check("0.30 lab: shield_pruned report posted with the exact bytes",
      measured.length === 1 && measured[0].bytes === expected && measured[0].events === 1,
      JSON.stringify(w.__ipcSent));
    check("0.30 lab: the accumulator drains after the flush",
      w.__picoShieldPruned.bytes === 0 && w.__picoShieldPruned.events === 0,
      JSON.stringify(w.__picoShieldPruned));
    w.close();
  }
}

// ------------------------------------------------------------------
// Chrome 0.34: the Downloads panel on the shared DlRows
// renderer. The chrome snapshot carries THE downloads
// snapshot in `downloads` ({tasks, active_count,
// failed_unseen, settings, notice}); rows are
// DlRows.renderTaskList(snapshot, "popover") strings, so a
// progress snapshot re-renders the list (no element reuse —
// the 0.32 keyed-reconciliation checks became content
// checks); every control posts the canonical
// {"kind":"dl","action":…} body through the chrome
// transport (send adds v/boot/req framing, asserted loosely
// here and exactly on the downloads page, whose transport
// adds nothing).
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  const dlMsgs = (a) => sent.filter((m) => m.kind === "dl" && m.action === a);
  const TASKS = [
    { id: 11, name: "big-file.zip", host: "files.example.com", url: "https://files.example.com/big-file.zip", status: "downloading", total: 100000000, downloaded: 45000000, percent: 45, size_text: "100 MB", detail_text: "45 MB of 100 MB · 1.2 MB/s · 45 s left", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730000000000 },
    { id: 12, name: "paper.pdf", host: "docs.example.org", url: "https://docs.example.org/paper.pdf", status: "paused", total: 5000000, downloaded: 1000000, percent: 20, size_text: "5 MB", detail_text: "1 MB of 5 MB · Paused", connections: 4, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730003600000 },
    { id: 13, name: "song.flac", host: "music.example.net", url: "https://music.example.net/song.flac", status: "done", total: 30000000, downloaded: 30000000, percent: 100, size_text: "30 MB", detail_text: "30 MB", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730007200000 },
    { id: 14, name: "broken.iso", host: "bad.example.com", url: "https://bad.example.com/broken.iso", status: "failed", total: 200000000, downloaded: 60000000, percent: 30, size_text: "200 MB", detail_text: "60 MB of 200 MB · Failed", connections: 2, error: "HTTP 404", hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730010800000 },
  ];
  const dlSnap = (tasks, over = {}) => ({
    tasks, active_count: 1, failed_unseen: 0,
    settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 },
    notice: null, ...over,
  });
  const baseSnap = {
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, focusAddr: false,
    palette: "forest", themes: [], paletteOpen: false,
    teleportOpen: false, teleport: null,
    downloadsOpen: false, downloads: dlSnap([]),
  };
  const push = (over) => window.__pico({ ...baseSnap, ...over });
  const dlBtn = doc.getElementById("downloads");
  const panel = doc.getElementById("dpanel");
  const badge = doc.getElementById("dl-count");
  const list = doc.getElementById("dl-list");
  const rowOf = (id) => list.querySelector(`[data-dl-id="${id}"]`);

  push({ downloads: dlSnap([], { active_count: 0 }) });
  check("0.34 downloads: button present in the navbar",
    !!dlBtn && !!dlBtn.closest("#navbar"));
  check("0.34 downloads: button stands before the teleport button",
    (dlBtn.compareDocumentPosition(doc.getElementById("teleport")) &
      window.Node.DOCUMENT_POSITION_FOLLOWING) !== 0);
  check("0.34 downloads: panel starts closed",
    !panel.classList.contains("on"));
  check("0.34 downloads: no backdrop element exists (the popover dims nothing)",
    doc.getElementById("dp-backdrop") === null);
  check("0.34 downloads: badge hidden while nothing is active",
    !badge.classList.contains("on") && badge.textContent === "");

  push({ downloadsOpen: true, downloads: dlSnap(TASKS) });
  check("0.34 downloads: panel opens on the snapshot",
    panel.classList.contains("on"));
  check("0.34 downloads: button reports expanded",
    dlBtn.getAttribute("aria-expanded") === "true");
  const rows = list.querySelectorAll(".dl-row");
  check("0.34 downloads: four rows render", rows.length === 4, `got ${rows.length}`);
  check("0.34 downloads: rows stand in snapshot order",
    JSON.stringify([...rows].map((r) => r.dataset.dlId)) === JSON.stringify(["11", "12", "13", "14"]),
    [...rows].map((r) => r.dataset.dlId).join(","));
  let r11 = rowOf(11), r12 = rowOf(12), r13 = rowOf(13), r14 = rowOf(14);
  check("0.34 downloads: name + status word render as exact text",
    r11.querySelector(".dl-name").textContent === "big-file.zip" &&
    r11.querySelector(".dl-status").textContent === "downloading",
    r11.querySelector(".dl-name").textContent);
  check("0.35 downloads: the host leaves the popover row (the page keeps it in meta)",
    r11.querySelector(".dl-host") === null && !r11.textContent.includes("files.example.com"),
    r11.textContent);
  check("0.34 downloads: detail line is the preformatted string, verbatim",
    r11.querySelector(".dl-prog-text").textContent === "45 MB of 100 MB · 1.2 MB/s · 45 s left",
    r11.querySelector(".dl-prog-text").textContent);
  check("0.34 downloads: bar width is the snapshot percent",
    r11.querySelector(".dl-fill").style.width === "45%",
    r11.querySelector(".dl-fill").style.width);
  check("0.34 downloads: conn select carries the task's count",
    r11.querySelector(".dl-conn").value === "8" &&
    r12.querySelector(".dl-conn").value === "4" &&
    r14.querySelector(".dl-conn").value === "2");
  check("0.35 downloads: downloading row offers Pause (data-dl-action), no Folder (no placeholders)",
    r11.querySelector(".dl-act").getAttribute("aria-label") === "Pause" &&
    r11.querySelector(".dl-act").getAttribute("data-dl-action") === "pause" &&
    r11.querySelector(".dl-folder") === null &&
    r11.querySelector(".dl-open-file") === null &&
    r11.querySelector(".dl-cancel").title === "Cancel" &&
    r11.querySelector(".dl-cancel").getAttribute("data-dl-action") === "cancel");
  check("0.34 downloads: paused row offers Resume",
    r12.querySelector(".dl-act").getAttribute("aria-label") === "Resume" &&
    r12.querySelector(".dl-act").getAttribute("data-dl-action") === "resume" &&
    r12.querySelector(".dl-fill").style.width === "20%");
  check("0.35 downloads: done row has no Pause placeholder, shows Folder, x removes",
    r13.querySelector(".dl-act") === null &&
    !!r13.querySelector(".dl-folder") &&
    r13.querySelector(".dl-folder").getAttribute("aria-label") === "Open folder" &&
    r13.querySelector(".dl-cancel").title === "Remove from list" &&
    r13.querySelector(".dl-cancel").getAttribute("data-dl-action") === "remove" &&
    r13.querySelector(".dl-fill").style.width === "100%");
  check("0.34 downloads: failed row shows its error in the detail area",
    r14.querySelector(".dl-error").textContent === "HTTP 404" &&
    r14.querySelector(".dl-error").style.display !== "none",
    r14.querySelector(".dl-error").textContent);
  check("0.34 downloads: failed row offers Resume",
    r14.querySelector(".dl-act").getAttribute("aria-label") === "Resume");
  check("0.35 downloads: a row exposes the .dl-prog line with progress + conn, buttons on line 1",
    !!r11.querySelector(".dl-prog") &&
    !!r11.querySelector(".dl-prog .dl-prog-text") &&
    !!r11.querySelector(".dl-prog .dl-conn") &&
    !!r11.querySelector(".dl-name-line .dl-act") &&
    r11.querySelector(".dl-prog .dl-act") === null);
  check("0.34 downloads: healthy rows show no error line",
    r11.querySelector(".dl-error").style.display === "none");
  check("0.34 downloads: done row's conn select is disabled, live rows' are not",
    r13.querySelector(".dl-conn").disabled === true &&
    r11.querySelector(".dl-conn").disabled === false &&
    r12.querySelector(".dl-conn").disabled === false);
  check("0.34 downloads: default-conn select mirrors the snapshot",
    doc.getElementById("dl-default-conn").value === "8",
    doc.getElementById("dl-default-conn").value);
  check("0.34 downloads: badge shows the active count",
    badge.classList.contains("on") && badge.textContent === "1",
    badge.textContent);
  check("0.34 downloads: badge title counts downloads in progress",
    badge.title === "1 download in progress", badge.title);
  check("0.34 downloads: empty state hidden while rows exist",
    !doc.getElementById("dl-empty").classList.contains("on"));

  // Progress snapshot: the re-rendered row's bar + detail advance.
  const advanced = TASKS.map((t) => t.id === 11
    ? { ...t, percent: 60, downloaded: 60000000, detail_text: "60 MB of 100 MB · 1.4 MB/s · 29 s left" }
    : t);
  push({ downloadsOpen: true, downloads: dlSnap(advanced) });
  r11 = rowOf(11);
  check("0.34 downloads: a progress snapshot advances the row's bar + detail",
    r11.querySelector(".dl-fill").style.width === "60%" &&
    r11.querySelector(".dl-prog-text").textContent === "60 MB of 100 MB · 1.4 MB/s · 29 s left",
    r11.querySelector(".dl-prog-text").textContent);
  check("0.34 downloads: badge still shows 1 while the task downloads",
    badge.textContent === "1" && badge.title === "1 download in progress",
    badge.title);

  // Focus discipline: a focused conn select keeps its value
  // (restored across the re-render) while the bar refreshes;
  // after blur the value syncs again.
  let conn11 = r11.querySelector(".dl-conn");
  conn11.focus();
  const reconn = advanced.map((t) => t.id === 11 ? { ...t, connections: 32, percent: 65 } : t);
  push({ downloadsOpen: true, downloads: dlSnap(reconn) });
  conn11 = rowOf(11).querySelector(".dl-conn");
  check("0.34 downloads: a focused conn select is never revalued",
    conn11.value === "8", conn11.value);
  check("0.34 downloads: the bar still refreshes around the focused select",
    rowOf(11).querySelector(".dl-fill").style.width === "65%");
  conn11.blur();
  push({ downloadsOpen: true, downloads: dlSnap(reconn) });
  check("0.34 downloads: the conn select syncs again after blur",
    rowOf(11).querySelector(".dl-conn").value === "32",
    rowOf(11).querySelector(".dl-conn").value);
  const addUrl = doc.getElementById("dl-add-url");
  addUrl.focus();
  addUrl.value = "https://typing.example/draft";
  push({ downloadsOpen: true, downloads: dlSnap(reconn) });
  check("0.34 downloads: a snapshot never revalues the focused add-url input",
    addUrl.value === "https://typing.example/draft", addUrl.value);
  addUrl.value = "";
  addUrl.blur();

  // Canonical IPC from every control.
  rowOf(11).querySelector(".dl-act").click();
  check("0.34 downloads: Pause posts {kind:dl, action:pause, id}",
    dlMsgs("pause").length === 1 && dlMsgs("pause")[0].id === 11,
    JSON.stringify(dlMsgs("pause")));
  rowOf(12).querySelector(".dl-act").click();
  rowOf(14).querySelector(".dl-act").click();
  check("0.34 downloads: Resume posts resume for paused and failed rows",
    dlMsgs("resume").length === 2 &&
    dlMsgs("resume")[0].id === 12 && dlMsgs("resume")[1].id === 14,
    JSON.stringify(dlMsgs("resume")));
  rowOf(11).querySelector(".dl-cancel").click();
  check("0.34 downloads: x on a live row posts cancel",
    dlMsgs("cancel").length === 1 && dlMsgs("cancel")[0].id === 11,
    JSON.stringify(dlMsgs("cancel")));
  rowOf(13).querySelector(".dl-cancel").click();
  check("0.34 downloads: x on a done row posts remove",
    dlMsgs("remove").length === 1 && dlMsgs("remove")[0].id === 13,
    JSON.stringify(dlMsgs("remove")));
  const connSel = rowOf(11).querySelector(".dl-conn");
  connSel.value = "16";
  connSel.dispatchEvent(new window.Event("change", { bubbles: true }));
  check("0.34 downloads: conn change posts set-connections {id, conn}",
    dlMsgs("set-connections").length === 1 &&
    dlMsgs("set-connections")[0].id === 11 &&
    dlMsgs("set-connections")[0].conn === 16,
    JSON.stringify(dlMsgs("set-connections")));
  const defSel = doc.getElementById("dl-default-conn");
  defSel.value = "4";
  defSel.dispatchEvent(new window.Event("change", { bubbles: true }));
  check("0.34 downloads: default change posts set-default-connections {conn}",
    dlMsgs("set-default-connections").length === 1 &&
    dlMsgs("set-default-connections")[0].conn === 4,
    JSON.stringify(dlMsgs("set-default-connections")));
  addUrl.value = "https://example.com/new.bin";
  doc.getElementById("dl-add-btn").click();
  check("0.34 downloads: Add posts add-url {url}",
    dlMsgs("add-url").length === 1 &&
    dlMsgs("add-url")[0].url === "https://example.com/new.bin",
    JSON.stringify(dlMsgs("add-url")));
  check("0.34 downloads: the add input clears after a successful Add",
    addUrl.value === "", addUrl.value);
  addUrl.value = "https://example.com/two.bin";
  addUrl.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  check("0.34 downloads: Enter in the add input also adds",
    dlMsgs("add-url").length === 2 &&
    dlMsgs("add-url")[1].url === "https://example.com/two.bin" &&
    addUrl.value === "",
    JSON.stringify(dlMsgs("add-url")));
  doc.getElementById("dl-add-btn").click();
  check("0.34 downloads: an empty add sends nothing",
    dlMsgs("add-url").length === 2);
  rowOf(13).querySelector(".dl-folder").click();
  check("0.34 downloads: the done row's Folder posts open-folder {id}",
    dlMsgs("open-folder").length === 1 && dlMsgs("open-folder")[0].id === 13,
    JSON.stringify(dlMsgs("open-folder")));

  // Dismissal paths: close button, outside pointerdown,
  // Escape, toolbar toggle — all post close-panel; the
  // toolbar button posts open-panel when closed.
  doc.getElementById("dp-close").click();
  check("0.34 downloads: close button hides the panel + posts close-panel",
    !panel.classList.contains("on") && dlMsgs("close-panel").length === 1,
    JSON.stringify(dlMsgs("close-panel")));
  dlBtn.click();
  check("0.34 downloads: toolbar button posts open-panel",
    dlMsgs("open-panel").length === 1, JSON.stringify(dlMsgs("open-panel")));
  push({ downloadsOpen: true, downloads: dlSnap(reconn) });
  check("0.34 downloads: panel reopens on the answering snapshot",
    panel.classList.contains("on"));
  panel.dispatchEvent(new window.MouseEvent("pointerdown", { bubbles: true }));
  check("0.34 downloads: a pointerdown ON the panel stays open",
    panel.classList.contains("on") && dlMsgs("close-panel").length === 1);
  dlBtn.dispatchEvent(new window.MouseEvent("pointerdown", { bubbles: true }));
  check("0.34 downloads: a pointerdown on the downloads button stays open",
    panel.classList.contains("on") && dlMsgs("close-panel").length === 1);
  doc.body.dispatchEvent(new window.MouseEvent("pointerdown", { bubbles: true }));
  check("0.34 downloads: an outside pointerdown closes + posts close-panel",
    !panel.classList.contains("on") && dlMsgs("close-panel").length === 2);
  push({ downloadsOpen: true, downloads: dlSnap(reconn) });
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  check("0.34 downloads: Escape closes first + posts close-panel",
    !panel.classList.contains("on") && dlMsgs("close-panel").length === 3);
  push({ downloadsOpen: true, downloads: dlSnap(reconn) });
  dlBtn.click();
  check("0.34 downloads: toolbar button toggles the open panel shut",
    !panel.classList.contains("on") && dlMsgs("close-panel").length === 4 &&
    dlMsgs("open-panel").length === 1);

  // A connection count outside the six choices renders as
  // its own option, and leaves once standard again.
  const oddTask = { id: 15, name: "odd.bin", host: "odd.example.com", url: "https://odd.example.com/odd.bin", status: "paused", total: 7000, downloaded: 0, percent: 0, size_text: "7 KB", detail_text: "7 KB · Paused", connections: 7, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730014400000 };
  push({ downloadsOpen: true, downloads: dlSnap([...TASKS, oddTask]) });
  check("0.34 downloads: a non-standard conn count renders as its own option",
    !!rowOf(15) && rowOf(15).querySelector(".dl-conn").value === "7" &&
    [...rowOf(15).querySelectorAll(".dl-conn option")].some((o) => o.value === "7"),
    rowOf(15) ? rowOf(15).querySelector(".dl-conn").value : "no row");
  push({ downloadsOpen: true, downloads: dlSnap([...TASKS, { ...oddTask, connections: 8 }]) });
  check("0.34 downloads: the borrowed option leaves once the count is standard",
    rowOf(15).querySelector(".dl-conn").value === "8" &&
    ![...rowOf(15).querySelectorAll(".dl-conn option")].some((o) => o.value === "7"));

  // Badge at zero + the empty state.
  push({ downloadsOpen: false, downloads: dlSnap([], { active_count: 0 }) });
  check("0.34 downloads: badge hides at zero active",
    !badge.classList.contains("on") && badge.textContent === "");
  push({ downloadsOpen: true, downloads: dlSnap([], { active_count: 0 }) });
  check("0.34 downloads: an empty open panel shows the empty state and no rows",
    doc.getElementById("dl-empty").classList.contains("on") &&
    list.querySelectorAll(".dl-row").length === 0 &&
    doc.getElementById("dl-empty").textContent === "Downloads you start will show here");

  /* ---------------------------------------------------------------
     The toast: renders from snapshot.notice (seq + open) — a
     new sequence renders the pill and pulses the badge; the
     same sequence never re-renders; open:false hides; the
     open panel suppresses; ANY click posts the single
     dismiss-notice verb (0.34 — the app opens the page).
     --------------------------------------------------------------- */
  const toast = doc.getElementById("dl-toast");
  const toastLead = doc.getElementById("dl-toast-lead");
  const toastName = doc.getElementById("dl-toast-name");
  const noticePush = (notice, over = {}) => push({
    downloadsOpen: false,
    downloads: dlSnap([], { active_count: 1, notice, ...over }),
  });
  noticePush({ seq: 1, kind: "started", name: "ShutUp10.rar", reason: null, open: true });
  check("0.34 toast: a started notice shows the pill with the file name",
    toast.classList.contains("on") && !toast.classList.contains("warn") &&
    toastLead.textContent === "Download started" && toastName.textContent === "ShutUp10.rar",
    toastLead.textContent + " | " + toastName.textContent);
  check("0.34 toast: the badge pulses on the new task",
    badge.classList.contains("pulse"));
  noticePush({ seq: 1, kind: "started", name: "OtherName.zip", reason: null, open: true });
  check("0.34 toast: the same sequence never re-renders",
    toast.classList.contains("on") && toastName.textContent === "ShutUp10.rar",
    toastName.textContent);
  noticePush({ seq: 1, kind: "started", name: "ShutUp10.rar", reason: null, open: false });
  check("0.34 toast: expiry (open:false) hides the pill",
    !toast.classList.contains("on"));
  noticePush({ seq: 2, kind: "blocked", name: "x.rar", reason: "unsupported link type (blob:)", open: true });
  check("0.34 toast: a blocked notice shows the warning variant with the reason",
    toast.classList.contains("on") && toast.classList.contains("warn") &&
    toastLead.textContent === "Download blocked" &&
    toastName.textContent === "unsupported link type (blob:)",
    toastLead.textContent + " | " + toastName.textContent);
  check("0.34 toast: a blocked notice pulses the badge too",
    badge.classList.contains("pulse"));
  push({ downloadsOpen: true, downloads: dlSnap(TASKS, { notice: { seq: 2, kind: "blocked", name: "x.rar", reason: "unsupported link type (blob:)", open: true } }) });
  check("0.34 toast: the open panel suppresses the pill",
    !toast.classList.contains("on") && doc.getElementById("dpanel").classList.contains("on"));
  noticePush({ seq: 3, kind: "started", name: "PhotoPack.zip", reason: null, open: true }, { active_count: 2 });
  check("0.34 toast: a fresh notice re-arms after retirement",
    toast.classList.contains("on") && !toast.classList.contains("warn") &&
    toastName.textContent === "PhotoPack.zip",
    toastName.textContent);
  sent.length = 0;
  toast.click();
  check("0.34 toast: clicking the pill hides it + posts dismiss-notice only",
    !toast.classList.contains("on") && dlMsgs("dismiss-notice").length === 1 &&
    dlMsgs("open-panel").length === 0 && dlMsgs("open-page").length === 0,
    JSON.stringify(sent.filter((m) => m.kind === "dl")));
  push({});
  check("0.34 toast: an absent notice leaves the pill hidden",
    !toast.classList.contains("on"));

  /* ---------------------------------------------------------------
     Popover footer, failed-unseen badge, Ctrl+J, and the
     finished/failed notice kinds.
     --------------------------------------------------------------- */
  const dlAll = doc.getElementById("dl-all");
  check("0.34 chrome: the popover footer button lives inside #dpanel",
    !!dlAll && !!dlAll.closest("#dpanel") && dlAll.textContent === "All downloads");
  sent.length = 0;
  dlAll.click();
  check("0.34 chrome: All downloads posts open-page",
    dlMsgs("open-page").length === 1, JSON.stringify(sent.filter((m) => m.kind === "dl")));

  const failBadge = doc.getElementById("dl-fail-badge");
  push({ downloads: dlSnap(TASKS, { failed_unseen: 2 }) });
  check("0.34 chrome: the failed badge shows failed_unseen",
    failBadge.classList.contains("on") && failBadge.textContent === "2",
    failBadge.textContent);
  push({ downloads: dlSnap(TASKS, { failed_unseen: 0 }) });
  check("0.34 chrome: the failed badge hides at zero",
    !failBadge.classList.contains("on") && failBadge.textContent === "");

  sent.length = 0;
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "j", ctrlKey: true, bubbles: true }));
  check("0.34 chrome: Ctrl+J posts open-page",
    dlMsgs("open-page").length === 1, JSON.stringify(sent.filter((m) => m.kind === "dl")));

  noticePush({ seq: 4, kind: "finished", name: "Done.zip", reason: null, open: true }, { active_count: 0 });
  check("0.34 toast: a finished notice leads 'Download finished'",
    toast.classList.contains("on") && !toast.classList.contains("warn") &&
    toastLead.textContent === "Download finished" && toastName.textContent === "Done.zip",
    toastLead.textContent + " | " + toastName.textContent);
  sent.length = 0;
  toast.click();
  check("0.34 toast: clicking a finished notice posts dismiss-notice (the app opens the page)",
    dlMsgs("dismiss-notice").length === 1 &&
    dlMsgs("open-page").length === 0 && dlMsgs("open-panel").length === 0,
    JSON.stringify(sent.filter((m) => m.kind === "dl")));

  noticePush({ seq: 5, kind: "failed", name: "Bad.rar", reason: "HTTP 403", open: true }, { active_count: 0 });
  check("0.34 toast: a failed notice leads 'Download failed' in the warn styling",
    toast.classList.contains("on") && toast.classList.contains("warn") &&
    toastLead.textContent === "Download failed" && toastName.textContent === "HTTP 403",
    toastLead.textContent + " | " + toastName.textContent);
  sent.length = 0;
  toast.click();
  check("0.34 toast: clicking a failed notice posts dismiss-notice",
    dlMsgs("dismiss-notice").length === 1,
    JSON.stringify(sent.filter((m) => m.kind === "dl")));

  /* The themed selection + caret (0.32.3): CSS-level pins
     over the assembled page. */
  check("0.32.3 addr: ::selection paints the theme tint, never the OS block",
    chromeHtml.includes("::selection { background: var(--selection); color: var(--text); }"));
  check("0.32.3 addr: the address caret is the theme accent",
    chromeHtml.includes("caret-color: var(--accent)"));
  window.close();
}

// ------------------------------------------------------------------
// Chrome 0.34.1: the context menu. The snapshot gains
// `contextMenu`: null, or {x, y, items:[{id, label, shortcut,
// enabled, sep}]} — the chrome face renders whatever items
// the snapshot carries (never a hardcoded list), positions
// the menu at x/y, and answers with exactly two commands:
// {t:"ctx_pick", id} for an enabled row's click and
// {t:"ctx_close"} for Escape / an outside pointerdown (the
// send() envelope v/boot/req rides along, like every other
// chrome command). A following contextMenu:null snapshot
// hides the menu silently.
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  const ctxMsgs = (t) => sent.filter((m) => m.t === t);
  const baseSnap = {
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, focusAddr: false,
    palette: "forest", themes: [], paletteOpen: false,
    teleportOpen: false, teleport: null,
    downloadsOpen: false,
    downloads: { tasks: [], active_count: 0, failed_unseen: 0, settings: { default_connections: 8 }, notice: null },
    contextMenu: null,
  };
  const ITEMS = [
    { id: "back", label: "Back", shortcut: "Alt+Left", enabled: true, sep: false },
    { id: "forward", label: "Forward", shortcut: null, enabled: false, sep: false },
    { id: "reload", label: "Reload", shortcut: "Ctrl+R", enabled: true, sep: false },
    { id: "", label: "", shortcut: null, enabled: false, sep: true },
    { id: "copy", label: "Copy", shortcut: "Ctrl+C", enabled: true, sep: false },
    { id: "inspect", label: "Inspect", shortcut: null, enabled: true, sep: false },
  ];
  const menuSnap = { contextMenu: { x: 120, y: 240, items: ITEMS } };
  const push = (over) => window.__pico({ ...baseSnap, ...over });
  const menu = doc.getElementById("ctxmenu");
  const rows = () => [...menu.querySelectorAll(".ctx-row")];
  const rowByLabel = (label) =>
    rows().find((r) => r.querySelector(".ctx-label").textContent === label);

  check("0.34.1 ctx: menu element exists as a role=menu overlay",
    !!menu && menu.getAttribute("role") === "menu" &&
    menu.getAttribute("aria-label") === "Context menu");
  push({});
  check("0.34.1 ctx: menu starts hidden on a null snapshot",
    !menu.classList.contains("on"));

  // (a) Rendering: rows, the separator, position, disabled face.
  push(menuSnap);
  check("0.34.1 ctx: menu opens on the snapshot",
    menu.classList.contains("on"));
  check("0.34.1 ctx: position left/top come from the snapshot x/y",
    menu.style.left === "120px" && menu.style.top === "240px",
    `${menu.style.left}/${menu.style.top}`);
  check("0.34.1 ctx: one row renders per non-separator item",
    rows().length === 5, `got ${rows().length}`);
  check("0.34.1 ctx: the separator renders as its own element",
    menu.querySelectorAll(".ctx-sep").length === 1 &&
    menu.children.length === 6,
    `seps=${menu.querySelectorAll(".ctx-sep").length} children=${menu.children.length}`);
  check("0.34.1 ctx: label and shortcut render as exact text",
    rowByLabel("Back").querySelector(".ctx-label").textContent === "Back" &&
    rowByLabel("Back").querySelector(".ctx-key").textContent === "Alt+Left" &&
    rowByLabel("Forward").querySelector(".ctx-key") === null,
    rowByLabel("Back").textContent);
  check("0.34.1 ctx: a disabled row carries the off styling + aria-disabled",
    rowByLabel("Forward").classList.contains("off") &&
    rowByLabel("Forward").getAttribute("aria-disabled") === "true" &&
    rowByLabel("Forward").getAttribute("role") === "menuitem");
  check("0.34.1 ctx: an enabled row is not off and not aria-disabled",
    !rowByLabel("Copy").classList.contains("off") &&
    rowByLabel("Copy").getAttribute("aria-disabled") === null &&
    rowByLabel("Copy").getAttribute("role") === "menuitem");
  const firstRowEl = rows()[0];
  push(menuSnap);
  check("0.34.1 ctx: an unchanged signature does not rebuild the rows",
    rows()[0] === firstRowEl);

  // (b) Picks: an enabled row posts exactly ctx_pick {id}
  // (plus the send() envelope); a disabled row posts nothing.
  rowByLabel("Copy").click();
  check("0.34.1 ctx: an enabled click sends exactly {t:ctx_pick, id} + envelope",
    ctxMsgs("ctx_pick").length === 1 &&
    ctxMsgs("ctx_pick")[0].id === "copy" &&
    ctxMsgs("ctx_pick")[0].v === 1 &&
    typeof ctxMsgs("ctx_pick")[0].boot === "string" &&
    typeof ctxMsgs("ctx_pick")[0].req === "number" &&
    JSON.stringify(Object.keys(ctxMsgs("ctx_pick")[0]).sort()) ===
      JSON.stringify(["boot", "id", "req", "t", "v"]),
    JSON.stringify(ctxMsgs("ctx_pick")));
  rowByLabel("Forward").click();
  check("0.34.1 ctx: a disabled row's click sends nothing",
    ctxMsgs("ctx_pick").length === 1 &&
    ctxMsgs("ctx_close").length === 0,
    JSON.stringify(sent.filter((m) => m.t && m.t.startsWith("ctx_"))));

  // (c) Escape dismisses — first in the document chain.
  doc.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  check("0.34.1 ctx: Escape sends {t:ctx_close} and hides the menu",
    ctxMsgs("ctx_close").length === 1 && !menu.classList.contains("on"),
    JSON.stringify(ctxMsgs("ctx_close")));

  // (d) Outside pointerdown dismisses; inside does not.
  push(menuSnap);
  check("0.34.1 ctx: the menu reopens on a fresh snapshot",
    menu.classList.contains("on"));
  rowByLabel("Back").dispatchEvent(
    new window.MouseEvent("pointerdown", { bubbles: true }));
  check("0.34.1 ctx: a pointerdown INSIDE the menu does not close it",
    menu.classList.contains("on") && ctxMsgs("ctx_close").length === 1);
  doc.body.dispatchEvent(new window.MouseEvent("pointerdown", { bubbles: true }));
  check("0.34.1 ctx: a pointerdown OUTSIDE sends {t:ctx_close} and hides the menu",
    ctxMsgs("ctx_close").length === 2 && !menu.classList.contains("on"),
    JSON.stringify(ctxMsgs("ctx_close")));

  // (e) A following null snapshot hides the menu, silently.
  push(menuSnap);
  check("0.34.1 ctx: the menu opens again for the null test",
    menu.classList.contains("on"));
  push({ contextMenu: null });
  check("0.34.1 ctx: a null snapshot hides the menu and echoes no ctx_close",
    !menu.classList.contains("on") && ctxMsgs("ctx_close").length === 2);
  window.close();
}

// ------------------------------------------------------------------
// Downloads page 0.34: the dumped downloads.html (fixture
// init from examples/dump_pages.rs — THE snapshot, one task
// per status) renders rows via DlRows with per-status
// actions; search + chips filter; every control posts its
// exact {kind:"dl", …} body; a live __picoSetDownloads push
// re-renders while preserving the search text and the open
// refresh editor.
// ------------------------------------------------------------------
{
  const sent = [];
  const dom = new JSDOM(downloadsHtml, {
    runScripts: "dangerously",
    url: "https://pico.downloads/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  doc.execCommand = () => true;
  const dlMsgs = (action) => sent.filter((m) => m.kind === "dl" && m.action === action);
  const exact = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const list = doc.getElementById("dl-page-list");
  const rows = () => [...list.querySelectorAll(".dlp-row")];
  const rowOf = (id) => list.querySelector(`[data-dl-id="${id}"]`);
  const btnOf = (row, label) =>
    [...row.querySelectorAll("button")].find((b) => b.textContent === label);
  const SETTINGS = { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 };
  const FIXTURE = [
    { id: 11, name: "ubuntu-24.04.iso", host: "releases.ubuntu.com", url: "https://releases.ubuntu.com/ubuntu-24.04.iso", status: "downloading", total: 1500000000, downloaded: 637500000, percent: 42.5, size_text: "1.4 GB", detail_text: "612 MB of 1.4 GB · 3.2 MB/s · 4 min left", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730000000000 },
    { id: 12, name: "lecture.mp4", host: "media.example.org", url: "https://media.example.org/lecture.mp4", status: "paused", total: 251658240, downloaded: 50331648, percent: 20, size_text: "240 MB", detail_text: "48 MB of 240 MB · Paused", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730003600000 },
    { id: 13, name: "file.rar", host: "dl.soft98.ir", url: "https://dl.soft98.ir/file.rar?token=abc123", status: "failed", total: 89128960, downloaded: 26738688, percent: 30, size_text: "85 MB", detail_text: "25 MB of 85 MB · Failed", connections: 8, error: "HTTP 403", hint: "the link may have expired — use Refresh link", retry_text: "Retrying in 15 s · attempt 1 of 3", retry_pending: true, refreshing: false, sha256: null, added_ms: 1730007200000 },
    { id: 14, name: "manual.pdf", host: "docs.example.com", url: "https://docs.example.com/manual.pdf", status: "done", total: 12582912, downloaded: 12582912, percent: 100, size_text: "12.0 MB", detail_text: "12.0 MB", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08", added_ms: 1730010800000 },
    { id: 15, name: "notes.zip", host: "files.example.com", url: "https://files.example.com/notes.zip", status: "queued", total: 5242880, downloaded: 0, percent: 0, size_text: "5.0 MB", detail_text: "Queued", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730014400000 },
  ];
  const pay = (tasks, over = {}) => ({
    theme: "light", palette: "forest", tasks,
    active_count: 2, failed_unseen: 1, settings: SETTINGS, notice: null, ...over,
  });

  check("0.34.1 downloads page: version marker reads 0.35.1",
    doc.getElementById("dl-ver").textContent === "0.40.1",
    doc.getElementById("dl-ver").textContent);
  check("0.34 downloads page: the fixture renders five rows in order",
    rows().length === 5 &&
    JSON.stringify(rows().map((r) => r.dataset.dlId)) === JSON.stringify(["11", "12", "13", "14", "15"]),
    rows().map((r) => r.dataset.dlId).join(","));
  check("0.34 downloads page: the count line summarizes totals",
    doc.getElementById("dl-count-line").textContent === "5 downloads · 2 active",
    doc.getElementById("dl-count-line").textContent);
  check("0.34 downloads page: settings selects reflect the fixture settings",
    doc.getElementById("dl-max-concurrent").value === "3" &&
    doc.getElementById("dl-speed-limit").value === "0");
  const r11 = rowOf(11), r12 = rowOf(12), r13 = rowOf(13), r14 = rowOf(14), r15 = rowOf(15);
  check("0.34 downloads page: downloading row offers Pause and its bar is at 42.5%",
    !!btnOf(r11, "Pause") && !btnOf(r11, "Resume") &&
    r11.querySelector(".dlp-fill").style.width === "42.5%",
    r11.querySelector(".dlp-fill").style.width);
  check("0.34 downloads page: downloading meta line is host · size · date",
    r11.querySelector(".dlp-meta").textContent.startsWith("releases.ubuntu.com · 1.4 GB · "),
    r11.querySelector(".dlp-meta").textContent);
  check("0.34 downloads page: paused row offers Resume + Restart, no Refresh link",
    !!btnOf(r12, "Resume") && !!btnOf(r12, "Restart") && !btnOf(r12, "Refresh link…"));
  check("0.34 downloads page: failed row offers Resume + Restart + Refresh link",
    !!btnOf(r13, "Resume") && !!btnOf(r13, "Restart") && !!btnOf(r13, "Refresh link…"));
  check("0.34 downloads page: failed row shows the error, the hint and the retry line",
    r13.querySelector(".dlp-error").textContent === "HTTP 403" &&
    r13.querySelector(".dlp-hint").textContent === "the link may have expired — use Refresh link" &&
    r13.querySelector(".dlp-retry").textContent === "Retrying in 15 s · attempt 1 of 3",
    r13.textContent);
  check("0.34 downloads page: done row offers Open file and carries the SHA-256 line",
    !!btnOf(r14, "Open file") && !!btnOf(r14, "Open folder") &&
    !!r14.querySelector(".dlp-sha") &&
    r14.querySelector(".dlp-sha").textContent.includes("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"));
  check("0.34 downloads page: the SHA-256 line appears ONLY on the done row",
    !r11.querySelector(".dlp-sha") && !r12.querySelector(".dlp-sha") &&
    !r13.querySelector(".dlp-sha") && !r15.querySelector(".dlp-sha"));
  check("0.34 downloads page: queued row offers Pause + Remove",
    !!btnOf(r15, "Pause") && !!btnOf(r15, "Remove"));
  check("0.34 downloads page: every row offers the page-local Copy link",
    [r11, r12, r13, r14, r15].every((r) => !!btnOf(r, "Copy link")));
  check("0.34 downloads page: every task button carries a data-dl-action",
    [r11, r12, r13, r14, r15].every((r) =>
      [...r.querySelectorAll(".dlp-actions button")].every((b) =>
        b.hasAttribute("data-dl-action") || b.hasAttribute("data-dl-local"))));
  check("0.34 downloads page: both empty states start hidden with rows present",
    doc.getElementById("dl-page-empty").hidden === true &&
    doc.getElementById("dl-page-nomatch").hidden === true);

  // Search: by name, then by host.
  const search = doc.getElementById("dl-search");
  search.value = "ubuntu";
  search.dispatchEvent(new window.Event("input", { bubbles: true }));
  check("0.34 downloads page: search filters by name",
    rows().length === 1 && rows()[0].dataset.dlId === "11",
    rows().map((r) => r.dataset.dlId).join(","));
  search.value = "soft98";
  search.dispatchEvent(new window.Event("input", { bubbles: true }));
  check("0.34 downloads page: search filters by host",
    rows().length === 1 && rows()[0].dataset.dlId === "13",
    rows().map((r) => r.dataset.dlId).join(","));
  search.value = "";
  search.dispatchEvent(new window.Event("input", { bubbles: true }));
  check("0.34 downloads page: clearing the search restores all rows",
    rows().length === 5, String(rows().length));

  // Chips: failed → 1 row, done → 1 row, all → 5.
  doc.querySelector('[data-chip="failed"]').click();
  check("0.34 downloads page: the failed chip filters to the failed row",
    rows().length === 1 && rows()[0].dataset.dlId === "13",
    rows().map((r) => r.dataset.dlId).join(","));
  doc.querySelector('[data-chip="done"]').click();
  check("0.34 downloads page: the done chip filters to the done row",
    rows().length === 1 && rows()[0].dataset.dlId === "14",
    rows().map((r) => r.dataset.dlId).join(","));
  doc.querySelector('[data-chip="all"]').click();
  check("0.34 downloads page: the all chip restores every row",
    rows().length === 5, String(rows().length));

  // Exact IPC bodies, one control at a time.
  btnOf(rowOf(11), "Pause").click();
  check("0.34 downloads page: Pause posts the exact pause body",
    dlMsgs("pause").length === 1 && exact(dlMsgs("pause")[0], { kind: "dl", action: "pause", id: 11 }),
    JSON.stringify(dlMsgs("pause")));
  btnOf(rowOf(12), "Resume").click();
  check("0.34 downloads page: Resume posts the exact resume body",
    dlMsgs("resume").length === 1 && exact(dlMsgs("resume")[0], { kind: "dl", action: "resume", id: 12 }),
    JSON.stringify(dlMsgs("resume")));
  const restartBtn = btnOf(rowOf(12), "Restart");
  restartBtn.click();
  check("0.34 downloads page: the first Restart click only arms (posts nothing)",
    dlMsgs("restart").length === 0 && restartBtn.textContent === "Sure?",
    restartBtn.textContent);
  restartBtn.click();
  check("0.34 downloads page: the second Restart click posts the exact restart body",
    dlMsgs("restart").length === 1 && exact(dlMsgs("restart")[0], { kind: "dl", action: "restart", id: 12 }),
    JSON.stringify(dlMsgs("restart")));
  btnOf(rowOf(13), "Refresh link…").click();
  const refreshInput = rowOf(13).querySelector(".dl-refresh-input");
  check("0.34 downloads page: Refresh link opens the editor prefilled with the task URL",
    !!refreshInput && refreshInput.value === "https://dl.soft98.ir/file.rar?token=abc123",
    refreshInput ? refreshInput.value : "no editor");
  refreshInput.value = "https://dl.soft98.ir/file.rar?token=xyz789";
  refreshInput.dispatchEvent(new window.Event("input", { bubbles: true }));
  rowOf(13).querySelector(".dl-refresh-apply").click();
  check("0.34 downloads page: Apply posts the exact refresh body with the new URL",
    dlMsgs("refresh").length === 1 &&
    exact(dlMsgs("refresh")[0], { kind: "dl", action: "refresh", id: 13, url: "https://dl.soft98.ir/file.rar?token=xyz789" }),
    JSON.stringify(dlMsgs("refresh")));
  check("0.34 downloads page: the editor closes after Apply",
    rowOf(13).querySelector(".dl-refresh-input") === null);
  btnOf(rowOf(14), "Open file").click();
  check("0.34 downloads page: Open file posts the exact open-file body",
    dlMsgs("open-file").length === 1 && exact(dlMsgs("open-file")[0], { kind: "dl", action: "open-file", id: 14 }),
    JSON.stringify(dlMsgs("open-file")));
  btnOf(rowOf(14), "Open folder").click();
  check("0.34 downloads page: Open folder posts the exact open-folder body",
    dlMsgs("open-folder").length === 1 && exact(dlMsgs("open-folder")[0], { kind: "dl", action: "open-folder", id: 14 }),
    JSON.stringify(dlMsgs("open-folder")));
  btnOf(rowOf(15), "Remove").click();
  check("0.34 downloads page: Remove posts the exact remove body",
    dlMsgs("remove").length === 1 && exact(dlMsgs("remove")[0], { kind: "dl", action: "remove", id: 15 }),
    JSON.stringify(dlMsgs("remove")));
  btnOf(rowOf(12), "Delete file").click();
  check("0.34 downloads page: Delete file posts the exact delete-file body",
    dlMsgs("delete-file").length === 1 && exact(dlMsgs("delete-file")[0], { kind: "dl", action: "delete-file", id: 12 }),
    JSON.stringify(dlMsgs("delete-file")));
  // Copy link is client-only: it must post NOTHING.
  const sentBeforeCopy = sent.length;
  btnOf(rowOf(14), "Copy link").click();
  check("0.34 downloads page: Copy link is client-only (posts nothing)",
    sent.length === sentBeforeCopy,
    JSON.stringify(sent.slice(sentBeforeCopy)));
  doc.getElementById("dl-clear-completed").click();
  check("0.34 downloads page: Clear completed posts the exact body",
    dlMsgs("clear-completed").length === 1 &&
    exact(dlMsgs("clear-completed")[0], { kind: "dl", action: "clear-completed" }),
    JSON.stringify(dlMsgs("clear-completed")));
  doc.getElementById("dl-clear-all").click();
  check("0.34 downloads page: Clear all posts the exact body",
    dlMsgs("clear-all").length === 1 &&
    exact(dlMsgs("clear-all")[0], { kind: "dl", action: "clear-all" }),
    JSON.stringify(dlMsgs("clear-all")));
  const addUrl = doc.getElementById("dl-add-url");
  addUrl.value = "  https://example.com/new.zip  ";
  doc.getElementById("dl-add-btn").click();
  check("0.34 downloads page: Add posts the trimmed URL and clears the input",
    dlMsgs("add-url").length === 1 &&
    exact(dlMsgs("add-url")[0], { kind: "dl", action: "add-url", url: "https://example.com/new.zip" }) &&
    addUrl.value === "",
    JSON.stringify(dlMsgs("add-url")));
  const maxConc = doc.getElementById("dl-max-concurrent");
  maxConc.value = "5";
  maxConc.dispatchEvent(new window.Event("change", { bubbles: true }));
  check("0.34 downloads page: the At-once select posts set-max-concurrent {maxConcurrent}",
    dlMsgs("set-max-concurrent").length === 1 &&
    exact(dlMsgs("set-max-concurrent")[0], { kind: "dl", action: "set-max-concurrent", maxConcurrent: 5 }),
    JSON.stringify(dlMsgs("set-max-concurrent")));
  const speed = doc.getElementById("dl-speed-limit");
  speed.value = "1024";
  speed.dispatchEvent(new window.Event("change", { bubbles: true }));
  check("0.34 downloads page: the Speed-limit select posts set-speed-limit {speedLimitKbps}",
    dlMsgs("set-speed-limit").length === 1 &&
    exact(dlMsgs("set-speed-limit")[0], { kind: "dl", action: "set-speed-limit", speedLimitKbps: 1024 }),
    JSON.stringify(dlMsgs("set-speed-limit")));

  // Live push: the bar advances; the search text and the open
  // refresh editor (with its draft) survive the re-render.
  const advanced = FIXTURE.map((t) => t.id === 11 ? { ...t, percent: 50, downloaded: 750000000 } : t);
  window.__picoSetDownloads(pay(advanced));
  check("0.34 downloads page: a push re-render advances the progress bar",
    rowOf(11).querySelector(".dlp-fill").style.width === "50%",
    rowOf(11).querySelector(".dlp-fill").style.width);
  search.value = "rar";
  search.dispatchEvent(new window.Event("input", { bubbles: true }));
  btnOf(rowOf(13), "Refresh link…").click();
  const draftInput = rowOf(13).querySelector(".dl-refresh-input");
  draftInput.value = "https://draft.example/keep-me";
  draftInput.dispatchEvent(new window.Event("input", { bubbles: true }));
  window.__picoSetDownloads(pay(advanced));
  check("0.34 downloads page: a push preserves the search text",
    search.value === "rar" && rows().length === 1 && rows()[0].dataset.dlId === "13",
    search.value);
  const keptInput = rowOf(13).querySelector(".dl-refresh-input");
  check("0.34 downloads page: a push preserves the open refresh editor and its draft",
    !!keptInput && keptInput.value === "https://draft.example/keep-me",
    keptInput ? keptInput.value : "editor lost");

  // Empty states: no tasks at all, then a filter excluding all.
  window.__picoSetDownloads(pay([], { active_count: 0, failed_unseen: 0 }));
  check("0.34 downloads page: zero tasks shows the empty state, not the no-match state",
    doc.getElementById("dl-page-empty").hidden === false &&
    doc.getElementById("dl-page-nomatch").hidden === true &&
    rows().length === 0);
  window.__picoSetDownloads(pay(FIXTURE));
  search.value = "no-such-download";
  search.dispatchEvent(new window.Event("input", { bubbles: true }));
  check("0.34 downloads page: a filter excluding everything shows the no-match state",
    doc.getElementById("dl-page-nomatch").hidden === false &&
    doc.getElementById("dl-page-empty").hidden === true &&
    rows().length === 0);
  window.close();
}

// ------------------------------------------------------------------
// 0.34 JS↔Rust pair test (JS half): the DlRows splice, the
// frozen verb set, and the start-page Downloads card.
// - Both documents carry the SAME DlRows bytes (the token is
//   gone after assembly; renderTaskList + bindCommands are
//   present in both).
// - NO legacy download tokens remain in any template.
// - Every rendered task action carries a data-dl-action in
//   the frozen verb set, and the rendered task-verb set is
//   exactly the expected union of both surfaces.
// - Driving every control across both surfaces emits exactly
//   the frozen verb set minus copy-link (client-only). The
//   page-chrome clear-completed / clear-all verbs are NOT
//   part of this set: they predate the frozen list, are
//   asserted exactly in the page block above, and are
//   flagged as a contract gap in the 0.34 report.
// ------------------------------------------------------------------
{
  const FROZEN = ["open-panel", "close-panel", "open-page", "pause", "resume",
    "cancel", "restart", "refresh", "remove", "delete-file", "open-file",
    "open-folder", "copy-link", "add-url", "set-connections",
    "set-default-connections", "set-max-concurrent", "set-speed-limit",
    "dismiss-notice"];
  check("0.34 pair: the DlRows token was spliced out of both documents",
    !chromeHtml.includes("/*__DLROWS__*/") && !downloadsHtml.includes("/*__DLROWS__*/"));
  check("0.34 pair: both documents carry the shared renderer + binder",
    chromeHtml.includes("DlRows.renderTaskList") && downloadsHtml.includes("DlRows.renderTaskList") &&
    chromeHtml.includes("bindCommands") && downloadsHtml.includes("bindCommands"));
  const LEGACY = ["download_pause", "download_resume", "download_cancel",
    "download_set_conn", "download_set_default_conn", "download_add",
    "download_open_folder", "downloads_close", "downloads_page",
    't: "downloads"', "download_set_default"];
  check("0.34 pair: no legacy download tokens in any of the three templates",
    LEGACY.every((tok) => !chromeHtml.includes(tok) && !downloadsHtml.includes(tok) && !startHtml.includes(tok)),
    LEGACY.filter((tok) => chromeHtml.includes(tok) || downloadsHtml.includes(tok) || startHtml.includes(tok)).join(","));

  // --- Rendered verb set across both surfaces.
  const cSent = [];
  const cDom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => cSent.push(JSON.parse(m)) };
    },
  });
  const cDoc = cDom.window.document;
  const pSent = [];
  const pDom = new JSDOM(downloadsHtml, {
    runScripts: "dangerously",
    url: "https://pico.downloads/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => pSent.push(JSON.parse(m)) };
    },
  });
  const pDoc = pDom.window.document;
  pDoc.execCommand = () => true;
  const PAIR_TASKS = [
    { id: 21, name: "a.zip", host: "a.example", url: "https://a.example/a.zip", status: "downloading", total: 100, downloaded: 40, percent: 40, size_text: "100 B", detail_text: "40 B of 100 B", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730000000000 },
    { id: 22, name: "b.zip", host: "b.example", url: "https://b.example/b.zip", status: "paused", total: 100, downloaded: 10, percent: 10, size_text: "100 B", detail_text: "Paused", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730003600000 },
    { id: 23, name: "c.zip", host: "c.example", url: "https://c.example/c.zip", status: "failed", total: 100, downloaded: 10, percent: 10, size_text: "100 B", detail_text: "Failed", connections: 8, error: "HTTP 403", hint: "the link may have expired — use Refresh link", retry_text: null, retry_pending: false, refreshing: false, sha256: null, added_ms: 1730007200000 },
    { id: 24, name: "d.zip", host: "d.example", url: "https://d.example/d.zip", status: "done", total: 100, downloaded: 100, percent: 100, size_text: "100 B", detail_text: "100 B", connections: 8, error: null, hint: null, retry_text: null, retry_pending: false, refreshing: false, sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08", added_ms: 1730010800000 },
  ];
  const pairSnap = (tasks, over = {}) => ({
    tasks, active_count: 1, failed_unseen: 1,
    settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 },
    notice: null, ...over,
  });
  cDom.window.__pico({
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, activeError: "", blockedTotal: 0, blockedTab: 0,
    shieldOn: true, version: "0.40.1", liteOn: false, liteBlocked: 0,
    focusAddr: false, palette: "forest", themes: [], paletteOpen: false,
    teleportOpen: false, teleport: null,
    downloadsOpen: true, downloads: pairSnap(PAIR_TASKS),
  });
  // Push the pair fixture to the page, then open its refresh
  // editor so the refresh verb renders.
  pDom.window.__picoSetDownloads({
    theme: "light", palette: "forest", tasks: PAIR_TASKS,
    active_count: 1, failed_unseen: 1,
    settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 },
    notice: null,
  });
  const pList = pDoc.getElementById("dl-page-list");
  const pRowOf = (id) => pList.querySelector(`[data-dl-id="${id}"]`);
  const pBtnOf = (row, label) => [...row.querySelectorAll("button")].find((b) => b.textContent === label);
  pBtnOf(pRowOf(23), "Refresh link…").click();
  const rendered = new Set();
  cDoc.querySelectorAll("#dl-list [data-dl-action]").forEach((el) => rendered.add(el.getAttribute("data-dl-action")));
  pDoc.querySelectorAll("#dl-page-list [data-dl-action]").forEach((el) => rendered.add(el.getAttribute("data-dl-action")));
  check("0.34 pair: every rendered task action is in the frozen verb set",
    [...rendered].every((a) => FROZEN.includes(a)),
    [...rendered].join(","));
  const EXPECTED_RENDERED = ["pause", "resume", "cancel", "remove", "open-folder",
    "open-file", "delete-file", "restart", "refresh", "copy-link", "set-connections"];
  check("0.34 pair: the rendered task-verb set is exactly the two surfaces' union",
    JSON.stringify([...rendered].sort()) === JSON.stringify(EXPECTED_RENDERED.slice().sort()),
    [...rendered].sort().join(","));

  // --- Emitted verb set: drive every verb.
  const cList = cDoc.getElementById("dl-list");
  const cRowOf = (id) => cList.querySelector(`[data-dl-id="${id}"]`);
  cRowOf(21).querySelector(".dl-act").click();               // pause
  cRowOf(22).querySelector(".dl-act").click();               // resume
  cRowOf(21).querySelector(".dl-cancel").click();            // cancel
  cRowOf(24).querySelector(".dl-cancel").click();            // remove
  cRowOf(24).querySelector(".dl-folder").click();            // open-folder
  const cConn = cRowOf(21).querySelector(".dl-conn");
  cConn.value = "4";
  cConn.dispatchEvent(new cDom.window.Event("change", { bubbles: true })); // set-connections
  const cDef = cDoc.getElementById("dl-default-conn");
  cDef.value = "2";
  cDef.dispatchEvent(new cDom.window.Event("change", { bubbles: true }));  // set-default-connections
  const cAdd = cDoc.getElementById("dl-add-url");
  cAdd.value = "https://x.example/f.zip";
  cDoc.getElementById("dl-add-btn").click();                 // add-url
  cDoc.getElementById("dl-all").click();                     // open-page
  cDoc.getElementById("dp-close").click();                   // close-panel
  cDoc.getElementById("downloads").click();                  // open-panel
  cDom.window.__pico({
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, activeError: "", blockedTotal: 0, blockedTab: 0,
    shieldOn: true, version: "0.40.1", liteOn: false, liteBlocked: 0,
    focusAddr: false, palette: "forest", themes: [], paletteOpen: false,
    teleportOpen: false, teleport: null,
    downloadsOpen: false,
    downloads: pairSnap([], { active_count: 0, notice: { seq: 9, kind: "started", name: "n.zip", reason: null, open: true } }),
  });
  cDoc.getElementById("dl-toast").click();                   // dismiss-notice

  const pRestart = pBtnOf(pRowOf(22), "Restart");
  pRestart.click(); pRestart.click();                        // restart
  const pRefreshInput = pRowOf(23).querySelector(".dl-refresh-input");
  pRefreshInput.value = "https://c.example/c2.zip";
  pRefreshInput.dispatchEvent(new pDom.window.Event("input", { bubbles: true }));
  pRowOf(23).querySelector(".dl-refresh-apply").click();     // refresh
  pBtnOf(pRowOf(24), "Open file").click();                   // open-file
  pBtnOf(pRowOf(22), "Delete file").click();                 // delete-file
  const pMax = pDoc.getElementById("dl-max-concurrent");
  pMax.value = "4";
  pMax.dispatchEvent(new pDom.window.Event("change", { bubbles: true }));  // set-max-concurrent
  const pSpeed = pDoc.getElementById("dl-speed-limit");
  pSpeed.value = "512";
  pSpeed.dispatchEvent(new pDom.window.Event("change", { bubbles: true })); // set-speed-limit

  // XSS: a poisoned task name (network data) must stay inert
  // text through the string renderer — escaped, never parsed.
  const poisonName = '<img src=x onerror="window.__pwned=1">';
  cDom.window.__pico({
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, activeError: "", blockedTotal: 0, blockedTab: 0,
    shieldOn: true, version: "0.40.1", liteOn: false, liteBlocked: 0,
    focusAddr: false, palette: "forest", themes: [], paletteOpen: false,
    teleportOpen: false, teleport: null,
    downloadsOpen: true,
    downloads: pairSnap([{ ...PAIR_TASKS[0], id: 29, name: poisonName }]),
  });
  check("0.34 pair: a poisoned task name stays inert text in the popover",
    cDoc.querySelectorAll("#dl-list img").length === 0 &&
    !cDom.window.__pwned &&
    cRowOf(29).querySelector(".dl-name").textContent === poisonName,
    cRowOf(29) ? cRowOf(29).querySelector(".dl-name").textContent : "no row");

  const emitted = new Set();
  cSent.filter((m) => m.kind === "dl").forEach((m) => emitted.add(m.action));
  pSent.filter((m) => m.kind === "dl").forEach((m) => emitted.add(m.action));
  const EXPECTED_EMITTED = FROZEN.filter((a) => a !== "copy-link");
  check("0.34 pair: the emitted verb set EQUALS the frozen set minus client-only copy-link",
    JSON.stringify([...emitted].sort()) === JSON.stringify(EXPECTED_EMITTED.slice().sort()),
    [...emitted].sort().join(","));
  cDom.window.close();
  pDom.window.close();

  // --- Start page Downloads card (0.35.1, the Defect 2 proof).
  // The card model is the contract object
  // {state, count, speed_bps, paused, queued, newest_name,
  // unseen} — in INIT.downloads and in every
  // __picoSetDownloadsCard push. Fixture INIT (dump_pages.rs)
  // carries {state:"active", count:2, speed_bps:1258291, ...};
  // the speed humanizes through the start page's own
  // humanizeBytes: 1258291 -> "1.26 MB" (verified against the
  // live page function), so the total line pins
  // "Downloading · 1.26 MB/s" — NOT the rounded "1.2 MB/s" a
  // one-decimal humanizer would print. Card DOM:
  // #dl-card-top (row), #dl-card-active (number),
  // #dl-card-badge (pill), #dl-card-total (line).
  const sDom = new JSDOM(startHtml, {
    runScripts: "dangerously",
    url: "https://pico.start/",
    beforeParse(window) {
      window.ipc = { postMessage: () => {} };
    },
  });
  const sDoc = sDom.window.document;
  const card = sDoc.getElementById("dl-card");
  check("0.34 start: the Downloads card is present and links to pico://downloads",
    !!card && card.getAttribute("href") === "pico://downloads",
    card ? card.getAttribute("href") : "missing");
  const cardTop = () => sDoc.getElementById("dl-card-top");
  const cardActive = () => sDoc.getElementById("dl-card-active");
  const cardBadge = () => sDoc.getElementById("dl-card-badge");
  const cardTotal = () => sDoc.getElementById("dl-card-total");
  const topIsHidden = (el) => !!el && (el.hidden === true ||
    el.style.display === "none" || el.style.visibility === "hidden" ||
    el.classList.contains("hidden"));
  const cardDebug = () => `top=${cardTop() ? (topIsHidden(cardTop()) ? "hidden" : "shown") : "missing"} ` +
    `active=${cardActive() ? cardActive().textContent : "missing"} ` +
    `badge=${cardBadge() ? `${cardBadge().textContent}|${cardBadge().className}` : "missing"} ` +
    `total=${cardTotal() ? cardTotal().textContent : "missing"}`;
  const driveCard = (payload) => {
    let threw = false;
    try { sDom.window.__picoSetDownloadsCard(payload); } catch (e) { threw = true; }
    return threw;
  };
  // INIT state: the fixture's active model renders as active.
  check("0.35.1 start card: INIT (active, count 2, speed 1258291) renders the active card",
    !!cardTop() && !topIsHidden(cardTop()) &&
    !!cardActive() && cardActive().textContent === "2" &&
    !!cardBadge() && cardBadge().textContent === "ACTIVE" &&
    cardBadge().className === "scheme" &&
    !!cardTotal() && cardTotal().textContent === "Downloading · 1.26 MB/s",
    cardDebug());
  // All five states, driven through the hook.
  check("0.35.1 start card: empty hides the top row and reads 'No downloads yet'",
    !driveCard({ state: "empty", count: 0, speed_bps: null, paused: 0, queued: 0, newest_name: null, unseen: false }) &&
    !!cardTop() && topIsHidden(cardTop()) &&
    !!cardTotal() && cardTotal().textContent === "No downloads yet",
    cardDebug());
  check("0.35.1 start card: active with speed prints the humanized speed line",
    !driveCard({ state: "active", count: 3, speed_bps: 1258291, paused: 0, queued: 0, newest_name: null, unseen: false }) &&
    !!cardTop() && !topIsHidden(cardTop()) &&
    cardActive().textContent === "3" &&
    cardBadge().textContent === "ACTIVE" && cardBadge().className === "scheme" &&
    cardTotal().textContent === "Downloading · 1.26 MB/s",
    cardDebug());
  check("0.35.1 start card: active without speed prints plain 'Downloading'",
    !driveCard({ state: "active", count: 1, speed_bps: null, paused: 0, queued: 0, newest_name: null, unseen: false }) &&
    cardActive().textContent === "1" &&
    cardBadge().textContent === "ACTIVE" && cardBadge().className === "scheme" &&
    cardTotal().textContent === "Downloading",
    cardDebug());
  check("0.35.1 start card: waiting mixed (2 paused + 1 queued) badges WAITING and joins both parts",
    !driveCard({ state: "waiting", count: 3, speed_bps: null, paused: 2, queued: 1, newest_name: null, unseen: false }) &&
    cardActive().textContent === "3" &&
    cardBadge().textContent === "WAITING" && cardBadge().className === "scheme muted" &&
    cardTotal().textContent === "2 paused · 1 queued",
    cardDebug());
  check("0.35.1 start card: waiting paused-only badges PAUSED",
    !driveCard({ state: "waiting", count: 2, speed_bps: null, paused: 2, queued: 0, newest_name: null, unseen: false }) &&
    cardActive().textContent === "2" &&
    cardBadge().textContent === "PAUSED" && cardBadge().className === "scheme muted" &&
    cardTotal().textContent === "2 paused",
    cardDebug());
  check("0.35.1 start card: waiting queued-only badges QUEUED",
    !driveCard({ state: "waiting", count: 1, speed_bps: null, paused: 0, queued: 1, newest_name: null, unseen: false }) &&
    cardActive().textContent === "1" &&
    cardBadge().textContent === "QUEUED" && cardBadge().className === "scheme muted" &&
    cardTotal().textContent === "1 queued",
    cardDebug());
  check("0.35.1 start card: failed unseen with a name warns and shows the name",
    !driveCard({ state: "failed", count: 1, speed_bps: null, paused: 0, queued: 0, newest_name: "file.rar", unseen: true }) &&
    cardBadge().textContent === "FAILED" && cardBadge().className === "scheme warn" &&
    cardTotal().textContent === "file.rar",
    cardDebug());
  check("0.35.1 start card: failed unseen without a name reads 'Needs attention' and warns",
    !driveCard({ state: "failed", count: 1, speed_bps: null, paused: 0, queued: 0, newest_name: null, unseen: true }) &&
    cardBadge().textContent === "FAILED" && cardBadge().className === "scheme warn" &&
    cardTotal().textContent === "Needs attention",
    cardDebug());
  check("0.35.1 start card: failed seen without a name is muted and reads 'Download failed'",
    !driveCard({ state: "failed", count: 1, speed_bps: null, paused: 0, queued: 0, newest_name: null, unseen: false }) &&
    cardBadge().textContent === "FAILED" && cardBadge().className === "scheme muted" &&
    cardTotal().textContent === "Download failed",
    cardDebug());
  check("0.35.1 start card: finished with a name shows the name, muted FINISHED",
    !driveCard({ state: "finished", count: 1, speed_bps: null, paused: 0, queued: 0, newest_name: "manual.pdf", unseen: false }) &&
    cardBadge().textContent === "FINISHED" && cardBadge().className === "scheme muted" &&
    cardTotal().textContent === "manual.pdf",
    cardDebug());
  check("0.35.1 start card: finished without a name reads 'All downloads finished'",
    !driveCard({ state: "finished", count: 0, speed_bps: null, paused: 0, queued: 0, newest_name: null, unseen: false }) &&
    cardBadge().textContent === "FINISHED" && cardBadge().className === "scheme muted" &&
    cardTotal().textContent === "All downloads finished",
    cardDebug());
  check("0.35.1 start card: an undefined payload renders the empty state, no throw",
    !driveCard(undefined) &&
    !!cardTop() && topIsHidden(cardTop()) &&
    !!cardTotal() && cardTotal().textContent === "No downloads yet",
    cardDebug());
  // A name is TEXT: HTML metacharacters must not become markup.
  const cardPoisonName = "<img src=x>";
  check("0.35.1 start card: a metacharacter name lands as textContent",
    !driveCard({ state: "finished", count: 1, speed_bps: null, paused: 0, queued: 0, newest_name: cardPoisonName, unseen: false }) &&
    !!cardTotal() && cardTotal().textContent === cardPoisonName,
    cardDebug());
  check("0.35.1 start card: the metacharacter name created no element",
    !!cardTotal() && cardTotal().childElementCount === 0 &&
    sDoc.querySelectorAll("#dl-card img").length === 0 &&
    !sDom.window.__pwned);
  sDom.window.close();
}

// ------------------------------------------------------------------
// 0.34.2 HOSTILE DOWNLOADS CORPUS — the permanent fixture
// set born from the 0.34.1 field failure ("cant see the
// download, cant open the page, cant hit the button"). The
// render-fragility hypothesis behind it was KILLED by this
// corpus's first run (63 entries × both surfaces × live +
// JSON-round-tripped pushes: zero throws — the actual root
// cause was the inverted dedupe guard in the shell, pinned
// in tests/media_tests.rs), but the corpus stays: every
// real-data shape the field produces — unknown totals from
// chunked CDN responses, Persian/RTL names, quotes,
// backslashes, emoji, HTML metacharacters, 5000-char server
// error bodies, out-of-range percents, off-list statuses,
// 100 tasks, degenerate snapshots, the legacy bare-array
// shape — must render without a throw, with the panel open
// and the expected row count, on BOTH surfaces, twice (the
// app re-pushes on every progress tick). The block also
// pins the 0.34.2 behavior fixes (G1/G2/G3/G4/G5), the
// per-section render guards' blast radius, and the
// min-window popover CSS caps.
// ------------------------------------------------------------------
{
  const DEL = Symbol("del");
  const cTask = (over = {}) => {
    const t = {
      id: 1, name: "file.zip", host: "files.example.com",
      url: "https://files.example.com/file.zip", status: "downloading",
      total: 100000000, downloaded: 45000000, percent: 45,
      size_text: "100 MB",
      detail_text: "45 MB of 100 MB · 1.2 MB/s · 45 s left",
      connections: 8, error: null, hint: null, retry_text: null,
      retry_pending: false, refreshing: false, sha256: null,
      added_ms: 1730000000000,
    };
    for (const k of Object.keys(over)) {
      if (over[k] === DEL) delete t[k]; else t[k] = over[k];
    }
    return t;
  };
  const cSnap = (tasks, over = {}) => ({
    tasks, active_count: 1, failed_unseen: 0,
    settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 },
    notice: null, ...over,
  });
  const manyTasks = [];
  {
    const sts = ["downloading", "paused", "done", "failed", "queued"];
    for (let i = 1; i <= 100; i++) {
      manyTasks.push(cTask({
        id: i, name: `file-${i}.bin`, status: sts[i % sts.length],
        percent: i % 101, downloaded: i * 1000,
        error: sts[i % sts.length] === "failed" ? "HTTP 500" : null,
      }));
    }
  }
  // [id, downloads value, expected rows on both surfaces]
  const CORPUS = [
    ["unknown-total-rust-shape", cSnap([cTask({ total: null, downloaded: 993000, percent: null, size_text: "Unknown size", detail_text: "970 KB · 0 B/s" })]), 1],
    ["unknown-total-guessed", cSnap([cTask({ total: null, downloaded: 993000, percent: null, size_text: "Unknown", detail_text: "993 KB of Unknown size" })]), 1],
    ["unknown-total-missing-texts", cSnap([cTask({ total: null, downloaded: 993000, percent: null, size_text: DEL, detail_text: DEL })]), 1],
    ["percent-absent", cSnap([cTask({ percent: DEL })]), 1],
    ["percent-0", cSnap([cTask({ percent: 0, downloaded: 0 })]), 1],
    ["percent-100-downloading", cSnap([cTask({ percent: 100, downloaded: 100000000 })]), 1],
    ["percent-137.5", cSnap([cTask({ percent: 137.5, downloaded: 137500000 })]), 1],
    ["percent-negative", cSnap([cTask({ percent: -3, downloaded: 0 })]), 1],
    ["percent-string", cSnap([cTask({ percent: "42" })]), 1],
    ["percent-null-known-total", cSnap([cTask({ total: 1000, downloaded: 500, percent: null, size_text: "1 KB", detail_text: "500 B · 0 B/s" })]), 1],
    ["status-error", cSnap([cTask({ status: "error", error: "boom" })]), 1],
    ["status-empty", cSnap([cTask({ status: "" })]), 1],
    ["status-uppercase", cSnap([cTask({ status: "DOWNLOADING" })]), 1],
    ["status-missing", cSnap([cTask({ status: DEL })]), 1],
    ["status-null", cSnap([cTask({ status: null })]), 1],
    ["name-persian-rtl", cSnap([cTask({ name: "دانلود فایل", host: "dl.soft98.ir" })]), 1],
    ["name-quotes", cSnap([cTask({ name: `a"b'c"d'e.zip`, host: `ho"st'x.example`, url: `https://x.example/a"b'c.zip`, status: "failed", error: `say "hi" and 'bye'` })]), 1],
    ["name-backslashes", cSnap([cTask({ name: "C:\\path\\file.rar", url: "https://files.example.com/C:\\path\\file.rar" })]), 1],
    ["name-emoji", cSnap([cTask({ name: "🎬 movie 🍿 night.mkv", host: "🎬.example.com" })]), 1],
    ["name-300char", cSnap([cTask({ name: "a".repeat(296) + ".zip" })]), 1],
    ["error-5000char", cSnap([cTask({ status: "failed", error: "E".repeat(5000), detail_text: "45 MB of 100 MB · Failed" })]), 1],
    ["html-script-everywhere", cSnap([cTask({ name: "<script>alert(1)</script>.zip", host: "<img src=x onerror=alert(1)>", url: "https://x.example/<script>alert(2)</script>", status: "failed", error: "<script>alert(3)</script>", hint: "<img src=x onerror=alert(4)>" })]), 1],
    ["name-img-onerror", cSnap([cTask({ name: '<img src=x onerror="window.__pwned=1">' })]), 1],
    ["sha-html-done", cSnap([cTask({ status: "done", percent: 100, downloaded: 100000000, sha256: '"><svg onload=alert(1)>' })]), 1],
    ["name-questionmarks", cSnap([cTask({ name: "???" })]), 1],
    ["error-newline", cSnap([cTask({ status: "failed", error: "line1\nline2\r\nline3" })]), 1],
    ["name-u2028", cSnap([cTask({ name: "before after.zip" })]), 1],
    ["name-dot-part", cSnap([cTask({ name: "archive.zip.part" })]), 1],
    ["zero-zero-downloading", cSnap([cTask({ downloaded: 0, total: 0, percent: 0, size_text: "0 B", detail_text: "0 B · 0 B/s" })]), 1],
    ["downloaded-gt-total", cSnap([cTask({ total: 1000, downloaded: 2500, percent: 250, size_text: "1 KB", detail_text: "2.4 KB of 1 KB" })]), 1],
    ["detail-empty", cSnap([cTask({ detail_text: "" })]), 1],
    ["detail-zero-speed", cSnap([cTask({ detail_text: "0 B/s" })]), 1],
    ["detail-infinity", cSnap([cTask({ detail_text: "∞" })]), 1],
    ["detail-verylong", cSnap([cTask({ detail_text: "D".repeat(5000) })]), 1],
    ["hundred-tasks", cSnap(manyTasks, { active_count: 40 }), 100],
    ["notice-long-name", cSnap([cTask({})], { notice: { seq: 999999, kind: "started", name: "N".repeat(500), open: true } }), 1],
    ["notice-weird-kind", cSnap([cTask({})], { notice: { seq: 1000000, kind: "weird", name: "x.zip", open: true } }), 1],
    ["notice-html-reason", cSnap([cTask({})], { notice: { seq: 1000001, kind: "failed", name: "n.zip", reason: '<img src=x onerror="window.__pwned=1">', open: true } }), 1],
    ["notice-missing", (() => { const s = cSnap([cTask({})]); delete s.notice; return s; })(), 1],
    ["notice-null", cSnap([cTask({})], { notice: null }), 1],
    ["settings-empty", cSnap([cTask({})], { settings: {} }), 1],
    ["settings-conn7", cSnap([cTask({})], { settings: { default_connections: 7, max_concurrent: 3, speed_limit_kbps: 0 } }), 1],
    ["settings-no-speed-limit", cSnap([cTask({})], { settings: { default_connections: 8, max_concurrent: 3 } }), 1],
    ["settings-off-list", cSnap([cTask({})], { settings: { default_connections: 8, max_concurrent: 7, speed_limit_kbps: 99999 } }), 1],
    ["added-ms-zero", cSnap([cTask({ added_ms: 0 })]), 1],
    ["added-ms-missing", cSnap([cTask({ added_ms: DEL })]), 1],
    ["added-ms-far-future", cSnap([cTask({ added_ms: 99999999999999 })]), 1],
    ["added-ms-negative", cSnap([cTask({ added_ms: -5 })]), 1],
    ["added-ms-string", cSnap([cTask({ added_ms: "1730000000000" })]), 1],
    ["added-ms-beyond-date", cSnap([cTask({ added_ms: 1000000000000000000 })]), 1],
    ["id-missing", cSnap([cTask({ id: DEL })]), 1],
    ["id-string", cSnap([cTask({ id: "7" })]), 1],
    ["id-duplicates", cSnap([cTask({ id: 11, name: "first.zip" }), cTask({ id: 11, name: "second.zip", status: "paused" })]), 2],
    ["task-null-element", cSnap([null]), 1],
    ["task-string-element", cSnap(["nope"]), 1],
    ["task-number-element", cSnap([42]), 1],
    ["name-numeric", cSnap([cTask({ name: 12345, host: 678, status: "failed", error: 500 })]), 1],
    ["connections-weird", cSnap([cTask({ id: 1, connections: DEL }), cTask({ id: 2, connections: "8", status: "paused" }), cTask({ id: 3, connections: 999999, status: "queued" })]), 3],
    ["snapshot-empty", {}, 0],
    ["tasks-null", { tasks: null, active_count: 0, failed_unseen: 0, settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 }, notice: null }, 0],
    ["tasks-string", { tasks: "nope", active_count: 0, failed_unseen: 0, settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 }, notice: null }, 0],
    ["legacy-bare-array", [cTask({ id: 21, name: "legacy-a.bin" }), cTask({ id: 22, name: "legacy-b.bin", status: "done", percent: 100 })], 2],
    ["downloads-null", null, 0],
  ];
  const cBase = {
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, focusAddr: false,
    palette: "forest", themes: [], paletteOpen: false,
    teleportOpen: false, teleport: null,
    downloadsOpen: false, downloads: cSnap([]),
    contextMenu: null,
  };
  const chromeForCorpus = () => {
    const dom = new JSDOM(chromeHtml, {
      runScripts: "dangerously",
      url: "https://pico.chrome/",
      beforeParse(window) {
        window.ipc = { postMessage: () => {} };
      },
    });
    try { dom.window.document.activeElement.blur(); } catch {}
    return dom;
  };
  const pageForCorpus = () => {
    const dom = new JSDOM(downloadsHtml, {
      runScripts: "dangerously",
      url: "https://pico.downloads/",
      beforeParse(window) {
        window.ipc = { postMessage: () => {} };
      },
    });
    try { dom.window.document.execCommand = () => true; } catch {}
    return dom;
  };

  for (const [id, snap, rows] of CORPUS) {
    // The wire shape is JSON: drive the round-tripped value,
    // exactly what the shell delivers on Windows.
    const wire = JSON.parse(JSON.stringify(snap === undefined ? null : snap));
    {
      const dom = chromeForCorpus();
      const win = dom.window;
      const doc = win.document;
      let threw = null;
      try {
        win.__pico({ ...cBase });
        win.__pico({ ...cBase, downloadsOpen: true, downloads: wire });
        win.__pico({ ...cBase, downloadsOpen: true, downloads: wire });
      } catch (e) { threw = e; }
      const panel = doc.getElementById("dpanel");
      const got = doc.querySelectorAll("#dl-list .dl-row").length;
      check(`0.34.2 corpus chrome: ${id} renders open with ${rows} rows, no throw`,
        threw === null && panel.classList.contains("on") && got === rows && !win.__pwned,
        threw ? String(threw && threw.message || threw) : `panel=${panel.classList.contains("on")} rows=${got}`);
      win.close();
    }
    {
      const dom = pageForCorpus();
      const win = dom.window;
      const doc = win.document;
      let payload;
      if (wire === null) payload = {};
      else if (Array.isArray(wire)) payload = wire;
      else payload = { theme: "light", palette: "forest", ...wire };
      let threw = null;
      try {
        win.__picoSetDownloads(payload);
        win.__picoSetDownloads(payload);
      } catch (e) { threw = e; }
      const got = doc.querySelectorAll("#dl-page-list .dlp-row").length;
      check(`0.34.2 corpus page: ${id} renders ${rows} rows, no throw`,
        threw === null && got === rows && !win.__pwned,
        threw ? String(threw && threw.message || threw) : `rows=${got}`);
      win.close();
    }
  }

  // --- 0.34.2 behavior pins on the fixed data shapes ---
  {
    // G1: the page shows the byte detail AND the countdown
    // as separate lines (no duplication, no lost detail);
    // the popover's single detail slot shows the countdown.
    const retryTask = cTask({
      id: 31, status: "failed", error: "Connection reset",
      detail_text: "25 MB of 85 MB · Failed",
      retry_text: "Retrying in 12 · attempt 1 of 3", retry_pending: true,
    });
    const dom = pageForCorpus();
    const win = dom.window;
    const doc = win.document;
    win.__picoSetDownloads({ theme: "light", palette: "forest", ...cSnap([retryTask]) });
    const row = doc.querySelector("#dl-page-list .dlp-row");
    check("0.34.2 G1: page detail line keeps the byte progress (not the countdown)",
      !!row && row.querySelector(".dlp-detail").textContent === "25 MB of 85 MB · Failed",
      row ? row.querySelector(".dlp-detail").textContent : "no row");
    check("0.34.2 G1: page retry line shows the countdown once",
      !!row && row.querySelector(".dlp-retry").textContent === "Retrying in 12 · attempt 1 of 3");
    win.close();
    const cdom = chromeForCorpus();
    const cwin = cdom.window;
    const cdoc = cwin.document;
    cwin.__pico({ ...cBase, downloadsOpen: true, downloads: cSnap([retryTask]) });
    const crow = cdoc.querySelector("#dl-list .dl-row");
    check("0.34.2 G1: popover detail slot prefers the retry countdown",
      !!crow && crow.querySelector(".dl-prog-text").textContent === "Retrying in 12 · attempt 1 of 3",
      crow ? crow.querySelector(".dl-prog-text").textContent : "no row");
    cwin.close();
  }
  {
    // G2: a paused task carrying an error (failed
    // link-refresh) shows it on the page too.
    const dom = pageForCorpus();
    const win = dom.window;
    const doc = win.document;
    win.__picoSetDownloads({
      theme: "light", palette: "forest",
      ...cSnap([cTask({ id: 32, status: "paused", error: "Link refresh failed (HTTP 403)" })]),
    });
    const row = doc.querySelector("#dl-page-list .dlp-row");
    check("0.34.2 G2: page shows a paused task's error line",
      !!row && !!row.querySelector(".dlp-error") &&
        row.querySelector(".dlp-error").textContent === "Link refresh failed (HTTP 403)",
      row ? row.textContent : "no row");
    win.close();
  }
  {
    // G3: a done task with unknown total (legacy record)
    // draws a FULL bar on both surfaces.
    const doneTask = cTask({
      id: 33, status: "done", total: null, percent: null,
      downloaded: 5000, size_text: "Unknown size", detail_text: "5 KB",
    });
    const dom = pageForCorpus();
    const win = dom.window;
    const doc = win.document;
    win.__picoSetDownloads({ theme: "light", palette: "forest", ...cSnap([doneTask]) });
    const fill = doc.querySelector("#dl-page-list .dlp-fill");
    check("0.34.2 G3: page draws a full bar for a done task with no percent",
      !!fill && fill.style.width === "100%",
      fill ? fill.style.width : "no fill");
    win.close();
    const cdom = chromeForCorpus();
    const cwin = cdom.window;
    const cdoc = cwin.document;
    cwin.__pico({ ...cBase, downloadsOpen: true, downloads: cSnap([doneTask]) });
    const cfill = cdoc.querySelector("#dl-list .dl-fill");
    check("0.34.2 G3: popover draws a full bar for a done task with no percent",
      !!cfill && cfill.style.width === "100%",
      cfill ? cfill.style.width : "no fill");
    cwin.close();
  }
  {
    // G4: an off-list speed limit renders as a real option,
    // never a blank select.
    const dom = pageForCorpus();
    const win = dom.window;
    const doc = win.document;
    win.__picoSetDownloads({
      theme: "light", palette: "forest",
      ...cSnap([cTask({})], {
        settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 768 },
      }),
    });
    const sel = doc.getElementById("dl-speed-limit");
    check("0.34.2 G4: an off-list speed limit (768) still selects a value",
      !!sel && sel.value === "768",
      sel ? `value=${JSON.stringify(sel.value)}` : "no select");
    win.close();
  }
  {
    // G5: an out-of-range added_ms never prints
    // "Invalid Date" on the page.
    const dom = pageForCorpus();
    const win = dom.window;
    const doc = win.document;
    win.__picoSetDownloads({
      theme: "light", palette: "forest",
      ...cSnap([cTask({ id: 34, added_ms: 1000000000000000000 })]),
    });
    const meta = doc.querySelector("#dl-page-list .dlp-meta");
    check("0.34.2 G5: an out-of-range added_ms prints no 'Invalid Date'",
      !!meta && !meta.textContent.includes("Invalid Date"),
      meta ? meta.textContent : "no meta");
    win.close();
  }
  {
    // Blast radius (chrome): a downloads snapshot whose
    // `tasks` getter throws must NOT take the rest of the
    // snapshot application with it — the error is recorded
    // and painted on the diagnosis strip, later sections
    // (the errbar) still apply, and a later healthy push
    // recovers and clears the error.
    const dom = chromeForCorpus();
    const win = dom.window;
    const doc = win.document;
    const poison = {};
    Object.defineProperty(poison, "tasks", {
      enumerable: true,
      get() { throw new Error("POISON_TASKS"); },
    });
    let threw = null;
    try {
      win.__pico({ ...cBase, activeError: "boom", downloadsOpen: true, downloads: poison });
    } catch (e) { threw = e; }
    check("0.34.2 guard: a throwing downloads section does not escape __pico",
      threw === null, threw ? String(threw && threw.message || threw) : "");
    check("0.34.2 guard: the failure is recorded and painted on the strip",
      !!win.__picoErrors && typeof win.__picoErrors["dl-render"] === "string" &&
        doc.getElementById("shellwarn").classList.contains("on") &&
        doc.getElementById("shellwarn").dataset.kind === "error",
      JSON.stringify(win.__picoErrors || {}));
    check("0.34.2 guard: later sections still applied (errbar shows)",
      doc.getElementById("errbar").classList.contains("on"));
    check("0.34.2 guard: only the downloads section is down (panel stayed closed)",
      !doc.getElementById("dpanel").classList.contains("on"));
    win.__pico({ ...cBase, downloadsOpen: true, downloads: cSnap([cTask({ id: 41 })]) });
    check("0.34.2 guard: a healthy push recovers the section and clears the strip",
      doc.getElementById("dpanel").classList.contains("on") &&
        Object.keys(win.__picoErrors).length === 0 &&
        !doc.getElementById("shellwarn").classList.contains("on"));
    win.close();
  }
  {
    // Blast radius (page): a poisoned payload is caught by
    // the render guard and surfaced on the count line.
    const dom = pageForCorpus();
    const win = dom.window;
    const doc = win.document;
    const poison = {};
    Object.defineProperty(poison, "tasks", {
      enumerable: true,
      get() { throw new Error("POISON_PAGE"); },
    });
    let threw = null;
    try { win.__picoSetDownloads(poison); } catch (e) { threw = e; }
    check("0.34.2 guard: a poisoned page payload does not escape __picoSetDownloads",
      threw === null, threw ? String(threw && threw.message || threw) : "");
    check("0.34.2 guard: the page records and surfaces the render failure",
      !!win.__picoErrors && typeof win.__picoErrors.render === "string" &&
        doc.getElementById("dl-count-line").textContent.includes("problem"),
      doc.getElementById("dl-count-line").textContent);
    win.close();
  }
  // Min-window geometry caps (AUDIT 0.34.2 layout 1.6):
  // the popover card is height-capped against the window
  // and the centered cards share the same discipline —
  // jsdom cannot measure layout, so the assembled CSS
  // itself is the assertion surface.
  check("0.34.2 layout: the downloads popover is height-capped to the window",
    chromeHtml.includes("max-height: calc(100vh - 88px)"));
  check("0.34.2 layout: teleport/picker cards cap at min(486px, window)",
    chromeHtml.includes("max-height: min(486px, calc(100vh - 90px))"));
}

// ------------------------------------------------------------------
// 0.35 WS1 + WS5: rich popover rows (progressText with live
// speed/ETA), page parity, the aggregate header speed, the
// popover footer/header upgrades, and the page's Queued
// chip + Open folder + clear-completed gating. Fixtures
// carry speed_bps/eta_secs explicitly (null included) —
// that presence is what switches a task off the legacy
// detail_text fallback.
// ------------------------------------------------------------------
{
  const T35 = (over = {}) => ({
    id: 51, name: "file.bin", host: "files.example.com",
    url: "https://files.example.com/file.bin", status: "downloading",
    total: 21400000, downloaded: 12400000, percent: 57.94,
    size_text: "21.4 MB", detail_text: "LEGACY DETAIL",
    connections: 8, error: null, hint: null, retry_text: null,
    retry_pending: false, refreshing: false, sha256: null,
    added_ms: 1730000000000, speed_bps: null, eta_secs: null, ...over,
  });
  const RICH = [
    T35({ id: 51, speed_bps: 1200000, eta_secs: 9 }),
    T35({ id: 52, total: null, downloaded: 993000, percent: null, size_text: "Unknown size", speed_bps: 500000 }),
    T35({ id: 53, downloaded: 2500, total: 1000, percent: 100 }),
    T35({ id: 54, status: "paused", downloaded: 1000000, total: 5000000, percent: 20, speed_bps: 999999, eta_secs: 42 }),
    T35({ id: 55, status: "queued", downloaded: 0, total: 5000000, percent: 0, size_text: "5 MB" }),
    T35({ id: 56, status: "queued", downloaded: 0, total: 7000000, percent: 0, size_text: "7 MB" }),
    T35({ id: 57, status: "failed", downloaded: 60000000, total: 200000000, percent: 30, error: "HTTP 404" }),
    T35({ id: 58, status: "done", downloaded: 30000000, total: 30000000, percent: 100, detail_text: "30 MB", sha256: "abc123" }),
  ];
  const snap35 = (tasks, over = {}) => ({
    tasks, active_count: 3, failed_unseen: 0,
    settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 },
    notice: null, ...over,
  });

  // --- Formatters + API surface (via the chrome window's DlRows).
  const sent = [];
  const dom = new JSDOM(chromeHtml, {
    runScripts: "dangerously",
    url: "https://pico.chrome/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => sent.push(JSON.parse(m)) };
    },
  });
  const { window } = dom;
  const doc = window.document;
  const D = window.DlRows;
  check("0.35 formatters: DlRows exports the formatters + progressText",
    !!D && typeof D.humanizeBytes === "function" && typeof D.humanizeDuration === "function" &&
    typeof D.fmtSpeed === "function" && typeof D.fmtEta === "function" &&
    typeof D.progressText === "function");
  check("0.35 formatters: humanizeBytes pinned values (incl. the 12.25 HALF-UP tie)",
    D.humanizeBytes(12250) === "12.3 KB" && D.humanizeBytes(999) === "999 B" &&
    D.humanizeBytes(1000) === "1 KB" && D.humanizeBytes(21400000) === "21.4 MB",
    [12250, 999, 1000, 21400000].map((n) => D.humanizeBytes(n)).join(","));
  check("0.35 formatters: humanizeDuration pinned values",
    D.humanizeDuration(9000) === "9 s" && D.humanizeDuration(90000) === "1.5 min" &&
    D.humanizeDuration(3600000) === "1 h",
    [9000, 90000, 3600000].map((n) => D.humanizeDuration(n)).join(","));
  check("0.35 formatters: fmtSpeed / fmtEta compose the mirrors",
    D.fmtSpeed(1200000) === "1.2 MB/s" && D.fmtEta(9) === "9 s" &&
    D.fmtSpeed(null) === "" && D.fmtEta(null) === "",
    D.fmtSpeed(1200000) + " / " + D.fmtEta(9));
  check("0.35 snapshot: normalizeTask picks up speed_bps/eta_secs (snake + camel)",
    D.normalizeTask({ speed_bps: 5, eta_secs: 7 }).speed_bps === 5 &&
    D.normalizeTask({ speed_bps: 5, eta_secs: 7 }).eta_secs === 7 &&
    D.normalizeTask({ speedBps: 11, etaSecs: 13 }).speed_bps === 11 &&
    D.normalizeTask({ speedBps: 11, etaSecs: 13 }).eta_secs === 13);
  check("0.35 progressText: a legacy-shaped task (fields absent) returns detail_text",
    D.progressText({ status: "downloading", downloaded: 45000000, total: 100000000, percent: 45, detail_text: "LEGACY ONLY" }, []) === "LEGACY ONLY");

  // --- Rich popover rows.
  const baseSnap = {
    tabs: [{ id: 1, title: "App", url: "http://localhost:5173/", loading: false, start: false, failed: false }],
    active: 1, theme: "light", address: "http://localhost:5173/",
    canBack: false, canForward: false, maximized: false, services: 0,
    otherListeners: 0, activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.40.1",
    liteOn: false, liteBlocked: 0, focusAddr: false,
    palette: "forest", themes: [], paletteOpen: false,
    teleportOpen: false, teleport: null,
    downloadsOpen: false, downloads: snap35([]),
  };
  const push = (over) => window.__pico({ ...baseSnap, ...over });
  const list = doc.getElementById("dl-list");
  const rowOf = (id) => list.querySelector(`[data-dl-id="${id}"]`);
  const progOf = (id) => rowOf(id).querySelector(".dl-prog-text").textContent;
  const dlMsgs = (a) => sent.filter((m) => m.kind === "dl" && m.action === a);

  push({ downloadsOpen: true, downloads: snap35(RICH) });
  check("0.35 popover: downloading row prints bytes · pct · speed · ETA left",
    progOf(51) === "12.4 MB of 21.4 MB · 58% · 1.2 MB/s · 9 s left", progOf(51));
  check("0.35 popover: unknown-total row prints bytes + speed only (no 'of', no %)",
    progOf(52) === "993 KB · 500 KB/s", progOf(52));
  check("0.35 popover: downloaded>total row caps at 100% and keeps sizes honest",
    progOf(53) === "2.5 KB of 1 KB · 100%", progOf(53));
  check("0.35 popover: paused row shows NO speed even with a stale speed_bps riding along",
    progOf(54) === "1 MB of 5 MB · 20%", progOf(54));
  check("0.35 popover: the newer queued row counts the older queued task ahead of it",
    progOf(55) === "5 MB · Waiting · 1 ahead", progOf(55));
  check("0.35 popover: the older queued row is next up",
    progOf(56) === "7 MB · Waiting · next up", progOf(56));
  check("0.35 popover: failed row prints its bytes line and its error line",
    progOf(57) === "60 MB of 200 MB · 30%" &&
    rowOf(57).querySelector(".dl-error").textContent === "HTTP 404",
    progOf(57));
  check("0.35 popover: done row prints its size and carries open-file + folder on line 1",
    progOf(58) === "30 MB" &&
    !!rowOf(58).querySelector(".dl-name-line .dl-open-file") &&
    !!rowOf(58).querySelector(".dl-name-line .dl-folder") &&
    rowOf(58).querySelector(".dl-act") === null,
    progOf(58));
  check("0.35 popover: line 1 holds exactly the state's buttons (2 live / 3 done), no placeholders",
    rowOf(51).querySelectorAll(".dl-name-line .dl-icon-btn").length === 2 &&
    rowOf(58).querySelectorAll(".dl-name-line .dl-icon-btn").length === 3);
  check("0.35 popover: CSS pricing is name-line 24 / prog-line 22 in the shipped bytes",
    chromeHtml.includes(".dl-name-line { display: flex; align-items: center; gap: 8px; height: 24px; }") &&
    chromeHtml.includes(".dl-prog { display: flex; align-items: center; gap: 6px; height: 22px; }"));
  check("0.35 header: aggregate speed is the SUM of the downloading meters (1.2 MB/s + 500 KB/s)",
    doc.getElementById("dl-agg").textContent === "1.7 MB/s" &&
    doc.getElementById("dl-agg").hidden === false,
    doc.getElementById("dl-agg").textContent);
  check("0.35 footer: Clear finished is visible while a done task exists",
    doc.getElementById("dl-clear-finished").hidden === false);

  // Footer + done-row clicks: exact bodies, id rules.
  sent.length = 0;
  rowOf(58).querySelector(".dl-open-file").click();
  check("0.35 popover: done-row open-file posts {action:open-file, id}",
    dlMsgs("open-file").length === 1 && dlMsgs("open-file")[0].id === 58,
    JSON.stringify(dlMsgs("open-file")));
  doc.getElementById("dl-clear-finished").click();
  check("0.35 footer: Clear finished posts exactly clear-completed (+ chrome envelope)",
    dlMsgs("clear-completed").length === 1 &&
    dlMsgs("clear-completed")[0].kind === "dl" &&
    !("id" in dlMsgs("clear-completed")[0]),
    JSON.stringify(dlMsgs("clear-completed")));
  doc.getElementById("dl-open-folder").click();
  check("0.35 footer: Open downloads folder posts open-folder with NO id property",
    dlMsgs("open-folder").length === 1 && !("id" in dlMsgs("open-folder")[0]),
    JSON.stringify(dlMsgs("open-folder")));

  // One downloading task: no aggregate; no done: footer hides.
  push({ downloadsOpen: true, downloads: snap35([RICH[0]]) });
  check("0.35 header: a single downloading task shows no aggregate",
    doc.getElementById("dl-agg").textContent === "" &&
    doc.getElementById("dl-agg").hidden === true,
    doc.getElementById("dl-agg").textContent);
  check("0.35 footer: Clear finished hides when no task is done",
    doc.getElementById("dl-clear-finished").hidden === true);

  // Legacy fallback through the full render path.
  const legacy = T35({ id: 59, detail_text: "45 MB of 100 MB · 1.2 MB/s · 45 s left" });
  delete legacy.speed_bps;
  delete legacy.eta_secs;
  push({ downloadsOpen: true, downloads: snap35([legacy]) });
  check("0.35 popover: a 0.33-shaped task renders its detail_text verbatim",
    progOf(59) === "45 MB of 100 MB · 1.2 MB/s · 45 s left", progOf(59));

  // Failed + retry_pending: the countdown owns line 3.
  push({ downloadsOpen: true, downloads: snap35([T35({
    id: 60, status: "failed", downloaded: 60000000, total: 200000000, percent: 30,
    error: "HTTP 404", retry_pending: true, retry_text: "Retrying in 12 · attempt 1 of 3",
  })]) });
  check("0.35 popover: failed + retry_pending shows the countdown in line 3, not progress",
    progOf(60) === "Retrying in 12 · attempt 1 of 3" &&
    rowOf(60).querySelector(".dl-error").textContent === "HTTP 404",
    progOf(60));

  // Queued with bytes on the wire: the bytes branch of the line.
  push({ downloadsOpen: true, downloads: snap35([T35({
    id: 61, status: "queued", downloaded: 2000000, total: 10000000, percent: 20, size_text: "10 MB",
  })]) });
  check("0.35 popover: a queued row with bytes prints bytes · pct · waiting",
    progOf(61) === "2 MB of 10 MB · 20% · Waiting · next up", progOf(61));
  const popoverProg = {};
  push({ downloadsOpen: true, downloads: snap35(RICH) });
  RICH.forEach((t) => { popoverProg[t.id] = progOf(t.id); });
  window.close();

  // --- Page parity + WS5 page fixes, on the SAME fixtures.
  const pSent = [];
  const pDom = new JSDOM(downloadsHtml, {
    runScripts: "dangerously",
    url: "https://pico.downloads/",
    beforeParse(window) {
      window.ipc = { postMessage: (m) => pSent.push(JSON.parse(m)) };
    },
  });
  const pWin = pDom.window;
  const pDoc = pDom.window.document;
  pDoc.execCommand = () => true;
  const pay = (tasks, over = {}) => ({
    theme: "light", palette: "forest", tasks,
    active_count: 3, failed_unseen: 0,
    settings: { default_connections: 8, max_concurrent: 3, speed_limit_kbps: 0 },
    notice: null, ...over,
  });
  pWin.__picoSetDownloads(pay(RICH));
  const pList = pDoc.getElementById("dl-page-list");
  const pRowOf = (id) => pList.querySelector(`[data-dl-id="${id}"]`);
  const parityIds = [51, 52, 53, 54, 55, 56, 57, 58];
  check("0.35 parity: the page detail line is IDENTICAL to the popover progress line",
    parityIds.every((id) =>
      pRowOf(id).querySelector(".dlp-detail").textContent === popoverProg[id]),
    parityIds.map((id) =>
      `${id}:${pRowOf(id).querySelector(".dlp-detail").textContent}!=${popoverProg[id]}`).join(" | "));
  check("0.35 parity: queued position survives the page's filtered render path (full-list context)",
    pRowOf(55).querySelector(".dlp-detail").textContent === "5 MB · Waiting · 1 ahead",
    pRowOf(55).querySelector(".dlp-detail").textContent);
  const pDlMsgs = (a) => pSent.filter((m) => m.kind === "dl" && m.action === a);
  pDoc.querySelector('[data-chip="queued"]').click();
  check("0.35 page: the Queued chip filters to queued tasks only",
    pList.querySelectorAll(".dlp-row").length === 2 &&
    [...pList.querySelectorAll(".dlp-row")].every((r) => r.dataset.status === "queued"),
    [...pList.querySelectorAll(".dlp-row")].map((r) => r.dataset.dlId).join(","));
  pDoc.querySelector('[data-chip="all"]').click();
  check("0.35 page: Clear completed is visible while a done task exists",
    pDoc.getElementById("dl-clear-completed").hidden === false);
  pSent.length = 0;
  pDoc.getElementById("dl-open-folder").click();
  check("0.35 page: the settings Open folder button posts open-folder with NO id property",
    pDlMsgs("open-folder").length === 1 && !("id" in pDlMsgs("open-folder")[0]) &&
    JSON.stringify(pDlMsgs("open-folder")[0]) === JSON.stringify({ kind: "dl", action: "open-folder" }),
    JSON.stringify(pDlMsgs("open-folder")));
  pWin.__picoSetDownloads(pay(RICH.filter((t) => t.status !== "done")));
  check("0.35 page: Clear completed hides when no task is done",
    pDoc.getElementById("dl-clear-completed").hidden === true);
  pWin.close();
}

console.log(failures === 0 ? "\nDOM TEST: all checks passed" : `\nDOM TEST: ${failures} FAILURE(S)`);
process.exit(failures === 0 ? 0 : 1);
