#!/usr/bin/env bash
# Phase B+ oracle: run a Lua program against C-Lua and our Rust impl,
# diff stdout + exit code.
# Usage: ./diff-output.sh <program.lua>
#
# Exits 0 if outputs match, 1 otherwise. Writes any diff to
# results/<program>.output.diff.

set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <program.lua>" >&2
    exit 2
fi

PROG="$1"
NAME="$(basename "$PROG" .lua)"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RESULTS="$ROOT/harness/oracle/results"
C_LUA="$ROOT/reference/lua-5.4.7/src/lua"
RUST_LUA="$ROOT/target/release/omnilua"

mkdir -p "$RESULTS"

# Reference run
REF_OUT="$RESULTS/$NAME.ref.stdout"
REF_RC=0
"$C_LUA" "$PROG" > "$REF_OUT" 2>&1 || REF_RC=$?

# Our run
if [ ! -x "$RUST_LUA" ]; then
    echo "[diff-output] $RUST_LUA not built — Phase B not complete" >&2
    echo "FAIL: lua-rs not built" > "$RESULTS/$NAME.output.diff"
    exit 1
fi
OUR_OUT="$RESULTS/$NAME.ours.stdout"
OUR_RC=0
"$RUST_LUA" "$PROG" > "$OUR_OUT" 2>&1 || OUR_RC=$?

if [ "$REF_RC" = "$OUR_RC" ] && cmp -s "$REF_OUT" "$OUR_OUT"; then
    echo "PASS $NAME (rc=$REF_RC)"
    exit 0
else
    {
        echo "exit codes: ref=$REF_RC ours=$OUR_RC"
        echo "--- stdout diff ---"
        diff "$REF_OUT" "$OUR_OUT" || true
    } > "$RESULTS/$NAME.output.diff"
    echo "FAIL $NAME (see $RESULTS/$NAME.output.diff)"
    exit 1
fi
