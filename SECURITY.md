# Pico — Security Model

This document describes the **public build** of Pico 0.40.1 —
the one built from this source tree. The `Pico.exe` attached
to this repository's releases is built from this tree.

The author also keeps a private variant that bundles its
filter lists instead of downloading them. That variant is
**not** distributed; nothing in this document describes it.
This document describes only what the public build does
today, including the download manager added in 0.32 and the
audit pass that shipped in 0.40.1.

Pico is a young, one-person project for **local
development**. For banking, email and anything else
sensitive, use a mainstream browser (Edge, Firefox,
Chrome).

## Verifying the release download

The `Pico.exe` attached to the 0.40.1 release is TODO
bytes with SHA-256 TODO. (Filled in when the release is
published.) If your copy's hash differs, it is not the file
published from this tree.

## Network behavior

- **No telemetry, no accounts, no sync.** Pico sends
  nothing about you or your browsing anywhere.
- The traffic Pico itself originates is: the **one-time
  filter-list download** (below); **downloads you start**
  (below — fetched by Pico's own HTTP client, with the
  cookies the page's site set, scoped as described); and a
  **Google search** when you type non-URL text into the
  address bar. Everything else is the page traffic you
  asked for, carried by WebView2.
- The WebView2 rendering engine is Microsoft's: its
  security patches arrive through Edge / Windows Update,
  not from this project. Pico itself has no auto-update
  mechanism.

## Downloads

Since 0.32 Pico runs its own download engine instead of
the browser's. This is the largest attack surface in the
program, so the routing is deliberately narrow:

- **Capture.** WebView2's own download path is disabled.
  A download starts only when the finished response is
  recognizably a file (its disposition or content type),
  for a click you made or a form you submitted — the same
  evidence a browser uses. Responses to background
  requests never become downloads.
- **Two surfaces, one command table.** Downloads are
  driven from the toolbar popover and the downloads page
  through a single command table that says which surface
  may issue which command — and the table is enforced,
  not just documented: the page cannot reach
  toolbar-only commands, the toolbar cannot reach
  page-only ones (like deleting a downloaded file or
  clearing the history), and both are checked against
  the task's current state before anything acts. The
  toolbar channel additionally drops any command body
  that arrives without its delivery envelope, so a
  replayed or misdirected body is inert.
- **Internal pages can't be opened by web content.**
  The start page and the downloads page are the only
  internal pages. A website cannot navigate or pop up a
  tab onto either of them, and their commands are
  accepted only from the tab that is actually showing
  them.
- **Transport.** Downloads run over WinHTTP with system
  TLS validation — no certificate checks are disabled
  anywhere in Pico, and there is no flag in the code
  that could disable them. Redirects are followed
  manually, hop by hop, with every header recomputed
  for the new URL: an https download is never silently
  downgraded to http (only the explicit
  refresh-link flow may cross schemes, and it says so),
  cookies are recomputed per hop so a site's cookies
  are never sent to a different site, and the Referer
  is sent in full only to its own origin — a token in
  a page's address does not leak to third-party CDNs.
  A server that streams more bytes than its promised
  range is cut off.
- **Cookies.** A download carries the cookies the
  *page's site* set, matched by host relatedness
  (registrable domain), recomputed for each redirect
  hop. Cookie values are never logged, never shown,
  and never sent to an unrelated host. A finished
  download's cookies are wiped from its on-disk
  record; unfinished downloads keep theirs because
  resume needs them — the same trust boundary as the
  browser profile itself.
- **File names.** The name a server suggests is
  treated as hostile: path separators, control
  characters and invisible Unicode format characters
  (including the right-to-left override that can
  disguise a file's extension) are stripped, Windows
  device names and reserved names are defused,
  trailing dots and spaces are removed, and the file
  always lands directly inside the downloads folder —
  never in a subdirectory the server picked. If the
  name is taken, Pico numbers its own copy (`name
  (2).ext`) and never overwrites a stranger's file.
- **Integrity.** Ranged downloads reassemble from a
  segment map that is only trusted after the server's
  validators (ETag / Last-Modified) and the partial
  file's true length check out; any doubt restarts
  the download rather than splicing suspect bytes.
  Finished files get a SHA-256 shown on the downloads
  page. A server lying about a file's size cannot
  fill your disk: free space is proven before the
  download starts and again at segment boundaries.
- **Refresh link.** When a signed link expires, Refresh
  link revisits the page the download came from at
  Pico's own request (cookies scoped as above), takes
  the single unambiguous fresh link, and probes it
  before swapping it in; the probe result is
  revalidated against the task's state before it
  lands, and an ambiguous or missing fresh link
  changes nothing.

## The built-in local server

Local HTML files and folders are served by Pico's own
static server so they can be previewed with live
reload. It is built to be reachable by nothing but this
machine:

- Binds to **127.0.0.1 only**, on a random port. Pico
  never opens a listener on the LAN.
- A **Host-header wall** rejects requests whose Host is
  not the loopback address, which blocks DNS-rebinding
  attacks from websites open in other browsers.
- Every served folder gets a **random per-root token**
  in its URLs; paths are decoded once and normalized,
  with traversal, NUL bytes, alternate data streams
  and Windows device names rejected, then checked for
  canonical containment. UNC paths are refused before
  any filesystem access.
- Connection counts and timeouts are capped, and a
  traversal test corpus in the test suite covers the
  rejection rules.
- Live reload runs over server-sent events scoped to
  each served root, which is cleaned up when its last
  tab closes.

## Navigation and the shell

- The address bar accepts **http, https and Pico's
  internal pages** — nothing else.
- Privileged start-page actions (opening files,
  revealing paths) are accepted only from the start
  tab while it is actually showing the internal start
  page. A web page cannot trigger them.
- WebView2's built-in context menu and download UI
  are disabled. Pico's own right-click menu replaces
  the context menu; a menu selection is honored only
  if it names an item that menu actually offered, in
  the state it offered it.
- Tabs are capped at 30.
- Popup windows are opened with `window.opener`
  severed. The known cost: popup-based sign-in flows
  on some sites break.
- Right-click **Inspect** (DevTools) is available,
  aimed at the local-development use case.

## The Shield (ad/tracker blocking)

- Three layers — network, cosmetic and scriptlet
  filtering — with Adblock-Plus rule semantics.
- The filter lists (EasyList, EasyPrivacy, uBlock
  Origin) are **downloaded at runtime, never
  bundled** — see [SOURCES.md](SOURCES.md) for the
  exact sources and licensing. The downloader's URL
  set is a closed constant: it fetches those four
  lists and nothing else, ever.
- The download is HTTPS-only, follows at most five
  https-to-https redirects, sends no cookies and no
  custom headers, and enforces per-file size caps
  while streaming. Downloaded bytes are untrusted
  input: each file is validated (size, encoding, list
  shape) and the whole set must compile into a sane
  engine before anything is cached or installed. The
  cache in `data\lists\` is written atomically, and
  Pico retries a failed download on the next launch
  and once after 60 seconds.
- Until the lists land, the Shield is **idle** and
  Pico browses normally. If the filter engine ever
  hits an error, it **fails open**: filtering stops,
  browsing continues.
- Loopback and internal pages are never blocked or
  injected. Main-frame documents are never blocked
  except by explicit `$document` rules. Turning the
  Shield off is a single atomic flag checked by every
  layer.

## Lite mode

Lite mode blocks heavy resources on public pages.
Exactly one video per page — the one you chose by
clicking it, or the one a video page presents as its
main video — plays normally; hover previews and all
other unsolicited playback are refused. Images and
fonts are never exempted for the chosen video. The
Shield is senior: Lite never re-allows anything the
Shield blocked.

## Your data

- Everything Pico keeps lives in a **`data\` folder
  beside the exe**: settings/state, `pico.log`, the
  filter lists, download task records and the WebView2
  browser profile. If that folder is not writable,
  Pico falls back to `%APPDATA%\Pico` and logs that it
  did.
- **Pico itself stores no passwords.** Password
  autosave and general autofill are switched off via
  the WebView2 profile API. Page logins live in the
  WebView2 profile, which **is not encrypted by
  Pico**: whoever can copy the `data\` folder can copy
  the logins inside it — treat the folder as private.
  Deleting the folder is a factory reset.
- Download task records store the file's name, URL,
  size and state — and, for unfinished tasks only,
  the site's cookies (see Downloads). The URL of a
  finished download is kept as the history record;
  its cookies are not.
- `pico.log` strips query strings, fragments and
  credentials from URLs before writing them, never
  contains cookie values, and rotates. Downloads
  engine log lines name hosts only. State files are
  sanitized when loaded, and a corrupt state or
  download record degrades to a clean start instead
  of breaking startup.
- **Known limitation:** Pico has no single-instance
  lock; two copies can run at once. State saves are
  atomic and named per process, so the worst case is
  the last copy to exit wins the settings file.
  Downloads in progress are owned by the copy that
  started them.

## User agent

Web pages see the Chromium user-agent string of the
**real** WebView2 runtime version, with the Edge
product token removed. Several sign-in flows — notably
Google's — reject or degrade the Edge token; this is
a compatibility choice, disclosed here rather than
hidden. Pico's own requests (the filter-list download
and the download engine) identify as `Pico/0.40.1`.

## Phone Teleport

Teleport shows a QR code holding the current page's
URL, with loopback addresses rewritten to this
machine's LAN address so a phone on the same network
can open the page. Pico opens no LAN listener for
this: your dev server becomes reachable from the
phone only if that server itself already listens on
the LAN (for example Vite with `--host`). Use
Teleport on networks you trust.

## Dependency audit

Checked **2026-11-11** for this release, on top of a
six-front audit of Pico's own code (IPC gates, the
download engine, the WinHTTP transport, the local
server, the Shield, the shell and the parsers — the
last also fuzz-probed with 574 malformed inputs, zero
panics). The shipped dependency graph
(`x86_64-pc-windows-msvc`) is **104 crates**; every one
was checked against the **RustSec advisory database**.

Result: **no known vulnerabilities** in the shipped
graph. One "unmaintained" notice — `instant` 0.1.13,
a transitive dependency of `notify` (the file
watcher) — is accepted and tracked upstream; it is
not a vulnerability. The browser engine (WebView2)
is Microsoft's and updates through Edge / Windows
Update.

## Reporting a problem

Ordinary bugs: open a GitHub issue. For anything
sensitive — a way out of the local server's loopback
wall, a Shield bypass that exposes users, a download
that escapes its folder, a log leak — please contact
the author privately first: Telegram
[@AskKawaBot](https://t.me/AskKawaBot).
