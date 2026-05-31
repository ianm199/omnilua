# Adversarial category: suite_slices

Method: ran curated official test FILES through version-selected lua-rs vs the
matching reference C binary, with the standard preamble
`_soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}`, from inside the
test directory (absolute path to lua-rs). For every file that diverged, the
first divergence was isolated into a minimal snippet and reconfirmed with
`diff_one.sh`. Test sets: 5.3 = `/tmp/lua-refs/lua-5.3.4-tests`,
5.4 = `reference/lua-5.4.7-tests`, 5.5 = fetched `lua-5.5.0-tests`.

## Whole-file sweep (curated slice: numbers/strings/bitwise/vararg/locals/
## constructs/closures/events/literals/calls/sort/goto + pm/utf8/tpack/attrib/
## coroutine/db)

5.4 is essentially clean: every file MATCHes except pure-noise files
(math/sort = RNG+timing, constructs = `math.random(0,1)` GLOB1, literals =
locale availability). No 5.4 structural regressions found by the slices.

5.3 and 5.5 carry the real fidelity gaps (see table). 5.5 additionally fails to
parse several files outright due to unimplemented 5.5 syntax (`global<const>`,
named vararg `...t`).

File-level result (MATCH = byte-identical after normalization):

| file | 5.3 | 5.4 | 5.5 |
|---|---|---|---|
| math | DIFF(assert@41) | NOISE(rng) | DIFF(parse `global`) |
| strings | DIFF(fmt msg) | MATCH | DIFF(parse `...t`) |
| bitwise | DIFF | MATCH | MATCH |
| vararg | MATCH | MATCH | DIFF(parse `...t`) |
| locals | MATCH | MATCH | DIFF(`global` strict) |
| constructs | DIFF(concat<<) | NOISE(GLOB1) | DIFF |
| closure | DIFF(closure-eq) | MATCH | DIFF(_ENV idx) |
| events | DIFF(le-via-lt) | MATCH | MATCH |
| literals | DIFF(lexmsg) | NOISE(locale) | NOISE(locale) |
| nextvar | MATCH | MATCH | DIFF |
| calls | DIFF(print/tostring) | MATCH | DIFF(`global` strict) |
| sort | NOISE(timing) | NOISE(timing) | DIFF |
| goto | DIFF(label parse) | MATCH | DIFF(parse `global<const>`) |
| pm | MATCH | MATCH | MATCH |
| utf8 | DIFF(arg msg) | MATCH | DIFF |
| tpack | DIFF(arg msg) | MATCH | DIFF |
| attrib | MATCH | MATCH | MATCH |
| coroutine | DIFF(le-via-lt) | MATCH | DIFF |
| db | DIFF(arg msg) | MATCH | DIFF |

## Confirmed minimal-snippet divergences

| snippet | ver | ours | ref | class | note |
|---|---|---|---|---|---|
| `print("7" .. 3 << 1)` | 5.3 | error: bitwise on a string value | `146` | FIDELITY | 5.3 coerces numeric-looking string to integer for bitwise ops; we reject string operands. Central 5.3 semantics. |
| `print(10 >> 1 .. "9")` | 5.3 | error: bitwise on a string value | `0` | FIDELITY | same string→int bitwise coercion. |
| `local mt={__lt=...}; a<=b` (no `__le`) | 5.3 | error: compare two table values | `true` | FIDELITY | 5.3 `LUA_COMPAT_LT_LE`: `a<=b` falls back to `not(b<a)` via `__lt`. Removed in 5.4. We don't do the fallback. Central 5.3 metamethod semantics; hit by both events.lua and coroutine.lua. |
| `local a={};for i=1,5 do a[i]=function(x) return x+a+_ENV end end; a[3]==a[4]` | 5.3 | `false false` | `true true` | FIDELITY | 5.3 caches/shares closures with identical upvalue sets so they compare equal; we make distinct closures. 5.4/5.5 ref returns false too (MATCH) — 5.3-only. |
| `tostring=nil; pcall(print,1)` | 5.3 | `true` (prints `1`) | error: attempt to call a nil value | FIDELITY | 5.3 `print` routes through the *global* `tostring`; we use an internal one. 5.4/5.5 MATCH (both don't route) — 5.3-only. |
| `print(math.type("10"))` | 5.3/5.4/5.5 | `false` | `nil` | REGRESSION(5.4)/FIDELITY(5.3,5.5) | `math.type` returns boolean `false` for non-numbers instead of `nil`. Version-independent bug; the 5.4 instance is a divergence from lua5.4.7. |
| `_ENV[true]=10; print(_ENV[1<2])` | 5.3 | error: index a number value | `10` | FIDELITY | indexing `_ENV` with a relational-comparison key mis-evaluates (register clash). 5.4 ref also errors (MATCH-ish), 5.3/5.5 ref return 10. |
| `_ENV[true]=10; print(_ENV[(1<2)])` | 5.5 | error: index a number value | `10` | FIDELITY | same root; also fails with `2>1`, `1==1` keys. Generic `local t; t[1<2]` works — bug is specific to `_ENV` index by comparison result. |
| `global<const> print; print("hi")` | 5.5 | parse error `'*' expected` | `hi` | FIDELITY | 5.5 `global<const>` attributed global declaration unimplemented. (`global x` plain parses OK.) Breaks math/goto/etc. |
| `local function f(...t) return t[1] end; f(9)` | 5.5 | parse error `')' expected near 't'` | `9` | FIDELITY | 5.5 named/table vararg `...t` unimplemented. Breaks vararg/strings. |
| `global function g() return 7 end` | 5.5 | parse error `<name> expected` | strict err `variable 'print' not declared` | FIDELITY | `global function` form unimplemented (both error, different messages). |
| `string.format("%100.3d",10)` | 5.3 | `invalid conversion specification` | `invalid format (width or precision too long)` | FIDELITY | format-error message text. |
| `utf8.offset("abc",1,5)` | 5.3 | `bad argument #3 (position out of bounds)` | `bad argument #3 to 'utf8.offset' (position out of range)` | FIDELITY | arg-error omits `to 'fn'` and uses different wording. Pattern repeats across utf8/tpack/db. |
| `string.unpack("c0","\0\0",0)` | 5.3 | `true` (no error) | error: initial position out of string | FIDELITY | `string.unpack` bounds check missing/lenient in 5.3. |
| `goto.lua` `::l3:: ::l3_1::` block | 5.3 | parse: label 'l3' already defined on line 50 | OK | FIDELITY | label parser falsely reports duplicate `l3` (confuses `l3` with `l3_1` / earlier scope). |

## Noise (helper did not / cannot normalize)

| snippet/file | class | note |
|---|---|---|
| math.lua `random seeds:` lines | NOISE | unseeded RNG. |
| sort.lua `... msec` / comparison counts | NOISE | timing + RNG-dependent element order. |
| constructs.lua `short-circuit optimizations (1/0)` | NOISE | `_ENV.GLOB1 = math.random(0,1)`. |
| literals.lua `pt_BR locale not available` | NOISE | locale presence differs by host build. |
| `_ENV[(1<2)]` traceback tail `[C]: in ?` | NOISE-ish | 5.4/5.5 ref appends `[C]: in ?` to tracebacks from `-e`; ours omits it. Pure traceback-format, not behavior. |
