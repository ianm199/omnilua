#!/usr/bin/env bash
# Print Rust lua-rs and reference C Lua value/frame layout sizes.
#
# This is telemetry, not a benchmark. It quantifies the representation gap that
# shows up in VM/value-copy discussions.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

REF_SRC="$ROOT/reference/lua-5.4.7/src"

echo "==> Rust layout" >&2
cargo run --quiet -p lua-vm --example value_layout

echo "==> C Lua 5.4.7 layout" >&2
C_PROBE=$(mktemp "${TMPDIR:-/tmp}/lua-layout-XXXXXX.c")
C_BIN=$(mktemp "${TMPDIR:-/tmp}/lua-layout-XXXXXX")
trap 'rm -f "$C_PROBE" "$C_BIN"' EXIT

cat > "$C_PROBE" <<'C'
#include <stdio.h>
#include "lstate.h"
#include "lobject.h"
#include "lfunc.h"
#include "ltable.h"

#define ROW(name, type) printf("c\t%s\t%zu\t%zu\n", name, sizeof(type), _Alignof(type))

int main(void) {
  ROW("TValue", TValue);
  ROW("StackValue", StackValue);
  ROW("CallInfo", CallInfo);
  ROW("lua_State", lua_State);
  ROW("TString", TString);
  ROW("Table", Table);
  ROW("Node", Node);
  ROW("LClosure", LClosure);
  ROW("UpVal", UpVal);
  return 0;
}
C

cc -std=c11 -I "$REF_SRC" "$C_PROBE" -o "$C_BIN"
"$C_BIN"
