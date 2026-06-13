#!/usr/bin/env bash
# Count executed VM opcodes for one lua-rs workload.
#
# This is telemetry, not a ledgered benchmark. It builds lua-rs with the
# opt-in `opcode-profile` feature, which instruments the dispatch loop and
# overwrites target/release/omnilua with the instrumented binary. Rebuild a
# normal release binary before running compare.sh afterward.
#
# Usage:
#   bash harness/bench/opcode-profile.sh fibonacci
#   PROFILE_REPEAT=10 bash harness/bench/opcode-profile.sh binarytrees
#   PROFILE_LUA_EVAL='for i=1,10 do dofile("harness/bench/workloads/binarytrees.lua") end' \
#     bash harness/bench/opcode-profile.sh binarytrees_x10

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOAD="${1:?usage: $0 <workload-name>}"
RS_BIN="$ROOT/target/release/omnilua"
WORKLOAD_FILE="$ROOT/harness/bench/workloads/${WORKLOAD}.lua"
PROFILE_LUA_EVAL="${PROFILE_LUA_EVAL:-}"
PROFILE_REPEAT="${PROFILE_REPEAT:-1}"

case "$PROFILE_REPEAT" in
    ''|*[!0-9]*) echo "[err] PROFILE_REPEAT must be a positive integer" >&2; exit 2 ;;
    0)           echo "[err] PROFILE_REPEAT must be >= 1" >&2; exit 2 ;;
esac

if [ -z "$PROFILE_LUA_EVAL" ]; then
    [ -f "$WORKLOAD_FILE" ] || { echo "[err] workload not found: $WORKLOAD_FILE" >&2; exit 2; }
fi

TS=$(date -u +%Y%m%dT%H%M%SZ)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
WORKLOAD_LABEL="$WORKLOAD"
if [ -z "$PROFILE_LUA_EVAL" ] && [ "$PROFILE_REPEAT" -gt 1 ]; then
    escaped_workload=${WORKLOAD_FILE//\\/\\\\}
    escaped_workload=${escaped_workload//\"/\\\"}
    PROFILE_LUA_EVAL="for _profile_i = 1, ${PROFILE_REPEAT} do dofile(\"${escaped_workload}\") end"
    WORKLOAD_LABEL="${WORKLOAD}_x${PROFILE_REPEAT}"
fi

OUT_DIR="$ROOT/harness/bench/profiles/opcode-profile/${TS}-${COMMIT}-${WORKLOAD_LABEL}"
mkdir -p "$OUT_DIR"

echo "==> building instrumented lua-rs (--features opcode-profile)" >&2
CARGO_PROFILE_RELEASE_DEBUG=true \
RUSTFLAGS="-C force-frame-pointers=yes" \
    cargo build --release -p omnilua-cli --features opcode-profile

export LUA_RS_OPCODE_PROFILE="$OUT_DIR/opcodes.tsv"

# Watchdog: a hung workload must not wedge the profile run
# (PROFILE_MAX_S, default 600s; perl alarm — macOS has no timeout(1)).
PROFILE_MAX_S="${PROFILE_MAX_S:-600}"
if [ -n "$PROFILE_LUA_EVAL" ]; then
    echo "==> running $RS_BIN -e <PROFILE_LUA_EVAL> ($WORKLOAD_LABEL)" >&2
    "$ROOT/harness/bench/with-timeout.sh" "$PROFILE_MAX_S" \
        "$RS_BIN" -e "$PROFILE_LUA_EVAL" >"$OUT_DIR/stdout.txt" 2>"$OUT_DIR/stderr.txt"
else
    echo "==> running $RS_BIN $WORKLOAD_FILE" >&2
    "$ROOT/harness/bench/with-timeout.sh" "$PROFILE_MAX_S" \
        "$RS_BIN" "$WORKLOAD_FILE" >"$OUT_DIR/stdout.txt" 2>"$OUT_DIR/stderr.txt"
fi

echo "==> opcode report: $LUA_RS_OPCODE_PROFILE" >&2
column -t -s $'\t' "$LUA_RS_OPCODE_PROFILE"
