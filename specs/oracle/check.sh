#!/usr/bin/env bash
# Oracle diff harness for the multi-version work.
#
# Runs a battery of one-line snippets through BOTH our version-selected lua-rs
# (LUA_RS_VERSION=<v> target/debug/lua-rs) and the matching reference C binary
# in /tmp/lua-refs/bin, normalizes (first line, strip program-name prefix), and
# reports PASS/FAIL per snippet. The reference binary is the oracle.
#
#   specs/oracle/check.sh 5.3   # or 5.4 (sanity) or 5.5
#
# Exit code = number of failures (0 == all match the reference).

set -uo pipefail
ver="${1:?usage: check.sh 5.3 or 5.4 or 5.5}"
case "$ver" in
  5.1) ref=/tmp/lua-refs/bin/lua5.1.5 ;;
  5.2) ref=/tmp/lua-refs/bin/lua5.2.4 ;;
  5.3) ref=/tmp/lua-refs/bin/lua5.3.6 ;;
  5.4) ref=/tmp/lua-refs/bin/lua5.4.7 ;;
  5.5) ref=/tmp/lua-refs/bin/lua5.5.0 ;;
  *) echo "unknown version $ver"; exit 2 ;;
esac
ROOT="/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port/.claude/worktrees/git-issues"
LUARS="$ROOT/target/debug/lua-rs"
[ -x "$ref" ] || { echo "missing reference binary $ref"; exit 2; }
[ -x "$LUARS" ] || { echo "missing $LUARS (cargo build -p lua-cli)"; exit 2; }

norm() { head -1 | sed -E 's#^[^ ]+: ##'; }   # first line, drop "PROG: " prefix
pass=0; fail=0
run() { # label  code
  local label="$1" code="$2" a b
  a=$(LUA_RS_VERSION="$ver" "$LUARS" -e "$code" 2>&1 | norm)
  b=$("$ref" -e "$code" 2>&1 | norm)
  if [ "$a" = "$b" ]; then pass=$((pass+1)); printf "PASS  %s\n" "$label"
  else fail=$((fail+1)); printf "FAIL  %s\n        rs : %s\n        ref: %s\n" "$label" "$a" "$b"; fi
}

echo "== oracle battery: lua-rs($ver) vs $(basename "$ref") =="
run "_VERSION"            'print(_VERSION)'

if [ "$ver" = "5.3" ]; then
  run "coroutine.close=nil" 'print(type(coroutine.close))'
  run "warn=nil"            'print(type(warn))'
  run "math.type present"   'print(type(math.type))'
  run "bit32 present"       'print(type(bit32))'
  run "bit32.band(6,3)"     'print(bit32.band(6,3))'
  run "bit32 full surface"  'print(bit32.btest(6,3), bit32.extract(0xF0,4,4), bit32.replace(0,5,0,4), bit32.arshift(-8,1), bit32.lrotate(1,1), bit32.rrotate(1,1))'
  run "<const> rejected"    'local x <const> = 1; print(x)'
  run "strcoerce->float"    "print(math.type('0x10'+0))"
  run "table.create=nil"    'print(type(table.create))'
  # Expanded slice drawn from official-5.3 test surface (bitwise/math/string),
  # all behaviors the shared modern core + 5.3 seams implement.
  run "bitwise &"           'print(6 & 3)'
  run "bitwise ~"           'print(5 ~ 3)'
  run "bitwise <<"          'print(1 << 10)'
  run "bnot"                'print(~0)'
  run "math.type int"       'print(math.type(3), math.type(3.0))'
  run "floor div"           'print(7//2, math.type(7//2))'
  run "maxinteger"          'print(math.maxinteger)'
  run "tointeger"           'print(math.tointeger(3.0))'
  run "format %d"           "print(string.format('%d', 42))"
  run "format %f"           "print(string.format('%5.2f', 3.14159))"
  run "tostring float"      'print(tostring(1.0))'
  run "pow is float"        'print(math.type(2^2))'
  run "bit32.band mask"     'print(bit32.band(0xFF,0x0F))'
fi

if [ "$ver" = "5.4" ]; then
  run "coroutine.close fn"  'print(type(coroutine.close))'
  run "warn fn"             'print(type(warn))'
  run "bit32=nil"           'print(type(bit32))'
  run "<const> ok"          'local x <const> = 1; print(x)'
  run "strcoerce->integer"  "print(math.type('0x10'+0))"
  run "table.create=nil"    'print(type(table.create))'
fi

if [ "$ver" = "5.2" ]; then
  # Float-only number model (no int subtype): integer-valued floats print
  # without ".0", and the int-subtype math members are absent.
  run "10/2 (no .0)"        'print(10/2)'
  run "2^2 (no .0)"         'print(2^2)'
  run "tostring(1.0)"       'print(tostring(1.0))'
  run "concat 1..2"         'print(1 .. 2)'
  run "1.5 stays float"     'print(1.5)'
  run "math.floor"          'print(math.floor(3.7))'
  run "string.format %d"    "print(string.format('%d', 42))"
  run "string.format %.3f"  "print(string.format('%.3f', 3.14159))"
  # %d-family truncates a non-integral number toward zero (float-only); 5.3+ errors.
  run "format %d trunc"     "print(string.format('%d', 3.5))"
  run "format %d neg trunc" "print(string.format('%d', -3.5))"
  run "format %x trunc"     "print(string.format('%x', 255.9))"
  run "format %d strcoerce" "print(string.format('%d', '3.5'))"
  run "math.type absent"    'print(type(math.type))'
  run "math.tointeger nil"  'print(type(math.tointeger))'
  run "math.maxinteger nil" 'print(type(math.maxinteger))'
  run "math.ult absent"     'print(type(math.ult))'
  # Stdlib roster (5.2): bit32 present, utf8/table.move absent, math compat on,
  # unpack/loadstring globals present, no warn/coroutine.close.
  run "bit32 present"       'print(type(bit32))'
  run "bit32.band(6,3)"     'print(bit32.band(6,3))'
  run "bit32 surface"       'print(bit32.btest(6,3), bit32.extract(0xF0,4,4), bit32.lrotate(1,1))'
  run "utf8 absent"         'print(type(utf8))'
  run "table.pack"          'print(type(table.pack))'
  run "table.unpack"        'print(type(table.unpack))'
  run "table.move absent"   'print(type(table.move))'
  run "unpack global"       'print(unpack({10,20,30}))'
  run "loadstring global"   'print(loadstring("return 7")())'
  run "math.atan2 present"  'print(type(math.atan2))'
  run "math.cosh present"   'print(type(math.cosh))'
  run "math.pow present"    'print(type(math.pow))'
  run "math.log10 present"  'print(type(math.log10))'
  run "math.frexp present"  'print(type(math.frexp))'
  run "coroutine.close nil" 'print(type(coroutine.close))'
  run "warn absent"         'print(type(warn))'
  run "table.create absent" 'print(type(table.create))'
  run "_ENV present"        'print(type(_ENV))'
  run "getfenv absent"      'print(type(getfenv))'
  run "setfenv absent"      'print(type(setfenv))'
  run "goto/label"          'do goto x ::x:: end print("ok")'
  # Syntax rejection: the 5.3 integer operators must not parse in 5.2.
  run "// rejected"         'print(7//2)'
  run "& rejected"          'print(6 & 3)'
  run "| rejected"          'print(6 | 3)'
  run "<< rejected"         'print(1 << 4)'
  run ">> rejected"         'print(256 >> 4)'
  run "~ binary rejected"   'print(5 ~ 3)'
  run "~ unary rejected"    'print(~0)'
  run "<const> rejected"    'local x <const> = 1'
  # Number behavior / coercion stays float.
  run "strcoerce add"       'print("10"+5)'
  run "tonumber hexfloat"   'print(tonumber("0x1p4"))'
  run "math.modf"           'print(math.modf(3.7))'
  run "math.huge"           'print(math.huge)'
  run "big int literal"     'print(9007199254740993)'
  run "module present"      'print(type(module))'
  run "package.loaders"     'print(type(package.loaders))'
  run "package.searchers"   'print(type(package.searchers))'
fi

if [ "$ver" = "5.5" ]; then
  run "implicit global ok"  'y = 3; print(y)'
  run "global decl r/w"     'global print, a; a = 5; print(a)'
  run "multi-name decl"     'global print, a, b; a = 1; b = 2; print(a + b)'
  run "undeclared assign"   'global a; a = 1; z = 9'
  run "undeclared read"     'global print; print(nope)'
  run "undeclared in fn"    'global print; local function f() return nope end print(f())'
  run "const global reassign" 'global print; global x <const> = 7; print(x); x = 9'
  run "declared global in fn" 'global print, c; c = 0; local function inc() c = c + 1 end inc(); print(c)'
  run "table.create fn"     'print(type(table.create))'
fi

echo "-- $pass passed, $fail failed (vs reference) --"
exit "$fail"
