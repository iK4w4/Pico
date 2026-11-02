# Pico 0.31 — Public Release (2026-11-02)

## What this edition is

The public tree. Features, laws, UI, and DISPLAY_VERSION
("0.31") are identical to the private 0.31 build; exactly
three things differ:

1. **List delivery.** The private build embeds the four
   filter lists (EasyList, EasyPrivacy, uBlock filters +
   privacy) in the exe. This edition contains NO list
   content anywhere — not in the repo, not in the exe.
   The Shield resolves its lists at runtime
   (`src/core/lists.rs`): a valid cache in `data/lists/`
   beside the exe first, else an HTTPS download of the
   four canonical URLs (SOURCES.md) through WinHTTP —
   the Windows OS stack, via the already-present
   `windows-sys` surface; no new dependencies (flate2 was
   removed). Downloads are size-capped while streaming,
   validated (shape + rule ratio + the compiler's own
   sanity floors) before anything is cached, and cached
   atomically (temp + rename). Any failure logs
   `shield: lists unavailable (…); Shield idle until
   lists land` and retries on next launch plus once after
   60 s; until lists land the Shield is idle (fail-open)
   and Pico otherwise fully works. Caps carry ~3–4×
   headroom over the genuine sizes verified 2026-11-02
   (easylist.to's current file is 714,159 bytes; the
   official ABP mirror's fuller variant is 2,043,354 —
   both fit).
2. **Test corpus.** Every fixture that quoted the bundled
   lists was replaced by a hand-written SYNTHETIC corpus
   (`tests/fixtures/`, invented hosts on .example/.invalid
   plus a hand-written video-site section driving the DOM
   suite). The semantics matrix is unchanged; counts are
   corpus-relative. Totals: **400 Rust + 269 DOM** (private
   0.31: 393 + 269 — the delta is +7 new `core::lists`
   tests; list-coupled tests were replaced one-for-one,
   and every non-list suite is unchanged in count).
3. **Repo files.** LICENSE (MIT, the author's own code),
   a public README.md, SOURCES.md replacing FILTERS.md,
   .gitignore, and a publication scrub (machine paths,
   internal process references, third-party marks in
   comments/test data). AUDIT.md carries a provenance
   addendum at its head; SECURITY.md a matching delta.

## Download-path verification (in-environment)

The acquire path (URL set → validation → compile → atomic
cache → cache-only recompile) was driven end-to-end by
`examples/fetch_lists.rs` against real list content fetched
in this environment: the two uBlock lists came byte-exact
from the canonical GitHub URLs Pico itself uses; the
EasyList pair came complete from the official ABP mirror
(easylist-downloads.adblockplus.org) because this sandbox's
network truncates easylist.to transfers mid-body (the host
answers — 200/206 with correct Content-Range — but bodies
arrive cut). That mirror serves a FULLER distribution
variant than easylist.to's optimized one (rule-set diff
verified: only ~23k of ~50k/30k `||` rules are shared), so
its compile counts are NOT comparable to the private
build's: **102,921 network rules / 1,738 exceptions /
32,718 cosmetic / 2,912 scriptlet entries**, vs the
private build's 40,716 / 15,453 from easylist.to's variant.
On the user's machine Pico fetches easylist.to's variant
and lands on the private build's numbers (modulo list
drift). The WinHTTP transport itself is compile-verified
for the Windows target (clippy-clean) and logic-reviewed;
it cannot execute in this Linux environment.

## Verification

Tests: **400 Rust + 269 DOM**, all passing. Clippy: 0
warnings, host `--all-targets` and Windows target.
`cargo fmt --check` clean. Zero unwrap/expect/panic in
shipped source (scan; test modules excepted, as before).
Battery: `scripts/battery.sh 10` — **10 consecutive PASS**
(iteration 10 includes Windows clippy + release tests).

## Proof of absence

Strings-scan of the public exe for distinctive list-only
strings from the previously bundled lists — gentside,
ohmymag, adBreakHeartbeatParams, the EasyList commit hash,
uAssets-only scriptlet names — all ABSENT. (The string
"easylist.txt" IS present: it is Pico's own cache-file-name
constant in the source table, next to its canonical URL —
disclosed in SOURCES.md.) The repo tree greps clean for
list file names and content. Gate v2: PASS on the build
exe (0 RT_MANIFEST, RT_ICON ×8 + RT_GROUP_ICON ×1, no CRT
imports, WINDOWS_GUI; winhttp.dll now appears in imports —
the OS downloader). Exe: 1,329,152 bytes (private: 1,924,608
— the difference is the removed lists).

# Pico 0.31 — Build Notes (2026-11-02)

## What 0.31 is

Two user orders in one release. (1) SEVEN NEW THEMES: the
user sent seven @uiuxaadi palette cards; all seven join
v27's nine in the picker — sixteen palettes, each deriving
dark + light through the same one-derivation machinery.
The card images were read by eye and the transcription
matched them on every slot (zero corrections). (2) A DEEP
SCAN of the whole tree by six read-only auditors with the
standing hostile brief, complete reports collected before
any ruling (the 0.30 corrective-report discipline), every
finding verified against the shipped code before fixing:
1 HIGH, 11 MED, ~40 LOW fixed; accepted/suspect items are
ledgered in the AUDIT addendum at the head of AUDIT.md.
The release is named exactly "0.31": Cargo package
0.31.0, `core::DISPLAY_VERSION` is "0.31" (the pin ties
the display string to the Cargo version or its dotted
prefix — feature releases display major.minor, repair
releases the full version). Chrome badge, start hero and
footer show 0.31. Existing users keep their saved
palette; Forest stays the fresh-install default and the
exe icon is unchanged (Forest).

## The seven palettes, as shipped

Slots are the table's [slot1 Primary, slot2 derived
raised rung, slot3 Accent, slot4 Secondary, slot5
Surface, slot6 Slate-class]. Slot mapping (documented in
`src/core/themes.rs`): slot2 = `mix(Primary→Accent,
0.35)`; slot6 = the card's own Muted for Mist Slate,
`mix(Secondary→Surface, 0.50)` for the other six.

| id | name | slots |
|---|---|---|
| teal | Deep Teal | #0B3D3B, #0E6860, #14B8A6, #99F6E4, #E6FFFB, #C0FBF0 |
| mist | Mist Slate | #191D23, #2F3A41, #57707A, #7B919C, #989DAA, #C5BAC4 |
| lavender | Lavender | #0D0B2D, #282177, #5B4BFF, #A78BFA, #E6E0FF, #C7B6FD |
| royal | Royal Blue | #0B1F3B, #143779, #2563EB, #60A5FA, #DBEAFE, #9EC8FC |
| tangerine | Tangerine | #1F1D2B, #6D3E1C, #FF7A00, #FFA94D, #FFE4C7, #FFC78A |
| amethyst | Amethyst | #5B21B6, #6C36CC, #8B5CF6, #C7B3FF, #EDE9FE, #DACEFF |
| rose | Rose | #6D0D3F, #99225F, #EC4899, #F9A8D4, #FCE7F3, #FBC8E4 |

All 32 derivations hold AA ≥ 4.5 in-hue for the text
classes (independent WCAG recomputation; minima recorded
in AUDIT). Shipped CSS == re-run derivation,
token-for-token (test-pinned). Glow census unchanged.

## Fix highlights (full ledger in AUDIT.md)

- A6 HIGH: Started-cancels-intent + started-landing
  history reconciliation (`record_commit_started`).
- A2: CommitFinish no longer wipes an in-load chosen
  designation; gate route intent covers keyboard and
  popstate; gate mirrors same-video survival.
- A1: picker retires on navigation like Teleport;
  tab-strip scroll survives snapshots.
- A5: tolerant stats deserialization (a malformed stats
  subtree can no longer wipe state); ReloadHub one-lock;
  temp recovery hardened.
- A4: parsed `is_served_url` (root-token leak via alias
  spellings closed); kind-aware LAN chooser.
- A6: credentialed same-tab navigation needs the
  one-shot typed-URL expectation.
- JS/Rust trailing-dot + loopback parity across all
  four JS runtimes; WHATWG IPv4 forms in `is_local_host`.

## Verification

- Tests: **393 Rust + 269 DOM**, all passing (0.30.1:
  362 + 238). Clippy: 0 warnings, host `--all-targets`
  and Windows target. `cargo fmt --check` clean. Zero
  unwrap/expect/panic in shipped source (scan).
- Battery: `scripts/battery.sh 10` — **10 consecutive
  PASS** (log: battery-0.31.log; iteration 10 includes
  Windows clippy + release check).
- Renders read by eye (visual-check/shot-v32-*): the
  16-card picker (Forest dark) at 1280, 2048 and 760;
  Deep Teal dark chrome; Deep Teal light and Rose light
  start pages; savings band under Royal Blue light.
  A3-F1 (soft Teal-light edges) resolved by render as
  designed behavior; A3-F2's picker-dot inset ring is
  visible working in the picker shots.
- Shield filter lists byte-identical to 0.30.1
  (`diff -rq` over assets/filters).
- Strings audit on the exe: "0.31" present (banner
  "Pico 0.31"); "0.30.1" occurs only in historical
  comments, never as the current version; each social
  URL (t.me/iK4w4, youtube.com/@iK4w4) exactly once.
- Build note: the release link prints the known benign
  LNK4099 linker messages (static-CRT PDB references in
  the cross environment) — present in prior releases,
  not a code warning.

## Hashes

- Pico.exe: 1,924,608 bytes, md5
  `50009df1f98fb64c01e5f9a1314edc00`
- dist/pico-0.31.zip (Pico.exe + README.md): 1,235,487
  bytes, md5 `afb6c225599bf2721f55a1bc86993a09`
- Gate v2: PASS on the build exe and on the exe
  extracted from dist/pico-0.31.zip (extracted md5 ==
  build md5): 0 RT_MANIFEST entries, RT_ICON x8 +
  RT_GROUP_ICON x1, no CRT imports, WINDOWS_GUI
  subsystem.
- dist/pico-0.31-source.zip: hash recorded in the
  delivery report (a file cannot hash itself).

## On-device witnesses owed

Unchanged from 0.30.1 (chosen video under Lite, Back×2,
stats growth, login survival) plus: the 16-theme picker
on the user's ultrawide, and the seven new themes'
look in the real shell.

---

# Pico 0.30.1 — Build Notes (2026-11-01)

## What 0.30.1 is

The repair round for 0.30, from the 0.30 review's
corrective report (the full auditor texts surfaced three
false verdicts in the 0.30 ledger and a fixed list of
defects in seams the battery does not cover). No new
features, no re-audit, no scope growth. The release is
named exactly "0.30.1": Cargo package 0.30.1, and
`core::DISPLAY_VERSION` is "0.30.1" — the display pin
now ties the display form to the FULL Cargo version
(0.30's major.minor display rule is superseded; a repair
release is named by its full version). Every user-visible
string — chrome badge (static fallback included), start
hero and footer ("Pico 0.30.1 · by @iK4w4"), log banner —
shows 0.30.1. Full errata: the AUDIT addendum at the head
of AUDIT.md, which also corrects the three false 0.30
verdicts in place (marked, history preserved) — including
two lines in the 0.30 notes below (its ledger's "A2-2b
(no double-measure path)" and "A6-1/A6-2 (correct browser
semantics)" summaries, and its "measured exactly"
phrasings, are superseded by the errata).

## The fix list (each verified against the shipped 0.30 code first)

1. **[HIGH] `record_traversal` step-replace** —
   `src/core/history.rs`: with a pending intent it now
   consults `landing_index` before stepping (the A6-M2
   treatment `record_commit` got in v29); step-replace
   remains only for landings no entry holds (S13
   redirect). Tests: `v30_1_traversal_signal_reconciles_multi_step_landing`,
   `v30_1_traversal_redirect_still_step_replaces`,
   `v30_1_source_first_multi_step_traversal_keeps_history`
   (tests/history_state.rs).
2. **Intent expiry (A6-MED residue)** — intents are
   stamped (`go_back`/`go_forward` take the caller's
   monotonic ms); `TabHistory::pending_expired`
   (INTENT_TTL_MS = 5 s) lets `App::go` retire a
   no-signal intent at the re-click, logged, instead of
   wedging Back/Forward behind the no-stacking guard.
   Test: `v30_1_pending_intent_expires_only_after_the_ttl`.
3. **A2-C3** — `core::lite::is_watch_shaped_url` strips
   query/fragment before locating the path; the C3 cases
   (`https://site.example?next=/watch`, fragment and port
   variants) join `watch_shape_reads_exact_path_segments_only`.
4. **A2-C4** — `core::lite::chosen_survives_url_change`
   + `watch_identity` (new pure fns); `App::apply_url_event`
   retires the host chosen flag on commits unconditionally
   and on same-document changes per the survival matrix.
   Tests: `watch_identity_separates_videos_not_presentations`,
   `chosen_survival_matrix`.
5. **A2-C2** — Shield XHR runtime (`src/core/shield.rs`):
   `open()` runs `resetXhrInstance` and bumps a generation
   counter; the fallback hooks check the generation. DOM
   test (2d) in scripts/dom_test.mjs, proven to fail with
   the teardown disabled.
6. **Labels (A2-C5/C8/C9 + A3-F4)** — semantics kept,
   words fixed: tile sub-line ("≈ X of removed values,
   counted per removal"), tile tooltip, `core::stats`
   header, README. The AUDIT size figure corrected:
   stats book is 1,775 B of the 1,962-byte default
   state.json (measured on the save path), not ~420 B.
7. **Cheap LOWs** — (a) field-level `#[serde(default)]`
   on all stats structs (+ `partial_stats_objects_load_with_zeroed_siblings`);
   (b) A4-2 Teleport retirement in the failed-tab
   address-bar retarget (`App::navigate_active`);
   (c) folded `https://pico.start/` closed in
   `core::addr::parse_input` (+ `folded_start_url_typed_by_hand_is_the_start_page`);
   (d) `landing_index` doc: "furthest" → nearest (code
   was right); (e) A2-C10: `StatsBook::record_block`
   takes the method — HEAD blocks count, credit 0/0
   (+ `head_blocks_count_but_earn_no_credit`); both
   app.rs call sites pass the request's real method.

## Build / verify

- `cargo test --locked --no-default-features` — 362
  tests green (0.30: 353), incl. every seam test above.
- DOM suite — 238 checks green (0.30: 236), incl. the
  XHR reuse pair (2d) and the relabeled band fixtures.
- `scripts/battery.sh 10` — 10 consecutive PASS under
  the user's reduced requirement (BATTERY_100.md
  addendum; log battery-0.30.1.log).
- Windows clippy (all targets) and host clippy: 0
  warnings. `cargo fmt --check` clean.
- Release: `cargo xwin build --release --locked
  --target x86_64-pc-windows-msvc`; gate v2
  (scripts/check_no_manifest.py --expect-icon) PASS on
  the build exe and on the exe extracted from
  dist/pico-0.30.1.zip (extracted md5 == build md5).
- Strings audit on Pico.exe: "0.30.1" present (9 hits),
  "29.0.0" absent, "Pico v29" absent, no bare current
  "0.30" version string, https://t.me/iK4w4 ×1,
  https://www.youtube.com/@iK4w4 ×1.
- Shield filter lists byte-identical to v29/v30
  (diff -rq clean, FILTERS.md included).
- Renders read by eye: start page, Forest dark 1280,
  populated fixture — hero reads "Pico 0.30.1", the
  Shield tile's sub-line reads "+ 34 ad payloads
  stripped in-page · ≈ 512 KB of removed values,
  counted per removal", footer "Pico 0.30.1 · by
  @iK4w4" (visual-check/shot-v31-forest-dark-1280.png).

## Sizes / hashes

- Pico.exe: 1,907,200 bytes, md5
  f9d48a41f2a7c506cf246be15333852c
- dist/pico-0.30.1.zip: 1,226,244 bytes, md5
  d703fb5ab76462cfa8f398ef1d26a8eb
- dist/pico-0.30.1-source.zip: hash recorded in the
  delivery report (a file cannot hash itself).

## Honesty notes

- The stripped-payload figure counts removed VALUE
  bytes per removal operation — an understatement of
  wire bytes, never an overstatement; the wording now
  says so everywhere it appears. Blocked counts stay
  exact; bytes/time stay calibrated ≈ estimates.
- The intent-expiry fix is core-tested (the TTL
  decision is pure); its app wiring is compile-verified
  on the Windows target. The A4-2 retirement mirrors
  two existing, proven code paths (apply_url_event's
  adoption retirement, convert_start_tab's A6-11).
- On-device witness for 0.30.1: browse with Shield and
  Lite on → open the start tab → the tiles grow; hover
  a video thumbnail → still no preview; open the video
  → it plays; multi-step Back/Forward in an SPA (e.g.
  YouTube Back ×2) no longer duplicates history.

---

# Pico 0.30 — Build Notes (2026-11-01)

## What 0.30 is

Three orders. (1) The release is named exactly "0.30":
Cargo package 0.30.0, and every user-visible string —
the chrome badge, the start page hero and footer
("Pico 0.30 · by @iK4w4"), the log banner — shows 0.30.
`core::DISPLAY_VERSION` is the single source, pinned
against the Cargo major.minor by a unit test, and a
source-scan pin guarantees the snapshot (which the badge
renders verbatim) is built from it.

(2) The headline: a SAVINGS BAND on the start page (and
nowhere else — chrome chips unchanged). Two tiles,
Shield and Lite, each showing requests blocked (exact
integers, counted once at the same point the chips use),
data saved (a ≈ calibrated estimate), and — per tile —
the session's smaller line; a combined time-saved line
underneath (also ≈); the Shield tile additionally shows
the exact in-page stripped payload, kept conceptually
separate. A "Reset stats" control rides the gated
dispatch (`stats-reset`). Semantics in one paragraph:
blocked requests are unknowable in size, so a persisted
response ledger (per-type totals of Content-Lengths and
dispatch→response durations from ALLOWED traffic)
credits each block its type's measured average once 20
samples exist, else documented built-in defaults;
documents/subdocuments/websockets/pings are excluded
from credit entirely. Lifetime + ledger persist in
state.json (serde-default migration + sanitization);
a pre-0.30 file's legacy `blocked_total` seeds the
Shield count once. Full contract, defaults tables and
the honesty bounds: AUDIT 0.30 addendum + core::stats
module doc. Implementation: `src/core/stats.rs`
(~1,200 lines with tests), a prune-measurement
extension in the scriptlet runtime (exact UTF-8 byte
count of removed subtrees, reported over the validated
`shield_pruned` IPC), the response-observer wiring in
`src/app.rs`, and the band in `assets/start.html`
(neumorphic tiles; glow census now 12, honestly
recounted).

(3) Assurance: the six-area hostile fan-out over the
whole tree (the stats code split across A2/A5),
verify-before-fix centrally — 3 MED fixes (the
stats-reset fold, the prune-flush event cap, the
correlation TTL), a LOW cluster fixed, and a long
disproven list (including two claimed MEDs that did
not survive contact with the code); plus the full
battery run 100 consecutive times (BATTERY_100.md).
One maintainer find along the way: the v24 recents
actions' anchor transport had been silently
dead-ending on an app-local classifier since v24; it
now delegates to the core classifier like everything
else. Shield filter LISTS byte-identical to v29.

## Verify-before-fix ledger (summary; full table in AUDIT)

Confirmed + fixed: A1-F1/A4-1 stats-reset fold (MED);
A2-1 prune flush events cap (MED); A2b-1 correlation
TTL at take (MED); A2b LOW cluster (humanizer tie
rounding, signature omissions, reset log budget,
prune-gate forget, duplicate-note eviction,
zero-duration samples, malformed Content-Type
buckets, seed hardening); A1-C1/C2/C3; A1-2 report
pre-filters tightened to the kind assignment (a nav
note with a marker word in its title could be
swallowed); A1-3 0/0 prune reports refused before
they consume gates + a session event ceiling; A5-L2
proportional ledger scale-down. Disproven: A5-H1
(seed "missing" — it is at startup and persists),
A5-M1 (latch "race" — single atomic swap), A5-L1,
A2-5 (excluded types credit 0/0), A2-2b (no
double-measure path), A2-S1 (byteSizeOf already
guarded), A1-C4 (identical liveness checks), A3-3.3
(no icon.png exists), A3-3.4 (document parses
balanced), A4-2/A4-3 in part, A6-1/A6-2 (sequences
reproduce correct browser semantics; the landing
fallback is the documented v28/v29 design).

## Build / verify

- `cargo test --locked --no-default-features` — 353
  tests green (lib unit + integration + doc), incl.
  the new stats suites (reconciliation corpus, prune
  IPC shapes, seed, sanitize, band placement).
- DOM suite (jsdom over `examples/dump_pages`
  output) — 236 checks green, incl. band rendering
  from fixture stats in both theme modes, the zero
  state, the shared humanizer fixtures (ties
  included), and the prune-measurement proof (a
  14-byte subtree with two € signs measured exactly).
- `scripts/battery.sh 100` — 100 consecutive full
  passes; Windows-target clippy + release check every
  10th iteration. Log summary: BATTERY_100.md.
- Windows clippy (all targets) and host clippy: 0
  warnings. `cargo fmt --check` clean.
- Release: `cargo xwin build --release --locked
  --target x86_64-pc-windows-msvc`; gate v2
  (`scripts/check_no_manifest.py --expect-icon`)
  PASS on the build exe and on the exe extracted
  from the release zip.
- Strings audit on Pico.exe: "0.30" present (13
  hits), "29.0.0" absent, "Pico v29" absent,
  https://t.me/iK4w4 ×1, https://www.youtube.com/@iK4w4 ×1.
- Renders read by eye: start page with band —
  Forest dark 1280 + 2048, Ember light, Violet dark;
  populated + zero-state fixtures.

## Sizes / hashes

- Pico.exe: 1,903,616 bytes, md5
  6635d4c767244ced52d662cbd091792e
- dist/pico-0.30.zip: 1,225,126 bytes, md5
  4b6add133bf6d81cf9a68a9b31f1e27c
- dist/pico-0.30-source.zip: hash recorded in the
  delivery report (a file cannot hash itself).

## Honesty notes

- Blocked counts are exact; bytes and time are
  calibrated estimates and wear their ≈ everywhere,
  with the method in the tile tooltips, the README
  and the AUDIT. The stripped-payload figure is exact
  under the documented definition (the subtree as
  removed, re-serialized at removal).
- Duration correlation depends on WebView2 handing
  the same request object to both resource events;
  on a miss, bytes still calibrate and duration
  simply doesn't — stated, never faked. The pointer
  ABA bound is documented in the AUDIT.
- Playback/chosen-video proof remains jsdom-level,
  as in v29. On-device witness for 0.30: browse with
  Shield + Lite on → open the start tab → tiles grow;
  hover previews still dead; the chosen video still
  plays; Reset stats returns the band to zero.
