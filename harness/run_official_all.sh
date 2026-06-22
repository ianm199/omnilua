#!/usr/bin/env bash
# run_official_all.sh [--version 5.4] [--tests-dir DIR]
#
# Runs a versioned official Lua test tree through the lua-rs binary and reports
# per-test PASS / FAIL / TIMEOUT plus a final summary line.

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
    echo "usage: $0 [--version 5.1|5.2|5.3|5.4|5.5] [--tests-dir DIR]" >&2
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

default_tests_dir() {
    case "$1" in
        5.5) echo "$ROOT/reference/lua-5.5.0-tests" ;;
        5.4) echo "$ROOT/reference/lua-c/testes" ;;
        5.3) echo "$ROOT/reference/lua-5.3.6-tests" ;;
        5.2) echo "$ROOT/reference/extra-tests/lua-5.2.2-tests" ;;
        5.1) echo "$ROOT/reference/extra-tests/lua5.1-tests" ;;
        *) echo "$ROOT/reference/lua-c/testes" ;;
    esac
}

TEST_TIMEOUT_S=${TEST_TIMEOUT_S:-60}
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
            echo "[err] unexpected argument: $1" >&2
            usage
            exit 2
            ;;
    esac
done

VERSION="$(normalize_version "$VERSION")"
if [ -z "$TESTES_DIR" ]; then
    TESTES_DIR="$(default_tests_dir "$VERSION")"
fi

if [ ! -x "$BIN" ]; then
    echo "[err] lua-rs binary not built at $BIN" >&2
    exit 2
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
TSV="$OUT_DIR/run_all.tsv"
mkdir -p "$OUT_DIR"
: > "$TSV"

PREAMBLE_EXPR="_soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}; _G=_G or _ENV; if _VERSION==nil then _VERSION=\"Lua $VERSION\" end"

export LUA_PATH="$TESTES_DIR/?.lua;$TESTES_DIR/?/init.lua;./?.lua;./?/init.lua"
export LUA_RS_VERBOSE=1
export OMNILUA_VERSION="$VERSION"

pass=0; fail=0; timeout=0
declare -a FAILED

for test_file in "$TESTES_DIR"/*.lua; do
    [ -e "$test_file" ] || continue
    base=$(basename "$test_file" .lua)
    case "$base" in _*) continue ;; esac

    wrap="$OUT_DIR/$base.wrap.lua"
    outfile="$OUT_DIR/$base.out"
    {
        printf '%s\n' "$PREAMBLE_EXPR"
        printf 'dofile([[%s]])\n' "$test_file"
    } > "$wrap"

    case "$base" in
        all) run_cwd="$TESTES_DIR" ;;
        *)   run_cwd="$ROOT" ;;
    esac

    if command -v gtimeout >/dev/null 2>&1; then
        ( cd "$run_cwd" && gtimeout --signal=KILL "$TEST_TIMEOUT_S" "$BIN" "$wrap" > "$outfile" 2>&1 )
        rc=$?
    elif command -v timeout >/dev/null 2>&1; then
        ( cd "$run_cwd" && timeout --signal=KILL "$TEST_TIMEOUT_S" "$BIN" "$wrap" > "$outfile" 2>&1 )
        rc=$?
    else
        ( cd "$run_cwd" && "$BIN" "$wrap" > "$outfile" 2>&1 ) &
        pid=$!
        ( sleep "$TEST_TIMEOUT_S" && kill -9 "$pid" 2>/dev/null ) &
        wpid=$!
        wait "$pid" 2>/dev/null
        rc=$?
        kill "$wpid" 2>/dev/null
        wait "$wpid" 2>/dev/null || true
    fi

    if [ "$rc" = "137" ] || [ "$rc" = "124" ]; then
        timeout=$((timeout+1))
        FAILED+=("$base")
        printf "  %-20s TIMEOUT (%ds)\n" "$base" "$TEST_TIMEOUT_S"
        printf '%s\tTIMEOUT\t-\n' "$base" >> "$TSV"
        continue
    fi

    if [ "$rc" = "0" ] \
        && ! grep -qE "not yet implemented|panicked at|^\[err\]|pcall_k failed" "$outfile"; then
        pass=$((pass+1))
        printf "  %-20s PASS\n" "$base"
        printf '%s\tPASS\t-\n' "$base" >> "$TSV"
    else
        fail=$((fail+1))
        FAILED+=("$base")
        msg=$(grep -E "pcall_k failed|panicked at|not yet implemented|^\[err\]|stack overflow|assertion failed|syntax error|attempt to " "$outfile" | head -1)
        if [ -z "$msg" ]; then
            msg=$(awk 'NF { print; exit }' "$outfile")
        fi
        msg=$(printf '%s' "$msg" | sed -E "s#$ROOT/##g; s#.*target/debug/omnilua: #omnilua: #" | cut -c1-140)
        printf "  %-20s FAIL  %s\n" "$base" "$msg"
        printf '%s\tFAIL\t%s\n' "$base" "$msg" >> "$TSV"
    fi
done

total=$((pass+fail+timeout))
if [ "$total" -gt 0 ]; then
    pass_pct=$(( pass*100/total ))
else
    pass_pct=0
fi

echo ""
echo "=========================================="
printf "  Version: %s\n" "$VERSION"
printf "  Total:   %3d official tests\n" "$total"
printf "  Pass:    %3d  (%3d%%)\n" "$pass" "$pass_pct"
printf "  Fail:    %3d\n" "$fail"
printf "  Timeout: %3d\n" "$timeout"
echo "  TSV:     $TSV"
echo "=========================================="
exit 0
