# Pico — Security Model

## What changed in the public release (0.31)

- **Filter lists are no longer compiled into the executable.**
  The Shield resolves them at runtime: cache in
  `data/lists/` beside the exe, else an HTTPS download of
  the four canonical list URLs (SOURCES.md) on the Windows
  WinHTTP stack — HTTPS only, ≤5 https→https redirects,
  per-file size caps enforced while streaming, no cookies,
  no custom headers, system proxy honored by the OS stack.
  This supersedes the v21-era statements below ("compiled
  into the executable", "no list is fetched at runtime")
  for this edition; the private 0.31 build keeps the old
  delivery. Nothing else about the Shield changed: decisions
  are still computed locally, nothing about browsing is
  transmitted, and the downloader's URL set is a closed
  constant — it fetches the four lists and nothing else,
  ever.
- **Downloaded bytes are untrusted input.** Each file is
  validated (size cap, UTF-8, list header shape, rule-line
  ratio) and the whole set must compile into a sane engine
  (line/rule floors) before anything is cached or installed;
  a failed validation is never cached. The atomic cache
  write (temp + rename) and the compiler's existing
  parse-success gate stand behind it. Failure is fail-open:
  the Shield stays idle, Pico browses normally, one log
  line records why.
- **Pre-download posture.** Until lists land, the Shield
  installs an idle runtime: requests pass, no scriptlet or
  cosmetic payload runs. The inert-but-safe state is the
  same fail-open law the register below already states.

## What changed in 0.30 (delta over the v29 model below)

- **The savings book observes; it never decides.** Blocked
  counts move only inside the request handler, after a
  neutered response has actually been delivered, attributed
  by the `filterpipe::decide` verdict — the same single
  counting point the chrome chips use. Nothing in
  `core::stats` can block, allow, or redirect a request,
  and no page-controlled value can move a blocked count.
- **The `shield_pruned` report.** The scriptlet runtime
  reports the exact serialized bytes of ad subtrees it
  pruned (`{"kind":"shield_pruned","bytes":N,"events":M}`,
  deny-unknown-fields, bytes ≤ 16,777,216, events ≤
  100,000). It is honored only from a live, public,
  non-start content tab with the Shield on, and the book
  applies a per-tab rate floor (1 s; the honest runtime
  flushes every 2.5 s) plus a 1 TiB session ceiling. A
  forged report can therefore inflate ONLY the pruned
  figures of the forging tab's session, bounded and cured
  by the band's own Reset control; it cannot touch blocked
  counts, the estimates, or any privileged path.
- **The response ledger samples metadata, never content.**
  For public responses the response observer reads two
  header VALUES (`Content-Length`, `Content-Type`) and the
  request object's identity token (for duration
  correlation). No URL, no body, no other header is read
  for statistics, and nothing per-request is persisted —
  the ledger stores only per-type totals (bytes, count,
  duration, count). Loopback and internal responses are
  never sampled. Pico's own neutered responses are
  structurally excluded: blocked requests are never noted
  for correlation, and neutered responses carry no
  `Content-Length`.
- **Correlation state is capped and expiring.** In-flight
  timing entries are bounded (2,048 entries, 3-minute TTL,
  oldest-first eviction) inside `core::stats::PendingTimings`;
  a response that cannot be correlated records no duration
  (bytes still calibrate by Content-Type). The worst case
  of a token collision is one misattributed calibration
  sample — a statistic, never a decision.
- **`stats-reset` rides the existing dispatch.** The band's
  Reset control is a start-page action through the same
  conjunctive gate (start tab AND start page) and the same
  two transports as every other privileged gesture; there
  is no second path to zeroing the book. 0.30 also widened
  the navigation handler's action classification to the
  core classifier, so the v24 recents actions' anchor
  transport reaches that same dispatch (previously it
  dead-ended silently; the IPC transport was unaffected).

## What changed in v29 (delta over the v28 model below)

- **Lite chosen-video model.** While Lite is on, each page
  context may designate ONE chosen media element (a tap on
  the element or its controls container; the next large
  video after a tap-followed route change; the first large
  video on a fresh watch-shaped page). The chosen element's
  playback is allowed; every other element's `play()` is
  refused. v29 removes v28's document-activation pass: it
  blessed the document, not an element. Hover never
  designates. The gate restores the exact original `play`
  (and its history hooks) on `restore()`, resets its
  designation on route change and Lite toggle, and keeps
  the loopback/internal exemption.
- **The `lite_chosen` report.** On designation the gate
  posts one payload-free IPC body (`{"kind":"lite_chosen"}`,
  unknown fields rejected) over the existing content IPC
  channel. It is a hint, never a command: the app honors it
  only from a live, public, non-start content tab with Lite
  on, and its only effect is setting that tab's chosen flag
  in the Lite runtime — a fact the request pipeline reads.
  A forged report can therefore cost at most the tab's own
  Lite data-saving on media/stream requests until the next
  navigation, toggle or tab close clears it; it cannot
  unblock images or fonts, move a counter, or reach any
  privileged path (the start-page action gate is a separate,
  conjunctive check this channel never touches).
- **Lite stream backstop, narrowed.** Fetch/XHR-class
  media-stream shapes are LiteBlocked only when the page is
  not watch-shaped AND the tab has no chosen video; Media-
  class requests get the same chosen-context exemption.
  Images and fonts are Lite-blocked unconditionally on
  public pages. Innertube/API JSON is never a stream shape.
  Accepted gap, by design: on a watch-shaped page (or in a
  chosen tab) preview SEGMENTS may download — the play gate
  is the guarantee they can never play; the backstop is a
  data-saver, not the playback control.
- **State-save failure logging is latched.** A persistent
  save failure logs one error line per session instead of
  one per save attempt (saves ride navigation notes, so the
  unlatched behavior could grow pico.log without bound).
- **Nothing else moved.** Shield filter lists are
  byte-identical to v28; the security register below stands.

## What changed in v28 (delta over the v27 model below)

- **Lite play gate.** While Lite is on, a page's media
  `play()` calls require genuine user intent (real
  pointer/click/key gestures or document user activation;
  hover never counts). The gate ships inside the Lite
  document script, follows the live Lite toggle via
  evaluated scripts on live views, restores the exact
  original `play` on `restore()`, and applies the same
  loopback/internal exemption as every other Lite law. It
  changes no page data and sends nothing anywhere; its only
  effect is refusing un-gestured playback, which is Lite's
  standing purpose.
- **Lite stream backstop.** Fetch/XHR-class requests whose
  URL path is a media-stream shape are LiteBlocked while
  Lite is on. Innertube/API JSON is never a stream shape —
  the Shield's response-transform path for those endpoints
  is untouched.
- **Teleport strips credentials.** Public Teleport URLs
  (QR + copyable text) no longer carry authority userinfo.
- **Page-minted credentialed URLs are denied.** A web page
  cannot open a popup whose URL embeds userinfo (the
  trusted-prefix spoof shape). URLs the user types keep
  their userinfo — that is the user's own act.
- **Migration can no longer clobber a portable profile.**
  The migration planner is profile-aware; profile copies
  precede the (now atomic) state copy, and per-entry copy
  errors are counted, not swallowed.
- **Log hardening.** Invisible Unicode format characters
  (line/paragraph separators, bidi controls, zero-width
  marks) are stripped before any value reaches pico.log —
  they rendered as line breaks and could forge log lines.
  The "startup argument ignored" line is redacted first.
- **Address-bar origin integrity.** Pre-commit observer URL
  notes are origin-gated against the seed URL as well, and
  address submissions on failed tabs retarget the tab
  record before any view is built.
- **Nothing else moved.** Shield filter lists are
  byte-identical to v27; the request rules themselves are
  unchanged except where AUDIT.md's V28 entries record a
  confirmed defect fix (HEAD now passes through the Shield
  method gate; POST remains never Shield-blocked).


# Pico v27 — Security Model

## What changed in v27 (delta over the v26 model below)

- **Portable profile (user-ordered).** Pico's WebView2
  profile — cookies, site storage, and any signed-in
  session (e.g. YouTube) — now lives in `data\` beside
  Pico.exe instead of %APPDATA%. Plainly: **whoever can copy
  the Pico folder can copy the logins inside it.** That is
  the user's explicit trade for "delete everything with
  ease"; no encryption is claimed or implied, and none was
  present before either (the appdata profile was equally
  readable by the account's other processes). The migration
  copy leaves the old appdata profile untouched as a backup
  — deleting it is the user's call, and README does not
  pretend otherwise.
- **Theme system is local-only.** Palettes are a compiled-in
  table; switching themes makes no network request, loads
  no remote asset, and accepts no page-supplied value: the
  `theme_set` id is length-bounded at IPC parse and
  re-validated against the table by the app before anything
  persists (unknown ids are logged and ignored). Theme
  state rides the existing single state writer.
- **Data-dir fallback never widens access.** When the exe
  folder is not writable, Pico falls back to the previous
  %APPDATA%\Pico location and logs why; it never runs
  data-less and never picks a world-writable substitute.


## What changed in v26 (delta over the v25 model below)

- **Live URL tracking (SourceChanged).** Every content
  webview now reports its live `Source` to the shell, and the
  tab's stored URL follows it through a pure reducer
  (`core::urlstate`). The security-relevant direction is the
  one this closes: through v25 the shell's idea of a tab's
  URL could be arbitrarily stale (frozen at an old commit),
  and every consumer of that value — the Shield's
  party/`domain=` context, the media probe's facts, the
  live-reload root scope, Teleport's payload — inherited the
  staleness. All of them now key off the live value.
  Observer-script notes were NOT promoted in trust: in the
  pre-Source window they still pass the same-origin gate, and
  after Source settles a document, a disagreeing note is
  refused as stale. Source itself is the platform's value,
  never page-supplied.
- **Start-tab conversion closes the privileged gate.** An
  address-bar submission on a start tab now converts the tab
  in place (D5). The conversion flips `is_start` and removes
  the tab from the live start set in the same instant it
  issues the load, so (a) the action dispatch's conjunctive
  gate (`Requester` is assembled from the tab's live fields)
  denies every start action from that webview from that
  moment — pinned by a dispatch test — and (b) the webview's
  own navigation handler, which reads the live set rather
  than a build-time capture, stops applying start-tab rules
  to it. Start-page launcher actions are unchanged (they
  still open new tabs and still pass the same gate on the
  start tab itself).
- **One filter pipeline, one origin classifier.** Shield and
  Lite decisions now come from a single pure function with a
  single loopback/internal classifier (127/8 + localhost +
  *.localhost + ::1). Immunity is therefore identical in
  both layers by construction; through v25 the layers spelled
  "local" differently and a 127.0.0.2 page fell between them.
  Hostless/opaque URLs (data:, about:) classify as local —
  the fail-open direction, chosen deliberately and uniformly.
- **Collapse layer.** The new document-layer script that
  hides filter-broken images is inert on loopback/internal
  pages (same classifier, spelled identically in JS), acts
  only while at least one filter is on (the app maintains
  its disarm flag on every load; both-off is byte-identical
  passthrough), and touches only `display` on img/picture
  elements that failed to load. It reads no page data and
  reports nothing.
- **Layout witness.** pico.log gains a `layout:` line
  (window size, scale, chrome/content rects, panel state)
  whenever geometry changes. It contains no URLs and no page
  data — it exists so a layout defect is provable from the
  log the user already knows to send.
- **Unchanged.** The static server (loopback-only, Host
  wall, containment), the scheme allowlist, the 30-tab cap,
  download denial, profile/autofill handling, log redaction,
  state-file sanitization, and the no-manifest/icon-only
  resource law (gate v2 re-run PASS) all stand as documented
  below. No new dependencies; no new network surface.


## What changed in v25 (delta over the v22–v24 model below)

- **Teleport's LAN address stays on this machine.** To rewrite
  a loopback URL for the phone, Pico enumerates the PC's own
  IPv4 adapters (`GetAdaptersAddresses`) and reads only each
  address and its up/down state. The chosen address is used
  exactly once: baked into the URL inside the QR code the user
  asked to see, in-process. It is never transmitted anywhere,
  never logged (the `teleport:` log line carries only the
  outcome kind and the *original page's* host, redacted per the
  standing log laws), and no adapter inventory (names,
  descriptions, MACs) is read at all. Scanning the QR is the
  user's act, on their own phone, on their own network.
- **The built-in static server is unchanged.** It still binds
  127.0.0.1 only, with the same Host wall, root tokens, and
  containment. Teleport never rewrites a Pico-served URL —
  those pages are NotTeleportable by design (AUDIT V25-4).
  Dev servers become reachable from the phone only when the
  *user's own server* binds the LAN (e.g. Vite's `--host`),
  which is the user's decision in their terminal, not a Pico
  behavior.
- **Lite Mode is local-only filtering.** It adds no network
  surface: the same request handler, the same neutered
  responses, decisions made entirely on-device from the
  request's class and origins. Its persisted trace is one
  boolean (`lite_on`). Its counters are in-memory, per session,
  and aggregate (no hosts or URLs are recorded for Lite blocks
  beyond the same host-only, 20-line-capped log samples the
  Shield has always kept).
- **No new dependencies.** One additional *feature flag* on the
  already-used `windows-sys` crate (Ndis types gate the
  IpHelper API in windows-sys 0.59). The QR encoder is
  first-party pure Rust (`core::qr`), golden-vector verified
  against an independent implementation (RESEARCH_v25.md §a).

Everything below is the v22 security model as carried through
v23 and v24, with the v25 deltas above.

---

## What changed in v24 (delta over the v22/v23 model below)

- **Address-bar search sends typed prose to Google — stated
  plainly.** v24 adds ONE new outbound URL shape: when the user
  types address-bar text that is not a URL, not a local
  host:port, not an existing file and not a plausible bare
  host, pressing Enter navigates to
  `https://www.google.com/search?q=<the typed text,
  percent-encoded>`. The query the user typed goes to
  google.com — that is the feature. Nothing is transmitted
  while typing (no suggestions, no prefetch, no telemetry);
  the text leaves the machine only as the navigation itself,
  exactly like typing the same words into any browser's search
  bar. No client id or referral parameters are added.
- **The exe carries an icon-only resource section.** v24's exe
  has a `.rsrc` containing RT_ICON ×8 + RT_GROUP_ICON ×1,
  compiled by llvm-rc from a generated .ico (AUDIT V24-1). It
  contains ZERO RT_MANIFEST entries — gate v2
  (`scripts/check_no_manifest.py --expect-icon`) enforces this
  on the shipped binary — so the activation-context posture of
  the manifest-free versions is unchanged. The runtime window
  icon (tao, from embedded RGBA bytes) involves no resource
  section at all.
- **Recents management adds no new surface.** Removing a
  recent, clearing the list, and hiding the section are decided
  by the same gated start-page dispatch as opening one (start
  tab AND start page, checked before parsing; every decision
  logged). The persisted state gains one boolean
  (`recents_hidden`); no new files, no new network surface, and
  the recents list itself remains local-only, as before.

Everything below is the v22 security model as carried through
v23, with the v24 deltas above.

---

# Pico v22 — Security Model

Pico is a single-user, local-development browser. This document is
the honest register of what it defends against, how, and what it
deliberately does not. Every control below is implemented in
`src/core/` or `src/app.rs` and most are pinned by a named regression
test in `tests/`. (This document was rewritten for the v22 tree;
where it differs from the v21 edition — the stale "no ad blocker"
non-goal, the schema version, the user-agent rationale — the v22
text is the correct one. See AUDIT.md V22-13.)

## Assets and adversaries

Assets: the user's local files (readable through the static server),
their WebView2 profile data, and the integrity of the chrome UI.

Adversaries in scope:
1. **A malicious or compromised website** open in a tab — it must not
   read local files, invoke privileged UI actions, or escape its tab.
2. **A malicious web page on the user's LAN / a DNS-rebinding site** —
   it must not reach the loopback file server.
3. **A hostile local HTML file** the user opens — it is content, not
   code with Pico privileges.
4. **Network-adjacent attackers** are out of scope beyond the above:
   the server binds 127.0.0.1 only, nothing listens on LAN interfaces.

## Controls

### Static file server (`src/core/server.rs`)
- **Bind:** 127.0.0.1 only, OS-assigned port.
- **Host wall (anti DNS-rebinding):** every request — files *and* the
  SSE endpoint — requires a Host of `localhost`, `127.0.0.1`, or
  `::1`; a present port must equal the server's. Foreign names, wrong
  ports, and userinfo-shaped hosts get 400/403. No CORS headers are
  ever emitted, so even a same-host page from another origin cannot
  read responses. (`host_header_wall`) Scope, stated precisely
  (AUDIT V29, A5-10): the wall stops rebinding *names* — a public
  origin addressing `127.0.0.1:<port>` directly sends a passing
  Host; the barriers there are the OS-random port, the unguessable
  64-bit root token, and no-CORS (its responses are unreadable
  cross-origin).
- **Roots are capabilities:** a folder becomes servable only through
  `register_root`, returning a random 64-bit token (OS-seeded
  hasher + pid + time + counter through a splitmix finalizer). URLs
  without a live token 404. Roots are **refcounted**: the last
  releasing tab unregisters the root and its event endpoint
  (AUDIT M2). (`root_lifecycle_register_release_and_refcounts`)
- **Path policy:** the request path is percent-decoded exactly once
  (double-encoding stays literal — `percent_escaped_names_resolve`),
  split, and normalized with backslashes, NULs, `:`-suffixed ADS
  names, and Windows device names rejected; the result must be
  contained by the canonical root. A 13-case traversal corpus runs in
  `traversal_corpus_never_escapes_the_root`.
- **DoS bounds:** 64 concurrent connections (excess load is shed, not
  queued — `connection_cap_sheds_load_and_recovers`), 16 KiB request
  head cap (431), 3 s per-read header timeout plus a 5 s total head
  deadline enforced inside the line reader (slow-loris,
  `silent_peer_is_cut_by_timeout`), GET/HEAD only (405), at most 100
  requests per connection, 5 s keep-alive idle timeout, 8 SSE
  subscribers per root (503 beyond — `sse_subscriber_cap_is_enforced_per_root`).
- **HEAD discipline (AUDIT V22-1/V22-2):** a HEAD response carries
  headers only on every path, including 416 range failures and the
  events endpoint's errors; GET keeps its bodies. Pinned by
  `head_range_unsatisfiable_has_no_body`,
  `head_on_events_endpoint_is_headers_only_and_not_a_subscription`,
  and `head_responses_match_get_headers_without_bodies`.
- **Live reload is per-root (AUDIT M1):** the SSE stream lives at
  `/__pico/events/<token>`; a broadcast for root A is undeliverable to
  root B's pages (`sse_broadcast_reaches_only_its_own_root`). Injected
  reload scripts are added to HTML responses only, never to partial
  (Range) HTML.
- **Responses** carry `X-Content-Type-Options: nosniff`,
  `Cache-Control: no-store`, `Referrer-Policy: no-referrer`.
  Range requests are supported for media (`range_requests_over_the_wire`,
  206/416 per the parser matrix in `range_parser_matrix`).

### Navigation and tabs (`src/app.rs`, `src/core/addr.rs`)
- **Scheme allowlist** for document navigations: `http:`, `https:`,
  and Pico's own server URLs. `javascript:`, `data:`, `file:`,
  `view-source:` etc. are refused both in the navigation handler and
  again at page-load state tracking (defense in depth).
  (`navigation_allowlist`)
- **Start-page actions go through ONE dispatch (AUDIT V17-1):**
  every start-page gesture — recent file, file picker, service
  card, social chip — is decided by the pure function
  `core::action::dispatch` and by nothing else. Clicks arrive
  over `window.ipc` (`{kind:"action", …}`) or, as a fallback,
  as legacy `pico://…` anchor navigations; both transports are
  parsed into the same request type and the wry handlers that
  carry them only classify and forward. The gate is conjunctive
  and evaluated from the app's own tab state, never from
  page-supplied data: the requesting webview must BE the start
  tab AND that tab must currently be showing the internal start
  page — a start tab that has navigated away loses the
  privilege, and any other webview (even one somehow displaying
  the start URL) is denied. Gate checks run before transport
  parsing, so a hostile page cannot probe the action parser
  through a denied request. Service targets must pass the same
  navigation allowlist as typed input; social targets are the
  `SocialSite` enum only. Every dispatch — allowed or denied —
  is logged with its classified decision (file basenames only,
  service URLs redacted), so a refused action is always
  diagnosable from pico.log. (`tests/action_tests.rs` pins the
  full requester × action matrix for both transports.)
- **Start tab containment:** the start page cannot navigate itself;
  web links from it open in a new tab through the dispatch above.
- **New-window routing (AUDIT V22-10/V22-15):** `window.open` /
  `target=_blank` from a content webview never creates a browser
  window. The target is decided by the pure function
  `core::addr::route_new_window`: internal action URLs are denied
  (`internal-action-url` — a page cannot mint a privileged action
  by opening a window), allowlisted http/https (and the start
  page) open in a NEW TAB under the same 30-tab cap and with the
  Shield applying, and every other scheme (`javascript:`, `data:`,
  `file:`, `about:blank` script-popups, …) is denied
  (`scheme-not-allowed`). Every decision is logged. The chrome
  webview's own new-window requests remain denied unconditionally
  (AUDIT V15-4). The full matrix is pinned in
  `tests/newwindow_tests.rs`. One documented trade-off: tab
  routing severs `window.opener`, so opener-dependent popup
  widgets on sites with no full-page sign-in path may not
  complete (AUDIT V22-11).
- **Tab cap:** 30 tabs; `window.open` storms cannot exhaust memory
  with WebViews.
- **Downloads are denied** at the wry handler — there is no downloads
  manager by design — and counted, with the count + last URI shown in
  the chrome (the log copy of the URI is redacted).

### IPC (`src/core/ipc.rs`)
- Chrome commands are typed JSON with a protocol version (`v: 1`;
  newer versions are rejected, never guessed), a per-page-load boot
  id, and monotonic request ids deduped in a bounded set — a replayed
  or double-fired command cannot act twice.
  (`commands_reject_garbage`, `dedupe_drops_replays_and_resets_on_new_boot`,
  `dedupe_stays_bounded`)
- **Destroyed generations cannot speak (AUDIT V15-1):** when the
  watchdog recreates the chrome, the dead page's boot id is retired
  in the dedupe set; any of its still-queued commands (including a
  late `ready`) are dropped without touching the live generation's
  state, so a stale handshake can never mark a dead replacement
  shell healthy. (`dedupe_retired_boot_is_dropped_forever`)
- Size caps: 64 KiB command bodies, 8 KiB navigate text (boundary
  tested), 16 KiB nav-note bodies, 512-char titles.
- Content-page navigation notes are hints, never commands: they must
  be http(s), and the app drops any note whose origin differs from
  the tab's current page (a page cannot spoof another origin's
  address bar or history).

### Local-file opening (`src/core/addr.rs`, `src/core/fileopen.rs`, `src/app.rs`)
- **UNC/device paths are rejected lexically before any filesystem
  call** — resolution of `\\host\share` can trigger SMB
  authentication (NTLM disclosure), so the check is purely textual
  and runs first. (`unc_paths_are_refused_as_input`,
  `resolve_unc_is_refused_before_any_filesystem_touch`)
- **Resolution is a tested core function (AUDIT V17-2):**
  `resolve_open_target` owns the gate → canonicalize → folder/rel
  split → existence check for every file-open entry point
  (recents, picker, drag-and-drop, argv, address bar). The
  existence check joins the name onto the *folder*; the v13–v16
  in-line version joined it onto the file itself and rejected
  every regular file (correctness bug, now regression-pinned by
  `resolve_file_points_at_the_file_itself`).
- Files are never loaded as `file://` documents; they are served by
  the static server so the path policy above applies uniformly.

### WebView2 profile (`src/app.rs`)
- Password autosave and general autofill are disabled via
  `ICoreWebView2Profile6` on every webview (chrome and content).
  Failure is logged, not fatal (older runtimes may lack the
  interface). Those are the ONLY two profile settings Pico
  touches (AUDIT V18-1 inventory, re-verified V22-12).
- The profile itself is the fixed folder
  `%APPDATA%\Pico\WebView2` (derived once in
  `core::state::webview_data_dir`, pinned by test): the same
  folder every launch, never deleted, relocated, or run in
  private mode — cookies and signed-in sessions persist across
  restarts by design. The state.json sanitizer cannot reach it
  (it sanitizes an in-memory value whose fields are theme,
  recents, window geometry, and the Shield fields only).
- **Remember-me is an audited absence (AUDIT V22-12/V22-16):**
  nothing in the tree restricts cookies or site storage (no
  cookie-manager calls, no third-party-cookie policy), and no
  code path clears cookies, storage, or cache. This is pinned
  by a source-scan test that fails if the relevant WebView2 API
  names ever appear in the shell sources, and witnessed at
  startup by the log line `profile: persistent, cookies db
  present=<bool>` — an existence check on the profile's Cookies
  database (both WebView2 layouts); its contents are never read.
- **User agent (v18; form changed in v22, AUDIT V22-14):** every
  content WebView sets an explicit UA composed at startup from
  the installed WebView2 runtime's version
  (`core::media::compose_user_agent`; the version input is
  sanitized to digits/dots before it can reach a header). The
  form is the standard reduced desktop-Chrome UA
  (`… Chrome/<runtime-major>.0.0.0 Safari/537.36`): the v18–v21
  Edge-token form is a signal Google sign-in uses to refuse
  embedded contexts, so v22 dropped the `Edg/` token. The UA
  contains no Pico/app token — naming the wrapper would be a
  fingerprinting gift and buys nothing. This is compatibility,
  not impersonation: the version is the real runtime's, and no
  other client is emulated anywhere.
- **Browser arguments (v18, AUDIT V18-2):** the WebView2
  additional-arguments string is Pico's own pinned constant
  (`core::media::PICO_BROWSER_ARGS`, media-safety enforced by
  `args_are_media_safe` + a source-scan test), set identically
  on every builder — all WebViews share one user-data folder,
  and environments over one folder must agree on options. Its
  content is exactly the media-safe default wry 0.53.5 used to
  substitute invisibly: three Edge-UI feature disables plus
  the gesture-free autoplay policy. No flag touching GPU,
  media, codecs, EME, QUIC, WebRTC, or service workers may be
  added without breaking the test suite.
- Chrome and start pages run under a strict CSP
  (`default-src 'none'`, inline-only script/style, no connections,
  no forms, no base-uri) and receive dynamic strings exclusively via
  `textContent` (constant-SVG `innerHTML` only, with no interpolated
  data). Snapshot pushes are serialized JSON, never string-built JS
  with user data.

### State file (`src/core/state.rs`)
- Schema-versioned JSON (v3; v1 and v2 files migrate on load),
  written atomically (temp file + rename), size-capped at read
  (1 MiB), and sanitized: theme is a closed set, recents are
  capped/deduped/UNC-and-control-free, and window geometry must
  pass range validation or it is dropped. Corrupt files are left
  untouched for inspection; the app starts on defaults.
  (`v1_state_file_migrates_to_v2`, `v2_state_file_migrates_to_v3_with_shield_defaults`,
  `corrupt_state_recovers_to_defaults`,
  `oversized_state_file_is_corrupt_not_loaded`,
  `sanitize_drops_hostile_recents_and_bad_theme`,
  `window_rect_validation_bounds`,
  `relaunch_preserves_theme_shield_recents_and_window`)

### Logging (`src/log.rs`, `src/core/redact.rs`)
- pico.log never receives a URL with query or fragment contents or
  userinfo: every URL is redacted at the call site (scheme + host +
  path kept; `?…`/`#…` markers show presence). Non-URL strings get
  the same treatment. Lines are control-stripped and length-capped;
  the file rotates at 1 MiB both at startup and mid-session.
  (`tests/redact_tests.rs`)
- **Startup media/profile diagnostics (v18, AUDIT V18-3; v22):**
  the WebView2 runtime version, the effective content user agent
  with its form named (first 80 chars), the profile folder path,
  and the profile witness line. None carries page data.
- **Media probe (v18, AUDIT V18-3):** a passive
  `WebResourceResponseReceived` observer on content WebViews
  logs media-stream responses (googlevideo.com hosts, or a
  `/videoplayback` path) as `media probe: tab=<id> host=<host>
  status=<code> class=<served|server-refusal|transport|
  unexpected>`. It records the host and status ONLY — never the
  URL (signed media URLs carry access tokens in the query),
  never headers, never bodies — and is capped at 20 lines per
  tab per session (`core::media::MediaProbeBudget`), so it
  cannot become a traffic mirror or a log flood. It observes
  responses after the fact; it cannot delay, alter, or block
  any request.

## Branding links (v15; byline moved into the tab row in v16)

The author's links — Telegram `https://t.me/iK4w4` and YouTube
`https://www.youtube.com/@iK4w4` — are the only intentional
external URLs in the binary (the strings audit whitelists exactly
these two). They are opened under strict enum discipline:

- **No URL ever crosses IPC for this feature.** The chrome brand
  block's byline sends a typed `social` command carrying only the enum
  `{telegram | youtube}`; the start-page chips post
  `{action:"social", site:<name>}` through the v17 action dispatch
  (with the `pico://social/<name>` anchor as the no-IPC fallback),
  where `<name>` is the same enum. Rust maps the variant to one of the
  two constant URLs (`SocialSite::url`) and opens a new tab.
  Unknown variants are rejected by serde / the dispatch, so neither
  surface can be turned into an open redirect, and no page content
  can retarget the links (neither page contains the URL strings —
  asserted by `branding_is_present_and_enum_wired`).
- The chips are `<a>` elements pointing at the action URLs exactly
  like the existing pick-file / recents actions: the start tab
  itself never navigates; the click produces a new tab through the
  same allowlist as every other tab creation.

## Shell integrity and failure surfacing (AUDIT V14-1..4)

v14 adds no new attack surface; it hardens the shell's failure
modes, two of which were security-relevant:

- **Chrome navigation allowlist.** The chrome WebView's navigation
  handler now allows exactly the chrome document (`pico://chrome`
  and its trailing-slash form, after the same normalization the
  protocol handler uses) and denies everything else. This replaces
  v13's deny-all handler, which also cancelled the chrome's own
  initial load (the black-shell bug, AUDIT V14-1). The security
  property is unchanged — the chrome can never navigate away to
  attacker content — without breaking the shell itself.
  v15 tightened the shared normalization (AUDIT V15-2): the
  `https://pico.` fold now applies only to Pico's internal hosts,
  so a web page cannot produce the rewritten spelling of an
  internal document, and real `pico.*` websites stay browsable.
  The chrome also denies `window.open` explicitly (AUDIT
  V15-4) instead of relying on runtime defaults.
- **Chrome downloads denied.** The chrome WebView regained the
  download-started handler v12 had and v13 dropped: downloads from
  the shell are denied and counted through the same
  `DownloadBlocked` path as content views. Downloads remain denied
  everywhere (no downloads manager by design).
- **Assembly integrity.** The chrome and start pages are assembled
  by tested core functions (`core::assets`); asset-integrity tests
  assert on the final served bytes (no surviving template tokens,
  script-breakout neutralization intact, required controls
  present), so a future edit cannot silently ship a gutted or
  token-leaking shell.
- **Failure is loud, never silent.** Handshake bookkeeping, a
  recreate-once watchdog, a native escalation dialog, and an
  in-page warning strip ensure a compromised or broken shell is
  surfaced to the user rather than presenting as an unexplained
  black rectangle. The dialog names only the problem and the log
  path; log redaction (queries/fragments never logged) is
  unchanged, and the new handshake log lines contain no URLs.

## Transparent chrome (v16 — documented exception, AUDIT V16-2)

v14's rule was that every surface carries an explicit themed
background color. v16 makes exactly one deliberate exception: the
**chrome** WebView's controller background is transparent (wry
`with_transparent(true)`, i.e. WebView2's `DefaultBackgroundColor`
set to transparent), so the chrome document's bottom 12 DIP
feather strip can blend the header into the content beneath it.
The shell is still safe against the silent-black failure class,
for structural reasons:

- The tao window itself keeps the explicit themed background
  (v14 rule, unchanged), and it is *behind* the chrome: wherever
  the chrome document is transparent — including everywhere, if
  the document fails to load — the user sees the theme color,
  never unpainted black.
- Transparency changes no trust boundary: the chrome document,
  its navigation allowlist, its CSP, its IPC discipline and its
  download denial are the v15 design; the content WebViews keep
  their explicit themed backgrounds.
- The watchdog recreation path builds the chrome through the same
  single `build_chrome`, so a recreated shell is identically
  transparent and identically hardened.
- Nothing may re-background the chrome at runtime: wry's
  `set_background_color` forces alpha to 255, which would
  silently undo the transparency — `toggle_theme` therefore
  updates the window and content views only (AUDIT V16-2).
- Z-order is part of the mechanism: the chrome's container HWND
  is kept above the content HWNDs (`raise_chrome_above_content`,
  AUDIT V16-1), so the feather composites over the page's top
  rows instead of being painted over by them.
- One honest trade-off of the mandated overlap: pointer/wheel
  input over the content's top 12 DIP is absorbed by the chrome
  HWND (sibling HWNDs do not forward input through transparent
  regions). The area is visually blended; it is a strip the
  height of a typical page margin.

## Detector information hygiene (AUDIT V14-5)

The start page shows only listeners that qualify as dev
servers (real page title, known dev-server signature, dev-runtime
process, or curated dev port). Other local listeners — chat-app
RPC endpoints, random TLS services, printer/agent daemons — are
no longer enumerated per-port; only their count is shown ("N other
local listeners (not dev servers)"). This is a precision fix, and
it also stops the start page from advertising a map of every
listening service on the machine to any page that can read it.

## Per-tab failure containment (AUDIT M6)
A content WebView that fails to create marks only its tab failed;
the failure reason rides the state snapshot and Reload retries
creation. Only three failures are fatal (native dialog + pico.log):
no window, no chrome WebView, no server port — states where no
honest UI exists at all.

## Deliberate non-goals (absence is a requirement)
No bookmarks, history UI, downloads manager, Make-Default,
paste-HTML, find bar, accounts, sync, telemetry, extensions,
or settings page. (The Shield is a feature, not a non-goal — see
its section below; an earlier edition of this document listed it
wrongly, AUDIT V22-13.) DevTools exist because the user's spec
requires right-click Inspect.

## Residual risks (accepted, stated plainly)
- **Unregistered-boot staleness (AUDIT V15-1):** dedupe retirement
  covers every chrome generation that ever registered a command.
  A command from a generation destroyed before its first command
  was processed is indistinguishable from the replacement's first
  command; reaching it requires the event loop to stall for the
  watchdog's full 2.5 s during startup, and a second missed
  handshake still escalates to the native dialog.
- **Popup opener severance (AUDIT V22-11):** new-window requests
  are routed to tabs, not real windows, so `window.opener` is
  null in the opened tab. Full-page sign-in flows are unaffected;
  opener-dependent embedded sign-in widgets may not complete.
- **Same-user processes** can read pico.log, state.json, and the
  WebView2 profile directory, and can connect to the file server
  with a valid Host if they learn a root token (tokens are not
  printed anywhere except page URLs in the address bar). Pico does
  not defend against malware already running as the user.
- **TOCTOU on opened folders:** canonical containment is checked at
  request time per request; a symlink swapped *inside* an opened
  project between requests is followed, as with every dev server.
  The user opened that folder deliberately.
- **WebView2 patch level** is the user's Windows responsibility;
  Pico rides the system Evergreen runtime.
- **The protocol handler does not distinguish webviews** (AUDIT L11,
  retained): any webview can *request* `pico://chrome`/`pico://start`
  documents, but content navigation to them is denied, and the
  documents carry no secrets — the start page's data (services,
  recents) is information the user's own pages could equally show.
- **DevTools are enabled** (user requirement); a user who opens
  devtools on the chrome page can script the chrome — the same trust
  boundary as any browser's own UI devtools.
- **cargo audit was not run** — the tool is absent from this
  environment (unchanged since v13; see BUILD_NOTES.md).
  Dependency hygiene was checked by tree inspection instead; no
  advisory verdict is claimed.

## The Shield (v21; counting corrected v22; scriptlet layer v23): request interception + in-page filtering

- The Shield registers a `WebResourceRequested` handler on
  CONTENT WebViews only — never on the chrome WebView, so the
  shell's own traffic is outside its reach by construction.
- What it sees: request URLs and resource types of pages the
  user visits. Decisions are computed locally from lists
  compiled into the executable at build time; nothing about
  browsing is transmitted anywhere, and no list is fetched at
  runtime (updates ship with new builds; provenance and
  hashes are in `assets/filters/FILTERS.md`).
- Blocked requests are answered locally with a neutered 200
  (1x1 GIF / empty body). The origin server is never
  contacted for them, and the page receives no Pico error
  surface for subresources.
- Fail-open is a security property, not a convenience: any
  internal error leaves the request untouched, so a Shield
  defect can degrade blocking but can never break a site or
  wedge a page (the v7 failure mode). Loopback and internal
  origins are exempt unconditionally — local development
  traffic is never filtered, whatever the lists contain.
  Counting follows delivery (AUDIT V22-9): a block is recorded
  only after its neutered response was actually set, so the
  counters never claim a block fail-open did not deliver.
- Engine correctness (AUDIT V22-4..V22-7): anchored rules are
  matched against the authority-parsed host span (userinfo can
  no longer misalign a pattern, IPv6 literals anchor after the
  bracket); `$genericblock` suppresses generic block rules only,
  never generic exceptions; `elemhide` and global `generichide`
  page flags are honored by the cosmetic layer.
- The scriptlet layer (v23, AUDIT V23-1): a document-start
  script runs the compiled `##+js` payload on content pages —
  fetch/XHR response pruning, `JSON.parse` pruning, initial-
  data traps, attribute jobs. The security posture is the
  load-bearing part: list rules cross into the page as **data**
  (a canonical name + raw argument tokens, JSON-encoded) and
  are interpreted by ONE hand-written runtime through a fixed
  dispatch table — list content is never evaluated as code, so
  a hostile or corrupted list line can at worst name a known
  scriptlet with odd arguments on its own scoped hosts, never
  inject arbitrary script. The runtime is a no-op on
  non-http(s) and loopback origins (checked before any wrapper
  is installed, so local development pages keep their exact
  native `fetch`/`XMLHttpRequest`/`JSON.parse` identities),
  every entry and wrapper is individually fail-open, and the
  app's OFF state disarms all wrappers through one in-page
  flag they consult per action (the network layer reads the
  host-side atomic flag per request and needs no in-page
  cooperation).
- Prune reporting (v23): the payload posts per-path prune
  counts over the same `window.ipc` channel as navigation
  notes. Pages can forge those messages, so they are parsed
  under strict caps and used ONLY for capped sample log lines
  — they move no counter, gate nothing, and persist nothing.
- Engine correctness (AUDIT V23-2/3/4): cosmetic and
  scriptlet scopes are parsed with the comma grammar they are
  written in; page flags scope by their `domain=` option or
  their rule's domain anchor, never accidentally global;
  procedural selectors are never shipped as CSS (one invalid
  selector would void a host's whole stylesheet).
- Logging: hosts only (never full URLs), capped at the first
  20 blocks per session, under the same redaction discipline
  as the media probe. Counters are in-memory per tab plus a
  lifetime total in state.json (a count, not a history — no
  URLs or hosts are persisted).
- Residual honesty: party/`domain=` context uses the tab's
  current committed URL (WebView2 does not expose the page
  URL on the event), and the registrable-domain computation
  uses a built-in suffix table rather than the full public
  suffix list. Both approximations fail toward Allow.
