#!/usr/bin/env python3
"""Pico release gate — GATE V2 (v24, AUDIT V24-1).

History that governs this gate: Pico v11 failed on the user's
Windows with "side-by-side configuration is incorrect" because
of its embedded MANIFEST — an RT_MANIFEST resource produced by
a hand-built .rsrc + lld-link. v13's gate (v1 of this script)
therefore demanded an EMPTY resource directory: no .rsrc at
all. That rule was always a proxy. The forensic fact, proven by
PE dumps and by the user's own C# v7 binary (a FULL .rsrc —
icon + version info — from a real resource compiler, which
launched fine on the same machine), is:

    Activation / SxS failures come from the MANIFEST alone.
    Windows reads RT_MANIFEST to build the activation context;
    icons (RT_ICON / RT_GROUP_ICON) are inert data that no
    activation path ever consults.

Gate v2 therefore RETIRES the "resource directory must be
empty" assertion and replaces it with the precise law:

  * RT_MANIFEST entries in the shipped exe: ZERO. [THE hard law]
  * When the exe-icon route shipped (llvm-rc was available),
    RT_ICON + RT_GROUP_ICON are present — check with
    --expect-icon (absence is then a FAIL; without the flag it
    is reported as informational, for runtime-icon-only builds).
  * Static CRT: no VCRUNTIME / MSVCP / api-ms-win-crt imports.
  * Subsystem: WINDOWS_GUI (2).

Every verdict is printed on its own line; size and md5 are
reported. Usage:
  scripts/check_no_manifest.py [--manifest-only] [--expect-icon] <exe>
Exit code 0 = PASS, 1 = FAIL. `--manifest-only` makes the exit
code reflect the manifest verdict alone (kept for checking
historical binaries that predate the other rules).
"""
import hashlib
import struct
import sys
from collections import Counter
from pathlib import Path

RT_ICON = 3
RT_GROUP_ICON = 14
RT_MANIFEST = 24
IMAGE_SUBSYSTEM_WINDOWS_GUI = 2


def u16(b, o):
    return struct.unpack_from("<H", b, o)[0]


def u32(b, o):
    return struct.unpack_from("<I", b, o)[0]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    manifest_only = "--manifest-only" in sys.argv
    expect_icon = "--expect-icon" in sys.argv
    if len(args) != 1:
        print("usage: check_no_manifest.py [--manifest-only] [--expect-icon] <path-to-exe>")
        return 2
    path = Path(args[0])
    d = path.read_bytes()
    manifest_errors = []  # the hard law
    crt_errors = []       # static-CRT violations
    subsystem_errors = []  # GUI subsystem violations
    icon_errors = []      # only when --expect-icon

    assert d[:2] == b"MZ", "not an MZ executable"
    pe = u32(d, 0x3C)
    assert d[pe:pe + 4] == b"PE\0\0", "not a PE executable"
    nsec = u16(d, pe + 6)
    opt_size = u16(d, pe + 20)
    opt = pe + 24
    magic = u16(d, opt)
    assert magic == 0x20B, "expected PE32+ (x86-64)"
    dd_base = opt + 112  # PE32+ data directories start after 112 bytes
    subsystem = u16(d, opt + 68)

    # --- section table ---
    sections = []  # (vaddr, span, rawptr)
    so = opt + opt_size
    print(f"sections ({nsec}):")
    for i in range(nsec):
        o = so + i * 40
        name = d[o:o + 8].split(b"\0")[0].decode("ascii", "replace")
        vsize, vaddr, rawsize, rawptr = u32(d, o + 8), u32(d, o + 12), u32(d, o + 16), u32(d, o + 20)
        sections.append((vaddr, max(vsize, rawsize), rawptr))
        print(f"  {name:8s} vsize={vsize:#010x} vaddr={vaddr:#010x} rawsize={rawsize}")
    # NOTE (gate v2): a .rsrc section is ALLOWED. What is inside
    # it is what the verdicts below judge.

    def rva2off(rva):
        for vaddr, span, rawptr in sections:
            if vaddr <= rva < vaddr + span:
                return rawptr + (rva - vaddr)
        return None

    # --- resource directory (index 2) ---
    rsrc_rva, rsrc_size = u32(d, dd_base + 16), u32(d, dd_base + 20)
    print(f"resource data directory: RVA={rsrc_rva:#x} size={rsrc_size}")
    type_counts = Counter()
    if rsrc_rva and rsrc_size:
        base = rva2off(rsrc_rva)

        def walk(off, names):
            count = u16(d, off + 12) + u16(d, off + 14)
            for i in range(count):
                e = off + 16 + i * 8
                name_field, data_field = u32(d, e), u32(d, e + 4)
                if name_field & 0x80000000:
                    no = base + (name_field & 0x7FFFFFFF)
                    ln = u16(d, no)
                    nm = d[no + 2:no + 2 + ln * 2].decode("utf-16-le", "replace")
                else:
                    nm = name_field
                if data_field & 0x80000000:
                    walk(base + (data_field & 0x7FFFFFFF), names + [nm])
                else:
                    type_counts[names[0] if names else nm] += 1

        walk(base, [])
    manifest_n = type_counts.get(RT_MANIFEST, 0)
    icon_n = type_counts.get(RT_ICON, 0)
    group_icon_n = type_counts.get(RT_GROUP_ICON, 0)
    if type_counts:
        print(f"resource types present: {dict(sorted(type_counts.items(), key=lambda kv: str(kv[0])))}")
    if manifest_n:
        manifest_errors.append(f"RT_MANIFEST resource present ({manifest_n} entries) — THE hard law")
    if expect_icon and (icon_n == 0 or group_icon_n == 0):
        icon_errors.append(
            f"--expect-icon: RT_ICON x{icon_n}, RT_GROUP_ICON x{group_icon_n} (exe-icon route did not ship)")

    # --- imports (index 1) ---
    imp_rva = u32(d, dd_base + 8)
    dlls = []
    if imp_rva:
        off = rva2off(imp_rva)
        while True:
            name_rva = u32(d, off + 12)
            if name_rva == 0:
                break
            noff = rva2off(name_rva)
            end = d.index(b"\0", noff)
            dlls.append(d[noff:end].decode("ascii", "replace"))
            off += 20
    print(f"imports ({len(dlls)}): {', '.join(dlls)}")
    for dll in dlls:
        low = dll.lower()
        if "vcruntime" in low or "msvcp" in low or low.startswith("api-ms-win-crt"):
            crt_errors.append(f"CRT runtime import present (static CRT violated): {dll}")

    if subsystem != IMAGE_SUBSYSTEM_WINDOWS_GUI:
        subsystem_errors.append(f"subsystem is {subsystem}, expected 2 (WINDOWS_GUI)")

    print(f"size: {len(d)} bytes")
    print(f"md5:  {hashlib.md5(d).hexdigest()}")
    print(f"manifest verdict: {'FAIL' if manifest_errors else 'PASS'} "
          f"(resource directory {'populated' if (rsrc_rva and rsrc_size) else 'empty'}, "
          f"{manifest_n} RT_MANIFEST entries) [gate v2 hard law]")
    if expect_icon or icon_n or group_icon_n:
        print(f"icon verdict: {'FAIL' if icon_errors else 'PASS'} "
              f"(RT_ICON x{icon_n}, RT_GROUP_ICON x{group_icon_n}"
              f"{', expected by --expect-icon' if expect_icon else ''})")
    else:
        print("icon verdict: INFO (no icon resources — runtime-icon-only build)")
    print(f"static-CRT verdict: {'FAIL' if crt_errors else 'PASS'} ({len(crt_errors)} CRT imports)")
    print(f"subsystem verdict: {'FAIL' if subsystem_errors else 'PASS'} "
          f"(subsystem={subsystem}, WINDOWS_GUI=2)")

    if manifest_only:
        failed = bool(manifest_errors)
    else:
        failed = bool(manifest_errors or crt_errors or subsystem_errors or icon_errors)
    if failed:
        print(f"PICO GATE v2: FAIL for {path}:")
        for e in manifest_errors + crt_errors + subsystem_errors + icon_errors:
            print(f"  - {e}")
        return 1
    print(f"PICO GATE v2: PASS for {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
