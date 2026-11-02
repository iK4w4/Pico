#!/usr/bin/env bash
# Pico full battery (0.30): the CI-style set, repeatable.
#
# One iteration =
#   1. cargo test --locked --no-default-features
#      (lib unit tests + every integration suite + doc tests,
#      including the full-list Shield regressions)
#   2. examples/dump_pages + the jsdom DOM suite over the
#      dumped pages
#   3. every 10th iteration additionally:
#      cargo xwin clippy (Windows target, all targets) and
#      cargo xwin check --release — the passes that compile
#      the Windows-only GUI layer (src/app.rs).
#
# Usage:
#   scripts/battery.sh [iterations] [logfile]
#   JSDOM_DIR=<dir with node_modules/jsdom> scripts/battery.sh 100 battery-100.log
#
# The loop STOPS at the first failed iteration (a flake is a
# failure: root-cause it, fix it, restart the count). Each
# iteration appends one summary line to the log:
#   BATTERY iteration <n>: PASS|FAIL (<d>s)
# Exit code is 0 only when every requested iteration passed.
set -u
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

ITERS="${1:-1}"
LOG="${2:-battery.log}"
JSDOM_DIR="${JSDOM_DIR:-$HOME/render-tools/picodom}"
PAGES_DIR="target/battery-pages"

mkdir -p "$PAGES_DIR"
: >>"$LOG"

run_pass() {
    local n="$1"
    local t0 dur status
    t0=$(date +%s)
    status=PASS
    {
        echo "===== BATTERY iteration $n ====="
        cargo test --locked --no-default-features 2>&1 || status=FAIL
        if [ "$status" = PASS ]; then
            cargo run --locked --no-default-features --example dump_pages -- "$PAGES_DIR" 2>&1 || status=FAIL
        fi
        if [ "$status" = PASS ]; then
            JSDOM_DIR="$JSDOM_DIR" node scripts/dom_test.mjs "$PAGES_DIR" 2>&1 || status=FAIL
        fi
        if [ "$status" = PASS ] && [ $((n % 10)) -eq 0 ]; then
            echo "--- iteration $n: windows-target clippy + release check ---"
            cargo xwin clippy --locked --target x86_64-pc-windows-msvc --all-targets 2>&1 || status=FAIL
            if [ "$status" = PASS ]; then
                cargo xwin check --locked --release --target x86_64-pc-windows-msvc 2>&1 || status=FAIL
            fi
        fi
    } >>"$LOG" 2>&1
    dur=$(( $(date +%s) - t0 ))
    echo "BATTERY iteration $n: $status (${dur}s)" | tee -a "$LOG"
    [ "$status" = PASS ]
}

n=1
while [ "$n" -le "$ITERS" ]; do
    if ! run_pass "$n"; then
        echo "BATTERY: stopped at iteration $n (FAIL)" | tee -a "$LOG"
        exit 1
    fi
    n=$((n + 1))
done
echo "BATTERY: $ITERS consecutive iteration(s) PASS" | tee -a "$LOG"
exit 0
