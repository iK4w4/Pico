#!/usr/bin/env python3
"""Pico 0.31 render variants: the 16-card theme picker (Forest
dark at 1280 / 2048 / narrow 760), three of the new themes as
chrome + start (Deep Teal dark, Rose light, Deep Teal light —
the A3-F1 edge-visibility check), and the savings band under a
new theme (Royal Blue light, populated fixture book).

Usage: gen_v32.py <pages-dir> <out-dir>
Pages come from examples/dump_pages (chrome.html, start.html).
"""
import json
import sys
from pathlib import Path

IPC_STUB = """<script>
window.ipc = { postMessage: function () {} };
window.__picoRequests = [];
</script>
"""

THEMES = [
    ("forest", "Forest", ["#0F1F17", "#1E3A2E", "#3E5A3F", "#6B7F67", "#C7CBB8", "#3D4337"]),
    ("ember", "Ember Navy", ["#0B132B", "#1C2541", "#3A506B", "#5BC0DE", "#F4D6B0", "#D97706"]),
    ("sage", "Sage Gold", ["#0B0F0D", "#1F2A1F", "#3E5A3F", "#8CA98C", "#D9C7A3", "#B08D57"]),
    ("lagoon", "Lagoon", ["#0A0F14", "#0F3D3E", "#2E6F73", "#7FB3B8", "#EADCC8", "#C6601F"]),
    ("ultraviolet", "Ultraviolet", ["#1A0F2E", "#3B1B4F", "#7A2E6F", "#D6598F", "#F4D7D4", "#FF6B6B"]),
    ("indigo", "Indigo Amber", ["#1A1A1F", "#2D2B55", "#6A5ACD", "#A678B9", "#F0D9B5", "#F4A62A"]),
    ("graphite", "Graphite Pink", ["#121212", "#1F2937", "#334155", "#64748B", "#F1F5F9", "#EC4899"]),
    ("violet", "Violet", ["#0E0B1D", "#2A1E4D", "#5A3B8C", "#8B7CC7", "#C9C2E3", "#3A3A52"]),
    ("bronze", "Bronze", ["#1A120E", "#3A2418", "#6B4A2E", "#A88462", "#D5C4AE", "#4A4A46"]),
    ("teal", "Deep Teal", ["#0B3D3B", "#0E6860", "#14B8A6", "#99F6E4", "#E6FFFB", "#C0FBF0"]),
    ("mist", "Mist Slate", ["#191D23", "#2F3A41", "#57707A", "#7B919C", "#989DAA", "#C5BAC4"]),
    ("lavender", "Lavender", ["#0D0B2D", "#282177", "#5B4BFF", "#A78BFA", "#E6E0FF", "#C7B6FD"]),
    ("royal", "Royal Blue", ["#0B1F3B", "#143779", "#2563EB", "#60A5FA", "#DBEAFE", "#9EC8FC"]),
    ("tangerine", "Tangerine", ["#1F1D2B", "#6D3E1C", "#FF7A00", "#FFA94D", "#FFE4C7", "#FFC78A"]),
    ("amethyst", "Amethyst", ["#5B21B6", "#6C36CC", "#8B5CF6", "#C7B3FF", "#EDE9FE", "#DACEFF"]),
    ("rose", "Rose", ["#6D0D3F", "#99225F", "#EC4899", "#F9A8D4", "#FCE7F3", "#FBC8E4"]),
]


def snapshot(mode: str, palette: str, palette_open: bool) -> dict:
    return {
        "tabs": [
            {"id": 1, "title": "Fixture App", "url": "http://localhost:5173/",
             "loading": False, "start": False, "failed": False},
            {"id": 2, "title": "Docs", "url": "https://example.com/docs",
             "loading": False, "start": False, "failed": False},
        ],
        "active": 1,
        "theme": mode,
        "palette": palette,
        "themes": [{"id": i, "name": n, "slots": s} for i, n, s in THEMES],
        "address": "http://localhost:5173/",
        "canBack": True,
        "canForward": False,
        "maximized": False,
        "services": 3,
        "otherListeners": 5,
        "blockedDownloads": 0,
        "downloadNotice": "",
        "activeError": "",
        "blockedTotal": 128,
        "blockedTab": 12,
        "shieldOn": True,
        "liteOn": True,
        "liteBlocked": 4,
        "paletteOpen": palette_open,
        "teleportOpen": False,
        "teleport": None,
        "focusAddr": False,
        "version": "0.31",
    }


def chrome_variant(chrome_src: str, name: str, mode: str, palette: str,
                   palette_open: bool, out: Path) -> None:
    snap = json.dumps(snapshot(mode, palette, palette_open))
    driver = f"""
<script>
window.addEventListener("load", function () {{
  if (window.__pico) window.__pico({snap});
}});
</script>
"""
    html = chrome_src.replace("<head>", "<head>" + IPC_STUB, 1)
    html = html.replace("</body>", driver + "</body>", 1)
    p = out / f"chrome-{name}.html"
    p.write_text(html, encoding="utf-8")
    print("wrote", p, len(html), "bytes")


def start_variant(start_src: str, name: str, mode: str, palette: str,
                  out: Path) -> None:
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
    # The picker with all 16 cards, Forest dark (fresh-install
    # default), at desktop, ultrawide and narrow widths.
    chrome_variant(chrome_src, "picker-forest-dark", "dark", "forest", True, out)
    # New themes as chrome (panel closed).
    chrome_variant(chrome_src, "teal-dark", "dark", "teal", False, out)
    chrome_variant(chrome_src, "teal-light", "light", "teal", False, out)
    chrome_variant(chrome_src, "rose-light", "light", "rose", False, out)
    # New themes as start pages; royal-light carries the
    # populated savings-band fixture from dump_pages.
    start_variant(start_src, "teal-dark", "dark", "teal", out)
    start_variant(start_src, "teal-light", "light", "teal", out)
    start_variant(start_src, "rose-light", "light", "rose", out)
    start_variant(start_src, "royal-light", "light", "royal", out)


if __name__ == "__main__":
    main()
