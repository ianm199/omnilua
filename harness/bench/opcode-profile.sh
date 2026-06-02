#!/usr/bin/env bash
# Count executed VM opcodes for one lua-rs workload.
#
# This is telemetry, not a ledgered benchmark. It builds lua-rs with the
# opt-in `opcode-profile` feature, which instruments the dispatch loop and
# overwrites target/release/lua-rs with the instrumented binary. Rebuild a
# normal release binary before running compare.sh afterward.
#
# Usage:
#   bash harness/bench/opcode-profile.sh fibonacci
#   PROFILE_LUA_EVAL='for i=1,10 do dofile("harness/bench/workloads/binarytrees.lua") end' \
#     bash harness/bench/opcode-profile.sh binarytrees_x10

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOAD="${1:?usage: $0 <workload-name>}"
RS_BIN="$ROOT/target/release/lua-rs"
WORKLOAD_FILE="$ROOT/harness/bench/workloads/${WORKLOAD}.lua"
PROFILE_LUA_EVAL="${PROFILE_LUA_EVAL:-}"

if [ -z "$PROFILE_LUA_EVAL" ]; then
    [ -f "$WORKLOAD_FILE" ] || { echo "[err] workload not found: $WORKLOAD_FILE" >&2; exit 2; }
fi

TS=$(date -u +%Y%m%dT%H%M%SZ)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
OUT_DIR="$ROOT/harness/bench/profiles/opcode-profile/${TS}-${COMMIT}-${WORKLOAD}"
mkdir -p "$OUT_DIR"

echo "==> building instrumented lua-rs (--features opcode-profile)" >&2
CARGO_PROFILE_RELEASE_DEBUG=true \
RUSTFLAGS="-C force-frame-pointers=yes" \
    cargo build --release -p lua-cli --features opcode-profile

export LUA_RS_OPCODE_PROFILE="$OUT_DIR/opcodes.tsv"

if [ -n "$PROFILE_LUA_EVAL" ]; then
    echo "==> running $RS_BIN -e <PROFILE_LUA_EVAL>" >&2
    "$RS_BIN" -e "$PROFILE_LUA_EVAL" >"$OUT_DIR/stdout.txt" 2>"$OUT_DIR/stderr.txt"
else
    echo "==> running $RS_BIN $WORKLOAD_FILE" >&2
    "$RS_BIN" "$WORKLOAD_FILE" >"$OUT_DIR/stdout.txt" 2>"$OUT_DIR/stderr.txt"
fi

echo "==> opcode report: $LUA_RS_OPCODE_PROFILE" >&2
column -t -s $'\t' "$LUA_RS_OPCODE_PROFILE"
