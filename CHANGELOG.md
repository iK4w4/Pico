# Changelog

All notable changes to Pico are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

Older releases (0.31–0.40) predate this file; their history is in
SECURITY.md and the version notes in `src/core/mod.rs`.

## [0.41.3] - 2026-11-16

- Black-frame fix: tab switch/open/close no longer presents an
  empty frame
- Deepest audit pass yet: 30+ proven fixes across migration,
  history, downloads capture, tab-move, Shield and redaction
- New app icon (user artwork)

## [0.41.0] - 2026-11-16

- All data moved out of the exe folder into `%LOCALAPPDATA%\Pico`,
  with automatic migration of the old layout; downloads land in
  the normal Downloads folder
- Bookmarks (star in the address bar) and a history page
- Tabs can be dragged to reorder and moved to a new window
- The exe works from anywhere, alone

## [0.40.2] - 2026-11-11

- External-review fixes, including a mixed-separator UNC /
  device-path bypass and duplicate history entries
