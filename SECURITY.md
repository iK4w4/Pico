# Pico — Security Model

This document describes the **public build** of Pico 0.31 — the
one built from this source tree. The `Pico.exe` attached to this
repository's releases is built from this tree.

The author also keeps a private variant that bundles its filter
lists instead of downloading them. That variant is **not**
distributed; nothing in this document describes it. Earlier
versions of this file mixed the two and carried sections dating
back to v22 that no longer matched the code; this rewrite
replaces all of that. It describes only what the public build
does today.

Pico is a young, one-person project for **local development**.
For banking, email and anything else sensitive, use a
mainstream browser (Edge, Firefox, Chrome).

## Verifying the release download

The `Pico.exe` attached to the 0.31.1 release is 1,327,616
bytes with SHA-256
`763e72391a87d13376c00ed8a2bbfea688e788b520a6f2a64bd7c3423b4a87e4`.
If your copy's hash differs, it is not the file published
from this tree.

## Network behavior

- **No telemetry, no accounts, no sync.** Pico sends nothing
  about you or your browsing anywhere.
- The only traffic Pico itself originates is the **one-time
  filter-list download** (below), plus a **Google search** when
  you type non-URL text into the address bar. Everything else
  is the page traffic you asked for, carried by WebView2.
- The WebView2 rendering engine is Microsoft's: its security
  patches arrive through Edge / Windows Update, not from this
  project. Pico itself has no auto-update mechanism.

## The built-in local server

Local HTML files and folders are served by Pico's own static
server so they can be previewed with live reload. It is built
to be reachable by nothing but this machine:

- Binds to **127.0.0.1 only**, on a random port. Pico never
  opens a listener on the LAN.
- A **Host-header wall** rejects requests whose Host is not the
  loopback address, which blocks DNS-rebinding attacks from
  websites open in other browsers.
- Every served folder gets a **random per-root token** in its
  URLs; paths are decoded once and normalized, with traversal,
  NUL bytes, alternate data streams and Windows device names
  rejected, then checked for canonical containment. UNC paths
  are refused before any filesystem access.
- Connection counts and timeouts are capped, and a traversal
  test corpus in the test suite covers the rejection rules.
- Live reload runs over server-sent events scoped to each
  served root, which is cleaned up when its last tab closes.

## Navigation and the shell

- The address bar accepts **http, https and Pico's internal
  pages** — nothing else.
- Privileged start-page actions (opening files, revealing
  paths) are accepted only from the start tab while it is
  actually showing the internal start page. A web page cannot
  trigger them.
- **Downloads are denied** in every webview. There is no
  downloads manager; Pico cannot save files.
- Tabs are capped at 30.
- Popup windows are opened with `window.opener` severed. The
  known cost: popup-based sign-in flows on some sites break.
- Right-click **Inspect** (DevTools) is available, aimed at
  the local-development use case.

## The Shield (ad/tracker blocking)

- Three layers — network, cosmetic and scriptlet filtering —
  with Adblock-Plus rule semantics.
- The filter lists (EasyList, EasyPrivacy, uBlock Origin) are
  **downloaded at runtime, never bundled** — see
  [SOURCES.md](SOURCES.md) for the exact sources and licensing.
  The downloader's URL set is a closed constant: it fetches
  those four lists and nothing else, ever.
- The download is HTTPS-only, follows at most five
  https-to-https redirects, sends no cookies and no custom
  headers, and enforces per-file size caps while streaming.
  Downloaded bytes are untrusted input: each file is validated
  (size, encoding, list shape) and the whole set must compile
  into a sane engine before anything is cached or installed.
  The cache in `data\lists\` is written atomically, and Pico
  retries a failed download on the next launch and once after
  60 seconds.
- Until the lists land, the Shield is **idle** and Pico
  browses normally. If the filter engine ever hits an error,
  it **fails open**: filtering stops, browsing continues.
- Loopback and internal pages are never blocked or injected.
  Main-frame documents are never blocked except by explicit
  `$document` rules. Turning the Shield off is a single atomic
  flag checked by every layer.

## Lite mode

Lite mode blocks heavy resources on public pages. Exactly one
video per page — the one you chose by clicking it, or the one
a video page presents as its main video — plays normally;
hover previews and all other unsolicited playback are refused.
Images and fonts are never exempted for the chosen video. The
Shield is senior: Lite never re-allows anything the Shield
blocked.

## Your data

- Everything Pico keeps lives in a **`data\` folder beside the
  exe**: settings/state, `pico.log`, the filter lists and the
  WebView2 browser profile. If that folder is not writable,
  Pico falls back to `%APPDATA%\Pico` and logs that it did.
- **The profile is not encrypted by Pico.** Whoever can copy
  the `data\` folder can copy the logins inside it — treat the
  folder as private, exactly as the README warns. Deleting
  the folder is a factory reset.
- Password autosave and general autofill are switched off via
  the WebView2 profile API.
- `pico.log` strips query strings and credentials from URLs
  before writing them, and rotates. State is sanitized when
  loaded.

## User agent

Pico presents the Chromium user-agent string of the **real**
WebView2 runtime version, with the Edge product token removed.
Several sign-in flows — notably Google's — reject or degrade
the Edge token; this is a compatibility choice, disclosed here
rather than hidden.

## Phone Teleport

Teleport shows a QR code holding the current page's URL, with
loopback addresses rewritten to this machine's LAN address so
a phone on the same network can open the page. Pico opens no
LAN listener for this: your dev server becomes reachable from
the phone only if that server itself already listens on the
LAN (for example Vite with `--host`). Use Teleport on networks
you trust.

## Dependency audit

Checked **2026-11-03**, and repeated for each release: the
full `Cargo.lock` (377 packages across all platforms) was
checked against the **RustSec advisory database**, and the
dependency graph of the shipped target
(`x86_64-pc-windows-msvc`) was analyzed separately, because a
lockfile also carries other platforms' stacks (Linux GTK,
D-Bus) that never enter `Pico.exe`.

Result: 10 advisory records covering 7 packages. **None is a
vulnerability in the shipped Windows binary:**

- `glib` (unsoundness in `VariantStrIter`) and `rand` 0.7.3
  ("unsound with a custom logger") are real advisories for
  packages that **do not occur in the Windows graph at all** —
  glib belongs to the Linux GTK stack; the only `rand` in any
  build graph here is 0.9.5, which is unaffected.
- `zvariant`, `zbus_names`, `zvariant_derive` and `fxhash`
  carry **"unmaintained" notices** (not vulnerabilities) and
  are likewise absent from the Windows graph;
  `proc-macro-error` is an unmaintained notice for a
  build-time macro crate.
- The one flagged package that *is* in the Windows graph is
  **`instant` 0.1.13** (pulled in by `notify`, the file
  watcher): an "unmaintained" notice, not a vulnerability.
  It is accepted, recorded here, and tracked upstream through
  `notify`.

## Reporting a problem

Ordinary bugs: open a GitHub issue. For anything sensitive —
a way out of the local server's loopback wall, a Shield
bypass that exposes users, a log leak — please contact the
author privately first via Telegram
([@iK4w4](https://t.me/iK4w4)) so it can be fixed before it is
described in public.
