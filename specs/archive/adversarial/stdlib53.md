# Adversarial: stdlib53 (Lua 5.3 stdlib vs lua5.3.6)

Category: 5.3 stdlib differential. Black-box: cases derived from the 5.3 manual,
`specs/research/5.3-upstream-delta.md`, and by probing `/tmp/lua-refs/bin/lua5.3.6`.
All DIFFs reproduced with `specs/oracle/diff_one.sh 5.3 '<code>'`.

Totals: 150 cases run, 114 MATCH, 36 DIFF.

## Divergence summary by family

| family | count | class |
|---|---|---|
| stdlib C-error message missing `to 'fn'` + traceback `[C]: in ?` tail | 14 | FIDELITY |
| `math.atan2/cosh/sinh/tanh/pow/log10/ldexp/frexp` absent (compat-on funcs) | 15 | FIDELITY |
| `math.type/tointeger` return `false` instead of `nil` | 3 | FIDELITY |
| string-in-bitwise-core coercion (`"3" & 5`) not done | 1 | FIDELITY |
| `__le` derived from `__lt` not reinstated | 1 | FIDELITY |
| `collectgarbage("setpause")` wrong return + `generational`/`incremental` accepted | 3 | FIDELITY |

No REGRESSION (this is a 5.3 category; nothing here touches 5.4). No NOISE.

## Confirmed DIFFs (snippet | ours | ref | class | note)

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `math.atan2(1,1)` | err nil field 'atan2' | `0.78539816339745` | FIDELITY | central: whole compat-math family missing |
| `math.cosh(0),math.sinh(0),math.tanh(0)` | err nil field 'cosh' | `1.0  0.0  0.0` | FIDELITY | compat-on default in 5.3 |
| `math.pow(2,10)` | err nil field 'pow' | `1024.0` | FIDELITY | |
| `math.log10(1000)` | err nil field 'log10' | `3.0` | FIDELITY | |
| `math.ldexp(1,4)` | err nil field 'ldexp' | `16.0` | FIDELITY | |
| `math.frexp(8)` | err nil field 'frexp' | `0.5  4` | FIDELITY | |
| `math.atan2(0,-1)` | err | `3.1415926535898` | FIDELITY | |
| `math.cosh(1)` | err | `1.5430806348152` | FIDELITY | |
| `math.sinh(1)` | err | `1.1752011936438` | FIDELITY | |
| `math.tanh(100)` | err | `1.0` | FIDELITY | |
| `math.pow(2,0.5)` | err | `1.4142135623731` | FIDELITY | |
| `math.log10(0)` | err | `-inf` | FIDELITY | |
| `math.ldexp(0.5,3)` | err | `4.0` | FIDELITY | |
| `math.frexp(0)` | err | `0.0  0` | FIDELITY | |
| `math.type(3),math.type(3.0),math.type("3")` | `integer float false` | `integer float nil` | FIDELITY | 3rd arg: should be nil not false |
| `math.tointeger(3.0),math.tointeger(3.5)` | `3 false` | `3 nil` | FIDELITY | non-int returns nil not false |
| `math.tointeger(2^63)` | `false` | `nil` | FIDELITY | |
| `"3" & 5` | err bitwise on string | `1` | FIDELITY | 5.3 coerces strings in core bitwise ops |
| `local a=setmetatable({},{__lt=function() return true end}); a<=a` | err compare two tables | `false` | FIDELITY | 5.3 derives `__le` from `__lt` (a<=b = not(b<a)) |
| `collectgarbage("setpause",200)` | `50` | `200` | FIDELITY | should return PREVIOUS pause value |
| `pcall(collectgarbage,"generational")` | `true incremental` | `false bad argument #1 ... invalid option 'generational'` | FIDELITY | 5.3 has no generational option |
| `pcall(collectgarbage,"incremental")` | `true incremental` | `false bad argument #1 ... invalid option 'incremental'` | FIDELITY | 5.3 has no incremental option |
| `bit32.band(3.5)` | `bad argument #1 (number has no integer representation)` | `... #1 to 'band' (...)` | FIDELITY | msg missing `to 'band'`; traceback missing `[C]: in ?` |
| `bit32.band(0/0)` | (same shape) | (same) | FIDELITY | same message-format gap |
| `bit32.extract(0,-1)` | `bad argument #2 (field cannot be negative)` | `... #2 to 'extract' (...)` | FIDELITY | message format |
| `bit32.extract(0,32)` | `bad argument #2 (trying to access non-existent bits)` | `(command line):1: trying to access non-existent bits` | FIDELITY | ours over-decorates as bad-arg; ref is plain msg + lacks `[C]: in ?` |
| `bit32.extract(0,0,33)` | `bad argument #2 (trying to access non-existent bits)` | `trying to access non-existent bits` | FIDELITY | same |
| `bit32.extract(0,0,0)` | `bad argument #3 (width must be positive)` | `... #3 to 'extract' (...)` | FIDELITY | message format |
| `bit32.extract(0,30,3)` | `bad argument #2 (...)` | `trying to access non-existent bits` | FIDELITY | |
| `bit32.extract(0,31,2)` | `bad argument #2 (...)` | `trying to access non-existent bits` | FIDELITY | |
| `bit32.replace(0,1,32)` | `bad argument #3 (...)` | `trying to access non-existent bits` | FIDELITY | |
| `bit32.replace(0,1,0,0)` | `bad argument #4 (width must be positive)` | `... #4 to 'replace' (...)` | FIDELITY | message format |
| `bit32.replace(0,1,31,2)` | `bad argument #3 (...)` | `trying to access non-existent bits` | FIDELITY | |
| `string.packsize("s4")` | `bad argument #1 (variable-length format)` | `... #1 to 'packsize' (...)` | FIDELITY | message format |
| `utf8.codepoint("\xF4\x90\x80\x80")` | `invalid UTF-8 code` | `(command line):1: invalid UTF-8 code` + `[C]: in ?` | FIDELITY | missing chunk-location prefix + traceback tail |
| `string.format("%d", 3.5)` | `bad argument #2 (...)` | `... #2 to 'format' (...)` | FIDELITY | message format |

## Notable MATCHes (confirm correctness; not bugs)

bit32 arithmetic, lshift/rshift/arshift boundaries, lrotate/rrotate, extract/replace
valid cases, btest, mod-2^32 reduction, float args with integral value, negative→2^32
wrap, `>2^32` reduction all MATCH. `math.maxinteger/mininteger`, `math.ult`,
`string.pack/unpack/packsize` valid forms, `utf8` valid + surrogate-accept
(`#utf8.char(0xD800)`==3) MATCH. `coroutine.close`/`warn`/`loadstring` presence,
`table.move/pack/unpack`, `9223372036854775808` decimal-overflow-wraps, `"10"+"20"`
integer coercion, `1<<4` all MATCH.

## Top 5 most important confirmed divergences

1. **Whole compat-math family is missing.** `math.atan2/cosh/sinh/tanh/pow/log10/ldexp/frexp`
   all error with "attempt to call a nil value". Stock 5.3 ships these (LUA_COMPAT_MATHLIB
   default ON). This is the single biggest 5.3-fidelity gap — central, not niche.
   Repro: `diff_one.sh 5.3 'print(math.pow(2,10))'`

2. **`__le` is not derived from `__lt` in 5.3.** 5.3 evaluates `a<=b` as `not (b<a)` when
   `__le` is absent but `__lt` exists; ours raises "attempt to compare two table values".
   Repro: `diff_one.sh 5.3 'local a=setmetatable({},{__lt=function() return true end}); print(a<=a)'`
   (ref prints `false`, ours errors).

3. **`collectgarbage` option fidelity.** `setpause` returns a hardcoded `50` (then `200`)
   instead of the *previous* pause value; and `"generational"`/`"incremental"` are silently
   accepted (returning `incremental`) instead of erroring `invalid option`. 5.3 has neither
   mode.
   Repro: `diff_one.sh 5.3 'print(collectgarbage("setpause",200))'` (ours `50`, ref `200`).

4. **Strings not coerced in core bitwise ops.** `"3" & 5` errors in ours ("bitwise operation
   on a string value") but 5.3 coerces in the core and yields `1`.
   Repro: `diff_one.sh 5.3 'print("3" & 5)'`

5. **`math.type`/`math.tointeger` return Lua `false` instead of `nil`** for the
   non-number / non-representable case. `math.type("3")`→`false` (should be `nil`);
   `math.tointeger(3.5)`→`false` (should be `nil`). This breaks any `if math.type(x)==nil`
   guard and is a pervasive truthiness bug.
   Repro: `diff_one.sh 5.3 'print(math.type("3"))'` (ours `false`, ref `nil`).

   (Honorable mention, highest count: all bit32 / packsize / format / utf8 *error* paths drop
   the `to '<fn>'` clause and the `[C]: in ?` traceback tail, and bit32 wraps the "non-existent
   bits" runtime error as a bad-argument error. 14 cases — a systemic C-stdlib error-formatting
   gap rather than a behavior bug.)
