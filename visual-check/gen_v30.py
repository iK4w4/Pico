#!/usr/bin/env python3
"""Pico 0.30 render variants (AUDIT 0.30): start pages
carrying the savings band — the dump_pages fixture INIT
already holds a POPULATED book, so populated variants need
only a theme driver; the zero-state variant additionally
pushes an all-zero view through the band's own hook.

Usage: gen_v30.py <pages-dir> <out-dir>
"""
import sys
from pathlib import Path

IPC_STUB = """<script>
window.ipc = { postMessage: function () {} };
window.__picoRequests = [];
</script>
"""

ZERO = {
    "shield": {"blocked": 0, "estBytes": 0, "estDurationMs": 0,
               "prunedEvents": 0, "prunedBytes": 0, "sessionBlocked": 0,
               "sessionEstBytes": 0, "sessionEstDurationMs": 0},
    "lite": {"blocked": 0, "estBytes": 0, "estDurationMs": 0,
             "prunedEvents": 0, "prunedBytes": 0, "sessionBlocked": 0,
             "sessionEstBytes": 0, "sessionEstDurationMs": 0},
}


def start_variant(start_src: str, name: str, mode: str, palette: str,
                  zero: bool, out: Path) -> None:
    import json
    zero_call = ""
    if zero:
        zero_call = (
            '  if (window.__picoSetStats) window.__picoSetStats('
            + json.dumps(ZERO) + ');\n'
        )
    driver = f"""
<script>
window.addEventListener("load", function () {{
  if (window.__picoSetTheme) window.__picoSetTheme("{mode}", "{palette}");
{zero_call}}});
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
    start_src = (pages / "start.html").read_text(encoding="utf-8")
    start_variant(start_src, "forest-dark", "dark", "forest", False, out)
    start_variant(start_src, "forest-dark-zero", "dark", "forest", True, out)
    start_variant(start_src, "ember-light", "light", "ember", False, out)
    start_variant(start_src, "violet-dark", "dark", "violet", False, out)


if __name__ == "__main__":
    main()
