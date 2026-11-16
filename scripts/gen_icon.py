#!/usr/bin/env python3
"""Pico icon resampler (0.41.3).

The artwork is the user's own master — assets/icon-master.png
(1024x1024 RGBA) — and is NEVER redrawn here. Every shipped
size is a faithful resample of that master; there is no
drawing or palette code in this script.

Pipeline: float-precision premultiplied-alpha Lanczos.
Premultiplying before the filter (and dividing back after)
keeps edge pixels' colour — a naive RGBA resize lets the
black RGB of transparent pixels bleed into the fringe.

Outputs:
  assets/pico.ico      multi-size icon (16/20/24/32/48/64/128/256,
                       every frame PNG-compressed) — compiled into
                       the exe by llvm-rc via assets/pico.rc
  assets/icon_64.rgba  raw 64x64 RGBA for the RUNTIME window icon
                       (tao Icon::from_rgba, include_bytes! in
                       core::assets)

Requires: Pillow and numpy (maintainer tool only; the build
itself needs neither — the outputs are checked in).

Regenerate + compile the resource with:
  python3 scripts/gen_icon.py
  (cd assets && ~/workspace/tools/llvm-18/bin/llvm-rc /fo pico.res pico.rc)
The .res is checked in; the build links it (see .cargo/config.toml)
so NO resource toolchain is needed at build time.
"""

import io
import struct
import sys
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MASTER = ROOT / "assets" / "icon-master.png"
ICO_SIZES = [16, 20, 24, 32, 48, 64, 128, 256]


def _res_f(plane: np.ndarray, size: int) -> np.ndarray:
    img = Image.fromarray(plane.astype(np.float32), mode="F")
    return np.asarray(img.resize((size, size), Image.LANCZOS)).astype(np.float64)


def resample(master: np.ndarray, size: int) -> np.ndarray:
    """master: float64 HxWx4 RGBA (0..255). Returns uint8 size x size x 4."""
    alpha = master[..., 3] / 255.0
    out = np.zeros((size, size, 4), dtype=np.float64)
    for c in range(3):
        out[..., c] = _res_f(master[..., c] * alpha, size)
    out[..., 3] = _res_f(master[..., 3], size)
    a = out[..., 3] / 255.0
    for c in range(3):
        out[..., c] = np.where(a > 0, out[..., c] / np.maximum(a, 1e-12), 0)
    return np.clip(np.floor(out + 0.5), 0, 255).astype(np.uint8)


def png_bytes(rgba: np.ndarray) -> bytes:
    buf = io.BytesIO()
    Image.fromarray(rgba, mode="RGBA").save(buf, format="PNG")
    return buf.getvalue()


def build_ico(master: np.ndarray) -> bytes:
    frames = [png_bytes(resample(master, s)) for s in ICO_SIZES]
    out = io.BytesIO()
    out.write(struct.pack("<HHH", 0, 1, len(ICO_SIZES)))
    offset = 6 + 16 * len(ICO_SIZES)
    for s, frame in zip(ICO_SIZES, frames):
        # Width/height bytes: 0 means 256.
        out.write(struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32,
                              len(frame), offset))
        offset += len(frame)
    for frame in frames:
        out.write(frame)
    return out.getvalue()


def main() -> int:
    master_img = Image.open(MASTER).convert("RGBA")
    if master_img.size != (1024, 1024):
        print(f"master must be 1024x1024, got {master_img.size}", file=sys.stderr)
        return 1
    master = np.asarray(master_img).astype(np.float64)

    ico = build_ico(master)
    (ROOT / "assets" / "pico.ico").write_bytes(ico)
    print(f"assets/pico.ico      {len(ico)} bytes, {len(ICO_SIZES)} PNG frames {ICO_SIZES}")

    rgba64 = resample(master, 64).tobytes()
    assert len(rgba64) == 64 * 64 * 4
    (ROOT / "assets" / "icon_64.rgba").write_bytes(rgba64)
    print(f"assets/icon_64.rgba  {len(rgba64)} bytes, raw 64x64 RGBA")
    print("now recompile the resource:")
    print("  (cd assets && ~/workspace/tools/llvm-18/bin/llvm-rc /fo pico.res pico.rc)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
