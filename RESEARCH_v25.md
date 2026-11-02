# Pico v25 — Research (written BEFORE feature code; findings govern the build)

Two features: **Phone Teleport** (QR of the current tab, LAN-rewritten
for loopback dev servers) and **Lite Mode** (block images/fonts/media
on public pages to save data). Constraints inherited from v24: Rust +
tao/wry only, single exe, **no new crates** (none were needed), Shield
lists byte-identical, all security laws unchanged.

---

## (a) QR encoding — pure Rust, byte mode, EC level M, versions 1–12

Implemented from the QR Code Model 2 specification structure (ISO/IEC
18004): a single byte-mode segment, automatic smallest-version
selection, Reed–Solomon error correction over GF(256), interleaved
blocks, all 8 masks tried and chosen by the specification's penalty
evaluation. No QR crate exists in the dependency tree and none was
added — the encoder is `src/core/qr.rs`, small and fully unit-tested.

### Construction details that matter

- **GF(256)**: primitive polynomial `x^8+x^4+x^3+x^2+1` (0x11D),
  generator α=2. Multiplication is the shift-and-reduce (Russian
  peasant) form, so no exp/log tables are materialized. The RS
  divisor for degree *d* is `∏_{i=0..d-1} (x − α^i)`; the remainder
  is synthetic division of `data·x^d` by that divisor. This is the
  same construction the reference implementation uses (see vectors).
- **Segment**: mode indicator `0100`, character-count indicator of
  8 bits for versions 1–9 and 16 bits for versions 10+, the raw
  UTF-8 bytes, an up-to-4-bit zero terminator, zero padding to a
  byte boundary, then alternating pad bytes `0xEC 0x11`.
- **Capacity (EC level M)**, cross-checked against the reference
  tables (ECC codewords per block / number of blocks) and the raw
  module count formula `(16v+128)·v+64` minus alignment and
  version-info reservations:

  | v | data cw | ecc/block | blocks | byte capacity |
  |---|---------|-----------|--------|---------------|
  | 1 | 16  | 10 | 1 | 14  |
  | 2 | 28  | 16 | 1 | 26  |
  | 3 | 44  | 26 | 1 | 42  |
  | 4 | 64  | 18 | 2 | 62  |
  | 5 | 86  | 24 | 2 | 84  |
  | 6 | 108 | 16 | 4 | 106 |
  | 7 | 124 | 18 | 4 | 122 |
  | 8 | 154 | 22 | 4 | 152 |
  | 9 | 182 | 22 | 5 | 180 |
  | 10| 216 | 26 | 5 | 213 |
  | 11| 254 | 30 | 5 | 251 |
  | 12| 290 | 22 | 8 | 287 |

  Unequal block groups are handled by the standard arithmetic:
  with `R` total codewords and `n` blocks, the short block length
  is `R / n`; the first `n − (R mod n)` blocks are short, the rest
  carry one extra data codeword. Interleaving is column-wise over
  data codewords (skipping the short blocks' phantom byte), then
  column-wise over ECC codewords. Remainder bits: 0 for versions
  1 and 7–12, 7 for versions 2–6.
- **Function patterns**: timing, three finders with separators,
  alignment patterns at the positions given by the standard
  step formula (v2 `[6,18]`, v3 `[6,22]`, v4 `[6,26]`, v5 `[6,30]`,
  v6 `[6,34]`, v7 `[6,22,38]`, v8 `[6,24,42]`, v9 `[6,26,46]`,
  v10 `[6,28,50]`, v11 `[6,30,54]`, v12 `[6,32,58]`), the always-dark
  module at `(8, 4v+9)`.
- **Format information** is computed, not table-copied: the 5 data
  bits are `(formatbits(M)=0b00 << 3) | mask`, the 10 BCH bits come
  from dividing by the generator `0x537`, and the 15-bit result is
  XORed with `0x5412`, then placed in both standard copies.
- **Version information** (v ≥ 7) is likewise computed: 6 version
  bits plus 12 BCH bits from the generator `0x1F25`, placed in both
  3×6 blocks.
- **Mask selection**: the specification's four penalty rules with
  weights N1=3, N2=3, N3=40, N4=10, including the finder-like
  pattern rule evaluated with the run-history method padded by a
  virtual quiet zone of one symbol width. All 8 masks are drawn
  (format bits included, since they are part of the evaluated
  symbol) and the lowest score wins; ties go to the lowest mask
  number.

### Test vectors — the contract

Golden matrices were generated with an **independent reference
implementation**: Nayuki's `qrcodegen` (single-file Python,
`scripts/research/qrcodegen.py`, fetched from the project's
canonical repository for exactly this purpose), driven in forced
single-BYTE-segment mode with `Ecc.MEDIUM` and error-boosting
disabled, so the inputs exercise precisely Pico's encoder profile.
`scripts/research/gen_qr_vectors.py` emits `tests/qr_vectors.inc`;
`tests/teleport_lite_qr.rs` pins every vector end-to-end
(size, version, chosen mask, and every module):

1. `HELLO` (short text → v1)
2. `https://example.com/` (an https URL)
3. `http://localhost:5173/` (a localhost:port URL)
4. A percent-encoded non-ASCII search URL (110 bytes → v7,
   multi-block interleaving)
5. A 219-byte URL with a long query (→ v11: 16-bit count
   indicator, version-information blocks, five blocks)

AUDIT V28 correction: items 4–5 above previously read "v6" and
"v10" — the vectors were actually v7 and v11; the doc had
drifted from the generator. V28 also adds twelve length-band
vectors (`len-v01`…`len-v12`), one per version, so all twelve
versions' ECC/block/alignment tables are pinned
module-for-module (17 vectors total; the test asserts the
exact count and full 1..=12 coverage).

A QR that scans wrong is worse than none; if Pico's encoder and
the reference ever disagree, the build stops here. (Scanner-side
verification on a physical phone remains an on-device verdict —
see BUILD_NOTES' ran-vs-not-run.)

### Capacity honesty

URLs longer than 287 bytes at level M do not fit versions 1–12.
Teleport returns an explicit error for those and the panel shows a
"link too long for a QR" note with the URL still copyable as text —
never a truncated or broken QR.

---

## (b) LAN IPv4 enumeration on Windows

- **Primary route**: `GetAdaptersAddresses` (iphlpapi — already
  linked for the detector; `windows-sys` already enables
  `Win32_NetworkManagement_IpHelper` + `Win32_Networking_WinSock`,
  so **no manifest of new features was needed**). Family
  `AF_INET`, two-call sizing pattern (`ERROR_BUFFER_OVERFLOW`
  first), walking `IP_ADAPTER_ADDRESSES_LH` → `FirstUnicastAddress`,
  reading the `SOCKET_ADDRESS` as `SOCKADDR_IN` and the adapter's
  `OperStatus` (`IfOperStatusUp = 1`). Only address + up/down state
  are read; descriptions and friendly names are never touched.
  (Build note: in windows-sys 0.59 these items are additionally
  gated on the `Win32_NetworkManagement_Ndis` feature — one more
  feature flag on the existing crate, enabled in v25's
  Cargo.toml; still no new dependency.)
- **Pure chooser** (`core::teleport::choose_lan_ip`, headlessly
  tested): drop loopback, 0.0.0.0, and link-local 169.254.0.0/16
  (APIPA addresses cannot route phone traffic in practice). Among
  survivors, prefer RFC1918 tiers in the order `192.168.0.0/16` >
  `10.0.0.0/8` > `172.16.0.0/12` > any other (public/CGNAT —
  kept, because some home routers hand out CGNAT space that still
  routes locally); inside a tier, prefer adapters that are Up;
  remaining ties keep enumeration order (Windows returns adapters
  in route-preference order, so order is a sane final tie-break).
- **Fallback**: if enumeration yields no candidate, a
  UDP-connect egress probe (std `UdpSocket::connect` to a public
  address, then `local_addr` — no packet is ever sent by a UDP
  connect) yields the egress interface's address. Also the only
  route on non-Windows hosts (where Teleport is not a shipped
  path, but the code stays honest).
- **Privacy**: the chosen address is used only to build the URL
  baked into the QR the *user* scans on the panel, in-process.
  Nothing is transmitted anywhere, no adapter inventory is logged
  (the teleport log line carries only the outcome kind and a
  redacted host, per the existing log laws).

---

## (c) Dev-server bind reality (governs the panel note)

Researched 2026-11-01 from vendor docs/discussions:

- **Vite** (and Vite-based tools): dev server binds `localhost`
  (loopback) by default; the terminal prints "Network: use --host
  to expose". Restart flag: `--host` (or `server.host: true`).
- **Next.js**: `next dev` binds **all interfaces by default**
  unless `-H` narrows it (vercel/next.js discussion #64650), so a
  rewritten LAN URL usually just works; `-H 0.0.0.0` is the
  explicit pin, `-H 127.0.0.1` the lockdown.
- **Astro**: loopback by default; `--host` exposes.
- **Python `http.server`**: binds all interfaces by default;
  `--bind 127.0.0.1` restricts.
- **webpack-dev-server** and others vary by version, so the panel
  note names the two most common fixes and keeps the general rule
  first: same Wi-Fi, and if the page won't load the server is
  bound to this PC only.

**Panel note (LanRewrite), as specified and shipped**: "Phone must
be on the same Wi-Fi. If it won't load, your dev server is bound
to this PC only — restart it with --host (Vite) / -H 0.0.0.0
(Next) and re-open."

---

## (d) WebView2 resource contexts for Lite Mode

`COREWEBVIEW2_WEB_RESOURCE_CONTEXT` (Microsoft Learn, WebView2
Win32 IDL reference): ALL=0, DOCUMENT=1, STYLESHEET=2, IMAGE=3,
MEDIA=4, FONT=5, SCRIPT=6, XML_HTTP_REQUEST=7, FETCH=8,
TEXT_TRACK=9, EVENT_SOURCE=10, WEBSOCKET=11, MANIFEST=12,
SIGNED_EXCHANGE=13, PING=14, CSP_VIOLATION_REPORT=15, OTHER=16.

The existing handler already receives every context
(`..._CONTEXT_ALL` filter) and the Shield's
`resource_type_from_context` already maps 3→`Image`, 4→`Media`,
5→`Font` (and 7|8→`Xhr`, 1→`Document`, …). **Lite reuses exactly
that classification** (`shield::ResourceType`), applied after the
Shield's decision in the same handler: classes `{Image, Font,
Media}` are Lite's block set — no parallel mapping exists to drift
out of sync.
