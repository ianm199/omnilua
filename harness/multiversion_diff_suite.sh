#!/usr/bin/env bash
# multiversion_diff_suite.sh — per-version official-suite parity, ours vs reference.
#
# For each version 5.1-5.5, runs every official test file through BOTH our
# version-selected omnilua AND the matching real PUC-Rio reference binary under
# the identical stock harness preamble, then classifies each file:
#   OUR_BUG    ref PASS, we FAIL/TIMEOUT  (the only thing that counts against us)
#   both_fail  ref also fails under the stock harness (needs the C `ltests` lib)
#   our_better ref fails, we pass (stress/ltests artifact — not a real win)
#
# This is the honest compatibility number: ours / (files the reference passes).
# Reference binaries come from /tmp/lua-refs/bin (see specs/oracle/CONTRACT.md;
# rebuild with the curl+make recipe there). Output TSVs in /tmp/compat-report/diff.
#
#   bash harness/multiversion_diff_suite.sh            # all five versions
#   TEST_TIMEOUT_S=60 bash harness/multiversion_diff_suite.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUR="${LUA_RS_BIN:-$ROOT/target/debug/omnilua}"
OUTROOT="${LUA_RS_DIFF_OUT:-/tmp/compat-report/diff}"
TIMEOUT="${TEST_TIMEOUT_S:-30}"
mkdir -p "$OUTROOT"

testdir() { case "$1" in
  5.1) echo "$ROOT/reference/extra-tests/lua5.1-tests";;
  5.2) echo "$ROOT/reference/extra-tests/lua-5.2.2-tests";;
  5.3) echo "$ROOT/reference/lua-5.3.6-tests";;
  5.4) echo "$ROOT/reference/lua-c/testes";;
  5.5) echo "$ROOT/reference/lua-5.5.0-tests";;
esac; }
refbin() { case "$1" in
  5.1) echo /tmp/lua-refs/bin/lua5.1.5;;
  5.2) echo /tmp/lua-refs/bin/lua5.2.4;;
  5.3) echo /tmp/lua-refs/bin/lua5.3.6;;
  5.4) echo /tmp/lua-refs/bin/lua5.4.7;;
  5.5) echo /tmp/lua-refs/bin/lua5.5.0;;
esac; }

run_one() { # bin version wrap cwd lpath
  local bin="$1" ver="$2" wrap="$3" cwd="$4" lp="$5" out rc
  out=$(cd "$cwd" && OMNILUA_VERSION="$ver" LUA_PATH="$lp" gtimeout --signal=KILL "$TIMEOUT" "$bin" "$wrap" 2>&1)
  rc=$?
  if [ "$rc" = 137 ] || [ "$rc" = 124 ]; then echo "TIMEOUT"; return; fi
  if [ "$rc" = 0 ] && ! printf '%s' "$out" | grep -qE "not yet implemented|panicked at|pcall_k failed"; then echo "PASS"; else echo "FAIL"; fi
}

for ver in 5.1 5.2 5.3 5.4 5.5; do
  td="$(testdir "$ver")"; ref="$(refbin "$ver")"
  [ -d "$td" ] || { echo "skip $ver (no testdir $td)"; continue; }
  [ -x "$ref" ] || { echo "skip $ver (no refbin $ref)"; continue; }
  lp="$td/?.lua;$td/?/init.lua;./?.lua;./?/init.lua"
  wrapdir="$OUTROOT/$ver"; mkdir -p "$wrapdir"
  preamble="_soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}; _G=_G or _ENV; if _VERSION==nil then _VERSION=\"Lua $ver\" end"
  tsv="$OUTROOT/$ver.tsv"; : > "$tsv"
  for tf in "$td"/*.lua; do
    base=$(basename "$tf" .lua)
    case "$base" in _*) continue;; esac
    wrap="$wrapdir/$base.wrap.lua"
    { printf '%s\n' "$preamble"; printf 'dofile([[%s]])\n' "$tf"; } > "$wrap"
    case "$base" in all) cwd="$td";; *) cwd="$ROOT";; esac
    rstat=$(run_one "$ref" "$ver" "$wrap" "$cwd" "$lp")
    ostat=$(run_one "$OUR" "$ver" "$wrap" "$cwd" "$lp")
    printf '%s\t%s\t%s\n' "$base" "$rstat" "$ostat" >> "$tsv"
  done
  awk -F'\t' -v v="$ver" '{t++} $2=="PASS"{rp++} $3=="PASS"{op++}
    $2=="PASS"&&$3!="PASS"{bug++} $2!="PASS"&&$3!="PASS"{bf++} $2!="PASS"&&$3=="PASS"{ob++}
    END{printf "%s: files=%d ref_pass=%d our_pass=%d | OUR_BUGS=%d both_fail=%d our_better=%d\n",v,t,rp,op,bug,bf,ob}' "$tsv"
done
