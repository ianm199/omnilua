#!/usr/bin/env bash
# Differential oracle: run ONE Lua snippet through version-selected lua-rs and
# the matching reference C binary; print MATCH or a DIFF block. Normalizes
# program-name paths and heap addresses (known noise). The CALLER must avoid
# nondeterministic snippets (unseeded random, os.time/clock) or treat such DIFFs
# as noise.
#   diff_one.sh 5.3 'print(math.type("3"+0))'
set -uo pipefail
ver="${1:?usage: diff_one.sh <5.3|5.4|5.5> <luacode>}"; shift; code="$*"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RS="${LUA_RS_BIN:-$ROOT/target/debug/lua-rs}"

choose_ref() {
  local local_ref="$1" tmp_ref="$2"
  if [ -x "$local_ref" ]; then
    echo "$local_ref"
  else
    echo "$tmp_ref"
  fi
}

case "$ver" in
  5.1) ref="${LUA_RS_REF_51:-/tmp/lua-refs/bin/lua5.1.5}" ;;
  5.2) ref="${LUA_RS_REF_52:-/tmp/lua-refs/bin/lua5.2.4}" ;;
  5.3) ref="${LUA_RS_REF_53:-$(choose_ref "$ROOT/reference/lua-5.3.6/src/lua" "/tmp/lua-refs/bin/lua5.3.6")}" ;;
  5.4) ref="${LUA_RS_REF_54:-$(choose_ref "$ROOT/reference/lua-5.4.7/src/lua" "/tmp/lua-refs/bin/lua5.4.7")}" ;;
  5.5) ref="${LUA_RS_REF_55:-$(choose_ref "$ROOT/reference/lua-5.5.0/src/lua" "/tmp/lua-refs/bin/lua5.5.0")}" ;;
  *) echo "unknown version $ver"; exit 2 ;;
esac
[ -x "$ref" ] || { echo "missing reference binary $ref"; exit 2; }
[ -x "$RS" ] || { echo "missing $RS (cargo build -p lua-cli)"; exit 2; }
norm(){ sed -E -e 's#[^ ]*/lua-rs#PROG#g' -e 's#[^ ]*/lua5\.[0-9.]+#PROG#g' \
                -e 's#(table|function|userdata|thread): (builtin: )?0x[0-9a-fA-F]+#\1: ADDR#g' \
                -e 's#0x[0-9a-fA-F]{6,}#ADDR#g'; }
a=$(LUA_RS_VERSION="$ver" "$RS" -e "$code" 2>&1); ae=$?
b=$("$ref" -e "$code" 2>&1); be=$?
na=$(printf '%s' "$a" | norm); nb=$(printf '%s' "$b" | norm)
if [ "$na" = "$nb" ] && [ "$ae" = "$be" ]; then
  echo "MATCH"
else
  printf 'DIFF ver=%s rs_exit=%s ref_exit=%s\n  CODE: %s\n  OURS: %s\n  REF : %s\n' \
    "$ver" "$ae" "$be" "$code" "$(printf '%s' "$na" | tr '\n' '|')" "$(printf '%s' "$nb" | tr '\n' '|')"
fi
