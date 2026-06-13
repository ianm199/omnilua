#!/usr/bin/env bash
# Run a single official Lua 5.4.7 test file against our Rust implementation.
# Usage: ./run-test-file.sh <name.lua>   (e.g. constructs.lua)
#
# The test is run with _U=true (user-test mode, no internal hooks).
# Exit 0 = passed; non-zero = failed (output captured to results/).

set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <test-name.lua>" >&2
    exit 2
fi

TEST="$1"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TESTS_DIR="$ROOT/reference/lua-5.4.7-tests"
RESULTS="$ROOT/harness/oracle/results"
RUST_LUA="$ROOT/target/release/omnilua"

mkdir -p "$RESULTS"

if [ ! -f "$TESTS_DIR/$TEST" ]; then
    echo "[run-test-file] no such test: $TESTS_DIR/$TEST" >&2
    exit 2
fi
if [ ! -x "$RUST_LUA" ]; then
    echo "[run-test-file] $RUST_LUA not built" >&2
    exit 1
fi

OUT="$RESULTS/test-$TEST.stdout"
cd "$TESTS_DIR"
if "$RUST_LUA" -e "_U=true" "$TEST" > "$OUT" 2>&1; then
    echo "PASS $TEST"
else
    rc=$?
    echo "FAIL $TEST (rc=$rc, see $OUT)"
    exit "$rc"
fi
