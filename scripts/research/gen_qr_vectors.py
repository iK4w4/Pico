#!/usr/bin/env python3
"""Generate the golden QR matrices that pin Pico's pure-Rust encoder.

Independent implementation: Nayuki's qrcodegen (single-file Python,
fetched from the canonical QR-Code-generator repository), driven in
forced single-BYTE-segment mode with Ecc.MEDIUM and error boosting
DISABLED, so the reference encodes exactly Pico's profile
(src/core/qr.rs). Output: tests/qr_vectors.inc, consumed by
tests/teleport_lite_qr.rs.

Run from the repository root:
    python3 scripts/research/gen_qr_vectors.py
It rewrites tests/qr_vectors.inc deterministically. If the Rust
encoder and this file ever disagree, the test suite fails — that
is the point of the vectors.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qrcodegen  # noqa: E402

VECTORS = [
    ("short-text", "HELLO"),
    ("https-url", "https://example.com/"),
    ("localhost-port", "http://localhost:5173/"),
    (
        "percent-encoded-query",
        "https://example.org/search?q=%D8%B3%D9%84%D8%A7%D9%85-%D8%AF%D9%86%DB%8C%D8%A7"
        "&page=2&lang=fa&sort=newest-first",
    ),
    (
        "long-url",
        "https://example.com/articles/2026/a-very-long-article-slug-about-local-development"
        "?utm_source=pico&utm_medium=teleport&utm_campaign=v25-research"
        "&session=0123456789abcdef0123456789abcdef&return=%2Fdashboard%3Ftab%3Dstats",
    ),
    # AUDIT V28 (A4-M3): one payload per version band 1..=12, so
    # every version's ECC/block/alignment tables are pinned
    # module-for-module — through v27 only v1/v2/v7/v11 carried
    # matrices and v12 asserted nothing but its version number.
    # Byte-mode EC-M capacities: v1:14 v2:26 v3:42 v4:62 v5:84
    # v6:106 v7:122 v8:152 v9:180 v10:213 v11:251 v12:287.
    ("len-v01", "A" * 5),
    ("len-v02", "B" * 20),
    ("len-v03", "C" * 30),
    ("len-v04", "D" * 50),
    ("len-v05", "E" * 70),
    ("len-v06", "F" * 95),
    ("len-v07", "G" * 110),
    ("len-v08", "H" * 130),
    ("len-v09", "I" * 165),
    ("len-v10", "J" * 195),
    ("len-v11", "K" * 219),
    ("len-v12", "L" * 260),
]


def main() -> None:
    out = []
    for name, text in VECTORS:
        data = text.encode("utf-8")
        seg = qrcodegen.QrSegment.make_bytes(data)
        qr = qrcodegen.QrCode.encode_segments(
            [seg], qrcodegen.QrCode.Ecc.MEDIUM, boostecl=False
        )
        size = qr.get_size()
        rows = []
        for y in range(size):
            rows.append("".join("1" if qr.get_module(x, y) else "0" for x in range(size)))
        out.append(f"=== {name}")
        out.append(f"input: {text}")
        out.append(f"version: {qr.get_version()}")
        out.append(f"mask: {qr.get_mask()}")
        out.append(f"size: {size}")
        out.extend(rows)
        print(f"{name}: {len(data)} bytes -> v{qr.get_version()} mask {qr.get_mask()} size {size}")
    dest = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "tests", "qr_vectors.inc")
    with open(dest, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print("wrote", dest)


if __name__ == "__main__":
    main()
