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
  check("skeleton: version badge is 0.31",
    doc.getElementById("ver").textContent === "0.31",
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
    blockedDownloads: 0,
    downloadNotice: "",
    activeError: "",
    blockedTotal: 7,
    blockedTab: 3,
    shieldOn: true,
    palette: "violet",
    themes: THEMES,
    paletteOpen: false,
    liteOn: false,
    focusAddr: false,
    version: "0.31",
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
  check("snapshot: version badge stays 0.31 (display version)",
    doc.getElementById("ver").textContent === "0.31",
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
    blockedDownloads: 0, downloadNotice: "", activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: false, version: "0.31",
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
    otherListeners: 0, blockedDownloads: 0, downloadNotice: "", activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.31",
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
    otherListeners: 0, blockedDownloads: 0, downloadNotice: "", activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.31",
    liteOn: false, liteBlocked: 0, teleport: null,
    palette: "forest", themes: THEMES16, paletteOpen: false,
  };
  window.__pico({ ...baseSnap });
  check("v27 clusters: root carries the snapshot palette",
    doc.documentElement.getAttribute("data-palette") === "forest",
    doc.documentElement.getAttribute("data-palette"));
  const navIds = [...doc.querySelectorAll("#navbar button, #navbar input")]
    .map((el) => el.id).filter(Boolean);
  const want = ["back", "fwd", "reload", "home", "openfile", "addr", "shield", "lite", "teleport", "palette", "theme"];
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
  check("0.31 picker: cards stand in catalogue order",
    JSON.stringify([...cards].map((c) => c.dataset.id)) ===
    JSON.stringify(THEMES16.map((t) => t.id)),
    [...cards].map((c) => c.dataset.id).join(","));
  check("0.31 picker: panel scrolls its grid (16 cards overflow the sheet)",
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
  check("0.31 picker: the last card (rose) is present with six dots",
    !!roseCard && roseCard.querySelectorAll(".pdot").length === 6);
  roseCard.focus();
  roseCard.click();
  const roseMsgs = sent.filter((m) => m.t === "theme_set" && m.id === "rose");
  check("0.31 picker: last-card click emits theme_set rose",
    roseMsgs.length === 1, JSON.stringify(roseMsgs));
  window.__pico({ ...baseSnap, palette: "rose", paletteOpen: true });
  check("0.31 picker: pressed state reaches the last card",
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
    otherListeners: 0, blockedDownloads: 0, downloadNotice: "", activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.31",
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
    otherListeners: 0, blockedDownloads: 0, downloadNotice: "", activeError: "",
    blockedTotal: 0, blockedTab: 0, shieldOn: true, version: "0.31",
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
  // v27: the fixture INIT carries the forest palette, and the
  // two-argument theme push retokens mode + palette together
  // (the one-argument form stays compatible).
  check("start: fixture INIT applies the forest palette",
    doc.documentElement.getAttribute("data-palette") === "forest",
    doc.documentElement.getAttribute("data-palette"));
  window.__picoSetTheme("dark", "ember");
  check("start: theme push sets mode and palette together",
    doc.documentElement.getAttribute("data-theme") === "dark" &&
    doc.documentElement.getAttribute("data-palette") === "ember");
  window.__picoSetTheme("light");
  check("start: one-argument push still sets the mode only",
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
  check("start: footer reads 'Pico 0.31 · by @iK4w4'",
    doc.querySelector(".footer").textContent.includes("Pico 0.31 · by @iK4w4"));
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
      "≈ 16.7 min of loading prevented · 17.1 s this session",
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
      "≈ 0.8 s of loading prevented · 0.8 s this session" &&
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
// this suite is re-run once the runtime lands.
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

console.log(failures === 0 ? "\nDOM TEST: all checks passed" : `\nDOM TEST: ${failures} FAILURE(S)`);
process.exit(failures === 0 ? 0 : 1);
