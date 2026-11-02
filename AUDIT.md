# Pico 0.31 — Provenance Addendum, Public Release (2026-11-02)

This tree is the public edition of 0.31: identical features,
laws, and UI; the filter lists are no longer bundled. This
addendum records the provenance audit behind that change.

## Where list content lived, and where it went

- **The lists themselves** (`assets/filters/*.txt(.gz)`,
  embedded via flate2 + `include_bytes!`): REMOVED. The
  Shield now resolves its lists at runtime: cache in
  `data/lists/` first, else an HTTPS download of the four
  canonical URLs (SOURCES.md), validated before compile and
  cached atomically; any failure leaves the Shield idle
  (fail-open) with one honest log line and a retry on the
  next launch plus one after 60 s. Orchestration is pure and
  tested in `src/core/lists.rs`; the transport is WinHTTP
  through the already-present `windows-sys` surface (no new
  dependencies; flate2 was removed).
- **Scriptlet/cosmetic rule content baked into code**: NONE
  FOUND. The payload runtimes in `shield.rs` are original,
  hand-written interpreters (a 13-name dispatch); the compiled
  rule data they consume always came from list text and now
  comes from the downloaded lists identically. YouTube's JSON
  field names appearing in hand-written prune behavior
  (adPlacements, adSlots, …) are YouTube's API facts, not
  list content, and no uAssets rule text is reproduced in
  code, comments (one comment naming two list domains was
  rewritten synthetically), or tests.
- **Tests**: every fixture lifted from the bundled lists was
  replaced by a hand-written SYNTHETIC corpus in
  `tests/fixtures/` (invented hosts: *.example, *.invalid,
  plus the video-site section the DOM suite drives, written
  for this suite). The semantics matrix is unchanged —
  anchors, exceptions, $options, $important/$badfilter,
  cosmetics, the scriptlet tokenizer hazards — with
  corpus-relative counts instead of list totals.
- **Repo hygiene**: LICENSE is MIT (the author's own code);
  FILTERS.md became SOURCES.md (URLs + runtime delivery +
  courtesy credit; the lists' licenses stay with the lists,
  which Pico no longer distributes). The tree was grepped
  for machine/personal traces (home paths, usernames,
  internal process references, the author's other projects'
  URLs) and scrubbed; battery logs and visual-check output
  are excluded/.gitignored.

## Proof of absence

The public exe is strings-scanned for distinctive list-only
rule/domain strings from the previously bundled lists; all
must be absent, and the repo tree is grepped for list file
names and content. Numbers: see the public-release section
of BUILD_NOTES.md.

# Pico 0.31 — Sixteen Themes + Deep-Scan Addendum (2026-11-02)

0.31 does two things, both on the user's order: (1) seven new
@uiuxaadi palettes join the nine from v27 — sixteen themes in
the picker; (2) a full six-auditor deep scan of the whole tree
("fix all bugs and holes and glitches... do it in all ways
possible with deep scans"), with every confirmed finding fixed.
Process discipline was the 0.30.1 corrective-report rule,
applied harder: COMPLETE auditor texts were collected before
any ruling, every finding was re-verified against the shipped
code before it was fixed, and every fix was then re-verified
in the final tree by the maintainer directly (tests, diffs
and renders — not by fix-worker report; two fix workers'
interim reports described work that had not yet landed, and
only the tree was believed). Totals for the round: 1 HIGH,
11 MED, ~40 LOW fixed; the rest of this addendum is the
ledger, including what was NOT fixed and why.

## Order 1 — the seven palettes

The seven card images were read by eye; the parent
transcription matched the images on EVERY slot — zero
corrections. Cards are light-anchored and name five roles
(Mist Slate carries a sixth, Muted); the mapping into the
6-slot table is documented at the top of
`src/core/themes.rs`: slot1 = card Primary, slot2 = derived
`mix(Primary→Accent, 0.35)` (the raised rung the cards do
not carry), slot3 = card Accent, slot4 = card Secondary,
slot5 = card Surface (the card Background folds into it by
the one-material law), slot6 = Slate class for all seven
(Mist Slate uses its own card Muted; the other six use
derived `mix(Secondary→Surface, 0.50)`), both recipes
re-run from the table's own colors by a pin test. All 32
derivations hold AA ≥ 4.5 in-hue for text/muted/faint/
accent/danger (independent WCAG recomputation; minima:
muted/accent 4.5183 Rose light, danger 4.5024 Tangerine
dark, text 6.23 Mist dark) and the shipped CSS equals a
re-run derivation token-for-token. A3-F5 hardened
`--accent-text` through `ensure_contrast` on its real
pairing (text ON the accent): a proven no-op for all 32
shipped values, pinned token-for-token
(`accent_text_pins_all_32`); an earlier experiment that
judged it against the surface pairing changed 18 values
and was rejected on that evidence. A3-F3 pins the rendered
text-on-raised pairing (min 5.22, Mist dark) and documents
that arbitrary token-on-`--raised` pairings are NOT all AA
(worst known: Tangerine dark danger-on-raised 3.41) —
future UI must check before pairing. Existing users keep
their saved palette; Forest stays the fresh-install
default and the icon. The picker renders read by eye:
16 cards, Forest dark, at 1280 / 2048 / 760 — centered
panel (the v25 far-corner bug stays dead), grid scrolls,
all cards reachable; Deep Teal dark chrome, Deep Teal
light + Rose light start pages, and the savings band
under Royal Blue light all render clean. A3-F1 (Teal
light's neumorphic edges are the softest of the sixteen,
1.11:1 shadow tint) resolved BY RENDER: edges are subtle
but perceptible, all text holds AA — the one-material
law working as designed; accepted, no token change (any
change would have moved the glow census and every other
palette's derivation). A3-F2's picker dots gained a
neutral inset ring in the chrome CSS; six distinct dots
per card are now visible in every derivation, dark and
light (visible in the renders above).

## Order 2 — the deep scan

Six read-only auditors (chrome/IPC, filter pipeline,
themes/start, teleport/QR, server/state/data, nav/URL)
over the whole tree, hostile brief, the 0.30/0.30.1 seams
named in advance. Headline results:

- **A6 HIGH — the Started-cancels-intent seam (the 0.30
  class, third door).** `on_page_load(Started)` cancelled
  the pending traversal intent whenever the start URL
  differed from the intent's target, so a coalesced
  multi-step landing (Back intent, webview lands two
  entries back, then redirects) was forced into a plain
  push: [a,b,c,a′], phantom forward branch. Fixed on both
  halves: the shell no longer cancels when the start URL
  is itself a history entry, and `TabHistory` now records
  the started URL (`note_commit_start`) so the finish
  reconciles at the started entry, the finish URL
  replacing it ([a′,b,c]) — `record_commit_started`, with
  composed-path tests. The SPA/Source path never had the
  pre-cancellation; document and SPA traversals now agree.
- **A2 MED ×3 — the chosen-video model, three more
  seams.** (F1) `CommitFinish` wiped a designation made
  DURING that document's load (tap an embedded player on
  a slow link, designation posts, the queued finish then
  cleared the host flag and its streams starved — the
  v28 shape through an event-ordering door): a finish
  with a preceding start no longer clears; an orphan
  finish still does. (F2) Gate route intent was
  pointer-only — keyboard Enter, popstate (Back/Forward)
  and programmatic routes to a watch page designated
  nothing: gestures now include keydown, and a
  popstate-driven route counts as intent. (F3) The gate
  dropped its designation on EVERY route change while
  the host kept the flag for same-video churn, so a
  player retry after a presentation pushState paused
  the chosen video: the gate now mirrors
  `chosen_survives_url_change`/`watch_identity`.
- **A1 MED ×2.** The theme picker survived navigation
  (Teleport retired, the picker — and the grown chrome
  rect — did not): the adopted-URL path and start-tab
  conversion now retire both panels. Tab-strip scroll
  restoration was immediately defeated by an
  unconditional `scrollIntoView` on the active tab:
  it now fires only when the active tab changes.
- **A5 MED — state-load resilience, the other half.**
  0.30.1's serde fix covered MISSING stats keys only;
  `"stats": null`, wrong-typed scalars (-1, 1.5, "5")
  and ledger arrays of length ≠ 11 still failed the
  WHOLE PersistState parse and the next save wiped
  theme/recents/savings. The stats subtree now
  deserializes tolerantly (Value-based: null/wrong-type
  → defaults, numbers coerced, ledger truncated/padded
  to 11); serialization format unchanged. Tests cover
  every malformed shape at all three levels.
- **A4 MED ×2.** Pico-served detection was exact-string
  only: the same server reached as `localhost`, with
  userinfo, or with a trailing dot was classified NOT
  served and Teleport LAN-rewrote it — the QR carried
  the secret root token (the phone still could not
  fetch: the server binds loopback only; the leak was
  the capability URL itself). `core::server::
  is_served_url` now parses (scheme/port/host/path),
  alias matrix pinned. The LAN chooser could not see
  adapter kind: a VPN tunnel egress beat the reachable
  Wi-Fi address. Candidates now carry kind (IfType) +
  DAD state; duplicates are excluded at enumeration, a
  tunnel egress no longer beats a physical candidate, a
  physical egress still wins outright.
- **A6 MED — credentialed same-tab navigation.** The
  userinfo denial covered popups only; a page could
  navigate its own tab to
  `https://google.com:443@evil.example/…`. Now a
  one-shot shell expectation: `navigate_active` records
  the exact typed URL; the navigation handler consumes
  the expectation on the tab's first navigation event
  and allows a userinfo URL only on an exact match
  (typed credentialed URLs remain the documented
  user-act exception). Decision table is core-pure
  (`userinfo_nav_allowed`) and pinned.
- **JS/Rust classifier parity (A2-F4/F5).** Trailing-dot
  FQDNs defeated the JS cosmetic/scriptlet host walks
  (the innertube prune layer never applied on
  `www.youtube.com.`) while Rust matched fine; JS
  loopback spellings (`foo.localhost.`) likewise
  diverged. All four JS runtimes now normalize the
  hostname once at acquisition, like `host_of` does.
  Rust-side, `is_local_host` additionally accepts
  WHATWG IPv4 number forms for 127/8 (A2-F15's Rust
  half). Parity claims are scoped to canonical URLs for
  the remaining exotic spellings (percent-encoded
  hosts, backslash authorities) — whether WebView2
  canonicalizes them before Pico sees them is unproven
  either way, and no claim is made beyond that.
- **~40 LOWs fixed**, each with a seam test, including:
  XHR gate partial-install teardown (the 0.30.1 stale-
  response bug's side door) + fallback listener removal
  + fake-response generation check; flushCounts dropping
  prune paths past the 16-item cap; anchored slash-form
  rules missing on explicit-port URLs; stream-shape
  gaps (`;param`, percent-encoded names); markTree
  blessing previews inside multi-media containers;
  `note_response` not validating its sample + HEAD
  responses teaching the ledger; picker Escape layering,
  focusAddr over a dirty draft, focus loss on closing a
  focused inactive tab, insertBefore churn; chrome
  `Command` parsing now rejects unknown fields like the
  report parsers do; cross-tab address drafts (a dirty
  draft no longer follows a keyboard tab switch);
  `record_pop`'s same-URL early return not consuming a
  pending intent; `parse_input` start-URL spellings
  (`?x`, `#x`, `//`) and bracketed-host junk
  (`[::1]8080` now refused, not searched); the
  invisible-format set extended to Unicode Tags
  (U+E0001, U+E0020–E007F) + U+2065 in both the log
  redactor and the address layer; ReloadHub's tombstone
  race closed structurally (one lock); `recover_from_
  temp` length-checks before reading and runs on
  non-NotFound read errors too; the seed log line no
  longer claims "carried over" when the book's gate
  refused; log rotation zeroes its counter only on a
  confirmed removal; `copy_tree` depth cap (32);
  `humanize_bytes` gained the EB unit (Rust + JS
  mirror); theme pushes re-check the closed sets at the
  sink; the NoNetwork Teleport branch strips userinfo;
  the https LAN-rewrite note names the TLS dead end;
  GetAdaptersAddresses retries on buffer growth.

## Accepted / suspect — not fixed, with reasons

- **A2-F8 — report forgery (accepted, restated).** Any
  page can post max-size `shield_pruned` reports; caps
  bound the RATE, and lifetime figures persist until a
  manual reset, so a hostile page can inflate the one
  figure the band presents as measured. No authenticity
  primitive exists (the honest runtime holds no secret
  a page script does not). Accepted in 0.30; the
  persistence dimension is now on record.
- **A2-F11 — chosen-context breadth (accepted).** The
  watch/chosen network exemption is page/tab-wide;
  pre-buffering previews on a watch page fetch at full
  rate under Lite even though the gate refuses their
  playback. Playback-dead ≠ byte-dead; the v29
  trade-off, restated so it is not misread as covered.
- **A6-F3 — pushState-to-visited classification
  (accepted ambiguity, claims corrected).** The
  reducer's signals cannot distinguish a pushState to
  a previously-visited URL from a traversal landing;
  Pico's index can diverge from the WebView's real
  stack until the next document commit reconverges it.
  0.30-era comments claimed it "self-heals on its next
  landing" — false as stated; the comments and this
  ledger now say what actually happens, and both
  traces are pinned as tests. The platform
  CanGoBack/CanGoForward OR-ing mitigation was
  considered and not adopted: the values are not
  readable at snapshot time without new platform
  plumbing, and landings already reconverge the model.
- **Suspects left documented, not chased:** A1-S2
  (tpAwaiting wedge — no reachable no-active-tab state
  found), A1-S3 (queued-snapshot panel flicker —
  self-healing by construction), A2-F14 (request
  handler judging subresources against the committed
  URL during a navigation — depends on unproven
  WebView2 event interleaving), A6-F4 (SourceChanged
  coalescing — no reducer-level fix exists; the
  `pico.log` witness that would settle it is a dropped
  intermediate URL on a fast SPA), A6-F10 (a load
  producing no PageLoad/Source event leaving Tab.url
  stale — WebView2 raises SourceChanged in practice).
- **A5 residuals:** the canonicalize→open TOCTOU is
  inside the local trust boundary (accepted; the 416
  arm now stats the opened handle), concurrent
  instances remain whole-file last-writer-wins
  (single-instance stays a non-goal), migration resume
  never deletes destination files removed from the
  source (a deliberate safety choice), a UNC APPDATA
  fallback now logs a plain network-path note (A5-L11)
  rather than changing behavior.
- **A4-F7** — Teleport Esc dismissal lives in the
  chrome document only; with focus in the content
  HWND, Esc never reaches it (×/backdrop/re-click/
  tab-switch/navigation all dismiss; accepted).
- **A3-F4** — brand hover icons (#229ED9/#FF0033, the
  documented purity exception) fall below 3:1 on some
  new light surfaces; hover-only affordances with
  passing text labels; accepted.

## Proof

- Tests: **393 Rust + 269 DOM** (0.30.1: 362 + 238),
  all passing; clippy 0 warnings on host and Windows
  targets; `cargo fmt --check` clean; zero
  unwrap/expect/panic in shipped source (scan).
- Battery: `scripts/battery.sh 10` — 10 consecutive iterations PASS (iteration 10 additionally ran the Windows-target clippy + release check)
  (log: battery-0.31.log).
- Gate v2 on the build exe and the release-zip exe:
  PASS on both (0 RT_MANIFEST, RT_ICON x8 + RT_GROUP_ICON, static CRT, GUI subsystem). Shield filter lists byte-identical to
  0.30.1 (`diff -rq`). Strings audit: "0.31" present,
  "0.30.1" not the current version, each social URL
  exactly once.
- Renders read by eye (visual-check/shot-v32-*): the
  16-card picker at 1280/2048/760, Deep Teal dark
  chrome, Deep Teal light and Rose light start pages,
  the savings band under Royal Blue light.
- What this does NOT prove: anything that needs the
  user's Windows machine. The on-device witnesses owed
  are unchanged from 0.30.1 (chosen video plays +
  previews dead under Lite, Back×2, stats tiles grow,
  login survival) plus the new ones: the picker with
  16 themes on his ultrawide, and the look of the
  seven new themes themselves.

---

# Pico 0.30.1 — Audit Errata & Repair Addendum (2026-11-01)

0.30.1 is a repair release with a fixed list and no new
features. Its raw material is the 0.30 review's own
CORRECTIVE REPORT: when the six auditors' complete final
texts arrived (the maintainer had worked from truncated
deliveries), several findings had never reached the fix
list, and three verdicts in the 0.30 addendum below were
wrong. The 0.30 packages, hashes, gate, strings and
battery results were accurate — what was wrong was the
audit ledger, and the shipped tree contained real defects
in seams the battery does not cover (each auditor noted
the same about their own finds). This addendum is the
correction: the history below is preserved verbatim, the
three false verdicts carry [CORRECTED 0.30.1] markers
pointing here, and every repair is verified against the
shipped 0.30 code before it is touched (verify-before-fix
cuts both ways).

## The repairs

- **A6 HIGH — `record_traversal` step-replaced a
  multi-step landing.** The v29 A6-M2 fix gave
  `record_commit` the landing search; `record_traversal`
  — the third consumer of a pending intent, and the ONLY
  signal same-document SPA traversals produce — never got
  it. [a,b,c] at c with a Back intent, landing straight on
  a (a coalesced 2-step traversal), became [a,a,c]:
  consecutive duplicates and a phantom Back. Fixed: with
  an intent in flight, `record_traversal` consults
  `landing_index` first, exactly like `record_commit`;
  step-replace remains only for a landing no entry holds
  (the genuine redirect, S13). The 0.30 addendum's
  "A6-1/A6-2 disproven" verdict reconstructed sequences
  against `record_commit`/`record_pop` only and never
  drove this seam — it is corrected below. Tests:
  `v30_1_traversal_signal_reconciles_multi_step_landing`,
  `v30_1_traversal_redirect_still_step_replaces`,
  `v30_1_source_first_multi_step_traversal_keeps_history`.
- **A6 MED (residue) — a no-signal intent wedged
  Back/Forward.** An intent whose landing never arrives
  (the webview's stack diverged far enough that
  `history.back()` is a silent no-op) sat in the single
  pending slot forever, and the shell's no-stacking guard
  refused every later traversal until an unrelated
  navigation cleared it. Fixed with a clean expiry, not a
  half-measure: intents are stamped on the shell's
  monotonic clock when issued (`INTENT_TTL_MS` = 5 s —
  orders of magnitude past any genuine landing, which
  arrives in the same event-loop breath), and `App::go`
  retires an expired intent (one log line) and issues the
  click's own. A late landing after expiry reconciles by
  URL via the landing search, like any uninitiated
  traversal. The companion "false-Pop" classification
  ambiguity (a pushState to a previously-visited URL is
  recorded as a move) remains the documented v29
  trade-off: it does not corrupt, because every landing
  reconciles by URL; it is not relitigated here. Test:
  `v30_1_pending_intent_expires_only_after_the_ttl`.
- **A2-C3 (MED) — `is_watch_shaped_url` read a slash in
  the query as the path.** A host-only URL like
  `https://site.example?next=/watch` was watch-shaped in
  Rust but not in the document gate (which reads
  `location.pathname`): a Lite network exemption with no
  gate designation behind it. Fixed: query/fragment are
  stripped before the path is located (neither can occur
  inside an authority, so no path can be eaten). Test:
  the C3 cases join `watch_shape_reads_exact_path_segments_only`.
- **A2-C4 (MED) — the host's chosen flag outlived the
  gate's designation.** The gate resets its designation on
  every route change; the host flag cleared only on full
  navigation / toggle / close, so after an SPA route
  change, preview media and streams stayed network-allowed
  for the tab indefinitely (playback still refused — the
  data-saving was silently off). Fixed in
  `App::apply_url_event`, the one writer of the live URL:
  a commit retires the flag unconditionally (new
  document); a same-document change consults
  `core::lite::chosen_survives_url_change` — same-video
  watch churn survives (the designation bridge), a
  different video or any exit from watch territory
  clears, entering watch territory is unaffected, and a
  move between non-watch pages clears. Tests:
  `watch_identity_separates_videos_not_presentations`,
  `chosen_survival_matrix` (both in core::lite).
- **A2-C2 (MED) — the Shield's XHR getter gate was never
  removed on `open()`.** A page reusing one XHR object for
  a second, non-relevant request kept reading the FIRST
  response's transformed text through the stale
  per-instance getters; the fallback event hook had the
  same stale-URL reach. Fixed: `open()` now runs
  `resetXhrInstance` (deletes the gate accessors and every
  shadowed property — all Pico-installed properties are
  configurable, so the prototype's native accessors return)
  and bumps a generation counter the fallback hooks check
  before transforming. Test: DOM (2d) `0.30.1 xhr reuse: …`
  — verified to FAIL against the un-fixed runtime (the
  reused instance served the first response verbatim) and
  pass with the teardown.
- **A2-C5/C8/C9 + A3-F4 — the stripped-payload figure's
  LABEL, corrected (semantics kept, words fixed).** The
  figure counts prune OPERATIONS (a fresh parse of the
  same body is a fresh removal set and is counted again),
  measures VALUES only (keys and separators excluded: the
  fixture subtree whose removal cut 26 wire bytes is
  credited 14), and does not measure replace-family
  removals at all. 0.30's wording — "exact", "measured
  byte for byte", a tile that appended "measured exactly"
  to a rounded figure — overstated it, and the A2-2b
  "disproven" verdict below ("one download is never
  measured twice") is wrong on the re-parse axis: the
  layered hooks are idempotent over ONE object, not over
  re-parses. The parent decision: keep the semantics (they
  count real removal work, and the value-only rule makes
  the number an understatement of wire bytes, never an
  overstatement), fix the WORDS everywhere — the tile now
  reads "≈ X of removed values, counted per removal", the
  tooltip and the `core::stats` header say precisely what
  is counted, and the README matches. The corrected
  definition lives in the `core::stats` module header.
- **Cheap LOWs.** (a) The stats structs' fields had no
  serde defaults, so a `stats` object missing any sibling
  field failed the WHOLE state parse: every field of
  `TypeLedger` / `EngineStats` / `Savings` now carries
  `#[serde(default)]` (the ledger array's length stays
  structural, like the schema around it). Test:
  `partial_stats_objects_load_with_zeroed_siblings`.
  (b) A4-2: an address-bar retarget on a view-failed tab
  rewrote the tab URL directly, so no URL event ever
  retired a stale Teleport panel; the retarget now retires
  it, mirroring convert_start_tab's A6-11 fix. (c) A typed
  `https://pico.start/` bypassed the M1 go-home guard
  (it parsed as a website): `parse_input` folds wry's
  spelling to the canonical start target before the http
  branch can claim it; real `pico.*` sites are untouched.
  Test: `folded_start_url_typed_by_hand_is_the_start_page`.
  (d) `landing_index`'s doc said "furthest"; the code
  takes the nearest match in the pending direction (and
  the tests pin nearest) — the doc now agrees with the
  code. (e) A2-C10: a blocked HEAD request was credited
  the full per-type body average; `record_block` is now
  method-aware — a HEAD block counts (exactly, like every
  block) but earns zero byte/time credit, because a HEAD
  response has no body and Pico does not invent a headers
  figure. Test: `head_blocks_count_but_earn_no_credit`.
- **The size figure.** The 0.30 addendum's "~420 bytes"
  for the stats book was wrong. Measured on the real save
  path in 0.30.1: the default state.json is 1,962 bytes,
  of which the stats book is 1,775 (the same file without
  it is 187 bytes). The A5 addendum's figures were right;
  the review's summary figure was not.

## Verification

- Tests: 362 Rust (lib + integration + doc) and 238 DOM
  checks, all green (0.30: 353 + 236) — every repair above
  carries the named seam test, and the XHR test is proven
  to fail without its fix. Clippy 0 on host and Windows
  targets; `cargo fmt --check` clean. Shield filter LISTS
  byte-identical to v29/v30 (diff-verified).
- Battery: `scripts/battery.sh` — see the 0.30.1 addendum
  in BATTERY_100.md for the 10-consecutive-pass result
  under the user's reduced requirement.
- Gate v2, hashes, renders: BUILD_NOTES.md (0.30.1
  section). The version is 0.30.1 — Cargo 0.30.1 and
  DISPLAY_VERSION "0.30.1" (the pin now ties the display
  form to the full Cargo version; the repair release is
  named by its full version, superseding 0.30's
  major.minor display rule).
- On-device witness (unchanged shape from 0.30, plus one
  new line): browse with Shield and Lite on → open the
  start tab → the tiles grow; hover a video thumbnail →
  no preview; open the video → it plays; in an SPA (e.g.
  YouTube), Back twice in quick succession → the tab's
  history stays coherent, no phantom step; Reset returns
  the band to the zero state.

# Pico 0.30 — Audit Addendum (2026-11-01): THE SAVINGS BAND + SIX-AREA HOSTILE SWEEP + 100-RUN BATTERY

0.30 does three things. (1) It renames the release: the
version is exactly "0.30" — Cargo 0.30.0 underneath, and
every user-visible string (chrome badge, start hero and
footer, log banner, this document) says 0.30. (2) It adds
the SAVINGS BAND to the start page: per-engine figures for
the waste Pico prevented — requests blocked (exact),
data saved (≈, calibrated), time saved (≈, calibrated,
added mid-build at the user's direction after Brave's
new-tab stats), and the Shield's in-page stripped payload
(exact, separate) — with a gated Reset control. (3) It
re-runs the full assurance programme: the six read-only
auditor fan-out over the whole tree with the new stats
code as a dedicated target, verify-before-fix throughout,
and the full test battery 100 consecutive times (see
BATTERY_100.md). Shield filter LISTS are byte-identical
to v29 (diff-verified).

## The band's semantics (the contract the figures keep)

- **Requests blocked — exact.** One count per delivered
  block, recorded in the request handler after the
  neutered response is set (the v22-9 law), attributed by
  the `filterpipe::decide` verdict — the same single
  counting point the chrome chips have always used. A
  Rust reconciliation test drives `decide` over a fixture
  corpus (both engines, toggles, a watch page, a chosen
  tab, loopback) and asserts the book's totals equal the
  sum of the verdicts. Loopback blocks never occur and
  are never counted; the chosen video's media is ALLOWED,
  so it is never counted as saved; an engine that is off
  accrues nothing.
- **Data saved — calibrated estimate, shown with ≈.** A
  blocked request's true size is unknowable without
  fetching it. The response ledger learns from ALLOWED
  responses: for each public response, the observer reads
  Content-Length (defensive parse — digits only, no zero,
  capped) and, when the request was noted at dispatch,
  the duration to the response. Per ResourceType the
  ledger persists (total_bytes, byte_samples,
  total_duration_ms, duration_samples). A block of type T
  is credited T's measured average once T has ≥ 20
  samples, else a documented built-in default (Image
  50 KB, Media 400 KB, Script 30 KB, Stylesheet 15 KB,
  Font 40 KB, Xhr 20 KB, Other 10 KB — order-of-magnitude
  figures consistent with published web-almanac resource
  weights). Averages recompute from persisted totals;
  there is no drift term. Document, Subdocument,
  WebSocket and Ping are EXCLUDED from credit (both
  credit functions return 0 for them before any lookup):
  main-frame documents are essentially never blocked
  (only `$document` rules can), and crediting a guessed
  document size would swamp the honest figures.
- **Time saved — the same ledger, the duration half.**
  Correlation keys on the WebView2 request object's
  identity token between the request and response events.
  Correlation state is capped (2,048 entries) and
  expiring (180 s TTL enforced at take time; see A2b-1):
  a response that arrives stale still lands its BYTES in
  the request's type slot, but its duration is forgotten —
  never invented. If the token was never noted (or was
  evicted), bytes alone calibrate, bucketed by the
  response's Content-Type, and no duration is recorded.
  Honest bound (A2-4/A2b-7): the token is a raw COM
  pointer; if WebView2 recycles an address for a new
  request inside the window, one duration sample can land
  in the wrong type's slot. Averages need ≥ 20 samples,
  samples are capped at 600 s, and the figure is an
  estimate — the maintainer accepted this bound rather
  than churn Windows-only COM code mid-release. Duration
  defaults until calibration: Script 600 ms, Image
  800 ms, Stylesheet 400 ms, Xhr 500 ms, Media 2,500 ms,
  Font 700 ms, Other 400 ms — deliberately conservative
  for a ~300 kbps link. Lite's stream-backstop blocks
  credit their WIRE type (an MSE segment fetch is an Xhr
  and is credited as one — A2b-13, accepted: type-true
  crediting; it under-credits Lite slightly on
  streaming-heavy sites rather than inflating it).
- **In-page stripped payload — exact, and separate.**
  [CORRECTED 0.30.1: the figure is the removed VALUES'
  size counted per removal operation — not an exact wire
  count. See the 0.30.1 errata at the head of this file
  and the `core::stats` header for the corrected
  definition.] The
  scriptlet runtime measures the serialized size of every
  ad subtree it prunes (UTF-8 bytes, surrogate-correct)
  and reports `{kind:"shield_pruned", bytes, events}`
  over a new validated IPC (deny-unknown-fields, bytes
  ≤ 16,777,216, events ≤ 100,000; honored only from a
  live public non-start tab with the Shield on; per-tab
  1 s rate floor; 1 TiB session ceiling). "Exact"
  (A2-2a, documented definition): the subtree as the
  runtime removed it, in its re-serialized form at
  removal time — whitespace in the wire body could only
  have made the true wire cost larger. An "event" is one
  removed subtree; a terminal wildcard removing twelve
  children counts twelve. Deletes are idempotent across
  the layered hooks, so one download is never measured
  twice (A2-2b disproven below). [CORRECTED 0.30.1:
  idempotent over one object only — a re-parse is
  measured again; see the errata.] The figure is NEVER
  added into the ≈ estimate — those bytes were
  downloaded inside the page's JSON before being cut.
  Known bound (A2b-3): the report channel is
  fire-and-forget; if two flushes land inside the
  host's 1 s rate floor (e.g. a timer flush followed
  immediately by pagehide), the second report is refused
  and its window is lost — at most one flush window
  (≤ 2.5 s of prunes) per page. The floor is the
  anti-forgery defense; the loss is its price.
- **Persistence.** Lifetime figures + the ledger ride
  `PersistState.stats` (serde-defaulted: every older
  file loads a zero book; sanitized on load — engine
  figures capped at 2⁶², ledger totals capped at what
  their samples support, sample counts past a billion
  scaled down proportionally so averages survive).
  A pre-0.30 file's legacy `blocked_total` SEEDS the
  Shield count once at startup (the book must be
  completely empty; the value takes the 2⁶² cap) —
  bytes/time honestly start at zero, and from then on
  the book is the authority and `blocked_total` its
  mirror (A2-6: a hand-edit to the mirror alone is
  overwritten on the next save). Session figures live
  only in memory. State-save failures never break
  counting (the book is in-memory authoritative; the
  v29 once-per-session failure latch is unchanged).
- **Reset.** The band's "Reset stats" rides the same
  conjunctive gated dispatch as every privileged action
  (start tab AND start page; IPC + anchor transports),
  action name `stats-reset`. It zeroes lifetime, session,
  the ledger, pending correlation and the prune gates
  (including the prune log budget).
- **Rendering.** The band lives ONLY on the start page,
  updates through the existing snapshot/SharedData path
  (plus a signature-gated push when figures change; the
  signature hashes all 18 rendered figures). Counts use
  one pure Rust humanizer mirrored in JS, cross-pinned
  by shared fixtures — including exact rounding ties
  (A2b-2). The band adds one glow site (tile hover);
  the census is now 12 (10 chrome + 2 start), pinned in
  tests. A3 verified the band's text contrast across
  all 18 theme derivations (worst case 4.81:1).

## The fan-out (six areas, read-only, hostile)

Confirmed-and-fixed, in the order a user would care:

- **A1-F1 / A4-1 (MED) — the Reset anchor's fold was
  missing.** `normalize_pico_url` folds wry's
  `https://pico.<host>/` rewrites back to `pico://` only
  for a listed set of internal hosts; `stats-reset` was
  not on it (the v24 recents actions are), so the band's
  anchor fallback dead-ended on the unfolded rewrite.
  The IPC transport was unaffected and the dispatch gate
  itself was never weakened. Fixed: the host joins the
  list; pins added in addr_ipc + stats_tests, including
  dispatch through the rewritten spelling.
- **A2-1 (MED) — the prune flush could lose a whole
  report.** The runtime chunked BYTES at the host's
  ceiling but sent EVENTS uncapped; a >100,000-event
  flush was rejected whole by the host parser AFTER the
  runtime had discarded the accumulators. Fixed: both
  fields chunk, only the sent amounts are subtracted,
  the remainder rides the next flush; source-pinned.
- **A2b-1 (MED) — the correlation TTL was not enforced
  at take time.** Below the 2,048-entry cap, the expiry
  sweep never ran, so a response 181–600 s after
  dispatch still banked a duration sample — exactly the
  outliers the 180 s TTL exists to exclude. Fixed:
  `take` forgets durations past the TTL (the type still
  returns, so bytes land in the right slot).
- **The A2b LOW cluster** — all fixed: humanizer mirrors
  diverged at exact rounding ties (Rust `{:.1}` is
  half-even, JS `toFixed` half-up; 12.25 → "12.2" vs
  "12.3") — Rust now quantizes half-up and tie fixtures
  pin both suites; `view_signature` omitted three
  rendered figures (latent band freeze) — all 18 now
  hashed; `reset()` did not restore the prune log
  budget its doc claimed; the per-tab prune gate map
  was never forgotten on tab close — `forget_tab` is
  now wired like the Shield's and Lite's; re-noting an
  existing correlation token ran the cap eviction and
  discarded a live entry; zero-millisecond durations
  counted as samples (zero bytes never did); malformed
  Content-Types (`/`, `text/`, comma-joined) taught the
  ledger — the bucket classifier now refuses them; the
  seed accepted a non-empty book and an uncapped value
  — it now requires a completely empty book and takes
  the 2⁶² cap.
- **A1-C3 (LOW)** — the `shield_pruned` pre-filter now
  requires the literal `kind` key as well as the value,
  matching `lite_chosen`'s discipline. (Superseded and
  strengthened by A1-2 below.)
- **A1-2 (LOW) — report pre-filters could swallow a
  nav note.** The content-IPC handler is an if-cascade
  whose branches return unconditionally, with nav notes
  parsed last; the report pre-filters matched the bare
  quoted marker word ANYWHERE in a body, so a page
  whose title contained `"shield_prune"` (or the other
  markers) had its nav note routed into a report
  branch, rejected there, and lost. Impact was a lost
  title hint (SourceChanged stays authoritative), but
  the fix is systemic: all three report pre-filters
  (`shield_prune`, `lite_chosen`, `shield_pruned`) now
  match the kind ASSIGNMENT `"kind":"<name>"` — the
  body's own kind field, disjoint by construction (the
  closing quote keeps `shield_prune` disjoint from
  `shield_pruned`). Pinned by a nav-note-with-markers
  regression test. Found in A1's full report after the
  first battery run had started; the battery was
  stopped at 45 consecutive passes, the fix applied,
  and the count restarted from zero — see
  BATTERY_100.md.
- **A1-3 (LOW) — prune-report gate hygiene.** A forged
  or empty 0/0 `shield_pruned` report consumed the
  tab's rate slot and one of the 20 session log
  samples, and session EVENTS had no ceiling (bytes
  had one). Fixed: 0/0 reports are refused before any
  gate is touched (bytes 0 with events > 0 stays
  legitimate — zero-size subtrees exist), and a
  100M-event session ceiling now bounds both
  dimensions.
- **A1-C2 (LOW)** — the chrome badge renders the
  snapshot's version verbatim; a source-scan pin now
  guarantees the snapshot is built from
  `core::DISPLAY_VERSION` ("0.30"), never from
  `CARGO_PKG_VERSION` ("0.30.0"), so the surfaces
  cannot diverge on a future bump.
- **A1-C1 / A3-3.1 (LOW)** — in the zero state the tiles
  (and their method tooltips) are hidden; the zero
  panel's copy now carries the method clause itself
  ("Requests are counted exactly; the data and time
  figures are calibrated estimates from your own
  traffic."). The Reset control stays visible in the
  zero state DELIBERATELY: it also clears the
  calibration ledger, which exists before any block.
- **Maintainer find (LOW, latent since v24)** — the
  nav handler's local `is_action_url` knew only three
  action prefixes; the v24 recents actions' anchor
  transport silently dead-ended on it (their IPC
  transport always worked). It now delegates to the
  core classifier, so every action family reaches the
  same gated dispatch by either transport. Verified:
  classification alone still grants nothing — the
  dispatch's conjunctive gate decides, and a content
  page navigating itself to an action URL is cancelled
  by the same path as before.

## Disproven (verify-before-fix earns its keep again)

- **A5-H1 (claimed MED) — "the legacy seed has no
  caller; upgrading users lose their count."** False:
  the caller is at app startup (load → seed when the
  book is empty and the legacy total is positive), and
  the seeded value persists because save_state writes
  the book and mirrors the legacy field FROM it.
- **A5-M1 (claimed MED) — "the save-failure latch is a
  non-atomic two-flag race."** False: it is a single
  AtomicBool claimed by `swap` — exactly one log line
  per run. That it never re-arms mid-session is the
  documented v29 law, not a race.
- **A5-L1 — "the seed log prints the unsanitized
  total."** The value printed is the post-sanitize one
  (load sanitizes before startup reads it) — the same
  figure the band shows.
- **A2-5 — "$document blocks credit duration but not
  bytes."** Both credit functions return 0 for the
  excluded types before any calibration lookup.
- **A2-2b — "dual hooked surfaces double-measure a
  body."** [CORRECTED 0.30.1: true only within one
  object. A fresh PARSE of the same body creates new
  objects and a fresh removal set, which IS measured
  again (A2-C5) — the verdict below is wrong on that
  axis; see the errata.] Deletes are idempotent across the parse/XHR/
  fetch layers (a second pass finds nothing to delete,
  and measurement only follows a successful delete);
  response bodies are single-consumption. The only
  multi-count is a body genuinely downloaded twice —
  which is correct counting.
- **A2-S1 — "byteSizeOf can throw into the page's
  JSON.parse."** It already catches and returns 0;
  accounting can never break the prune.
- **A1-C4 — "on_shield_pruned lacks lite_chosen's
  liveness check."** Both handlers check the tab's own
  view slot identically.
- **A3-3.3 — "assets/icon.png is an unreferenced
  leftover."** No such file exists in the tree (the
  icon pipeline is pico.ico / icon_64.rgba / pico.rc).
- **A3-3.4 — "start.html has a stray </div>."** A real
  parser shows the document balanced; the count came
  from `<div>` literals inside JS template strings.
- **A4-2 (in part) — "chrome.html's badge fallback
  still says 29."** It says "0.30"; the file contains
  zero "29" tokens. (The same report's cosmetic point
  about the README's bare social handles was taken:
  they are full URLs now.)
- **A4-3 (in part) — "no battery script exists."**
  `scripts/battery.sh` exists and is what produced
  BATTERY_100.md.
- **A6-1/A6-2 (LOW) — history landing classification.**
  [CORRECTED 0.30.1: this verdict is FALSE for the
  `record_traversal` seam. The reconstruction drove
  `record_commit`/`record_pop` only; the SourceChanged
  consumer still step-replaced, and the S1 trace
  ([a,b,c], Back intent, landing on a) corrupts to
  [a,a,c] through it — a HIGH, fixed in 0.30.1. See the
  errata at the head of this file.] The auditor's executed sequences were reconstructed
  against the real machine: Back-then-navigate yields
  [A,B,D] under every signal mapping — correct browser
  semantics, not corruption. The landing fallback they
  flagged is v28/v29's deliberate, test-pinned answer
  to uninitiated traversals (mouse buttons, coalesced
  jumps), which WebView2 gives no signal to distinguish
  from fresh navigations to known URLs. Accepted
  design, recorded here as the standing trade-off: if
  Back/Forward availability ever drifts on device,
  this reconciliation is the first suspect.

## Accepted and documented (not defects)

- A1-4: both content-report handlers judge the sending
  tab's CURRENT URL (the events carry no document
  identity), so a report queued by one document can be
  judged against its successor's origin class. The
  pattern is symmetric with v29's `lite_chosen`, the
  windows are narrow, and the worst case is a dropped
  or accepted informational report inside the book's
  own caps — no path to privilege. Recorded as the
  channel's standing posture.


- A2b-3: host-refused prune reports are lost (above) —
  the rate floor is the anti-forgery defense.
- A2b-10: ledger sampling is gated on the REQUEST's
  origin class, exactly as documented — a loopback page
  loading a public CDN asset contributes that asset's
  real size; durations never accrue for un-noted
  requests, so local traffic cannot teach timing.
- A2b-13 / A2-3 / A2-2a / A2-4: type-true crediting,
  the removed-subtree event definition, the
  re-serialized measurement definition, and the
  pointer-ABA bound (all under Semantics, above).
- A2-7: prune accounting accrues in-page while the
  Shield is toggled off for that page; the host drops
  such reports while off — no wrong totals, minor
  wasted work.
- A3-3.2: the Shield chip's `clean_session` figure is,
  as documented since v28, the largest single-response
  count the runtime observed (an upper-bound estimate,
  labeled as such in its tooltip); unchanged in 0.30.
- A5-L3: `PushRule::HostSnapshot`'s empty-URL snapshot
  (carried from v28's audit as A5-L3 there) is
  unchanged — a synthetic devtools caller can at worst
  render one local page's data into the start view;
  the start page's privileged gate does not consult
  the snapshot.
- A5/HOST: state.json grows ~420 bytes for the book —
  one bounded write per save point, as before.
  [CORRECTED 0.30.1: the figure is 1,775 bytes of the
  1,962-byte default file, measured on the save path —
  see the errata.]

## Verification

- Tests: 353 Rust (lib + integration + doc) and 236
  DOM checks, all green — including the stats
  reconciliation corpus, the prune-measurement DOM
  proof (a two-€ subtree measured at its 14 removed-value
  bytes — the value-only rule of the errata),
  the band rendering fixtures in both theme modes,
  and the shared humanizer fixtures with ties.
- Battery: the full battery (cargo test all targets +
  dump_pages/DOM + full-list Shield regressions, with
  Windows-target clippy + release check every 10th
  iteration) run 100 consecutive times — see
  BATTERY_100.md for the log summary.
- Gate v2 PASS on the build exe and on the exe
  extracted from pico-0.30.zip. Strings: "0.30"
  present, "29.0.0" and "Pico v29" absent, each social
  URL exactly once. Exe 1,903,616 bytes (< 5 MB).
- Renders read by eye: start page with the band
  (Forest dark 1280 + 2048, Ember light, Violet dark;
  populated + zero state) — the band is native to the
  neumorphic system in every derivation rendered.
- Honesty: the prune-measurement and chosen-video
  proofs are jsdom/decision-level, as in v29. The
  on-device witness for 0.30: browse with Shield and
  Lite on → open the start tab → the tiles grow; hover
  a video thumbnail → still no preview; open the video
  → it plays; Reset returns the band to the zero state.

# Pico v29 — Audit Addendum (2026-11-01): LITE VIDEO SEMANTICS V2 + HOSTILE RE-AUDIT OF THE V28 DIFF

The user's report: v28 fixed Lite's hover previews and broke
the thing itself — "I want to play a video, but nothing
else — no preview from other videos or any other thumbnail —
but now the video itself isn't loading at all." Method: the
same six read-only audit areas as v28, this time instructed
to treat the ENTIRE v28 diff as hostile evidence — every v28
claim re-proven or refuted against code — plus a maintainer
pass. Verify-before-fix throughout. Shield filter LISTS are
byte-identical to v28 (diff-verified).

## The witness inversion (read this first)

v28's proof obligation was "previews die", and it was met —
per layer, never composed. v29's obligation is the inversion:
**the chosen video LIVES**, proven composed, while previews
keep dying:

- Rust: `regression_watch_page_main_video_lives_under_lite`
  — on a watch-shaped page under Lite, the main video's
  segment fetches (Xhr/Other stream shapes) and direct media
  loads pass the REAL `filterpipe::decide`; its poster and
  thumbnail images stay LiteBlocked.
  `matrix_lite_chosen_tab_exemption_is_media_only` — the
  chosen-tab exemption covers media/streams, never images.
  `v29_full_lists_do_not_shield_block_the_chosen_videos_playback`
  — the same playback URLs pass with the FULL shipped lists
  and the Shield ON (settles A2-H2 below empirically).
- DOM (jsdom): the watch fixture — a fresh watch-page load's
  main video `play()` succeeds with NO gesture, its retry
  `play()` passes, the designation reports exactly once, and
  the sidebar preview is refused on the same page; the
  tap→SPA-route fixture crowns the next LARGE video and a
  further route change retires the designation; the Shield
  innertube prune suite stays green with the gate installed
  on a watch page.
- Honesty: this proof is jsdom-level for playback and
  decision-level for bytes. The on-device witness is the
  user's: Lite ON → open a video → it plays; hover other
  thumbnails → nothing moves; the Lite counter rises from
  blocked images/previews only.

## The headline: why the chosen video died in v28 (V29-LITE-1..3)

Three causes, each confirmed in code by A1 and A2
independently:

- **V29-LITE-1 (network, class rule).** `decide` LiteBlocked
  every Media-class request on every public page. A direct
  `<video src>` the user clicked could pass the play gate
  and still never receive a byte.
- **V29-LITE-2 (network, stream backstop).** The v28
  backstop LiteBlocked every Xhr/Other request whose URL
  was a media-stream shape — `/videoplayback`, `.m3u8`,
  `.mpd`, segment suffixes. The MAIN YouTube player streams
  through exactly those shapes (MSE). The backstop could not
  tell the chosen video from a hover preview, so it starved
  both.
- **V29-LITE-3 (document, play gate).** The gate's intent
  was a per-element 2 s gesture mark in the SAME document,
  or transient document activation. The chosen flow defeats
  both: a thumbnail click marks a different element in a
  different subtree (or a different document entirely after
  full navigation), and the player calls `play()`
  programmatically after setup — on the user's ~300 kbps
  link, far outside any 2 s window. (In WebView2 the
  activation fallback was additionally inert in the tested
  configuration — dead code carrying a law.) The v28 gate's
  own doc comment stated the repealed law: "deliberately NO
  watch-page exception… watching is what turning Lite off is
  for." The user overruled it; v29 repeals it everywhere it
  was written (gate, README, chrome tooltip — the DOM suite
  pinned the old tooltip text; it pins the new law now).

## The corrected law: Lite Video Semantics V2

With Lite ON: **the video the user CHOSE plays (loads +
plays normally); hover previews and every other unsolicited
playback stay dead; thumbnail/poster images stay
Lite-blocked.**

- **Chosen-video model (document layer).** Each page context
  designates at most ONE chosen media element, by: (i) a
  direct tap on the element or its controls container (the
  gesture marks the element, its media ancestors, and the
  media of any container around the tap holding ≤ 3 media —
  a player, not a feed); (ii) a route change (the gate hooks
  pushState/replaceState/popstate itself) that follows a
  pointerdown anywhere within 2 s → the next LARGE video
  (≥ 320 CSS px wide, or ≥ 40% of the viewport) to request
  `play()`; (iii) a fresh load on a watch-shaped URL → the
  first LARGE video to request (a size-less element falls
  back to first-video-in-DOM-order). Designation resets on
  route change and on Lite toggle; the chosen element's
  repeated `play()` calls always pass (players retry); once
  chosen, all other elements stay refused, including on
  watch pages' sidebars. Hover events are never listened to.
  v28's document-activation pass is DELETED (A2-F7: it
  blessed the document, not an element — click anything and
  previews could play inside the window; the DOM suite now
  pins an unrelated click blessing nothing).
- **Watch-shaped corpus** (one definition, two spellings —
  Rust `core::lite::is_watch_shaped_url`, mirrored in the
  gate's JS, both pinned): a whole path segment, query and
  fragment excluded, case-insensitive, in
  {watch, video, view, play, episode, clip}. `/watchtower`,
  `/watch-tower`, `/?next=/watch`, `/videos` are NOT watch
  pages; `/WATCH`, `/shows/episode/3` are.
- **Network narrowing.** `filterpipe::decide` takes two new
  facts: `page_watch_shaped` (derived by the handler from
  the page's live URL) and `tab_chosen` (the tab's flag in
  the Lite runtime). Media-class requests and stream-shaped
  Xhr/Other requests are LiteBlocked ONLY when the page is
  not watch-shaped AND the tab has no chosen video. Images
  and fonts are blocked unconditionally on public pages.
  The exemption lives INSIDE Lite's steps, never above the
  Shield's — a rule-hit request is still the Shield's (A2's
  verified-clean constraint), and the full-list test above
  proves the shipped lists never rule-hit real playback.
- **The chosen flag's seam.** On designation the gate posts
  one payload-free body, `{"kind":"lite_chosen"}` (the
  content channel's `kind` dialect — A1-M1: the brief's
  `{t:…}` spelling belongs to the chrome protocol and would
  have been silently dropped; the dialect was corrected,
  the payload-free design kept). Core validation rejects
  unknown fields outright. The app honors it only from a
  live, public, non-start content tab with Lite on; the
  first designation per document logs one line through a
  session-capped budget. The flag is cleared on full
  navigation (CommitStart), Lite toggle (either way) and
  tab close — and NOT on SourceChanged (A2-F6: clearing on
  route would erase the SPA bridge before its segments
  arrive). A forged report's worst case is one tab's Lite
  data-saving on media/stream requests until the next
  navigation — it cannot unblock images/fonts, move a
  counter, or reach any privileged path, and the play gate
  still refuses unchosen playback in that tab. That is the
  brief's accepted-gap shape: the backstop is a data-saver;
  the gate is the guarantee.
- **Gate timing (A2-F4).** The gate now ALSO rides as a
  document-start initialization script (main frame only,
  chained after the NAV observer) whenever Lite is on at
  tab build. Through v28 it arrived only at PageLoad
  Finished — after the player's first play() attempt and
  first fetches were already decided. The Finished/toggle
  evaluate pushes remain as the re-arm path for tabs built
  while Lite was off.

## The fan-out

| Area | Findings | Confirmed-fixed | Disproven | Accepted / documented |
|---|---|---|---|---|
| A1 chrome/IPC | H1 (headline, shared), M1 seam, M2 interaction, H2 design risk | 3 (H1 via V29-LITE, M1, M2 by construction) | 0 | 1 (H2 videoId grant design — declined, below) |
| A2 filter pipeline | 15 (F1–F15: 3 crit, 3 high, 6 med, 3 low) | 9 (F1 F2 F3 F4 F6 F7 F8 F12 F13) | 0 | 6 (F5-variant, F9, F10, F11-behavior, F14, F15) |
| A3 themes/start | 0 | 0 | 0 | 0 (v28 claims all re-proven) |
| A4 teleport/QR/LAN | 0 | 0 | 0 | 0 (v28 claims all re-proven) |
| A5 server/state/log | 10 (A5-1..10) + 1 channel demand | 9 (A5-1..9) | 1 (A5-L3-equivalent, below) | 1 (A5-10) + channel demand declined |
| A6 navigation/lifecycle | 11 (H1, M1, M2, L1–L8) | 9 (H1 M1 M2 L1 L2 L4 L6 L7 L8) | 0 | 2 (L3, L5) |

Plus one maintainer find (V29-C1) and one integration
consequence (the doc-start gate) found while fixing F4.

## A6 — the v28 traversal fix was narrower than claimed (V29-A6-H1)

The round's most serious non-Lite find. v28's A6-1 fix made
SourceChanged authoritative for traversals ONLY when the
landing matched the pending target byte-exactly; every other
adopted Source change defaulted to `record_push`. An
UNINITIATED SPA traversal (mouse buttons, gestures, the
page's own `history.go()` — every SPA's in-app back) landing
on an existing entry was therefore recorded as a push:
entries [a,b,c] on b became [a,b,c,b], the forward branch
destroyed, Pico's history permanently diverged from the
WebView's. `record_traversal`'s redirect/multi-step branches
were unreachable in production for the same reason.

Fix (classification, in `UrlState::apply_landing` — the app
passes one new fact, whether the Source URL already lives
in the tab's history at a non-current entry): a pending
intent (any) → Traversal (`record_traversal` reconciles
exact, redirected and coalesced landings); no intent +
in-history → Pop (`record_pop`'s landing search moves the
index, forward branch preserved); otherwise Push. The
ambiguity is documented in code: a pushState to a
previously-visited URL is indistinguishable from a
traversal in the Source signal and is now recorded as a
move — chosen because uninitiated traversals are constant
in real SPA use and were permanently corrupting, while a
same-URL pushState is rare and self-heals on its next
landing. Composed UrlState→TabHistory tests pin all three
shapes plus the control (genuinely new URL still pushes).

Also in A6: **M1** — typing `pico://start` dead-ended on
content tabs and corrupted a start tab (conversion flipped
its state, then the load was refused); `navigate_active`
now routes it to `go_home()` before the route table.
**M2** — `record_commit`'s pending branches blindly stepped
one and replaced ([a,b,c] landing on a became [a,a,c]);
they now consult the landing search first, keeping the
redirect-replace only for landings no entry matches.
**L1** failed start tabs now retry on conversion (A6-3's
fix was content-only). **L2** the failed-tab retarget is
total (history machine + pending-main reset too). **L4**
`go()` records no intent on a view-less tab. **L6**
`record_replace` can no longer leave consecutive duplicates
([a,b,a] → [a,a]). **L7** dead `UrlState.doc_open` deleted.
**L8** the Lite toggle's "synced to N live views" count now
names only views the layers can actually arm (public
content tabs).

## A5 — portable data honesty (V29-A5-*)

- **A5-1 (MEDIUM, the round's second real bug class).**
  Migration's "state presence = completion" marker was
  dishonest on ANY copy error, not just crashes: a locked
  profile file still landed state.json, the planner stood
  down forever, and the log read as success. Worse, the
  planner couldn't tell a partial portable profile from a
  live one. Fix, three parts: the report splits `failed`
  from `skipped`; state is withheld when the profile phase
  had failures; and a `migration.pending` marker (written
  before a full copy, removed only at clean completion)
  tells the planner to RESUME a partial copy instead of
  taking the state-only shortcut — while a live profile
  whose state was lost (no marker) still never gets
  recopied over (the v28 clobber law). The app logs an
  incomplete migration AS incomplete, and retires a stale
  marker when state is present.
- **A5-2** — temp cleanup on load no longer deletes FRESH
  temps: per-process temp names + no single-instance guard
  meant one instance's load could delete another's
  in-flight save temp. Anything under a minute old is
  treated as live; crash leftovers age past the guard.
- **A5-3** — the file server's streamed length now comes
  from the OPENED handle (not pre-open metadata), ranges
  clamp to it, and a mid-stream shrink returns an error
  that closes the connection instead of leaving a short
  body on a keep-alive stream (protocol desync).
- **A5-4** — the log's invisible-format set now covers the
  bidi isolates U+2066–U+2069, U+061C, U+00AD, U+180E,
  U+206A–U+206F, U+FFF9–U+FFFB (one shared helper; both
  cleaners inherit).
- **A5-5** — temp recovery picks the NEWEST complete temp
  (was: read_dir order — could resurrect the oldest intent),
  and a corrupt main file now falls back to temp recovery
  instead of the cleanup destroying the recovery source
  before the parse.
- **A5-6** — the reload hub tombstones dropped roots: a
  subscribe racing `drop_root` can no longer resurrect a
  root entry and park a connection forever.
- **A5-7** — the portable-dir probe now includes a rename
  round-trip (every save needs one; a write-only filesystem
  previously passed and failed saves all session), and
  startup no longer stats the appdata tree when portable
  state already stands the migration down (a UNC-pointed
  APPDATA was an SMB touch for a migration that couldn't
  happen).
- **A5-8** — migration's atomic copy fsyncs before rename
  (like `save_to` always did); uncopied symlink entries are
  counted in `skipped` instead of vanishing from the report.
- **A5-9** — a stale test header claiming log redaction
  hides server root tokens (the opposite of the shipped,
  documented decision) is corrected so a future auditor
  doesn't "fix" the code back.
- **V29-C1 (maintainer).** A persistent state-save failure
  logged one error per save — and saves ride title-bearing
  navigation notes, so a long session could grow pico.log
  by thousands of identical lines. The failure now logs
  once per session (it remains user-visible in effect:
  state simply doesn't persist).

## Declined and accepted, with reasons

- **A1-H2 / A5-channel (grant design).** Two auditors
  independently proposed richer chosen-grant machinery
  (videoId-bound grants with an href payload; or routing
  the report through the chrome command channel). Declined:
  the parent design specifies a payload-free content
  report, and the chrome channel is the shell's privileged
  surface — content pages must never speak it. What was
  taken from both: the `kind` dialect (A1-M1), sender
  validation at processing time, transition-only logging
  under a session budget, no snapshot side-effects, Shield
  verdict still senior, images/fonts never exempt. The
  residual forgery surface is documented in SECURITY.md.
- **A2-F9 (corpus reach).** The corpus stays exactly as
  specified (watch/video/view/play/episode/clip). YouTube
  /shorts, /live, /embed paths are not watch-shaped: a
  TAPPED Short still works end-to-end (click designation
  sets the tab flag on any page; the gate crowns it), but a
  pasted shorts link gets no no-gesture arm. Accepted gap,
  named here rather than silently widened.
- **A2-F10 (embeds).** Iframe players are a known
  limitation: request facts carry the top page, and the
  gate is a main-frame law (now explicitly main-frame at
  document start too). An embedded player under Lite plays
  only when the top page is itself watch-shaped or chosen.
- **A2-F11 (toggle-on sweep).** Re-arming Lite resets
  designations and re-derives the watch arm from the
  current route: a playing main video on a watch page
  survives a Lite toggle; a playing video elsewhere needs
  a fresh tap on the player. Predictable, and stated here.
- **A2-F14.** Lite is method-blind by design (the Shield
  owns the method dimension); POST stream shapes on
  non-watch, unchosen pages stay LiteBlocked.
- **A2-F15.** Load-order proof gap (gate evaluated before
  the Shield payload in one app branch, after it in the
  fixture): surfaces are disjoint and string-pinned; noted.
- **A6-L3.** Same-URL reload preserves `source_seen_in_doc`
  into the new document (A6-9's preservation over-fires):
  indistinguishable from Source-before-CommitStart with the
  signals available; Source remains authoritative when it
  fires. Accepted as A6-9's standing trade-off.
- **A6-L5.** Address-matrix edges outside the v28-claimed
  forms (full-URL dotted-quad spellings, `127.1`, trailing-
  dot localhost in `parse_input`): documented here as
  known inconsistencies; the claimed forms are all pinned.

## Disproven this round

- **A5-L3-equivalent (range comment).** The claim that
  `parse_range`'s suffix comment misdescribes its digit
  check: re-read centrally — the comment accurately
  describes the check directly above it. No change.
- **A click-gesture/snapshot tab-clobber race** (from an
  early compressed auditor summary): the mechanism could
  not be located in the code on central re-read, the full
  report of that area (A1) records no such finding, and no
  fix was shipped blind. Recorded so it isn't resurrected
  from the summary.
- **A2-H2 as a blocker.** The concern that Shield-first
  ordering defeats the chosen exemption on real lists:
  settled empirically — under the full shipped lists the
  playback catalogue passes with Shield ON (new test). The
  ordering stands by design.

## Verification totals

330 Rust tests (v28: 316) and 219 DOM checks (v28: 205),
all passing; clippy 0 on host and Windows targets; fmt
clean; gate v2 PASS; Shield lists byte-identical to v28;
unsafe set unchanged (17 blocks). The v28 regression
fixtures were kept and re-purposed: the DOM section that
proved "a container gesture passes a second video" now
proves the chosen model's opposite, as the law changed.
Full numbers and hashes: BUILD_NOTES.md.

---

# Pico v28 — Audit Addendum (2026-11-01): SIX-AREA LINE-BY-LINE RECHECK + LITE HOVER-PREVIEW FIX

The user's order: "do a full recheck all codes and lines,
fix everything" — and one named bug: with Lite mode on,
hovering a YouTube thumbnail still plays the inline preview.
Method: six read-only audit agents fanned out in parallel
(one per area below); every claim was verified against the
code by the maintainer before any fix was written
(verify-before-fix — this project's history has confident
false findings, and this round produced one again, A1-L4).
Fixes were implemented centrally. Shield filter LISTS are
byte-identical to v27 (diff-verified); all v27 behavior is
preserved unless a confirmed bug below says otherwise.

## The fan-out

| Area | Scope | Findings | Confirmed-fixed | Disproven | Accepted / documented |
|---|---|---|---|---|---|
| A1 | chrome JS, IPC, snapshot, dedupe, focus, panels | 11 (1H 1M 7L 2I) | 10 | 1 (L4, as stated) | 0 |
| A2 | Shield engine, filterpipe, Lite, collapse, scriptlet runtime | 25 (3H 8M 14L) | 22 (+1 partial) | 0 | 3 |
| A3 | themes derivation/contrast/purity, start page | 8 (4M 4L) | 8 | 0 | 0 |
| A4 | Teleport, QR encoder, LAN chooser | 9 (3M 6L) | 7 | 0 | 2 |
| A5 | static server, state, portable data, log | 9 (1H 2M 6L) | 9 | 0 | 0 |
| A6 | address/URL-state/history, app lifecycle | 14 (1H 5M 8L) | 10 | 0 | 4 |

Plus one integration find (V28-INT-1) made by the DOM lab
during verification — the most serious scriptlet bug of the
round, invisible to every static audit.

## The headline bug: Lite vs YouTube hover previews

**Root cause.** YouTube hover previews stream through Media
Source Extensions: the segment fetches are fetch/XHR-context
requests (WebView2 reports them as `ResourceType::Xhr`,
never Media), and the preview `<video>` then plays from a
`blob:` URL. Lite's network gate only ever blocked the
Image/Font/Media resource classes, so the preview's bytes
never crossed a Lite rule at all — Lite was working exactly
as built; the preview simply wasn't made of anything Lite
saw. A2 confirmed there was no URL-shape inspection anywhere
in the Lite path.

**The fix — three generic layers, no host lists.**

- **V28-HP-1, the play gate** (document layer,
  `core::filterpipe::LITE_GATE_RUNTIME`): while Lite is
  active for a page (same loopback/internal exemption as
  every Lite law), `HTMLMediaElement.prototype.play` is
  wrapped. A play request with no genuine user intent for
  that element is refused with a `NotAllowedError` rejection
  — the shape a browser's own autoplay policy produces — and
  the element stays paused. Genuine intent = a real
  pointerdown/click/keydown marked the element, an ancestor,
  or a container holding it within 2 s, or the document
  holds transient user activation. Hover events are never
  listened to, so they can never mark anything. A
  capture-phase `play`-event belt pauses media that starts
  without the wrapper (attribute autoplay), and installing
  or re-arming the gate sweeps already-playing unmarked
  media. There is deliberately NO watch-page exception:
  under Lite a watch page's media is network-blocked anyway,
  so gating its play() changes nothing observable — Lite's
  law stays uniform (video does not play while Lite is on;
  turning Lite off is the watching path).
- **V28-HP-2, the network stream backstop**
  (`core::lite::is_media_stream_url` + the Xhr/Other branch
  in `filterpipe::decide`): while Lite is on, fetch/XHR-class
  requests whose URL path is a media-stream shape — a whole
  `/videoplayback` segment, or `.mpd/.m3u8/.mp4/.webm/.m4a/
  .m4v/.ts` — are LiteBlocked. The matcher is deliberately
  narrow: innertube API calls (`youtubei/v1/player`,
  `/browse`, …) are NOT stream shapes and are never blocked
  here, because the Shield's prune path must transform those
  responses, not drop them. This starves previews that try
  to buffer before play.
- **V28-HP-3, live state sync**: the gate follows the LIVE
  Lite toggle. Toggling Lite on evaluates the combined Lite
  document script (collapse layer + gate, one script) into
  every live content view and re-arms/sweeps; toggling off
  sets `window.__picoLiteGateOff = 1` on live views (the
  wrapper then passes through byte-exactly), and
  `__picoLiteGate.restore()` restores the exact original
  prototype method and removes the listeners. Load boundaries
  maintain the flags. The sync count is logged
  (`lite: toggled on; document layer synced to N live view(s)`).

**Proof.** Rust: the filterpipe matrix gained the
stream-shape corpus (true shapes block; innertube/API JSON
never does). DOM (jsdom): hover-style play() refused;
play() after a synthetic pointerdown passes the gate;
container gestures mark their media; OFF flag is a
passthrough; a second install re-arms without double-wrap;
restore() returns the exact original function; a loopback
fixture never installs; and the innertube prune suite stays
green with the gate installed on the same page (the gate
wraps `play`, the scriptlets wrap fetch/XHR/JSON —
orthogonal surfaces). HONESTY: this proof is jsdom-level.
The final verdict is the user's own hover — see
BUILD_NOTES for the witness.

## V28-INT-1 — the integration find: `set` pins could never yield (found by the DOM lab)

A2-H1 made `defineConst` honor the Shield OFF flag by
capturing the property's real value at pin time. The DOM
lab's set-scriptlet fixture then failed: the pin read back
the constant even with the flag set. Cause: `applySet`
applies a chain TWICE at install — once for the
already-present root object and once through the root
handler's initial call — so the second `defineConst` read
the CURRENT value through the first pin's getter and
recorded the constant itself as the "real" value. Any
multi-segment `set` chain whose root existed at install
(the common case) was poisoned. Fix: pin state lives in a
shared record (WeakMap-keyed by object); re-pinning updates
only the constant, the original real value stands. The lab
fixture now proves the full cycle: pinned while on, real
value while off, assignments land while off, pin returns
when the Shield comes back.

## A1 — chrome JS / IPC / snapshot (10 fixed, 1 disproven)

- **V28-1 (H1)** Teleport's open state now rides the
  snapshot (`teleportOpen`), exactly like the picker's. A
  Rust-side close (navigation, tab switch, conversion) is
  mirrored by the chrome instead of leaving a stale panel
  painted — which also resurfaced over the picker and
  swallowed the next Teleport click as a phantom close.
  The mirror does not echo `teleport_close` back. A4's M2
  was the same defect seen from the Teleport side.
- **V28-2 (M1)** `push_state`'s dedupe gate is now a cheap
  hash over the snapshot's primitive fields, computed BEFORE
  any construction. A gated-out push no longer clones the
  themes catalogue and the QR matrix and serializes the
  whole snapshot just to discard it (block events call this
  per blocked request). The catalogue is a `&'static` from a
  OnceLock; the Teleport payload is represented in the
  signature by a version counter bumped at every assignment.
  The one-shot focus hint still forces its push.
- **V28-3 (L1)** Dedupe adopts a new boot id only when there
  is no live boot, or on the new generation's `ready`
  handshake. A stale queued command from a chrome page that
  reloaded without the watchdog can no longer flip the live
  boot and clear the live seen-set (stale execution +
  double execution). A genuine generation always introduces
  itself with `ready`, so it loses nothing.
- **V28-4 (L2)** Escape cancels a PENDING Teleport opening
  (`tpAwaiting`), not only an open panel.
- **V28-5 (L3)** `cycle_tab`'s `dir` is validated in
  `parse_command`: ±1 or the command is rejected (a raw
  i32 reached the tab-index arithmetic before).
- **V28-6 (L5)** Oversize chrome IPC bodies are dropped in
  the handler, before clone/queue, not after queueing.
- **V28-7 (L6)** Tab DOM is keyed reconciliation: existing
  tab elements update in place, only departed tabs are
  removed, new tabs are created. A routine snapshot no
  longer destroys the focused tab element (focus used to
  drop to <body>, silently killing arrow/Delete tab keys).
- **V28-8 (L7)** Teleport Copy reports honestly: the
  `teleport_copy` command and the "Copied" label fire only
  on an actual clipboard success; failure shows
  "Copy failed" and logs nothing.
- **V28-9 (I1)** Dead snapshot fields removed
  (`Snap.maximized`, `Snap.other_listeners`,
  `TeleportSnap.state` — sent every push, read by nobody).
- **V28-10 (I2)** Asset census now covers the Teleport
  panel internals (`tp-close/-url/-qr/-note/-copy`); IPC
  parse pins added for switch_tab/back/forward/home/
  open_file/theme_toggle/win_*; DOM gained the panel
  mutual-exclusion suite.
- **L4 DISPROVEN as stated:** a `theme_set` id cannot inject
  a raw newline into pico.log — the app's log sanitize
  replaces every Cc control on every write (A5 re-verified).
  The residual class A1 pointed at — Unicode line/paragraph
  separators and bidi controls that render as line breaks —
  was real, and is fixed under A5-L6 (V28-35).

## A2 — filter pipeline (22 fixed, 1 partial, 3 accepted)

- **V28-11 (H1)** `set` scriptlets honor Shield OFF:
  `defineConst` tracks the page's real assigned value and
  serves pin-while-on / real-while-off; `applySet` creates
  nothing while off. (See V28-INT-1 for the composition bug
  the DOM lab caught in this fix's first form.)
- **V28-12 (H2)** The collapse layer no longer collapses on
  `<source>` errors (candidate fallthrough is the element's
  job) and no longer collapses an `<img>` instantly: an
  error schedules a deferred revalidation (350 ms) that
  collapses only if the element is still connected, still
  on the same src, complete and `naturalWidth === 0`. The
  v27 behavior broke `<picture>` fallback selection and
  onerror fallback-src swaps.
- **V28-13 (H3)** Unsupported uBO pseudo-actions
  (`:style(`, `:remove(`, `:matches-path(`, `:remove-attr(`,
  `:remove-class(`) are classified as procedural skips
  instead of shipping as CSS that could poison a whole
  concatenated host rule. Cosmetic accounting now starts
  from the real runtime length; the budget is 344,000 and
  the shipped payload measures 341,567 bytes over 1,119
  compiled specific hosts (v27's "1,152" included 33 hosts
  whose only cosmetics were unimplementable — now honestly
  counted as procedural skips, not coverage).
- **V28-14 (M1)** The Lite stream backstop (V28-HP-2).
- **V28-15 (M2)** The dead `ShieldRuntime::should_block`
  seam is deleted — it had no production callers, and its
  tests asserted POST blocking the real pipeline never
  performed. The pipeline is the spec now: Shield blocks
  GET and HEAD (HEAD was silently exempt through v27), POST
  is never Shield-blocked; both laws are pinned in
  filterpipe's matrix, where production actually runs. The
  Document→Subdocument reclassification survives as the pure
  `shield::effective_rtype`.
- **V28-16 (M3)** Entity-scoped cosmetics can match: the
  runtime computes an entity key (registrable domain with
  the two-level-suffix adjustment) and checks it alongside
  the suffix walk. Before, payload keys like `example.*`
  were unreachable.
- **V28-17 (M4)** Scoped `#@#` exceptions now apply against
  GENERIC selectors too (a `generic_exceptions` map and a
  `ge` payload channel); through v27 an exception against a
  generic selector silently did nothing.
- **V28-18 (M5)** Anchored rules with implausible hosts
  (wildcards, host:port, trailing `.^`) no longer compile to
  dead rules: non-flag rules are downgraded to generic
  patterns over the text after `||`; page-flag rules keep
  their raw anchor for scoping.
- **V28-19 (M6)** The XHR transform is getter-first:
  per-instance getters over `responseText`/`response`
  transform lazily on first read at readyState 4, so a page
  handler registered before `send()` sees the transformed
  body (the v27 in-send listener hook ran after it). The
  event hook remains only as a fallback. The DOM fixture's
  FakeXHR gained real prototype accessors (WebIDL-shaped)
  so the suite proves the ordering case instead of the
  fallback.
- **V28-20 (M7)** The collapse layer's arming flag is
  maintained on TOGGLES as well as load boundaries
  (Shield and Lite toggle paths both).
- **V28-21 (M8)** badfilter/dedupe identity includes the
  exception bit: a block rule and its exact `@@` twin can
  no longer dedupe each other away.
- **V28-22 (L1/L2)** `0.0.0.0` is local in
  `shield::is_local_host`; the JS runtimes' loopback tests
  use the exact numeric 127/8 spelling of the Rust
  classifier.
- **V28-23 (L4–L6, L8, L12)** Exclude-only generic
  cosmetics produce inverted exception entries; the
  two-level-suffix table gained `co.nz` and `github.io`;
  `host_of` trims trailing dots; the scriptlet entity key
  handles two-level suffixes (`foo.co.uk` no longer yields
  `co.*`).
- **V28-24 (L9–L11)** Transformed fetch Responses no longer
  inherit the original Content-Length; transforms re-check
  the OFF flag at completion; `transformText` parses with
  the original JSON.parse where available (no double
  json-prune application, no double hit counting).
- **V28-25 (L13, partial)** The app's request handler reads
  page state once and shares it between main-frame
  determination and filter facts. The remainder — worst-case
  cost of naive segment search on very long URLs — is
  accepted (no cap imposed; see Accepted).
- **V28-26 (L14)** An important exception beating an
  important block is pinned by test.
- **Accepted:** L3 (negative page-flag scopes for cosmetics
  are latent; zero shipped occurrences), L7 (filter context
  uses the top-level page URL for iframe-originated requests
  — the WebView2 surface Pico uses does not expose the frame
  origin there; platform limitation), L13-remainder (above).

## A3 — themes + start page (8 fixed)

- **V28-27 (M1)** The picker/panel close glyph sits on
  `--danger` on hover; white on that hue failed AA on
  several palettes. New derived token `--on-danger`:
  best-of candidates stepped in its own luminance direction
  until it holds 4.5:1 on `--danger`, in all 18 themes
  (test-asserted).
- **V28-28 (M2)** Dead tokens `--accent-strong` and
  `--accent2` removed from the token set and the derivation
  output (slot 6 still feeds `--glow-soft` internally).
- **V28-29 (M3)** An unknown palette id in the state file is
  no longer rewritten silently: `sanitized()` flags it
  (`theme_id_replaced`) and startup logs the line once.
- **V28-30 (M4)** The contrast tests were partly circular
  (they re-derived with the code under test). An independent
  ratio pin now checks hand-recomputed constants (Forest
  dark text 10.31, accent 4.88; Violet dark text 11.35;
  Graphite light text 17.58; Bronze light accent 5.55;
  ±0.2).
- **V28-31 (LOW-1)** The picker render key is the catalogue
  (ids + names + slot hexes), not the selection; a palette
  switch updates `aria-pressed` in place instead of
  rebuilding the grid and destroying the focused card. The
  DOM fixture now gives every palette its own slots (the
  v27 fixture gave all nine Forest's) and asserts distinct
  dot colors, pressed-card movement, and focus survival.
- **V28-32 (LOW-2)** A same-id `set_palette` re-pick also
  re-pushes the theme to start tabs (a recreated start page
  heals instead of staying on the old palette).
- **V28-33 (LOW-3)** `PersistState.theme` has a serde
  default; `save_to` writes via a per-process temp file,
  `sync_all()`s, then renames; the loader recovers from a
  parseable temp sibling when the main file is missing and
  cleans stale temps after a good load.
- **V28-34 (LOW-4)** `Rgb::from_hex` is total (short input
  yields black, never a panic); the dead legacy-palette
  constant is removed.

## A4 — Teleport + QR (7 fixed, 2 accepted)

- **V28-35 (M1)** The LAN chooser is egress-aware, in core
  and in the app: the UDP egress address is probed always
  (it used to be a last-resort candidate only) and wins when
  it is among the usable candidates; otherwise up-state,
  then RFC1918 tier, then enumeration order. A merely
  enumerated adapter on the wrong subnet no longer beats
  the interface the machine actually routes through. The
  egress remains the fallback when enumeration yields
  nothing usable.
- **V28-36 (M3)** QR golden vectors now cover every version
  1–12 module-for-module (17 vectors; the test asserts the
  exact count and the version set). RESEARCH_v25's version
  claims were corrected (percent-encoded URL is v7, the
  long URL is v11).
- **V28-37 (M4)** Public Teleport URLs drop authority
  userinfo (host/port/path/query/fragment byte-exact; an
  `@` in the path is not userinfo). The QR and the copyable
  text no longer carry embedded credentials.
- **V28-38 (L1/L2/L3)** Trailing-dot loopback spellings
  (`localhost.`, `127.0.0.1.`) resolve as loopback;
  non-web schemes (`data:`/`blob:`/`view-source:`/
  `javascript:`) are pinned NotTeleportable; `with_lan_host`
  validates the port (`:0`, `:99999`, non-numeric are
  NotTeleportable, not dead QRs).
- **V28-39 (L5)** `Teleport::qr_url()` returns a URL only
  for LanRewrite/PublicUrl — a NoNetwork resolution no
  longer encodes its loopback display URL into a QR a phone
  cannot open. The app encodes `qr_url()`, and keeps
  `resolved.url` for display/copy.
- **Accepted:** L6 (IPv6-only LANs remain unsupported
  end-to-end; the gateway-less UDP fallback has a known
  hole), L7 (`QrMatrix::get` is test-only API; an Empty QR
  maps silently at the app layer).

## A5 — server / state / portable data / log (9 fixed)

- **V28-40 (H1)** Migration can no longer clobber a portable
  profile: the planner takes the portable-profile fact, a
  portable profile + appdata state yields a state-only copy,
  profile copies run before the (now atomic) state copy,
  and `copy_tree` counts per-entry errors instead of
  flattening them away.
- **V28-41 (M1)** The shared fixed temp name is gone:
  state saves and the migration state copy use per-process
  temp names with rename (two Pico processes could
  previously interleave one `state.json.tmp`).
- **V28-42 (M2)** `WindowRect::is_valid` compares in i64 —
  an `i32::MIN` coordinate overflowed the i32 abs() check.
- **V28-43 (L1)** A streamed file whose `File::open` loses
  the race after canonicalize now gets a synthesized 404
  (respecting HEAD/close semantics) instead of zero response
  bytes and a hung keep-alive client.
- **V28-44 (L2, documented)** Log redaction keeps URL paths
  (including Pico server root tokens) BY DECISION, now
  written in `core::redact`: the log's reader is the user on
  their own machine, the server is loopback-only and
  Host-walled, the token dies with the process, and query /
  fragment contents — where credentials actually live —
  remain hidden.
- **V28-45 (L4)** The exe-dir writability probe is public
  and pinned by a real tempdir test (writable dir probes
  true and creates `data/`; `data` as a regular file probes
  false and the resolver falls back).
- **V28-46 (L5)** A failed log open no longer silences the
  whole session: `write_line` retries the open whenever no
  file is held (init and rotation failures included).
- **V28-47 (L6)** Invisible Unicode format characters
  (U+2028/2029, U+200B–200F, U+202A–202E, U+2060–2064,
  U+FEFF) are stripped by both the redaction cleaner and the
  log sanitizer — they rendered as line breaks / bidi
  overrides and could forge log lines. One shared helper
  (`core::redact::is_invisible_format`) serves both.
- **V28-48 (L7)** The "startup argument ignored" log line
  passes the argument through `redact_url` before logging.

## A6 — navigation / URL-state / app lifecycle (10 fixed, 4 accepted)

- **V28-49 (A6-1, HIGH)** `HistEffect::Traversal` was a
  no-op in the app, but the history index had never moved —
  an intent only records `pending`. When SourceChanged beat
  the observer's pop note to a traversal landing, the index
  stayed on the old entry, Forward was lost, and the
  dangling intent then corrupted the NEXT commit (it
  overwrote the traversal's origin entry and left a phantom
  forward entry). The reducer's doc even claimed the index
  "already moved" — the opposite of `history.rs`'s contract.
  Now `TabHistory::record_traversal` moves the index to the
  pending neighbour (a redirect replaces the destination
  entry, as commits do) and consumes the intent; the app
  calls it. A composed UrlState→TabHistory test — the seam
  had none — pins both event orders.
- **V28-50 (A6-2)** Pre-commit observer notes are origin-
  gated against the SEED URL too: the gate no longer
  requires `doc_open`, closing a URL-spoof window on every
  fresh tab and conversion (a forged note could paint a
  foreign origin into the address bar until the first
  commit corrected it).
- **V28-51 (A6-3)** An address submission on a FAILED tab
  (no view) is no longer swallowed: the tab record is
  retargeted to the typed URL first, then Reload's retry
  path builds the view for the new URL. The submission is
  logged.
- **V28-52 (A6-4)** `open_path` checks the 30-tab cap
  BEFORE registering a server root / watcher (at the cap,
  both used to leak for the session), records the Recent
  only on a successful open, and releases root + watcher on
  any other `add_tab` refusal.
- **V28-53 (A6-5/A6-6)** The address matrix is coherent:
  an explicit host:port with an impossible port is REFUSED
  for every host shape (numeric, bracketed, alphabetic,
  bare `:port`, bare number) instead of falling through to
  a Google search for the numeric forms; bracketed IPv6
  with a path/query/fragment but no port navigates.
- **V28-54 (A6-7)** TabHistory pending-intent fragility:
  multi-step landings (coalesced double-Back, `history.
  go(-2)`) reconcile to the matching entry instead of being
  pushed as fresh (which truncated the forward branch and
  duplicated an entry); `record_replace` drops a pending
  intent it would strand; the shell never stacks a second
  traversal intent on a pending one.
- **V28-55 (A6-9)** Source/Commit ordering: CommitStart and
  CommitFinish preserve the Source flag when the Source
  event already adopted that very URL (a new document's
  Source can beat its CommitStart across channels;
  resetting unconditionally re-opened the note window to a
  late previous-document note), and a CommitFinish whose
  start was never delivered re-opens the window for its
  document. Both orders are test-pinned.
- **V28-56 (A6-11)** `convert_start_tab` conversion is
  total: it stamps load timing (`loading` /
  `last_load_start`), retires any Teleport payload/panel
  immediately (with layout), and seeds the Shield's pending
  main URL alongside the page URL.
- **V28-57 (A6-12, partial)** Page-initiated credentialed
  URLs are denied in `route_new_window` (a page must not
  mint a tab whose bar leads with a trusted-looking
  `https://google.com:…@evil…` prefix; TYPED credentialed
  URLs are the user's own act and pass verbatim). Bare
  loopback / unspecified IPv4 without a port goes over
  plain HTTP like its with-port form (`127.0.0.1` typed for
  a dev server no longer attempts TLS to 443; a bare
  PRIVATE address keeps the pinned https reading). A
  digits-and-dots host that is not a valid IPv4
  (`256.256.256.256:8080`) is refused, not navigated to as
  a public https host.
- **V28-58 (A6-13, partial)** New-window events are keyed
  by their source tab (a popup queued by a since-closed tab
  is dropped, like every other URL event) and a cap refusal
  gets its own log line naming the popup.
- **Accepted:** A6-8 (Source-as-authority cannot carry
  replace semantics — SourceChanged has no "how"; a
  replaceState adopted via Source is recorded as a push.
  The note that could distinguish them is exactly the
  signal v26 demoted for cause; documented in
  `urlstate.rs`), A6-10 (the chrome watchdog race is
  structural — handshake and timer travel independent
  channels; the one-shot recreation budget is the standing
  posture), A6-12 remainder (`parse_input` has no length
  cap of its own — IPC caps at 8192 and argv is OS-bounded;
  octal/decimal IP spellings are treated as public hosts —
  Chromium would canonicalize some to loopback, so Pico may
  over-filter such a page, never under-filter it),
  A6-13 remainder (the ScaleFactorChanged first-layout
  transient self-corrects on the following Resized; the
  1 px phys_to_rect clamp difference is unreachable above
  the window minimum), A6-14 (the two "dead" functions are
  `pub` core API with real tests — kept; the newwindow grep
  test's limits are noted, and the coverage gaps it listed
  are closed above).

## Clean areas (verified, credited)

A1: Rust-side panel exclusion paths, the theme application
path (no stale route), counter single-attribution, the
focus hint's exactly-once consumption. A2: verdict-owned
network counting, OFF passthrough, loopback/internal
immunity, the engine's ABP semantics core, scriptlet
injection safety, composition order, compile accounting,
the YouTube scriptlet catalogue tests. A3: the derivation
math was exact across all 18 themes; the glow census is
exactly 11; sentinel migration airtight. A4: QR capacity is
exact (287 bytes fits v12-M, 288 is TooLong) and the tables
verify against Nayuki; the panel data path is
injection-safe. A5: server path hardening, the Host wall
incl. SSE, the DoS limits, range math, the state loader's
Missing/Corrupt distinction. A6: address control-character
and UNC gates, the navigation allowlist's consistency,
new-window routing for internal/action/non-web cases,
history basics, stale-event dropping, close-tab behavior,
drag-drop containment, layout math, `dpi.rs` soundness,
the submit_route matrix.


---

# Pico v27 — Audit Addendum (2026-11-01): THEMES, CLUSTERS, PORTABLE DATA

The user's order: a full recheck and remaster, a full
rearrangement of the buttons, all data kept in the same
folder as the exe ("so you can delete everything with
ease"), his nine uploaded palette cards (@uiuxaadi) shipped
as switchable themes — the first card (Forest) as the new
default — with a glow layer "for futuristic and modern
look". Method as every round: verify-before-fix, root
causes, every v26 behavior (D1–D7) preserved, Shield lists
byte-identical.

## Workstream A — the theme system (AUDIT V27-A)

**One derivation, one source.** `core::themes` holds the
nine palettes exactly as read from the user's cards (hexes
pinned verbatim by test) and ONE derivation,
`derive(palette, dark) -> 22 tokens`. Role mapping, applied
uniformly: slot1 deep base, slot2 raised, slot3
mid/accent-strong, slot4 accent-light, slot5 pale/text,
slot6 resolved per palette by a documented override —
`Slot6::Accent2` (ember/sage/lagoon/ultraviolet/indigo/
graphite) feeds accent-2 and the danger hue family;
`Slot6::Slate` (forest/violet/bronze) is a well/slate tint
and danger falls back to a standing red stepped for AA.
Dark: surface=slot1, text=slot5, accent=slot4. Light:
surface=slot5 lifted toward white, text=slot1,
accent=slot3. Contrast is ENFORCED, not hoped for:
`ensure_contrast` steps a color's lightness within its own
hue until the pairing clears its floor — text/muted/faint/
accent/danger/accent-text hold AA >= 4.5 in all 18
derivations (test-computed). The generated CSS blocks are
emitted into both documents from this one table at assembly
(`assets.rs::shared_css_full`); a test re-runs the
derivation and demands the shipped blocks equal it
token-for-token, and a generalized purity scan rejects any
hex literal in the shipped styles that is not a palette
slot, a derived value, or a named functional constant (QR
matrix, failure-strip red, two service-brand tints).

**Migration decision (deliberate, per the brief).** A
pre-v27 state has no `theme_id`; serde fills an EMPTY
sentinel, and on first v27 run the app resolves it to
FOREST — the user's "use this as the new theme" wins over
preserving his violet look, and Violet stays one picker
click away. Fresh installs default to Forest via
`PersistState::default`. The resolution is logged
(`theme: pre-v27 state migrated to the forest palette ...`)
and persisted through the same single save path as every
other preference — no second writer (D-check below).

**Picker.** A palette button in the chrome opens a centered
card on Teleport's mechanics (same overlay rect growth,
backdrop, dismissal laws): nine cards, six palette dots +
name each, click applies instantly — persist, window/tab
backgrounds repainted from `themes::surface_rgba`, start
tabs pushed the two-argument `__picoSetTheme(mode,
palette)`, chrome re-tokened by the next snapshot. The
dark/light toggle now switches MODE within the palette.
Panels are mutually exclusive: opening one retires the
other, in the app and in the page (pinned both sides).
Icon: regenerated in Forest (deep-green tile, pale P) via
the same SDF generator + llvm-rc route; gate v2 re-run
PASS. The exe icon cannot follow the picker at runtime —
README says so plainly.

**Glow (V27-A3).** `--glow`/`--glow-soft` derive from each
theme's own accent (alpha .38 dark / .22 light). The
approved sites are pinned by a census test (exactly 11
`var(--glow)` references across both pages' local styles):
address bar focused, active tab's top edge, brand mark,
icon buttons on hover, shield/lite/teleport/palette in
their ON/expanded states, picker cards (hover + current),
start cards on hover. Resting surfaces never glow. The v20
shadow-vocabulary test was amended consciously: glow tokens
are the second sanctioned shadow vocabulary.

**Violet fidelity.** The derivation recovers v26's violet
dark EXACTLY (surface/text/accent pinned). Two conscious
exceptions, both from the uniform mapping winning over
v26's hand exceptions: light surface is the derived slot5
lift (#D9D4EB, was the raw slot5), light text is slot1
(#0E0B1D, was slot2). Both re-pinned with reasons in
`v26_palette_materials_and_accents`.

## Workstream B — button rearrangement (AUDIT V27-B)

Row 1 untouched. Row 2 is now three clusters separated by
neumorphic groove slits: [back fwd reload home openfile] ·
address bar · [shield+counters] [lite+counters] ·
[teleport] [palette] [mode]. Every control and IPC verb
from v26 survives; geometry is unchanged (86 DIP total,
feather contract intact). At <=900px the counter badges
collapse to icon-only; at <=820px the brand byline yields —
render-pinned at 760. DOM order == visual order == focus
order, asserted by a navbar census in BOTH the Rust
contracts and the DOM suite.

## Workstream C — portable data (AUDIT V27-C)

All runtime data lives in `<exe dir>/data/`: state.json,
pico.log (+rotation), WebView2 profile. `core::state`
resolves ONCE from probed facts — pure
`resolve_data_dir(exe_dir, appdata_dir, writable)` over a
create+delete probe — caches the choice, and everything
(state, log, profile) derives from it. Startup logs
`data: portable <path>` or `data: fallback appdata
(<reason>) <path>`; a not-writable exe folder (including
`data` existing as a file) falls back to %APPDATA%\Pico
and Pico never runs data-less. First v27 run migrates: if
the portable dir has no state and appdata holds state
and/or a profile, state.json + the profile tree are COPIED
(profile before any WebView2 context exists; per-file
failures counted, never fatal; original left untouched as
backup) and logged with counts. Fixture-tested: full copy,
missing pieces, no-clobber, idempotent second run,
root/unicode/UNC exe paths. SECURITY.md carries the plain
consequence: the profile — cookies, logins — now travels
with the folder.

## Workstream D — recheck findings (V27-1..)

- **V27-1 (real, fixed): start tabs kept a stale palette on
  disk-backed renders.** `SharedData` mirrored theme mode
  but not the palette, so a start tab rendered after a
  palette switch (but before any push) opened on the old
  palette. Fixed: `SharedData.palette` synced in
  `sync_shared`, init JSON carries it.
- **V27-2 (real, fixed): the app hand-derived the appdata
  profile path** in the new migration block
  (`appdata.join("WebView2")`) — the media source-scan's
  "no second derivation" pin caught it. Moved into
  `core::state::appdata_webview_dir`.
- **V27-3 (real, fixed): picker cards carried no stable id**
  in the DOM (the click used a closure). Cards now carry
  `data-id`, making the picker contract testable end-to-end.
- **V27-4 (checked, disproven): theme-switch race.** Palette
  changes force a snapshot (`push_state(true)`), snapshots
  read the palette from the single persisted field on the
  UI thread, and start pushes interpolate only closed-set
  values — no stale-theme application path exists.
- **V27-5 (checked, disproven): layout witness regression.**
  The panel bool now reads `any_panel_open()` in BOTH
  `chrome_rect()` and the witness block, and the witness
  names the panel (`teleport`/`palette`/`closed`) — the
  content rect is invariant under either panel, same as D2.
- **V27-6 (checked, disproven): migration vs first-run
  focus.** Migration completes in `run()` before
  `run_with_server` loads state or creates any tab; the
  focus flag path is untouched.
- **V27-7 (checked): state write atomicity.** `theme_id`
  rides the existing save path (same writer, same file);
  sanitize maps unknown ids to Forest and passes the empty
  pre-v27 sentinel through for the app to resolve.
- **Census:** shipped code still has zero
  unwrap/expect/panic (all hits are inside `#[cfg(test)]`);
  unsafe grep lines 19 — identical to v26, zero added
  (themes/state/ipc are pure safe Rust).

## Conscious re-pins (tests changed, with reasons)

- `theme_block()` reads generated blocks from the ASSEMBLED
  page (the tokens no longer live in raw style.css).
- `both_themes_define_the_v20_token_set` generalized to all
  9 palettes x 2 modes; the --neu-* geometry tokens are now
  asserted in the shared :root where they live.
- `address_focus_is_a_shadow_only_glow`: the focus layer
  gains `var(--glow)` (approved site), focus-glow alpha
  bound now checked across all 18 blocks.
- `every_box_shadow_references_a_neu_token`: glow tokens
  admitted as the second sanctioned vocabulary.
- `v26_palette_materials_and_accents`: see Violet fidelity.
- Version strings v26 -> v27 in asset + DOM suites; the
  teleport backdrop regex accepts v27's shared two-panel
  rule; DOM fixtures carry palette/themes/paletteOpen.

## Totals

Rust tests 277 (v26: 255), DOM checks 166 (v26: 149).
Clippy 0 (host all-targets + windows target), fmt clean.
Shield: filter lists diff-identical to v26 AND the compiled
shield fixture is byte-identical to v26's dump (518,002 B
file). Gate v2 PASS on the build exe and the zip exe.
Exe 1,826,304 B (< 5 MB), md5 in BUILD_NOTES.

# Pico v26 — Audit Addendum (2026-11-01): SUPER REMASTER (defects D1–D8)

The user ran v25 on an ultrawide (~2048px) and reported five
defects with screenshots, then added three scope orders
mid-build: a full Shield×Lite revisit/rewrite (D6), a
navigation-state rework (D7, which re-frames D1), and a new
UI palette (D8). Method, as every round: forensics first —
each root cause below was established in the code before any
fix, and audit claims the code did not confirm were not
"fixed" (see the disproven list). Everything not implicated
carries over: Shield lists byte-identical (diff-verified,
FILTERS.md included), QR encoder, icon route + gate v2,
recents, Google search, the whole security register.

## D7 — the address bar was frozen (root cause of D1)

**Evidence.** Through v25, no `SourceChanged` registration
existed anywhere in the tree. `Tab.url` was written by exactly
two paths: `on_page_load` (full commits only) and
`on_nav_note` (the injected observer script). SPA navigations
(pushState/replaceState) surface in WebView2 as
SourceChanged/HistoryChanged, not as load events — so on
YouTube the bar kept whatever the last full commit said
(screenshot: a video playing while the bar read the channel
URL).

**The wedge.** `on_nav_note` dropped any note whose origin
differed from the *stored* URL (the `same_origin` gate), while
the observer script's internal cursor advanced regardless.
One host-form flip (YouTube replaceState between
`www.youtube.com` and `youtube.com`) made every later note
"cross-origin" against the frozen stored URL — tracking
wedged permanently until the next full commit.

**Fix.** (1) `core::urlstate`: a pure URL state machine per
tab. Commits, SourceChanged values and observer notes feed
it in arrival order; Source is never origin-gated (it is the
platform's own value) and settles the document, after which
disagreeing notes are refused as stale; pre-Source notes keep
the v25 origin gate (a page must never spoof another origin
into the bar). History effects are classified by the machine
(a pending back/forward target classifies a commit as a
traversal rather than a fresh entry). (2) `attach_shield`'s
registration block now also wires `SourceChanged` (reads the
sender's live `Source`) and `HistoryChanged` for every
content webview (one new `unsafe` block: the Source read;
the `add_*` calls merged into the existing registration
block). (3) A single writer, `App::apply_url_event`, mirrors
the machine's live value into `Tab.url`, records the history
effect, re-keys the Shield's page state and the live-reload
root scope to the LIVE URL (D7-4: the media probe and party
facts follow it too), and retires a stale Teleport panel when
the active tab navigates. **Invariant:** the address bar, the
tab record and Teleport all read this one value; nothing else
stores a URL. Pinned by unit tests (including the exact wedge
scenario) and by `teleport_after_spa_navigation_uses_the_
live_url` (commit → host-flip source-change → video
source-change → Teleport yields the video URL byte-exact).

**Siblings (D7-4).** Title: wry's document-title-changed
handler was already wired by design (his tab title updated
because of it, not by luck) — unchanged. Nav buttons:
HistoryChanged triggers a snapshot re-push; the buttons are
computed from Pico's own history machine, which stays the
authority because Pico initiates traversals itself and the
reducer classifies their landings — WebView2's CanGoBack
would double-count SPA entries Pico never recorded.

**Editing guard (D7-2).** The chrome tracks address-bar
dirtiness (`input` event). A snapshot updates the input
unless it is focused AND dirty; blur/submit/Escape re-sync
it. DOM-pinned both ways (dirty draft survives a navigation
snapshot; clean focused input follows it; blur re-syncs).

## D1 — Teleport showed the wrong URL

**Verdict: not a transport bug — D7's frozen store.** Every
transport hop (`teleport::resolve`, the snapshot JSON, the
panel renderer) was read end to end and is byte-exact for
public URLs; a new identity matrix proves it (www hosts,
mixed-case paths, percent-encoding, @handles, query+fragment,
ports — the payload equals the input string everywhere, and
the LAN rewrite substitutes only the host token). The panel
showed `youtube.com/@iK4w4` because that is what the tab had
*stored* (an early redirect hop), while the bar showed a
different stale commit. With D7 both read the one live value.
Panel field hardening (the field also mis-showed by
design): full-width in the 400px card, `title` carries the
full string, click selects all, scroll pinned HOME after
render (an end-scrolled overflow is forbidden), 12px font.

## D2 — the black band across the page

**Root cause (confirmed in the chrome document).**
`#feather` was `position: fixed; bottom: 0` and `#errbar`
`bottom: 16px` — both anchored to the chrome *viewport's*
bottom. v25 grows the chrome HWND by the panel height while
Teleport is open (his screenshot has the panel open), so the
viewport grew and both strips re-pinned ~424 DIP too low:
the feather's dark shade (`--feather-shade`, 0.30 black in
the dark theme) plus its surface melt painted a thick dark
band across the page area.

**Disproven alternates (proven, not shrugged).** (a) The
layout math: `core::layout::tile()` never saw panel state,
and v26 adds the invariant test — content rect identical
with the panel open vs closed, at 760×500 / 1280×800 /
2048×1152 / 2560×1080 / 3440×1440 × DSF {1, 1.25, 1.5, 2},
always exactly window-minus-header (content y=74). (b)
`TeleportClose` did call `layout()` in v25 (read in code).
(c) `phys_to_rect` is a pure physical mapping.

**Fix.** Both strips are anchored by `top` now (feather at
rows 74–86 by contract; errbar at 92) — true at every chrome
height, panel or not. **Witness:** `layout()` logs the
applied geometry (window size, scale, chrome rect, content
rect, panel state) to pico.log whenever it changes, so a
recurrence is diagnosable from the log alone. Remaining
suspects, enumerated for that case: the content HWND's own
paint surface (WebView2), the window background (now the
palette surface — `theme_bg_rgba` returns #0E0B1D / #C9C2E3,
so an unpainted pixel can no longer be black in light theme
or grey in dark), and a DSF-change resize path.

## D3 — Teleport panel in the far corner

Redesigned as a **centered card**: 400 DIP wide,
`left: 50%` + `translateX(-50%)`, hanging just under the
address row, QR at **264 DIP** (was 184, in a 264-wide card
at the window's right edge). A `--backdrop` tint dims the
chrome area around it — deliberately only the chrome area:
the content webview is a separate HWND the chrome document
cannot paint over, and no full-window dim is faked. The
grown chrome region is 496 DIP (`TELEPORT_PANEL_H`), keeping
content geometry untouched (D2 law). Dismissal: Esc, ×,
backdrop click, phone-button re-click, tab switch
(pre-existing), and now navigation too (the app retires the
panel when the active tab's URL moves). DOM pins: centering
declarations, QR size floor, backdrop presence + click,
TooLong error state (no QR, URL stays copyable, note
explains — never a blank panel).

## D4 — new tab must focus the address bar

Flow traced: `Command::NewTab` focused the chrome HWND but
nothing focused the input; the chrome focused it only at
page boot. **Fix:** a one-shot `focus_addr` snapshot flag.
Set on: NewTab command, startup, and the Ready handshake
when the active tab is a start tab (the last makes focus
survive a watchdog chrome recreation — the hint rides the
very snapshot Ready forces). `push_state` consumes it
exactly once and force-pushes when set (a signature-gated
push would strand it). The chrome focuses + selects on the
flag (the existing focus handler selects). **Decision:**
tab *switches* do not yank focus (the snapshot carries no
hint for them) — browser-like and DOM-pinned (focus stays on
the control the user was using).

## D5 — searching on a start tab spawned another tab

**Root cause (verbatim).** `navigate_active`: active-is-start
→ `add_tab`. **Fix:** routing is core's pure
`submit_route(active_is_start, target)` — start tab + URL →
`ConvertStart`, start tab + file → `ConvertStartWithFile`,
content tab keeps the v12 split (`LoadCurrent` /
`OpenFileInNewTab`). Conversion is total and immediate:
`is_start` flips, history and URL state restart at the
target, the same webview loads it — including file targets,
via the static server with the same root-ownership fixup
`open_path` applies. Two enablers matter: (1) the navigation
handlers classify "start tab" from a **live set**
(`Arc<Mutex<HashSet>>` maintained by the app) instead of the
build-time capture that made conversion impossible — a
converted tab's handler stops treating it as start in the
same instant; (2) the action dispatch's `Requester` is built
from the tab's live fields, so the privileged gate closes
for a converted tab at the first conjunct — pinned by
`v26_converted_start_tab_loses_the_gate`. Start-PAGE
launcher actions (cards, recents, socials) never route
through `submit_route`; they keep open-in-new-tab by design.

## D6 — Shield × Lite: full revisit + unified pipeline

**Rewrite.** `core::filterpipe::decide` is the one decision:
facts (URL, page URL, type, method) + a `FilterState`
snapshot + the engine → `Allow | ShieldBlock | LiteBlock`.
The WebResourceRequested handler builds facts once (page
state read once — v25 read it up to three times per request,
in two layers), snapshots both toggle atomics once (the
toggle-race fix (f): a verdict always describes one real
instant), and counts exactly on the verdict's owner — a
double count is impossible by construction. ABP semantics
are untouched (same lists, same engine); the v23 jsdom
innertube suite passes unchanged, plus new interplay cases.

**Interplay audit.** (a) *Ghost boxes — CONFIRMED.* A
neutered image returns a 1×1 GIF or an empty body; the
element keeps its CSS box, and nothing anywhere collapsed
it (no error listener existed in the tree). Fix:
`COLLAPSE_RUNTIME`, a document layer (capture-phase error
listener + load sweep for `complete && naturalWidth === 0`)
that hides the img/picture with a `data-pico-collapsed`
marker; it rides as the document script's third segment and
is pushed per-load on Lite-only pages (v25's injection gap —
item (d) — where no Shield script existed). Its arming flag
`__picoCollapseOff` is maintained on every load: 1 only when
BOTH filters are off (OFF stays byte-identical passthrough,
DOM-pinned in both dimensions). (b) *Cosmetic coverage —
MEASURED.* The full specific set is 1,152 hosts / 2,128
rules = 326,923 B serialized; the 300,000 budget shipped 578
hosts (volume-order starvation — long-tail sites got
generic-only). Budget is now **340,000** (all hosts + ~13 KB
drift headroom); a pin test fails loudly if coverage ever
drops (`v26_cosmetic_payload_covers_every_compiled_host`).
A compact index encoding (~73 KB measured possible) was
rejected as unnecessary risk at this size. Document script:
487,345 → 515,479 B (coverage delta + the ~1.2 KB collapse
runtime); scriptlets unchanged (2,913 entries, all hosts —
they were never the problem). (c) *Counters:* v25 had no
double-count as written, but attribution was by convention
across two runtimes; the single verdict makes it structural.
(e) *Classifier disagreement — CONFIRMED.* Lite treated all
of 127/8 as local; the Shield named only 127.0.0.1 (a
127.0.0.2 page was public to one, local to the other); the
JS runtimes spelled a third variant. One classifier now
(`filterpipe::classify_origin`: 127/8 + localhost +
*.localhost + ::1 → Local), and both JS runtimes' guards
were widened to spell it identically. Hostless/data: URLs
classify Local (fail-open — a documented, deliberate change:
an opaque-origin page now gets no filtering instead of
inconsistent filtering).

**Caught by the new tests (V26-S1).** The first pipeline cut
lost the v22 GET-only law — it lived in
`ShieldRuntime::should_block`'s composition, not the
engine. The origin-dimension matrix test failed on a POST
case; the gate now lives in `decide` (Shield step skipped
for non-GET; unnamed methods proceed, exactly as before)
and the test pins it.

**General revisit (V26-S).** Compile: budgets had three
definitions (app consts, example literals, engine defaults)
— now one (`core::shield::{COSMETIC,SCRIPTLET}_SCRIPT_
BUDGET`), consumed by app, dump and tests. Chip: snapshot
carries both counters from their single runtimes; no
collision found (searched; the v25 collision suspicion is
DISPROVEN as written, hardened by construction anyway).
Disproven also: scriptlet payload truncation (never
truncated: 186,717 B < 220,000 budget), teleport-request
race (payloads are computed from the active tab at request
time and retired on tab switch/navigation), panel state
after theme toggle (the panel is theme-agnostic; the toggle
re-pushes the snapshot and the class state persists),
ultrawide + 200% DSF crowding (render-verified at 2048 and
DSF 1.25/2 geometry in the layout invariants).

## D8 — the violet palette (user-supplied)

All six hexes are the user's: `#0E0B1D` `#2A1E4D` `#5A3B8C`
`#8B7CC7` `#C9C2E3` `#3A3A52`. Dark is built on the
violet-black **kept pure** (no lift needed: the lavender
light-shadow at rgba(139,124,199,0.15) over
rgba(2,1,6,0.60) separates on it — render-verified in two
passes at 1280/2048, DSF 1/1.25). Light is the mirror on
`#C9C2E3`. Neumorphism laws unchanged (one material,
`--bg == --surface`, dual shadows only, no border-drawn
controls). `--danger` became per-theme (one red cannot hold
AA on both surfaces). Contrast (WCAG, computed):

| pair | dark | light |
|---|---|---|
| text on surface | 11.35 (#C9C2E3 on #0E0B1D) | 8.85 (#2A1E4D on #C9C2E3) |
| muted on surface | 7.08 | 5.05 |
| faint on surface | 5.35 | 6.45 (slate — quieter by saturation) |
| accent on surface | 5.35 (#8B7CC7) | 5.05 (#5A3B8C) |
| text on accent fill | 5.35 | 5.05 |
| ok / warn on surface | 9.9 / 8.0 | 4.74 / 5.14 (same hues, one step darker) |

Everything token-driven followed: chrome, start page,
teleport card, chips, warn strip, focus ring/glow (re-hued
to lavender/indigo, glow alphas inside the pinned ≤0.18
bound), feather shade, spinner. The `#shellwarn` strip keeps
its self-contained material red by design (it must render
when every token is dead). Brand hovers (Telegram/YouTube)
keep brand colors, as before. A purity test fails if the
old hexes (#212429, #e6e9ef, #2e5f8a, #93b8e6, #dde3ec)
reappear in shipped styles. **Icon re-tinted** via
`gen_icon.py`: tile gradient #5A3B8C → #2A1E4D, glyph
#C9C2E3; `.res` rebuilt with llvm-rc (icon-only, zero
manifest — gate v2 re-run PASS); 16px legibility
render-checked.

## Verification totals

Rust 255 tests (v25: 233) + DOM 149 checks (v25: 125), all
passing; clippy 0 on host and Windows targets; fmt clean;
zero unwrap/expect/panic in shipped source; `unsafe` blocks
16 (v25: 15; +1 is the SourceChanged handler body D7
mandates, its registrations merged into the existing
block). Gate v2 PASS on the built exe and on the
zip-extracted exe. Shield lists byte-identical; compile
report differs from v25 ONLY by the cosmetic coverage and
the collapse segment, as recorded in D6.

# Pico v25 — Audit Addendum (2026-11-01): Phone Teleport + Lite Mode

The user picked two ideas from a deliberately outside-the-box
round — "1-2 is super good" — and demanded research first,
maximum precision, and full testing/debugging. The research is
`RESEARCH_v25.md`, written BEFORE any feature code, and it
governs what follows. Everything else carries over from v24:
filter lists byte-identical (diff-verified, FILTERS.md
included), Shield compile report identical (document script
487,345 bytes, 2,913 scriptlet entries), the icon + gate v2,
recents management, the start page, Google search, and every
security law.

## V25-1 — QR encoding: pure Rust, golden-vector pinned

No QR crate was added (none was needed, and the dependency tree
is a promise). `core::qr` implements byte mode / EC level M /
versions 1–12 from the specification's construction: GF(256)
Reed–Solomon (0x11D, α=2), the standard block-split and
interleave arithmetic (unequal groups included), format and
version information COMPUTED through their BCH codes rather
than copied from tables, and mask selection by the
specification's four penalty rules (N1=3, N2=3, N3=40, N4=10,
finder-pattern rule with the virtual quiet zone).

A QR that scans wrong is worse than none, so the encoder's
contract is a set of **golden matrices generated by an
independent implementation** — Nayuki's qrcodegen, fetched for
this purpose, driven in forced single-byte-segment mode at
Ecc.MEDIUM with error-boosting disabled
(`scripts/research/gen_qr_vectors.py` → `tests/qr_vectors.inc`).
Five vectors (v1 short text; v2 https URL; v2 localhost:port;
v7 percent-encoded query, four blocks + version info; v11
~200-byte URL, 16-bit count field + five blocks) are pinned
module-for-module: size, version, chosen mask, every module.
They pass. Capacity is honest: past 287 bytes the encoder
returns `TooLong` and the panel says so — the URL stays
copyable as text; a truncated QR is never drawn.

## V25-2 — Teleport resolution (pure) and the LAN address

`core::teleport::resolve` is the whole decision, unit-tested
across a 15-case matrix: public http(s) passes through
unchanged; loopback hosts (localhost, *.localhost, all of
127/8, [::1], 0.0.0.0) are rewritten onto the chosen LAN IPv4
with port/path/query/fragment preserved and userinfo dropped
(a dev URL's credentials are not baked into an on-screen QR);
loopback with no LAN candidate yields NoNetwork (original URL
shown, honest note); Pico-internal pages, Pico-served files,
file:// and non-web schemes are NotTeleportable with a
one-line reason.

The LAN address comes from `GetAdaptersAddresses` (iphlpapi,
already linked; one new windows-sys FEATURE flag — Ndis, whose
types gate the API in windows-sys 0.59 — no new crate). Only
address + OperStatus are read; names/descriptions are never
touched, and nothing is logged but the outcome kind and a
redacted host. `choose_lan_ip` is pure and headlessly tested:
loopback/unspecified/link-local filtered; RFC1918 tiers
192.168/16 > 10/8 > 172.16/12 > other; adapter-Up wins inside a
tier; enumeration order breaks remaining ties. A UDP-connect
egress probe (no packet is sent) is the fallback when
enumeration yields nothing. +1 unsafe block total (16), the
adapter-list walk, SAFETY-commented: two-call sizing, buffer
outlives the walk, every pointer null-checked.

**Why Teleport needs no start-tab gate.** The gate guards
*start-page actions* because the start page is web content's
neighbour — a compromised or confused page must not speak
privileged commands. Teleport is a **chrome command**:
`{t:"teleport"}` travels the same typed IPC channel as
`new_tab` / `shield_toggle`, which only the chrome document —
Pico's own UI, loaded from the embedded asset, CSP-locked —
can originate with a valid (boot, req) envelope. Content
webviews have no path to the chrome command parser at all.
The resolution it triggers is read-only (enumerate + encode);
its only output is a snapshot the chrome renders.

**The geometry compromise, stated.** The chrome WebView is 86
DIP tall; a QR panel cannot paint below its HWND. So while the
panel is open the chrome rect grows downward by
`TELEPORT_PANEL_H` (424 DIP) via `core::layout::chrome_rect_with_panel`
— an overlay in the feather's own spirit: the content rect
never moves, the 86-DIP geometry and feather are untouched
when closed, and the added region is transparent except the
panel card. The cost mirrors the feather's known trade-off,
larger: while the panel is open, the page's top band sits
under the chrome HWND and does not take clicks. Esc / × /
re-click / tab-switch all dismiss it, and dismissal restores
the plain rect (`teleport_close`). Panel content is rendered
from the snapshot payload only; the matrix (rows of "1"/"0")
becomes one SVG path built from character checks — data, never
markup — and the URL/note reach the DOM as input value /
textContent.

## V25-3 — Lite Mode: a class filter with local immunity

`core::lite::decide(lite_on, class, page_is_local,
request_is_local)` — pure, matrix-tested over every
`ResourceType`. Block only when Lite is on AND the class is
Image/Font/Media AND the page is not local AND the request is
not local. Classification is the Shield's own
(`resource_type_from_context`: WebView2 contexts 3/4/5, per
Microsoft's enum — RESEARCH §d), so there is no second mapping
to drift.

**Local immunity is a hard law.** Any loopback or Pico-internal
page renders completely untouched, and any request to a local
origin passes even from a public page — the user's dev work is
never degraded by a data-saving feature. Conversely there is
**no per-host special-casing**: Lite does not know YouTube
exists. Video will not play while Lite is on; that is stated
in the toggle's tooltip, in the panel copy of the README, and
here, because predictability is the feature.

**Ordering vs the Shield.** Lite is consulted in the existing
`WebResourceRequested` handler only when `should_block`
returned None. A Shield block is final: delivered, counted by
the Shield, logged as `shield block:` — Lite never sees it and
cannot double-count it. Lite blocks take the identical neuter
path (type-appropriate empty body / transparent GIF), are
recorded only after the response is set (the V22-9 rule), emit
`lite block:` sample lines (same 20-line cap), and count into
the Lite chip only. State gains `lite_on` (serde-default
false — Lite is opt-in, and verbatim v24/v3 files load with it
off; compat test pinned). The flip is a per-request flag read,
so it takes effect immediately in every tab; already-loaded
resources are not retroactively removed (a reload shows the
full effect).

## V25-4 — What was NOT built

- The static server was NOT rebound or proxied for Teleport:
  Pico-served files stay loopback-only, full stop. Teleporting
  a dev server works because the *user's* server can bind the
  LAN (`--host`); Pico's own server keeps its v12-era security
  posture, and the panel note says exactly that.
- No QR *decoding*, no camera, no phone-side app, no network
  transport of any kind: the QR is a picture on the screen.
- No Lite scheduling, per-site Lite lists, or class pickers —
  closed scope: one toggle, three classes, local immunity.

# Pico v24 — Audit Addendum (2026-11-01): icon, recents management, start-page reorganization, address-bar search

The user's verdict on v23 was "pico 23 is god" — the Shield
works. v24 therefore changes exactly four ordered things and
carries everything else over untouched in behavior: the filter
lists are byte-identical (diff-verified against the v23 tree,
FILTERS.md included), the Shield compile report is unchanged
(document script 487,345 bytes, 2,913 scriptlet entries), and
every v23 security control and Shield law stands. The four
changes: a real icon at last, a manageable Recents list, a
reorganized start page with a light modern touch, and Google
search in the address bar.

## V24-1 — The icon (and gate v2)

**The problem.** Pico's taskbar button showed no mark: with no
embedded icon resource and no runtime window icon, Windows had
nothing to draw.

**The governing forensics** (from the v11 SxS case, established
by PE dumps at the time): activation / side-by-side failures
come from the MANIFEST (RT_MANIFEST). v11's suspect was a
HAND-BUILT .rsrc container. The user-proven C# v7 binary
carried a FULL .rsrc (icon + version info) produced by a real
resource compiler and launched fine. Icons are inert data; no
activation path reads them.

**Route (a) — runtime window icon, zero resource risk.**
`scripts/gen_icon.py` (stdlib-only, signed-distance primitives,
deterministic) draws the Pico mark — a steel-blue rounded-square
tile with a bold white P (the waist-bar construction; pass 1's
bare clipped ring read as a question mark and was redrawn) — and
emits `assets/icon_64.rgba`. The app hands it to tao at window
creation (`Icon::from_rgba` + `with_window_icon`); pico.log
gains the witness line `window icon: set (runtime)`. A rejected
asset degrades to the platform default icon plus an error line,
never a failed launch.

**Route (b) — exe icon via a REAL resource compiler.**
The same generator emits `assets/pico.ico` (16/20/24/32/48/64/
128/256, every frame PNG-compressed). `assets/pico.rc` contains
exactly one statement — `1 ICON "pico.ico"` — no manifest, no
version info, nothing else. llvm-rc compiled it to the checked-in
`assets/pico.res`, which the build links via a rustflags
link-arg (no build.rs, no build-time toolchain dependency).
The shipped exe's resource directory contains exactly
{RT_ICON ×8, RT_GROUP_ICON ×1} — verified by the gate on both
the build output and the zip-extracted exe.

**Gate v2.** `scripts/check_no_manifest.py` is rewritten: the
old "resource directory must be empty" assertion is RETIRED in
favor of the precise law it was always proxying — **zero
RT_MANIFEST entries** — plus per-line verdicts for the icon
(expected with `--expect-icon`), static CRT (0 CRT imports)
and subsystem (WINDOWS_GUI). Each verdict prints separately;
the manifest verdict is labeled the hard law.

## V24-2 — Recents management

Through the EXISTING single action dispatch — new action kinds,
same conjunctive gate, same one-log-line-per-decision law:

- `recent-remove` (IPC `{action:"recent-remove", index}` /
  `pico://recent-remove/<i>`): each recent row grows a small ×
  anchor (visible on hover/focus-within, neumorphic-sm) that
  removes that entry. Range-checked in dispatch against the
  same list snapshot the page rendered; out-of-range is a
  deny-logged `RecentOutOfRange`, never a clamp or a panic.
- `recents-clear` ("Clear all" in the section header, next to
  the section's count): empties the list.
- `recents-visibility` (`hidden: bool` / `…/hide|show`):
  persists section visibility. Hidden collapses the section to
  the single muted header line "RECENT FILES — hidden" + Show.

State: `recents_hidden: bool` (default false) rides schema v3
via the serde default — a verbatim v21–v23 state file loads
with the section visible; no version bump, no migration (the
structural forward rule doing its job). Rust keeps the list as
the single source: mutations save state, then the new list /
flag is PUSHED to live start tabs through the existing
`__picoSetRecents` hook plus a new `__picoSetRecentsHidden`;
fresh page loads render from the shared snapshot, which now
carries `recentsHidden` in INIT. The three new internal hosts
joined `normalize_pico_url`'s explicit internal set (AUDIT
V15-2 discipline), so the folded `https://pico.…` spellings of
the new action URLs reach the same dispatch. Tests: dispatch
grid for both transports + gate + range/malformed (3 new test
fns), state compat (2 new), the full DOM seam (13 new checks).

## V24-3 — Start-page reorganization + modern touch

Placement, top to bottom: (1) compact hero (smaller ring and
wordmark, tagline; the byline and the drop hint LEFT the hero);
(2) RUNNING SERVICES directly under it, cards + the aggregate
"other listeners" line; (3) RECENT FILES with the V24-2 controls
in its header — the pick-file button and the drop hint sit
attached under the list, since they are the file-opening
cluster; (4) the footer zone: shortcuts line, the byline row
(Telegram chip · by @iK4w4 · YouTube chip), and the version line
"Pico v24 · by @iK4w4". Branding is demoted, not removed.

Modern touch (a light hand, v20 tokens as the base): the accent
desaturates toward steel blue in both themes (light #2e5f8a,
5.5:1 on the material; dark #93b8e6, 7.7:1 — the AA claims in
style.css are recomputed, not inherited); the --neu-* blur radii
soften (+2–4px, offsets — the geometry — unchanged); --r-lg
12→14 and --r-xl 16→18 (+2 on cards/inputs: the radius contract
test was consciously re-pinned); section headers become one
component (letter-spaced small-caps title, meta, hairline,
tools); hover/press transitions sit in the 120–160ms band on
buttons, cards, rows and chrome controls. Chrome received ONLY
what flows through the shared tokens plus its transitions —
**no chrome layout or geometry change** (86/40+46/74/12 stands;
every v19/v20 alignment contract passes unmodified except the
two radius pins above). Render-verified in headless Chromium:
chrome + start, both themes, DSF 1 and 1.25, feather blends,
plus the new hidden-recents state (visual-check/, 2 passes).

## V24-4 — Address-bar Google search

`core::addr::parse_input` gains a search TAIL — nothing before
it moved. Full URLs, localhost/host:port forms, bare ports,
drive paths and existing files resolve exactly as in v23. What
falls through resolves as:

1. Backslash-bearing remnants: refused (unchanged).
2. Anything with whitespace: a Google search — EXCEPT the two
   scheme lines that still refuse first: a scheme-shaped token
   before the colon ("recipe: …" is a scheme claim, refused
   even with prose after it), and an obfuscated scheme whose
   pre-colon prefix has whitespace but whose payload is one
   token ("java script:alert(1)"). A multi-word prefix with
   whitespace after the colon too ("how to: cook rice") is
   prose and searches.
3. A plausible bare host — ≥2 well-formed DNS labels with an
   alphabetic TLD of ≥2 chars, or a bare IPv4 — navigates via
   the legacy https tail ("example.com", "sub.example.co.uk/
   path", "192.168.1.1").
4. A file-name-shaped token (final segment ends in a known
   file extension: "LLM.html", "notes.txt", "PAGE.HTML")
   follows the FILE logic — the existing-file check first, the
   legacy https tail after — and is NEVER a search.
5. Everything else — single labels ("youtube", "github") and
   implausible hosts ("foo.123", "example.c", "foo/bar") —
   becomes `https://www.google.com/search?q=` + the typed
   text, RFC 3986 percent-encoded over its UTF-8 bytes
   (spaces %20; unicode verified).

Two deliberate test changes ride with this: the v23 junk list
PINNED "hello world"/"a b c" as rejected — they are prose, and
prose is a query now; they moved to the search matrix
(`address_bar_search_matrix`, 29 pinned cases, in
tests/addr_ipc.rs). The start-tab privileged scheme and typed
`pico://` handling are untouched.

## Verification summary

223 Rust tests (v23: 215) + 99 DOM checks (86) green; clippy 0
on host and on the Windows target; fmt clean; zero
unwrap/expect/panic in shipped src; unsafe 15 blocks, unchanged;
Shield compile identical (lists byte-identical, document script
487,345 bytes / 2,913 entries, decide p50 7.0 µs / p95 9.1 µs);
gate v2 PASS on the exe and on the zip-extracted exe; exe
1,737,728 bytes, md5 b5fa514702e404c2276e4353c5a1d48b. The exe
was built but never executed here (no Windows in this VM) —
taskbar icon visibility is an on-device verdict whose witnesses
are the pico.log line `window icon: set (runtime)`, the
Explorer/taskbar appearance, and the per-decision
`start action: … decision=remove-recent / clear-recents /
recents-hidden` lines.

---

# Pico v23 — Audit Addendum (2026-10-30): the Shield scriptlet rewrite

This addendum records the v23 Shield rewrite: the subsystem was
rebuilt so YouTube ads actually die. The user's evidence on v22
was a screenshot of a YouTube watch page: Shield ON, "11 blocked"
on the chip — and a pre-roll ad ("Sponsored • 1 of 2") playing in
the player. His order: "i don't feel any uBlock Origin, fully ads
on — fix it, rewrite it entirely, make sure it's on and working
properly, the hardest you can, analyse and scan all lines,
research more." The v22 addendum below remains the record for
everything else; where it and this addendum disagree about the
Shield, this addendum governs.

Method: Phase 0 mined the shipped filter lists
(`assets/filters/*.txt`, byte-identical to v21/v22 per
FILTERS.md) for every rule scoped to the YouTube family and for
the global scriptlet census, before any code was written; the
Shield engine, compiler, injector, and wiring were then read
line by line and rewritten; every v22 claim about the Shield was
re-verified on the v23 tree (215/215 Rust tests, 86/86 DOM
checks, no-manifest gate PASS on the shipped exe).

## Part 1 — Why the v22 Shield could not kill a YouTube ad (V23-1)

**Root cause (Verified, High).** YouTube in-stream ads are not
network resources. The ad inventory travels inside the player's
own innertube JSON — `adPlacements` / `playerAds` / `adSlots`
keys in `youtubei/v1/player` responses and in the watch page's
`ytInitialPlayerResponse` — and the ad segments then stream from
the same googlevideo hosts, with the same URL shapes, as the
content itself. Domain/type network rules structurally cannot
separate them, and cosmetic rules cannot reach inside a player.
uBlock Origin kills them with **scriptlets** (`##+js(...)`):
`json-prune` and its fetch/XHR variants delete the ad keys from
responses before the player reads them, `trusted-replace-*`
rules rewrite the raw response text, and `set` traps strip the
keys from the initial data at assignment time. That entire rule
family — 2,734 list lines — was the one v21/v22 counted as
*skipped-unsupported*. The "11 blocked" in the screenshot were
real (peripheral trackers/ads endpoints); the ad that mattered
was never a network decision. v23 implements the family.

## Part 2 — Findings

| ID | Severity | Where | Finding | v23 disposition |
|----|----------|-------|---------|-----------------|
| V23-1 | High | core/shield.rs (v21/v22) | The scriptlet rule family was skipped wholesale; YouTube's entire anti-ad arsenal lives in it (Part 1). | Fixed: scriptlet compiler + runtime (Part 3). 1,620 lines compile to 2,913 entries on 2,197 hosts. |
| V23-2 | High | core/shield.rs `DomainSet::parse` used for cosmetic scopes | Cosmetic/scriptlet scopes are comma-separated, but they were parsed with the pipe-separated `domain=` grammar, so `a.com,b.com##.ad` was filed under ONE pseudo-host `"a.com,b.com"` that no hostname can ever equal. 111 of the 938 "hosts" in the v22 payload were dead comma-joined keys; every rule on those lines applied nowhere. | Fixed: `DomainSet::parse_cosmetic`. Specific hosts 938 → 1,152, specific rules 1,777 → 2,128. Test: `multi_domain_cosmetic_scopes_reach_their_hosts`. |
| V23-3 | High | core/shield.rs page-flag extraction | Flag scope ignored the rule's domain anchor. The lists spell generichide as `@@\|\|host^$ghide` (401 of the 405 shipped flag lines); with no `domain=` option the engine filed each as GLOBAL. Once V22-7 made global flags reach the runtime, generic cosmetics were suppressed on **every site** (verified on the v23 tree pre-fix: the payload carried `"ghAll":true`). | Fixed: scope = `domain=` if present, else the anchor host. Shipped payload now has `ghAll:false` with the 401 hosts carried individually. Test: `anchored_ghide_scopes_to_its_anchor_not_the_world`. |
| V23-4 | Medium | core/shield.rs cosmetic compile | uBO procedural filters shipped under `##` — `##^script:has-text(...)` html filters and extended-pseudo selectors (`:has-text`, `:upward`, …; 159 specific selectors) — were emitted as CSS. They are not valid CSS, and CSS drops an ENTIRE comma-joined rule on the first invalid selector, so one such selector silently disabled all cosmetics (generic included) on its host. | Fixed: classified procedural at compile and skipped (never shipped as CSS). `skipped_procedural` 3 → 167. Test: `procedural_selectors_under_plain_marker_are_not_css`. The generic set itself was verified clean (0 extended pseudos in 13,676). |
| V23-5 | Medium | core/shield.rs option parser | The parser rejected the spellings the real lists use — `1p` (330 lines), `3p` (239), `xhr` (199), `doc` (223), `ghide` (405), `frame` (101), `from=` (18), `to=` (45), `method=` (8), `css`, `all`, `strict1p/3p` — skipping 1,960 rules, among them YouTube's `*_ad_$media,domain=youtube.com,3p`. | Fixed: aliases + `from=`/`to=`/`method=` implemented (`NetRule` gains method allow/deny lists and a request-host `to` set; `decide_with_method`; `attach_shield` reads the request method). Network rules 40,716 → 41,108, exceptions 192 → 349, page-flag rules 0 → 405. Tests: `option_spellings_the_lists_use`, `method_and_to_options_scope_decisions`, `youtube_network_catalogue_decisions`. |
| V23-6 | Low | core/shield.rs option parser | `$replace=` counted as generic unsupported; worse, replace values containing commas (8 of 25) broke the comma-split option grammar and could fall through to `NotOptions`, compiling the line as a never-matching pattern — a silent misapplication shape. | Fixed: `replace=` is intercepted before the comma split and counted in its own bucket (`skipped_replace`: 25). Rationale for not implementing: WebView2 exposes no response-body mutation for network requests (Chromium uBO lacks `$replace` for the same reason); YouTube's `$replace` rules are twinned by the trusted-replace scriptlets v23 does implement. |
| V23-7 | Info | boundary | `xml-prune` has ZERO rules in the shipped lists — not implemented, by absence of evidence. The json-edit family (5 lines, incl. m.youtube L49) uses recursive-descent paths (`..a.*[?.x]`) — a different path language; not implemented, counted as skipped scriptlets. | Documented boundaries (Part 5). |
| V23-8 | Info | core/shield.rs scriptlet tokenizer | 16 list lines carry unterminated strings (`acs, Math, '0x`), unterminated regexes (`/get_watch` as a final arg), or bare JSON-ish args (`"adBlockWallEnabled":true`). uBO's parser tolerates all three shapes. | The tokenizer decides token kind at arg start with termination lookahead and degrades unterminated tokens to bare ones, as uBO does; `skipped_invalid` is back to exactly v22's 8. Tests: `scriptlet_tokenizer_survives_youtube_regex_args`, `scriptlet_calls_parse_with_uvo_grammar`. |
| V23-9 | Info | src/app.rs toggle | OFF must disable all three layers with one flag. WebView2 cannot unregister a document-start script, so a view built while ON keeps its payload registered: live pages are disarmed instantly via `window.__picoShieldOff` (every wrapper consults it per action) + style removal; documents loaded later in such a view are disarmed at load-start and load-finish; views built while OFF get no payload at all. The network layer reads the atomic flag per request and is exact. Residual: between document-start and load-start of a post-toggle navigation, wrappers are installed but the page's own scripts have not run yet — no page request can be pruned in that window in practice, and none can be *blocked* (the network layer is off). | Implemented + documented here; the flag is also what the DOM suite's OFF control exercises. |
| V23-10 | Info | core/ipc.rs | The payload reports prune counts over `window.ipc` (`kind:"shield_prune"`). Any page can post that channel, so the messages are parsed under strict caps (≤16 items, path ≤256 B, count clamped) and feed ONLY capped sample log lines (`shield prune: host=… path=… n=…`, 20/session). They move no counter and gate nothing; a forged report can write at most 20 plausible log lines. | `parse_shield_prune` + tests (`shield_prune_*` in tests/addr_ipc.rs). The chip stays network-blocks-only (BUILD_NOTES states the choice). |

## Part 3 — The catalogue (Phase 0, from the shipped lists)

YouTube family, by rule family (host scope includes
subdomains/entities as written; file:line in ublock-filters.txt
unless noted):

- **Scriptlets — 14 distinct lines.** www.youtube.com:
  `trusted-replace-xhr-response` ×2 (L20: regex cutting the whole
  `adPlacements` span; L21: `…"adPlacements…("adSlots"|"adBreakHeartbeatParams")` → `$1`),
  `trusted-rpfr` ×2 (L22/23: `'"adPlacements"'`/`'"adSlots"'` → `'"no_ads"'`, URL cond `player?`),
  `set` ×4 shared with five other hosts (L35–38:
  `ytInitialPlayerResponse.playerAds/.adPlacements/.adSlots`,
  `playerResponse.adPlacements` → `undefined`),
  `json-prune-fetch-response` (L46, six hosts: reel `isAd` paths,
  cond `url:/reel_watch_sequence?`),
  `json-prune` (L47, six hosts: same reel path for JSON.parse),
  `no-xhr-if` (ublock-privacy L1176: the `/api/stats/atr` pattern
  with `method:POST`). tv.youtube.com: its own
  `trusted-replace-xhr-response` (L18). m.youtube.com additionally:
  `json-prune` (L40: the six playerResponse/adPlacements paths +
  `important`) and the family's one **json-edit** line (L49 —
  boundary, V23-7). youtube-nocookie.com / youtubekids.com /
  music.youtube.com share the `set`/`json-prune` lines.
- **Network — 20 distinct lines.** The six `$replace` player
  rules (L25–30 + L51–52, skipped_replace, twinned by the
  scriptlets above); `*_ad_$media,domain=youtube.com,3p` (L12 —
  skipped in v22, compiles now); the TVHTML5 googlevideo
  initplayback rule (L44); the rest are `removeparam` /
  `redirect=` / `urlskip` privacy rules (still skipped, by
  family) and one `$doc` shorts rule (compiles now; document
  rules remain law-1-gated).
- **Cosmetic:** one `##` selector (L42 — a `:has-text`
  procedural, reclassified per V23-4) and one `#@#` exception
  (EasyList: `youtube.com#@#.video-ads`).

Global scriptlet census (2,731 `##+js` lines, 3,373 host
entries): aopr 636, acs 346, set 314, aeld 201, nostif 193,
aopw 183, nowoif 177, href-sanitizer 69,
set-local-storage-item 56, nano-sib 51, remove-cookie 50,
nowebrtc 47, no-xhr-if 37, no-fetch-if 34, rmnt 33, nano-stb 31,
ra 30, nosiif 26, aost 24, json-prune 23, noeval 23, and a long
tail. **Supported dispatch set (13)** — every YouTube rule plus
the high-value general families: `set` (+`set-constant`),
`json-prune`, `json-prune-fetch-response`,
`json-prune-xhr-response`, `trusted-rpfr`
(+`trusted-replace-fetch-response`),
`trusted-replace-xhr-response`, `no-fetch-if`, `no-xhr-if`
(+`prevent-xhr`), `aopr`, `aopw`, `acs`, `ra` (+`remove-attr`),
`set-attr`. 1,620 of 2,731 lines compile; the rest stay
skipped-and-counted (aeld/nostif/nowoif/nano-* are anti-adblock
defusers and fingerprinting-adjacent traps — deliberately not
approximated; href-sanitizer / storage / cookie families are
enhancement scriptlets outside the ad-blocking mission).

## Part 4 — Engine design (as built)

- **Compiler.** `##+js` lines are tokenized with uBO's grammar
  (quoted strings, `/regex/flags` literals with `[...]` classes,
  bare tokens; commas split only at top level — YouTube's L20
  regex contains an escaped `\,`). Entries are DATA: canonical
  name + raw arg tokens. `#@#+js` exceptions subtract by
  (name, args) on intersecting hosts; a no-args exception
  removes the whole name in scope. Every line is accounted:
  the compile report's identity (total = comments + network +
  exceptions + cosmetics + flags + scriptlets + exceptions +
  duplicates + badfilter + skipped) is test-pinned.
- **Payload.** Per-host entries ship as JSON inside ONE
  hand-written runtime; args are never interpolated into code.
  Budget 220,000 B against a measured full set of ~187 KB —
  every host ships; if lists grow, YouTube-family hosts are
  ordered first, then entry volume. The document script is the
  scriptlet runtime followed by the cosmetic runtime (one
  injection, two independently-guarded IIFEs).
- **Runtime semantics** (the uBO subset the lists use):
  - *json-prune*: wraps `JSON.parse`; arg0 = paths (or
    *required* paths when arg1 is present — `enabled,
    force_disabled` shape), arg1 = prune paths, later arg =
    needle (raw-text substring gate). Path language: dotted
    keys, numeric indices, `[]` any element, `[-]` last
    element, `*` any key.
  - *json-prune-fetch/xhr-response*: same paths; arg1 = needle;
    remaining args are URL conditions (`propsToMatch` marker
    skipped, `url:<pat>`, bare `/regex/`, bare substrings).
    fetch transforms a `clone()` — the original stream is
    never consumed; a rebuilt `Response` is returned only when
    the text changed. XHR hooks `open`/`send`; text responses
    are transformed and shadowed as own properties, `json`
    responses pruned in place.
  - *trusted-rpfr / trusted-replace-xhr-response*: needle as
    `/regex/flags` (native `$1` backrefs), quoted literal
    (split/join), or bare regex source (`player?`).
  - *set*: one accessor trap per window root, shared by all
    chains rooted there; handlers run on the present value and
    every reassignment (`var ytInitialPlayerResponse = …`
    assignments fire the setter because the accessor
    pre-exists — the mechanism uBO's set-constant relies on).
    The leaf becomes a constant getter with an ignoring
    setter. Traps never remove themselves: SPA navigations
    reassign these globals per video.
  - *aopr/aopw/acs*: leaf getter/setter that throws under the
    uBO conditions (needle in the error stack / inline
    `document.currentScript` whose text matches), preserving
    the original value otherwise.
  - *no-fetch-if / no-xhr-if*: pattern = `/regex/` or
    substring, optional trailing ` method:POST`; fetch resolves
    an empty 200, XHR completes synthetically without touching
    the network.
  - *ra / set-attr*: applied at install, DOMContentLoaded, and
    on a throttled MutationObserver; writes happen only when
    the value differs (no self-loop).
  - Everything is per-entry try/caught, wrappers are
    fail-open (any error → the original response/value), and
    every wrapper consults `window.__picoShieldOff` per
    action.
- **Accounting.** Prunes count per path in-page and flush
  (throttled, on pagehide) as `shield_prune` IPC (V23-10) →
  capped `shield prune:` log lines.

## Part 5 — Boundaries (what uBO does that v23 still doesn't)

- Procedural cosmetics (`#?#`, `:has-text` & friends under
  `##`, `##^` html filters): counted, not executed (167 lines).
- The json-edit family (recursive-descent `..` / `*[?…]`
  paths, 5 lines) and `xml-prune` (0 lines in the lists).
- `redirect=`/`redirect-rule` resources (no redirect-resource
  library), `$removeparam`, `$csp`, `$popup`, `$urlskip`,
  `$uritransform`, `$permissions`, `$denyallow`: rules carrying
  them stay skipped-and-counted.
- `$replace` network rules (V23-6): no response-body API in
  WebView2; the scriptlet twins cover YouTube.
- A rebuilt fetch `Response` has an empty `.url` (the
  constructor cannot set it) — pages that read `response.url`
  after a prune see `""`; uBO's Firefox implementation shares
  a version of this caveat, Chromium uBO avoids the path by
  not pruning fetch bodies it cannot rebuild faithfully.
  Accepted: the player reads the body, not the URL.
- XHR transform ordering: our completion hook registers at
  `send()`; a page listener registered earlier for
  `readystatechange` can observe the untransformed text first.
  `load` listeners (the common consumer shape) run after.
  Same constraint class as uBO's own XHR scriptlets.
- `strict1p/strict3p` are treated as `1p/3p` (the strict
  same-site refinement needs eTLD+1 subtleties the hostname
  engine approximates elsewhere too); `method=` with an
  unknown method makes the rule inapplicable rather than
  applicable (fail-open).
- Entity-scoped generichide anchors (`||userupload.*^$ghide`)
  under-match in the cosmetic runtime's host list (the flag is
  carried as the literal `userupload.*` key, which the
  runtime's exact/suffix checks won't equate with
  `userupload.net`): those hosts keep generic cosmetics where
  uBO would suppress them — the benign direction.
- The scriptlet payload, like the cosmetic one, is selected
  per host at runtime from the shipped map; hosts beyond the
  byte budget (none today) would lose scriptlets first by
  lowest volume, YouTube family never.

## Part 6 — The honesty line

YouTube rotates its countermeasures and the filter lists chase
them; no build can promise a pre-roll never appears again.
What v23 guarantees is narrower and checkable: the CURRENT
lists' full YouTube arsenal — as fetched on the FILTERS.md date
(2026-10-24), every scriptlet, network rule, and exception the
catalogue in Part 3 enumerates — is now compiled and executed,
with jsdom proofs against the real payload (fetch, XHR,
JSON.parse, and the initial-data trap all strip the ad keys
while `streamingData` survives), and `pico.log` witnesses
(`shield: lists compiled — … scriptlets …`, then `shield
prune:` lines) that let the user verify the layer fired on his
own machine. If YouTube changes the shape again, the fix path
is a list update, not another engine.


---
# Pico v22 — Audit Addendum (2026-10-30)

This addendum records the from-zero, line-by-line re-audit of the
complete v21 tree that preceded the v22 rewrite, and the v22
video/login chapter. The v21 audit below it remains the historical
record; where this addendum and the older text disagree, this
addendum governs.

Method: every v21 source file was read completely (src/, assets/,
tests/, examples/, scripts/); v21's headline claims were
independently re-verified on the unmodified tree before any v22
code was written (Rust tests 175/175, DOM checks 69/69, no-manifest
gate PASS on the shipped v21 exe, exe size/md5 matching its
BUILD_NOTES, filter lists byte-identical to their FILTERS.md
hashes); every control on both pages was traced to a working
handler; every IPC variant was matched on both sides; every
persisted state field was traced to a reader; every silent error
path was checked for a log line.

## Part 1 — Findings

| ID | Severity | Where (v21) | Finding | v22 disposition |
|----|----------|-------------|---------|-----------------|
| V22-1 | Low | core/server.rs:1307, call site 1078 | `respond_range_unsatisfiable` wrote the 416 body even for HEAD requests (the `send_body` flag was not threaded into it). A HEAD client received body bytes its own Content-Length accounting forbids. | Fixed: the helper takes `send_body`; regression test `head_range_unsatisfiable_has_no_body`. |
| V22-2 | Low | core/server.rs (events branch + reserved-prefix branch) | Error responses on the SSE events endpoint and the reserved `/__pico/` prefix hardcoded body output, so HEAD requests there also carried bodies. | Fixed: `send_body` threaded; non-GET on events is a headers-only 405. Malformed-request-head error responses (431/parse failures) still carry bodies — the method is unknowable at that point and a conforming HEAD client never reaches them; accepted residual, stated here. Regression test `head_on_events_endpoint_is_headers_only_and_not_a_subscription`. |
| V22-3 | Low | core/detect.rs:584 | `parse_http_response` computed the body offset from a status line measured after trimming `\r`, drifting one byte short on CRLF responses; a probe body could be mis-sliced by one byte. | Fixed: head/body split at the exact `\r\n\r\n` / `\n\n` terminator. |
| V22-4 | Low/Med | core/shield.rs:1299 | Anchored-rule matching located the request host with `url_lower.find(&req_host)` over the whole URL. A host string appearing in userinfo (or the path) misaligned the pattern remainder (fail-open direction), and for IPv6 literals the anchor landed inside the brackets. | Fixed: `host_span(url, host)` parses the authority (userinfo skipped, bracket-aware) and returns the true (host_start, after_host). Tests: `host_span_skips_userinfo_and_handles_ipv6`, `anchored_rules_judge_the_real_host_not_userinfo`. |
| V22-5 | Low | core/shield.rs (decide) | `$genericblock` suppressed generic EXCEPTIONS together with generic block rules; uBlock Origin keeps exceptions in force, so a generic exception could silently lose to a host-anchored block on a genericblock page. | Fixed: suppression applies only when `!rule.exception`. Test: `genericblock_spares_generic_exceptions`. |
| V22-6 | Low | core/shield.rs (cosmetic build) | The `elemhide` page flag was parsed but never honored: it never reached the injected cosmetic payload, so elemhide pages still got element hiding. | Fixed: `eh_hosts`/`eh_all` collected and shipped (`eh`/`ehAll` in the payload); the runtime returns early on elemhide pages. Test: `elemhide_and_global_generichide_reach_the_cosmetic_payload`. |
| V22-7 | Low | core/shield.rs (cosmetic build) | A global (unscoped) `$generichide` declaration never reached the payload's host set — only domain-scoped ones did. | Fixed: `gh_all` tracked and shipped as `ghAll`; the runtime suppresses the generic set globally. Same test as V22-6. |
| V22-8 | Info | core/state.rs (module doc) | The module doc said "Schema **version 2**" while `SCHEMA_VERSION` is 3. Doc-only drift. | Fixed in the v22 header. |
| V22-9 | Low | src/app.rs:518 | `attach_shield` recorded the block (counters + event + log) BEFORE building and setting the neutered response; a COM failure in between left the counters claiming a block that fail-open had not delivered. | Fixed: record only after `SetResponse` succeeds; failures log `shield: neuter failed … (request left untouched)` / `shield: SetResponse failed …` and leave the request alone. |
| V22-10 | Low | src/app.rs (NewWindow arm) | New-window routing existed only as inline event-arm code: untested as a unit, allowed decisions unlogged, denial reasons collapsed into one line. | Fixed: the decision is the pure `core::addr::route_new_window`, unit-tested as a matrix (`tests/newwindow_tests.rs`); the arm logs one line per decision (`new-window: opened in new tab: …` / `new-window: denied reason=… url=…`). |
| V22-11 | Info | boundary | Routing popups to tabs severs `window.opener`. wry 0.53.5 does expose an opener-preserving `NewWindowResponse::Create`, but constructing a WebView2 control on wry's handler thread inside Pico's tab model is a new machinery class the v22 brief does not take. Full-page sign-in flows (YouTube/Google) are unaffected; opener-dependent "sign in with Google" popup widgets on sites with no full-page path may still fail, and `about:blank` script-popups are denied by design. | Documented boundary (here and in BUILD_NOTES); revisit only if a real site the user needs hits it. |
| V22-12 | Verified clean | whole tree | Grep-proof over v21 src: NO cookie/storage restriction or clearing APIs anywhere (zero hits for the cookie-manager / clear-browsing-data / delete-cookies families). The ONLY profile settings touched in the tree are `SetIsPasswordAutosaveEnabled(false)` and `SetIsGeneralAutofillEnabled(false)` in `harden_webview_profile`. | Carried as a standing pin: `tests/media_tests.rs::source_scan_no_cookie_or_storage_restriction_apis_anywhere_in_the_shell` fails the build if any of those API names ever appears in src/app.rs or src/main.rs. |
| V22-13 | Info | SECURITY.md:328 | v21's SECURITY.md "Deliberate non-goals" still listed "No ad blocker" — stale since the v21 Shield amendment. | v22 SECURITY.md rewritten; the Shield is documented as a feature, not a non-goal. |
| V22-14 | High | core/media.rs:65 | The composed UA carried the **Edge token** (`… Chrome/{v} Safari/537.36 Edg/{v}`). Google's sign-in refuses embedded-browser fingerprints ("This browser or app may not be secure"); an Edg token emitted by a WebView2 host is exactly such a signal. This was the largest Pico-side obstacle to "login and remember it". | Fixed: the UA is the standard reduced desktop-Chrome form `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/<runtime-major>.0.0.0 Safari/537.36`, version still derived from the installed runtime and sanitized as before, applied to every content webview, and the startup log names the form (`content user-agent (desktop-chrome): …`, `core::media::UA_FORM`). Tests rewritten: `ua_is_the_desktop_chrome_form_with_the_runtime_major`, `ua_falls_back_when_the_version_is_missing_or_garbage`. |
| V22-15 | Analysis | src/app.rs (content new-window handler) | The v22 brief's premise "Pico denies ALL new-window requests" was partly stale against the code: v21 content views already routed http/https popups to tabs via `UserEvent::NewWindow` → `add_tab` (chrome view denies all — correct). The real gaps were V22-10's (untested, unlogged, inline, action-URL check done with the nav classifier). | Routing kept, moved into pure core, logged per decision, action URLs denied with their own reason before the allowlist is consulted. |
| V22-16 | Verified | core/state.rs + src/app.rs | Remember-me substrate verified in code: the profile is the pinned `pico_dir()/WebView2` derivation, one WebContext over it, no private/incognito mode anywhere, no clearing code (V22-12). | v22 adds the startup witness `profile: persistent, cookies db present=<bool>` (`core::media::cookies_db_present`, existence only, both WebView2 layouts), a profile-path stability test across simulated launches, and a state-relaunch test (theme/shield/recents/window survive save→load). |

### Verified not-defects (spot list)

Traced end-to-end and working in v21, re-implemented and
re-tested in v22: every chrome control (new tab, back, forward,
reload, home, open-file, theme toggle, shield toggle, window
min/max/close/drag, brand byline, address submit, tab
click/close/dblclick, keyboard shortcuts) reaches a handler;
every start-page control (social chips, service cards, recents,
pick-file) reaches the single action dispatch; every
`ipc::Command` variant is matched in `on_chrome_cmd` and every
content IPC family (nav notes, actions) is routed; every
`PersistState` field is read somewhere (theme, recents, window,
shield_on, blocked_total, version); boot-generation retirement
and the dedupe window behave as documented; the NAV_OBSERVER
and cosmetic scripts are document-start only on http(s); the
detector qualification bar, the server Host wall (including
SSE), and the action gate were all re-verified in place. The
v21 Shield compile report was reproduced line-exact on the
shipped lists (65,519 non-empty lines → 40,716 network rules +
192 exceptions; 15,457 cosmetic lines; 4,712 skipped-and-counted)
— V22-4..V22-7 do not change it, because the shipped lists
contain zero page-flag rules.

## Part 2 — Video & login (the v22 headline chapter)

**What the evidence says.** The user's own pico.log (v18 probe,
2026-10-24) shows googlevideo answering Pico's media requests:
main formats return 403 classified `server-refusal` while other
streams serve 200, on an anonymous session from his egress.
Anonymous playback permission is therefore YouTube's server-side
trust decision about the session/egress — not a Pico bug and not
Pico's to override. Pico does not circumvent it: no client
impersonation, no PO-token forgery, no embed/TV-client tricks.
That boundary is deliberate and stays.

**The engineering target** is the user's stated fallback —
"at least able to login and remember it" — and v22 removes
every Pico-side obstacle the audit found:

1. **Sign-in fingerprint (V22-14, High):** the UA is now the
   standard desktop-Chrome form. The Edg-token form told
   Google's risk engine "embedded Edge-flavored context";
   the Chrome form is the shape a normal desktop browser
   presents. (Stated plainly: this removes a known refusal
   signal; it does not fake a different client — the UA is
   composed from the real WebView2 runtime version, and the
   media-safe argument pinning is unchanged.)
2. **Popup gap (V22-10/15):** sign-in and `target=_blank`
   flows that open a new window now get a new tab under the
   same allowlist and tab cap, with the Shield applying;
   action URLs and non-web schemes are denied and logged.
3. **Remember-me (V22-12/16):** one pinned persistent profile,
   default cookie behavior (nothing restricts third-party
   cookies or storage), no clearing code path anywhere —
   grep-proven in v21 and pinned by a source-scan test in v22 —
   plus the startup witness line so a future pico.log can
   answer "did the previous session persist?" at a glance.

**The exact on-device test** (BUILD_NOTES repeats it): sign in
to YouTube once → close Pico → reopen Pico → YouTube should
still show the signed-in avatar, and a video should play. If
the restart comes back signed out, pico.log's
`profile: persistent, cookies db present=` line separates the
causes: `false` on the second launch means the profile folder
moved or was wiped outside Pico; `true` with a signed-out site
means the site declined to persist the session, which is
site-side.

---
# Pico v21 — Addendum: the Shield (uBlock-grade ad blocking, correct ABP semantics)

## Scope amendment (read first)

The 2026-10-21 reset spec listed an ad blocker as a NON-GOAL.
The user explicitly reversed that on 2026-10-24: "it doesn't
have uBlock Origin any more — add it, make it better, rework,
remaster, remake." This addendum records the amendment: the
Shield is in scope by explicit user order. Everything else in
the reset spec stands.

## Why this is not the C#-era Shield

The v7 catastrophe is the design constraint: that build
flattened path-scoped uBlock rules into a domain list, so
google.com — present in the lists only inside path-scoped
rules — became a blocked MAIN DOCUMENT. v21 therefore
implements the Adblock-Plus semantics themselves, in a pure
core module (`src/core/shield.rs`), never a flattened list:

- Rules keep their patterns (domain anchors `||`, edge
  anchors `|`, wildcards `*`, separators `^`), their options
  (party, `domain=` incl. `~` exclusions and `.*` TLD
  wildcards, type sets incl. `~` negations, `$important`,
  `$badfilter`), and their precedence: `$important` block >
  `$important` exception > exception > block; ALL options
  must match for a rule to apply.
- Options outside the supported set (`csp`, `redirect`,
  `removeparam`, `rewrite`, `popup`, `webrtc`, `match-case`,
  unknown keys, …) SKIP the rule — counted in the compile
  report, never silently misapplied. Regex rules, procedural
  cosmetics (`#?#`) and scriptlets (`##+js`) are likewise
  skipped and counted: they are the documented v1 boundary.

## The three safety laws (test-pinned, tests/shield_tests.rs)

1. A main-frame DOCUMENT is blocked only by a rule that is
   domain-anchored AND explicitly carries the document type
   — uBO's no-strict-blocking posture. Named regressions run
   the FULL compiled lists: google.com and youtube.com main
   documents always Allow.
2. Loopback (`localhost`, 127.0.0.1, [::1], *.localhost) and
   internal origins are never blocked in either layer,
   whatever the lists say — even against synthetic rules
   naming them directly.
3. Fail open: the request handler is a checked shell around a
   pure function; any COM or state error leaves the request
   untouched. No unwrap/expect/panic anywhere in the path.
   (Law 4: OFF means the whole pipeline short-circuits on one
   atomic flag.)

## Engine shape

- Domain-anchored rules index by host key; a request walks
  its host and parents. Generic patterns compile to segment
  matchers; their longest literal (>= 3 chars) becomes an
  Aho-Corasick needle (hand-rolled, no new dependency) so a
  decision costs one host walk + one text scan. Needle-less
  generics (2 in the shipped lists) are always evaluated.
- Party is computed on registrable domains (built-in
  two-level suffix table — a documented approximation of the
  public suffix list, exact for the common cases).
- `$badfilter` neutralizes its identity twin (pattern +
  canonical options); duplicates are removed by the same
  identity. Both are counted, not silent.
- Compile accounting is exact: every non-empty list line is
  in exactly one report bucket, asserted by test.

## Network layer (app.rs)

Per content WebView (never the chrome): a
`WebResourceRequested` handler for all contexts, following the
media probe's COM pattern. Decisions are synchronous from the
Arc-shared engine. The page URL for party/`domain=` comes
from App-tracked tab state (NavigationStarting records the
pending main target; commits and SPA notes update the
committed URL) — WebView2's event carries no page URL, so
this is a documented approximation: requests are decided
against the tab's current committed URL. A DOCUMENT-context
request counts as the main frame only when its URL equals
the tab's pending/committed main URL; other document-shaped
requests are iframes (`$subdocument`). Blocked requests get
a neutered 200: a 1x1 transparent GIF for images, an empty
body with the right Content-Type for the rest — never an
error page. Four new `unsafe` blocks (15 total, from 11),
each with a SAFETY comment: the filter registration, the
handler body, the handler registration, and the response
factory (whose one non-empty body goes through the
environment vtable + SHCreateMemStream because the typed
signature names an `IStream` type from a crate Pico does
not directly depend on).

## Cosmetic layer and its measured boundary

One document-start script (wry initialization script on
content builders; pushed per-load into views built before
compilation finished). Payload decision, measured on the
shipped lists: the GENERIC selector set alone serializes to
~237 KB of JSON, so the "~150 KB" sketch budget cannot hold
the generic bulk — the generic set ships in full (it is the
visible bulk of cosmetic value), and host-specific sets fill
the remaining room to a 280,000-byte ceiling, highest rule
volume first: 338 hosts ship, of 938 with rules. The rest are
the documented boundary (their sites still get generic
hiding + full network blocking). The runtime applies the
generic style + the current host's set once, re-applies on
DOMContentLoaded, no-ops on non-http(s) and loopback hosts,
and honors generichide. `MutationObserver` churn was left
out deliberately: style-based hiding covers late insertions
without per-mutation JS cost.

## Lists and the one new dependency

EasyList, EasyPrivacy, uBlock filters + privacy (uAssets),
fetched 2026-10-24 from the canonical sources, pinned in
`assets/filters/` with `FILTERS.md` (URLs, sizes, sha256,
licenses, uBlock Origin credit). Embedded gzip-compressed
(560,255 bytes for 1,792,382 raw). Decompression needs
`flate2` — the ONE new dependency the task permitted (pure
Rust backend, `default-features = false`); the exe math is
in BUILD_NOTES. `windows-sys` gained the `Win32_UI_Shell`
feature (SHCreateMemStream) — a feature of an existing
dependency, not a new one.

## State

state.json schema v3 adds `shield_on` (default ON, including
after a v2 migration) and `blocked_total` (lifetime, across
sessions). The chrome snapshot gains `shieldOn`,
`blockedTotal` (session) and `blockedTab` (active tab);
the address row gains a neumorphic shield toggle (pressed +
accent when ON — the v20 token vocabulary, nothing new) and
a blocked-count chip.

---

# Pico v20 — Addendum: TRUE neumorphism (one material, dual shadows, attached tab)

User verdict (screenshot of the v19 header, 2026-10-24):
"the header still looks bad, its not neumorphism, and its not
aligned good or attached well — do a full rework and remake."
The verdict is correct on the facts, and this addendum says why
in the v19 sources' own declarations: v19 (like v16–v18) was a
FLAT dark UI — pills and buttons separated by 1px borders and
subtly different fills. v19's own style.css even used the word
"neumorphic" for a `.raised` class almost nothing used. Real
neumorphism is a material discipline: ONE surface color, shape
drawn ONLY by a dual soft shadow (light toward the light source,
dark away), elements extruded or pressed from the same slab,
and no border ever drawing a shape. This addendum is (1) the
evidence table, (2) the v20 design language that replaces it,
(3) the attachment construction, (4) the render-pass summary.
Geometry (86 / 40+46 / 74 / 12), all behavior, and all security
controls are unchanged; the Rust diff vs v19 is version strings
plus test adaptations — verified by diff (see BUILD_NOTES).

**Baseline re-verified in this session (v19 code, unmodified,
in the v20 tree):** `cargo test --locked --no-default-features`
→ **140 passed, 0 failed**; jsdom DOM test → **59/59**.

---

## Part 1 — Why the v19 chrome was not neumorphism (evidence)

Sources: `assets/style.css` (shared tokens), `assets/chrome.html`,
`assets/start.html` — all values as shipped in v19.

### N1 — Two materials: fills, not shadows, separate the elements
Dark theme: `--bg: #212429` vs `--surface: #26292f` vs
`--raised: #2c2f36` — three stacked grays. `#topbar` painted
`var(--bg)`, `#navbar` and the active tab painted
`var(--surface)`, `.nbtn`s were flat `background: none` at rest
and gained a *fill* (`var(--surface)`) on hover. Every shape in
the header was drawn by a fill boundary — a card on a shelf.
That is the flat-UI discipline, whatever the comments claimed.
*Disposition:* **Fixed** — per theme, `--bg` == `--surface` ==
THE material (dark `#212429`, light `#e6e9ef`); elements sit IN
the slab and are read by shadow alone. A contract test pins
the equality of the two tokens in both themes.

### N2 — Shadows were hand-rolled garnish, in one direction, per element
Thirteen raw `box-shadow` literals across the two pages, each a
different geometry (7/14 hero ring, 5/11 card, 4/9 chip, 4/8
recent, inset 2/4 scheme tag, inset 3/7 well/empty/errbar…), and
the most important one wasn't even a dual shadow: the active
tab's `3px -3px 7px dark, -3px -3px 7px light` has NO
bottom-right dark pass — it's a top glow that reads as an
outline. Buttons cast shadows only on :hover. Neumorphism's
vocabulary — one raised pair, one inset pair, everywhere —
existed nowhere.
*Disposition:* **Fixed** — the `--neu-*` vocabulary (Part 2),
defined once per theme as geometry × the theme's two shadow
tints. A contract test greps all three CSS sources: every
`box-shadow` declaration must be composed solely of `--neu-*`
references (or `none`).

### N3 — The active tab floats (the attachment defect)
The tab's fill matched the NAVBAR (`--surface`) but the strip
behind it was `--bg`; its partial shadow stopped dead at the
bottom corners, and the bottom edge sat on the row boundary as
a visible seam. Net effect (user's screenshot): a detached
lozenge hovering in a darker tray.
*Disposition:* **Fixed** — Part 3's construction: one slab,
the tab's shadow on a clipped `::before` layer that cannot
cross the tab's bottom edge, and concave radial-gradient
fillets sweeping the bottom corners into the toolbar.

### N4 — Focus was drawn with a border and an outline
`#addr-wrap` carried a 1px transparent border at rest precisely
so focus could paint it (`border-color: var(--accent)`), and
the shared `:focus-visible` drew a 2px outline. Both are
vector lines — the exact thing the material forbids.
*Disposition:* **Fixed** — the well has no border declaration
at all; focus adds ONLY `--neu-focus` (a soft accent glow,
alpha 0.16 light / 0.18 dark — contract-pinned ≤ 0.18) layered
over the unchanged `--neu-inset`; `:focus-visible` is a
shadow ring (`--neu-focus-ring`), and the input suppresses its
own ring (render pass 1 showed the generic ring painting a
hard rectangle inside the pill).

### N5 — A border still drew a control's shape
`.brand .ver { border: 1px solid var(--accent) }` — the version
badge was an outlined pill. *Disposition:* **Fixed** — border
deleted; the badge is a `--neu-raised-sm` pill of the material
with accent text (still exactly 22px tall: line-height 18 +
2×2 padding, matching the brand mark). Global contract: no
border declaration in the shipped CSS may draw (values `none`
only), with ONE allowlisted exception — `.tab .spin`, the
loading spinner, whose 2px ring is a functional progress
rendering, not a shape outline (removed from the scan by exact
selector, and the test asserts the exception is that rule).

### N6 — The caption cluster is the one honest fill exception
Window controls tile to the window's top-right corner, where a
raised shadow would clip against the window edge. Their hover
therefore uses the `--raised` token — a documented ±few-steps
luminance nudge fill (dark `#262a31`, light `#eceff5`) — and no
shadow. The close button's hover keeps the `--danger` fill
(platform convention, already a token in v19).

### N7 — What deliberately stays outside the system
`#shellwarn` keeps its hard-coded emergency styling (v14: it
must render when every other style and the IPC channel are
dead). The start page's `h2::after` hairline is a *background*
rule (a content divider), not a control border. Text colors
are untouched from v19 — neumorphism's low contrast applies
to shapes, never to text (all text tokens stay ≥ 4.5:1).

---

## Part 2 — The v20 design language (final token values)

Defined once, in `assets/style.css`, per theme; geometry is
shared, tints are per-theme (built from the existing
`--shadow-dark` / `--shadow-light` ingredient tokens):

| token | value (geometry × tints) | used by |
|---|---|---|
| `--neu-raised` | `6px 6px 12px dark, -6px -6px 12px light` | service cards, hero ring, picker button, `.raised` |
| `--neu-raised-sm` | `4px 4px 8px dark, -4px -4px 8px light` | nav/utility/new-tab buttons, tabs, badges, chips, recents, hint, errbar, brand mark + badge |
| `--neu-inset` | `inset 4px 4px 8px dark, inset -4px -4px 8px light` | address well, scheme tag, empty state, `.well` |
| `--neu-pressed` | `inset 3px 3px 6px dark, inset -3px -3px 6px light` | `:active` of raised controls |
| `--neu-focus` | `0 0 8px 0 var(--focus-glow)` | address `:focus-within`, over the inset pair |
| `--neu-focus-ring` | `0 0 0 2px var(--focus-outline)` | `:focus-visible` (keyboard only) |

Tints — dark: dark `rgba(0,0,0,0.55)`, light
`rgba(255,255,255,0.09)` (rose 0.06 → 0.09 across the render
passes; see Part 4). Light: dark `rgba(163,177,198,0.65)`,
light `rgba(255,255,255,0.9)`.
Material — dark `#212429`, light `#e6e9ef` (`--bg` ==
`--surface`, contract-pinned).
Radii: tabs and the address pill stay on the v19 scale
(`--r-lg` 12 / `--r-pill`); nothing about the radius or spacing
systems changed.

Resting states are now neumorphic too: nav buttons, the +
button, badges, chips and recents are raised AT REST (v19
raised them only on hover, or not at all); pressing swaps the
raised pair for `--neu-pressed` — the control sinks into the
slab. The accent-filled elements (brand mark, hero ring core,
primary picker button fill) keep their identity fill — an
accent disc is a logo/CTA, not a shape outline — and take the
same dual shadow as everything else.

## Part 3 — Attachment: how the tab grows out of the toolbar

Three mechanisms, all in `assets/chrome.html`:

1. **One slab.** `#topbar` now paints `var(--surface)` — the
   same token the navbar's solid zone paints and the feather
   melts from. There is no seam, line, or shadow between the
   two rows anywhere; the strip behind the tabs and the
   toolbar beneath them are one continuous material.
2. **Clipped shadow layer.** The tab's shadow moved to
   `.tab::before` (z-index −1 inside the tab's stacking
   context, opaque material background) with
   `clip-path: inset(-18px -18px 0 -18px)`: the shadow may
   spill 18px up and sideways but is cut EXACTLY at the tab's
   bottom edge. No shadow line ever crosses into the navbar,
   so the tab's material runs unbroken into the toolbar.
   Inactive tabs are flat labels on the slab; hover raises
   through the same clipped layer, so even a hovered tab stays
   attached at its bottom.
3. **Concave fillets.** `.tab[aria-selected="true"]::after`
   carries two 12px radial gradients (`circle at 0 0` /
   `circle at 100% 0`, transparent inside the quarter arc,
   material outside) at the bottom corners — the classic
   connected-tab sweep, bending the side shadows around the
   arc instead of stopping them at a square corner.

Render evidence (Part 4) chose this over the neck fallback:
with one material everywhere, square corners + clipped shadow
already merge the tab downward, and the fillets add the
concave sweep the eye expects at the junction — visible and
calm in both themes at 4× crops.

## Part 4 — Render passes (summary; full log in BUILD_NOTES)

Three render → read → adjust passes over the ASSEMBLED pages
(chrome-headless-shell, stub snapshot, both themes, DSF 1 +
1.25, blend variants, 4× tab crops, start pages):
- **Pass 1** caught three defects in the FIRST v20 cut: the
  tab face rendered cloudy (the transparent `::before` let its
  own shadow silhouette bleed through the tab's interior);
  the generic `:focus-visible` ring painted a hard blue
  rectangle inside the focused pill; the focus glow (12px blur
  + 2px spread) threw a blue rule across the header.
- **Pass 2** fixed all three (opaque `::before`; the input's
  own ring suppressed — the wrap glow is the only address
  focus sign; glow tightened to `0 0 8px 0`) and re-rendered:
  flat tab face, calm halo, attached tab in both themes.
- **Pass 3** tuned presence: the dark theme's light tint rose
  0.07 → 0.09 after the dark tab's top edge nearly vanished
  against the slab; re-renders confirmed the top light rim
  reads without washing anything out. Start pages verified in
  both themes (cards/recents/chips read as extrusions of the
  slab), and the feather still melts the slab into a page.

---

# Pico v19 — Addendum: chrome visual rework (design audit + one alignment system)

User verdict (screenshot crop of the merged header, 2026-10-24,
v18 running): "how ugly is this and unadjusted — fix it and
rework." The crop shows the tab row + address row: the address
bar wears a loud neon-blue focus ring that dominates the header
and fights the neumorphic design, and the elements sit *near*
each other instead of *on* a grid — cramped brand block, uneven
gaps, floating window controls. Order: visual rework only. The
geometry contract (chrome 86 / rows 40+46 / solid 74 / feather
12), all behavior, and all security controls are unchanged;
this addendum is (1) a measured defect table of the v18 chrome
CSS and (2) the single alignment system v19 imposes, with each
defect's disposition.

**Baseline re-verified in this session (v18 code, unmodified, in
the v19 tree):** `cargo test --locked --no-default-features` →
**135 passed, 0 failed**; jsdom DOM test → **52/52**.

---

## Part 1 — The defect table (all values measured from the v18 sources)

Sources: `assets/chrome.html` (chrome `<style>`), `assets/style.css`
(shared tokens), `assets/start.html`. "Scale" below refers to the
v19 system defined in Part 2: spacing {0, 2, 4, 6, 8, 12, 16},
radii {6, 8, 12, 16, 999}, one 12px page inset, icon buttons
30×30 r8, glyphs 16px / stroke 1.8.

### D1 — The address-bar focus ring is a neon hoop (the headline defect)
`#addr-wrap:focus-within { outline: 2px solid var(--accent);
outline-offset: 1px; }` — a **2px, full-alpha** accent outline
(light `--accent` #1d4ed8, saturated royal blue; dark #7aa8ff,
bright periwinkle) drawn around the *entire* pill — the widest
element in the chrome — with a 1px offset gap that lets the
navbar color show through, producing a double-line hoop. No
blur or softening anywhere; the hard vector edge is the exact
opposite of the neumorphic soft-shadow language every other
element speaks. Because the page focuses the address bar at
startup, this ring is the header's resting appearance, not an
edge case.
*Disposition:* **Fixed in v19** — 1px accent border (box
pre-allocated with a 1px transparent border at rest, so there
is zero layout shift) + a soft outer glow from a new token
`--focus-glow` (light rgba(29,78,216,0.16), dark
rgba(122,168,255,0.18) — alpha ≤ 0.25 in both themes), 120ms
transition on border-color + box-shadow. The global
`:focus-visible` outline is softened to the new
`--focus-outline` token (accent at 0.55 alpha; keyboard-only).

### D2 — The two rows do not share a page inset; left and right edges wander
`#topbar { padding: 0 8px 0 14px }` vs `#navbar { padding: 1px
12px 13px }`. Left edges: the brand mark starts at x=14, the
back button's box at x=12 — a 2px stagger on what should be one
vertical line. Right edges: the window-controls cluster ends
8px from the window edge, the navbar's last button 12px — a
4px stagger. Neither side lines up.
*Disposition:* **Fixed in v19** — one 12px inset both rows:
`#topbar { padding: 0 0 0 12px }`, `#navbar { padding: 1px 12px
13px }`. Brand mark left edge == back button left edge (x=12).
The window-controls cluster is the documented borderless
exception: it sits flush to the window's top-right corner (0
inset, full-height caption boxes — the Windows caption
convention), while its glyphs join the common 16px system.

### D3 — Six gap values, three of them off any scale
Measured `gap` values: #topbar 4, .brand 8, #tabstrip 6,
.tab 7, #navbar 7, .badges 8, #addr-wrap 8, .win-controls 6,
#errbar 7. The value **7** (used three times) belongs to no
scale; the set {4, 6, 7, 8} reads as "near each other" rather
than rhythm.
*Disposition:* **Fixed in v19** — all gaps drawn from
{0, 4, 6, 8}: topbar 4, brand 8, tabstrip 6, tab 8, navbar 8,
badges 8, addr-wrap 8, errbar 8, win-controls 0 (caption boxes
tile contiguously). A contract test greps the chrome CSS for
margin/padding/gap literals outside the spacing scale.

### D4 — Five control boxes, three glyph boxes, six radii
Boxes: `.wbtn` 34×30, `#newtab` 30×30, `.nbtn` 30×30,
`.tab .x` 28×28, `.brand .byline` ~28 tall (padding 7px 8px on
11px text). Glyph boxes: `svg.ic` is 16×16 in the shared CSS,
but `.wbtn svg` overrides to **13×13** and `.tab .x svg` to
**11×11** (start-page chips use 15×15) — the window glyphs and
tab close render visibly smaller and thinner than the nav
glyphs beside them, though all share stroke-width 1.8. Radii:
8 (wbtn), 9 (newtab), 11 (tab top), 6 (tab ×, byline,
focus-visible), 50% (nbtn), 999 (addr, badges, version pill) —
six systems.
*Disposition:* **Fixed in v19** — icon buttons are uniformly
30×30 with radius `--r-md` (8): `.nbtn` (was a 50% circle),
`#newtab` (was r9). Tab close is 30×30, radius `--r-sm` (6).
Both glyph-size overrides are deleted — every glyph renders in
the shared 16px box at stroke 1.8. Window controls become
38×40 caption boxes (equal width, full row height, radius 0).
Radii come from tokens in the shared CSS `:root`:
`--r-sm 6 / --r-md 8 / --r-lg 12 / --r-xl 16 / --r-pill 999`;
tabs take `--r-lg` (was 11px). The byline's padding becomes
8px (hit height ~29 DIP).

### D5 — Brand block metrics are off-grid and off-weight
Mark `.brand .dot` is 18×18 with a 10.5px/800 "P". The version
badge has a **1.5px** border — a half-pixel stroke weight found
nowhere else in the system (hairlines 1px, icon strokes 1.8,
the old outline 2px) — padding `0 6px`, line-height 16px →
~19px tall against the 18px mark. Brand gap 8 ✓; wordmark
13px/650.
*Disposition:* **Fixed in v19** — mark 22×22, glyph 12px/800;
badge border 1px, padding 2px 6px, line-height 16 → badge box
is exactly 22px tall, matching the mark; both center on the
row midline with the wordmark.

### D6 — A bright seam shadow between the rows (light theme only)
`#navbar { box-shadow: 0 -3px 8px var(--shadow-light) }` paints
an 8px-blurred glow *above* the navbar: in the light theme
`--shadow-light` is rgba(255,255,255,0.9) — a visible white
band separating the rows; in the dark theme it is
rgba(255,255,255,0.06) — effectively absent. Same code, two
different headers.
*Disposition:* **Fixed in v19** — the seam shadow is deleted.
The rows separate by the surface-token step (the tab-into-
toolbar design: #topbar `--bg`, #navbar solid `--surface`,
active tab `--surface`) and by the feather's shade below —
identically in both themes.

### D7 — The feather's shade is a hardcoded gray stripe in the light theme
`#feather` layers: `rgba(0, 0, 0, 0.22) → 0 @78%` (top) over
`var(--surface) → transparent`. The melt layer correctly starts
at the navbar's solid color (`--surface` == the gradient stop
at SOLID_H), so there is no *color* banding; but the shade
layer is a literal black at 0.22 alpha in **both** themes —
over a light page it reads as a gray stripe rather than a soft
shadow, and it cannot adapt.
*Disposition:* **Fixed in v19** — the shade is tokenized as
`--feather-shade`: light rgba(96,110,140,0.16) (a blue-gray
matched to the light shadow family, softer), dark
rgba(0,0,0,0.30) (slightly deeper, where a shadow over dark
pages needs it). The melt layer is unchanged.

### D8 — Hardcoded colors outside the token system
Grep of the chrome `<style>` for literals: `.wbtn.close:hover
{ background: #d64545 }` and the feather shade (D7) are the
only un-tokenized colors besides `#shellwarn` (#b3261e / #fff
— literal **by design**: the strip must render even if the
token layer dies; retained and documented). Everything else
already resolves through the `[data-theme]` tokens in both
themes — there are no dark-only leftovers; the theme defects
are D1/D6/D7, where one shared value is wrong per theme.
*Disposition:* **Fixed in v19** — close-hover moves to a
`--danger` token (#d64545, both themes, `:root`).

### D9 — Address text has no explicit line-height
`#addr` inherits `line-height: normal` (~15–16px for its 13px
font) inside the 32px wrap; centering is left to flex luck.
*Disposition:* **Fixed in v19** — `#addr { line-height: 32px }`
(== the wrap height), so the text sits exactly on the row-2
midline (y=57, the solid zone's center — the one vertical
measurement v18 already had right: navbar's 1px/13px vertical
padding centers the 32px content box in the 34px solid zone,
and flex centering puts 30px buttons on the same midline;
v19 keeps that split as the geometry exception to the spacing
scale, pinned by the geometry contract test).

### D10 — Start page shares the tokens but not the rhythm (light touch only)
`.card .scheme` radius 5; `.grid` gap 14; `.recent` gap 11;
`.chip` gap 7; `h2` gap 10 — one-off values against the chrome
system. Card radius 16 and recent radius 12 already match
`--r-xl`/`--r-lg`.
*Disposition:* **Fixed in v19 (harmonization only, no
redesign)** — scheme radius 6, grid gap 12, recent gap 12,
chip gap 8, h2 gap 8; card/recent/chip/action/hint radii now
reference the shared `--r-*` tokens.

### Verified not defects (v18 measurements that are correct and kept)
- Row heights and the geometry contract: 40 + 46 = 86,
  solid stop at 34px into the navbar gradient (SOLID_H − 40),
  feather 12 — unchanged, still pinned to `core::layout`.
- Tabs bottom-seated in the strip (align-items: flex-end,
  34px tab in the 40px row) — the browser convention; kept.
- Active tab == hover background (`--surface`): with D3/D4
  fixed, the active tab is distinguished by its raised
  neumorphic shadow pair *and* full-contrast text vs. the
  hover's muted text — kept, and the hover no longer applies
  to the already-active tab (`.tab[aria-selected="true"]:hover`
  inherits the active treatment).

---

## Part 2 — The v19 system (implemented; contract-tested)

- **Spacing scale** {0, 2, 4, 6, 8, 12, 16} for every
  margin/padding/gap in the chrome CSS; sole exception: the
  geometry-derived #navbar vertical padding (D9).
- **Page inset** 12px both rows (D2); caption cluster flush
  to the corner as documented.
- **Radii tokens** `--r-sm/--r-md/--r-lg/--r-xl/--r-pill`
  defined once in the shared CSS `:root`, used by both pages.
- **Focus** = 1px border + `--focus-glow` halo (alpha ≤ 0.25,
  both themes), no box change between states (D1).
- **New tokens** (defined in *both* theme blocks — a contract
  test asserts token parity): `--focus-glow`,
  `--focus-outline`, `--feather-shade`; `--danger` in `:root`.
- **Visual verification**: the assembled chrome was rendered
  in headless Chromium (chrome-headless-shell) at DSF 1 and
  1.25, both themes, populated via a stub snapshot, over dark
  and light page backdrops — see v19 BUILD_NOTES for the
  render log and what was checked in them. The renders are a
  VM-Chromium approximation of WebView2; the user's screen
  remains the final verdict.

---

# Pico v18 — Addendum: "Video unavailable" (media forensic scan + media environment)

User report (screenshot, 2026-10-24, v17 running, a YouTube
watch page): the page loads completely — title, channel, likes,
recommendation thumbnails — but the player shows "Video
unavailable" at 0:00 with the duration known. So HTML, API data,
and thumbnails (plain HTTPS resources) succeed; the media stream
itself is refused or failing. Order: "fix it, rework, remaster,
redo, recheck all of it, and remake". This addendum is (1) the
forensic inventory of everything media-relevant in v17 + the
pinned wry 0.53.5, with each known failure class verdicted as
**Pico-side / runtime-side / YouTube-side** on code or crate-source
evidence, and (2) the v18 changes that eliminate every Pico-side
suspect the scan could not clear, and instrument the rest.

**Baseline re-verified in this session (v17 code, unmodified, in
the v18 tree):** `cargo test --locked --no-default-features` →
**123 passed, 0 failed**; jsdom DOM test → **52/52**.

---

## Part 1 — The forensic inventory

### What Pico v17 itself sets (grep over `src/`, exhaustive)

| Surface | v17 state | Media relevance |
|---|---|---|
| User agent | **Not set anywhere** — no `with_user_agent` in the tree; the WebView2 runtime's default UA (composed by the runtime, standard Edge shape, `Edg/<runtime version>`) is what sites see | Not a broken/custom UA — but unpinned and unlogged |
| Additional browser arguments | **None set by Pico** — see wry defaults below | — |
| Settings API calls by Pico | Exactly two, via `ICoreWebView2Profile6` in `harden_webview_profile`: password autosave OFF, general autofill OFF. Nothing else — confirmed by reading the function and grepping the tree | Neither touches media, cookies, EME, or tracking prevention |
| Environment / profile | One `WebContext` over `%APPDATA%\Pico\WebView2` (`create_dir_all` only), shared by chrome + all content views; `incognito` attribute never set → wry default `false` → **not** private mode | Profile (cookies, storage, sessions) persists across restarts |
| Cookie/storage clearing | **None.** No cookie APIs, no storage APIs, no `remove_dir` anywhere; the only `remove_file` in the tree is pico.log rotation | Nothing wipes a signed-in session |
| State sanitizer | Operates on the in-memory `PersistState` (theme / recents / window geometry); writes only `state.json` | Physically cannot touch the profile folder |
| Download handler | `with_download_started_handler` → WebView2 `DownloadStarting` only. Media/resource requests never raise it | Cannot intercept a stream |
| Navigation handler | wry binds it to `add_NavigationStarting` (crate source, webview2/mod.rs `attach_handlers`) — an event raised for **document** navigations (main frame + subframes) only. Media and subresource requests raise `WebResourceRequested`/`WebResourceResponseReceived`, never `NavigationStarting` | **Proof: the nav handler cannot touch a media request** |
| Nav observer script | Main-frame http/https only, posts SPA navigation notes | No media interaction |
| Runtime version | **Never queried, never logged** | Codec/EME state undiagnosable from pico.log |
| Media responses | **Never observed** | A refused stream left zero Pico-side evidence |

### What the pinned wry 0.53.5 does when the app sets nothing
(read from the crate sources in the local registry, not memory)

- **Default arguments** (`webview2/mod.rs`, `create_environment`):
  when `additional_browser_args` is unset, wry substitutes
  `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`
  and — because `WebViewAttributes.autoplay` defaults to **`true`**
  (lib.rs `Default` impl) — appends
  `--autoplay-policy=no-user-gesture-required`. The three disabled
  features are Edge UI extras (mini menu, PDF UI, SmartScreen).
  **No flag touches GPU, media, codecs, EME, QUIC, WebRTC, or
  service workers.**
- **Settings** (`set_webview_settings`): status bar off, context
  menus per attribute, zoom control, devtools per attribute,
  script enabled; UA via `ICoreWebView2Settings2` **only when the
  app provides one**; browser accelerator keys off; pinch zoom;
  swipe navigation; non-client region support. Nothing about
  media, cookies, or third-party cookie blocking.
- **Environment**: `CoreWebView2EnvironmentOptions::default()` +
  system language + scrollbar style; browser extensions disabled
  by default; one environment is created per WebView over the
  context's data directory (this wry version caches no
  environment in the context) — which is why wry's own docs warn
  that WebViews sharing a data directory must agree on browser
  arguments: Pico's all-default v17 setup satisfied that
  trivially, and v18's pinning keeps it satisfied by construction.
- **`wry::webview_version()`** exists and wraps
  `GetAvailableCoreWebView2BrowserVersionString` — the runtime
  version is obtainable pre-environment, at startup.

### Failure-class verdicts

1. **YouTube bot mitigation (page shell served, streams refused
   to an anonymous session / flagged egress IP)** —
   **YouTube-side; the dominant suspect, and not fixable by
   code.** It matches the report exactly: everything except the
   signed media URLs works. No Pico code participates in that
   decision. Pico's only duty in this class is that a *signed-in*
   session be possible and durable — verified above (persistent
   non-private profile, nothing wipes it) and pinned by test in
   v18 (V18-3).
2. **Codec / EME / Widevine (runtime-side)** — the Evergreen
   WebView2 runtime ships Widevine and the proprietary codecs;
   an outdated or damaged runtime is the remaining suspect in
   this class. Pico neither ships nor blocks any codec path
   (inventory above). v17 could not even name the runtime in its
   log; v18 logs it at startup (V18-3) so this class is
   diagnosable from pico.log from now on.
3. **UA served a broken player config (fixable class)** — v17
   sent the runtime's own default UA, i.e. a standard Edge UA;
   no evidence it was the cause. Pinned explicitly anyway
   (V18-1): an unpinned UA is a variable nobody here controls.
4. **GPU/rendering flags breaking the media pipeline (fixable
   class)** — **eliminated by evidence**: no such flag exists in
   Pico or in wry's substituted defaults. v18 pins the argument
   string in source + test so this class can never regress
   silently (V18-2).
5. **Third-party cookie blocking breaking googlevideo session
   requests (fixable class)** — **eliminated by evidence**:
   nothing in Pico or in wry's defaults sets any cookie policy;
   WebView2's defaults apply. (If a future runtime changes its
   defaults, the V18-3 probe will show the resulting 403s.)

---

## Part 2 — The v18 changes

### V18-1 — Explicit standard Edge UA, composed from the runtime version

`core::media::compose_user_agent(Option<&str>) -> String` builds
the standard desktop Edge UA
(`Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36
(KHTML, like Gecko) Chrome/<v> Safari/537.36 Edg/<v>`) with `<v>`
taken from the installed runtime's version string at startup
(sanitized to digits/dots — it lands in an HTTP header, so the
composer is a trust boundary; a documented constant covers an
unobtainable version). Set via `with_user_agent` on every
content WebView. **No Pico/app token in the UA** — naming the
wrapper is a fingerprinting gift and buys nothing.

### V18-2 — Browser arguments pinned in Pico's own source

`core::media::PICO_BROWSER_ARGS` is byte-identical to the string
wry substituted in v17 (the three Edge-UI disables + the
gesture-free autoplay policy), now set explicitly via
`with_additional_browser_args` on **both** builders (chrome and
content — one user-data folder, one options set, per the
environment-consistency constraint found in Part 1).
`core::media::args_are_media_safe` defines the allowlist (only
those disable-features entries, only that autoplay value), and a
source-scan test pins: exactly two call sites, both naming the
constant, and no media-hostile flag text
(`--disable-gpu`, `--disable-quic`, `--disable-webrtc`,
`EncryptedMedia`, gesture-requiring autoplay, `--mute-video`)
anywhere in `src/app.rs`, forever.

### V18-3 — Startup diagnostics + the media probe + the profile pin

- **Startup lines in pico.log** (one each): the WebView2
  runtime version (`wry::webview_version()`), the effective
  content user agent (first 80 chars), and the profile folder
  path. Every media/cookie question now starts from facts on
  the record.
- **Profile pin:** the folder derivation moved to
  `core::state::webview_data_dir()` (single source), with a test
  pinning `pico_dir()/WebView2` and a source-scan asserting the
  shell contains no second derivation.
- **Media probe:** every content WebView registers a
  `WebResourceResponseReceived` handler (via webview2-com,
  `ICoreWebView2_2`) that logs — for media-stream requests only
  (`core::media::probe_target`: any `*.googlevideo.com` host,
  or a `/videoplayback` path on any host) — one line:
  `media probe: tab=<id> host=<host> status=<code>
  class=<served|server-refusal|transport|unexpected>`
  (`core::media::classify_media_status`). Host + status only:
  never the URL (signed media URLs carry access tokens in the
  query), never headers. Capped at 20 lines per tab per session
  (`core::media::MediaProbeBudget`). Purely observational —
  the event fires after the response exists; nothing is
  delayed, altered, or blocked. Registration failure is
  logged, never fatal. Reading guide for the next report:
  `403/401` = the server refused the stream (bot-mitigation /
  region / sign-in class — decided server-side); no line at all
  for a failing video = the request never completed an HTTP
  exchange (transport); `200/206` + still unavailable =
  player/config class on the page's side.

### Residuals (stated plainly)

- The dominant suspect (class 1) is **YouTube-side and remains
  possible after v18**: if YouTube is refusing anonymous streams
  on the user's egress, v18 will prove it via the probe
  (`class=server-refusal` on googlevideo hosts) but cannot
  override it — the in-spec remedy is signing in, which the
  verified-persistent profile supports. **v18 does not claim
  videos are fixed; it claims every Pico-side cause the scan
  could find is eliminated or pinned, and that the remaining
  classes are now identifiable from pico.log.**
- `cargo audit` remains **not installed** in this environment
  (documented absence since v13); no advisory verdict claimed.
- No Windows/WebView2 execution is possible in this environment
  (Wine lacks wine32); the new log lines are the on-device
  witness. The probe's two new `unsafe` blocks (COM getters +
  event registration in `attach_media_probe`, SAFETY-commented)
  raise the shipped total from 9 to 11; both are the same
  documented-getter pattern as the existing profile hardening.

---

# Pico v17 — Addendum: Start-page actions were dead (root cause + dispatch rebuild)

User report (screenshots, 2026-10-24): on the start page, the
RECENT FILES entries and the "Open a local HTML file…" button do
nothing when clicked — no recent opens, no file picker, no error.
Order: "fix them all, remake / rework / remaster". This addendum
is (1) the root-cause diagnosis, traced through the v16 source
with evidence graded as *proven from code* vs *runtime-reported*,
and (2) the v17 rebuild that makes the failure class structurally
impossible to reintroduce silently.

**Baseline re-verified in this session (v16 code, unmodified, in
the v17 tree):** `cargo test --locked --no-default-features` →
**105 passed, 0 failed**; jsdom DOM test → **42/42**. The dead
buttons shipped with a fully green suite — which is itself a
finding (V17-3).

---

## Part 1 — Root-cause diagnosis

### The v16 click path, traced

All four start-page actions were plain anchors in
`assets/start.html`:

| Gesture | Markup (v16 start.html) |
|---|---|
| Recent file *i* | `<a class="recent" href="pico://recent/<i>">` (built by `renderRecents`) |
| File picker | `<a class="action primary" href="pico://pick-file">` |
| Service card | `<a class="card" href="<service URL>">` (http/https) |
| Social chip | `<a class="chip" href="pico://social/<name>">` |

The start page's script contained **zero** other channels: `grep
window.ipc assets/start.html` (v16) → 0 hits, and the page's own
CSP (`connect-src 'none'`) forecloses fetch/XHR. So an action
click was exactly one thing: a top-level navigation attempt. The
intended chain was: click → content WebView's
`with_navigation_handler` (`src/app.rs`, `build_content_view`) →
`is_action_url()` → `UserEvent::OpenAction(url)` (only when the
closure-captured `is_start` was true) → string matching in the
event loop → `open_path` / `open_file_dialog` / `open_social`.
The chain breaks at **two independent points**, either of which
alone kills recent + pick-file completely.

### BREAK A (proven from code) — the transport was never guaranteed: unregistered-scheme anchor navigations

The `pico:` scheme is **never registered with WebView2**. In the
pinned wry 0.53.5 (`src/webview2/mod.rs`):

- `grep CustomSchemeRegistration` over the crate → **0 hits**;
  there is no `CoreWebView2CustomSchemeRegistration` anywhere.
- The custom protocol exists only as a `WebResourceRequested`
  filter for the *workaround* prefix `https://pico.*`
  (`attach_custom_protocol_handler`).
- `apply_uri_work_around` (which rewrites `pico://…` →
  `https://pico.…`) is called exactly once, on the builder's
  initial `with_url` value (the "Navigation" block in `create()`).
  Nothing rewrites — or otherwise intercepts — an **in-page**
  navigation to `pico://…`.
- `add_NavigationStarting` passes the raw URI string to the
  app's handler; whether that event even fires for a navigation
  to a scheme the engine does not know is the engine's choice,
  not a contract anything in the tree establishes.

So a click on `pico://recent/0` asked Chromium to navigate to an
unregistered scheme — the standard handling of which is the
*external-protocol* path (dead end / scheme error inside the
engine), not a document navigation Pico's handler is guaranteed
to see. The design assumed an event the transport never promised.
The user's report is the runtime evidence for what the engine
chose on his machine: total silence — no picker, no tab, no
error — for the `pico://` actions, while ordinary http(s)
navigations (the one transport that indisputably exists) are how
he browses in the same screenshots. Service cards were the only
start-page links whose transport was real, and even they fed
Break B's sibling path only via `NewWindow`; social chips share
the `pico://` transport with recents/pick-file and were equally
exposed.

*Ruled out by the code, for the record:* the
`normalize_pico_url` fold is correct for all four action shapes
(workaround spellings `https://pico.recent/1`,
`https://pico.pick-file/`, `https://pico.social/telegram` all
fold back exactly — now pinned by `tests/action_tests.rs`); the
gate state in v16 was the closure-captured `is_start`, true for
the genuine start tab by construction; the JS emitted exactly
the `pico://recent/<i>` / `pico://pick-file` forms the Rust
string matching expected (no drift); `is_action_url` matches all
three families. Had the event fired with the URL intact, this
half of the chain would have worked. It is the *firing* that was
never guaranteed — hypothesis (1)/(2) of the brief, confirmed as
"the clicks never reliably become handler-visible navigations",
and hypothesis (3)/(4) exonerated.

### BREAK B (proven from code) — `open_path` rejected every regular file (present since v13)

Even a perfectly delivered action died at the last step. v16
`App::open_path` (identical code in v13, v14, v15 — `grep
canonical.join` shows it in all four trees) computed:

```rust
let (folder, rel) = if canonical.is_dir() {
    (canonical.clone(), "index.html".to_string())
} else if let Some(parent) = canonical.parent() {
    (parent.to_path_buf(), /* rel = */ file_name)
} …
if !canonical.join(&rel).is_file() { log "no file at …"; return; }
```

For a **file**, `canonical` is the file itself and `rel` is its
name, so the check tests `<file>/<name>` — a path that is never a
file. Every regular-file open therefore returned before
registering a root or opening a tab: recent entries (files),
picker selections (files), drag-and-drop of a file, argv files,
and address-bar file paths — all of them funnel through
`open_path`. **Directory** opens were unaffected
(`canonical == folder` there), which is why the defect could
hide: the folder flow genuinely worked. The two files in the
user's screenshot ("Good Girl - Jasmine.html", "LLM.html") are
regular files — Break B is sufficient by itself to produce the
report even if Break A had not existed. (Which break fired
*first* on the user's machine cannot be distinguished without
his pico.log: the recent-arm of `OpenAction` had no logging of
its own, and `open_path`'s "no file at" line is the only trace
Break B would have left. Both breaks are fixed; the ambiguity is
recorded, not papered over.)

Picker sub-case (hypothesis 5): `open_file_dialog` itself is a
plain `rfd` call downstream of both breaks; there is no evidence
of a dialog-crate defect, and if the dialog *had* appeared, the
chosen file would still have died in Break B. The native dialog
is the one step that cannot be exercised without a live
WebView2/desktop and remains runtime-witnessed (see residuals).

### V17-3 — Why the green suite could not catch any of this

The untested seam is precisely **page click → (transport) →
Rust decision → `open_path`**, and each test layer stopped at
its own edge:

- The jsdom DOM test asserted what chip clicks *target* (their
  `href` strings) — never what a click *emits* or what Rust does
  with it. Worse, the fixture start page rendered with
  `recents: []` and `services: []`, so **no clickable recent or
  service element existed in the fixture at all**.
- Rust tests covered chrome-command IPC parsing and the address
  allowlist in isolation — pure functions that were all correct.
- The two components containing the breaks — the content
  navigation handler and `open_path` — live in `src/app.rs`,
  which is GUI-only code (`--no-default-features` test runs never
  compile it, and nothing exercised it against a live WebView2).

No layer was wrong about what it tested; the chain between them
was tested by nobody. That is the seam v17 rebuilds and pins.

---

## Part 2 — The v17 rebuild

### V17-1 — One dispatch, two transports, per-action logging (fixes Break A)

- **New pure core module `src/core/action.rs`.** `dispatch(source,
  requester, recents) -> Decision` is the ONLY place a start-page
  action is decided. `Requester` is assembled by the app from its
  own tab state; the gate is conjunctive and checked before any
  parsing: the requesting webview must be the start tab AND that
  tab's current URL must be the internal start page (a start tab
  navigated away loses the privilege; any other tab — even one
  somehow showing the start URL — is denied). Decisions:
  `OpenRecent(path)` (index resolved against the recents list
  here), `OpenPicker`, `OpenService(url)` (navigation-allowlist
  validated), `OpenSocial(site)` (enum-only, as since v15), or
  `Deny(reason)` with a closed reason set
  (`not-start-tab`, `not-on-start-page`, `malformed`,
  `unknown-action`, `recent-out-of-range`, `bad-service-url`,
  `unknown-social-site`).
- **Primary transport is `window.ipc`** — the channel wry defines
  on *every* webview and that chrome commands and content
  navigation notes already prove live on the user's machine. The
  start page's script posts `{kind:"action", action:…}` per click
  (recent: `index`, social: `site`, service: `url`); the content
  IPC handler routes by `kind` (action vs nav note — disjoint).
- **Fallback transport is the legacy anchor navigation**, kept
  deliberately: the anchors' hrefs are byte-identical to v16, and
  when `window.ipc` is unavailable the click is not intercepted,
  so the navigation proceeds and the navigation handler forwards
  it — undecided — into the same `UserEvent::StartAction`. Both
  wry handlers (IPC and navigation) only classify and forward;
  neither contains gate logic. There is no parallel
  implementation left: the v16 event-loop string matching for
  `OpenAction` is deleted.
- **Observability:** `App::on_start_action` logs exactly one line
  per dispatch — `start action: kind=<k> tab=<id>
  decision=<open-recent file=<basename>|open-picker|
  open-service url=<redacted>|open-social site=<url>|deny
  reason=<r>>`. Silent death is no longer a possible outcome: a
  denied or broken action is a log line with a classified reason.
- **Look and scope are unchanged:** same start-page HTML/CSS
  (the only markup delta is `id="pick-file"` on the existing
  button); the v16 header/feather are untouched; security
  semantics preserved exactly (see SECURITY.md's rewritten
  action-dispatch section).

### V17-2 — File-open resolution extracted to a tested core; the join fixed (fixes Break B)

- **New pure-ish core module `src/core/fileopen.rs`:**
  `resolve_open_target(path)` performs the lexical UNC gate
  (before any filesystem call, as always), canonicalization, the
  folder/rel split, and the existence check — now correctly
  `folder.join(&rel)`, so a file resolves to *itself*.
  `App::open_path` delegates to it and keeps its existing error
  log lines per error kind.
- Every entry point inherits the fix because every one of them
  already funnels through `open_path`: recents (via dispatch),
  picker (via dispatch and via the chrome's OpenFile command),
  drag-and-drop (`DroppedPaths`), argv (`open_target`), and
  address-bar file paths (`navigate_active` → `Target::File`).
- *Stated micro-change:* opening a directory now records the
  directory's `index.html` path in recents (v16 recorded the
  directory itself). The recent entry opens the identical page;
  its label is the served file's name.
- Regression pins run against real files on disk
  (`tests/action_tests.rs`): a plain file resolves to itself
  (fails under the v13–v16 expression), a directory resolves to
  its `index.html`, a directory without one is `MissingFile`, a
  missing path is `CannotResolve`, and UNC spellings are refused
  lexically before any filesystem touch.

### V17-3 — The seam is now covered at both ends

- `tests/action_tests.rs` (18 tests): the full dispatch matrix —
  requesters {start tab @ start page, start tab @ website, other
  tab @ website, other tab @ start page} × actions {recent
  valid / out-of-range / malformed / huge / traversal-flavored,
  pick-file, service valid / `javascript:` / `data:` / `file:` /
  `ftp:`, social telegram / youtube / unknown / wrong-case} ×
  both transports, plus gate-before-parse ordering and the
  fileopen pins above.
- `scripts/dom_test.mjs` (52 checks, +10): the fixture start page
  now renders two recents and a service card
  (`examples/dump_pages.rs`), `window.ipc` is stubbed as wry
  provides it, and clicking each control asserts the *exact* IPC
  request emitted — recent `{action:"recent", index:1}`,
  pick-file, social enum-only, service URL — plus that the click
  was intercepted (default prevented), so the fallback navigation
  does not also fire.
- What remains untestable here, stated plainly: the native
  `rfd` dialog itself and WebView2's delivery of the IPC message
  on the user's machine. Everything up to the dialog call and
  everything after it is covered; the on-device witness is now
  the per-action dispatch line in pico.log.

### Standing sweep on the changed tree (regression check)

- All 105 v16 Rust tests and 42 DOM checks carried (version
  strings updated); totals are now **123 Rust + 52 DOM**.
- No changes to: server, detector, state, layout/geometry, IPC
  command protocol, chrome construction, watchdog, dedupe
  retirement, scheme allowlist, Host wall, download denial.
  The `NewWindow` path still refuses action URLs outright
  (window.open is not an action transport).
- One new `UserEvent` variant replaces `OpenAction`; no new
  unsafe (inventory stays 9); no new dependencies.

---

# Pico v16 — Addendum: Header redesign + full scan of the changed tree

v16 implements the user's header redesign (from their v15
screenshots): the standalone titlebar row is gone; the brand block
("Pico by @iK4w4"), the tab strip and the window controls share one
40 DIP row over a slimmed 46 DIP address row — 86 DIP of chrome
total, down from 142 — and the chrome's bottom 12 DIP is a
transparent feather strip that overlaps the content, so the header
melts into the page. This addendum records the redesign's own
defect scan (performed on the changed tree, standing sweep
re-run), what was found, and what was done about it.

**Baseline:** the v15 tree was copied to pico-v16 and its suite
re-run before any change: 102/103 Rust tests passed and the DOM
baseline was not re-run (the v15 DOM run is on record). The one
failure was `connection_cap_sheds_load_and_recovers`, root-caused
below as a pre-existing test-side race (V16-3) — not a server
defect — and hardened as part of this pass.

---

## Medium

### V16-1 — Content HWNDs would paint over the feather (sibling z-order)

*Evidence:* the feather design requires the chrome WebView to
composite *over* the content in the 12-DIP overlap rows — the
content's own top rows are opaque page pixels, so whichever HWND
is higher in the sibling z-order wins those rows. Win32 orders
sibling child windows by creation (later = higher); the chrome is
built *first* (before any tab), so every tab's container HWND
lands above it, and the chrome's gradient would be painted over
by the page — the blend silently absent, no error anywhere (the
failure shape this project hates most). wry 0.53.5 exposes no
z-order or HWND accessor for a WebView.
*Disposition:* **Fixed in v16** — `raise_chrome_above_content`
(app.rs) fetches the chrome controller's parent HWND via
`ICoreWebView2Controller::ParentWindow` (the same raw-interface
route as the profile hardening) and re-asserts it at the top of
the sibling z-order with
`SetWindowPos(HWND_TOP, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` on
every layout pass — startup, tab create/activate/close, resize,
DPI change, and watchdog recreation all funnel through
`App::layout`. Failure is benign and ignored (worst case is the
pre-fix behavior). One new `unsafe` block (9 total in `src/`, up
from 8 — a single handle-only Win32 call, justified here and in
SECURITY.md).

### V16-2 — Transparent chrome vs. the v14 explicit-background rule (and the theme-toggle trap)

*Evidence:* the feather requires the chrome controller's
background to be transparent — a direct, deliberate exception to
v14's "every surface gets an explicit background" rule
(AUDIT V14-2). Two sub-findings from reading the pinned wry
0.53.5 source: (a) `with_transparent(true)` is fully supported by
the WebView2 backend (it passes `(0,0,0,0)` as the controller's
default background color at creation), so no fallback design was
needed; (b) wry's runtime `set_background_color` *forces alpha
to 255* for any non-zero alpha — and v15's `toggle_theme` called
it on the chrome every toggle. Carried into v16 unchanged, the
first theme toggle would have silently turned the chrome opaque
and killed the feather for the rest of the session.
*Disposition:* **Exception accepted, documented, and fenced** —
the transparency is confined to the chrome; the tao window behind
it keeps the explicit themed background, so a failed/unpainted
chrome document shows the theme color, never black; the
handshake, watchdog, warn strip and hidden-until-ready apparatus
is untouched; content views keep explicit backgrounds. The
`toggle_theme` call on the chrome is removed (the document
repaints from theme tokens; the window repaints behind it), with
the reason recorded at the call site. `build_chrome` remains the
single construction path, so startup and watchdog recreation are
identically transparent. Full rationale in SECURITY.md
("Transparent chrome").

## Low

### V16-3 — Flaky connection-cap test (pre-existing, test-side race)

*Evidence:* reproduced on the *unmodified* v15 baseline in the
v16 tree (failed 1 of 3 runs): `connection_cap_sheds_load_and_recovers`
asserted the 65th connection's `read()` returns Err. The server
sheds load by accepting and immediately dropping, and the
observable refusal shape depends on packet timing: RST when the
probe's request bytes beat the drop (close with unread data),
clean EOF (Ok(0)) when they don't, or a read timeout when the
probe never leaves the listen backlog. Only two of the three
correct behaviors satisfy `is_err()`. The server's cap behavior
itself was verified correct in all runs (the 65th is never
served; recovery to 200 after the flood drains held every time).
*Disposition:* **Fixed in v16** — the test now asserts the real
invariant (the 65th connection receives *zero response bytes*,
across however the refusal arrives) and keeps the recovery half.
Five consecutive full server-suite runs are green.

### V16-4 — Merged-row overflow: flex floor could push brand/controls off-screen

*Evidence:* in the merged row the tab strip sits between two
fixed elements. Flex items default to `min-width: auto` (their
content floor), so a strip full of tabs could refuse to shrink
and shove the window controls — or be shoved — outside the row.
*Disposition:* **Fixed and defined in v16** — the strip carries
`min-width: 0` and scrolls internally (`overflow-x: auto`,
scrollbar hidden); brand block and window controls are
`flex: none`. Defined overflow behavior: tabs shrink to their
100 DIP floor, then the strip scrolls/clips horizontally; brand
and controls never move, at any tab count up to the 30-tab cap,
at any window width down to the 760 DIP minimum. The min-width
math (BUILD_NOTES) shows brand ≈170 + one floored tab 100 + "+"
30 + controls ≈114 + padding ≈ 440 DIP worst case — comfortably
inside 760, so the minimum window size is **unchanged**. The
`min-width: 0` opt-out is pinned by
`header_structure_is_the_v16_contract`.

## Info — verified by the standing sweep on the changed tree

### V16-5 — Geometry has exactly one source of truth

The snapshot carries **no** chrome height (checked field by
field) — the JS cannot disagree with Rust about the geometry.
Heights exist only as `core::layout::{CHROME_H 86, SOLID_H 74,
FEATHER_H 12 (derived)}` on the Rust side and the matching CSS
numbers in chrome.html; the new asset test derives its CSS
expectations from the Rust constants (`height: 86px`,
`height: 12px`, the navbar's `SOLID_H − 40 = 34px` solid stop), so
the two sides cannot drift silently. The old numbers (142 /
40+46+56) are fully swept from shipped code and pages (grep).

### V16-6 — Hit targets stay ≥28 DIP at the new sizes

Win buttons 34×30 (were 34×26 — raised to clear the floor), nav
buttons 30×30, new-tab 30×30, address input 32 high, tab close
28×28 (was 19×19), byline ≈29 tall in-row (padding raised). Tabs
are 34 tall in the 40 row. No text clips at scale 1.0–2.0: the CSS
is in DIP throughout and the layout tests pin the physical
geometry at all five scale factors, including the feather
overlap being exactly round(12·sf) at each.

### V16-7 — Everything else re-verified unchanged

- **Shell apparatus:** handshake, watchdog (with V15-1 generation
  retirement), warn strip (now a top overlay — DOM-verified to
  sit at `top: 0`, `position: fixed`, so it can never reflow the
  86 DIP geometry), hidden-until-ready, native escalation: all
  present, all still wired through the single `build_chrome`.
- **IPC/dedupe:** no new commands; `win_drag`/`win_max` meanings
  unchanged (the drag source moved from a dedicated div to the
  top row's non-control areas — DOM-tested: strip drags, controls
  don't, dblclick maximizes).
- **Badges** moved from the old titlebar into the address row's
  right end (same ids, same snapshot fields, same behavior) —
  the only home left for them once the titlebar row was deleted.
- **Server, detector, state, log redaction:** untouched code,
  suites green (23 server, 21 detector, 17 history/state, 6
  redact). The one new log line (`layout: chrome_h=86
  feather=overlap`) contains no user data.
- **Disclosed trade-off:** pointer/wheel input over the content's
  top 12 DIP is absorbed by the chrome HWND — inherent to the
  mandated overlap (sibling HWNDs don't forward input through
  transparent regions). Documented in SECURITY.md.

---

# Pico v15 — Addendum: Full scan of Pico v14

Complete re-scan of the shipped v14 tree (`~/workspace/pico-v14/`,
user-confirmed working on real Windows: "ok its working",
2026-10-24), performed before any v15 change. Nothing in v14's
AUDIT.md / SECURITY.md / BUILD_NOTES.md was taken on trust: every
claim below was re-derived from source, grep, or a test run in
this session.

**Baseline re-verified in this session (v14 code, unmodified, in
the v15 tree):**
- `cargo test --locked --no-default-features` → **98 passed,
  0 failed** (lib 10, addr_ipc 19, assets 3, detect 21,
  history_state 17, redact 6, server 22) — matches v14's claim.
- jsdom DOM test → **20/20 checks pass** (pages emitted by
  `examples/dump_pages.rs`) — matches v14's claim.
- `cargo clippy --locked --no-default-features --all-targets` →
  **0 warnings**; `cargo fmt --check` clean; **zero**
  `unwrap()`/`expect()`/`panic!` in shipped `src/`; unsafe
  inventory unchanged (8 blocks: 7 in `src/app.rs`, 1 in
  `src/dpi.rs`, all Windows-API).

Severity scale and dispositions as below (Fixed in v15 /
Retained / Residual / Verified-not-a-defect).

---

## Medium

### V15-1 — Commands from a destroyed chrome generation are still accepted after watchdog recreation

*Evidence:* `UserEvent::ChromeCmd(String)` carries no generation
tag, and `Dedupe::check` (v14 `src/core/ipc.rs`) adopts *any* boot
id it has not seen, clearing its seen-set in the process. The v14
watchdog destroys a hung chrome WebView and builds a new one, but
commands the old page already queued into the event loop can be
delivered *after* the replacement boots. Two failure shapes:
(a) a stale `ready` from generation 1 is processed after
recreation → `on_chrome_cmd` sets `chrome_ready = true`, so
generation 2's watchdog stands down even if generation 2 is itself
dead — the silent-black-shell class v14 exists to eliminate,
re-entering through the recovery path; (b) any other stale command
(a queued `win_close`, `navigate`, `close_tab`, …) executes
against the new shell, and its boot switch wipes the new
generation's dedupe state.
*Window:* narrow — the stale message must still be queued when the
watchdog fires, which in practice requires the event loop to have
been stalled for the watchdog's 2.5 s while the page was alive
enough to send. Rated Medium (not High) for that reason; the
missing generation binding is nevertheless a real structural gap
in the shell-integrity design.
*Disposition:* **Fixed in v15** — `Dedupe` gains a bounded retired
set (16 boots). `retire_current_boot()` is called by the watchdog
*before* the destroyed WebView is replaced; `check()` drops any
envelope whose boot is retired, without touching the live
generation's state. Pinned by
`dedupe_retired_boot_is_dropped_forever` (stale ready dropped,
stale win_close dropped, gen-2 dedupe state provably survives
further stale traffic). *Residual, stated honestly:* a command
from a generation whose boot was never registered (no command of
its was ever processed before destruction) cannot be distinguished
from the new generation's first command by boot id alone; the
preconditions above (a >2.5 s event-loop stall during startup)
still bound that case, and the second watchdog miss still
escalates to the native dialog.

## Low

### V15-2 — `normalize_pico_url` folds real `https://pico.*` websites into the internal scheme

*Evidence:* v14 `src/app.rs` folded *any* URL starting
`https://pico.` / `http://pico.` to `pico://…`. That fold exists
for wry's WebView2 rewrite of Pico's own scheme, but it also hits
genuine web addresses: a content navigation to
`https://pico.dev/` (or any `pico.*` domain) was rewritten to
`pico://dev/` and then refused by the navigation handler's
`!url.starts_with("pico://")` rule; `window.open` to such a domain
was mangled the same way, and a page-load for one would have
recorded a `pico://` URL into tab state. Fail-closed throughout
(no security impact), but an entire class of real domains was
unbrowsable.
*Disposition:* **Fixed in v15** — the function moved to
`pico_core::core::addr` (so it is host-testable) and now folds
only Pico's own internal hosts — `chrome`, `start`, `recent`,
`social`, `pick-*`, all single DNS labels with no dot. A dotted
host is a real web address and passes through unchanged.
Pinned by `normalize_folds_only_internal_hosts` (internal folds,
real pico.* sites untouched, `pico.chrome.evil.example` never
folds into the chrome document).

### V15-3 — Version badge flips between "v14" and "v14.0.0"

*Evidence:* the static chrome badge reads `v14`; the snapshot
handler set it to `"v" + s.version` where `s.version` is the full
Cargo semver → after the first snapshot the badge read `v14.0.0`.
The v14 DOM test even asserted the flip. Pure polish, but the
badge is the user's version witness (and this user has caught a
stuck version number before).
*Disposition:* **Fixed in v15** — the badge renders the major
version only, so static and snapshot-driven states agree on
`v15`. DOM test updated to assert it stays `v15` after a snapshot
carrying `15.0.0`.

### V15-4 — Chrome WebView has no explicit new-window handler

*Evidence:* content WebViews install
`with_new_window_req_handler` (popups → new tab, response Deny);
`build_chrome` installed none, so the shell relied on wry/WebView2
defaults for `window.open`. The chrome page has no window.open
surface today (no links, no script calls it), so this is
defense-in-depth, not a live hole.
*Disposition:* **Fixed in v15** — `build_chrome` now denies
new-window requests explicitly and logs the (normalized, redacted)
URL. Because `build_chrome` is the single construction path, the
watchdog's recreated chrome gets the identical handler.

### V15-5 — Stale version references in the tree

*Evidence:* `Cargo.toml` package comment said "Pico v13 must link
with an EMPTY resource directory" (in the *v14* tree); test/bench
temp-dir prefixes said `pico-v13-*` / `pico-v14-*`; package name
was `pico-v14`.
*Disposition:* **Fixed in v15** — swept to v15 (package
`pico-v15`, version 15.0.0; the Cargo comment is now
version-neutral). Historical references inside audit narratives
(this file's older addenda, "introduced in v14" comments) are
deliberately kept — they are history, not version claims.

## Info — verified not a defect (the mandated sweep, re-run)

### V15-6 — Chrome navigation allowlist and protocol router resist the trick-URL matrix

Re-checked by source analysis against the mandated matrix, both
surfaces: the nav handler allows only exact equality with
`pico://chrome` / `pico://chrome/` *after* normalization; the
protocol router strips `?`/`#`, removes at most one trailing `/`,
and requires exact equality with the two document routes.
Case variations (`PICO://CHROME`), suffix hosts
(`pico://chrome.evil`), userinfo (`pico://chrome@evil`),
percent-encoded hosts, extra path segments, double trailing
slashes (`pico://chrome//` → 404), and query/fragment forms all
fail closed in both. With V15-2's tighter fold, a web page can no
longer even produce the rewritten spelling of an internal
document. **No bypass found — no Critical in this scan.**

### V15-7 — `build_chrome` is the single construction path; recreation parity holds

Startup and the watchdog both call `build_chrome` and then
`harden_webview_profile`. Inside the builder: the custom protocol
handler, the exact-match navigation handler, the download-denial
handler (tab 0 accounting), devtools, the themed background, and
(now, V15-4) the new-window denial. The recreated chrome is built
with the *current* theme and current bounds; no first-build/rebuild
drift found. Theme-toggle parity also verified: the toggle updates
the tao window, the chrome, and every tab view, and views created
later are built with the persisted theme. Layout parity: `Resized`
and `ScaleFactorChanged` both re-run the single device-pixel
tiling; maximize state rides the snapshot.

### V15-8 — The rest of the v14 register re-verified in place

- **Host wall (server):** duplicate `Host` → 400, absolute-form →
  400, network-path target → 400, obs-fold → 400, foreign/wrong-port
  /userinfo/whitespace hosts refused. v15 adds the parser-level
  matrix `host_wall_parser_matrix` to the existing wire tests.
- **Path policy / UNC:** the 13-case traversal corpus,
  decode-once normalization and canonical containment unchanged
  and green; `\\?\` is accepted only in verbatim-drive form,
  `\\.\` and NT-namespace spellings refused lexically before any
  filesystem touch (`unc_paths_are_refused_as_input`).
- **Start-tab-only actions after reorder/close:** the gate is a
  property of the *webview* (`is_start` captured in
  `build_content_view`'s handler), not of tab position — closing or
  reordering tabs cannot move the privilege to another webview;
  `UserEvent::NewWindow` additionally refuses action URLs outright.
- **Downloads:** denied on chrome (tab 0) and every content view,
  including retry-rebuilt tab views (same builder) and the
  recreated chrome (same `build_chrome`).
- **Detector qualification:** re-read `qualify()` /
  `clean_title()` / `parse_title()` — boilerplate exclusion runs
  after status-prefix stripping, entities (named + numeric) are
  decoded, a real product name like "500px" survives the status
  heuristic (the digits-only follow-up check), TLS qualifies only
  via dev process/port and never yields a title. The v14 fixtures
  are green unmodified.
- **State v2, log rotation/redaction, per-tab containment:** code
  re-read; suites green; no new gaps found. One retained nuance:
  rejected chrome IPC bodies are logged (redacted, truncated,
  control-stripped) — a *typed* navigate text is user input, in
  scope for the diagnostic log by design (AUDIT M5 covered
  page-shaped URLs).
- **Dedupe eviction window:** the 512-entry FIFO can re-accept an
  ancient evicted `(boot, req)` — pre-existing, asserted as
  by-design in `dedupe_stays_bounded`; the only sender is Pico's
  own chrome over its webview-bound IPC channel. **Retained.**

---

# Pico v14 — Addendum: Audit of Pico v13 (the black-chrome failure)

Forensic diagnosis of the v13 runtime failure reported from the
user's Windows (screenshot, 2026-10-24), performed from source
before the v14 rework. The v13 audit of v12.1 follows below,
unchanged, as the historical record.

**The failure.** v13 launches on real Windows and the start page
renders correctly, but the chrome region (the top band, CHROME_H
142 logical px ≈ 178 physical at the user's 125% DPI) is **solid
black** — no title bar, no tabs, no address bar, no window
controls, no error, no fallback. The same screenshot shows black
gaps at the window/content edges, and three junk detector cards:
Discord.exe's RPC port (:6463), an unidentified HTTPS port
(:19206), and :13508 titled literally "Not Found".

Severity scale and dispositions as below (Fixed in v14 /
Retained / Residual).

---

## Critical

### V14-1 — The chrome's navigation handler cancels the chrome's own initial load

*Mechanism (proven from source, not inferred):* v13's chrome
builder (`src/app.rs`, chrome creation in `run_with_server`) added
`.with_navigation_handler(|_url: String| false)` — deny-all — with
the comment "the chrome must never navigate anywhere". **v12's
working chrome builder has no navigation handler at all** (v12
`src/app.rs` chrome creation: url + protocol + IPC + bounds +
visible only). In the vendored wry 0.53.5 source
(`src/webview2/mod.rs`, `attach_handlers`), the navigation handler
is registered on WebView2's `NavigationStarting` event and calls
`args.SetCancel(!allow)` for **every** navigation — and
`NavigationStarting` fires for the initial `with_url("pico://chrome")`
load too. The handler returns `false`, so the very first navigation
is cancelled: the chrome document **never loads**. Content WebViews
use a permissive handler (`is_allowed_navigation_url(...)`), which
is why pages — including the start page in the same screenshot —
render fine while the shell is a void.

*Why black, specifically:* neither the chrome WebView, nor the
content WebViews, nor the tao window had an explicit background
color anywhere in v13. An unpainted/cancelled WebView2 surface and
an uncovered window area present as black on Windows — the chrome
band and the edge gaps in the screenshot are exactly the surfaces
nothing ever painted.

*Why silent:* chrome **creation** succeeded, so the
`fatal_webview` path (which only covers builder errors) never
fired. There was no other tripwire: `Command::Ready` was handled
but never recorded; `push_state` discards `evaluate_script` errors
(`let _ =`), so snapshots pushed into the dead page vanished
without a trace; the chrome had no page-load handler, no handshake
bookkeeping, no watchdog, and the in-page skeleton — which v13's
chrome.html *does* contain in full (static titlebar/tabstrip/
navbar HTML) — never got a chance to render because the document
itself was cancelled.

*Exonerated suspects:* the shared-CSS placeholder substitution
(`replacen("/*__SHARED_CSS__*/", …)`) is byte-identical in v12 and
v13 and demonstrably worked in v13 — the start page in the same
screenshot is fully styled. Bounds/z-order were also fine: the
black band matches the chrome rect exactly, i.e. the right HWND in
the right place, showing nothing.

*Secondary v12→v13 chrome deltas:* v13 dropped the chrome's
download-started handler that v12 had (restored in v14 for parity:
deny + count, like content views); v13 added `.with_devtools(true)`
to the chrome (harmless, retained).

*Disposition:* **Fixed in v14** — the chrome navigation handler now
allows exactly the chrome document (both spellings after
`normalize_pico_url`: `pico://chrome` and `pico://chrome/`) and
denies everything else. Chrome construction is a single function
(`build_chrome`) shared by startup and by the watchdog's
recreation, so the two paths can never drift apart again.

### V14-2 — No explicit background color on any surface

*Evidence:* grep of v13 `src/app.rs` — no `with_background_color`,
no `set_background_color`, no tao window background. Every surface
relied on "the page will paint soon" as its only appearance plan.
*Disposition:* **Fixed in v14** — the tao window and every WebView
(chrome + content) get an explicit background equal to the current
theme surface (`--bg`: light `#dde3ec`, dark `#212429`), set at
creation and updated on theme toggle. A surface that has not
painted yet — or that fails — now presents as an empty themed
surface, never black. The failure *class* "unpainted surface reads
as a black void" is eliminated by construction, independent of
which specific load failure occurs.

### V14-3 — The shell had no health signal: no handshake record, no watchdog, no escalation

*Evidence:* v13 handled `Command::Ready` by pushing state and
forgetting it happened; nothing logged chrome creation success
beyond one line, nothing logged whether a snapshot was ever
delivered, and nothing would ever have noticed a chrome that
created successfully but never came up (exactly V14-1's shape).
*Disposition:* **Fixed in v14** —
- The `ready` handshake is recorded; pico.log now unconditionally
  logs chrome creation, the handshake ("chrome handshake received
  (generation N)"), and first-snapshot delivery, so the log always
  answers "did the shell come up?".
- A one-shot watchdog fires ~2.5 s after each chrome creation
  (timer thread → `UserEvent::ChromeWatchdog(gen)`; the generation
  counter makes stale timers harmless). Missing handshake → ERROR
  log + the chrome WebView is destroyed and recreated **once**
  through the same `build_chrome` path. Second miss → ERROR log +
  a native dialog naming the problem and the pico.log path.
- In-page self-diagnosis: the chrome page itself shows a slim red
  strip ("Shell didn't connect — restart Pico; details in
  pico.log") if no snapshot arrives within ~2 s of load, using only
  inline JS/CSS. Even with IPC completely dead, the user sees an
  explanation instead of a void.

### V14-4 — Layout math rounded logical floats independently per surface; window visible during construction

*Evidence:* v13 computed chrome and content rects separately from
logical floats (each `set_bounds` re-divided the physical size by
the scale factor), so device-pixel rounding could leave a 1-px
unpainted seam or overlap at fractional DPI (the screenshot's edge
gaps at 125%); and the window was created visible, so construction
itself could flash unpainted frames.
*Disposition:* **Fixed in v14** — one conversion site:
`pico_core::core::layout::tile()` partitions the client area in
**device pixels** (chrome height rounded once; content top ==
chrome bottom by construction), unit-tested across scale factors
1.0/1.25/1.5/1.75/2.0 × odd window sizes with exact-partition
assertions. The window is created hidden and shown only after the
chrome and first tab exist and are laid out (sequence logged).

## High

### V14-5 — Detector precision: nearly any answering listener became a start-page card

*Evidence:* v13's `scan_services` pushed a card for **every**
listener whose probe classified HTTP or TLS. That is how the
screenshot got: Discord's RPC endpoint (:6463 — an HTTP+JSON API
owned by a chat app), an anonymous TLS listener (:19206 — v13
labeled every TLS speaker "HTTPS service"), and :13508, a plain
404 whose boilerplate `<title>Not Found</title>` v13 happily used
as the card's name. Recall was total; precision was accidental.
*Disposition:* **Fixed in v14** — a qualification bar in the
detector core (`qualify()` in `src/core/detect.rs`). A listener
becomes a card only if at least one holds:
(a) **TITLE** — an HTML response with a real `<title>` (cleaned:
entities decoded via the existing parser, whitespace collapsed,
control chars stripped, max 80 chars; boilerplate error titles —
"Not Found", "404 Not Found", "Internal Server Error", etc. —
count as NO title; titles never come from the status line or
plain-text bodies);
(b) **SIGNATURE** — a known dev-server signature in headers/body
(Vite, Next.js, webpack-dev-server, Angular, Django,
Werkzeug/Flask, PHP built-in, Python http.server, Live Server,
Hugo, Astro, SvelteKit);
(c) **DEV PROCESS** — process name is a dev runtime/tool (node,
python/pythonw, deno, bun, php, ruby, dotnet, java, hugo, air,
cargo) AND the probe got a syntactically valid HTTP response;
(d) **DEV PORT** — port in the curated dev set AND a valid HTTP
response with HTML content.
TLS listeners qualify only via (c) or (d) with handshake success;
label "HTTPS service"; never a title. Everything else is NOT a
card: it is counted and surfaced as one aggregate
(`other_listeners: N` in the snapshot and start-page init),
rendered as a single muted line "N other local listeners (not dev
servers)". Cards sort titled-first (by port), then untitled.
TTL caching, parallel probes (≤8, 300 ms), TLS classification, and
(port, pid) dedupe are retained, applied to the qualified set.

*Regression fixtures (tests/detect_tests.rs, stub servers, process
names injected via the detector's `name_of` closure):* plain-text
404 + unknown process → aggregate only; HTML 404 with
`<title>My App</title>` → card "My App"; boilerplate-titled 404 →
aggregate only; JSON API + node → card labeled from the process,
no title; JSON API + Discord.exe → aggregate only; reason-phrase
"Weird Title" → never a title; Werkzeug signature → card;
titled-before-untitled sort. The python http.server fixture still
detects (real title).

*Test changes justified by this redesign:*
- `scan_services` now returns `ScanOutcome { cards,
  other_listeners }` instead of `Vec<Service>`, and `signature()`
  takes the outcome (the aggregate count is part of the signature
  so the muted line stays fresh). All scan call sites updated.
- The two v13 TLS stub tests asserted an anonymous TLS listener on
  a random port becomes a card — that behavior IS the bug (the
  screenshot's ":19206"). They now inject a dev process name
  (node) so the listener qualifies via (c), and a new test asserts
  the anonymous case lands in the aggregate.
- `silent_listener_is_not_a_service` additionally asserts the
  listener is counted in `other_listeners`.

## Info

### V14-6 — Page assembly was untested string surgery in the GUI layer

*Evidence:* v13 assembled both embedded pages inside `app.rs` via
`replacen` on include strings; no test ever looked at the final
bytes.
*Disposition:* **Fixed in v14** — assembly moved to the core
library (`pico_core::core::assets::assemble_chrome_html()` /
`assemble_start_html()`, including the `<\/` script-breakout
neutralization), and `tests/assets_tests.rs` asserts on the final
strings: size, no surviving tokens, `--bg:` present, `</html>`,
every required control id, the shellwarn strip and snapshot flag,
the script-breakout neutralization, plus `node --check` on every
extracted inline script. A jsdom DOM test
(`scripts/dom_test.mjs`, pages emitted by
`examples/dump_pages.rs`) loads the assembled chrome, fires a
canned snapshot, and asserts tabs/address/theme/services render, a
poisoned `<img onerror>` title stays inert text, the warn strip
stays hidden after a snapshot — and appears, naming pico.log, when
no snapshot ever arrives. All 20 DOM checks pass (see
BUILD_NOTES.md).

---

# Pico v13 — Deep Audit of Pico v12.1

Forensic re-scan of the shipped v12.1 tree (`~/workspace/pico-v12/`),
performed before the v13 rewrite. Nothing in v12.1's SECURITY.md or
BUILD_NOTES.md was taken on trust: every claim below was re-derived
from source, grep, or a test run in this session.

**Baseline re-verified in this session (v12.1, unmodified tree):**
- `cargo test --locked --no-default-features` → **39 passed, 0 failed** (matches claim).
- `cargo clippy --locked --no-default-features --all-targets` → **0 warnings** (matches claim).
- `grep -c 'unwrap()/expect(' src/**` → **0** in shipped non-test code (matches claim; `unwrap_or_else` poison fallbacks only).
- No-manifest structure, static CRT, closed scope — all as documented.

Severity scale: Critical / High / Medium / Low / Info.
Disposition: **Fixed in v13** (with the mechanism), **Retained**
(deliberate, rationale given), or **Residual** (accepted risk, also
carried into v13's SECURITY.md).

---

## Medium

### M1 — Live-reload SSE is global: editing project A reloads project B's pages
*Evidence:* `src/core/server.rs` — `RELOAD_SCRIPT` connects every
served page to the single global `/__pico/events`; `ReloadHub` is one
fan-out for the whole server; `App`'s watcher callback broadcasts on
that one hub. The Rust-side fallback in `UserEvent::FilesChanged` is
root-aware, but the primary SSE path is not.
*Impact:* with two projects open, saving a file in project A reloads
the page showing project B — losing its scroll position, form state,
and SPA state. Wrong-project reloads are a correctness bug in the
headline live-reload feature.
*Disposition:* **Fixed in v13** — reload channels are scoped per root:
the injected script subscribes to `/__pico/events/<root-token>` and
the watcher broadcasts only to that root's subscribers. The global
endpoint is gone.

### M2 — Served roots and watchers are never released
*Evidence:* `src/core/server.rs` has no unregister API (grep: no
`remove`/`unregister` on roots); `src/app.rs` `watchers: HashMap` only
ever inserts. `FileServer::register_root` dedupes by path and keeps
the root (and its random token URL) valid for the process lifetime;
`ensure_watcher` likewise.
*Impact:* (a) resource leak — every folder ever opened stays watched
(recursive notify watchers are expensive: one inotify/FSEvents handle
per directory) and stays *servable* — a token URL captured earlier
(e.g. from pico.log or browser state) keeps working after the user
believes the project is "closed"; (b) watcher storms from a forgotten
watched tree (e.g. a build directory) keep firing reloads.
*Disposition:* **Fixed in v13** — roots are reference-counted by open
tabs; when the last tab showing a root closes (or navigates away),
the root is unregistered from the server and its watcher dropped.
The server gains `unregister_root`, the hub drops that root's SSE
subscribers, and a lifecycle integration test proves URLs 404 after
release.

### M3 — No HTTP Range support: local media cannot seek, some players fail
*Evidence:* grep for `Range` in `src/core/server.rs` → zero hits;
`respond_stream` always answers `200` with the full body.
*Impact:* Chromium-based players issue `Range:` requests for
`<video>`/`<audio>` served from a Pico root; without `206` +
`Content-Range` + `Accept-Ranges`, seeking is broken and some
players refuse playback. Local media preview is squarely inside
Pico's local-development scope.
*Disposition:* **Fixed in v13** — single-range support
(`bytes=a-b`, `bytes=a-`, `bytes=-n`) with 206/416 semantics,
applied to streamed bodies; malformed and multi-range requests fall
back to a plain 200 full-body response. Range requests suppress
reload-script injection decisions consistently (HTML is never
range-served from a partial body).

### M4 — No HTTP keep-alive: one TCP connection per request
*Evidence:* every response in `handle_conn`/`respond_*` carries
`Connection: close` and the handler returns after a single request.
*Impact:* a dev page with 100+ assets burns 100+ sequential localhost
connections (each a thread spawn under the 64-connection gate);
asset-heavy pages load measurably slower and can starve the gate
while SSE streams hold slots. Correct but needlessly slow — the
"500× better" bar includes not wasting the loopback.
*Disposition:* **Fixed in v13** — HTTP/1.1 keep-alive with a bounded
requests-per-connection cap and an idle timeout; `Connection: close`
still honored on request; connection accounting unchanged (a
keep-alive connection holds one permit for its lifetime, SSE
included).

### M5 — Log privacy: full URLs (query + fragment + root tokens) written to pico.log
*Evidence:* `src/app.rs` logs raw URLs in `on_nav_note`
("nav note dropped (cross-origin): {tab.url} -> {note.href}"),
`on_page_load` ("page load outside allowlist ignored: {url}"),
`add_tab` refusal, `UserEvent::NewWindow` refusal, and
`DownloadBlocked` (full URI). v12.1's log sanitizer strips control
characters only.
*Impact:* localhost dev URLs routinely carry secrets in query strings
(OAuth codes during dev login flows, API keys, session tokens) and
the static-server URL itself embeds the root token. pico.log is a
plain file under `%APPDATA%` that the user is routinely asked to
share for debugging — the log leaks exactly what the token design
tries to protect.
*Disposition:* **Fixed in v13** — a pure `redact_url` in the core:
scheme + host + port + path survive; query and fragment are replaced
with `?…`/`#…` markers (presence visible, contents never logged);
non-URL strings pass through a control-stripping cap. Every log call
that can carry a URL goes through it; a unit test feeds token-bearing
and credential-bearing URLs and asserts the secrets never appear.

### M6 — One tab's WebView2 creation failure kills the whole app
*Evidence:* `src/app.rs` `add_tab`: `Err(e) => fatal_webview(...)`
which shows a dialog and `std::process::exit(1)`. Any transient
per-instance WebView2 failure (profile lock contention, a hiccup
after many tabs) destroys every other tab.
*Impact:* robustness — a single bad tab creation is unrecoverable
data loss for the session.
*Disposition:* **Fixed in v13** — content-tab creation failure is
per-tab: the tab is created in a failed state showing an inline error
page (served via the internal protocol) with the reason, the app
stays alive, and the tab can be closed/reloaded. Only chrome
creation failure remains fatal (there is no UI without it), and it
keeps the native-dialog + log behavior.

### M7 — Detector caches can go permanently stale for a live listener
*Evidence:* `src/core/detect.rs` — `TitleCache` entries (`Http` and
`NotHttp`) are retained as long as `(port, pid)` remains in the
target set and are *never* re-probed (SECURITY.md F4 documents the
no-refetch design).
*Impact:* two real cases: (1) a process opens its listen socket
before its HTTP stack is ready (framework boot) → cached `NotHttp`
forever, card never appears; (2) a dev server's `<title>` changes
after boot (common with SSR frameworks) → card shows the boot title
forever. Both are silent wrongness in the flagship detector feature.
*Disposition:* **Fixed in v13** — cache entries carry timestamps:
`NotHttp` expires after 15 s, `Http` titles are refreshed after
120 s. Refetch stays bounded (same parallel cap) and a counting-stub
test proves unchanged services are *not* refetched inside the TTL.

### M8 — TLS-only local services are invisible to the detector
*Evidence:* `probe_http` speaks plaintext HTTP only; an HTTPS-only
dev server (mkcert/local-HTTPS setups, `dotnet dev-certs`, some
frameworks) accepts the TCP connection, sees a plaintext GET, and
resets or hangs → `NotHttp` → no card.
*Impact:* a whole class of local dev servers never appears on the
start page, with no explanation.
*Disposition:* **Fixed in v13** — when the plaintext probe fails, a
bounded TLS-handshake probe sends a minimal ClientHello and
classifies the listener as HTTPS when the reply is a TLS handshake
record (0x16). The card links `https://localhost:<port>` with label
"HTTPS service". Validation is *not* weakened: Pico performs no
title fetch over TLS and stores nothing about the certificate; the
user's own click opens the page in WebView2, whose normal TLS
validation and error pages apply.

### M9 — No TCP_NODELAY: every small response stalls ~40 ms (found by benchmark, Phase 3)
*Evidence:* neither v12.1 (`grep set_nodelay` = 0 hits in its server
and app) nor the first v13 build set TCP_NODELAY. The server's
head-then-body two-write pattern interacts with the peer's delayed
ACK: measured on the v13 harness before the fix — sequential
keep-alive requests ran at **25 req/s (40.2 ms/req)** while parallel
throughput was 5,711 req/s, the classic Nagle signature.
*Impact:* every page asset on every locally served page paid up to
40 ms; a page with 50 assets could waste ~2 s in pure stall.
*Disposition:* **Fixed in v13** — accepted sockets set
`set_nodelay(true)`; the same harness then measures **7,289–7,381
req/s sequential (0.135 ms/req)**. Regression evidence is the
benchmark itself (BUILD_NOTES.md); no timing assertion is baked into
the test suite (timing asserts are flaky by nature).

---

## Low

### L1 — Slow-loris total deadline missing on the static server
*Evidence:* `handle_conn` sets a 3 s *per-read* header timeout; a
client dribbling one byte every <3 s can hold a connection slot for
the whole 16 KiB budget (~13 hours) — bounded by the 64-connection
gate, but all 64 slots can be held this way.
*Disposition:* **Fixed in v13** — total head deadline (5 s) in
addition to per-read timeouts, enforced inside the line reader (a
dribbled line cannot stretch past it). The socket-level test proves
the observable half: a partial head followed by silence is cut at
the per-read timeout (~3 s) instead of holding the slot.

### L2 — Detector process-name cache never expires (PID reuse)
*Evidence:* `spawn_detector`'s `name_cache: HashMap<u32,
Option<String>>` is insert-only for the process lifetime.
*Impact:* after a dev server exits and its PID is recycled, a
different process can be labeled with the dead server's name.
*Disposition:* **Fixed in v13** — the name cache is rebuilt whenever
the listener set's PID set changes (cheap: names are only looked up
for PIDs not in the previous cycle's set), and entries for vanished
PIDs are evicted each cycle.

### L3 — `GetExtendedTcpTable` sizing race can blank one scan cycle
*Evidence:* `enumerate_listeners`/`enumerate_listeners_v6` size the
buffer with one call and fill with a second; if the table grows in
between, the fill returns `ERROR_INSUFFICIENT_BUFFER` and the
function returns an empty list — one cycle shows zero IPv4 (or IPv6)
services. The entry count is also trusted without clamping to the
buffer's capacity.
*Disposition:* **Fixed in v13** — retry loop (up to 3 attempts,
re-sizing from the API's reported size) and the row count is clamped
to `(buffer_len − 4) / size_of::<row>()`. The Win32 code itself is
not host-testable; the fix is code-reviewed and kept structurally
identical otherwise (this exact enumeration worked on the user's
machine in v12/v12.1).

### L4 — Multiple argv files: only the first is opened
*Evidence:* `run()` uses `std::env::args().nth(1)` only.
*Impact:* "Open with Pico" on a multi-selection, or a launcher
passing several files, silently drops all but the first.
*Disposition:* **Fixed in v13** — all argv entries are parsed; each
file/URL opens its own tab (tab cap applies); the start tab is
created only when no argument produced a tab.

### L5 — Tab strip is mouse-only; keyboard focus never reaches tabs
*Evidence:* `assets/chrome.html` renders tabs as `<div>`s with click
handlers — no `tabindex`, no `role=tab`, no Enter/Space activation;
focus-visible styling exists for buttons only.
*Impact:* keyboard users cannot reach or operate tabs; screen readers
see anonymous divs.
*Disposition:* **Fixed in v13** — tabs are real `<button type=button>`
elements with `role=tab` / `aria-selected`, roving focus is native
button behavior, and visible focus rings are part of the design
system in both themes.

### L6 — Focus is not moved to content on tab activation
*Evidence:* `App::activate` only toggles visibility; focus stays
wherever it was (usually the chrome). Page keyboard interactions
(space-scroll, in-page shortcuts) need a click first.
*Disposition:* **Fixed in v13** — activation driven by a tab click or
keyboard cycle focuses the content WebView; activation driven by the
address bar keeps address focus semantics (focus moves to content
after navigation submit, as before in `navigate_active`).

### L7 — Window geometry is not persisted
*Evidence:* `PersistState` holds theme + recents only; every launch
reopens at 1080×720 at the default position.
*Disposition:* **Fixed in v13** — state v2 persists window size +
position with strict validation (finite, ≥ minimum size, clamped to
sane ranges) and restores them; corrupt or off-screen values fall
back to defaults. Maximized state is persisted as a flag.

### L8 — Dead code and inert machinery carried in the server
*Evidence:* `ConnectionGate.drained: Condvar` is created, notified,
and never waited on (`#[allow(dead_code)]`); `FileServer::
active_connections` is public API used nowhere; `pico_core::core::
state::load`/`load_from` wrappers are unused by the app (the app uses
`load_from_checked`); `TabHistory::{len, index, entries}` are
test-only surfaces on the shipped type.
*Disposition:* **Fixed in v13** — removed; the gate is a plain counter
+ RAII permit, diagnostics go through the snapshot path instead, and
test-only accessors are `#[cfg(test)]`-visible only where the tests
live inside the crate (integration tests use the public API they
need).

### L9 — Detector title entity decoding is partial
*Evidence:* `decode_entities` handles 7 named entities; numeric
character references (`&#8212;`, `&#x2014;`) — common in real titles —
render literally on the card.
*Disposition:* **Fixed in v13** — numeric decimal/hex references are
decoded (bounded, invalid references left literal, surrogate/out-of
range values dropped to U+FFFD-free handling — invalid scalars are
skipped).

### L10 — `parse_title` can match `<title` inside attributes/comments
*Evidence:* `parse_title` finds the first `<title` byte run anywhere
in the first 64 KiB — including inside an HTML comment or a `<svg>`
block before the real title, and accepts `<titlex>` (the `>` search
makes `<titlefoo>` yield the foo element's contents).
*Impact:* cosmetic only (wrong card label in rare pages).
*Disposition:* **Fixed in v13** — the match requires `<title` followed
by whitespace, `>`, or `/`, and a `</title` search that skips
candidates inside `<!-- -->` comments. Kept deliberately simple and
byte-based; still a heuristic, documented as such.

### L11 — Chrome page can in principle be fetched as a subresource shape by content
*Evidence:* `pico_protocol_handler` answers `pico://chrome` /
`pico://start` for any webview that asks (it ignores the
`WebViewId`); content navigation to those URLs is denied by the
navigation handler, but the handler itself is registered on every
webview, and the response contains the full chrome HTML source.
*Impact:* no secrets live in the chrome HTML (no tokens, no state),
and document navigation is blocked — the realistic worst case is a
page reading Pico's own UI source, which ships in the open anyway.
*Disposition:* **Retained / residual** — documented in v13
SECURITY.md; restricting by WebViewId is impractical because tab
webviews are created after the handler closure is shared. The
start page's *data* (services/recents) is the only sensitive payload
and it is served only as part of the start document, which content
cannot navigate to.

### L12 — Log file can grow past its cap inside one long session
*Evidence:* `src/log.rs` truncates only at startup when > 1 MiB;
during a session every line is appended without a size check.
*Disposition:* **Fixed in v13** — the writer tracks bytes written and
rotates (truncate + reopen) when the running total crosses 1 MiB,
checked cheaply per write; startup behavior unchanged.

### L13 — SSE `Last-Event`/reconnect semantics rely on EventSource defaults
*Evidence:* the injected script's `EventSource` auto-reconnects;
after a Pico restart the port changes, the old page's EventSource
retries the dead port briefly, and the fallback hard-reload covers
the gap. No defect beyond noise; verified behavior in v12.1 tests.
*Disposition:* **Retained** — the 700 ms Rust-side fallback design is
sound; per-root scoping (M1) does not change it.

### L14 — `display_address` parses the served URL by string splitting
*Evidence:* `App::display_address` does `tab.url.split("/r/")` —
a remote URL containing `/r/` in its path *and* hosted on
127.0.0.1:<server port> could render a misleading address.
*Impact:* cosmetic; requires the page to be on Pico's own server
origin (i.e. an already-served local page), and the split only
affects what's *displayed*, never what's loaded.
*Disposition:* **Fixed in v13** — the display path is derived from
the server's structured `url_parts()` (parse the `/r/<digits>/`
prefix positionally) instead of a substring split.

---

## Info / verified-not-a-defect (re-checked claims)

- **I1 — Security controls from v12.1 all re-verified present and
  wired:** Host-header wall incl. SSE, absolute-form rejection,
  64-conn/16 KiB/3 s caps, GET/HEAD only, decode-once normalization +
  canonical containment, UNC gates *before* any FS touch (the
  `open_path` order — lexical check → canonicalize → re-check — is
  correct), downloads denied on chrome + tabs, profile hardening via
  `ICoreWebView2Profile6`, scheme allowlist in three places, action
  URLs gated to the start tab, 30-tab cap, random root tokens,
  same-origin nav-note check, IPC body caps + hostile corpus,
  state sanitization, control-stripped logging, zero unwrap/expect.
  All are ported to v13 and re-proved by its regression suite.
- **I2 — Wine check (this session):** `wine --version` reports
  wine32 missing / multiarch not enabled — the exe cannot be
  executed here. Same bounded conclusion as every prior version.
- **I3 — Dependency hygiene:** `cargo tree -d` over the core
  (no-default-features) set shows **no duplicate crates**; the full
  GUI tree's duplicates are the usual windows-sys/windows-core version
  pairs pulled by tao/wry/webview2-com (recorded in v13
  BUILD_NOTES). `cargo audit` is **not installed** in this VM (see
  Phase-3 note in BUILD_NOTES for the bounded attempt).
- **I4 — `panic=abort` + panic hook:** the hook logs and then the
  process aborts; acceptable for a single-user dev tool and retained.
- **I5 — v12.1's `signature()`** excludes the service `url`; a service
  whose answering IP flips (v4↔v6) with identical title/process would
  not re-push. Subsumed by M7's TTL refresh; v13's signature includes
  the URL.

---

*End of audit. Every "Fixed in v13" item has a regression test or a
named verification in v13's BUILD_NOTES.md; anything not fixed is
listed above as Retained/Residual with its reasoning, and mirrored in
v13's SECURITY.md.*
