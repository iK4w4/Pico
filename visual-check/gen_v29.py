#!/usr/bin/env python3
"""Pico v28 render variants (AUDIT V28): build chrome/start
pages carrying a full v27 snapshot (palette + themes
catalogue + picker state) for headless-shell screenshots.

Usage: gen_v27.py <pages-dir> <out-dir>
"""
import json
import sys
from pathlib import Path

PALETTES = [
    ("forest", "Forest", ["#0F1F17", "#1E3A2E", "#3E5A3F", "#6B7F67", "#C7CBB8", "#3D4337"]),
    ("ember", "Ember Navy", ["#0B132B", "#1C2541", "#3A506B", "#5BC0DE", "#F4D6B0", "#D97706"]),
    ("sage", "Sage Gold", ["#0B0F0D", "#1F2A1F", "#3E5A3F", "#8CA98C", "#D9C7A3", "#B08D57"]),
    ("lagoon", "Lagoon", ["#0A0F14", "#0F3D3E", "#2E6F73", "#7FB3B8", "#EADCC8", "#C6601F"]),
    ("ultraviolet", "Ultraviolet", ["#1A0F2E", "#3B1B4F", "#7A2E6F", "#D6598F", "#F4D7D4", "#FF6B6B"]),
    ("indigo", "Indigo Amber", ["#1A1A1F", "#2D2B55", "#6A5ACD", "#A678B9", "#F0D9B5", "#F4A62A"]),
    ("graphite", "Graphite Pink", ["#121212", "#1F2937", "#334155", "#64748B", "#F1F5F9", "#EC4899"]),
    ("violet", "Violet", ["#0E0B1D", "#2A1E4D", "#5A3B8C", "#8B7CC7", "#C9C2E3", "#3A3A52"]),
    ("bronze", "Bronze", ["#1A120E", "#3A2418", "#6B4A2E", "#A88462", "#D5C4AE", "#4A4A46"]),
]
THEMES = [{"id": i, "name": n, "slots": s} for i, n, s in PALETTES]

IPC_STUB = """<script>
window.ipc = { postMessage: function () {} };
window.__picoRequests = [];
</script>"""

SNAP = {
    "tabs": [
        {"id": 1, "title": "Start", "url": "pico://start", "loading": False, "start": True, "failed": False},
        {"id": 2, "title": "Fixture App", "url": "http://localhost:5173/", "loading": False, "start": False, "failed": False},
        {"id": 3, "title": "A Very Long Fixture Title That Must Ellipsize Gracefully", "url": "http://localhost:8080/deep/path", "loading": False, "start": False, "failed": False},
        {"id": 4, "title": "MDN Web Docs", "url": "https://developer.mozilla.org/", "loading": False, "start": False, "failed": False},
    ],
    "active": 2,
    "address": "http://localhost:5173/",
    "canBack": True, "canForward": False, "maximized": False,
    "services": 2, "otherListeners": 3,
    "blockedDownloads": 0, "blockedTotal": 128, "blockedTab": 12,
    "shieldOn": True, "downloadNotice": "", "activeError": "",
    "liteOn": True, "liteBlocked": 5, "teleport": None, "teleportOpen": False,
    "themes": THEMES,
    "version": "29.0.0",
}

CHROME_DRIVER = """
<script>
window.addEventListener("load", function () {
  var addr = document.getElementById("addr");
  if (addr) addr.blur();
  window.__pico(%s);
});
</script>
"""

TALL = "<style>html, body { height: 100% !important; }</style>"


QR_MATRIX = [
    "1111111010101111111",
    "1000101010101000101",
    "1011101010101011101",
    "1011101001001011101",
    "1011101010101011101",
    "1000101010101000101",
    "1111111010101111111",
]


def chrome_variant(chrome_src: str, name: str, mode: str, palette: str,
                   picker_open: bool, out: Path, teleport_open: bool = False) -> None:
    snap = dict(SNAP)
    snap["theme"] = mode
    snap["palette"] = palette
    snap["paletteOpen"] = picker_open
    if teleport_open:
        snap["teleportOpen"] = True
        snap["teleport"] = {
            "url": "http://192.168.1.20:5173/",
            "note": "Phone must be on the same Wi-Fi. If it won't load, your dev server is bound to this PC only \u2014 restart it with --host (Vite) / -H 0.0.0.0 (Next) and re-open.",
            "matrix": QR_MATRIX,
        }
    driver = CHROME_DRIVER % json.dumps(snap)
    html = chrome_src.replace("<head>", "<head>" + IPC_STUB, 1)
    html = html.replace("</head>", TALL + "</head>", 1)
    html = html.replace("</body>", driver + "</body>", 1)
    p = out / f"chrome-{name}.html"
    p.write_text(html, encoding="utf-8")
    print("wrote", p, len(html), "bytes")


def start_variant(start_src: str, name: str, mode: str, palette: str, out: Path) -> None:
    driver = f"""
<script>
window.addEventListener("load", function () {{
  if (window.__picoSetTheme) window.__picoSetTheme("{mode}", "{palette}");
}});
</script>
"""
    html = start_src.replace("<head>", "<head>" + IPC_STUB, 1)
    html = html.replace("</body>", driver + "</body>", 1)
    p = out / f"start-{name}.html"
    p.write_text(html, encoding="utf-8")
    print("wrote", p, len(html), "bytes")


def main() -> None:
    pages = Path(sys.argv[1])
    out = Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    chrome_src = (pages / "chrome.html").read_text(encoding="utf-8")
    start_src = (pages / "start.html").read_text(encoding="utf-8")
    chrome_variant(chrome_src, "forest-dark", "dark", "forest", False, out)
    chrome_variant(chrome_src, "forest-light", "light", "forest", False, out)
    chrome_variant(chrome_src, "violet-dark", "dark", "violet", False, out)
    chrome_variant(chrome_src, "ember-dark", "dark", "ember", False, out)
    chrome_variant(chrome_src, "picker-forest-dark", "dark", "forest", True, out)
    chrome_variant(chrome_src, "picker-ultraviolet-dark", "dark", "ultraviolet", True, out)
    chrome_variant(chrome_src, "teleport-forest-dark", "dark", "forest", False, out, teleport_open=True)
    start_variant(start_src, "forest-dark", "dark", "forest", out)
    start_variant(start_src, "forest-light", "light", "forest", out)
    start_variant(start_src, "ember-dark", "dark", "ember", out)


if __name__ == "__main__":
    main()
