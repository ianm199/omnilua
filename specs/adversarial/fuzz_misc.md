# Adversarial fuzz: fuzz_misc (structural / boundary)

Differential helper: `specs/oracle/diff_one.sh <ver> '<code>'`. Reference binaries
in `/tmp/lua-refs/bin`. All rows below were reproduced with the helper (DIFF) or
directly via `LUA_RS_VERSION=<v> target/debug/lua-rs`.

Run summary: ~310 distinct snippets across 5.3/5.4/5.5 (≈928 version-cases).
MATCH 811 / DIFF 117. Counts inflated by two pervasive root causes (the missing
`[C]: in ?` traceback frame, and the 5.5 float round-trip formatting) that each
fire on dozens of cases. Distinct root-cause divergences below.

## Confirmed divergences

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `local global = 5` (5.5) | **PANIC** exit 101, `index out of bounds: len 37 index 37` at `crates/lua-lex/src/lib.rs:662` | `5` (compat lets `global` be an identifier) | REGRESSION/CRASH | **HEADLINE.** 5.5-only. `global` as a regular name crashes the lexer. 5.3/5.4 handle it fine. Reference 5.5 has `LUA_COMPAT_GLOBAL` on, so `global`/`local global`/`t.global` are all valid identifiers; we hard-reserve it and overrun the reserved-word table. |
| `global=1` (5.5) | `<name> expected near '='` | (ref accepts, sets global) | REGRESSION | Same root: we treat `global` as a strict keyword; ref compat treats it as a name. |
| `string.find("hello","l+")` (all) | `3  4  ` (3 returns) | `3  4` (2 returns) | REGRESSION+FIDELITY | **Correctness bug, all versions.** A pattern with magic chars but no captures yields an extra trailing empty-string return. Plain literal patterns and patterns with explicit captures are correct. `select('#',...)`=3 vs ref 2. |
| `math.tointeger(2.5)` / `(-2.5)` / `(math.huge)` / `(nil)` (all) | `false` | `nil` | REGRESSION+FIDELITY | All versions. Non-convertible input must return `nil`, we return boolean `false`. |
| `local mt={__lt=...}; x<=y` (5.3, 5.4) | error `attempt to compare two table values` | `true` | REGRESSION+FIDELITY | 5.3 AND 5.4 reference derive `__le` from `__lt` when `__le` is absent; we don't (we behave like 5.5, which correctly errors). So 5.3/5.4 are wrong, 5.5 matches. |
| `print(1/3)` etc. (5.5) | 14-digit `0.33333333333333` | round-trip `0.33333333333333331` | FIDELITY (central, 5.5) | **Pervasive.** 5.5 prints floats with enough digits to round-trip (`%.17g`-ish); we still use 5.4's `%.14g`. Hits `math.pi`, `2^0.5`, `math.sqrt(2)`, `0.1+0.2`, `1e14`, `2^53`, large/denormal floats — any non-trivial float under 5.5. |
| `print(2^53, 2^53+1)` (5.5) | `9.007199254741e+15` | `9007199254740992.0` | FIDELITY (5.5) | Same root as float round-trip; 5.5 keeps fixed-point form for these magnitudes. |
| error tracebacks (all errors, all versions) | last frame is `... in main chunk` | adds final `\t[C]: in ?` | REGRESSION+FIDELITY | **Pervasive.** Every uncaught error's traceback omits the trailing `[C]: in ?` frame (the C bootstrap). Hits `error()`, `assert`, index/arith/call/compare errors, argerrors — essentially all DIFFs with a traceback. |
| `error("boom")`, `assert`, `select`, `tonumber` callsites (5.5) | `[C]: in function 'X'` | `[C]: in global 'X'` / `in field 'char'` | FIDELITY (5.5) | 5.5 renames traceback frames: globals → `in global 'name'`, table fields (`string.char`, `string.format`) → `in field 'char'`. We always emit `in function`. |
| `select(0,'a')` / `select(-4,...)` / `select(1.5,...)` (all) | `bad argument #1 (index out of range)` | `(command line):1: bad argument #1 to 'select' (index out of range)` | REGRESSION+FIDELITY | **Pervasive arg-error bug.** Our `luaL_argerror` output omits both the `(command line):N:` location prefix and the `to '<fname>'` clause. |
| `string.char(256)`, `string.format('%d',3.5)`, `tonumber('10',37)` (all) | `bad argument #N (...)` | `(command line):1: bad argument #N to 'char'/'format'/'tonumber' (...)` | REGRESSION+FIDELITY | Same arg-error root cause across stdlib. |
| `#nil` (all) | `attempt to get length of a nil value` | `(command line):1: attempt to get length...` | REGRESSION+FIDELITY | Length-error message lacks the `(command line):N:` location prefix (call/index errors DO have it — selective bug). |
| `true .. "x"`, `nil .. "x"` (all) | `attempt to concatenate a ... value` | `(command line):1: attempt to concatenate...` | REGRESSION+FIDELITY | Concat-error message lacks the location prefix, same family as `#nil`. |
| `-"x"` (5.4, 5.5) | `attempt to unm a 'string' with a 'function'` | `attempt to unm a 'string' with a 'string'` | REGRESSION | Wrong operand type reported in the unary-minus metamethod error ("function" vs "string"). |
| `-"x"` (5.3) | `attempt to unm a 'string' with a 'function'` + `[C]: in metamethod 'unm'` | `(command line):1: attempt to perform arithmetic on a string value (constant 'x')` | FIDELITY (5.3) | 5.3 uses an entirely different (older) arithmetic-error message shape; we emit the 5.4-style metamethod message. |
| `t.x` via `__index` that errors (5.3) | traceback `in metamethod 'index'` | `in metamethod '__index'` | FIDELITY (5.3) | 5.3 names metamethods with the leading `__` in tracebacks; we drop it. |
| `string.format("%q", 1/0)` (5.3) | `1e9999` | `inf` | FIDELITY (5.3) | 5.3 `%q` emits `inf` for infinity; we emit the 5.4-style `1e9999`. (5.4/5.5 match.) |
| `print(#"\u{110000}")` … `\u{7FFFFFFF}` (5.3) | accepts, prints byte count (`4`/`6`) | `UTF-8 value too large near ...` (error) | FIDELITY (5.3) | This 5.3.6 build rejects `\u` escapes >0x10FFFF at lex time; our 5.3 lexer accepts up to 0x7FFFFFFF (too lax). Boundary is exactly 0x10FFFF (matches) vs 0x110000 (diverges). |
| `error()` / `error(nil)` through pcall (5.5) | `false  nil` | `false  <no error object>` | FIDELITY (5.5) | 5.5 replaces a nil error object with the string `<no error object>`; we propagate `nil`. (5.3/5.4 correctly give `nil`.) |
| `utf8.offset("héllo", 3)` (5.5) | `4` (1 return) | `4  4` (2 returns) | FIDELITY (5.5) | 5.5 `utf8.offset` additionally returns the final byte position of the char; we return one value. |
| `#({...})` from varargs `f(1,nil,3)` (5.5) | `3` | `1` | FIDELITY (5.5, niche) | `#` of a table with a hole is unspecified, but 5.5's reference deterministically picks border 1 here (new array representation) while 5.3/5.4 ref and we all pick 3. Reproducible 5.5-only divergence. |

## Non-divergences worth recording (probed, MATCH)
- Integer overflow/wrap (`maxinteger+1`, `mininteger-1`, `-mininteger`, hex wrap), `1//0`/`1%0` first-line messages, `0/0`/`huge`/`-0.0` printing, bit ops incl. shifts ≥64, deep nesting (40 parens / 30-deep tables / 500-deep recursion), long arg lists (200 args), multiple-assignment truncation/extension, trailing commas/semicolons in constructors, `\x \z \65 \u{10FFFF}` escapes, long brackets `[==[...]==]`, string.pack/unpack, seeded `math.random`, integer/float key normalization, `__lt`-derived-`__le` under 5.5 (correctly errors).

## Most important (with repro)
1. CRASH: `LUA_RS_VERSION=5.5 lua-rs -e 'local global = 5'` → panic, exit 101 (lexer OOB). 5.5 must honor `LUA_COMPAT_GLOBAL` (treat `global` as identifier) or at minimum not panic.
2. `string.find` returns an extra empty value for magic-char patterns without captures: `string.find("hello","l+")` → `3 4 ""` (all versions).
3. `math.tointeger(2.5)` returns `false` instead of `nil` (all versions).
4. 5.3/5.4 lost `__lt`→`__le` derivation: `setmetatable({},{__lt=f}) <= ...` errors where reference returns the comparison (we incorrectly use 5.5 semantics for 5.3/5.4).
5. Pervasive error-formatting gaps: every traceback misses the trailing `[C]: in ?`; `luaL_argerror` drops the `(command line):N:` prefix and `to 'fname'` clause; `#nil`/concat errors drop the location prefix; 5.5 uses `in global`/`in field` frame naming we don't emit.
