#!/usr/bin/env bash
# run_official_test.sh [--version 5.4] [--tests-dir DIR] <test_file>
#
# Wraps a single official Lua test file with the small preamble that the PUC-Rio
# tests expect from their launcher, then runs it through the lua-rs CLI.

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
    echo "usage: $0 [--version 5.4|5.5] [--tests-dir DIR] <test_file>" >&2
}

normalize_version() {
    case "$1" in
        51) echo "5.1" ;;
        52) echo "5.2" ;;
        53) echo "5.3" ;;
        54) echo "5.4" ;;
        55) echo "5.5" ;;
        5.1|5.2|5.3|5.4|5.5) echo "$1" ;;
        *) echo "$1" ;;
    esac
}

TEST_TIMEOUT_S=${TEST_TIMEOUT_S:-30}
BIN="${LUA_RS_BIN:-$ROOT/target/debug/omnilua}"
VERSION="${OMNILUA_VERSION:-5.4}"
TESTES_DIR=""

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || { usage; exit 2; }
            VERSION="$2"
            shift 2
            ;;
        --tests-dir|--test-dir)
            [ $# -ge 2 ] || { usage; exit 2; }
            TESTES_DIR="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        -*)
            echo "[err] unknown option: $1" >&2
            usage
            exit 2
            ;;
        *)
            break
            ;;
    esac
done

if [ $# -lt 1 ]; then
    usage
    exit 2
fi

VERSION="$(normalize_version "$VERSION")"
TEST_FILE="$1"

if [ ! -f "$TEST_FILE" ]; then
    echo "[err] test file not found: $TEST_FILE" >&2
    exit 2
fi

if [ ! -x "$BIN" ]; then
    echo "[err] lua-rs binary not built at $BIN" >&2
    exit 2
fi

if [ -z "$TESTES_DIR" ]; then
    TESTES_DIR="$(dirname "$TEST_FILE")"
fi

if [ ! -d "$TESTES_DIR" ]; then
    echo "[err] tests dir not found: $TESTES_DIR" >&2
    exit 2
fi

TESTES_DIR="$(cd "$TESTES_DIR" && pwd)"

OUT_ROOT="${LUA_RS_OFFICIAL_OUT_DIR:-$ROOT/harness/impl/official}"
if [ "$VERSION" = "5.4" ]; then
    OUT_DIR="$OUT_ROOT"
else
    OUT_DIR="$OUT_ROOT/$VERSION"
fi
mkdir -p "$OUT_DIR"

BASE="$(basename "$TEST_FILE" .lua)"
OUT_SUFFIX=""
if [ -n "${LUA_RS_TESTC+x}" ]; then
    OUT_SUFFIX=".testc"
fi
COMBINED="$OUT_DIR/$BASE$OUT_SUFFIX.combined.lua"
OUTFILE="$OUT_DIR/$BASE$OUT_SUFFIX.out"

PREAMBLE_EXPR="_soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}; _G=_G or _ENV; if _VERSION==nil then _VERSION=\"Lua $VERSION\" end"

{
    printf -- '-- harness preamble (passed via -e, NOT prepended; preserves test file line numbers):\n'
    printf -- '-- %s\n\n' "$PREAMBLE_EXPR"
    sed '${/^$/d;}' "$TEST_FILE"
} > "$COMBINED"

export LUA_PATH="$TESTES_DIR/?.lua;$TESTES_DIR/?/init.lua;./?.lua;./?/init.lua"
export OMNILUA_VERSION="$VERSION"

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
elif grep -qE "syntax error|<name> expected" "$OUTFILE"; then
    cause="parse-or-compile-error"
elif grep -qE "assertion failed|attempt to " "$OUTFILE"; then
    cause="runtime-error"
else
    cause="unknown"
fi

printf "FAIL  %s  (cause=%s, reached_exec=%s)\n" "$TEST_FILE" "$cause" "$reached_exec"
echo "--- first 5 lines of failure context ---"
{
    grep -nE "not yet implemented|pcall_k failed|^thread '[^']+' .* panicked at |^\[err\]|panicked at |assertion failed|syntax error" "$OUTFILE" \
        || tail -n 20 "$OUTFILE"
} | head -n 5
echo "--- (full output at $OUTFILE) ---"
exit 1
