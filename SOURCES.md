# Filter list sources

Pico's Shield does not bundle any filter lists. On first run
it downloads the four lists below from their canonical
sources — the same distribution points uBlock Origin and
other ad-blockers fetch from — and caches them as plain text
in `data/lists/` beside the executable. Nothing else is ever
downloaded; the URL set is a closed constant in
`src/core/lists.rs`, and every downloaded byte is validated
before the Shield compiles it (a failed download leaves the
Shield idle and is retried on the next launch, plus once
after 60 s).

| File cached | Canonical source |
|---|---|
| `easylist.txt` | https://easylist.to/easylist/easylist.txt |
| `easyprivacy.txt` | https://easylist.to/easylist/easyprivacy.txt |
| `ublock-filters.txt` | https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters.txt |
| `ublock-privacy.txt` | https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/privacy.txt |

The lists remain the property of their authors and are
governed by their own licenses (EasyList and EasyPrivacy:
GPLv3 / CC BY-SA 3.0; the uBlock Origin lists: GPLv3). Pico
itself — the code in this repository — is MIT-licensed and
contains no list content; the Shield engine is an independent
implementation of the Adblock-Plus filter syntax.

Credits: the [EasyList](https://easylist.to/) project and
[uBlock Origin](https://github.com/gorhill/uBlock) by
Raymond Hill and contributors, whose lists make the Shield
possible.

## Downloader

The download rides on **WinHTTP**, the Windows OS HTTP stack
(via the already-used `windows-sys` crate) — no third-party
HTTP code and no new dependencies were added. HTTPS only,
at most 5 manually-followed redirects (https→https only),
per-file size caps enforced while streaming, no cookies,
no custom headers. TLS and the system proxy configuration
come from Windows itself.
