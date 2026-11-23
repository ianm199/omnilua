# Adversarial: num53 — 5.3 number model vs lua5.3.6 (+ cross-version checks)

Black-box differential testing via `specs/oracle/diff_one.sh`. The reference C
binaries are ground truth. Each row was reproduced with the helper (DIFF rows
print DIFF; MATCH rows omitted unless load-bearing for a contrast).

Approx **95 snippets run**: **~78 MATCH**, **~17 DIFF**.

The 5.3 number-model *core* is in excellent shape: int/float subtypes, `//`
floor-div, `%` modulo (incl. sign-of-divisor), `//0` and `%0` int errors,
`/0` float inf/nan, unary minus (incl. `-math.mininteger` wrap), integer/float
equality and `t[2]`/`t[2.0]` table-key identity, decimal-literal overflow→wrap,
hex int/float literals, large/overflow arithmetic, `math.*` subtype results
(`abs/floor/ceil/modf/fmod/max/min/ult`), `tonumber` bases/forms, concat
formatting, and `tostring`/`%d`/`%g`/`%.14g`/`%a`/`%x` formatting all MATCH.

The confirmed divergences cluster in three groups:

1. **`math.type`/`math.tointeger` return boolean `false` instead of `nil`** on
   the no-result path — a real semantic bug spanning ALL versions (REGRESSION).
2. **5.3 string→number coercion in the core arith/bitwise path is missing** — we
   use the 5.4 string-library-metamethod model unconditionally, so 5.3 bitwise on
   strings errors (should compute) and 5.3 arith-on-bad-string emits the 5.4
   metamethod message (FIDELITY).
3. **C-library error-message + traceback formatting** — missing `(command line):N:`
   position prefix, missing `to '<fn>'` name annotation, and missing trailing
   `[C]: in ?` frame. Universal across versions; the trailing-frame one is on
   every error. Not number-specific but surfaced by every numeric error here.

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `print(math.type("1"))` | `false` | `nil` | REGRESSION | non-number → must be nil; also fails 5.4/5.5 |
| `print(math.type({}))` | `false` | `nil` | REGRESSION | all non-number args |
| `print(math.type(nil))` / `(true)` | `false` | `nil` | REGRESSION | |
| `local x=math.type("s"); print(x,type(x))` | `false boolean` | `nil nil` | REGRESSION | value is boolean false, not nil — fails 5.4 too |
| `print(math.tointeger(3.5))` | `false` | `nil` | REGRESSION | non-convertible float |
| `print(math.tointeger(math.huge))` | `false` | `nil` | REGRESSION | |
| `print(math.tointeger(2^63))` | `false` | `nil` | REGRESSION | out of int range |
| `print(math.tointeger(nil))` / `(0/0)` | `false` | `nil` | REGRESSION | |
| `print(math.tointeger(1.5)==nil)` | `false` | `true` | REGRESSION | confirms it's `false` not nil |
| `local x=math.tointeger(1.5); print(x,type(x))` | `false boolean` | `nil nil` | REGRESSION | fails 5.4/5.5 too |
| `print("3" & 5)` | error: bitwise on string value | `1` | FIDELITY | 5.3 coerces strings in core bitwise; 5.4/5.5 ref also errors (correct) |
| `print(math.type("3" & 5))` | error | `integer` | FIDELITY | 5.3-only core coercion |
| `print("0xff" | 0)` | error | `255` | FIDELITY | hex string coerced in 5.3 bitwise |
| `print("3.0" & 5)` | error | `1` | FIDELITY | float-valued string → integer in 5.3 bitwise |
| `print("3.5" & 5)` | error: bitwise on string value | `number has no integer representation` | FIDELITY | 5.3 coerces then rejects non-integral; we reject the string first |
| `print("8" >> "1")` | error | `4` | FIDELITY | both operands string-coerced in 5.3 |
| `print(~"5")` | error | `-6` | FIDELITY | unary bnot string coercion in 5.3 |
| `print("abc"+1)` | `attempt to add a 'string' with a 'number'` / `[C]: in metamethod 'add'` | `attempt to perform arithmetic on a string value` (no metamethod frame) | FIDELITY | 5.3 coerces in core; on failure raises core arith error, NOT the 5.4 string-metamethod message. 5.4/5.5 ref matches our body (only prefix/trailing-frame differ there) |
| `print(string.format("%q", 1/0))` | `1e9999` | `inf` | FIDELITY | 5.3 `%q` emits `inf`/`nan`; 5.4/5.5 ref matches our loadable form (`1e9999`) → we're hardwired to 5.4 `%q` |
| `print(string.format("%q", 0/0))` | `(0/0)` | `nan` | FIDELITY | 5.3-only; 5.4/5.5 MATCH |
| `print(string.format("%q", -1/0))` | `-1e9999` | `-inf` | FIDELITY | 5.3-only |
| `print(1//0)` | `divide by zero` (no `[C]: in ?`) | same + `[C]: in ?` | NOISE-adjacent | universal trailing-frame gap, all versions |
| `print(1%0)` / `print(0%0)` | `attempt to perform 'n%0'` (no `[C]: in ?`) | same + `[C]: in ?` | NOISE-adjacent | universal |
| `print(3.5 & 1)` / `print(1<<2.5)` | `number has no integer representation` (no `[C]: in ?`) | same + `[C]: in ?` | NOISE-adjacent | universal trailing frame |
| `error("x")` | `...x` traceback w/o final `[C]: in ?` | same + `[C]: in ?` | NOISE-adjacent | universal; not number-specific |
| `print(string.format("%d", 3.5))` | `bad argument #2 (number has no integer representation)` | `(command line):1: bad argument #2 to 'format' (...)` + `[C]: in ?` | REGRESSION | missing position prefix AND `to 'format'` name; universal 5.3/5.4/5.5 |
| `print(string.format("%d", 2^63))` | `bad argument #2 (...)` | `(command line):1: bad argument #2 to 'format' (...)` | REGRESSION | same C-arg-error formatting gap |
| `print(math.type())` | `bad argument #1 (value expected)` (no prefix/name) | `(command line):1: bad argument #1 to 'type' (value expected)` | REGRESSION | same gap on math.type |

## Top 5 confirmed divergences (with repro)

1. **`math.type`/`math.tointeger` return boolean `false`, not `nil`** (REGRESSION,
   all versions). The no-result path yields an actual boolean.
   `LUA_RS_VERSION=5.3 lua-rs -e 'local x=math.type("s"); print(x,type(x))'`
   → `false boolean`; ref → `nil nil`. Also fails 5.4.7 and 5.5.0. High impact:
   any `if math.tointeger(x) then` / `== nil` check behaves wrong, and this is a
   core 5.4 regression too.

2. **5.3 bitwise ops do not coerce strings in the core** (FIDELITY, 5.3-central).
   `LUA_RS_VERSION=5.3 lua-rs -e 'print("3" & 5)'` → error "bitwise operation on a
   string value"; lua5.3.6 → `1`. Same for `"0xff"|0`→255, `"8">>"1"`→4, `~"5"`→-6.
   Correctly version-gated downward (5.4/5.5 ref also errors), so this is a missing
   5.3-only core-coercion path, exactly the documented §1a / delta-#3 behavior.

3. **5.3 arithmetic-on-non-coercible-string uses the 5.4 metamethod error**
   (FIDELITY, 5.3). `LUA_RS_VERSION=5.3 lua-rs -e 'print("abc"+1)'` →
   `attempt to add a 'string' with a 'number'` with a `[C]: in metamethod 'add'`
   frame; lua5.3.6 → `attempt to perform arithmetic on a string value` with NO
   metamethod frame (core coercion failed). 5.4/5.5 ref bodies match ours, so this
   is specifically the 5.3 core-coercion path being absent.

4. **`string.format("%q", inf/nan)` emits the 5.4 loadable form under 5.3**
   (FIDELITY, 5.3). `LUA_RS_VERSION=5.3 lua-rs -e 'print(string.format("%q",1/0))'`
   → `1e9999`; lua5.3.6 → `inf`. `0/0`→`(0/0)` vs `nan`; `-1/0`→`-1e9999` vs `-inf`.
   5.4/5.5 MATCH, confirming `%q` is hardwired to the 5.4 representation.

5. **C-library bad-argument errors omit the position prefix and function name**
   (REGRESSION, all versions). `LUA_RS_VERSION=5.3 lua-rs -e 'print(string.format("%d",3.5))'`
   → `bad argument #2 (number has no integer representation)`; ref →
   `(command line):1: bad argument #2 to 'format' (number has no integer representation)`.
   Affects `string.format`, `math.type`, etc. Separately, every error traceback is
   missing the final `[C]: in ?` frame (universal; classify as low-severity
   traceback-fidelity noise, but it is a genuine reproducible DIFF the helper does
   not normalize).
