#!/usr/bin/env bash
# run_one_test.sh <test_file>
#
# Quiet single-test runner extracted from run_official_test.sh so that
# harness/stop-hook.sh can probe the smoke set without dumping per-test
# context to the agent transcript.
#
# Prints exactly one of: PASS | FAIL | TIMEOUT
# Exit code is always 0 (status carried in stdout) so callers can read it
# under `set -e`.
#
# Honours TEST_TIMEOUT_S (default 20). Requires target/debug/omnilua.

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TEST_TIMEOUT_S=${TEST_TIMEOUT_S:-20}
BIN="$ROOT/target/debug/omnilua"
OUT_DIR="harness/impl/official"
mkdir -p "$OUT_DIR"

if [ $# -lt 1 ]; then
    echo "FAIL"
    exit 0
fi

TEST_FILE="$1"
if [ ! -f "$TEST_FILE" ] || [ ! -x "$BIN" ]; then
    echo "FAIL"
    exit 0
fi

BASE="$(basename "$TEST_FILE" .lua)"
COMBINED="$OUT_DIR/$BASE.stop.combined.lua"
OUTFILE="$OUT_DIR/$BASE.stop.out"

PREAMBLE='-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end
'

{ printf '%s\n' "$PREAMBLE"; cat "$TEST_FILE"; } > "$COMBINED"

TESTES_DIR="$(cd "$(dirname "$TEST_FILE")" && pwd)"
export LUA_PATH="$TESTES_DIR/?.lua;$TESTES_DIR/?/init.lua;./?.lua;./?/init.lua"
export LUA_RS_VERBOSE=1

rc=0
if command -v gtimeout >/dev/null 2>&1; then
    gtimeout --signal=KILL "$TEST_TIMEOUT_S" "$BIN" "$COMBINED" > "$OUTFILE" 2>&1
    rc=$?
elif command -v timeout >/dev/null 2>&1; then
    timeout --signal=KILL "$TEST_TIMEOUT_S" "$BIN" "$COMBINED" > "$OUTFILE" 2>&1
    rc=$?
else
    ( "$BIN" "$COMBINED" > "$OUTFILE" 2>&1 ) &
    pid=$!
    ( sleep "$TEST_TIMEOUT_S" && kill -9 "$pid" 2>/dev/null ) &>/dev/null &
    watcher=$!
    wait "$pid" 2>/dev/null
    rc=$?
    kill "$watcher" &>/dev/null
    wait "$watcher" 2>/dev/null || true
fi

if [ "$rc" = "137" ] || [ "$rc" = "124" ]; then
    echo "TIMEOUT"
    exit 0
fi

if [ "$rc" = "0" ] \
    && ! grep -qE "not yet implemented|^thread '[^']+' .* panicked at |^\[err\]|pcall_k failed" "$OUTFILE"; then
    echo "PASS"
else
    echo "FAIL"
fi
exit 0
