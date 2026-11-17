# Pico — Build Notes

## 0.41.4 (2026-11-17)

Maintenance release, rebuilt from the exact tree in this repo so the
release matches the source file for file. No behavior changes since
0.41.3. This file — the build log cited throughout the code — is now
included in the repo.

---


## The black frame on tab switch / open / close, root-caused

User's report on Windows: opening a tab, closing a tab, or
switching tabs flashes a black/blank frame for about one
frame.

The mechanism is WebView2 composition timing plus three
shell sequencing shapes. Controllers compose
asynchronously: hiding a controller stops its presentation
at once, a newly shown controller needs a beat before it
presents its retained frame, and a controller that has
never been shown has no frame at all (controllers do not
compose while invisible). In every gap the content region
shows the tao window background — the theme surface,
effectively black in a dark theme. The three shapes:

1. Hide-before-show (switch). `activate` flipped
   visibility in a single pass over the tab vec
   (`set_visible(t.id == id)` in vec order). A target
   sitting AFTER the current tab in the vec was shown only
   after the current view had already been hidden.
   Switching right flashed; switching left could escape by
   vec-order luck.
2. Drop-before-activate (close). `close_tab` removed the
   tab record and dropped its WebView — destroying the
   visible view — BEFORE activating the successor. A
   guaranteed empty frame on every active-tab close.
   (Background-tab close was and is fine.)
3. Fresh-controller first frame (open). A new tab's view
   was shown by the same single pass although it had never
   composed a frame; its first presented frame lags the
   show by however long the first composition takes.

What already mitigated this, and is unchanged: content
views are built with the themed background color
(`with_background_color`, AUDIT V14-2) and hidden
(`with_visible(false)`); the window background is the
theme surface; the chrome stays deliberately transparent
(AUDIT V16-2 — no background color on the chrome view,
ever).

## The fix: sequencing, decided in pure core

The decision moved to `src/core/visibility.rs`
(activation_plan / reveal_plan / successor_after_close,
unit-tested there and through tests/visibility_tests.rs);
`src/app.rs` only translates its tab records into facts
and executes the ordered plan (`exec_vis_plan`):

- `activate` sets the active id, runs `layout()` FIRST
  (the target's bounds are correct before presentation),
  then: a painted target is shown NOW and every other
  view is retired DEFERRED — a `UserEvent::RetireViews`
  posted through the existing proxy hides them on a later
  event-loop turn, after the target's compositor has had
  its beat. The retire arm re-checks its exception
  against the CURRENT active tab (a superseded activation's
  retire does nothing; its successor posted its own) and
  tolerates tabs that vanished in between.
- A fresh target (view built this call or by
  `retry_tab_view`, first load not yet started — the new
  `Tab::reveal_pending` flag) is neither shown nor does it
  retire anyone: the previous view stays on screen. The
  tab's first page-load event (Started or Finished,
  whichever arrives first — the hook sits at the top of
  `on_page_load`, before the refuse/swallow branches, so
  failed first navigations resolve in the same call)
  reveals it: show + deferred retire. A navigation refused
  at NavigationStarting produces no page-load event, so
  the NavigationSettled arm resolves the reveal from the
  completion verdict instead. The flag also clears when
  the tab's load starts in the background and when another
  tab is activated — a pending tab switched away from is
  an ordinary background tab, and no later activation ever
  waits for a load event that already fired. A target with
  NO view (creation failed) retires the others immediately,
  matching the historical failed-tab behavior.
  The race that is NOT possible: a fresh view's load event
  firing between its construction inside `add_tab` and
  `activate` in the same call. The view's handlers only
  post through the event-loop proxy; the event loop is
  busy executing `add_tab`, so the earliest any load event
  is handled is after `add_tab` returns — by which time
  activation has already marked the tab reveal-pending.
- `close_tab` on the ACTIVE tab computes the successor
  from the current vec (`successor_after_close`: the
  right neighbour, else the left — the old post-removal
  index arithmetic, stated purely), activates it under
  the rules above, and only THEN removes the record and
  drops the closing view. Closing the LAST tab creates
  the replacement start tab un-activated, activates it
  under the fresh-tab rules (the dying view stays up
  until the new start page's first load reveals it), and
  then removes the old record. All non-view cleanup
  (Shield/Lite/stats forget, the pending-visit flush,
  live-set removals, root release) keeps its semantics;
  the flush still runs before the record goes.
- `retry_tab_view` aligns with the model: a rebuilt view
  is marked fresh and is no longer shown synchronously;
  it reveals on its first load like a new tab's.
- `focus_content` now applies only when the plan actually
  showed the target now; focusing a still-hidden fresh
  view was a no-op against WebView2 anyway, and the new-tab
  flow's focus lives in the chrome (address bar) as before.

Pins (tests/visibility_tests.rs): no
`set_visible(t.id ==` loop anywhere; exactly three
`set_visible` call sites in the shell (one hide in the
shared helper, the ShowNow arm, the startup window show);
activate lays out before it executes the plan; close_tab's
successor activation and last-tab creation both precede
its single `tabs.remove`; the content builder still sets
the themed background + starts hidden; the chrome builder
stays transparent with no background color; the retire /
reveal wiring (event, arm with its active re-check, both
reveal call sites, both freshness marks) exists.

## Other flash candidates, swept and parked

- Resize / scale-change: `layout()` re-bounds the chrome
  and the ACTIVE view only; hidden views keep stale
  bounds until activated — and activation now lays out
  before showing, so a stale-bounds frame cannot present
  on switch. The resize repaint itself shows the themed
  window background, as designed (AUDIT V14-2). No change.
- Chrome watchdog recreation: the recreated chrome view
  is visible from construction and has never painted, so
  the chrome band can show the themed window background
  for its first frames. That path runs only when the
  chrome has already failed (the alternative was a dead
  shell); not a tab-gesture flash. Parked.
- In-place navigations and conversions
  (convert_start_tab / convert_tab_to_start / reload):
  the same view stays visible throughout; any blank is
  the new document's own first paint, not a visibility
  gap. No change.

## Honesty notes

- There is no WebView2 on this host: the shell compiles
  for the Windows target (xwin clippy, `-D warnings`) and
  the plan is host-tested, but the frame timing itself —
  that the deferred retire lands after the target's first
  presented frame, and the z-order of the two stacked
  content views during the one-turn overlap — is reasoned
  from the composition model above, not witnessed. The
  on-device witness is the user's: switch left and right,
  open a tab, close the active tab; no black frame.

## The deep pass: four fronts, proven-only fixes

The full ledger (every finding with its proof, disproof,
or parking reason) is in the round's audit note; the
load-bearing designs:

- History verdicts are now paired by NavigationId.
  0.41.2 stashed the candidate URL at commit and read the
  verdict later — but wry derives its Finished page-load
  from NavigationCompleted with the LIVE Source at
  dispatch, so under a superseding navigation the stash
  could hold the NEXT navigation's URL: the next page got
  recorded before it displayed (sometimes twice, sometimes
  despite failing), and the displayed page never recorded.
  `PendingVisit` is now a small queue keyed by
  NavigationId: a ContentLoading handler notes the
  candidate (id + the dispatch-time Source), the raw
  NavigationCompleted handler notes the verdict for the
  SAME id, and unmatched candidates flush toward recording
  (the 0.41.0 degrade), never toward silence.
- Deletions are tombstoned. The two-process nav merge was
  union-only, so one Pico's save resurrected everything
  the other Pico had cleared or removed. nav.json now
  carries a clear-watermark plus per-key tombstones
  (bounded, 512); merge takes the max watermark and the
  tombstone union, then filters; re-adding a key drops
  its tombstone. Same-millisecond causality resolves to
  the deletion (documented in navstore).
- A migration resume no longer rewinds the live home:
  the pending+Live arm respects an existing destination
  profile (skip_profile), and the no-pending fill arm is
  missing-only — files the interim session already wrote
  are never overwritten by legacy bytes.
- A non-navigation download (`<a download>`) no longer
  moves the tab it was clicked on: the capture chain only
  gains the live URL when a navigation is actually in
  flight, matching the coordinator's own contract (and
  the referer is again the page being read).
- The New-Pico secret check matches secret STEMS inside
  key segments (`refresh_token`, `id_token`,
  `client_secret`, `X-Amz-Signature`, bare `code`, …)
  instead of an exact-match deny-list; the tab-move wire
  verb names the drop gap's right-hand neighbour
  (`before: <id|null>`) instead of a bare gap index, so a
  tab closing mid-drag can no longer shift the landing
  slot. The core resolves it via `final_from_neighbor`.
- One fuzz-proven panic class closed: `shield::host_span`
  sliced by byte index on an unverified boundary (not
  reachable from its only caller — host always derived
  from the same URL — but a pub core landmine). The
  shield-prune report gained the sender gate its sibling
  reports always had, its parser drops userinfo-shaped
  paths, and `redact_url` now strips userinfo from
  scheme-relative URLs too. Report prefilters are
  structural (parsed kind/action keys), so honest bodies
  whose titles contain "nav"/"action" are no longer
  swallowed, and whitespace variants route consistently.
- Smaller proven fixes: titles that settle before the
  verdict now follow the recorded visit (and bookmarks);
  favicon bytes are re-checked against the sender's
  current origin before caching (no cross-host icon
  theft), a failed icon-stream read caches nothing, and
  a hot icon's eviction age refreshes (at most daily);
  the Args2 status-cast failure logs once instead of
  silently dropping displayed error-page visits;
  canonical nav keys no longer split `https://host` from
  `https://host/`, and IPv6 origins render canonically;
  the Lite classifier and the media probe are one matcher
  again; the Downloads page and the chrome popover render
  one shared per-task story (indeterminate wash, progress
  line, retry line, error line) and the page's refresh
  editor keeps focus and caret across data pushes;
  dlrows coerces task ids to numbers like every other
  field (a string id could otherwise inject markup);
  `convert_tab_to_start` cleans the one-shot nav maps
  like `close_tab` does; the Service action refuses
  credentialed URLs; SaveLock re-verifies its token after
  acquiring; detect's title scan resumes past a broken
  first `<title>` tag.

## What a hard kill loses (documented, accepted)

Both graceful exit arms are in parity (pause + persist
downloads, capture geometry, flush pending visits, save
state, save nav). If the process is killed instead, the
loss is bounded to: at most one pending history visit per
tab, window geometry since the last save, and stats
(blocked counts / savings) accumulated since the last
`save_state` — record_block/savings do not save on their
own, so a long quiet session killed at the wrong moment
loses that session's counters. Files on disk are never
corrupt (every writer is temp + fsync + rename); the
loss is freshness, not integrity. Accepted: flushing on
every block would trade SSD wear and event-loop stalls
for a counter.

---

# Pico 0.41.2 — Build Notes (2026-11-16)

## The history witness, root-caused

User's report on 0.41.1: "You hit a bookmark and go
into a website — it records that in history. But after
that, whatever you open doesn't count or save."

The recording path has exactly one gate: a commit in
`apply_url_event` recorded only if the committed URL
differed from the tab's "last failed source" mark
(0.41.1, Front B F1). The mark was written by a raw
`NavigationCompleted` registration whose handler read
the verdict's URL from the core's LIVE `Source`
property — the completed args carry no URL, so the
handler substituted whatever page was current at
dispatch time. Three documented platform facts break
that design:

1. `NavigationCompleted` fires when loading fully
   stops (`body.onload` / error) — on a slow
   connection, seconds after the page commits and
   becomes clickable.
2. Starting a navigation before the previous one
   completes aborts it, and its `NavigationCompleted`
   reports `IsSuccess=false` ("navigating away before
   the original navigation completed" — Microsoft
   Learn). The two navigations' events interleave.
3. `Source` advances to the NEW navigation's URL
   before its `ContentLoading` — so the aborted
   navigation's failure verdict, dispatched amid the
   new navigation's events, was stamped with the NEW
   page's URL and became the tab's failed-source mark.
   When the new page's own commit reached the gate,
   mark == its URL: refused, silently (one log line).

A fresh tab's first navigation (the bookmark click)
has no predecessor in its tab and an empty mark, so
it always recorded — the witness's exact shape. On a
fast connection pages complete before the user moves
on, verdicts arrive as successes, and the bug never
shows; every test and battery run passed while the
user's browser recorded almost nothing. Two further
defects in the same design, proven independently:
`IsSuccess=false` does not mean "not a visit" (it is
also false for HTTP 4xx responses and for a page that
calls `window.stop()` after a 200 — the 0.41.1 code
comment claiming 404/500 complete with IsSuccess=true
was wrong), and a failed attempt's mark refused the
user's successful RETRY of the same URL (the mark is
consulted at the retry's commit, before the retry's
own success could clear it).

## The fix: the verdict travels with its navigation

The cross-navigation mark is deleted. A commit now
STASHES its URL as the tab's pending visit
(`core::navstore::PendingVisit`); the raw
`NavigationCompleted` handler reads the verdict from
the SAME navigation's own args — `IsSuccess` plus
`HttpStatusCode` via
`ICoreWebView2NavigationCompletedEventArgs2` — and
the event arm decides the pending candidate with it
(`navstore::verdict_records_visit`: record when the
navigation succeeded OR received any HTTP response;
a failure with no response — DNS/refused/timeout —
displayed the browser's error page and is not a
visit). The verdict carries no URL, so it can only
ever decide its own navigation's candidate: it is
queued immediately after that navigation's page-load
Finished from the same dispatch, and is always
processed before the next navigation's commit in
that tab. A candidate whose verdict never arrives
(registration or read failure) flushes as a record
at the next CommitStart, at tab close, and at app
close — degradation is to the 0.41.0
record-everything behavior, never to silence.

Regression tests (tests/nav_tests.rs): the verdict
truth table; the user's exact sequence (bookmark
open + two navigations, all recorded, persisted,
surviving a save/load cycle); the supersede
interleaving above; the failed-attempt/retry pair;
the lost-verdict flush. The sibling-pattern sweep
found no other instance of the fragile shape:
`expected_nav` is consumed by the first navigation
event for its tab, matched or not, and
`userinfo_adopt` is written by the allowing handler
itself with the exact URL it allowed — neither is a
verdict about one navigation consulted at another's
commit.

## Performance pass (measured; harness in
## examples/perf_bench.rs, equivalence in
## tests/perf_equiv.rs)

The emergency was the nav save path. At the caps
(2000 history + 200 bookmarks), `NavStore::sanitized`
cost 0.30–8.68 SECONDS and `save_merged` 0.57–20.9
SECONDS of pure CPU — and the shell saves on every
recorded visit and every title note. Root cause:
`canonical_nav_key` recomputed inside linear finds —
the dedupe/merge passes were O(N²) in key
computations (~4.04M per sanitized). sanitized and
merge_nav_stores now build the key set / first-index
map once (2,200 and ~4,450 key computations), proven
output-identical to the old algorithms by
differential tests over generated stores (canonical-
key collision pool, Unicode titles, future
timestamps): sanitized ~9 ms, merge ~7 ms,
save_merged ~27–30 ms at cap. (Absolute numbers are
medians on a shared VM and swung between runs; the
operation counts are exact, and the win was decisive
in every run.)

Also changed, each with equivalence evidence:
suggestions/search hoist the lowercased query out of
the per-entry scan (2000 allocations → 1 per search;
'article' suggestions 2709/629 µs → 472–566 µs);
shield `decide` partitions page flags by kind (the
shipped lists carry 406 generichide lines and ZERO
genericblock — the old code scanned all ~405 mixed
flags per request for the genericblock check),
walks host suffixes without allocating, and borrows
the URL lowercase when it is already lowercase (312
full-list + 180 synthetic golden decisions and the
full cosmetic script reproduced exactly); the nav
push gates (start tabs + history page) no longer
serialize + hash the full payloads on every call
just to discard them (up to ~4 ms per call at cap —
`push_state` calls one on every state push) and
instead compare a packed (nav_rev, icon_rev)
revision pair bumped at the six audited store-
mutation sites and on favicon stores; the downloads
page push takes ONE snapshot for both its signature
and its payload (was two full task-view clones,
~25 µs at 100 tasks / ~255 µs at 1000).

Measured and left alone, with numbers in the run
record: shield long-URL cost (the Aho-Corasick scan
itself; an array-indexed table is its own pass),
nav-file pretty-print serialization (kept for the
byte-identical discipline), state save (µs at real
sizes), SpeedMeter/snapshot cadence in core, the
localhost detector's pure parts (µs at real listener
counts), the live-reload watcher. Parked with proof
sketches: filterpipe's duplicate host extraction,
the matches_query host disjunct on the miss path,
single-op navstore scans (dwarfed by the save that
follows each).

Simpler-mechanism replacements taken under the
user's "smarter and better" mandate, one line each:
the per-navigation verdict + pending candidate
replaces the cross-navigation failed-source mark
(fewer moving parts, no cross-event state to go
stale); revision counters replace build-and-hash
push gates (O(1), and a change cannot hash-collide
into silence); key-set dedupe replaces repeated
linear finds (same outputs, O(N)).

## Real favicons (the user's other 0.41.2 ask)

Bookmark cards, Recent tiles, history rows and
suggestion rows showed a letter tile; they now show
the site's real icon when one is known, with the
letter kept as the no-layout-shift fallback. The
mechanism is the platform's own: a per-view
`FaviconChanged` registration on `ICoreWebView2_15`
(the vendored webview2-com 0.38.2 bindings expose
it), then `GetFavicon(PNG)` — the browser itself
fetched the icon from the page's own origin, so
Pico makes no extra request, uses no third-party
favicon service, and never fetches a credentialed
URL. If the cast to `ICoreWebView2_15` fails (old
WebView2 runtime), the handler never registers and
every surface keeps its letter tile; logged once,
never fatal.

One pipeline, one cache (`core::icons`): icons are
stored per host under `<Pico home>/icons/`, keyed
by the origin prefix of `canonical_nav_key`
(refusing internal, credentialed and non-http(s)
URLs), capped at 512 files (oldest-first eviction)
and 128 KiB per icon, stored as served PNG bytes.
Surfaces receive icons as `data:` URIs inside the
existing JSON payloads — the internal pages already
allow `img-src data:`, so no new protocol route, no
new internal origin, and no CSP change was needed;
only icons up to 32 KiB are inlined (larger ones
stay on disk and their rows keep the letter tile),
and pages validate the exact
`data:image/png;base64,` prefix before painting, so
a missing or bogus field degrades to the letter,
never a broken row.

# Pico 0.41.0 — Build Notes (2026-11-16)

## What 0.41.0 is

Workstream A: DATA RELOCATION. The v27 portable-data
design is reversed by the user's order: NOTHING is
written beside the exe anymore. All persistent state
moves to `%LOCALAPPDATA%\Pico\`, downloads default to
the standard Windows Downloads folder, and the legacy
homes migrate automatically on first run.

Cargo package 0.41.0, `core::DISPLAY_VERSION` is
"0.41.0", transport User-Agent `Pico/0.41.0`.
Cargo.lock diff vs 0.40.3: the own-package version line
only. No new dependencies (one added feature flag,
`Win32_System_Com`, on the existing windows-sys
dependency — CoTaskMemFree for the known-folder string
hand-off).

## The new home

- Resolution lives in `core::state::resolve_pico_home`
  (pure, facts injected, host-tested): the Windows
  known folder FOLDERID_LocalAppData
  (SHGetKnownFolderPath, `core::state::known_folder`,
  cfg(windows)) → the `LOCALAPPDATA` env value → an
  APPDATA-derived Local path (a trailing `Roaming`
  component swapped for `Local`; a non-standard APPDATA
  used as-is) → `$HOME/.local/share/Pico` → the relative
  `Pico` as the last resort. Never the exe folder, never
  a probe: the v27 `probe_exe_dir` and the whole
  writable-exe branch are deleted.
- Sub-layout, unchanged shapes under the new root:
  `state.json`, `pico.log`, `downloads\` (task state),
  `WebView2\` (the profile). `DataDirChoice` loses its
  `portable` field; the startup line is now
  `data: appdata (<reason>) <dir>`.

## WebView2

- What the 0.40.3 build actually creates: exactly ONE
  `WebContext` (`WebContext::new(Some(webview_data_dir()))`)
  and two builders on it (`build_chrome`,
  `build_content_view`, both `new_with_web_context`) —
  the profile already landed in the data dir's
  `WebView2\` and no default folder appeared beside the
  exe. 0.41.0 keeps that mechanism, points it at
  `%LOCALAPPDATA%\Pico\WebView2`, and PINS it: a
  source-scan test (`every_webview_is_built_on_the_one_
  explicit_context`) fails if a context-free builder or
  a second context ever appears.

## Migration (first 0.41 run, before log init)

Order in `App::run`: resolve the new home → migrate →
`log::init()` (into the NEW home) → log every migration
step from the outcome record → state load → WebViews.

- Source: `<exe>\data\` when present, else the Roaming
  `%APPDATA%\Pico` tree. `core::state::migrate_legacy_home`
  = plan → copy → carry log → verify → remove.
- The performer now copies the WHOLE legacy tree
  (profile, downloads task state, every Pico file),
  state.json last as the completion marker, pending
  marker as before. Exclusions: `pico.log` (carried
  separately), the pending marker, state temps.
  Through 0.40 the performer copied only state +
  profile — the downloads task files would have been
  left behind by a naive re-point; they ride now.
- Log carry (`carry_legacy_log`): no new-home log yet →
  the legacy log becomes it verbatim; one exists (a
  retry) → the legacy log lands as `pico-legacy.log`.
- Verification (`verify_migration_copy`): every source
  file present at the destination at the same size,
  state.json byte-identical. Only a verified copy lets
  the beside-exe tree be removed. On failure: legacy
  untouched, an ERROR line in the new pico.log, Pico
  runs from the migrated copies, plan re-issues next
  start. The Roaming source is never removed (standing
  backup, as in the v27 move).
- Beside-exe WebView2: `migrate_beside_exe_webview`
  carries `<exe>\Pico.exe.WebView2\` (contents, so
  `EBWebView` lands correctly) or a bare
  `<exe>\EBWebView\` into the new profile before any
  WebView is created; locked files count as failed in
  the report, the source is left in place, and an
  existing destination profile is never clobbered.
- Legacy `<exe>\Downloads\` and its files: never
  touched. Task records store file names, not absolute
  paths (the brief's "tasks reference absolute paths"
  does not match the shipped record shape — see the
  downloads note below).

## Downloads

- Default destination is now the standard Windows
  Downloads folder: `core::state::user_downloads_dir`
  (FOLDERID_Downloads known folder, else
  `%USERPROFILE%\Downloads`).
  `core::download::resolve_downloads_dir` is re-shaped
  to (custom, user_downloads, pico_home): a custom
  (user-changed) dir would win — no such setting exists
  in the tree, so the app passes None — then the user's
  Downloads folder, then `<AppData home>\Downloads` as
  the last resort. `probe_downloads_dir` is deleted;
  the exe folder is never a destination.
- Honest consequence, stated plainly: a task record
  holds only a file NAME, resolved against the CURRENT
  downloads folder. A download finished under 0.40.3
  into `<exe>\Downloads` stays on disk there untouched,
  but its record's Open-file path now resolves into the
  Windows Downloads folder. Unfinished tasks' `.part`
  files likewise stay in the old folder. This is the
  one visible seam of the move; the alternative
  (rewriting records or copying user files) was out of
  the brief's bounds — the legacy folder is left
  exactly in place for the user.

## Two-process safety

- Main state: unchanged (per-pid temp + fsync + rename).
- Downloads task state: `core::download::save_task`
  gains a lock FILE guard (`save.lock` in the task
  state dir, `create_new`-claimed, 30 s stale timeout,
  2 s bounded wait, released on drop; on timeout the
  save proceeds without it — the atomic rename is the
  correctness floor, the lock only serializes the
  write+rename window across two running Picos).
  Choice documented in the code at `SaveLock`.

## Also in this round

- The stale start-page footer ("Downloads are blocked
  by design — Pico is a viewer, not a downloader") now
  reads "Downloads land in your Downloads folder,
  managed from the Downloads page." Glow census
  unchanged: 13 `var(--glow*)` usage sites before and
  after.
- README + SECURITY updated to the AppData design
  (SECURITY gains a 0.41.0 delta at the top; the v27
  portable text is retained as history and marked
  superseded).

## Tests

- `tests/portable_data.rs` rewritten to the new
  contract (27 → 38 tests; the removed/changed list is
  in the workstream report). `tests/download_tests.rs`:
  the one `resolve_downloads_dir` test re-shaped to the
  new signature and semantics.
- Rust tests: 652 passed, 0 failed (0.40.3 baseline
  640; +12 net). DOM suite: all checks passed in every
  battery iteration.
- Battery: `./scripts/battery.sh 10` — 10/10
  consecutive PASS (log `battery-0.41.0.log`);
  iteration 10 includes Windows clippy (only the one
  pre-existing warning, winhttp `fetch_range` arity —
  identical to the 0.40.3 baseline) and the release
  check.
- Host clippy `--locked --no-default-features -- -D
  warnings`: clean (baseline was also clean). `cargo
  fmt --check` clean.
- Windows exe: 2,256,896 bytes, sha256
  c92ffc2c47eb6cd3e380bb9db37060b0eeae6134deae5ededb8644b96d3d7a9c
  (changed vs 0.40.3's 68cb60a3… as required). Gate v2:
  PASS — 0 RT_MANIFEST, RT_ICON ×8 + RT_GROUP_ICON ×1,
  static CRT, WINDOWS_GUI, under 5 MB. Strings audit:
  `Pico/0.41.0` present, `0.40.3` absent.

## Witness list (user's PC — cannot be proven here)

- First 0.41.0 run over a real 0.40.3 install: pico.log
  (in %LOCALAPPDATA%\Pico) shows the migration lines,
  the legacy `data\` folder is gone afterwards, logins
  are intact, and NO new folder appears beside the exe.
- A download lands in the Windows Downloads folder.
- Real WebView2 behavior (SHGetKnownFolderPath results,
  locked-cookie copies during migration) is Windows
  behavior: proven here only to the host-test +
  Windows-target-compile level, not on a live runtime.

## Workstream B — Bookmarks + History + Suggestions

Bookmarks and history arrive (they were Pico non-goals
until this release; 0.41.0 commissions them).

- STORAGE: a dedicated `nav.json` in the Pico home —
  NOT fields in `state.json`. state.json is capped at
  1 MiB under a "<4 KiB" assumption, and a corrupt nav
  file must never take settings down with it. Same
  hardening shape as the state machinery: per-pid temp
  + fsync + clear-readonly + rename, 8 MiB
  metadata-gated load, and a load-time sanitizer that
  drops unrecordable rows, merges duplicate URLs and
  re-applies the caps. Schema v1:
  `{bookmarks:[{url,title,added_at}], history:[{url,
  title,last_visited,visits}]}`. History caps at 2000
  entries, pruned least-recently-visited; bookmarks
  cap at 1000.
- RECORDING: only committed document navigations
  (HistEffect::Commit) are recorded — http/https,
  localhost and file URLs. `pico://` pages are NEVER
  recorded, in either spelling (raw or the folded
  `https://pico.*` form), nor are credentialed URLs.
  A re-visit updates the row in place (visits + 1,
  last_visited, title when the page reports a better
  one). Titles follow the tab's reported title;
  the fallback is the host, then the URL.
- FRECENCY: `score = visits × recency weight`, the
  weight by age of last visit: ≤1 h → 100, ≤1 d → 70,
  ≤7 d → 40, ≤30 d → 20, older → 10. Ties break by
  last_visited desc, then URL asc — a total order, so
  rankings are deterministic and unit-tested.
- PROTOCOL: `core::navcmd`, the dlcmd pattern — wire
  bodies `{"kind":"nav",...}` with deny_unknown_fields
  parsing, a surface table, conjunctive gates and
  chrome-transport envelope + dedupe. Verbs:
  `bookmark-toggle` (chrome), `bookmark-query`
  (chrome), `bookmarks-list` (start), `bookmark-remove`
  (start), `history-list` / `history-search` /
  `history-remove` / `history-clear` (history page),
  `suggestions-query` (chrome). Toggle/query carry NO
  url argument — they act on the shell's active tab,
  so a page can never aim them elsewhere. There is
  deliberately no open-URL verb; opening rides the
  existing start-page service action and the chrome's
  navigate command. The JS↔Rust vocabulary is pinned
  by `tests/nav_pair.rs` (the dl_pair pattern).
- CHROME: a star in the address well (filled when the
  active tab is bookmarked, mirrored from the
  snapshot), Ctrl+D toggles, Ctrl+H opens History —
  both through the one accelerator registration beside
  Ctrl+J. The suggestions dropdown renders bookmark
  matches first, then history by frecency, then the
  typed text as a "go" row; Up/Down/Enter/Esc/mouse/
  blur behave conventionally, and while it is open the
  chrome view grows by `layout::suggestions_h()` so
  the page is never covered by WebView overlap.
- START PAGE: a Bookmarks section (service-card
  anatomy, × per card) that renders ONLY while
  bookmarks exist, and a Recent section with the last
  8 history entries as tiles; its header carries the
  History link. Glow census unchanged: 13
  `var(--glow*)` sites before and after.
- HISTORY PAGE: `pico://history`, a downloads-family
  page (`assets/history.html`): search box (local
  filter + debounced `history-search`), rows with
  title + URL + last-visited + visits, per-row remove,
  Clear with the two-step arm confirm, empty state,
  version badge/footer 0.41.0.
- ROUTING FIX: typed or suggested `pico://downloads`
  and `pico://history` now open the MANAGED pages.
  `addr::typed_internal_page` classifies the exact
  spellings (query/fragment/trailing slashes tolerated,
  lookalikes like `pico://downloads.evil` refused and
  still refused by `parse_input`), and a navigation
  naming either page from a foreign tab is funneled to
  its opener instead of loading bare. FINDING: the
  funnel did not exist before this workstream — the
  start page's Downloads card relied on it per its
  comment, but no code implemented it; the "downloads
  page via all four doors" witness stays on the owed
  list below, now covering both cards.
- KNOWN EDGE: a `file://` bookmark/recent card on the
  start page is refused by the service action's
  web-origin gate (logged `bad-service-url`). Rare:
  content tabs never carry raw `file://` document
  URLs — typed file paths become local-server tabs.

## Tests (Workstream B)

- Rust tests: 682 passed, 0 failed (Workstream A
  baseline 652; +30: navstore/navcmd unit tests,
  `tests/nav_pair.rs` vocabulary pin, `tests/nav_tests.rs`
  routing fix + gate source scans, assets history
  coverage). DOM suite: 671 checks, all passed —
  38 new 0.41 checks (star, Ctrl+D/H, suggestions
  keyboard/mouse flow, start bookmarks/recent, history
  page search/remove/clear/empty) plus the v27 cluster
  order updated for the star.
- Battery: `./scripts/battery.sh 10` — 10/10
  consecutive PASS; iteration 10 includes Windows
  clippy (only the one pre-existing winhttp warning)
  and the release check.
- Host clippy `--locked --no-default-features
  --all-targets -- -D warnings`: clean. `cargo fmt
  --check` clean.
- Windows exe: 2,335,232 bytes, sha256
  4ddcfa3cfd5b799e4f52957a74166a4373cef6c1212931e26a608eb247460da5
  (differs from Workstream A's c92ffc2c… as required).
  Gate v2: PASS — 0 RT_MANIFEST, RT_ICON ×8 +
  RT_GROUP_ICON ×1, static CRT, WINDOWS_GUI, under
  5 MB. Strings audit: `Pico/0.41.0` present, `0.40.3`
  absent.

## Witness list additions (user's PC — cannot be proven here)

- Star fill, dropdown visuals and the chrome-rect
  growth on a real Windows HWND; Ctrl+D / Ctrl+H with
  page focus (the accelerator path is source-pinned
  and shares the Ctrl+J registration, but only a live
  runtime proves WebView2 delivers the chords).
- The managed-page funnel end-to-end: type
  `pico://downloads` and `pico://history`, click both
  start-page cards, and confirm the managed pages open
  (this also closes the pre-existing four-doors
  witness for the Downloads card).
- First-run `nav.json` creation in
  %LOCALAPPDATA%\Pico and its survival across restarts.

## Workstream C — Movable Tabs

Tabs can be reordered, moved by keyboard, and handed
to a new Pico window from a right-click menu.

- ORDER OWNERSHIP: the app's `Vec<Tab>` is the one
  source of truth. The strip is a renderer (keyed
  reconciliation already moved DOM nodes to match the
  snapshot); a drag only computes a GAP index and
  asks. The app converts it with the pure
  `core::tabcmd::insertion_to_final` / `reorder_ids`
  math (gap `g` = before the tab now at `g`; gaps
  `from` and `from + 1` are the identity; the moved
  element's final index is unit-tested for every
  (from, gap) pair) and re-renders. `active` is an id,
  so the active tab stays active; each Tab record
  owns its WebView, so views follow their tabs and
  the active view's visibility never changes. NOTHING
  persists tab order: sessions do not restore tabs
  (PersistState has no tab list; startup opens argv
  or a start tab), so there is no second store to
  round-trip — the order is session state by design.
- DRAG (chrome.html): pointerdown on a tab (never on
  its ×) arms a drag; 6px of movement makes it one —
  the tab dims (`.dragging`), pointer capture holds
  the gesture, and a 2px accent line (`#tab-drop`,
  a background, not a border) paints at the current
  gap. Below the threshold a press is a plain click
  and activates as always. pointerup posts ONE
  `move` body and suppresses the drag's tail click,
  so dropping an inactive tab does NOT switch to it.
  pointercancel, Escape and window blur cancel with
  nothing sent. The new-tab button is not a drop
  target: past the last tab's midpoint is simply the
  end gap.
- KEYBOARD: Ctrl+Shift+PageUp / PageDown moves the
  active tab one step, clamped at the ends
  (`step_final` — a step never wraps; cycling does).
  COLLISION CHECK: no binding anywhere in the tree
  named PageUp/PageDown before this round (grep over
  src/assets/tests/scripts: zero); Ctrl+Shift+Tab is
  the cycle chord, a different key. Both focus worlds
  are wired: the chrome document's own key handler,
  and the content accelerator registration (the
  Ctrl+J choke point) with two new arms — VK_PRIOR /
  VK_NEXT requiring Ctrl AND Shift (the exact inverse
  of the J/H/D Shift veto; Alt still vetoes all five
  chords) — forwarded as `UserEvent::MoveTabShortcut`.
- TAB MENU: right-click on a tab posts the new
  `tab_menu_open {tab, x, y}` chrome command (the
  `tab_menu_close` twin dismisses). The app builds
  the menu through the content menu's own machinery —
  `core::tabcmd::tab_menu_items` (Move Left / Move
  Right, enabled only toward an existing neighbour,
  plus the new-Pico row), `layout::ctx_menu_pos` /
  `ctx_menu_extra` geometry, a snapshot card
  (`tabContextMenu`) the chrome renders with the
  .ctx-row anatomy — and owns the open state like
  every other overlay (panel funnel, chrome-rect
  claim, focus the chrome). Picks return as tabcmd
  bodies, never `ctx_pick` (that whitelist is the
  content menu's alone).
- NEW-WINDOW ROUTE — NEW PROCESS, decided from the
  code: the app is single-window by construction
  (one `Rc<Window>`, one chrome, one `WebContext`;
  the event loop matches WindowEvent WITHOUT a
  window id; every content view is a child of the
  one window; the downloads coordinator and theme
  pushes assume it). A second in-process window is
  a redesign, not a tab action. The shipped route
  relaunches the CURRENT exe with the tab's URL as
  its argument — argv handling already opens every
  openable argument — under the pure plan in
  `core::tabcmd::new_pico_plan`: a plain web tab
  with nothing in flight is `MoveAndClose` (spawn
  succeeds → the original closes; the menu labels
  it "Move to New Window"); a web tab mid-download-
  capture (the coordinator's new
  `has_pending_capture` query — a capture awaiting
  its cookies) or a failed tab is `OpenWithUrl`
  (launch, keep the original); an internal page
  (start/downloads/history — `pico://` argv is
  refused by design) is `OpenBare` (launch with no
  argument); both keep-plans label the row "Open in
  New Pico". A local-server tab (its URL embeds
  THIS server's port + root token) or a
  credentialed URL is `NotOffered` — the row does
  not appear. A failed spawn keeps the original,
  always. Two Pico processes are the documented
  accepted posture (0.40.1 note; WS-A's save lock).
- PROTOCOL: `core::tabcmd`, the navcmd pattern —
  wire bodies `{"kind":"tab","action":…}` with
  deny_unknown_fields parsing, a surface table
  (Chrome only), the mandatory chrome envelope +
  dedupe in `on_chrome_tab_body`, one log line per
  body. Verbs: `move {tab, to}`, `move-step
  {tab, dir}`, `open-in-new-pico {tab}` — the URL
  for the launch comes from the shell's own tab
  record, never from a body. The JS↔Rust vocabulary
  is pinned by `tests/tab_pair.rs`; the app wiring
  (core-math routing, accelerator arms, spawn-then-
  close order) is pinned by source scans in
  `tests/tabmove_tests.rs`. One existing pin was
  extended, not weakened: media_tests' enumeration
  of `close_all_panels()` call sites goes 3 → 4
  (open_tab_menu follows on_context_menu's law).

## Tests (Workstream C)

- Rust tests: 701 passed, 0 failed (Workstream B
  baseline 682; +19: tabcmd unit tests — parser
  matrix, insertion/step math incl. every (from,
  gap) permutation property, plan matrix, menu
  labels — the ipc tab_menu_open/close test,
  `tests/tab_pair.rs` vocabulary pin,
  `tests/tabmove_tests.rs` public-surface + source
  scans). DOM suite: 689 checks, all passed — 18 new
  0.41C checks (snapshot-order rendering, reorder
  keeps the active tab, drag threshold/indicator/
  drop body/tail-click suppression, plain-press
  click, Escape cancel, Ctrl+Shift+Page keys, the
  tab menu open/paint/pick/hide flow; jsdom geometry
  is all-zero, so the drag check pins the end gap —
  the gap math itself is pinned in Rust).
- Battery: `./scripts/battery.sh 10` — 10/10
  consecutive PASS (log `battery-0.41.0-wsC.log`);
  iteration 10 includes Windows clippy (only the
  one pre-existing winhttp warning) and the release
  check. Host clippy `--locked --no-default-features
  --all-targets -- -D warnings`: clean. `cargo fmt
  --check` clean. Glow census unchanged: exactly 13
  `var(--glow*)` sites across assets.
- Windows exe: 2,421,248 bytes, sha256
  5f3440d3bc875fd2f82540a3e139b3d70e0dcfb00e0a4b0284c81b1c6ddae8ad
  (differs from Workstream B's 4ddcfa3c… as
  required). Gate v2: PASS — 0 RT_MANIFEST,
  RT_ICON ×8 + RT_GROUP_ICON ×1, static CRT,
  WINDOWS_GUI, under 5 MB. Strings audit:
  `Pico/0.41.0` present, `0.40.3` absent, the tab
  vocabulary (`move`/`move-step`/`open-in-new-pico`,
  `tab_menu_open`) embedded.

## Witness list additions (Workstream C, user's PC)

- Drag a tab in the real strip: the gap line tracks,
  the drop lands where the line showed, the dragged
  inactive tab does not steal focus, and the active
  tab's page never flickers (visibility is id-keyed,
  proven here only at the model/DOM level).
- Ctrl+Shift+PageUp/PageDown with a web page focused
  (the accelerator arms are source-pinned beside
  Ctrl+J/H/D, but only a live WebView2 runtime proves
  the chords are delivered with Shift held).
- Right-click a web tab → "Move to New Window": a
  second Pico opens on the page and the original tab
  closes here; right-click the start tab → "Open in
  New Pico" opens a fresh Pico and nothing closes.
- The tab menu card's position at the window's
  right/bottom edges (ctx_menu_pos clamping is
  unit-tested; the live HWND paint is not).

## Workstream D — Audit Fixes

The fix phase for the Workstream D audit sweep
(`~/workspace/pico-0410/audit-findings.md`: the whole
0.40.3 tree — every core module, `app.rs`,
`downloads.rs`, `dlbackend.rs`, `winhttp.rs`, and the
internal-page JS). Audit outcome summary: **1 Medium
+ 1 Low found in the whole sweep, no Critical, no
High.** The Low was F-2 (typed `pico://downloads`
silently refused), already fixed in Workstream B's
routing funnel. The Medium was F-1, fixed here. Four
suspicions were verdicted (S-1 = F-2; S-2/S-3/S-4
below) and every other lead died against existing
guards — the killed-leads list in the audit file is
the record, and was not re-chased.

### Fix ledger

- **F-1 (Medium — FIXED).** A download that reached a
  final state before the event loop drained its first
  `DownloadsChanged` got NO Finished/Failed toast and
  no `failed_unseen` badge: in
  `core::dlcoord::Coordinator::backend_changed` the
  new id had `prev == None` and both diff arms treated
  that as "not a transition". Fix, in the audit's
  shape: the coordinator now keeps a `seen_ids` set,
  seeded by the silent startup call (every restored
  id) and extended by every later snapshot; after
  startup, a first-sight id already Complete counts
  as a Finished transition and a first-sight id
  already Failed-and-final counts as newly final
  (toast + `failed_unseen`, exactly the normal
  transition's rules, including the retry-pending
  exclusion). Double-notice guard: records the
  coordinator itself creates already-Failed via
  `add_failed` (capture triage rejects, `add-url`
  rejects) are marked seen at creation — their
  Blocked notice already spoke, and their first
  backend sight must not re-notice them as Failed.
  Normal `add` creations are NOT marked: a task born
  Queued that finishes before its first drain is
  exactly the born-and-final case and must notice.
  Regression coverage in `tests/dl_replay.rs` (the
  sequence story 6 never exercised):
  `story6_born_and_final_failed_gets_notice_and_badge`,
  `story6_born_and_final_complete_gets_finished_notice`,
  `story6_restored_failed_at_startup_stays_silent`,
  `born_retry_pending_failed_is_not_final_until_the_retry_clears`,
  `blocked_record_first_sight_does_not_double_notice`.
  Every existing dl_replay story's verdicts are
  preserved (suite green, below).
- **F-A1 (relocation wart from Workstream A —
  FIXED).** Task records store only a file NAME,
  resolved against the CURRENT downloads folder, so
  after 0.41.0's relocation the files finished (and
  `.part` files left) under ≤0.40.3 in
  `<exe>\Downloads` were missed by Open-file /
  Open-folder and by `.part` resume. Fix WITHOUT
  moving/copying user files and WITHOUT rewriting
  task records: (a) during the startup migration
  sequence, when a legacy `<exe>\Downloads` folder
  exists and differs from the new default downloads
  folder, its path is persisted in the app state as
  the new serde-defaulted
  `PersistState::legacy_downloads_dir` (the pure
  decision is `core::state::legacy_downloads_candidate`;
  a fresh install's field stays unset, and the
  sanitizer drops non-local-path values); (b) task
  file resolution goes through the pure helper
  `core::download::resolve_task_file` /
  `resolve_task_part_file` — current folder first
  (it wins when the file exists in both), then the
  legacy folder when set, else current only (a
  legacy folder that no longer exists degrades to
  current-only, since nothing in it passes the
  existence check). Wired into the manager's
  `completed_path` (which also serves Open-folder
  via its parent), the supervisor's `.part` path
  (so the resume existence/size/spot checks and the
  workers all work on the resolved file, and a
  resumed legacy download finalizes beside its own
  `.part`), and the `.part` deletion paths
  (restart, cancel, clear-all, delete-file,
  auto-retry restart, refresh-link restart,
  wind-down cancel). New downloads never land in
  the legacy folder. This supersedes the Workstream
  A "Downloads" note above, which recorded the seam
  as the accepted consequence of the move. Tests:
  `resolve_task_file_prefers_current_when_the_file_is_in_both`,
  `resolve_task_file_falls_back_to_legacy_when_only_there`,
  `resolve_task_file_without_legacy_is_current_only`,
  `resolve_task_file_with_legacy_gone_or_file_nowhere_is_current`,
  `resolve_task_part_file_applies_the_same_rule_on_disk`
  (core, `tests/download_tests.rs`);
  `legacy_downloads_dir_roundtrips_and_defaults_unset`,
  `legacy_downloads_candidate_needs_an_existing_different_folder`
  (`tests/history_state.rs`); and the manager-level
  `fa1_completed_path_and_restart_use_the_legacy_fallback`
  (bin-crate, in `src/downloads.rs` — Windows-target
  compile-checked like T2–T4, not host-run).
- **S-3 (history `drop_top_entry` index edge) —
  FIXED, minimal.** The audit found
  `TabHistory::drop_top_entry` could leave
  `index == None` over non-empty entries (dropping
  entry 0 with forward entries remaining), and the
  next `push_new` would then CLEAR the survivors.
  Decision evidence: `drop_top_entry` still has NO
  production caller in the 0.41.0 tree (grep over
  `src/`: only `history.rs` itself, a doc comment in
  `download.rs`, and tests; the 0.41.0 history page
  removes GLOBAL history via `navstore`, a different
  store), so the only reachable paths are the pinned
  tests — single-entry drop (→ empty, `None`),
  top-entry drop (→ previous), and the 0.40.2
  equal-neighbour splice drop. The minimal fix —
  dropping entry 0 with entries remaining now lands
  on the new entry 0 (`Some(0)`) — is provably
  behavior-preserving for every one of those paths
  (each has either `i > 0` or an empty result, the
  two arms the fix does not touch), and it restores
  the invariant "`index` is `None` only when
  `entries` is empty" for this function. Pinned by
  `drop_top_entry_at_index_zero_lands_on_the_new_first_entry`
  (`tests/history_state.rs`): [a,b,c] at index 0 →
  drop → current is b, Back disabled, Forward
  enabled, survivors kept through a later commit.
- **S-2 (LEAVE — examined, not changed).**
  `core::action::IpcAction` lacks
  `#[serde(deny_unknown_fields)]`: no escalation
  path — parsing runs only behind the conjunctive
  start-tab + start-URL gate, the only producer is
  Pico's own start page, and an unknown field cannot
  change any read field or action routing (see the
  audit verdict; the same item was already recorded
  as informational P3-6 in the 0.32.1 notes).
- **S-4 (LEAVE — examined, not changed).**
  `winhttp.rs` probe: the ranged-GET fallback
  restarts from the original URL rather than the
  HEAD chain's final URL — deliberate, documented
  in-code behavior ("its own redirect chain is the
  honest one to measure") with a bounded per-chain
  redirect budget; the suspected harm (single-use
  signed redirect URLs) is unproven speculation
  with no repo/test/battery evidence.

## Tests (Workstream D)

- Rust tests: 714 passed, 0 failed
  (Workstream C baseline 701; +13:
  5 dl_replay F-1 stories, 5 core F-A1 resolution
  tests, 2 state F-A1 tests, 1 S-3 invariant test —
  host-run — plus 1 bin-crate F-A1 manager test
  that is Windows-compile-checked only, as with
  the existing manager tests).
- Battery: `./scripts/battery.sh 10` —
  10/10 consecutive PASS (log `battery-0.41.0-wsD.log`);
  iteration 10 includes Windows clippy (only the
  one pre-existing winhttp `fetch_range` arity
  warning, identical to every baseline since
  0.40.3) and the release check.
- Host clippy `--locked --no-default-features
  --all-targets -- -D warnings`: clean.
  `cargo fmt --check` clean. Glow census unchanged:
  exactly 13 `var(--glow*)` sites across assets.
- Windows exe: 2,424,832 bytes, sha256
  6ba6570898b982236caa79571163d9b152610e0fb8698dbb190c3291e1d4ce04
  (differs from Workstream C's 5f3440d3… as
  required). Gate v2: PASS — 0 RT_MANIFEST,
  RT_ICON ×8 + RT_GROUP_ICON ×1, static CRT,
  WINDOWS_GUI, under 5 MB. Strings audit:
  `Pico/0.41.0` present, `0.40.3` absent.

## Witness list additions (Workstream D, user's PC)

- A download finished under 0.40.x: on 0.41.0 its
  Downloads-page Open file / Open folder actions
  find the file in the old beside-exe `Downloads`
  folder; an unfinished pre-0.41 task resumes from
  its old `.part` instead of restarting.
- A very fast download (small file from a local
  server, or an instantly refused connection) shows
  its Finished/Failed toast and, on failure, the
  unseen-failed badge — the F-1 shape.

---

# Pico 0.40.2 — Build Notes (2026-11-11)

## What 0.40.2 is

The external-review release. The user handed the
0.40.1 source to another AI for bug fixing and
improvement; its patch set came back as exactly six
changed files (~52 lines). Every hunk was audited
against the baseline code before integration — the
standing rule for external work: credit what is
real, fix what is broken, revert what is harmful,
with evidence. A sibling hunt through the five
touched subsystems followed; it found more than
the external pass did.

Cargo package 0.40.2, `core::DISPLAY_VERSION` is
"0.40.2", transport User-Agent `Pico/0.40.2`.
Cargo.lock diff vs 0.40.1: the own-package version
line only. No new dependencies.

## The external ledger

- `src/core/state.rs` (sync handle) — KEEP. REAL
  Windows bug: `copy_atomic` fsynced the temp
  through a read-only `File::open` handle; on
  Windows `sync_all` is FlushFileBuffers, which
  requires GENERIC_WRITE, so the migration's state
  copy failed there every time while passing every
  Linux-host test. The plausible mechanism behind
  the user's 0.34-era "migration INCOMPLETE
  (1 failed)" log line. The fix opens the temp with
  write access.
- `src/core/addr.rs` (drive-form early return) —
  KEEP-WITH-FIX. The gap was real (`file://C:\…`
  converted to nothing), but the external branch
  also hijacked the slash form before
  percent-decoding, regressing
  `file://C:/my%20file.html` to a literal-%20 path
  and skipping the decoded-control check. Shipped
  fix: normalise the drive form's separators and
  run the EXISTING pipeline — one path, all checks.
- `src/core/download.rs` (`div_ceil`) — KEEP.
  Equivalent for every reachable input (verified
  by an exhaustive grid probe against the old
  formula, including u64::MAX rate/deficit; the
  rate >= 1 invariant is guarded at the call
  site), and removes a theoretical overflow-add.
- `src/core/history.rs` (condition merge) — KEEP.
  Provably identical truth table; a readability
  simplification only.
- `tests/detect_tests.rs` (python fallback) —
  KEEP. No assertion touched; the detector fixture
  now also spawns on Windows hosts. The battery
  host has python3, so behaviour there is
  unchanged.
- `src/core/server.rs` (handle reshuffle) —
  REVERT. Claimed timeout mis-assignment does not
  exist: timeouts are socket-level, and a probe
  with the baseline's exact arrangement showed a
  read on the cloned handle stopped at ~307 ms by
  a 300 ms timeout set on the OTHER handle. Pure
  churn in the host-wall file; not integrated.

## Sibling hunt (two auditors, five subsystems)

Every finding below was re-verified in code (and,
where the host allows, reproduced) before fixing:

- addr: mixed-separator UNC/device forms
  (`\/.\pipe\…`, `/\host\share`, slash-spelled
  NT namespaces) passed both `is_unc_address` and
  `is_safe_local_path` — Windows categorises by
  two separators of EITHER kind. Both gates now
  classify on separator-squashed prefixes and a
  both-separators test. The homogeneous verdicts
  are unchanged (pinned by test).
- addr: `localhost/my page.html`,
  `example.com:8080/a\b` and `[::1]/my page.html`
  minted URLs carrying a literal space/backslash
  that this module's own navigation allowlist then
  rejected. The host:port forms and the bare
  `localhost` shortcut now refuse a dirty tail up
  front and fall through (to search), like the
  bare-port branch already did.
- addr: `*.localhost` took https while every other
  local form took http. It is http now, bare and
  with a port, matching the Shield/Lite locality
  classification.
- history: the commit-start slot replace in
  `record_commit` never checked the finish URL
  against the list — [a,b] + Back-load starting at
  a and finishing on b became [b,b] (found by a
  BFS model check; the witness sequence is a
  regression test). The branch now skips the
  replace when the finish URL is already present,
  and the landing search reconciles.
- history: `drop_top_entry` could splice equal
  neighbours into consecutive duplicates
  ([a,b,a] at b → [a,a]); it now drops the
  shifted-in copies at the seam, mirroring
  `record_replace`'s A6-L6 guard.
- download: `TokenBucket::refill` added before
  clamping — an absurd rate × elapsed product
  overflowed (debug panic, reproduced). It
  saturates now; saturation is the clamped result.
- state: copies were born read-only — `fs::copy`
  copies permissions, and the 0.34.1 fix only
  cleared a PRE-EXISTING destination. `copy_atomic`
  and `copy_tree` now clear the attribute on the
  new copy too (probe: a read-only appdata
  state.json previously migrated to a read-only
  portable state.json).
- state: `save_to` was the one writer the 0.34.1
  retry-poisoning fix missed — it now clears a
  read-only destination before renaming, like
  `copy_atomic` and `copy_tree`.
- state: an oversized state file returned Corrupt
  without the temp-recovery attempt every other
  load-failure branch makes (reproduced: complete
  temp beside an oversized main was ignored). It
  recovers now.
- state: a failed `copy_atomic` left its temp
  behind, and the loader promotes complete temps
  as the newest intent — the failed migration copy
  silently became the state at next load,
  contradicting the withheld report (reproduced).
  Error paths now remove the temp.
- server: error answers on the events and
  reserved-prefix arms advertised a Content-Length
  they never sent for non-GET/HEAD methods (live
  probe: POST → 404, CL 13, 0 body bytes). Only
  HEAD suppresses a body now; every consumer below
  the method gate sees GET/HEAD only, unchanged.
- server: the 416 and resolve-error arms ignored
  write errors and looped keep-alive, against the
  file's own A5-3 rule. Both now close on error.

Checked and cleared by the hunt (evidence in the
round's audit): the server's deadline math
(dribble probe cut at the 5 s head deadline),
host wall, path canonicalisation, response
splitting, panic sweep over request bytes; the
download planner/sanitizer property probes
(2,000 trials each, zero problems) and every
other division site; history's BFS model apart
from the two duplicate findings; state
migration gates and handle/mode sweep (the only
two sync_all sites are the correct ones now).

## Verification

- Rust tests: 639 passed, 0 failed (0.40.1
  baseline 627; +12 regression tests this round,
  none removed or weakened). DOM suite: all
  checks passed in every iteration.
- Battery: `./scripts/battery.sh 10` — 10/10
  consecutive PASS (log `battery-0.40.2.log`);
  iteration 10 includes Windows clippy (only the
  pre-existing warnings) and the release check.
- `cargo fmt --check` clean.
- Windows exe: 2,237,440 bytes, sha256 88392a0c96d33d9cfcd4e9550ddb4dc63b27dc2114733e6d4fdea069d04db74c (changed vs 0.40.1's ef651cdf… before gating). Gate v2: PASS —
  0 RT_MANIFEST, RT_ICON x8 + RT_GROUP_ICON x1,
  static CRT, WINDOWS_GUI, under 5 MB.
- Dev kit: `dist/pico-0.40.2.zip` (Pico.exe +
  README.md).

## Witness list (user's PC)

- Migration: with an old %APPDATA%\\Pico present,
  first run completes the state copy (no
  "migration INCOMPLETE" line for the state file).
- Paste `file://C:\` + a real local page path into
  the address bar — the page opens.
- Type `foo.localhost:5173` against a dev server —
  it loads over http.
- soft98 download flow unchanged: capture, toast,
  tab fate line in pico.log.

# Pico 0.40.1 — Build Notes (2026-11-11)

## What 0.40.1 is

The security release, and the public re-sync: the
first GitHub update since 0.31.1, carrying the whole
0.32–0.35.1 line (the downloads manager and its
rebuild, the downloads page, Pico's own context
menu, live themes) plus this round's audit fixes.
The version jump is deliberate — five feature
releases had accumulated behind a suspended-then-
replaced GitHub account.

The audit ran six parallel fronts (IPC & gates,
downloads engine, WinHTTP FFI, server & host wall,
Shield, shell/Win32 + data & logs, supply chain,
plus an adversarial fuzz probe), and EVERY finding
was re-verified against the code before any fix —
the standing rule from the 0.32.1 round, where a
large share of forwarded findings were false.
Result this time: no criticals, no highs. Everything
below was confirmed real in code before it was
fixed; the accepted-risk list at the end is
explicit, with reasoning, per the mission rule.

Cargo package 0.40.1, `core::DISPLAY_VERSION` is
"0.40.1", transport User-Agent `Pico/0.40.1`.
Cargo.lock diff vs 0.35.1: the own-package version
line only. No new dependencies.

## The fixes (each with its regression test)

1. **A web page could open the downloads page.**
   `window.open("pico://downloads")` rode the
   dedicated-downloads-page addition (0.33) into
   the shared navigation allowlist. The page's
   actions were already dead from a web tab (the
   page gate keys on the host tab), but the history
   view itself rendered — names, sizes, sources.
   `route_new_window` now denies the downloads
   route as an internal page, before the allowlist.
   Test: `the_downloads_page_may_not_open_as_a_tab`
   (both `pico://` spellings + both wry-folded
   `https://pico.downloads` spellings).
2. **The downloads command surface table was
   documentation, not law.** Both app handlers
   accepted all 21 verbs; the page gate's surface
   conjunct compared only the url and status lists.
   The table is now reconciled to the renderers
   that actually ship (the popover DOES emit
   `remove` on finished rows — the old table said
   page-only) and enforced as a gate in both
   handlers: the downloads page cannot reach
   toolbar-only verbs, the toolbar cannot reach
   page-only ones like `delete-file` or
   `clear-all`, and `copy-link` is marked
   client-only (rendered locally, never sent).
   Tests: `surface_matrix_matches_the_shipped_
   renderers`, `per_file_emissions_stay_on_
   their_surface`, source-scan pins on both
   handlers.
3. **A toolbar downloads body without its delivery
   envelope skipped every rejection.** No envelope
   meant no dedupe AND no retired-boot rejection,
   so a replayed pre-restart body could act on a
   live task. The envelope is now mandatory:
   missing or mistyped is dropped with a log line.
   Test: source-scan pin on the handler span.
4. **A context-menu pick was never checked against
   the menu.** Any known item id was honored from
   any tab while any menu was open. A pick now
   requires an offered AND enabled item in the
   open menu's own list. Test: source-scan pin on
   the exact predicate.
5. **The content IPC handler had no size cap.**
   The chrome transport had one since A1-L5; the
   content handler parsed unbounded bodies. Same
   `MAX_NAV_BODY_BYTES` cap, same silent drop.
6. **The transport trusted a server's range
   promise.** `fetch_range` counted but never
   capped a bounded range's body: a lying 206
   streamed forever, and the resume spot-check
   sink buffered it all into RAM. The stream now
   aborts past `end - start + 1` bytes with a
   dedicated `RANGE_OVERFLOW` cause the manager
   maps to its existing contract-violation class.
7. **`split_url` let raw controls through.** Host
   and path bytes ≤ 0x20 / 0x7f (space, tab, CR,
   LF) reached WinHttp verbatim — a CRLF-bearing
   URL at the request-line boundary. Refused at
   parse, which every transport entry point
   funnels through. Encoded forms (%20) unaffected.
   Tests: three new winhttp unit tests (executed
   verbatim in a scratch probe — bin-crate tests
   type-check under the xwin all-targets check but
   the host harness skips the gui-gated bin; the
   probe run passed every assertion).
8. **Referer and If-Range scoping.** The full page
   URL — path and query, token included — went out
   as Referer on EVERY redirect hop and to a
   refresh-link's new host, and If-Range matched
   on host alone (port-blind). Referer is now
   browser-style: full only to its own origin
   (scheme, host, port), origin-form elsewhere, so
   referer-gated hosts still see their host. Both
   validators in a resume probe must now match the
   CURRENT url's full origin tuple.
9. **Resume trusted the segment map over the
   file.** A `.part` whose length no longer
   matched the recorded total (external truncation,
   a copy mishap) was extended and its segment map
   believed — a silent zero-splice of the 0.32.1
   class through a new door. A probed resume now
   compares the part's actual length against the
   recorded total and replans from zero on
   mismatch.
10. **Invisible characters survived filename
    sanitization.** `sanitize_file_name` filtered
    controls and separators but kept Unicode
    category Cf — U+202E (right-to-left override)
    turns `invoice‮fdp.exe` into a displayed
    "invoiceexe.pdf". The Cf set (bidi controls,
    zero-width, word joiner, BOM) is now stripped;
    legitimate non-Latin names are untouched
    (Persian test name survives byte-for-byte).
    Test: `sanitize_strips_bidi_and_zero_width_
    format_chars`.
11. **Refresh-link results landed unverified.**
    The probe thread applied its outcome under the
    final lock without re-checking the task it
    described; a mid-probe pause or restart was
    overridden. The apply now revalidates
    refreshing flag + url + status against the
    probe snapshot — the same pattern the
    auto-retry promotion already used — and drops
    stale outcomes with a log line.
12. **Finished downloads kept their session
    cookies on disk forever.** Capture cookies were
    written into the task record and never cleared;
    nothing reads a finished task's cookies. They
    are now cleared at completion. (Cookies on
    UNFINISHED tasks stay: resume needs them, same
    trust boundary as the browser profile itself.
    The finished task's URL also stays — it is the
    history record and the copy-link source.)
13. **Ctrl+J fired on Ctrl+Shift+J and Ctrl+Alt+J.**
    The accelerator leg tested Ctrl and the key
    only. Shift and Alt now veto, so the browser's
    own modified shortcuts keep their meaning.
14. **The task-record cap had a back door.** The
    1000-record cap was enforced on add but not on
    disk load; a hand-grown state file was believed
    whole. Oversized loads are truncated
    (unfinished first, then newest) at load.
15. **Name-exhaustion could overwrite a stranger's
    file.** `dedupe_file_name` gives up after
    ~100k collisions and returns a name that
    exists; three finalize/add paths renamed onto
    it. All three now refuse with a clear failure
    instead, and the server-rename path keeps the
    current name on exhaustion.
16. **The local server's device denylist missed
    CONIN$/CONOUT$.** Added; the Windows device
    namespace is now fully covered.
17. **Embedded-list decompression was unbounded.**
    Capped at 32 MiB per list (a real list is
    ~2 MB): a corrupt or hostile .gz cannot
    allocate without limit on the compile thread.
    (Public tree: the same discipline already
    exists as the acquire path's per-source byte
    budgets + SHA-256 gate in `core::lists`.)
18. **One production `expect` under
    `panic = "abort"`.** The download-starting
    cookie leg could panic the process on a
    WebView2 view that vanished mid-event. It now
    degrades like every sibling path (log, empty
    cookie set, return).

Also corrected while in the area: the cookie
capture comment claimed scheme/port withholding
the code never did (the copy filter is
host-relatedness; the comment now says so), the
surface table's module doc no longer calls itself
"single source of truth" for something it didn't
enforce, and the BUILD_NOTES claim of a "vendored
wry" (0.31 forensics section) — wry 0.53.5 is the
stock crates.io package, byte-identical to the
registry copy; this round's audit diffed it. The
NavigationCompleted note stands; the word
"vendored" does not.

## Supply chain

The shipped Windows graph is 104 crates. cargo
audit's own network path is blocked on this VM
(the advisory DB fetch fails; crates.io returns
403 through the proxy), so the scan ran directly
against a git clone of the RustSec advisory DB:
**zero vulnerable advisories** cover any crate or
version in the graph. One unmaintained advisory
(`instant 0.1.13`, transitive via tao) — accepted,
as before. The one lock delta vs the public
0.31.1 tree is `flate2` + its three deps, from the
0.32 Shield work, documented in AUDIT.md. The fuzz
probe re-ran clean: 574 adversarial inputs across
the parsers, debug and release, zero panics, zero
aborts — and it re-verified the five 0.32.1 engine
fixes and the 0.34.2 dedupe polarity in passing.

## Accepted risks (explicit, with reasoning)

- **No single-instance mutex.** Two Picos can run;
  state temp names are per-pid and saves are
  atomic renames, so the worst case is a
  last-writer-wins state file — but a mutex is new
  Win32 startup code, and Pico's standing law is
  never ship a startup path that cannot be
  witnessed from here. It lands in a future round
  WITH a witness test on the user's machine, or
  not at all. Known limitation, recorded.
- **Log redaction keeps the path.** Pico's log
  strips query, fragment and credentials from
  every URL it prints (pinned by tests); page
  PATHS remain, because they are the diagnostic
  content of the lines that carry them and the log
  never leaves the user's machine. Downloads
  engine logs are host-only. Anyone describing
  Pico's redaction as "host-only everywhere" is
  describing a different (worse) design.
- **No total wall-clock deadline on a download.**
  Deliberate: the user's connection is ~300 kbps
  and a big file legitimately takes hours. The
  stall floor (no 1024 B in 60 s closes the
  connection), the 5 s receive-gap timeout and
  segment stealing bound the damage a dead server
  can do.
- **`pico://` subframes.** Only the downloads
  route needed the popup fix; subframe loads of
  internal pages stay allowed because the action
  and downloads gates key on the HOST tab's flags,
  never the frame's — a framed internal page can
  display, never act.
- **Remaining as designed:** the dedupe window's
  512-entry eviction (double-fire protection, not
  authentication); the probe's redirect budget
  being per chain (two chains run per refresh —
  bounded, just 2× the module doc's shorthand);
  the scriptlet engine's in-page `RegExp` (pinned
  list data only, argument-capped); a planted
  `.part` symlink (needs local write access AND
  privileged symlink creation on Windows — a local
  attacker with both has shorter paths).
- **Mutex-adjacent, server-adjacent:** the static
  server's root token is 64-bit and loopback-only
  with no guessing oracle; its canonicalize→open
  window is reachable only by a process that can
  already write the served folder.

## Verification

- Battery: 10/10 consecutive PASS
  (`battery-0.40.1.log`). Per iteration: 627 Rust
  tests + 635 DOM checks. Iteration 10 adds the
  Windows clippy (3 pre-existing warnings, none
  in touched code) and the release check.
- Build: `cargo xwin build --release --locked
  --target x86_64-pc-windows-msvc`.
- Dev exe: 2,236,416 bytes, sha256
  ef651cdf50a4aa4f96685c27e9dea7f28847a16732595a971e1ecc1fb87cc3c0
  (changed vs 0.35.1's 990c6aff… as required).
  Gate v2 PASS: 0 RT_MANIFEST, RT_ICON ×8 +
  RT_GROUP_ICON ×1, 0 CRT imports, WINDOWS_GUI.
  `dist/pico-0.40.1.7z` 1,170,739 bytes.
- `cargo fmt` clean; the fmt pass touched only
  this round's edited regions.

## Still owed (witnesses, the user's machine)

Everything on the 0.35/0.35.1 witness list, plus:
picking a theme with the Downloads page open is
now expected to recolor it live (0.35.1's channel)
— that witness is still the open one. New in this
round to witness: nothing user-visible changed
behavior except the downloads page refusing to
open as a popup tab (invisible unless a site tries
it) and Ctrl+Shift+J no longer opening downloads.

# Pico 0.35.1 — Build Notes (2026-11-11)

## What 0.35.1 is

Two user-screenshot defects, one subsystem rewrite.
(1) Changing the theme did not reach an OPEN
internal page: sitting on the Downloads tab, picking
"Indigo Amber" recolored the chrome instantly while
the page behind stayed Forest — it only ever got its
colors from the theme current when it loaded.
(2) The start page's Downloads card showed a green
"0 ACTIVE" badge over "No downloads yet" — a badge
claiming activity in an empty state is noise that
reads as a bug.

Cargo package 0.35.1, `core::DISPLAY_VERSION` is
"0.35.1", transport User-Agent `Pico/0.35.1`.
Cargo.lock diff vs 0.35: the own-package version
line only. No new dependencies.

## Defect 1 — ONE live theme channel for internal pages

The old patchwork had THREE theme delivery paths,
and the downloads page was only on the two weakest:

- **Init bake (both pages, kept):** theme/palette
  embedded in the page's INIT JSON at serve time;
  the page applies it at boot. Always current at
  load — first paint was never wrong.
- **Start-only live push (the bug):**
  `push_theme_to_start_tabs` evaluated
  `__picoSetTheme` only into tabs flagged
  `is_start`. `set_palette` / `toggle_theme` called
  exactly that. A downloads tab is `is_downloads`,
  never `is_start`, so an open Downloads page NEVER
  received a theme change.
- **Piggyback (accidental):** the downloads page
  also applied theme keys merged into its DATA
  snapshot — but `push_downloads_page` only runs on
  downloads events, never on a theme change. The
  page recolored only when unrelated download
  activity happened to push (which is why it
  sometimes "fixed itself" later).

The rewrite leaves exactly one channel with two
legs, both fed by the app's one theme state
(`persist.theme` + `persist.theme_id`):

- **Serve leg:** INIT still embeds the current
  theme — for the downloads page it is now merged
  at assembly time in `render_downloads` from
  SharedData (the way `render_start` always did),
  NOT inside the downloads snapshot.
  `downloads_snapshot_json` is a plain snapshot
  serialize again and the page-push signature no
  longer folds theme in: data pushes never carry
  theme anymore.
- **Live leg:** `push_theme_to_internal_tabs`
  targets `is_start || is_downloads` and pushes ONE
  versioned payload through the existing eval path:
  `window.__picoTheme({"v":1,"mode":…,"palette":…})`.
  Called from both `set_palette` arms, from
  `toggle_theme`, and once more at PageLoad
  Finished for internal tabs (the heal: a page that
  loaded mid-change converges). The closed-set sink
  validation (mode ∈ {dark, light}, palette ∈ the
  palette table, refuse-and-log otherwise) is kept
  verbatim from AUDIT 0.31 A3-F10. Regular web
  pages are never touched; the local-file viewer
  injects only a live-reload script (no Pico
  theming), so it is not part of the channel.

Page side: both internal pages expose exactly one
theme entry point, `window.__picoTheme(payload)`.
It validates the version, the mode, and the
palette id against the page's OWN stylesheet (a
cached scan of the `[data-palette="…"]` blocks the
generated themes CSS embeds; shape-check fallback
if no sheet is readable), and on any failure
changes nothing, warns once on the console, and
returns false — it cannot throw. Deleted:
`window.__picoSetTheme` on both pages (the shipped
exe contains zero hits for it) and the theme
application inside `__picoSetDownloads`. The
internal pages' CSP posture is unchanged — the
push rides the host's evaluate-script channel the
other `__picoSet*` pushes already use, not any new
page-side trust.

Proof is computed, not attribute-deep: the DOM
suite drives the REAL hook on an already-loaded
page for BOTH internal pages and asserts the
resolved `--bg` token actually changes
(ember/dark `#0B132B` → forest/light `#D8DBCD`,
royal/dark `#0B1F3B` for the downloads page), in
the same window/document (no reload), plus seven
malformed-payload cases per page that must change
nothing and throw nothing. Fixtures now INIT both
pages under ember/dark (non-default), so boot
theming is proven too.

## Defect 2 — the Downloads card tells the truth

The badge was static HTML: the word "active" sat
in the card markup in the green `.scheme` style,
and the renderer only ever rewrote the number and
the sub-line. The fix is a status MODEL, computed
once in the coordinator, not ad-hoc JS:
`dlcoord::card_view` folds the snapshot's task
views + `failed_unseen` into a `DlCardView`
(`{state, count, speed_bps, paused, queued,
newest_name, unseen}`) with first-match precedence:

1. no tasks → `empty`
2. `failed_unseen > 0` → `failed` (unseen; count =
   the unseen count, the toolbar-badge law)
3. any Downloading → `active` (count = running;
   `speed_bps` = the sum of the running tasks'
   meter values — the same sum the popover header
   shows)
4. any Paused/Queued → `waiting`
5. any Complete → `finished` (newest by
   `added_ms`)
6. only seen-failed remain → `failed` (unseen =
   false)

`DlSnapshot` carries it as `card`; `push_state`
mirrors it into SharedData (so `render_start`'s
INIT is current) and pushes it live to start tabs
via `__picoSetDownloadsCard`, signature-gated on
the serialized card so speed ticks update the card
at popover cadence and idle pushes cost nothing.
The old `{active, total}` counts and their
SharedData fields are deleted. The page renders
the model and nothing else:

- empty → the number/badge row hides entirely;
  "No downloads yet" stays.
- active → green `N` + `ACTIVE`, sub-line
  "Downloading · <combined speed>/s".
- waiting → muted `PAUSED` / `QUEUED` / `WAITING`,
  sub-line "2 paused · 1 queued" (only the nonzero
  parts, pluralized).
- failed → `FAILED`: warn color while unseen,
  muted once seen; sub-line is the newest failed
  file's name (textContent only).
- finished → muted `FINISHED`; sub-line is the
  newest finished file's name.

## Sweep

Same two disease classes, checked everywhere:

- **Stale settings in open internal pages:**
  the start page's services / others / recents /
  recents-hidden / stats each already ride a
  dedicated live push — cleared with code
  evidence. Shield/Lite toggle STATE is not
  rendered on the start page at all (only their
  blocked counts, inside the pushed stats view) —
  cleared. The downloads page's settings ride its
  snapshot pushes, whose signature covers them —
  cleared. Theme was the only stale consumer.
- **Badges/labels in empty states:** chrome's
  downloads count + failed pills (gated on > 0),
  services badge (> 0), shield badge (on AND
  > 0), recents count (blank at 0, section head
  hidden), "other listeners" line (hidden at 0),
  the stats band (a dedicated zero state), the
  downloads page's count line ("No downloads") —
  all already gated or zero-stated; the start
  Downloads card was the only offender.

## Verification

- Rust tests: 621 passed, 0 failed (614 at 0.35 →
  +7: the `card_view` unit tests — all five states,
  precedence, speed sum, newest-by-`added_ms`,
  seen-failed fallback — beside the dlcoord tests;
  the snapshot JSON pin now covers the card's
  seven fields). Clippy: no new warnings (2
  pre-existing lib warnings untouched:
  download.rs `manual_div_ceil`, history.rs
  `if_same_then_else`); `cargo fmt --check` clean;
  `cargo xwin check` clean for the Windows target.
- DOM checks: 635 passed, 0 failed (592 at 0.35 →
  +43 net: the two-page live-theme block with real
  computed-token assertions, the defensive-payload
  matrix, and the card's five states including the
  empty-state pin of the user's exact screenshot
  and a metacharacter name landing as pure text).
- Battery: `./scripts/battery.sh 10
  battery-0.35.1.log` — 10 consecutive PASS on the
  final tree (iteration 10 includes Windows clippy
  and the release check).
- Exe: release build 2,224,128 bytes (under the
  5 MB cap), sha256
  990c6aff854ad4c1e75ef6461504848d034730e8efb73c2a58614bb4ce7dc9f1
  — changed vs 0.35's
  3405980c0751b8d1e9ee1ccc98465461902b6f006ae61b53a9a38c2fb6ab9852
  before gating, per the stale-exe rule. Gate v2
  PASS on the build exe and on the exe extracted
  from the shipped zip (inner hash identical):
  0 RT_MANIFEST, RT_ICON ×8 + RT_GROUP_ICON ×1,
  0 CRT imports, WINDOWS_GUI. Strings audit:
  transport UA `Pico/0.35.1` present,
  `__picoTheme` and `__picoSetDownloadsCard`
  embedded, `__picoSetTheme` absent.
- dist/pico-0.35.1.zip (1,365,578 bytes): Pico.exe
  + README.md.

## Still owed (witnesses, the user's machine)

- Open the Downloads page, open the theme picker,
  pick any other palette: the PAGE recolors with
  the chrome, no reload. Flip dark/light the same
  way. Glance at the Start page while doing it.
- Start page Downloads card: empty list → no
  badge, just "No downloads yet"; start a download
  → green ACTIVE with the live combined speed;
  let one fail → warn FAILED until the Downloads
  page is visited.
- Carried from 0.35: the soft98 redirect flow
  (tab closes itself), Ctrl+J with a web page
  focused, the right-click menu, Refresh link on
  an expired token, a completion through the CDN,
  a cookie-carrying download, pause → quit →
  relaunch → resume.

---

# Pico 0.35 — Build Notes (2026-11-11)

## What 0.35 is

The downloads polish round, from the user's verdict
on 0.34.2. His screenshot proved the dedupe repair
worked — the popover opened, with his task in it —
and indicted everything else: the row showed a name,
a status word and a sliver of bar, but no progress
numbers and no speed; the task sat "paused" with no
explanation on screen; and the site-opened download
tab STILL did not close. Five workstreams: rich
popover rows (WS1), the tab close hunted to its real
cause at last (WS2), the paused task explained and
the invariant hardened (WS3), fresh research folded
in (WS4), and a sweep of the small surfaces (WS5).

Cargo package 0.35.0, `core::DISPLAY_VERSION` is
"0.35", transport User-Agent `Pico/0.35`. Cargo.lock
diff vs 0.34.2: the own-package version line only.
No new dependencies (one added feature flag on the
existing windows-sys dependency — see WS5's Ctrl+J).

## WS1 — the popover row shows the download

The data was never missing; it was unusable. The
snapshot carried speed and ETA only inside a
preformatted `detail_text` string, and the popover
crammed that string onto a 22px sub-line shared with
the host, the connections select and three buttons
in a 360px card — on the user's screenshot it is
squeezed to nothing. 0.35 carries the numbers AS
numbers and gives the row room:

- **Producer:** `DownloadView` / `DlTaskView` gain
  `speed_bps` (the engine meter's rate, bytes/sec,
  present only while downloading) and `eta_secs`
  (the same `eta_seconds` value the detail text was
  built from, computed once in `views()`). The wire
  shape is pinned by a serialization test.
- **One formatter:** dlrows.js gains exact ports of
  `core::stats`' humanize_bytes / humanize_duration
  (the start page's existing JS mirrors, byte-for-byte
  — including the half-up tie behavior, fixture-pinned
  at 12250 → "12.3 KB") and ONE `progressText(task,
  tasks)` that both surfaces render: running =
  "12.4 MB of 21.4 MB · 58% · 1.2 MB/s · 9 s left";
  paused = bytes + percent (never a speed — the
  renderer keys speed off status, not field
  presence); queued = bytes + waiting text (below);
  failed = bytes (the reason rides the error line,
  as before); done = the size. Unknown total: bytes
  only — no percent, no ETA, no invented numbers.
  Legacy snapshots without the new fields fall back
  to `detail_text`, so old pushes render unchanged.
- **Row anatomy (popover):** line 1 (24px) = name +
  status word + the state's own icon buttons
  (pause/resume; for done rows Open file — new here —
  folder, remove; no hidden placeholder buttons
  anymore); line 2 = the bar; line 3 (22px) = the
  progress text + the per-task connections select.
  A failed task with a retry pending shows the
  countdown in line 3, as before. The host leaves
  the popover row (the page keeps it). The page
  row's detail line now shows the identical
  `progressText` string — DOM parity checks pin the
  same string on both surfaces for shared fixtures.
- **Queued position:** "Waiting · 1 ahead" /
  "Waiting · next up", derived from the snapshot's
  own order (newest first; the scheduler starts the
  oldest queued first, so the ahead-count is a fact
  about the queue, not an estimate). The page
  computes it against the FULL list, not the
  filtered subset (`renderTaskList` takes the full
  task list as an optional third argument).
- **Pricing:** the row is now honestly 76px
  (`DL_ROW_H` 66 → 76: 16 pad + 24 + 5 + 4 + 5 +
  22), and the popover's footer — in the DOM since
  0.33 — is priced for the first time
  (`DL_POPOVER_FOOT_H` = 28 + one gap; the WS5 audit
  found it missing from the formula entirely).
  Layout witnesses recomputed: (0 tasks) → 160,
  (1 task) → 206, cap still the 480-card ceiling.

## WS2 — why the tab STILL didn't close (root cause)

Not the fate machinery — the fate never existed.
His flow (from his own 0.34 pico.log): the page tab
opens a NEW tab at the file host (dl2.soft98.ir);
that URL 302-redirects to a CDN host; the CDN serves
the file. At capture, the tab's live URL still held
the PRE-redirect address while WebView2's
`DownloadOperation.Uri` handed the capture the
POST-redirect one — proven from his log, not
theory: the "download cookies skipped (cross-site)"
line can only fire when the tab URL and the captured
URI sit on opposite hosts, i.e. on opposite sides of
that redirect. Two exact-string matches then failed:

1. `unwind_action`'s first gate — live ≠ captured →
   fate **None**. Silently: a None fate produced no
   log line anywhere, which is why four rounds of
   logs showed a task and no fate.
2. `navigation_finished`'s swallow — exact URI
   equality against the captured URI, while the
   Finished event carries the live Source at
   completion (the other end of the redirect), so
   the captured navigation's completion could also
   miss its swallow and pose as a page commit.

Every prior round's tests passed because no replay
modelled a redirect: live == captured by
construction. The diagnosis also cleared, with code
evidence, the suspects that were NOT it: adoption
mechanics (the tab IS flagged adopted, in time),
landing timing (the cross-site landing is
synchronous — zero event-loop turns between capture
and fate execution), close reselection (the opener
regains focus correctly), and triage.

**The fix — a capture carries a CHAIN, not a
string.** Each tab now keeps a pending-navigation
record (in `apply_url_event`, before the reducer):
the URLs the current unsettled navigation has
occupied — reset at each CommitStart (snapshotting
the tab's adopted flag), appended on Source while
loading, cleared when a navigation really commits.
`CaptureFacts` carries the chain (pending chain +
live URL + captured URI); `unwind_action`'s first
gate becomes live ∈ chain, and committed/previous
entries are judged after dropping chain members (a
redirect hop is not a page the user read). The
swallow accepts any chain member. `fate_step` is
byte-for-byte UNCHANGED — the epoch guard is still
what refuses a fate when the user genuinely
navigated the tab elsewhere, and a genuine
navigation's URL is by construction outside the
chain. One bounded addition: a committed entry that
is itself a chain member with no real page behind
it is the captured navigation's own artifact (the
WebView2Feedback #5172 completion landing before
capture) — the facts present it as no-page and take
the origin from the chain's adopted snapshot.
**No silent fates:** every capture now logs
`download capture {token}: fate decided <Kind>` at
capture time; drops and executions keep their
existing lines.

Proof: six new replays in tests/dl_replay.rs —
replay 1 is his exact flow (hop-0 live URL, CDN
captured URL, chain of both → the task is created
from the FINAL URL and the effects contain
CloseTab); two-hop variants swallow finishes at the
final AND at an intermediate hop; the
genuine-navigation replay still drops the fate with
exactly one "tab moved on" log; and a revert check
was run — restoring the old exact-match gate makes
replays 1/3/4/5 fail (4 and 5 fail on the missing
drop log: the original silence) while the
no-redirect control keeps passing.

## WS3 — the "paused" task: verdict and hardening

**Verdict: no fresh capture can land in Paused.**
The audit enumerated every `TaskStatus::Paused`
write in the tree (six sites): three are the user's
own pause verb; one is the shutdown persist (disk
copy only); one is the restore sanitize; one is
flagged below. A fresh capture is born Queued in
`add()`, which calls `schedule()` synchronously —
with a free slot the task is Downloading before
`add()` returns; with no slot it waits as Queued.
Restored tasks cannot starve it: every slot count
in the tree counts Downloading only. The
screenshot's task is the by-design restore path:
it was Downloading/Queued/Paused when the earlier
build closed (a clean close persists running tasks
as Paused), and restoring parks it as Paused. That
is also why it carried no blame — but the popover
gave him no way to see it was a restored task with
its bytes intact; WS1's paused line (bytes +
percent) is the visible half of the answer.

**Hardening (the one latent hole):**
`wind_down_stopped`'s dispatch was
`match control { Cancel => remove, _ => Paused }` —
a catch-all that reads Control::Run as Pause. It
was unreachable as a cause (every Pause/Cancel
writer invalidates the run's generation gate in the
same lock hold, and the function's first line
returns when the run is no longer current), but the
invariant held by argument, not by construction.
The arms are now explicit: Pause → Paused, Cancel →
remove, Run → log and write nothing. Test T4 calls
it with control=Run and a current token — it FAILED
against the catch-all (`left: Paused, right:
Downloading`) and passes after the fix; T2/T3 pin
birth-and-scheduling (free slot → Downloading at
once; five restored Paused tasks + max_concurrent 3
→ three fresh tasks run, the fourth Queues, none
ever Paused); a core table test pins the restore
mapping for all five stored statuses.

## WS4 — research, applied and rejected

Fresh research (full draft kept at
research-0.35-draft.md; the 2026-11-10 study stands
for the rest). What it settled: Chrome's PAGE status
text includes live speed while its BUBBLE shows only
sizes + a single-unit ETA — no speed, no percent
text (percent is the bar and the toolbar ring);
Firefox's panel status computes speed in (its inline
rendering in the current release could not be
settled from primary sources); IDM's "chart" is a
per-segment progress map, not a speed graph, and no
researched product — Chrome, IDM, Gopeed, FDM —
shows a queue position or an aggregate speed in a
panel header. On the redirect question the research
is explicit: NO primary source names the chain
"new tab → cross-origin redirect → download → tab
closes"; "still closes" is an inference from
Chromium's mechanism docs (a server redirect is one
NavigationRequest and commits nothing; a download
commits nothing) — Pico's behavior follows that
rule, and WS2's fix is grounded in the code-level
diagnosis of his own log, not in the inference. The
empirical settlement is the witness on his machine,
below.

Applied: the page/popover line is sizes + percent +
speed + single-unit ETA in the house formats
(Chrome's two-layer split puts speed on the page;
Pico shows it in the popover too — see the override
note); paused rows lead with the state word
(Chrome's pattern); unknown totals show sizes only
(Edge's documented behavior, and Pico's existing
rule); no auto-open on start (existing law,
research-confirmed); Pico's speed meter already
smooths over a rolling window — a 5-second one,
the same window Gopeed's display practice uses.

Rejected for 0.35, with reasons: a per-task speed
graph/sparkline (no product ships one; IDM's chart
is a segment map; cost without precedent); a
segment/block map surface (the engine tracks
segments, but no surface this round); a determinate
progress RING on the toolbar button (Chrome's
model, noted as a future candidate — the button
keeps its count badge and pulse this round);
sortable Time-left/Transfer-rate COLUMNS on the
page (IDM/FDM are tables; Pico's page is a card
list and already prints both figures in the row);
an auto-opening partial popover when all downloads
finish (Chrome's gated, impression-capped variant
is still an auto-open; Pico's finished toast is the
signal).

Overridden by the user's explicit order (recorded
plainly): the popover shows speed and percent text
(Chrome's bubble omits both — his order, and the
manager-class precedents agree: Gopeed's rows show
the current speed, IDM's list shows Transfer rate);
the popover header shows the combined speed at ≥2
running (no researched header does — it is one
small number from the same meter); queued rows show
the ahead-count (no product shows a position — the
count here is the scheduler's exact order, not an
estimate).

## WS5 — the sweep

Verified working, no change: Esc closes the popover
(the one chrome-document handler, same as every
panel); settings reflect across surfaces from the
one coordinator mirror (neither surface keeps a
local copy); context-menu Download Link/Image
captures fire the Started toast through the same
`on_download_starting` as link clicks; page
search/filter re-renders from the snapshot (never
DOM-walks), so the richer rows are safe by
construction; SHA-256 shows on done rows on the
page, computed at completion before the record
persists.

Fixed:
- **A Failed notice could be buried or wiped
  unseen (real bug).** `set_notice` overwrote the
  single notice slot unconditionally — a later
  Started/Finished/Blocked notice silently replaced
  an open Failed one; and the panel-close paths
  (`close_all_panels`, Teleport, Palette) retired
  any open notice, Failed included. Now: an open
  Failed notice is never replaced by a non-Failed
  kind (Failed replaces anything, including
  Failed); panel closes use a guarded retire that
  leaves a Failed notice open; and `page_visited`
  actually retires the notice now (two app comments
  claimed it did; the code only zeroed the badge).
  The Failed notice's exits are the intended ones:
  its own click, Dismiss, the downloads popover
  opening, the downloads page being visited, or a
  newer failure. Six coordinator tests pin the
  machine.
- **Ctrl+J only worked from the chrome's focus
  (real bug).** The only binding lived in the
  chrome document; with a web page focused the
  chord never reached it. Content views now carry
  an AcceleratorKeyPressed registration at the one
  `build_content_view` choke point (the GotFocus
  attach is its template): Ctrl+J KEY_DOWN is
  handled before the page sees it and opens the
  downloads page through the same
  `open_downloads_page` path as the toolbar route,
  logging `downloads page opened (ctrl+j)`. A
  source-scan pin locks the registration's place,
  like the other attaches. One windows-sys feature
  flag added (Win32_UI_Input_KeyboardAndMouse) for
  GetKeyState; no new packages.
- **Aggregate speed (absent):** the popover header
  shows the combined speed while ≥2 downloads run —
  the sum of the same per-task meter values the
  rows show, computed in the chrome from the
  snapshot's `speed_bps`.
- **Clear finished (absent from the popover):**
  the footer gains Clear finished, shown only when
  a done task exists (the page's button is gated
  the same way). The verb is `clear-completed`;
  its dlcmd surfaces widen to include Chrome, as
  does `open-file` (the popover's new done-row
  button) — the pair test passes on the widened
  table with the chrome emissions as its other
  half, and a new unit pin locks the two widened
  surface sets.
- **Open the downloads folder (absent as a
  control):** popover footer + page settings row
  gain a folder button posting `open-folder` with
  NO id — the verb's id was already optional and
  the coordinator already resolves the bare folder;
  the buttons are the only new code.
- **Empty state copy:** the popover's bare
  "No downloads yet" becomes "Downloads you start
  will show here" (one line; the 30px pricing
  holds). The page gains a Queued status chip (the
  filter is generic status equality; queued tasks
  previously appeared only under All).

## Evidence

- Rust tests: 614 passed, 0 failed (595 at 0.34.2 →
  +10 WS2 replays/matrix/fate tests, +9 this round's
  remainder: the dlcmd surface pin, six notice-machine
  tests, the completed restore table, the Ctrl+J
  source-scan pin). The three new manager unit tests
  (T2/T3/T4) live in the bin crate, which the host
  suite skips; they were built for the Windows target
  and executed under Wine (3/3), as were the Windows
  builds of every suite (dl_replay 26/26,
  download_tests 110/110, media_tests 26/26,
  dl_pair 3/3). DOM checks: 592 passed, 0 failed
  (560 at 0.34.2 → +32: the rich-row fixtures, the
  formatter pins, parity, aggregate, footer and page
  gating checks).
- `cargo fmt --check` clean. Clippy (Windows target):
  no new warnings — the same three pre-existing ones
  in untouched files (core/download.rs div_ceil,
  core/history.rs identical blocks, winhttp.rs
  fetch_range arity).
- Battery: `./scripts/battery.sh 10 battery-0.35.log`
  — 10 consecutive PASS on the final tree (iteration
  10 includes Windows clippy and the release check).
- Exe: release build 2,215,424 bytes (under the
  5 MB cap), sha256
  3405980c0751b8d1e9ee1ccc98465461902b6f006ae61b53a9a38c2fb6ab9852
  — changed vs 0.34.2's
  fe38e247d6ba61d5b499b7e52856bd5d43bc1df1630970cbfc337607228ef5f3
  before gating, per the stale-exe rule. Gate v2
  PASS on the build exe and on the exe extracted
  from the shipped zip (inner hash identical):
  0 RT_MANIFEST, RT_ICON ×8 + RT_GROUP_ICON ×1,
  0 CRT imports, WINDOWS_GUI. Strings audit:
  transport UA `Pico/0.35` present, `Pico/0.34.2`
  absent; `progressText`, `speed_bps`, the
  fate-decided log format and the ctrl+j log line
  all embedded.
- dist/pico-0.35.zip (1,363,038 bytes): Pico.exe +
  README.md.

## Still owed (witnesses, the user's machine)

Short list, by design:

- **The soft98 flow, once:** on the page, click the
  download whose host redirects (dl2 → CDN): the
  task starts, the toast names the file, and the
  site-opened tab CLOSES by itself. If anything
  differs, pico.log now carries the paper trail —
  `download capture N: fate decided …` at capture,
  the fate's drop line or `download tab fate: …` at
  landing.
- **One look at a running popover row:** while a
  download runs, the row under the button shows
  bytes of total · percent · speed · time left, and
  the header shows the combined speed when two run
  at once.
- Carried: Ctrl+J pressed while a web page has
  focus (the page must not see the chord; the
  downloads page opens); a failed download's toast
  surviving a second download's start and a click
  into the page; the 0.34.2 carried list (context
  menu, refresh-link on an expired token, a
  completion through the CDN, the cookie-carrying
  download, pause → quit → relaunch → resume).


# Pico 0.34.2 — Build Notes (2026-11-11)

## What 0.34.2 is

The downloads UI repair. The user's field report on
0.34.1, from a fresh session: "cant see the download,
cant see is it started, cant open the page, cant hit
the button for downloads on top bar." The toolbar
button, the popover, the downloads page's chrome
doors — the whole chrome-side downloads surface was
dead on his machine while every check here was green.

The working hypothesis was render fragility: a JS
exception while painting his REAL downloads data
(unknown totals off a chunked Iranian CDN, Persian
names, long server errors) killing the chrome's
snapshot application mid-flight. Three parallel
investigations ran: a hostile-snapshot corpus against
the real renders, a producer-side audit of every
shape the snapshot can carry, and a shell audit of
the paths only the real shell exercises. The
hypothesis was KILLED and the real cause found —
one inverted guard in the shell's downloads-command
handler, invisible to every host-side test because
app.rs is Windows-gated shell code and the DOM
harness only ever asserted that the chrome EMITS the
command bodies, never that the shell accepts them.

Cargo package 0.34.2, `core::DISPLAY_VERSION` is
"0.34.2", transport User-Agent `Pico/0.34.2`.
Cargo.lock diff vs 0.34.1: the own-package version
line only. No new dependencies.

## The root cause — inverted dedupe (AUDIT 0.34.2)

`App::on_chrome_dl_body` (src/app.rs) guarded its
dispatch with the downloads body's `(boot, req)`
dedupe. `core::ipc::Dedupe::check` returns TRUE for
a FIRST delivery — the pair was unseen, is now
recorded, the delivery is accepted; it returns
false for a replay of a seen pair, for a retired
boot, and (until its first `Ready`) for an unadopted
boot. The ordinary command path has always used it
that way (`if !check { drop }`). The downloads
handler stored the result in a variable named
`deduped` and returned early when it was TRUE —
the exact inverse. Consequences, all in the shipped
0.34 and 0.34.1:

- Every chrome-surface downloads verb was dropped
  on its normal, single delivery: `open-panel`
  (the toolbar Downloads button), `close-panel`,
  every popover row verb (pause/resume/cancel/
  remove/open-file/open-folder/reveal/copy-link/
  refresh-link), `add-url`, `set-connections`,
  `clear-finished`, and the toast's `dismiss-notice`
  and click-through. The chrome sends each body
  exactly once (monotonic `req` per boot), so "drop
  the first delivery" meant "drop it entirely".
- Bodies from a RETIRED or unadopted boot — for
  which `check` returns false — were DISPATCHED,
  and because the retired branch never records the
  pair, every re-delivery executed again: a stale
  generation's `add-url` body arriving after the
  watchdog retired its boot created its task, twice
  if delivered twice.
- The downloads PAGE surface has no dedupe (its own
  handler + gate), which is why typed
  `pico://downloads` kept working and the dead
  chrome surface shipped unnoticed. The popover
  itself never painted for the user: opening it was
  one of the dropped verbs.

The fix restores the command path's polarity
verbatim: drop exactly when the check FAILS, and
the drop writes its log line ("downloads command
dropped (duplicate or stale boot, req N)") — never
silent. A source-scan pin
(`source_scan_chrome_dl_body_dedupe_polarity`,
tests/media_tests.rs) locks the handler's body span
to the `!self.dedupe.check(` shape, forbids the
inverted shape's return, and requires the log line;
`Dedupe`'s own semantics were already pinned in
tests/addr_ipc.rs and did not change.

## The hypothesis, killed with evidence (corpus)

The hostile corpus drove the REAL assembled chrome
and downloads pages (jsdom, the battery's own
harness) with 63 snapshot shapes built from the
field's data profile: unknown totals with the exact
producer phrasing, percent absent/0/100/137.5/−3/
string/null-with-known-total, statuses outside the
known set, Persian RTL names, quotes, backslashes,
emoji, a 300-char name, a 5000-char server error,
HTML metacharacters in every string field (the
`<img onerror>` probe never fired; no img/script
element ever appeared in either list), downloaded >
total, 0-byte tasks, 100 tasks at the cap, notices
with 500-char names and unknown kinds, off-list and
missing settings, `added_ms` at 0/missing/negative/
string/1e18, missing/string/duplicate ids, null and
non-object task elements, and degenerate whole
snapshots ({}, tasks:null, tasks:"nope", the legacy
bare array, downloads:null). Each entry ran on both
surfaces, as a live object and after a JSON
round-trip, applied TWICE (the app re-pushes on
every tick): 257 surface-runs, ZERO throws, panel
open with the expected rows every time. A poison
self-test (a `tasks` getter that throws) proved the
driver could detect failure — it is the only thing
that threw, and it is also the shape that motivated
the blast-radius work below. The corpus is now a
permanent battery fixture set (scripts/dom_test.mjs,
"0.34.2 HOSTILE DOWNLOADS CORPUS": 126 corpus checks
+ 17 behavior/guard/CSS checks).

## Blast radius — per-section render guards

The poison self-test exposed the structural weakness
the hypothesis had pointed at, even though no real
data triggers it: one section's exception aborted
the whole snapshot application at that point, leaving
every later section at its previous paint, silently.
0.34.2 makes a render failure visible and contained:

- Chrome: `window.__pico` now runs `picoApply` under
  an outer guard; the separable sections — picker,
  downloads (`dl-render`), context menu — run under
  `guardSection(name, fn)`. A throw is recorded in
  `window.__picoErrors`, logged to the console, and
  painted on the shell self-diagnosis strip
  (`#shellwarn`, kind "error": "Pico hit a display
  problem (NAME) — the rest of the shell is
  unaffected; details in pico.log"). Remaining
  sections still apply; a section that later renders
  cleanly clears its own error and the strip. The
  snapshot-arrival clear of the strip no longer wipes
  an error strip.
- Downloads page: `render()` wraps `renderNow()`; a
  throw is recorded the same way and surfaced on the
  page's count line. The next push re-renders from
  scratch on both surfaces.
- DOM pins drive a throwing-`tasks` snapshot through
  both surfaces: nothing escapes, the failure is
  recorded and painted, the sections after downloads
  (the errbar) still apply, and a healthy push
  recovers the panel and clears the strip.

## Data-shape fixes (producer + renderer)

The producer audit found no shape that can throw,
and five real defects short of that; each is fixed
at its own layer:

- **G1 — retry countdown overwrote the detail.**
  `DownloadManager::views()` replaced a failed+
  retry-pending task's `detail_text` with the
  countdown string, so the page (which renders both
  slots) printed the countdown twice and lost the
  byte progress for the whole pending window. The
  producer now keeps `detail_text` as the detail and
  the countdown in `retry_text` only; the popover's
  single detail slot prefers `retry_text` (the
  countdown is the fresher fact there — the same
  visible popover behavior as before, achieved in
  the renderer).
- **G2 — a paused task's error was invisible on the
  page.** A failed link-refresh sets `error` and
  keeps the task paused/queued; the popover showed
  it for any status, the page only under `failed`.
  The page row now renders the error whenever one is
  present on failed/paused/queued rows; the recovery
  hint stays failure-only.
- **G3 — a restored done task could carry no total.**
  Live completion forces a Complete task's total,
  but a legacy record can persist `total: null`, and
  the snapshot then drew an unfinished bar on a
  finished file. `TaskState::sanitized()` repairs a
  Complete record's missing total from its segment
  progress sum; belt-and-braces, both row renderers
  draw a full bar for a `done` row with no percent.
- **G4 — an off-list speed limit blanked the page
  select.** The settings sanitizer caps
  `speed_limit_kbps` but does not snap it to the
  select's choices, so a hand-edited state file's
  value (e.g. 768) matched no option and the select
  rendered blank. The page now appends a real option
  for an off-list value — the same rescue the
  chrome's default-connections select got in 0.32.1.
- **G5 — an out-of-range `added_ms` printed
  "Invalid Date".** `sanitized()` zeroes an
  `added_ms` past the year-9999 stamp (not a date;
  the renderers skip a zero stamp), and the page
  row's date is validity-guarded as well. A crafted
  `id` above 2^53 is documented, not fixed: it is
  reachable only by hand-crafting a task file (live
  ids are sequential), and a rounded id can at worst
  address the wrong task in the crafted file's own
  list.

## Shell layout fixes (AUDIT 0.34.2)

The shell re-audit cleared the combination math
(extension selection is a priority chain — context
menu → downloads → teleport/palette → toast — never
a sum; `chrome_rect_with_extra` saturating-clamps
to the window) and the watchdog recreation resync,
and found two real defects:

- **Stale popover height.** `chrome_phys` prices the
  open popover from the live task list, but task/
  error-row changes reached the shell as push-only
  effects that never relayouted — the one relayout
  trigger was a notice open-state flip. With a Failed
  notice open (it has no expiry timer), a second
  failure's error row left the rect 15 DIP short
  indefinitely; a page-side removal left a too-tall
  rect eating page clicks. `exec_dl_effects` now
  compares the popover's implied height before/after
  the batch (single pricing path `App::dl_popover_h`,
  also used by `chrome_phys`) and relayouts when it
  moved while the panel is open.
- **Min-window clipping.** The popover card had no
  height cap: at the 760×500 DIP minimum window its
  priced 478px from top:78 ran off the bottom, the
  "All downloads" footer unreachable. The card is
  now `max-height: calc(100vh - 88px)` with the list
  allowed to shrink (`min-height: 0`) so it scrolls
  internally and header/footer stay on screen; the
  teleport/palette cards cap at
  `min(486px, calc(100vh - 90px))`. On the user's
  2560×1392 window none of the caps bind; the
  layout.rs geometry contract is unchanged. (The
  HWND-level clamp underneath it — a priced rect
  taller than the window covers the whole window —
  is pre-existing behavior and unchanged.)

## Cleared with evidence

- **Producer percent/status arithmetic:** `percent()`
  caps at 100 and serde_json would emit null for a
  non-finite f64 regardless; the status set is
  exactly the five labels; ids are sequential. No
  snapshot the live producer can emit carries NaN/
  Infinity or an unknown status.
- **Watchdog recreation + dl bodies:** recreation
  happens only for a generation whose handshake
  never arrived; App state is untouched;
  `last_snap_sig = 0` + the Ready arm force a full
  `push_state` carrying the downloads snapshot and
  open mirror, and `layout()` sizes the new view
  from the current rect inputs. A fresh boot's first
  dl body is a first delivery (accepted — and with
  the polarity fix, now actually dispatched).
- **Layout combinations:** panel+toast (the panel's
  larger extension wins), toast+menu (the menu's
  claim supersedes), at minimum size, 2560×1392, and
  scales 1.0/1.25 — the popover extension caps at
  exactly 480 DIP at the 100-task cap, the context
  menu's at its content bottom ≤430 DIP.

## Evidence

- Rust tests: 595 passed, 0 failed (592 at 0.34.1 →
  +3: the dedupe-polarity source-scan pin, and the
  two `sanitized()` repair tests for G3/G5). DOM
  checks: 560 passed, 0 failed (417 at 0.34.1 →
  +143: the 126-check hostile corpus over both
  surfaces, the G1–G5 behavior pins, the two
  blast-radius guard suites, and the two CSS-cap
  checks).
- `cargo fmt --check` clean. Clippy: no new
  warnings (the three pre-existing ones from 0.34 —
  `core/download.rs` div_ceil, `core/history.rs`
  identical blocks, `winhttp.rs` fetch_range arity —
  are untouched and outside this round's scope).
- Battery: `./scripts/battery.sh 10
  battery-0.34.2.log` — 10 consecutive PASS on the
  final tree (iteration 10 includes Windows clippy
  and the release check).
- Exe: release build 2,200,576 bytes (under the 5 MB
  cap), sha256
  fe38e247d6ba61d5b499b7e52856bd5d43bc1df1630970cbfc337607228ef5f3
  — changed vs 0.34.1's
  e13315513a97a732d91d0cc537f645cf9b61e9fb021a35bf14faaf691ef0c398
  before gating, per the stale-exe rule. Gate v2
  PASS on the shipped binary: 0 RT_MANIFEST,
  RT_ICON ×8 + RT_GROUP_ICON ×1.
- dist/pico-0.34.2.zip: Pico.exe + README.md; the
  exe inside hashes to the SHA-256 above.

## Still owed (witnesses, the user's machine)

0.34.2 is the first build in which the chrome
downloads surface can work at all (see the root
cause), so the whole 0.34 witness list is live
again, plus this round's:

- **The four reported symptoms, retaken:** click a
  download link on soft98 → the "Download started"
  toast names the file; the toolbar Downloads button
  opens the popover with the task in it; the
  popover's "All downloads" opens the page; the
  site-opened tab closes itself (pico.log's
  `download tab fate:` line is the paper trail).
- **In-popover verbs:** pause/resume from the
  popover itself; Add URL from the popover; a
  finished download's toast dismisses on click.
- Carried from 0.34.1: the context menu on page/
  link/image/selection/text-field; Refresh link on
  an expired soft98 token (bytes kept, download
  resumes); a download actually COMPLETING via the
  Iranian CDN (`download completed:` in pico.log);
  the cookie-carrying (login-gated) download; pause
  → quit → relaunch → resume; and whether the
  same-folder extraction habit was adopted (data
  carry-over + the migration retry outcome).

---

# Pico 0.34.1 — Build Notes (2026-11-11)

## What 0.34.1 is

Pico's own context menu. The trigger was the user's
screenshot of a right-click inside a page: what opened
was WebView2's STOCK menu — unthemed, in the platform's
own language, and carrying verbs that were never
Pico's to offer. Two of them were worse than cosmetic:
"Save as" saved the file through WebView2's own path,
bypassing Pico's download manager ENTIRELY (no task,
no popover, no history, no resume), and "Print"
offered a capability Pico has deliberately never had.
The rest of the stock list — spellcheck entries,
"More tools", "Send tab to your devices" — was the
same class: another browser's menu inside Pico.

0.34.1 suppresses the stock menu on every CONTENT
webview and replaces it with a themed menu the chrome
draws, whose items are decided by a pure core model
(`core::ctxmenu`) and whose picks execute through the
app's existing machinery. Nothing else changed: the
downloads coordination layer is 0.34's, untouched.

Cargo package 0.34.1, `core::DISPLAY_VERSION` is
"0.34.1", transport User-Agent `Pico/0.34.1`.
Cargo.lock diff vs 0.34.0: the own-package version
line only. No new dependencies (the clipboard calls
ride two more feature flags on the already-used
windows-sys crate — see Cargo.toml's 0.34.1 comments).

## The suppression route

- `attach_context_menu` (src/app.rs) registers a
  `ContextMenuRequested` handler (via
  ICoreWebView2_11) on the view. The handler's FIRST
  act, before any fact is read, is
  `args.SetHandled(true)` — first and unconditional,
  so even a failed fact read can never fall back to
  the stock menu. It then reads only the target facts
  (kind, page/link/source URLs, selection, editable,
  clipboard-text presence) and forwards them as a
  `UserEvent::ContextMenu`; the event loop owns the
  menu from there. Registration failures log and are
  never fatal.
- The registration lives at the ONE choke point:
  `attach_context_menu` is called inside
  `build_content_view`, beside the panel-focus and
  media-probe attaches, so every content view — initial
  tab, new tab, adopted popup, retry — carries it and
  no creation path can miss it. A source-scan pin
  (tests/media_tests.rs) asserts exactly one
  `add_ContextMenuRequested` in the shell source and
  the attach call inside `build_content_view`'s body
  span.

## The item matrix, as built

`core::ctxmenu::menu_items` decides the rows from the
target facts plus the tab's history gates. Groups in
fixed order — nav, link, image, edit, Inspect — a
group present only when its precondition holds,
exactly one separator between consecutive present
groups, and a separator always before Inspect:

| Group | Present when | Items (enable rule) |
| --- | --- | --- |
| nav | always | Back (can go back), Forward (can go forward), Reload (always) |
| link | a link URL is present | Open Link in New Tab, Copy Link, Download Link (all always) |
| image | kind is Image and a source URL is present | Open Image in New Tab, Copy Image Address, Download Image (all always) |
| edit (editable) | the target is editable | Cut (has selection), Copy (has selection), Paste (clipboard holds text), Select All (always) |
| edit (selection) | not editable, but text is selected | Copy (always) |
| inspect | always, always last | Inspect (always) |

The id set is a closed table of fourteen
(`CTX_IDS`); there is no Save-as id and no Print id
in the table, so the model is incapable of emitting
them — a whitelist test pins the table against a
maximal target. Audio and Video targets get nav +
Inspect ONLY this round: no source group is modelled
for them yet, and a media element's incidental
link/selection state must not leak other groups in.

## Action plumbing

Picks return over the chrome IPC as `ctx_pick` with
the id validated against `CTX_IDS` in `core::ipc`
(unknown ids and unknown fields are denied at parse;
`ctx_close` is the second, field-less command). The
menu closes first; the pick then executes against the
STORED tab and target facts the menu was built from:

- **Nav:** Back/Forward ride `App::go`,
  Reload rides `reload_active` — the ordinary paths.
- **New tabs:** Open Link / Open Image ride
  `add_tab` (web URLs only), under the same tab cap
  and routing as every other tab creation.
- **Downloads:** Download Link / Download Image call
  `App::on_download_starting` VERBATIM — the proven
  capture entry, unmodified. The coordinator reads
  the tab's LIVE URL (the page being read, not the
  file URL) as the capture's live URL, so the tab
  fate is None by construction — the tab the user is
  reading is never unwound, converted or closed by a
  menu download — and the referer (the page) and the
  host-scoped cookie capture come from the same
  triage as a browser-initiated download.
- **Edit commands:** Copy / Cut / Select All execute
  in the page via `document.execCommand`. Paste is
  different on purpose: the app reads the clipboard
  itself (`clipboard_get_text`, Win32 Unicode text)
  and inserts via `execCommand('insertText')`, with
  the text crossing as a JSON string literal, never
  interpolated raw.
- **Copy Link / Copy Image Address:** app-side Win32
  clipboard helpers (`clipboard_set_text`): the
  documented open / empty / global-allocation /
  set sequence; the copied text never reaches the
  log (only its length does — a link can carry a
  query).
- **Inspect:** `open_devtools` on the stored tab's
  view — the standing right-click-Inspect requirement,
  kept.

## The anchor story

WebView2's `ContextMenuRequested` Location does not
document its coordinate space tightly enough to trust
blindly: it may be view-relative DIP, view-relative
physical pixels, or window-relative DIP.
`core::ctxmenu::resolve_anchor` computes all three
candidate readings and cross-checks them against the
REAL cursor position (window DIP): the candidate
within 48 DIP of the cursor wins; with no cursor, or
no candidate anywhere near it, the view-relative DIP
reading — the documented-most — is the default. The
result is then clamped/flipped by
`core::layout::ctx_menu_pos`.

The cursor itself needed a substitution. The natural
read is GetCursorPos + ScreenToClient, but
windows-sys 0.59 gates ScreenToClient behind the
Win32_Graphics_Gdi feature this tree does not enable,
so `cursor_client_dip` computes exactly
ScreenToClient's translation by hand: GetCursorPos
minus the window's `inner_position` (the client
area's own screen origin, from tao), divided by the
scale. Any failure yields None and the anchor falls
back to its default reading. resolve_anchor's
candidate choice is unit-tested at scales 1 and 2,
including the window-relative and far-cursor cases.

## Deliberate scope

- **The stock menu REMAINS inside Pico's own CHROME
  webview.** No ContextMenuRequested handler is
  registered there: the chrome is Pico's own UI, and
  its address field needs its native edit menu (cut /
  copy / paste in the address bar is the user's own
  text, in the shell's own field). Suppressing it
  would have removed a working tool to win a
  screenshot.
- **Internal pages in CONTENT tabs get Pico's menu:**
  the start page and the downloads page are content
  webviews, so a right-click there shows Pico's menu
  like any other page. That is the consistent reading
  of the law — the split is chrome-vs-content, not
  web-vs-internal.

## The close law

The menu rides the panels' one funnel. Opening it
calls `close_all_panels` first (it supersedes every
panel, as a panel opening supersedes it), and
`close_all_panels` itself now takes the context menu
too — so the existing dismissal signals close it
unchanged: a content webview's GotFocus (a click back
into the page) and the window's Focused(false)
(leaving the window). On open the app focuses the
chrome, so the menu's own keyboard dismissal (Escape
→ `ctx_close`) and its outside-pointerdown close in
the chrome DOM engage deterministically. A pick
closes the menu before it executes, always.

## Evidence

- Rust tests: 583 passed, 0 failed (553 at 0.34; the
  round adds the ctxmenu model/anchor tests, the IPC
  ctx_pick/ctx_close parse tests, the chrome DOM's
  Rust-side pins and the registration source-scan).
  DOM checks: 417 passed, 0 failed (399 at 0.34 →
  +18: the context-menu suite — render, position,
  disabled rows, pick/close envelopes, outside-click
  and null-snapshot behavior).
- `cargo fmt --check` clean. Windows clippy (battery
  iteration 10): the same three pre-existing warnings
  as 0.34, all outside this round's scope —
  `core/download.rs` (manual `div_ceil`),
  `core/history.rs` (identical `if` blocks),
  `winhttp.rs` (`fetch_range` argument count).
- Strings audit: transport UA `Pico/0.34.1`
  (winhttp.rs) present in the exe; built pages carry
  0.34.1 in the chrome badge + JS fallback, the start
  hero + footer, the downloads badge + footer, and
  the downloads INIT fixture; every remaining "0.34"
  string in source, built pages and the exe is a
  historical comment tag (feature-era notes) or the
  tao 0.34 dependency's own paths, by the standing
  rule.
- Battery: `./scripts/battery.sh 10
  battery-0.34.1.log` — 10 consecutive PASS on the
  final tree (iteration 10 includes Windows clippy
  and the release check). One audit-driven restart:
  the first 10/10 run (kept as
  battery-0.34.1-run1.log) preceded a strings-audit
  find — the dump_pages downloads fixture still
  carried the snapshot's `"version":"0.34"` — fixed
  to 0.34.1 and the count restarted, so the shipped
  log is the final tree's.
- Exe: release build 2,191,360 bytes (under the
  5 MB cap), sha256
  bf97b9728caa2cc794240b2634c8a16f9eea31a96d15067bea380de0f4b2b7cc
  — changed vs 0.34's
  46eb6173aba0f6df85ef28318fd3b9a0d403fbaec04baf6342e5f38a92df7012
  before gating, per the stale-exe rule. Gate v2
  PASS on the shipped binary: 0 RT_MANIFEST,
  RT_ICON ×8 + RT_GROUP_ICON ×1, 0 CRT imports,
  WINDOWS_GUI.
- dist/pico-0.34.1.zip: Pico.exe + README.md; the
  exe inside hashes to the SHA-256 above.

## Still owed (witnesses, the user's machine)

His first 0.34.1 session is the witness round:

- **The original screenshot, retaken:** right-click
  a web page → Pico's themed menu opens at the
  cursor (this is also the anchor witness: at his
  display scale the card must land ON the click
  point, not offset by the chrome height or the
  scale factor), with no Save as and no Print
  anywhere in it.
- **Download Link / Download Image:** the file lands
  in Pico's download manager (task, toast, popover —
  the full 0.34 path) and the tab being read does
  not move, convert or close.
- **The edit groups:** right-click selected text →
  Copy works; right-click inside a text field → Cut /
  Paste / Select All behave, with Paste greyed when
  the clipboard holds no text.
- **The chrome exception:** right-click the ADDRESS
  BAR → the native edit menu still appears there
  (deliberate — see Deliberate scope).
- **Dismissal:** with the menu open, click back into
  the page → the menu is gone; Alt-Tab away and back
  → already gone; Escape → gone.
- Carried from 0.34, still owed: the soft98 adopted-tab
  close, the downloads page via all four doors,
  refresh-link on an expired token, and the rest of
  the 0.34 witness list in that section below.

## Field addition — fate logging (0.34.1, Worker 5)

Folded into the same 0.34.1 release after the user's
first 0.34 field log: a landed download's tab fate
executed with NO log line anywhere, so the log could
show a capture and a task but never what happened to
the tab itself. Two gaps, both closed:

- **`App::exec_dl_fate` logs every executed fate** —
  exactly one `download tab fate: …` line per
  execution: `closed tab {id}`, `converted tab {id}
  to start`, `restored tab {id} to {redacted}` (the
  restore URL passes through `redact_url`, as
  everywhere), `tab {id} back`. Each line sits at its
  arm's point of commitment, so it never claims an
  act that did not happen: RestoreTo's line is AFTER
  its defensive tab-missing early return (the return
  logs nothing — no restore, no claim), and GoBack's
  line fires only inside the branch where the history
  machine actually returned a target. `TabFate::None`
  logs nothing.
- **The only-tab conversion names itself.** In
  `core::dlcoord::fate_step`, a CloseTab fate landing
  on what is NOW the only tab converted to
  ConvertToStart silently through 0.34. It now emits
  `download capture {token}: tab is now the only tab;
  close converted to start` BEFORE the ExecuteFate
  effect, in the voice of the two pre-existing drop
  lines (tab gone / tab moved on), which are
  unchanged. `fate_step` therefore returns
  `Vec<DlEffect>` (it was `Option<DlEffect>`); both
  callers extend their effect vectors.

Coverage: `fate_only_tab_conversion_logs_its_reason_
before_the_fate` (tests/dl_replay.rs) asserts the
effect stream ends with exactly `[Log(reason),
ExecuteFate{ConvertToStart}]` and that both drop
lines still fire verbatim;
`source_scan_exec_dl_fate_logs_every_executed_fate`
(tests/media_tests.rs) pins the four exact fate
format strings inside `exec_dl_fate`'s body span,
plus the redactor call.

## Field addition — migration per-item logging (0.34.1, Worker 5)

### The diagnosis

The user's real 0.34 log showed `data: migration
INCOMPLETE (1 failed, 1 skipped; state marker
withheld, will retry next start; originals untouched
in appdata)` and, later, `profile: persistent,
cookies db present=false` (the probe reads the
portable profile dir — the correct dir, verified).
Under the counting rules a failed copy increments
`failed` AND `skipped` together, while a pure
`skipped` alone means a symlink/exotic entry or a
depth-cap hit — so (1, 1) is exactly ONE failed copy
and zero pure skips, and because the state marker
was withheld, the failure sat in the profile phase
(or in the state copy itself after a clean profile
phase). The prime suspect is one locked or vanishing
WebView2 profile file — the Cookies database
(`Default/Network/Cookies`) is the one whose absence
the cookies-db probe independently shows. **Which
item actually failed is unknowable from 0.34's log:
every per-item error in the migration path was
discarded (`Err(_)` at every site). That absence IS
the defect this addition fixes first** — from this
build on, the log names the item.

### What changed

- **`MigrationReport` itemizes failures and pure
  skips.** New `details: Vec<MigrationDetail>` —
  `{ kind, path, error }`: kind is `profile file` /
  `profile dir` / `state file` / `entry` (a skip's
  reason rides in its kind: `(symlink)`, `(depth
  cap)`, `(exotic)`); path is RELATIVE to the
  migration source root (`WebView2/Default/Network/
  Cookies`, `.` for the tree root) — never absolute,
  never contents; error is the captured io error's
  Display for failures, `None` for deliberate skips.
  Successes are counted, never itemized. Details are
  recorded at EVERY site that used to discard an
  error: dir-create, read_dir, per-entry read,
  file_type, file copy, symlink/exotic skip,
  depth-cap skip, and the state `copy_atomic`
  failure. `MigrationReport` loses its `Copy` derive
  (a Vec cannot be Copy); every constructor and user
  was checked — the report is only ever built via
  `Default` and read by the one app call site.
- **The app logs each detail before the summary
  line:** failures at error level, `data: migration
  item failed: {kind} {path} — {error}`; skips at
  info level, `data: migration item skipped: {kind}
  {path}`. The summary lines are unchanged.
- **Retry poisoning — verdict: REAL, fixed.** On a
  retry, `fs::copy` over a destination the first
  partial copy left READ-ONLY (fs::copy copies the
  source's permissions) fails on Windows with access
  denied — forever, even after the original lock
  clears; the same held for `copy_atomic`'s rename
  over a read-only state.json. Fix: in copy_tree's
  file arm, on copy error with an existing
  destination, clear the destination's read-only
  attribute and retry ONCE (a successful retry counts
  as a copy, not a failure; a failed retry records
  the SECOND error); in `copy_atomic`, clear a
  read-only destination's attribute (best-effort)
  before the rename. The Windows read-only semantics
  are reasoned from the platform behavior, NOT
  verifiable on the Linux build host (where rename
  ignores the file mode and a same-owner copy is not
  blocked the same way) — the host tests pin the
  outcomes (fresh bytes land, counted as copies) and
  the on-Windows behavior is witness-owed.
- **Locked/vanishing source files — environmental,
  no code change.** A running process or AV holding
  the source profile is exactly what the
  retry-next-start design (state marker withheld,
  copy overwrite-idempotent) already covers once the
  lock clears; the per-item lines now name the file
  when it happens. Nothing else in plan/perform was
  found defective under the same verify-first pass:
  the performer's discarded `create_dir_all(to)` /
  pending-marker write errors surface downstream as
  itemized copy/state failures, so no silent path
  remains.

Tests added: at perform level
(`migration_details_name_the_failed_profile_file` —
the directory-squatted Cookies shape itemizes as
`WebView2/EBWebView/Default/Cookies` with the error
captured and the counters consistent;
`migration_details_record_a_symlink_as_a_pure_skip`;
`migration_replaces_a_readonly_state_destination`)
and at copy_tree level
(`copy_tree_details_name_a_failed_file`,
`copy_tree_details_name_a_symlink_skip_without_an_
error`, `copy_tree_details_name_the_depth_cap_skip`,
`copy_tree_retries_once_over_a_readonly_destination`).
All pre-existing migration tests pass unchanged.

### Updated evidence (supersedes the 0.34.1 Evidence above)

- Rust tests: 592 passed, 0 failed (583 at the
  Worker-4 landing; +9: the replay test, the
  source-scan pin, four copy_tree detail tests,
  three perform-level detail tests). DOM checks:
  417 passed, 0 failed (unchanged — this round
  touches no chrome behavior).
- Battery: `./scripts/battery.sh 10
  battery-0.34.1-final.log` — 10 consecutive PASS on
  the final tree. One restart: the first final run
  was also 10/10, but its iteration-10 clippy showed
  two NEW warnings from this round (a dead-code
  wrapper, a `set_readonly(false)` lint) — both
  fixed, the count restarted, and the shipped log's
  clippy is back to exactly the three pre-existing
  warnings.
- Exe: release build 2,194,432 bytes (under the
  5 MB cap), sha256
  e13315513a97a732d91d0cc537f645cf9b61e9fb021a35bf14faaf691ef0c398
  — changed vs the Worker-4 build's bf97b972… before
  gating, per the stale-exe rule. Gate v2 PASS on
  the shipped binary: 0 RT_MANIFEST, RT_ICON ×8 +
  RT_GROUP_ICON ×1, 0 CRT imports, WINDOWS_GUI.
  `cargo fmt --check` clean.
- dist/pico-0.34.1.zip: Pico.exe + README.md; the
  exe inside hashes to the SHA-256 above.
- SECURITY.md: no change — the new lines carry tab
  ids, source-root-relative paths and redacted URLs
  only; no absolute paths, no file contents, no
  cookie values.
- Still owed (witness, his machine): a migration
  retry on his real profile — the next 0.34.1 start
  after a failed migration must log the failed item
  BY NAME (`data: migration item failed: profile
  file WebView2/...`), and if the failed item was
  the read-only-poisoning shape, the retry must
  complete once the lock is gone.

# Pico 0.34 — Build Notes (2026-11-11)

## What 0.34 is

The downloads rebuild. No new user-facing features:
0.34 is 0.33's feature set on a coordination layer
rebuilt from zero, because the old one's shape was the
bug. Through 0.33 the downloads verbs lived in THREE
vocabularies that had drifted apart — the chrome's
snake_case `core::ipc::Command` variants, the downloads
page's kebab verbs in `core::dlaction`, and ad-hoc arms
in `app.rs` — and the tab-fate machinery to match
(navigation guard, swallow marker, pending-download
record, a deferred close that re-judged live facts).
Five patch rounds (0.32.1–0.32.4, 0.33 WS1) fixed each
failure in the field and the next one surfaced through
a different seam of the same split. 0.34 deletes the
split: one command table, one coordinator, one shared
row renderer. The download ENGINE — the multi-connection
workers, the `.part` machinery, resume/refresh/retry —
is 0.33's, kept untouched underneath.

Cargo package 0.34.0, `core::DISPLAY_VERSION` is
"0.34", transport User-Agent `Pico/0.34`. Cargo.lock
diff vs 0.33: the own-package version line only. No
new dependencies.

## The architecture in one page

Module map:

- `core::dlcmd` — the VOCABULARY. `DL_COMMANDS`: 21
  kebab verbs with their argument schemas and the
  surfaces allowed to send them (Chrome / Page /
  Start; `copy-link` is client-only and never reaches
  Rust). `parse_dl_body` is the one strict parser
  (dlaction's discipline absorbed whole: size cap,
  `kind` check, unknown action vs unknown field both
  denied, ids ≥ 1, URL validation, documented clamps);
  `page_gate` is the page transport's conjunctive law.
- `core::dlcoord` — the DECISIONS. A pure, host-tested
  `Coordinator` that owns capture triage and tab fate,
  the cookie race, the notice machine, the mirrored
  settings, and command dispatch. It does NO I/O: the
  app feeds it events and executes the `DlEffect`s it
  returns. It talks to the manager only through the
  `DlBackend` trait; `DlSnapshot` is the ONE data shape
  both surfaces render.
- `src/dlbackend.rs` — the ADAPTER: `ManagerBackend`
  implements `DlBackend` by forwarding to the real
  manager (one documented alias: the page's `remove`
  and the chrome's `cancel` both ride the manager's
  `cancel`, whose semantics are exactly remove's).
- `src/downloads.rs` — the engine RUNTIME (kept):
  workers, persistence, the supervisor. Unchanged.
- `core::download` — the pure engine (kept): triage,
  the unwind matrix the fate decision delegates to,
  cookie rules, retry/refresh policy, the token bucket.
- `assets/dlrows.js` — the one row renderer + command
  binder, spliced by `core::assets` into BOTH the
  chrome popover and the downloads page. The start
  page gained a Downloads card (counts + a door).

The capture sequence, end to end:

1. WebView2 reports a download starting; the chrome's
   own webview denies outright, a content tab's capture
   goes to `App::on_download_starting`.
2. The app assembles capture FACTS from the tab's
   current state (live URL, last committed page,
   previous entry, adopted-vs-user origin, only-tab,
   traversal pending, navigation epoch) plus the
   referer (the committed page, when it differs from
   the file).
3. `Coordinator::capture_started` triages the URI and
   decides the tab FATE once, from those facts. A
   reject is recorded as a Failed task with a Blocked
   notice and its fate executes immediately.
4. An accept opens the cookie race: the app fetches
   the tab's cookies (only when the download is within
   the page's own site — otherwise it lands at once
   with none) against a 1,500 ms timer; first answer
   wins, the loser is logged and ignored.
5. The winner creates the TASK through the backend;
   a Started notice fires (a manager refusal is a
   Failed notice + error log, never silence).
6. The fate is EXECUTED under revalidation: tab alive,
   epoch still the capture epoch, not newly the only
   tab (a CloseTab there converts to Start instead).
   Nothing is re-derived from the live tab.
7. Whenever the captured navigation's Finished event
   arrives, `on_page_load` consults the coordinator's
   `navigation_finished` BEFORE applying CommitFinish:
   a captured finish is swallowed by record — dropped
   whole, no history entry, no epoch bump.

**The tab-fate rule, stated once:** a captured tab's
fate is decided at capture time from capture-time
facts, and at landing time only plain facts are
revalidated (alive / epoch unchanged / only-tab) —
the decision itself is never re-made.

The command law: every downloads body on every
surface is `{"kind":"dl","action":"<verb>", ...}` in
the table's vocabulary. Page bodies pass `page_gate`
(downloads tab AND downloads URL) before parsing;
chrome bodies ride the chrome IPC transport, whose
type is the gate (and whose `(boot, req)` dedupe the
app checks for dl bodies too). The pair test
(`tests/dl_pair.rs`, Law 3) pins JS and Rust together:
every token the JS can emit is a table verb, and every
table verb with a surface is emitted — it fails if
either side grows without the other (verified at
integration by mutating both sides; both mutations
failed the test, both reverts restored green). The
replay tests (`tests/dl_replay.rs`, 13) drive the
coordinator through the field stories with a fake
backend: the soft98 adopted-tab capture in both race
orders, the epoch-moved-on drop, the triage reject,
manager refusal, dispatch of every verb, and the
notice machine.

## Kept vs deleted

Kept: the engine runtime (`src/downloads.rs`) and the
pure engine (`core::download`) entire; the 0.33
surfaces' look and behavior (popover, page, toasts,
badge, settings); the cookie plumbing in `app.rs`
(token-shaped now); the fate decision matrix itself
(it delegates to the proven `unwind_action`).

Deleted: `core::dlaction` (folded into `dlcmd`); the
`core::ipc` downloads `Command` variants and their
wire names (`downloads`, `downloads_close`,
`downloads_page`, `download_pause`, `download_resume`,
`download_cancel`, `download_add`,
`download_open_folder`, `download_set_conn`,
`download_set_default_conn`); the app-side patch stack
— `NavGuard` / `nav_guard`, the `dl_swallow` marker,
`PendingDownload`, and the deferred close that
re-judged live facts.

## Integration fixes (Worker 4)

- **The `conn` settlement.** W3's JS emitters name the
  connection-count field `conn`; W1's parser expected
  `connections`, and W2 bridged the two with a rename
  in `app.rs`. Settled on ONE side — the JS wire name:
  `dlcmd`'s parser now expects `conn` (its unit tests
  moved, plus one new assertion that the old spelling
  is denied as an unknown field), and
  `normalize_dl_body` keeps only the chrome-envelope
  strip (`v` / `boot` / `req`).
- **`tests/addr_ipc.rs`.** The three downloads parse
  tests asserted `Command` variants deleted by design
  (13 compile errors). Removed whole; the parse
  contract lives in `dlcmd`'s unit tests, dispatch in
  `dl_replay`, the pair law in `dl_pair`. Every other
  assertion in the file is byte-identical.
- **`tests/media_tests.rs` pins, re-pointed.** The
  three WS1 source-scan pins named deleted internals;
  each now locks the SAME claim on the new shape: the
  nav-guard pin → capture facts are assembled in
  `on_download_starting` and handed to
  `capture_started(`, with `nav_guard` / `NavGuard`
  absent from `app.rs`; the swallow pin →
  `navigation_finished(` is consulted in `on_page_load`
  before the CommitFinish application (same name, same
  byte-offset ordering); the adopted pin → `adopted =
  false` and `nav_epoch += 1` both live inside the
  `HistEffect::Commit` block of `apply_url_event`.
- **Formatting.** Four pre-existing `cargo fmt` diffs
  from the worker landings (app.rs ×2, ipc.rs,
  dlbackend.rs) were formatted so the tree is
  fmt-clean for the battery; no behavior change.

## Evidence

- Rust tests: 553 passed, 0 failed (529 at 0.33; the
  round adds dlcmd's table/parse tests, dlcoord's
  fate/snapshot tests, the 13 replay tests and the 3
  pair tests, and removes the deleted layers'
  assertions — dlaction's, the ipc downloads
  variants', addr_ipc's three downloads tests).
  `cargo xwin check --locked --target
  x86_64-pc-windows-msvc --all-targets` clean.
  Windows clippy: no warnings in any file a 0.34
  worker touched; three pre-existing warnings remain
  outside the round's scope and were left —
  `core/download.rs` (manual `div_ceil`),
  `core/history.rs` (identical `if` blocks),
  `winhttp.rs` (`fetch_range` argument count).
- DOM checks: 399 passed, 0 failed (387 at 0.33 →
  +12: the shared-renderer popover/page suites, the
  start card), re-run green at integration on the
  final 0.34 surfaces.
- `cargo fmt --check` clean.
- Strings audit: transport UA `Pico/0.34`
  (winhttp.rs); built pages carry 0.34 in the chrome
  badge, the start hero + footer, the downloads badge
  + footer, and the chrome JS fallback; remaining
  "0.33" strings in source and built pages are
  historical comment tags only (feature-era notes),
  by the standing rule.
- Battery: `./scripts/battery.sh 10
  battery-0.34.log` — 10 consecutive PASS on the
  final tree (iteration 10 includes Windows clippy
  `-D warnings`-equivalent scope per the battery
  script and the release check).
- Exe: release build 2,170,368 bytes (under the
  5 MB cap), sha256
  46eb6173aba0f6df85ef28318fd3b9a0d403fbaec04baf6342e5f38a92df7012
  — changed vs 0.33's bf88b364… before gating (per
  the stale-exe rule; the identical byte SIZE to
  0.33 is coincidence — the hash and the embedded
  strings audit are the proof: `Pico/0.34` UA
  present, `Pico/0.33` absent, `renderTaskList`
  embedded). Gate v2 PASS on the shipped binary:
  0 RT_MANIFEST, RT_ICON ×8 + RT_GROUP_ICON ×1,
  0 CRT imports, WINDOWS_GUI. The only build
  warning is the known benign LNK4099 class.

## Still owed (witnesses, the user's machine)

His first 0.34 session is the witness round:

- **The soft98 click:** the site opens the file in a
  NEW tab → the task is created, the toast names the
  file, and the new tab CLOSES by itself (the close
  lands when the task does — give the cookie fetch
  its moment; normally well under a second).
- **The downloads page via all four doors:** the
  popover's "All downloads" footer, Ctrl+J, a click
  on the download toast, and the start page's
  Downloads card — each must land on the same page
  with the same list.
- **Refresh link on an expired token:** fail a
  download with a dead token link (the soft98 shape),
  paste a fresh link via Refresh link…, and the bytes
  already down must survive when the file is the
  same — or the row must honestly restart when it
  is not.
- Carried from 0.32.4 / 0.33, still owed: panel
  dismissal by a click into the page; a same-tab
  download returning to its page (never closing);
  the cookie-carrying (login-gated) download; pause →
  quit → relaunch → resume finishing the same file;
  a cross-site download proceeding WITHOUT cookies;
  an https→http downgrade visibly refused; the
  validator-less resume spot-check line; a background
  adopted tab closing without stealing focus.

# Pico 0.33 — Build Notes (2026-11-10)

## What 0.33 is

The downloads feature release, driven by the user's
live test of the 0.32.x line ("there is no page
dedicated for downloads, or any way to retry one …
the download tab doesn't auto close after started the
download … add more needed features … make it better
in all terms") and by a benchmark study of IDM,
Gopeed, FDM, JDownloader2 and Chrome's downloads UI
(research notes, 2026-11-10). Five workstreams: the
bulletproof tab close (WS1), a dedicated downloads
page (WS2), honest recovery — resume / restart /
refresh link (WS3), reason-aware auto-retry and
limits (WS4), and completion manners (WS5).

Cargo package 0.33.0, `core::DISPLAY_VERSION` is
"0.33", transport User-Agent `Pico/0.33`. Cargo.lock
diff vs 0.32.4: the own-package version line only. No
new dependencies (the SHA-256 used for finished files
is a small hand-rolled FIPS 180-4 implementation in
`core::sha256`, test-vector-verified).

## WS1 — why the tab STILL didn't close, and the fix

0.32.4's close rule was sound; its facts were poisoned
before the decision ran. Three pollution arms, each
sufficient on its own, all confirmed in the code:

1. **The download's own completion commits the file.**
   The vendored wry (0.53.5) maps WebView2's
   `NavigationCompleted` to `PageLoadEvent::Finished`
   UNCONDITIONALLY — `IsSuccess` is dropped on the
   floor. Per WebView2Feedback #5172, a navigation
   that resolves to a download completes with
   `IsSuccess=false, HttpStatusCode=0`: not a real
   page landing. Pico's finished handler nevertheless
   applied `CommitFinish(file-url)`: the file entered
   the tab's history as a committed page, and
   `apply_url_event`'s "landed" rule cleared the
   tab's `adopted` flag.
2. **A pre-commit SourceChanged does the same.** If
   the file URL's `SourceChanged` is processed before
   the load-start, urlstate classifies the adoption
   as a same-document Push: `record_push(file)` plus
   the same landed rule clearing `adopted`.
3. **The deferred close re-judged from live facts.**
   0.32.4 armed the close at capture time but
   re-derived (committed, prev, origin) from the live
   tab when the task landed — by then arms 1/2 had
   rewritten those facts to (committed = the file,
   prev = none, origin = user), which reads as
   StartPage, not CloseTab. Exactly the user's
   symptom.

The fix (app layer, decision logic untouched):

- `adopted` now clears ONLY on `HistEffect::Commit`.
  A same-document landing presupposes an earlier
  commit, so nothing real is lost; a download
  navigation never commits, so it can no longer
  clear the flag.
- Each tab carries a **navigation guard**: at load
  start, before any URL event is applied, the app
  snapshots (url, adopted, history, urlstate). If the
  snapshot's history top is an uncommitted Source
  push of the very URL now starting, the snapshot
  drops that entry (`TabHistory::drop_top_entry`) —
  the push is part of THIS navigation, not a page.
  Capture decides from the GUARD's facts, not the
  live tab's, and restores history/urlstate/adopted
  from the snapshot for any non-None decision.
- A **swallow marker** is set on the tab at capture
  (accept and triage-reject arms alike). When the
  captured navigation's Finished event arrives —
  whenever it arrives — it is dropped whole: loading
  cleared, no CommitFinish, no error state, no
  loading wedge (the #5172 correlation).
- The deferred close no longer recomputes anything.
  It carries the tab's commit counter from capture
  time and closes iff the counter is unchanged, the
  tab is still adopted, and it is not the only tab.
  A tab that has since committed a real page is the
  user's tab and is left alone.
- `pico://downloads` joins `pico://start` in
  `addr.rs`'s navigation allowlist and scheme fold.

Regression coverage: a core composition test replays
his exact sequence (adopted tab → record_push(file)
→ drop_top_entry → facts → CloseTab), a baseline test
documents the unrepaired path (committed file →
StartPage), and three source-scan pins lock the
guard snapshot, the swallow-before-CommitFinish
ordering, and the Commit-gated adopted clearing.

## WS2 — the downloads page (`pico://downloads`)

A full internal page on the start page's
architecture: same assembly (shared CSS + INIT
JSON), same snapshot-push bridge
(`window.__picoSetDownloads`), conjunctive gate
first (a new pure `core::dlaction` dispatch mirrors
the start page's `core::action`: the sender must be
a downloads tab AND be on the downloads URL before
any body is parsed; unknown fields/actions denied).

- Full history, newest first, live progress; search
  by name/host; status chips (All / Downloading /
  Paused / Failed / Done — the snapshot's status
  vocabulary, where Complete is "done").
- Per item: Pause/Resume, Restart (inline two-click
  confirm, no modal), Refresh link… (WS3), Open
  folder, **Open file** (finished only — see the
  policy note below), Copy link, Remove from list,
  Delete file (a distinct action: removes the file
  or .part as well as the record).
- Header: Add URL, Clear completed, Clear all
  (list only — the button says files stay on disk),
  and the WS4 settings.
- **SHA-256** for finished downloads: the supervisor
  streams the finished file through `core::sha256`
  at completion, stores the digest in the task state
  (an io failure leaves it empty; completion stands),
  and the page shows it with click-to-copy.
- The popover gains an "All downloads" footer;
  Ctrl+J opens the page too (the chrome's existing
  Ctrl+letter accelerator pattern).

**POLICY CHANGE, flagged:** the 0.32 law "a finished
file is never opened" narrows to "never opened
AUTOMATICALLY". Open file fires only from the user's
own click on the downloads page, on a path the
manager itself resolves for a Complete task inside
the downloads folder. Nothing opens on completion,
on a timer, or from any page content. See the
SECURITY delta.

## WS3 — honest recovery

- **Resume**: unchanged (bytes kept, validators
  re-checked by the existing resume decision).
- **Restart**: discard the .part and the segment
  map, re-queue from the same URL.
- **Refresh link…**: paste a fresh URL into the
  existing task (the soft98 case: token links die).
  The new URL is probed; the pure
  `refresh_decision` rebinds and keeps the bytes iff
  size + ETag + Last-Modified all agree and ranges
  hold — otherwise the UI-visible outcome is a
  restart from zero on the new URL. The file name is
  never changed by a refresh.
- **Referer**: captures now store the initiating
  page's URL (userinfo-stripped, http/https only)
  with the task, and EVERY engine request — probe,
  segments, retries, refresh probes — sends it as
  `Referer`. It was entirely absent before 0.33;
  a forgotten referer is a classic cause of
  403-on-resume against token/CDN hosts.

## WS4 — reason-aware auto-retry + limits

- Pure taxonomy in core: `classify_failure` maps
  every failure string the manager can produce to
  network / server (5xx) / rate-limited (429) /
  not-found (404) / auth (401/403) / disk /
  range-lost / validator-changed. `retry_policy`:
  transient classes retry the TASK with backoff
  5 s → 15 s → 45 s, at most 3 automatic attempts,
  the row showing "Retrying in N · attempt k of 3";
  404 fails fast; 401/403 fails with the hint "the
  link may have expired — use Refresh link";
  range-lost / changed auto-restart once, then fail.
  Hints ride the task state to both surfaces.
  Segment-level retries are unchanged beneath this.
- Settings (persisted like the connections default):
  max concurrent downloads 1–5 (default 3 — replaces
  the hardcoded cap) and a global speed limit
  (Off / 512 KB/s / 1 / 2 / 5 / 10 MB/s) implemented
  as ONE token bucket shared by all tasks
  (`core::download::TokenBucket`, injected-clock unit
  tests incl. no-drift); workers draw full chunks
  from the one bucket. Controls live on the page
  header; the popover keeps only the connections
  default.

## WS5 — completion manners

The 0.32.3 notice system learns two kinds:
"finished" (name + size, ~5 s, click opens the
downloads page) and "failed" (name + reason, NO
auto-hide — it stays until dismissed or a panel
opens). A failed-count badge in the warn color sits
on the downloads button until the downloads page is
visited. Toasts fire only on transitions the app
actually observes (a restored failed task at startup
stays silent). Nothing steals focus; nothing is
modal. The started toast is unchanged.

## Evidence

- Rust tests: 529 passed, 0 failed (494 at 0.32.4 →
  +35: taxonomy/retry-policy/refresh-decision/token-
  bucket/SHA-256/dlaction/task-field tests, the WS1
  composition + baseline tests, 3 app source-scan
  pins, 1 IPC parse test).
- DOM checks: 387 passed, 0 failed (336 at 0.32.4 →
  +51: the downloads page render/actions/filters/
  empty states/exact IPC bodies, the popover footer,
  the failed badge, Ctrl+J, finished/failed toast
  routing).
- Battery: `./scripts/battery.sh 10` — 10 consecutive
  PASS (iteration 10 includes Windows clippy
  `-D warnings` and the release check). `cargo fmt
  --check` clean.
- Visual check: the downloads page rendered at 1280
  and 2560 in Forest dark and Forest light (plus a
  Mist light variant) and eyeballed — rows, actions,
  the 403 hint, the SHA-256 line and both settings
  selects render as designed (visual-check/pages-v33,
  shot-v33-*).
- Release exe: 2,170,368 bytes (0.32.4: 2,096,640),
  sha256
  bf88b3644875f9ae9352d54e138007ce4a0dde3aac0c19aae1781cec9c14d0da
  (changed vs 0.32.4's b5381326… before gating, per
  the stale-exe rule). Gate v2 PASS: 0 RT_MANIFEST,
  RT_ICON ×8 + RT_GROUP_ICON ×1, 0 CRT imports,
  WINDOWS_GUI. Strings audit: UA `Pico/0.33` present,
  `pico://downloads` embedded, zero `0.32.4` strings.

## What 0.32.4 is

The third on-device fix round, from the user's live
retest of 0.32.3 on his own Windows machine: the
download started and the toast appeared, but two things
still failed — (A) tapping the web page did not close
the downloads popover (the 0.32.2 "documented limit"
was rejected by the user, correctly: he taps the page,
the popover must close), and (B) the tab soft98 opened
for the download converted to the start page instead of
closing. The same report carried the standing order to
debug the whole capture → tab → panel lifecycle and
fix everything real in it.

Cargo package 0.32.4, `core::DISPLAY_VERSION` is
"0.32.4", transport User-Agent `Pico/0.32.4`. Cargo.lock
diff vs 0.32.3: the own-package version line only. No
new dependencies.

## Fix A — panels close on a click into the page

### Diagnosis: why every 0.32.3 close signal missed it

| Signal (as of 0.32.3) | Lives where | Why a plain page click defeats it |
| --- | --- | --- |
| Toolbar toggle, Escape, panel buttons | chrome DOM | the click never reaches the chrome document |
| Document pointerdown listener (0.32.2) | chrome DOM | covers only the chrome HWND's own area: the tab strip, the address bar, and the page strip under the extended chrome rect — the page itself is a different HWND |
| Mutual exclusion between panels | panel commands | fires only when another panel is opened |
| Navigation retirement (V26 D3 law) | app, on committed navigation | a plain click produces no navigation |
| Panel retirement on tab switch (`activate`) | app | fires on a tab CHANGE, not on a click inside the current tab — and never covered Downloads |

A plain click into the page has exactly one effect the
app can observe: the content webview — a separate child
HWND — takes keyboard focus. No DOM the chrome owns
sees it, and no navigation follows from it. Any fix
living in chrome.html is structurally blind to it; the
0.32.2 round's DOM tests passed while the real shell
disagreed for precisely this reason.

### The fix (app layer)

- `attach_panel_focus_close` (src/app.rs) registers
  `ICoreWebView2Controller::add_GotFocus` through
  webview2-com's `FocusChangedEventHandler` (wry's
  `WebViewExtWindows::controller()` is the access path,
  the same raw-binding family as the cookie manager).
  The handler only forwards `UserEvent::ContentFocus
  { tab }`. It is registered in `build_content_view`'s
  post-build attach block, beside `harden_webview_
  profile` / `attach_media_probe` / `attach_shield` —
  the single choke point every content view passes
  through — and pinned that way (below). Failures log
  and are never fatal (the media probe's precedent).
- Both outside-interaction signals funnel into ONE
  method, `App::close_all_panels`: the ContentFocus arm
  and the window event loop's new `WindowEvent::
  Focused(false)` arm (clicking another app or the
  desktop). It mirrors each panel's own Close-command
  semantics (Teleport's payload retires with its panel,
  version bumped), retires the download toast, and
  calls layout/push only when something actually
  changed — a focus event with nothing open is a true
  no-op, so the hook can fire on every page click
  forever at zero cost.
- One law for all panels: Downloads, the theme picker
  and Teleport all close the same way; no per-panel
  exception exists.
- **Why the signal cannot miss the case that matters:**
  a panel can only be opened by a chrome command —
  grep-verified: the only three `*_panel_open = true`
  sites in the tree live in `on_chrome_cmd` arms
  (Teleport via `request_teleport`, Palette, Downloads).
  So whenever a panel is open, the last interaction was
  in the chrome webview (focus is chrome-side) or the
  window has already lost focus (which now closes the
  panels itself). The first click into the page is
  therefore always a focus TRANSITION into a content
  controller, and GotFocus fires. The one theoretical
  gap — a panel open while a content view already holds
  focus, then a click that changes nothing — has no
  opener: no code path opens a panel without a chrome
  interaction first.
- **Accepted side effect:** tab switches that move
  focus into the content (Ctrl+Tab, the reselection
  after a tab close) now also close the Downloads
  panel. Previously the mouse and the keyboard
  disagreed (a tab-strip click already closed it via
  the chrome DOM listener; Ctrl+Tab did not). The
  focus law makes them agree, in the direction the
  user demanded: interacting with content dismisses
  the overlays.

## Fix B — a download-only tab closes when its task lands

### The extended decision

`core::download::unwind_action` gains two inputs and
one action. New inputs: `origin: UnwindOrigin`
(`Adopted` — the site opened the tab; `User` — the user
did) and `only_tab: bool`. New action: `CloseTab`. The
two 0.32.3 gates still dominate everything (live URL ≠
captured URL → None; traversal pending → None). The
full matrix:

| Committed state | Origin / tab count | Action |
| --- | --- | --- |
| a real page, not the file | any | **NavigateTo(page)** (0.32.3, unchanged) |
| the file itself; a different page before it | any | **Back** (0.32.3, unchanged) |
| the file itself; nothing (or only the file) before it | Adopted, not the only tab | **CloseTab** (new) |
| the file itself; nothing (or only the file) before it | User, or the only tab | **StartPage** (0.32.3) |
| nothing ever committed | Adopted, not the only tab | **CloseTab** (new) |
| nothing ever committed | User, or the only tab | **StartPage** (0.32.3) |

### Origin threading

`Tab.adopted` is a runtime-only flag (tabs are rebuilt
every session; nothing persists it). It is set in
exactly one place — the `UserEvent::NewWindow` arm,
after a successful non-start `add_tab` — and cleared
in `apply_url_event` the moment any real page lands in
the tab's history (any `HistEffect` other than None):
an adopted tab that commits a page of its own is a
normal tab from that commit on, and the committed-page
rows of the matrix cover its later downloads anyway.

### The close is deferred, and re-validated

CloseTab is DECIDED at capture time but not executed
there: the cookie fetch that follows reads the source
tab's cookies from its still-live view, and closing
the tab first would orphan that fetch. So
`capture_unwind` (the reworked `unwind_captured_tab`)
executes every action except CloseTab immediately, as
0.32.3 did, and returns the deferral;
`PendingDownload` carries `{ tab, close_tab }`; and
the close lands when the download's fate lands —
inside `create_pending_download`, AFTER the task is
created and the "started" toast has fired (and also on
a manager refusal: the attempt is over, and leaving
the tab parked on Loading would recreate the original
defect). A triage-rejected capture has no task to wait
for: its close lands in the Reject arm, right after
the failed row and the blocked toast. Execution is
`close_captured_tab_if_warranted`, which RE-COMPUTES
the decision on current facts before closing: a tab
the user has since navigated elsewhere, closed by
hand, or made the window's only tab is never closed.
The close itself is the ordinary `close_tab` — full
per-tab cleanup, neighbor reselection when the tab was
active, and no focus theft or active-tab change when
it was in the background (an adopted download tab the
user has already clicked away from closes silently
behind the tab he is reading).

Ordering guarantee (the mission's): task → toast →
close. The panel and badge live in the chrome and do
not depend on the source tab, so the toast snapshot
the user sees is unaffected by the close that follows
it in the same event.

## The lifecycle sweep (the standing order)

The whole capture → tab → panel lifecycle was re-walked
against the code. Beyond A and B: **no new code defects
found**; the items below were each verified in code,
with two behavior notes accepted deliberately.

| # | Walked item | Verdict |
| --- | --- | --- |
| 1 | Capture in a BACKGROUND adopted tab | closes without stealing focus or changing the active tab (close_tab's was_active=false path). By construction; witness-owed |
| 2 | Two adopted download tabs resolving back-to-back | first closes; second re-validates — if it is now the only tab it converts to StartPage instead of closing. The never-zero-tabs law outranks the close |
| 3 | User closes the source tab during the cookie window (≤ 1.5 s) | the pending entry resolves without the tab; the task is still created (manager-owned); the deferred close finds no tab and does nothing. Verified in code |
| 4 | Closing a tab that started downloads | tasks unaffected — DownloadManager holds no tab state (grep-verified); pause/resume live in the panel, keyed by task id |
| 5 | Window close with panels open | CloseRequested persists download state and exits; panels are window state. Focused(false) may fire on the way out — close_all_panels is idempotent |
| 6 | Popover open while a capture toast fires | 0.32.3 law intact (toast born closed under a panel; opening a panel retires it); close_all_panels retires the toast through the same retire path |
| 7 | Ctrl+Tab with the Downloads panel open | behavior CHANGE (accepted, see Fix A): the panel now closes, matching a tab-strip click |
| 8 | Deferred close landing while the user reads the Downloads panel, adopted tab active | close_tab's reselection focuses the neighbor's content → GotFocus → the panel closes too. Accepted: focus genuinely moved into content; needs the panel opened inside the ≤ 1.5 s cookie window of an active adopted tab |
| 9 | Popup adoption of the start URL itself | the NewWindow arm never marks a start tab adopted (Pico's own page is not a site tab); inert regardless — a capture's live-gate cannot match pico://start |
| 10 | Subframe / download-attribute captures in adopted tabs | live ≠ captured gate → None; no close, no unwind. Covered by `unwind_gates_dominate_origin` |
| 11 | Pause/resume interplay with tab closes | none exists: the manager's tasks, workers and persistence never reference tabs (item 4) |
| 12 | Sibling panels' stuck-open paths | Teleport and the picker share every signal Downloads has, and now the focus funnel too — one law, verified per signal in the Fix A table |

## Process honesty

- The first strings audit of this round's release build
  caught the version bump half-done: Cargo, the display
  constant and the README said 0.32.4 while the chrome
  and start pages' static badges, the chrome JS
  fallback, the start footer and the WinHTTP User-Agent
  still said 0.32.3. The battery could not catch it —
  those surfaces' pins were still mutually consistent
  at the old version; a partial bump is invisible to
  them. The bump was completed across every surface
  (assets_tests and dom_test fixtures included), the
  battery count restarted, and the final audit (below)
  is clean. This is what the strings audit is for.
- `cargo fmt` needed one pass over the new test call
  sites (flagged by the app worker, missed by a
  coordinator spot-check that reported clean against a
  stale read): formatted, battery restarted again. The
  shipped tree is fmt-clean.

## Evidence

- Battery: `./scripts/battery.sh 10` — **10/10
  consecutive PASS** on the final tree
  (battery-0.32.4.log; iteration 10 includes Windows
  clippy + release check). Rust tests 489 → **494**
  (+4 unwind-matrix tests in download_tests, 79 → 83;
  +1 source-scan pin in media_tests, 18 → 19). DOM
  checks **336**, count unchanged (Fixes A/B touch no
  chrome behavior; the version-literal updates ride the
  existing badge/footer checks). Clippy `-D warnings`
  clean on host and Windows targets; `cargo fmt
  --check` clean.
- The Fix A wiring is covered at the strongest level
  this architecture allows: the pure focus-decision is
  the event arm's direct funnel (no intermediate state
  to unit-test), so the pin is structural —
  `source_scan_got_focus_registration_lives_in_build_
  content_view` asserts exactly one `add_GotFocus` in
  the tree's shell source, the attach call inside
  `build_content_view`'s body span, and exactly two
  `close_all_panels()` call sites (the two arms). What
  a source pin cannot prove — that a real click raises
  GotFocus on his runtime — is witness-owed below.
- Release exe: 2,096,640 bytes (< 5 MB), sha256
  `b538132646be23e78168cd498b9158535df1348b944610f2003d4ccf4c268fad`
  (verified changed vs 0.32.3's `46580287…` before
  gating). Gate v2: PASS (RT_ICON ×8 + RT_GROUP_ICON
  ×1, zero RT_MANIFEST, static CRT, WINDOWS_GUI). The
  link's only warning is the known benign LNK4099
  class.
- Strings audit on the shipped exe: "0.32.4" present on
  every user-visible surface (chrome badge + JS
  fallback, start page heading + footer, `Pico/0.32.4`
  transport UA, display version); every remaining
  "0.32.3" hit is a historical comment tag inside the
  embedded pages (the P4-11 class); each social URL
  exactly once.
- dist/pico-0.32.4.zip: Pico.exe + README.md,
  1,309,804 bytes, md5 `cc028d8742d59b14014f9bd6226f5466`;
  the exe inside hashes to the SHA-256 above.
- SECURITY.md gained the 0.32.4 delta at the top: the
  focus funnel is UX state (no data read, no action
  granted), and the download-only close narrows the
  0.32.3 StartPage sentence.

## Still owed (witnesses, the user's machine)

- **The soft98 click, third time:** the download page
  in one tab, the site opens the file in a NEW tab →
  the task is created, the toast names the file, and
  the new tab CLOSES by itself (the close lands when
  the task does — give the cookie fetch its moment;
  normally well under a second). The download page's
  tab is untouched.
- **Panel dismissal:** open Downloads (or the picker,
  or Teleport) and click anywhere in the PAGE — not the
  toolbar, not the tab strip — the panel closes. Alt-tab
  away and back: already closed.
- **Same-tab download** (a link that navigates the SAME
  tab to a file): the tab returns to the page as in
  0.32.3 — it must NOT close; the tab is his.
- Carried from 0.32.3/0.32.2/0.32.1: the cookie-carrying
  download (a login-gated file completes), pause → quit
  → relaunch → resume finishes the same file, a
  cross-site download proceeding WITHOUT cookies, an
  https→http downgrade visibly refused, the
  validator-less resume spot-check line, and the P2-5
  host-only-cookie ABI reading.

# Pico 0.32.3 — Build Notes (2026-11-10)

## What 0.32.3 is

The second on-device fix round, from the user's live
test of 0.32.2 on his own Windows machine — the first
test in which auto-capture WORKED: on soft98.ir he
clicked the download link (`dl2.soft98.ir/.../ShutUp10
.v3.7.1143.rar?token`) and the download DID start in
Pico's manager. Two defects remained, plus one look
defect he reported the same evening:

1. The tab that navigated to the file URL never
   unwound: it sat on the file URL forever, address bar
   showing the raw link, tab spinner reading Loading.
2. Nothing told him the download had started — no
   notification of any kind; he knew only because he
   opened the panel by hand.
3. The address bar's selected text painted the platform
   default: one solid OS-blue block with white text,
   edge to edge — unthemed inside a fully themed shell.

Cargo package 0.32.3, `core::DISPLAY_VERSION` is
"0.32.3", transport User-Agent `Pico/0.32.3`. Cargo.lock
diff vs 0.32.2: the own-package version line only. No
new dependencies.

## Fix 1 — the captured navigation unwinds

### Why the tab stuck (the mechanism, from the code)

- The click is a main-frame navigation to the file URL.
  wry's PageLoad **Started** reaches `App::on_page_load`
  (src/app.rs:4796, `finished == false`), which sets
  `tab.loading = true` and runs `apply_url_event` with
  `CommitStart`: the URL reducer ADOPTS the file URL as
  the tab's live URL — the address bar now shows the
  raw file link and the tab shows its spinner.
- WebView2 then classifies the response as a download.
  The download handler (`with_download_started_handler`,
  src/app.rs:2170) returns **false** — WebView2 cancels
  its own download and the attempt is routed to Pico's
  manager (`UserEvent::DownloadStarting` →
  `App::on_download_starting`, src/app.rs:3012).
- **A download-cancelled navigation never produces a
  PageLoad Finished.** Finished is the only event that
  clears `loading` and the only path to
  `record_commit` (src/app.rs:4671). So nothing ever
  reconciles the tab: the adoption stands, the spinner
  stands, forever. Pico's own history never records the
  file URL (commit happens only on Finished), so
  `tab.hist.current()` still names the download page —
  the fact the fix is built on.

### The unwind

`App::on_download_starting` now calls
`App::unwind_captured_tab(tab, &uri)` (src/app.rs:3232)
as its FIRST step, with the source tab id and the
captured URL the existing `DownloadStarting` event
already carries. The decision is a pure function,
`core::download::unwind_action` (src/core/download.rs:
1145), over the tab state the app already tracks — the
live (adopted) URL, the last committed URL, the
history entry before it, and whether a Back/Forward
traversal is pending:

| Tab state | Action |
| --- | --- |
| live URL ≠ captured URL (subframe / download-attribute capture, or the user already navigated elsewhere) | **None** — never yank a tab back |
| a traversal is pending | **None** — its landing reconciles the tab; do not race it |
| committed page exists and is not the file (the normal case) | **NavigateTo(committed)** — reload the download page from the app's own state |
| the file URL itself was committed, and the entry before it is a different page | **Back** — a real `history.back()` traversal; the landing reconciles index, URL and loading through the ordinary path |
| the file URL was committed with nothing (or only itself) before it, or nothing was ever committed | **StartPage** — convert the tab to the start page in place |

Execution notes:

- **NavigateTo** reloads the committed page with
  `load_url`. History cannot double-record it: the URL
  is already the history's current entry, and
  `record_commit` treats a same-URL commit as a no-op.
  A credentialed committed page re-arms the one-shot
  navigation expectation first (`expect_nav`, the P3-2
  precedent in `go`/`reload`), or the navigation
  handler would deny the restore.
- **StartPage** is a new `convert_tab_to_start`
  (src/app.rs:3315), the mirror of the existing
  `convert_start_tab`: the tab's server root is
  released through `set_tab_root`, the id joins the
  LIVE start-tab set (start classification is read live
  by the navigation handler, so the existing view
  follows the flip), the record is reseeded exactly as
  a fresh start tab's, and the view loads `pico://start`
  (a viewless tab is rebuilt via the A6-3 retry). The
  window is never closed and the tab is never closed:
  the mission's tab-close option was not needed once
  conversion covers every no-previous-page shape.
- **Loop safety** is structural: the restore targets
  are pages (never the file URL — the
  previous-entry-is-the-file case falls to StartPage),
  and after any restore the tab's live URL no longer
  equals the captured URL, so a duplicate or late
  capture event decides None. Pinned by
  `unwind_is_consumed_by_its_own_restore`.

### The reject path — verified, then treated the same

The mission asked whether refused captures (0.32.2's
failed rows) also need the unwind. Verified in code:
the handler cancels identically in both paths (it
returns false unconditionally, BEFORE triage runs), so
a rejected URI that DID enter the tab's state would
wedge it exactly the same way — `on_download_starting`
therefore unwinds before the triage branch, and both
paths share the call. The shapes that never enter tab
state need nothing and get nothing: the URL reducer's
adoption gate (`urlstate::usable`) requires a web
origin, so `blob:` / `data:` / non-allowlisted captures
are never adopted as the live URL, and
`on_page_load`'s allowlist branch actively CLEARS the
loading flag for a non-allowlisted Started — for those,
`unwind_action` decides None on its own facts (the
live-URL gate fails). The unwind is thus unconditional
in placement and self-gating in effect.

## Fix 2 — "download started" feedback: the toast

Chrome-parity, in the chrome webview only:

- **Plumbing.** The snapshot gains `downloadsNotice`:
  `{ seq, kind, name, reason, open }`
  (`DownloadNoticeSnap`, src/app.rs). `seq` is a
  session-wide monotonic counter bumped ONLY when a
  task is created or a capture refused — never by
  progress, restore or resume — and hashed into the
  snapshot signature, so the notice's snapshot always
  goes out. The name is the manager's locked, deduped
  file name (`DownloadManager::name_of`,
  src/downloads.rs:527), not a second derivation.
- **Creation sites only.** `create_pending_download`
  (a capture's task exists) and the panel's Add URL
  show `started`; the triage Reject arm shows
  `blocked` with the plain reason when `add_failed`
  records its row. Tasks restored at startup (paused
  loads) and resumes create NO notice — stated here
  and in the type's doc comment, per the mission.
- **The toast is app-owned state, like the panels.**
  DEVIATION from the mission's sketch, with reason:
  the sketch had chrome hide the pill on its own ~4 s
  timer. The chrome HWND is only 86 DIP tall with no
  panel open, and the pill hangs below the toolbar —
  a chrome-timed hide would leave the HWND extended
  (a dead band over the page) or clip the pill's last
  frames. So Rust owns the window, exactly as it owns
  the panels': the notice is born `open` (unless a
  panel is already open), one timer thread per notice
  (the cookie-timeout pattern) fires
  `UserEvent::DownloadNoticeExpired { seq }` after
  4 s, and the arm closes the notice only if the
  sequence still matches — a newer capture replaces
  the text and re-arms, never cut short by the older
  timer. While open, `chrome_phys` extends the chrome
  rect by `core::layout::downloads_toast_h()` (36 DIP:
  the pill's 34 DIP height closed form — 18px line +
  2×8px padding, anchored at `DL_TOAST_TOP` 78 — plus
  shadow slack, minus the chrome already present);
  the geometry is a three-party contract (CSS,
  DL_TOAST_* constants, asset pin) like the popover's.
  Opening ANY panel retires the notice (the Downloads
  command included), and the toast's own click hides
  it locally and sends the ordinary Downloads command.
- **Chrome side.** `#dl-toast` (top 78, right 12 — the
  popover's anchor) mirrors `open` and renders on a
  sequence change: lead "Download started" + the file
  name (ellipsized at 360px), or the blocked variant —
  lead "Download blocked" in `var(--warn)` + the
  reason. The downloads badge pulses once per new
  sequence (`@keyframes dl-pulse`, re-armed by class
  removal + reflow; cleaned on `animationend`). All
  strings arrive via textContent — task names are
  network data, same rule as the rows.

## The address bar look rework (added workstream)

Scope held to styling: no behavior changed (Enter /
Escape / select-all-on-focus semantics untouched, no
tab-strip changes).

- **Before:** selected URL text painted the WebView2
  platform default — a solid OS-blue block, white text,
  edge to edge; the text caret was the platform color;
  focus was already Pico's (the wrap's shadow glow).
- **Selection:** a document-wide `::selection` rule in
  the chrome paints `background: var(--selection);
  color: var(--text)`. `--selection` is a NEW theme
  token (`core::themes`): each palette's accent at the
  largest alpha from {0.42, 0.36, 0.30, 0.24, 0.18}
  whose composite over the surface keeps `--text` at
  WCAG AA (≥ 4.5) — derived per palette, both modes,
  pinned for all 32 derivations by
  `selection_keeps_text_aa_for_all_32`. Document-wide
  on purpose: the shell's other inputs (Add-URL,
  Teleport URL) select in the same language — one
  input language for the chrome.
- **Caret:** `#addr` gains `caret-color:
  var(--accent)`. (The other two chrome inputs keep
  the inherited caret for now; the address bar was the
  reported surface.)
- **Focus:** assessed, already compliant — the wrap's
  `:focus-within` inset + focus ring + glow IS the
  established neumorphic input language the Add-URL
  field shares, and the input itself draws no ring
  (both pinned since v19/0.32). No change made.
- **Loading:** assessed — the bar carries no
  loading-specific chrome by design (the spinner lives
  in the tab); no raw edge exists to fix. No decoration
  invented.
- **Evidence:** visual-check/pages-v323 renders the
  bar idle and focused-selected on Forest dark and
  Mist light at 1280 + 2560, eyeballed: the selected
  URL sits on a soft accent-tinted band in the theme's
  own text color on both, starting exactly at the text
  (the globe glyph outside it). DOM pins assert the
  two CSS rules in the assembled page; the Rust pins
  assert the rule bodies and the token derivation.

## Evidence

- Battery: `./scripts/battery.sh 10` — 10/10
  consecutive PASS (battery-0.32.3.log). Rust tests
  477 → **489** (+8 unwind matrix in download_tests,
  +2 asset pins — toast geometry contract,
  selection/caret — +1 themes selection-derivation,
  +1 layout toast closed form). DOM checks 324 →
  **336** (+10 toast behavior, +2 address CSS).
  Iteration 10's Windows clippy + release check clean;
  `cargo fmt --check` clean.
- Visual: visual-check/gen_v323.py renders toast
  started/blocked, the popover regression, toast on
  Mist light, and the address bar idle/selected on
  Forest dark + Mist light (visual-check/pages-v323,
  1280 + 2560) — all eyeballed: pill legible at the
  popover's anchor with no overlap of the tab strip or
  the (closed) popover; the blocked variant leads in
  warn; the selected URL reads on its tint in both
  modes.
- Release exe: 2,093,568 bytes (< 5 MB), sha256
  `46580287e1ca8a5b992f7f0bf38f53230b11d82e261beca8a0a12a56e3996233`
  (verified changed vs 0.32.2's `36c4f491…` before
  gating). Gate v2: PASS (RT_ICON ×8 + RT_GROUP_ICON
  ×1, zero RT_MANIFEST, static CRT, WINDOWS_GUI). The
  link's only warning is the known benign LNK4099
  class.
- dist/pico-0.32.3.zip: Pico.exe + README.md,
  1,308,255 bytes, md5 `68ea96be19550e575322be064e68fc9a`.
- SECURITY.md gained a short 0.32.3 delta: the capture
  surface is unchanged; the unwind restores only
  already-committed tab state through the ordinary
  navigation gate; toast strings are text-only.

## Still owed (witnesses, the user's machine)

- The 0.32.2 test repeated on 0.32.3: click the
  download link on soft98.ir → the download starts,
  the toast appears naming the file, and the tab is
  back on the download page (no Loading, no file URL
  in the bar). Also: select the address text and see
  the themed tint.
- Carried from 0.32.2/0.32.1: the cookie-carrying
  download (a login-gated file completes), pause →
  quit → relaunch → resume finishes the same file,
  a cross-site download proceeding WITHOUT cookies,
  an https→http downgrade visibly refused, the
  validator-less resume spot-check line, and the
  P2-5 host-only-cookie ABI reading.

# Pico 0.32.2 — Build Notes (2026-11-10)

## What 0.32.2 is

The first on-device fix round for the download manager,
built from the user's first real test of 0.32.1 on his own
Windows machine (screenshots + his words). The engine
itself was witnessed working — a 21.4 MB file from
dl2.soft98.ir downloaded multi-connection (32 chosen) —
but three things failed around it: clicking the site's
download link produced NOTHING (he had to copy the link
and paste it into Add URL), Pause visibly lagged, and the
panel was a full-width centered sheet over a dimmed page
that did not close on outside clicks. Cargo package
0.32.2, `core::DISPLAY_VERSION` is "0.32.2", transport
User-Agent `Pico/0.32.2`. Cargo.lock diff vs 0.32.1: the
own-package version line only. No new dependencies.

## Fix 1 — a clicked download can no longer vanish silently

The whole interception path was traced in code and each
hypothesis tested:

- **(a) Handler registered on only some creation paths —
  KILLED.** Exactly two `WebViewBuilder` constructions
  exist in the tree: the chrome's (deny) and
  `App::build_content_view`, and EVERY content view —
  initial tab, new tab, popup/new-window adoption,
  retry — is built by the latter, which registers
  `.with_download_started_handler`. A new source-scan pin
  (tests/media_tests.rs) asserts the handler occurs
  exactly twice, one inside `build_content_view`'s body
  span and one inside `build_chrome`'s, so a future
  creation path cannot quietly miss it.
- **Shield-neuter variant — KILLED by experiment.** The
  shipped filter engine was run against the actual
  soft98 URLs (page + dl/dl2 hosts) under every resource
  type, Document and Subdocument included: `Allow`
  across the board — the click's request was never going
  to be neutered. (The scratch probe was deleted after
  use.)
- **(b)/(c) The silence — REAL, and fixed.** Two exits
  could swallow a download without the user ever knowing:
  the URL-triage refusal in `on_download_starting` (one
  log line; if WebView2 reported the attempt with a
  non-http(s) URI — an intermediate blob:/data: hop is
  the classic shape — the cancelled download simply
  evaporated), and `create_pending_download`, which
  DISCARDED `DownloadManager::add()`'s result, so even a
  manager refusal was invisible. Now:
  - The core owns a pure triage (`triage_download_uri`
    → Accept / Reject(reason) with plain user-facing
    reasons: "empty link", "not a web link", "the link
    has no host", "Pico can only download http/https
    links"), unit-tested over the full shape matrix.
  - Every interception outcome logs one redacted line:
    created (host + cookie count), rejected (reason), or
    creation-failed (manager refusal).
  - A rejected interception with a URL surfaces in the
    panel as a Failed task row carrying the reason
    (`DownloadManager::add_failed` — same caps, dedupe
    and persistence as `add`, born Failed, never queued),
    so "nothing happened" is no longer an available
    outcome. WebView2's own download dialog is still
    never resurrected.
- **What remains genuinely uncoverable from here:** a
  click that never becomes an http(s) navigation or a
  WebView2 download operation at all (a pure JS blob:
  download, or a popup intermediate Pico denies by
  design) produces no signal any WebView2 host can
  catch. soft98's own click mechanics could not be
  reproduced from this VM (the site bot-walls fetches),
  so WHICH shape his click took is witness-owed — with
  the new log lines, the next attempt says in `pico.log`
  which path fired (created / rejected + reason / or no
  line at all, which itself proves the event never
  fired).

## Fix 2 — Pause lands the instant it is clicked

Root cause: `pause()` on a Downloading task only set a
control flag; the status flip waited for every worker to
notice, and workers park inside
`WinHttpQueryDataAvailable`/`WinHttpReadData` under a
30 s receive timeout, with the write sink never checking
control at all. The fix is a per-run generation design
around a pure core gate:

- `core::download::GenerationGate` (begin / invalidate /
  accepts; token 0 never valid; unit-tested: stale tokens
  rejected after invalidate and after a newer begin).
  The manager stamps each supervisor run with the token
  minted at spawn and mirrors it into an atomic the
  workers read lock-free.
- `pause()` (and `cancel()`) for a Downloading task now
  complete IN THE MANAGER, in one lock hold: invalidate
  the gate, fold the live planner's confirmed progress
  into the task state, flip to Paused (or remove the
  record), persist, notify, schedule. The snapshot the
  click itself produces already reads Paused with exact
  bytes; the freed slot starts the next queued task
  immediately. `pause_all_and_persist` invalidates gates
  the same way.
- A stale run can no longer write anything: every
  supervisor state write (`wind_down_stopped`,
  `fail_task`, `finalize_complete`, the phase fold-back,
  the post-prealloc write) is gated on its token still
  being current, and the worker sink refuses its FIRST
  check — generation stale or control stopped → nothing
  is written; bytes in flight but unconfirmed at pause
  are simply not in the map and are re-fetched on resume
  (the planner's confirmed ranges remain the only truth;
  no double-count, no overlap writes). `worker_loop`'s
  claim head checks the generation too, so a worker whose
  run went stale in the instant a resume began cannot
  spin on its dead planner.
- Worst case is bounded anyway: segment reads now carry
  a 5 s WinHTTP receive timeout (`WINHTTP_OPTION_
  RECEIVE_TIMEOUT`, set per request and checked
  fail-closed like the redirect policy; probes keep the
  session's 30 s). A stale worker dies within ~5 s even
  if it never reaches a sink check; on a healthy link a
  timeout is an ordinary retryable failure and retries
  resume from the honest byte count. The 60 s stall
  floor (P1-14) stands behind it.

## Fix 3 — the Downloads popover

The sheet + backdrop are gone. The panel is a compact
dropdown popover: 360px wide, anchored at the window's
top-right under the toolbar cluster (top 78px, right
12px), no dim anywhere, height hugging its content. The
chrome rect extension is now computed from the live task
list (`core::layout::downloads_popover_h` — header 26 +
add row 30 + rows at 66px + 15px per error row, list
capped at 386px, card capped at 480px; empty state
122px card) via `chrome_rect_with_extra`;
teleport/palette keep the fixed 496px extension. The
CSS numbers and the layout constants name each other;
three new layout unit tests pin the closed-form values.

Rows are compact: name + status word, a 4px bar, and one
sub-line — host (ellipsized at 104px), the preformatted
detail (ellipsized), the small per-task connections
select (22px), and 24×24 icon buttons (pause/play swapped
by status, folder on done rows, × to cancel) whose words
live in aria-label/title. The snapshot/IPC surface is
unchanged. Two coordinated pin updates: `dp-backdrop`
left `REQUIRED_CHROME_IDS` (the element is ordered
deleted), and the v19 spacing-scale pin gained the
documented-exception treatment for the popover's four
contract measurements (the #navbar precedent — pinned by
exact string, tied to the layout constants).

Dismissal, all of it wired: the toolbar button toggles;
Escape closes; opening another panel closes it (existing
mutual exclusion); a NEW document-level pointerdown
listener (active only while open) closes it for any
click outside the popover and its button — covering the
tab strip, the address bar and the strip of page under
the extended chrome rect; and on the app side the
navigation-retirement law (active tab's committed
navigation retires panels, AUDIT V26 D3 / 0.31 A1-F1)
now covers the downloads panel too, so following a link
in the page dismisses it. Clicks deeper in the page that
produce no navigation and no chrome event remain
unobservable to the app — the same limit every panel in
Pico has always had — and are documented as such.

Visual proof (read by eye, not just generated):
visual-check/gen_v322.py renders the popover from the
dumped chrome pages — tasks and empty states at 1280 and
2560 wide, Forest dark (shot-v322-*). The popover sits
under the right cluster at both widths, no dim slab,
rows tidy, the very-long filename and host ellipsize,
and at six tasks the list caps with the sixth row
peeking at the fold (the scroll affordance) while the
card's bottom edge stays intact.

## Verification

- Tests first where testable: triage matrix + gate
  semantics (download_tests 68 → 71), layout closed
  forms (lib tests +3), the registration pin
  (media_tests 17 → 18), DOM dismissal/aria updates.
  Full `cargo test --locked --no-default-features`:
  **477 Rust tests, 0 failures** (0.32.1: 470).
- DOM suite: **324 checks, all passed** — counted from
  the suite's own output lines. (The 0.32.1 notes said
  340; a recount of that battery's log shows 320 `ok`
  lines — the 340 was a miscount there, corrected here:
  320 baseline + 4 new = 324.)
- Battery: `./scripts/battery.sh 10` — 10/10 consecutive
  PASS (log: battery-0.32.2.log; iteration 10 includes
  Windows clippy + release check). Clippy `-D warnings`
  clean on host and Windows targets; `cargo fmt --check`
  clean.
- Gate v2: PASS (RT_ICON ×8 + RT_GROUP_ICON ×1, zero
  RT_MANIFEST, static CRT, WINDOWS_GUI), run against the
  shipped binary itself.
- Strings audit: "0.32.2" present (8 hits, incl. the
  `Pico/0.32.2` user agent), each social URL exactly
  once; the single "0.32.1" hit is a historical comment
  inside the embedded chrome page (P4-11), not a version
  surface.
- SECURITY.md: no delta — no claim changed. The
  interception logging is additive, the no-dialog law
  stands, and the pause redesign strengthens the
  standing "progress only ever claims written bytes"
  property without altering it.

## Hashes

- `Pico.exe` — 2,083,328 bytes, sha256
  `36c4f491e95eda05b1b74dd19e65c90c67ea5d63d9e68f5e323bb178f7664baf`
  (0.32.1 was 2,076,672 bytes, sha256
  `1f2c4d196a214897390a9d0508bd3658e20bee85eb11921290b647fc209ab959`;
  the hash changed, as required. As before, relinking
  this tree is not bit-deterministic — the recorded hash
  is the shipped file's, and the exe inside the zip
  hashes identically.)
- dist/pico-0.32.2.zip (Pico.exe + README.md): 1,306,359
  bytes, md5 `a46d75866c653d1ac8680d1cf3dd4dec`.

## On-device witnesses owed

1. **Auto-capture on soft98 (the #1 witness):** click
   the site's download link. Expected: a task appears.
   If it does not, `pico.log` now discriminates the
   cause — a "download task created" line (capture
   worked; look at the panel), a "download rejected"
   line with its reason + a failed row in the panel
   (the URI shape was the problem), or no download line
   at all (the click never became a WebView2 download
   operation — the uncoverable class above).
2. **Instant pause:** pause a multi-connection download;
   the row must read Paused in the same instant, and a
   queued task (if any) starts immediately.
3. **The popover:** position under the button, compact
   rows, and click-anywhere-outside dismissal (tab
   strip, address bar, and following a link in the page;
   a click deep in the page that does nothing else is
   the documented unobservable case).
4. Everything 0.32/0.32.1 still owe (unchanged): the
   first logged-in (cookie) download, pause → quit →
   relaunch → resume, the `downloads: folder …` line,
   a cross-site download without cookies, an https →
   http redirect refused, a validator-less resume
   spot-check in the log, the P2-5 host-only-cookie ABI
   reading.

---

# Pico 0.32.1 — Build Notes (2026-11-10)

## What 0.32.1 is

The audit-repair release for 0.32. The user's order was a
full recheck of everything — all bugs, all holes, anything
security-related — fixed in one round. Four parallel audit
passes ran over the tree (download engine, interception /
cookies / IPC, the whole pre-0.32 tree against SECURITY.md,
and spec/logic + dependencies). Every finding was verified
against the actual code before any fix — this project's
history is that a large share of forwarded findings are
false, stale, or already-deliberate, and it held again.
No new features, no new dependencies (Cargo.lock diff vs
0.32: the own-package version line only, plus one
windows-sys feature flag, `Win32_Storage_FileSystem`).
Cargo package 0.32.1, `core::DISPLAY_VERSION` is "0.32.1",
transport User-Agent `Pico/0.32.1`.

## Audit outcome by pass

- **Pass 1 — download engine security** (23 findings:
  3 High, 7 Medium, 13 Low). 20 real and fixed
  (one partially — P1-5 below), 1 false, 2 accepted by
  design with notes in SECURITY.md.
- **Pass 2 — interception, cookies, IPC** (6 findings).
  The critical question came back CLEAN: every downloads
  IPC command is chrome-transport-only — `UserEvent::
  ChromeCmd` is constructed at exactly one site, in the
  chrome webview's handler; content pages have no path to
  any downloads verb. 5 real and fixed, 1 unproven either
  way (P2-5), no change.
- **Pass 3 — whole-tree vs SECURITY.md** (6 findings).
  Every pre-0.32 claim re-verified against the code: the
  server wall, Host wall, Shield laws, Teleport, Lite
  gating, the conjunctive start gate and log redaction all
  hold. 5 real and fixed, 1 informational (no claim covers
  it), no change.
- **Pass 4 — spec/logic + dependencies** (14 findings).
  13 real and fixed (one shared with Pass 1).
  Dependencies: CLEAN — the full OSV sweep over the
  shipped `x86_64-pc-windows-msvc` graph (104 of 382
  lockfile packages) found zero vulnerabilities; the only
  notice is `instant 0.1.13` (RUSTSEC-2024-0384,
  unmaintained, via notify) — the same accepted verdict
  as 0.31.1. The glib and rand 0.7.3 advisories exist only
  in non-shipped (Linux / dev) graphs.

## The fixes (audit IDs are the passes' own)

**Engine integrity (Pass 1 / Pass 4):**

- **P1-1 (High)** — response bodies streamed into the
  `.part` file and counted as progress BEFORE the status
  was judged; the repair ("rewind") was applied only to
  persisted state at phase end, and shutdown persistence
  used the raw live planner — a genuine splice-on-resume
  window. Root fix: the transport judges status and the
  full Content-Range before streaming a single byte, and
  the rewind machinery is deleted.
- **P4-A1 (High)** — an unknown-total download never
  fetched at all: the open-ended marker segment had
  `remaining() == 0`, so `claim()` skipped it and the
  task "completed" at 0 bytes. The planner now has an
  explicit open-ended mode whose marker is claimable
  exactly once.
- **P1-3 (High)** — Content-Disposition rename race: the
  deduped name was checked under the manager lock but
  claimed much later, so two tasks could share one
  `.part`. The claim is now written into shared state in
  the same lock hold as the dedupe.
- **P1-2 (High)** — no disk-exhaustion guard: the
  server-controlled total drove `.part` preallocation,
  and unknown-total streams were uncapped. Preallocation
  now checks the volume's free space first (fail with an
  honest error), and unknown-total streams re-check every
  32 MB and stop above a 256 MB reserve.
- **P4-A2 (High)** — a steal pushed the split segment at
  the vector END and replan appended fresh segments after
  kept ones, so the persisted map could be unsorted —
  and the load-time honesty check then discarded ALL
  progress. Segments are kept sorted by construction.
- **P4-A3 (High)** — tasks persisted as Queued loaded as
  Queued with no supervisor ever assigned, and `resume()`
  rejected Queued: stuck forever. Loading parks Queued
  as Paused, like Downloading.
- **P1-4 (Medium)** — ranged validation checked only the
  Content-Range START. Now start, end and total are
  judged in the transport before streaming, the worker
  re-checks as defense, and a segment response whose
  ETag/Last-Modified contradicts the task's validator is
  a changed file (restart, never splice).
- **P1-5 (Medium, partial)** — a resume with no shared
  validator rested on total-size equality alone. Pico
  now spot-checks the partial file's first and last
  chunks against the wire before trusting it. A
  mid-file-only change with no validators stays
  undetectable in principle (documented in SECURITY.md);
  weak-ETag string inequality still forces a restart —
  fail-safe, kept.
- **P1-6 (Medium)** — a HEAD `Content-Length: 0` was
  trusted and "completed" an empty file. A zero HEAD
  total is now unusable and falls through to the ranged
  GET probe.
- **P1-7 (Medium)** — https → http downgrade redirects
  were followed, and `If-Range` was sent to every hop
  host. Downgrades are refused; `If-Range` goes only to
  the origin host.
- **P1-8 (Medium)** — the task-state temp name was
  `{id}.json.tmp-<pid>`: two saves of one task in one
  process collided on it. A process-wide sequence number
  makes each temp name unique.
- **P1-9 + P4-A9 (Medium)** — cancel lost two races: at
  the finalize/rename gate, and when the probe errored
  after a stop landed (the task resurrected as Failed).
  The rename gate re-checks control first, and pre-phase
  failures go through a control-aware path where the
  user's stop wins.
- **P1-10 + P4-A8 (Medium)** — workers addressed
  segments by vector INDEX, and a steal-split's owner
  kept writing to its old, larger end — through the
  stolen half, racing the thief. The planner API is
  start-keyed throughout, and a worker's sink is bounded
  by the segment's LIVE end: it writes only the prefix
  that is still its own and retires from the segment.
- **P4-A4 (Medium)** — a connection change on a running
  task re-split ranges but never resized the worker pool
  (the pool was fixed per phase, so 1→32 changed nothing
  until the next resume). The pool is dynamic now: the
  supervisor tick spawns up to the new count, and surplus
  workers retire on tokens when the count drops.
- **P4-A5 (Medium)** — a connection change on a PAUSED
  task was never re-planned on the ranged resume path.
  Resume now re-splits not-yet-started ranges to the
  task's count before the phase starts.
- **P4-A6 (Medium)** — the same change was then silently
  REVERTED at phase end when the supervisor's stale
  snapshot overwrote shared state. Every supervisor state
  write preserves live-mutable fields first.
- **P4-A7 (Medium)** — replan's "started" budget counted
  FINISHED segments, starving the split budget. Only
  kept, unfinished segments count as started now.
- **P1-13 (Low)** — the probe had no stop channel: a
  pause/cancel during probing waited out up to 2×10 hops.
  The probe takes the control predicate and checks it per
  hop and while discarding bodies.
- **P1-14 (Low)** — timeouts bound single operations, so
  a slow-drip server could hold a worker forever. A
  connection delivering under 1 KB in 60 seconds is
  failed as stalled (the floor is ~136 bit/s, far below
  a 300 kbit/s link).
- **P1-15 (Low)** — the redirect-policy and timeout
  `WinHttpSetOption` results were ignored; an unapplied
  policy would have silently re-enabled auto-redirects,
  bypassing every transport law. Both are checked and
  fail closed.
- **P1-16 (Low)** — core accepted uppercase schemes but
  the transport stripped only lowercase, mangling
  `HTTPS://…` redirect targets into relative paths.
  Scheme handling is case-insensitive on both sides;
  foreign `://` schemes are refused outright.
- **P1-17 (Low)** — sanitizer margins: "CON .txt"-style
  names (space before the dot) bypassed the reserved
  check; CONIN$/CONOUT$ were missing; the cap counted
  chars, not UTF-16 units (an astral name could exceed
  NTFS's 255); a dedupe suffix could push a name past
  the cap. All four closed.
- **P1-19 (Low)** — cookies were sanitized only on the
  disk-load path; the live add() path stored them
  verbatim, and a `;` in a value minted extra cookie
  pairs. add() sanitizes captures; the sanitizer drops
  separator-bearing names/values.
- **P1-20 + P4-13 (Low)** — hand-planted state files
  with duplicate task ids aliased tasks, and duplicate
  file names shared one `.part`. Loading now drops
  later duplicate ids and dedupes claimed names among
  non-Complete tasks.
- **P1-21 (Low)** — shutdown persistence flipped
  Downloading → Paused only when a live planner existed;
  a task still probing persisted as Downloading. The
  flip is unconditional now.
- **P1-23 (Low)** — dedupe exhaustion returned an
  EXISTING "(999)" name, which Windows rename replaces —
  "never overwrites" broke exactly there. The counter
  now runs to 99,999 before any fallback.
- **P4-12 (Low)** — a single-stream overflow was
  mislabeled "ranges died". It now reads "server sent
  more bytes than its stated size".

**Interception / cookies / IPC (Pass 2):**

- **P2-1 (Medium)** — cookie capture was scoped to the
  DOWNLOAD's host, so any page could mint an
  authenticated GET at an arbitrary host. Capture is
  gated to the initiating page's site (same host, parent
  domain, or subdomain, either direction); cross-site
  downloads proceed without cookies.
- **P2-2 (Medium)** — no task-count cap (see Limits in
  SECURITY.md).
- **P2-3 (Low-Med)** — cookie Path scope was ignored on
  redirect hops; RFC 6265 §5.1.4 path-matching is
  enforced when the header is recomputed.
- **P2-4 (Low-Med)** — cookie Expires was never read, so
  server-deleted cookies replayed forever from task
  state; expiry is captured (OLE DATE → unix ms),
  enforced at sanitize and send time, and the stored
  task URL drops embedded credentials.
- **P2-6 (Low)** — the log redactor's fallback branch
  never stripped userinfo, so a rejected download_add's
  `user:pass@` could reach `pico.log`. It strips embedded
  credentials before the query/fragment cut now.

**Whole tree (Pass 3):**

- **P3-1 (Medium)** — the page-load gate re-checked
  only the scheme allowlist, so a credentialed URL
  adopted via redirect was never re-checked against the
  typed-expectation gate. Adoption now requires the
  exact marker recorded when the navigation handler
  allowed the URL (core decision table, unit-tested).
- **P3-2 (Low)** — Reload / Back / Forward never re-armed
  the expectation, so a legitimately typed credentialed
  page could not be reloaded or traversed back to
  (fail-closed, but it contradicted the documented
  exception). Traversal and reload re-arm for the exact
  target/current URL when it carries userinfo.
- **P3-3 (Low)** — the expectation was a single global
  slot: arming tab B disarmed tab A. It is per-tab now.
- **P3-4 (Low)** — `parse_shield_prune` lacked the
  `deny_unknown_fields` its sibling parsers carry, and
  prune item paths were logged verbatim (a forged
  "telemetry" entry could plant a URL-with-query in
  `pico.log`). Both parsers deny unknown fields, and
  paths are cut at `?`/`#` at parse time.
- **P3-5 (Low)** — the media probe budget's per-tab map
  was never forgotten on tab close; closed ids
  accumulated for the app's lifetime. `forget_tab` is
  called in close_tab now.
- **Deslop** — `CONNECTION_CHOICES` in the download
  core had zero references (chrome.html hardcodes the
  six choices); deleted.

**Panel (Pass 4):**

- **P4-10 (Low)** — a queued task that already held
  bytes showed only its total; it shows "X of Y ·
  Queued", like Paused.
- **P4-11 (Low)** — a connection count outside the six
  choices rendered a blank select, and done rows left
  the select enabled. The renderer guarantees the
  current value exists as an option (and cleans borrowed
  options up), and done rows disable the select.
- **P4-14 (Low)** — the DOM fixture claimed
  `downloadsActive: 2` for one downloading task; the
  fixture is self-consistent now.

## False / accepted (verified, deliberately not changed)

- **P1-11** (plan overflow) — FALSE as a live bug:
  segment sizes sum to exactly the total by
  construction; the latent addition in replan was
  hardened with `saturating_add` while editing, with no
  behavior change.
- **P1-12** (no loopback/private-IP download check) —
  accepted by design: local-dev downloads are a feature;
  see SECURITY.md.
- **P1-18** (RFC 5987 `filename*` charset taken on
  trust) — accepted cosmetic: worst case is replacement
  characters in a name that still passes the sanitizer.
- **P2-5** (host_only derived from the raw Domain's
  leading dot) — unproven either way (undocumented
  WebView2 ABI detail; matches Chromium's canonical
  form and fails closed). Already an owed on-device
  witness; no code change.
- **P3-6** (IpcAction lacks `deny_unknown_fields`) —
  informational; no SECURITY.md claim covers action
  payloads.

## Verification

- Every pure-logic fix landed test-first: the regression
  test was written against the old behavior, watched
  failing, then fixed. New/changed suites:
  download_tests 51 → 68, addr_ipc 42 → 46, media_tests
  16 → 17, redact_tests 7 → 8. Full `cargo test
  --locked --no-default-features`: **470 tests, 0
  failures** (0.32 baseline: 447). FFI/wiring fixes carry
  the strongest available check instead: the Windows
  target check + clippy, and the source-scan pins in the
  bin suite.
- DOM suite: **340 checks, all passed** (0.32: 317).
- Battery: `./scripts/battery.sh 10` — 10/10 consecutive PASS
  (log: battery-0.32.1.log).
- Clippy clean on host and Windows targets
  (`-D warnings`); `cargo fmt --check` clean.
- Gate v2: PASS (RT_ICON ×8 +
  RT_GROUP_ICON ×1, zero RT_MANIFEST, static CRT).

## Hashes

- `Pico.exe` — 2,076,672 bytes, sha256
  `1f2c4d196a214897390a9d0508bd3658e20bee85eb11921290b647fc209ab959`
  (0.32 was 2,053,120 bytes, sha256
  `cee0def025de7a32f21cde05851e6fcb4127213ec3c883c1aec90f568bc91ba6`;
  the hash changed, as required — this is not the stale
  0.32 binary).

## On-device witnesses owed (carried from 0.32, unchanged)

Everything 0.32 owed still stands — none of it can be
proven from this Linux VM: the first real multi-connection
download and its speed, a logged-in (cookie) download,
pause → quit → relaunch → resume, the panel's look, and
the `downloads: folder …` log line. 0.32.1 adds three
behaviors to witness when convenient: a cross-site
download proceeding without cookies, an https → http
redirect refused, and a validator-less resume spot-check
passing/failing in the log. The P2-5 host-only-cookie ABI
reading also remains an on-device question.

---

# Pico 0.32 — Build Notes (2026-11-10)

## What 0.32 is

The download manager. The user's order replaces the
standing denial law: downloads are no longer denied.
A download started in a content tab is cancelled at
the WebView2 boundary and routed to Pico's own manager.
Downloads from the chrome webview itself stay denied.
The release is named exactly "0.32": Cargo package
0.32.0, `core::DISPLAY_VERSION` is "0.32".

## Architecture

- Pure core planner in `src/core/download.rs`: segment
  planning, dynamic segment splitting (work stealing),
  filename sanitizing and dedupe, cookie-header
  matching, resume decisions, speed/ETA helpers, and
  per-task state load/save. The core makes decisions;
  it does not do I/O beyond task-state files and does
  not log.
- WinHTTP transport (`src/winhttp.rs`): manual redirect
  following, cap 10 hops, scheme re-validated per hop,
  probe (HEAD with a ranged-GET fallback) and ranged
  fetches that stream the body in chunks. No
  decompression, so range math sees raw bytes.
- Supervisor/worker runtime (`src/downloads.rs`): one
  supervisor thread per running task, worker threads
  per claimed segment. At most 3 tasks run at once;
  further tasks queue. While tasks download, progress
  events reach the chrome at most every 250 ms per task.
- Chrome panel: a Downloads panel in the chrome follows
  the existing Teleport/palette panel pattern (app-owned
  open state mirrored from the snapshot, mutual
  exclusion handled by the app). All dynamic strings
  (names, hosts, details) render via textContent.

## The connection-count rule

A task's connection count may be changed at any time.
For a queued task it is the plan used at start. For a
running or paused task it re-splits only the
not-yet-started byte ranges (`Planner::replan_pending`);
in-flight segments are never resized except by
stealing. If every remaining byte is already in flight,
the new count takes effect at the next steal/resume.
Counts are 1–32, default 8, choices 1/2/4/8/16/32, set
per task or as a global default.

## Cookie handling

For a tab download, page cookies are captured from
that tab's profile cookie manager for the exact target
URL. They are stored host-scoped in the task state
inside Pico's data dir, sent only to matching hosts
(recomputed per redirect hop; secure cookies only over
https), and never logged. Download URLs are logged
host-only. Manual
Add-URL tasks carry no cookies. URL userinfo is
stripped for connect and never forwarded.

## Resume validation

Each task keeps a `.part` file plus per-task JSON
state (`<id>.json`) in the state dir, so pause/resume
survives restarts; a task found Downloading on disk
loads as Paused. On resume the task is probed again
and the stored size, ETag, Last-Modified and range
support are revalidated: any mismatch restarts the
task from zero rather than splicing bytes from a
different version. A task with unknown total is
single-stream by construction and cannot resume.
Downloads land in `<exe dir>\Downloads`, with
fallback to the user's OS Downloads folder when the
exe dir is not writable. Filenames are sanitized
(no path separators, no reserved device names, no
trailing dots/spaces, length-capped) and deduped so
a download never overwrites an existing file. Nothing
is auto-opened or executed; the only shell action is
opening the Downloads folder on the user's click.

## Verification

- Tests: **447 Rust + 317 DOM**, all passing (0.31.1:
  394 + 269). New: 51 core download tests (planner,
  stealing, sanitizer, cookie scoping, resume matrix,
  state round-trip), 2 IPC parse tests, 48 DOM checks
  for the panel. Clippy: 0 warnings, host `--all-targets`
  and Windows target. `cargo fmt --check` clean. Zero
  unwrap/expect/panic in shipped source (scan).
- Battery: `scripts/battery.sh 10` — **10 consecutive
  PASS** (log: battery-0.32.log; iteration 10 includes
  Windows clippy + release check), plus one post-fix
  iteration after a coordinator CSS fix (the task
  list's max-height is 380px, not the palette sheet's
  486, so the sheet fits the panel extension with its
  header and add row — battery-0.32-postfix.log).
- Release: `cargo xwin build --release --locked
  --target x86_64-pc-windows-msvc`; the only build
  warning is the known benign LNK4099 class. Gate v2
  (`scripts/check_no_manifest.py --expect-icon`) PASS:
  0 RT_MANIFEST entries, RT_ICON x8 + RT_GROUP_ICON x1,
  no CRT imports, WINDOWS_GUI subsystem; `winhttp.dll`
  joins the import list (the OS stack, as designed).
- Strings audit on the exe: "0.32" present (11 hits,
  incl. the `Pico/0.32` user agent), "0.31.1" absent,
  each social URL (t.me/iK4w4, youtube.com/@iK4w4)
  exactly once.
- One pre-existing pin was amended, not removed: the
  v22 remember-me source scan (tests/media_tests.rs)
  banned the string "cookiemanager" in the shell; the
  0.32 cookie capture is a read-only GetCookies call,
  so the pin now bans the cookie MUTATION surface
  (Delete/AddOrUpdate/Create/Copy cookie APIs) in every
  shell source and keeps the manager's name banned
  outside src/app.rs. Nothing in Pico can clear,
  rewrite, or restrict a cookie — that absence stands.

## Hashes

- Pico.exe: 2,053,120 bytes, SHA-256
  `cee0def025de7a32f21cde05851e6fcb4127213ec3c883c1aec90f568bc91ba6`
  (relinking this tree is not bit-deterministic — an
  earlier link of identical source measured the same
  size with a different hash; the recorded hash is
  the shipped file's).
- dist/pico-0.32.zip (Pico.exe + README.md): 1,289,164
  bytes, md5 `83ecd4f0eeab929488d630d78442a669`; the
  exe inside hashes to the SHA-256 above.

## On-device witnesses owed

The engine's decisions are core-tested and the
transport compiles against the real WinHTTP surface,
but no download has run on the user's Windows yet.
Owed: a first real multi-connection download (speed
against the same file in a browser), a logged-in
download (cookies carried), pause → quit → relaunch →
resume, the panel's look in the real shell, and the
`downloads: folder …` log line confirming the
resolved folder on his machine.

---

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

The repair round for 0.30, from the 0.30 coordinator's
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
One coordinator find along the way: the v24 recents
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
