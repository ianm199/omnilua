# reg54_lang — 5.4 REGRESSION GUARD vs lua5.4.7

Category: closures/upvalues, metatables/metamethods, pcall/error/xpcall, coroutines,
`<const>`/`<close>`, goto/labels, varargs, table constructors, OOP, error messages, tostring.

All cases SHOULD match lua5.4.7. Any DIFF is a REGRESSION.

Helper: `specs/oracle/diff_one.sh 5.4 '<code>'`.

## Summary

- Cases run: 112 distinct snippets (72 broad + 20 + 20 probes; counts exclude `--` comment lines).
- MATCH: ~91. DIFF: 21 (all collapse into 3 root-cause regression families).
- Every confirmed DIFF is classed **REGRESSION** (5.4-vs-5.4).

## Confirmed regression families

### R1 — runtime error messages missing the `(command line):N:` position prefix
Affects: length (`#`), concatenation (`..`), and string-arithmetic errors. Note that
arithmetic-on-nil (`nil+1`, `1+nil`, `-nil`), index-nil, call-nil, and comparison errors
DO carry the prefix correctly, so this is subsystem-specific, not global.

### R2 — string-arithmetic / unary error reports wrong operand type (`'function'`) and can swap operands
In `sub`/`unm` (and the arith-metamethod path generally), when an operand is a
non-coercible string, ours reports the *other* operand's type as `'function'` instead of
its real type, and for `{}-"y"` ours reports `'string' with a 'string'`-shaped text with
the wrong leading type vs ref's `'table' with a 'string'`. This is a content bug, not just
a prefix bug. (Bitwise-on-string errors, e.g. `"x"&1`, are correct.)

### R3 — top-level error traceback is missing the trailing `\t[C]: in ?` frame
Every uncaught top-level error traceback omits the final `[C]: in ?` line that real
lua5.4.7 prints.

## Table: snippet | ours | ref | class | note

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `pcall(fn() return #nil end)` err | `attempt to get length of a nil value` | `(command line):1: attempt to get length of a nil value` | REGRESSION | R1 missing prefix |
| `pcall(fn() return #true end)` err | `...boolean value` | `(command line):1: ...boolean value` | REGRESSION | R1 |
| `pcall(fn() return #1 end)` err | `...number value` | `(command line):1: ...number value` | REGRESSION | R1 |
| `pcall(fn() return "a"..{} end)` err | `attempt to concatenate a table value` | `(command line):1: ...table value` | REGRESSION | R1 |
| `pcall(fn() return {}.."b" end)` err | `...table value` | `(command line):1: ...table value` | REGRESSION | R1 |
| `pcall(fn() return 1 .. {} end)` err | `...table value` | `(command line):1: ...table value` | REGRESSION | R1 |
| `pcall(fn() return {} .. {} end)` err | `...table value` | `(command line):1: ...table value` | REGRESSION | R1 |
| `pcall(fn() return true .. "x" end)` err | `...boolean value` | `(command line):1: ...boolean value` | REGRESSION | R1 |
| `pcall(fn() return nil .. "x" end)` err | `...nil value` | `(command line):1: ...nil value` | REGRESSION | R1 |
| `pcall(fn() return "x" + 1 end)` err | `attempt to add a 'string' with a 'number'` | `(command line):1: attempt to add a 'string' with a 'number'` | REGRESSION | R1 |
| `local a="z"; return a+1` (pcall) | `attempt to add a 'string' with a 'number'` | `(command line):1: ...` | REGRESSION | R1 |
| `"a" * 2` (pcall) | `attempt to mul a 'string' with a 'number'` | `(command line):1: ...` | REGRESSION | R1 |
| `"x" / 2` (pcall) | `attempt to div ...` | `(command line):1: ...` | REGRESSION | R1 |
| `"x" % 2` (pcall) | `attempt to mod ...` | `(command line):1: ...` | REGRESSION | R1 |
| `"x" // 2` (pcall) | `attempt to idiv ...` | `(command line):1: ...` | REGRESSION | R1 |
| `"x" ^ 2` (pcall) | `attempt to pow ...` | `(command line):1: ...` | REGRESSION | R1 |
| `return "x" - "y"` (top-level) | `attempt to sub a 'string' with a 'function'` | `(command line):1: attempt to sub a 'string' with a 'string'` | REGRESSION | R1+R2: wrong operand type AND missing prefix |
| `return {} - "y"` | `attempt to sub a 'string' with a 'function'` | `(command line):1: attempt to sub a 'table' with a 'string'` | REGRESSION | R2 wrong+swapped operand type |
| `return -"x"` / `-"abc"` | `attempt to unm a 'string' with a 'function'` | `(command line):1: attempt to unm a 'string' with a 'string'` | REGRESSION | R2 unary reports bogus 2nd operand `'function'` |
| `return "a"..{}` (top-level) | traceback ends `...in main chunk` | ref ends `...in main chunk` + `\t[C]: in ?` | REGRESSION | R3 missing `[C]: in ?` frame (also R1) |
| `local t={f(),10} local function f()...` | traceback missing `\t[C]: in ?` | ref has `\t[C]: in ?` | REGRESSION | R3 |

## Cases that correctly MATCH (regression guard passed)
closures/upvalues (per-iteration capture, shared upvalues), all binary/unary metamethods
(`__add __sub __mul __div __mod __idiv __pow __unm __concat __len __call __index __newindex
__eq __lt __le __band __bor __bxor __shl __shr __bnot __tostring`), pcall/xpcall return
shapes, `error({table})`, `error("msg",0)`, coroutine create/resume/wrap/status/yield/
error-propagation, `<const>`, `<close>` (single + multiple, LIFO `__close` order), goto/labels,
varargs (`select`, `{...}` with nil holes, table-constructor multi-return truncation),
table-constructor border/`#`, OOP single + inherited dispatch, tostring of
nil/bool/int/float/inf/-inf/nan/maxinteger/mininteger, `string.format %d %s %q`,
method-on-literal, `nil+1`/index-nil/call-nil/comparison error prefixes.

## Repro commands
```
bash specs/oracle/diff_one.sh 5.4 'local ok,e=pcall(function() return #nil end) print(e)'
bash specs/oracle/diff_one.sh 5.4 'local ok,e=pcall(function() return "a"..{} end) print(e)'
LUA_RS_VERSION=5.4 target/debug/lua-rs -e 'return "x" - "y"'   # vs /tmp/lua-refs/bin/lua5.4.7
LUA_RS_VERSION=5.4 target/debug/lua-rs -e 'return -"x"'
bash specs/oracle/diff_one.sh 5.4 'return "a"..{}'             # R3 traceback tail
```
