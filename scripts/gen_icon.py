#!/usr/bin/env python3
"""Pico icon generator (v24, AUDIT V24-1; recolored per theme era — v27 wears the Forest default).

Draws the Pico mark — a steel-blue rounded-square tile carrying a
bold white "P" — from signed-distance primitives, with NO image
libraries (stdlib only: math, struct, zlib). Deterministic: the
same script version always emits byte-identical assets.

Outputs:
  assets/pico.ico      multi-size icon (16/20/24/32/48/64/128/256,
                       every frame PNG-compressed) — compiled into
                       the exe by llvm-rc via assets/pico.rc
  assets/icon_64.rgba  raw 64x64 RGBA for the RUNTIME window icon
                       (tao Icon::from_rgba, include_bytes! in
                       core::assets)
  visual-check/icon-256.png         the 256 frame, for eyeballing
  visual-check/icon-16-up.png       the 16 frame nearest-neighbor
                                    upscaled x16 — the 16px
                                    legibility check
  visual-check/icon-32-up.png       the 32 frame upscaled x8

Regenerate + compile the resource with:
  python3 scripts/gen_icon.py
  (cd assets && /usr/lib/llvm-20/bin/llvm-rc /fo pico.res pico.rc)
The .res is checked in; the build links it (see .cargo/config.toml)
so NO resource toolchain is needed at build time.
"""

import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MASTER = 512  # master render size; every frame is a downscale

# --- palette (v24 steel-blue accent family, style.css) ---------------
TOP = (0x3E, 0x5A, 0x3F)     # tile gradient top — Forest mid green (v27, default theme)
BOTTOM = (0x1E, 0x3A, 0x2E)  # tile gradient bottom — Forest raised green (v27)
GLYPH = (0xC7, 0xCB, 0xB8)   # the P — Forest pale (v27)
EDGE = (0xFF, 0xFF, 0xFF)    # inner top-left light edge


def sd_round_box(px, py, cx, cy, hx, hy, r):
    """Signed distance to a rounded box (negative inside)."""
    qx = abs(px - cx) - (hx - r)
    qy = abs(py - cy) - (hy - r)
    ax, ay = max(qx, 0.0), max(qy, 0.0)
    return math.hypot(ax, ay) + min(max(qx, qy), 0.0) - r


def sd_segment_ring(px, py):
    """Signed distance to the P's bowl: a ring segment.

    Ring centered on the bowl center; the stem union covers the
    left half, so only the right arc + the parts overlapping the
    stem matter — the union with the stem does the rest.
    """
    d = math.hypot(px - BOWL_CX, py - BOWL_CY)
    return abs(d - BOWL_R) - BOWL_T / 2.0


# Geometry, in master pixels. Canvas 512; tile inset 28/side.
TILE_C, TILE_H, TILE_R = 256.0, 228.0, 96.0
STEM_X0, STEM_X1 = 158.0, 240.0      # stem bar
STEM_Y0, STEM_Y1 = 128.0, 384.0
BOWL_CX, BOWL_CY = 274.0, 206.0      # bowl ring center
BOWL_R, BOWL_T = 80.0, 66.0          # ring radius / thickness
WAIST_Y0, WAIST_Y1 = 206.0, 254.0    # the P's waist bar
WAIST_X1 = 368.0


def sd_glyph(px, py):
    """The P, built the way a bold P is actually structured: a
    stem, a top arc (ring kept above the waist line), and a
    straight waist bar closing the bowl. A bare clipped ring
    reads as a question mark — pass 1's render proved it; the
    waist bar is what makes it a P."""
    stem = sd_round_box(px, py, (STEM_X0 + STEM_X1) / 2.0,
                        (STEM_Y0 + STEM_Y1) / 2.0,
                        (STEM_X1 - STEM_X0) / 2.0,
                        (STEM_Y1 - STEM_Y0) / 2.0, 16.0)
    ring = sd_segment_ring(px, py)
    if py > WAIST_Y1 - 6.0:  # the arc stops at the waist
        ring = 1e9
    waist = sd_round_box(px, py, (STEM_X0 + WAIST_X1) / 2.0,
                         (WAIST_Y0 + WAIST_Y1) / 2.0,
                         (WAIST_X1 - STEM_X0) / 2.0,
                         (WAIST_Y1 - WAIST_Y0) / 2.0, 10.0)
    return min(stem, ring, waist)


def render_master():
    """Render the master RGBA buffer with 1px-band SDF antialiasing."""
    buf = bytearray(MASTER * MASTER * 4)
    for y in range(MASTER):
        py = y + 0.5
        t = y / (MASTER - 1.0)
        base = tuple(round(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3))
        for x in range(MASTER):
            px = x + 0.5
            i = (y * MASTER + x) * 4
            d_tile = sd_round_box(px, py, TILE_C, TILE_C, TILE_H, TILE_H, TILE_R)
            a_tile = min(1.0, max(0.0, 0.5 - d_tile))
            if a_tile <= 0.0:
                continue
            r, g, b = base
            # Inner light edge: a soft band just inside the tile's
            # top-left-facing rim (neumorphic nod, kept subtle).
            edge_band = max(0.0, 1.0 - abs(d_tile + 7.0) / 7.0)
            light_dir = max(0.0, 1.0 - (px + py) / (MASTER * 1.25))
            k = edge_band * light_dir * 0.28
            r = r + (EDGE[0] - r) * k
            g = g + (EDGE[1] - g) * k
            b = b + (EDGE[2] - b) * k
            d_g = sd_glyph(px, py)
            a_g = min(1.0, max(0.0, 0.5 - d_g))
            if a_g > 0.0:
                r = r + (GLYPH[0] - r) * a_g
                g = g + (GLYPH[1] - g) * a_g
                b = b + (GLYPH[2] - b) * a_g
            buf[i] = int(round(r))
            buf[i + 1] = int(round(g))
            buf[i + 2] = int(round(b))
            buf[i + 3] = int(round(255 * a_tile))
    return bytes(buf)


def downscale(src, sw, dw):
    """Area-average downscale of an RGBA buffer (square)."""
    if sw == dw:
        return src
    out = bytearray(dw * dw * 4)
    scale = sw / dw
    for oy in range(dw):
        y0, y1 = oy * scale, (oy + 1) * scale
        for ox in range(dw):
            x0, x1 = ox * scale, (ox + 1) * scale
            acc = [0.0, 0.0, 0.0, 0.0]
            wsum = 0.0
            sy = int(y0)
            while sy < y1:
                wy = min(sy + 1.0, y1) - max(float(sy), y0)
                sx = int(x0)
                while sx < x1:
                    wx = min(sx + 1.0, x1) - max(float(sx), x0)
                    w = wx * wy
                    i = (sy * sw + sx) * 4
                    for c in range(4):
                        acc[c] += src[i + c] * w
                    wsum += w
                    sx += 1
                sy += 1
            o = (oy * dw + ox) * 4
            for c in range(4):
                out[o + c] = int(round(acc[c] / wsum))
    return bytes(out)


def png_encode(rgba, size):
    """Minimal PNG encoder (RGBA, filter 0 rows, zlib)."""
    raw = bytearray()
    stride = size * 4
    for y in range(size):
        raw.append(0)
        raw += rgba[y * stride:(y + 1) * stride]

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        c += struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        return c

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + chunk(b"IEND", b""))


def upscale_nearest(rgba, size, factor):
    out = bytearray(size * factor * size * factor * 4)
    for y in range(size * factor):
        for x in range(size * factor):
            si = ((y // factor) * size + (x // factor)) * 4
            di = (y * size * factor + x) * 4
            out[di:di + 4] = rgba[si:si + 4]
    return bytes(out)


def main():
    master = render_master()
    sizes = [256, 128, 64, 48, 32, 24, 20, 16]
    frames = {s: downscale(master, MASTER, s) for s in sizes}
    pngs = {s: png_encode(frames[s], s) for s in sizes}

    # .ico: ICONDIR + ICONDIRENTRYs + PNG frames.
    ico = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    offset = 6 + 16 * len(sizes)
    blobs = []
    entries = []
    for s in sorted(sizes):
        data = pngs[s]
        entries.append(struct.pack(
            "<BBBBHHII",
            0 if s == 256 else s, 0 if s == 256 else s, 0, 0, 1, 32,
            len(data), offset))
        blobs.append(data)
        offset += len(data)
    for e in entries:
        ico += e
    for b in blobs:
        ico += b
    (ROOT / "assets" / "pico.ico").write_bytes(bytes(ico))
    (ROOT / "assets" / "icon_64.rgba").write_bytes(frames[64])

    vc = ROOT / "visual-check"
    vc.mkdir(exist_ok=True)
    (vc / "icon-256.png").write_bytes(pngs[256])
    (vc / "icon-16-up.png").write_bytes(
        png_encode(upscale_nearest(frames[16], 16, 16), 256))
    (vc / "icon-32-up.png").write_bytes(
        png_encode(upscale_nearest(frames[32], 32, 8), 256))
    print(f"pico.ico {len(ico)} bytes; icon_64.rgba {len(frames[64])} bytes; "
          f"frames {sorted(sizes)}")


if __name__ == "__main__":
    main()
