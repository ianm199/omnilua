#!/usr/bin/env bash
# build-pgo.sh — profile-guided-optimization build of lua-rs (PERF_PUSH_SPEC P4.1).
#
# Pipeline: instrumented build (-Cprofile-generate) -> training run (every
# bench workload once + a slice of the official suite, so the profile sees
# both microbench dispatch mixes and real-program shapes) -> llvm-profdata
# merge -> optimized rebuild (-Cprofile-use).
#
# The training command list is fixed in this script on purpose: PGO layout is
# only reproducible if the training set is pinned (PERF_PUSH_SPEC risk note).
#
# Usage:
#   bash harness/bench/build-pgo.sh            # leaves target/release/omnilua PGO'd
#   PGO_DIR=/tmp/lua-pgo-alt bash harness/bench/build-pgo.sh
#
# The caller is responsible for saving a stock binary first and A/B-ing via
# compare_bins.sh. Ledgered numbers from a PGO binary must be labeled.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PGO_DIR="${PGO_DIR:-/tmp/lua-pgo}"
WORKLOAD_DIR="$ROOT/harness/bench/workloads"
TRAIN_MAX_S=120

LLVM_PROFDATA="$(ls "$HOME"/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-profdata 2>/dev/null | head -1)"
[ -n "$LLVM_PROFDATA" ] || { echo "[err] llvm-profdata not found; rustup component add llvm-tools" >&2; exit 2; }

rm -rf "$PGO_DIR"
mkdir -p "$PGO_DIR"

echo "=== [1/4] instrumented build ===" >&2
RUSTFLAGS="-Cprofile-generate=$PGO_DIR" cargo build --release -p omnilua-cli -q
BIN="$ROOT/target/release/omnilua"

echo "=== [2/4] training run ===" >&2
export LLVM_PROFILE_FILE="$PGO_DIR/%p-%m.profraw"
for wpath in "$WORKLOAD_DIR"/*.lua; do
    wname=$(basename "$wpath" .lua)
    if grep -q "^${wname}	.*LIVELOCK" "$WORKLOAD_DIR/manifest.tsv" 2>/dev/null; then
        echo "    skip $wname (LIVELOCK)" >&2
        continue
    fi
    echo "    train: $wname" >&2
    "$ROOT/harness/bench/with-timeout.sh" "$TRAIN_MAX_S" \
        "$BIN" "$wpath" >/dev/null 2>&1 || echo "    [warn] $wname training run failed" >&2
done
for t in calls.lua nextvar.lua strings.lua closure.lua; do
    tp="$ROOT/reference/lua-c/testes/$t"
    [ -f "$tp" ] || continue
    echo "    train: official $t" >&2
    (cd "$ROOT/reference/lua-c/testes" && \
        "$ROOT/harness/bench/with-timeout.sh" "$TRAIN_MAX_S" \
        "$BIN" -e "_soft=true; _port=true" "$t" >/dev/null 2>&1) \
        || echo "    [warn] official $t training run failed" >&2
done

echo "=== [3/4] merge profiles ===" >&2
"$LLVM_PROFDATA" merge -o "$PGO_DIR/merged.profdata" "$PGO_DIR"/*.profraw
echo "    $(ls "$PGO_DIR"/*.profraw | wc -l | tr -d ' ') profraw files merged" >&2

echo "=== [4/4] optimized rebuild ===" >&2
RUSTFLAGS="-Cprofile-use=$PGO_DIR/merged.profdata" cargo build --release -p omnilua-cli -q

echo "PGO build complete: $BIN" >&2
echo "Rebuild a stock binary afterwards with: cargo build --release -p omnilua-cli (after touching a source file or clearing RUSTFLAGS)" >&2
