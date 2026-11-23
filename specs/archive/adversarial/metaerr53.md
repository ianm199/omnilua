# Adversarial findings: metaerr53 (5.3 metamethods + error wording vs lua5.3.6)

Black-box differential testing via `specs/oracle/diff_one.sh 5.3 '<code>'`.
All DIFFs below were reproduced with the helper. `<loc>` = `(command line):1:`.
Messages shown are post-normalization (PROG / ADDR substituted).

## Summary
- Cases run: 84
- MATCH: 47
- DIFF: 37 (all FIDELITY — 5.3-vs-lua5.3.6 gaps; no 5.4 regressions tested here)

## Distinct bug clusters (root causes)
1. **String/coerce-fail arithmetic uses 5.4 metamethod-path wording.** Ours emits
   `attempt to <op> a 'string' with a '<t>'` (operator name, no location); 5.3 emits
   `attempt to perform arithmetic on a string value`. Also misidentifies the *other*
   operand type (e.g. reports `'function'` for a number/nil operand).
2. **Concat / length / arith-on-string errors omit the `<loc>` prefix AND the name
   annotation** (`(local 'x')`, `(global 'g')`, `(field 'a')`, `(upvalue 'up')`,
   `(constant 'x')`). Index/call errors DO carry these correctly except upvalue-index.
3. **`bad argument` errors omit `<loc>` prefix and the `to '<fn>'` clause**, and differ
   in the parenthetical (missing `'badopt'`; `got number` vs none; `got nil` vs `no value`).
4. **`__lt`-emulates-`__le` not implemented** — `a<=b`/`a>=b` with only `__lt` raises
   "attempt to compare two table values" instead of evaluating `not (b<a)`.
5. **`__ipairs` metamethod not honored** — `ipairs` iterates the raw array instead of
   calling `__ipairs`.
6. **Uncaught-error traceback omits trailing `[C]: in ?`** frame (universal under `-e`).

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `"x"+1` (pcall) | `attempt to add a 'string' with a 'number'` | `<loc> attempt to perform arithmetic on a string value` | FIDELITY | cluster 1 — central; affects all string arith |
| `"x"-1` | `attempt to sub a 'string' with a 'number'` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `"x"*1` | `attempt to mul a 'string'...` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `"x"/1` | `attempt to div a 'string'...` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `"x"%1` | `attempt to mod a 'string'...` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `"x"^1` | `attempt to pow a 'string'...` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `"x"//1` | `attempt to idiv a 'string'...` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `1 - "y"` | `attempt to sub a 'string' with a 'function'` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 + wrong operand type |
| `1+("s")` | `attempt to add a 'string' with a 'function'` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 + wrong operand type |
| `("s")+1` | `attempt to add a 'string' with a 'number'` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `("s")+nil` | `attempt to add a 'string' with a 'nil'` | `<loc> ...arithmetic on a string value` | FIDELITY | cluster 1 |
| `-"x"` | `attempt to unm a 'string' with a 'function'` | `<loc> ...arithmetic on a string value (constant 'x')` | FIDELITY | cluster 1; unary, nonsensical 'function' operand |
| `"x" & 1` | `<loc> ...bitwise operation on a string value (constant 'x')` | `<loc> ...bitwise operation on a string value` | FIDELITY | spurious `(constant 'x')` annotation (5.4-style) |
| `"a" \| "b"` | `<loc> ...bitwise operation on a string value (constant 'a')` | `<loc> ...bitwise operation on a string value` | FIDELITY | spurious annotation |
| `nil.."x"` (uncaught) | `attempt to concatenate a nil value` | `<loc> attempt to concatenate a nil value` `... [C]: in ?` | FIDELITY | cluster 2 + 6 |
| `nil .. nil` (pcall) | `attempt to concatenate a nil value` | `<loc> attempt to concatenate a nil value` | FIDELITY | cluster 2 (no loc prefix) |
| `1 .. {}` (pcall) | `attempt to concatenate a table value` | `<loc> attempt to concatenate a table value` | FIDELITY | cluster 2 |
| `local x; x.."!"` | `attempt to concatenate a nil value` | `<loc> ...nil value (local 'x')` | FIDELITY | cluster 2 — missing loc + name |
| `undefinedg.."!"` | `attempt to concatenate a nil value` | `<loc> ...nil value (global 'undefinedg')` | FIDELITY | cluster 2 |
| `up.."!"` (upvalue) | `attempt to concatenate a nil value` | `<loc> ...nil value (upvalue 'up')` | FIDELITY | cluster 2 |
| `#nil` (pcall) | `attempt to get length of a nil value` | `<loc> attempt to get length of a nil value` | FIDELITY | cluster 2 |
| `#print` (pcall) | `attempt to get length of a function value` | `<loc> ...function value (global 'print')` | FIDELITY | cluster 2 |
| `local t={a=nil}; #t.a` | `attempt to get length of a nil value` | `<loc> ...nil value (field 'a')` | FIDELITY | cluster 2 |
| `#up` (upvalue) | `attempt to get length of a nil value` | `<loc> ...nil value (upvalue 'up')` | FIDELITY | cluster 2 |
| `up.x` (upvalue index) | `attempt to index a nil value` | `<loc> ...nil value (upvalue 'up')` | FIDELITY | cluster 2 — only upvalue-index affected; local/global match |
| `select(-5)` | `bad argument #1 (index out of range)` | `<loc> bad argument #1 to 'select' (index out of range)` | FIDELITY | cluster 3 |
| `string.sub()` | `bad argument #1 to 'sub' (string expected, got nil)` | `<loc> ...(string expected, got no value)` | FIDELITY | cluster 3 — nil vs no value |
| `table.insert({},1,2,3)` | `wrong number of arguments to 'insert'` | `<loc> wrong number of arguments to 'insert'` | FIDELITY | cluster 3 (loc prefix) |
| `ipairs()` | `bad argument #1 (value expected)` | `<loc> bad argument #1 to 'ipairs' (value expected)` | FIDELITY | cluster 3 |
| `math.max()` | `bad argument #1 (value expected)` | `<loc> bad argument #1 to 'max' (value expected)` | FIDELITY | cluster 3 |
| `string.char(-1)` | `bad argument #1 (value out of range)` | `<loc> bad argument #1 to 'char' (value out of range)` | FIDELITY | cluster 3 |
| `string.char(256)` | `bad argument #1 (value out of range)` | `<loc> bad argument #1 to 'char' (value out of range)` | FIDELITY | cluster 3 |
| `rawlen(5)` | `bad argument #1 (table or string expected, got number)` | `<loc> ...'rawlen' (table or string expected)` | FIDELITY | cluster 3 — extra `got number` |
| `collectgarbage("badopt")` | `bad argument #1 (invalid option)` | `<loc> ...'collectgarbage' (invalid option 'badopt')` | FIDELITY | cluster 3 — missing option value |
| `tonumber("10",37)` | `bad argument #2 (base out of range)` | `<loc> bad argument #2 to 'tonumber' (base out of range)` | FIDELITY | cluster 3 |
| `string.format("%d",3.5)` | `bad argument #2 (number has no integer representation)` | `<loc> ...'format' (number has no integer representation)` | FIDELITY | cluster 3 |
| `__lt-only: a<=b` | `<loc> attempt to compare two table values` (raises) | `false` | FIDELITY | cluster 4 — CENTRAL 5.3 feature missing |
| `__lt-only: a<=a` | raises compare error | `true`/`false` per `not(b<a)` | FIDELITY | cluster 4 |
| `__lt-only: a>=a` | raises compare error | `false` | FIDELITY | cluster 4 |
| `__lt-only: pcall(a<=b)` | `false  <loc> attempt to compare two table values` | `true false` | FIDELITY | cluster 4 |
| `__ipairs iterator` | `1 10 / 2 20` (raw) | `1 1000 / 2 2000` (metamethod) | FIDELITY | cluster 5 |
| `error("boom")` (uncaught) | traceback ends `...in main chunk` | `...in main chunk` `[C]: in ?` | FIDELITY | cluster 6 — universal traceback tail |
| `nil+1` (uncaught) | no trailing `[C]: in ?` | has `[C]: in ?` | FIDELITY | cluster 6 |

## MATCHed (verified faithful — not bugs)
`nil+1`/`1+nil`/`true+1`/`{}+1`/`-nil`/`-{}` arith wording (non-string operands, w/ loc);
all comparison-of-incompatible-types wording (`1<"x"`, `{}<{}`, `nil<nil`, etc.);
bitwise metamethods (`__band`/`__bnot`/`__shl`/`__idiv`); `__gc` non-function ignored;
explicit `__le`; `__lt` emulation of `__eq`-path n/a; float-no-int-rep bitwise
(`3.5|1`); `tostring` of nil/bool/int/float/inf/nan/-0.0; `type()` names;
`math.maxinteger`/`mininteger`; `error()` variants; `assert` variants; index/call
name annotations for local/global/field/method; `string.format("%d",3.0)`; many more.

## Notes for triage
- Clusters 1–3 are wording/location/name-annotation regressions that look like the
  **5.4 error-message machinery leaking into the 5.3 backend** (operator-named arith
  messages, `(constant 'x')` on bitwise, missing 5.3-style loc prefix on concat/len/
  badarg). The 5.3 `errors.lua` suite asserts on these exact substrings — high impact.
- Cluster 4 (`__lt`→`__le`) and cluster 5 (`__ipairs`) are documented 5.3 behaviors
  (research delta items 4 and 12) that are simply not wired into the 5.3 backend.
- Cluster 6 (traceback `[C]: in ?`) is universal and deterministic; affects every
  uncaught-error case run from `-e`. Not noise.
