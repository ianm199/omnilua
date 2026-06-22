#!/usr/bin/env bash
# quick_file.sh <ver> <base> — fast inner-loop check of one official test file.
#
# Runs the wrapped official test through our binary with a SHORT timeout (default
# 8s, override QUICK_TIMEOUT_S). Prints one of:
#   PASS         exit 0, no failure marker  (the file flipped)
#   FAIL <msg>   errored fast (the normal "still has a divergence" case)
#   HANG         exceeded the short timeout (infinite loop / pathological churn)
#
# WHY: the only iteration-speed trap measured in this port is a file that HANGS
# to the full 60s oracle timeout (e.g. db.lua@5.3, an infinite loop). The warm
# edit->build->diff_one loop is ~1.1s; a 60s hang re-run dwarfs it. A hang is
# itself a "not passing" signal — you do not need 60s to learn it. Develop on
# diff_one snippets (0.2s) and use THIS for the occasional whole-file advance
# check; only the final gate (multiversion_diff_suite.sh) needs the long timeout.
#
#   bash harness/quick_file.sh 5.3 errors
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${LUA_RS_BIN:-$ROOT/target/debug/omnilua}"
T="${QUICK_TIMEOUT_S:-8}"
ver="${1:?usage: quick_file.sh <ver> <base>}"; base="${2:?usage: quick_file.sh <ver> <base>}"
wrap="/tmp/compat-report/diff/$ver/$base.wrap.lua"
[ -f "$wrap" ] || { echo "no wrap $wrap (run harness/multiversion_diff_suite.sh once)"; exit 2; }
out=$(OMNILUA_VERSION="$ver" gtimeout --signal=KILL "$T" "$BIN" "$wrap" 2>&1); rc=$?
if [ "$rc" = 137 ] || [ "$rc" = 124 ]; then echo "HANG (>${T}s)"; exit 1; fi
if [ "$rc" = 0 ] && ! printf '%s' "$out" | grep -qE "not yet implemented|panicked at|pcall_k failed|assertion failed|attempt to "; then
  echo "PASS"; exit 0
fi
msg=$(printf '%s' "$out" | grep -oE "[a-z_]+\.lua:[0-9]+:.*" | head -1 | cut -c1-90)
echo "FAIL ${msg:-$(printf '%s' "$out" | head -1 | cut -c1-90)}"; exit 1
