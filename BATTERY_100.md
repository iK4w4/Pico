# Pico 0.30.1 — battery addendum (2026-11-01)

The repair round ran under the user's reduced
requirement: 10 consecutive full passes of the same
battery defined below (`scripts/battery.sh` — cargo test
all targets + dump_pages/DOM + full-list Shield
regressions per iteration, Windows-target clippy +
release check on the 10th).

Result: **10 consecutive iterations PASS, 0 failures,
0 restarts** (25–34 s per iteration; iteration 10 carried
the Windows clippy + release check and passed it). Log:
`battery-0.30.1.log` in this tree. Suite sizes grew over
0.30's runs: 362 Rust tests and 238 DOM checks per
iteration (0.30: 353 + 236) — the growth is the repair
round's seam tests.

One additional proof outside the battery: the new XHR
reuse DOM check (2d) was run once against the runtime
with its fix deliberately disabled — it FAILED, serving
the first response's text — and passes with the fix
restored. A test that cannot fail is not a test.

---

# Pico 0.30 — 100-run battery (2026-11-01)

The order: run the FULL battery 100 consecutive times;
any failure or flake stops the run, gets root-caused and
fixed, and restarts the count from zero.

Requirement change, recorded honestly: mid-run, the
user reduced the requirement from 100 consecutive
passes to 10. By the time the stop order arrived, run 2
(below) had already completed the full 100, so the
reduction curtailed nothing — the delivered evidence
is the complete 100-pass run, which also exceeds the
reduced requirement many times over. (The filename
keeps the original order's name.)

## What one iteration is

`scripts/battery.sh` (shipped in this tree):

1. `cargo test --locked --no-default-features` — the
   whole Rust suite (353 tests: lib unit, every
   integration suite incl. the full-list Shield
   regressions, doc tests).
2. `examples/dump_pages` + the jsdom DOM suite
   (236 checks over the dumped chrome/start/shield/
   lite/lab fixtures).
3. Every 10th iteration additionally: `cargo xwin
   clippy --target x86_64-pc-windows-msvc --all-targets`
   and `cargo xwin check --release` — the passes that
   compile the Windows-only GUI layer.

## Result

- **Run 2 (the shipped tree): 100 of 100 iterations
  PASS, consecutive, zero failures, zero flakes.**
  Total wall time 2,614 s (avg 26.1 s per iteration,
  min 22 s, max 32 s; the decile iterations' Windows
  passes are incremental on a warm target). The final
  line of `battery-100.log` reads:
  `BATTERY: 100 consecutive iteration(s) PASS`.
- **Run 1: 45 of 45 PASS, then stopped deliberately —
  not by a failure.** A1's full audit report landed
  during the run with two additional confirmed LOW
  findings (A1-2: report pre-filters could swallow a
  nav note whose title quoted a marker word; A1-3:
  0/0 prune-report gate hygiene). Per the battery's
  own law the tree was no longer final, so the count
  was stopped, both findings were fixed and pinned
  (see AUDIT 0.30), and the count restarted from zero
  as run 2. Run 1's log is preserved as
  `battery-run1-45.log`.

## Flakes

None — in either run. No iteration failed, timed out,
or needed a retry. The battery's stop-on-first-failure
rule was exercised only by the deliberate run-1 stop
above.
