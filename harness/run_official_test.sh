#!/usr/bin/env bash
# run_official_test.sh <test_file>
#
# Wraps a single Lua 5.4 official test file (reference/lua-c/testes/*.lua)
# with a small preamble that defines the globals those tests assume the
# launcher set up (e.g. _soft, _port, _nomsg, arg) and runs the combined
# file through the lua-rs CLI. Captures stdout+stderr, prints a one-line
# PASS/FAIL summary, and (on failure) the first 5 lines of failure
# context.
#
# Usage:
#   ./harness/run_official_test.sh reference/lua-c/testes/vararg.lua

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TEST_TIMEOUT_S=${TEST_TIMEOUT_S:-30}
BIN="$ROOT/target/debug/lua-rs"
OUT_DIR="harness/impl/official"
mkdir -p "$OUT_DIR"

if [ $# -lt 1 ]; then
    echo "usage: $0 <test_file>" >&2
    exit 2
fi

TEST_FILE="$1"
if [ ! -f "$TEST_FILE" ]; then
    echo "[err] test file not found: $TEST_FILE" >&2
    exit 2
fi

if [ ! -x "$BIN" ]; then
    echo "[err] lua-rs binary not built at $BIN" >&2
    exit 2
fi

BASE="$(basename "$TEST_FILE" .lua)"
OUT_SUFFIX=""
if [ -n "${LUA_RS_TESTC+x}" ]; then
    OUT_SUFFIX=".testc"
fi
COMBINED="$OUT_DIR/$BASE$OUT_SUFFIX.combined.lua"
OUTFILE="$OUT_DIR/$BASE$OUT_SUFFIX.out"

PREAMBLE_EXPR='_soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}; _G=_G or _ENV; if _VERSION==nil then _VERSION="Lua 5.4" end'

{
    printf -- '-- harness preamble (passed via -e, NOT prepended; preserves test file line numbers):\n'
    printf -- '-- %s\n\n' "$PREAMBLE_EXPR"
    sed '${/^$/d;}' "$TEST_FILE"
} > "$COMBINED"

TESTES_DIR="$(cd "$(dirname "$TEST_FILE")" && pwd)"
export LUA_PATH="$TESTES_DIR/?.lua;$TESTES_DIR/?/init.lua;./?.lua;./?/init.lua"

run_with_timeout() {
    local src_file="$1"
    local out_file="$2"
    if command -v gtimeout >/dev/null 2>&1; then
        gtimeout --signal=KILL "$TEST_TIMEOUT_S" "$BIN" -e "$PREAMBLE_EXPR" "$src_file" > "$out_file" 2>&1
    elif command -v timeout >/dev/null 2>&1; then
        timeout --signal=KILL "$TEST_TIMEOUT_S" "$BIN" -e "$PREAMBLE_EXPR" "$src_file" > "$out_file" 2>&1
    else
        ( "$BIN" -e "$PREAMBLE_EXPR" "$src_file" > "$out_file" 2>&1 ) &
        local pid=$!
        ( sleep "$TEST_TIMEOUT_S" && kill -9 "$pid" 2>/dev/null ) &>/dev/null &
        local watcher=$!
        wait "$pid" 2>/dev/null
        local rc=$?
        kill "$watcher" &>/dev/null
        wait "$watcher" 2>/dev/null || true
        return $rc
    fi
}

rc=0
run_with_timeout "$TEST_FILE" "$OUTFILE" || rc=$?

reached_exec=0
grep -qE "^\[4/4\] Executing chunk" "$OUTFILE" && reached_exec=1

if [ "$rc" = "0" ] \
    && ! grep -qE "not yet implemented|^thread '[^']+' .* panicked at |^\[err\]|pcall_k failed" "$OUTFILE"; then
    printf "PASS  %s\n" "$TEST_FILE"
    exit 0
fi

if grep -qE "not yet implemented" "$OUTFILE"; then
    cause="stub"
elif grep -qE "panicked at .*parser" "$OUTFILE"; then
    cause="parse-panic"
elif grep -qE "panicked at .*codegen|panicked at .*compil" "$OUTFILE"; then
    cause="codegen-panic"
elif grep -qE "^thread '[^']+' .* panicked at " "$OUTFILE"; then
    cause="runtime-panic"
elif grep -qE "^\[err\]" "$OUTFILE"; then
    if [ "$reached_exec" = "1" ]; then
        cause="runtime-error"
    else
        cause="parse-or-compile-error"
    fi
elif grep -qE "pcall_k failed" "$OUTFILE"; then
    cause="runtime-error"
else
    cause="unknown"
fi

printf "FAIL  %s  (cause=%s, reached_exec=%s)\n" "$TEST_FILE" "$cause" "$reached_exec"
echo "--- first 5 lines of failure context ---"
{
    grep -nE "not yet implemented|pcall_k failed|^thread '[^']+' .* panicked at |^\[err\]|panicked at " "$OUTFILE" \
        || tail -n 20 "$OUTFILE"
} | head -n 5
echo "--- (full output at $OUTFILE) ---"
exit 1
