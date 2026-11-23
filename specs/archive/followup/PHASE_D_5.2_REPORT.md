# Phase D — Lua 5.2 bridge: independent verification report

Branch: `finish-5.2-bridge` (commits d262b17 "lift V52" + 0167edd design specs).
Verifier: read-only judge run on 2026-05-31. Oracle = `/tmp/lua-refs/bin/lua5.2.4`
(also lua5.3.6 / lua5.4.7 / lua5.5.0 for no-regression). The official 5.2
conformance suite is not bundled, so the oracle here is differential probing +
the hand-built `check.sh 5.2` battery + the 5.1-test smoke scripts.

## Before / after

- BEFORE this branch: `LuaVersion::V52` existed in `version.rs` with
  `number_model() == FloatOnly` but was EXCLUDED from `is_supported()`;
  `Lua::new_versioned(V52)` / `LUA_RS_VERSION=5.2` refused with
  "not yet supported".
- AFTER: V52 is in `is_supported()` (V51 still correctly excluded — fenv is a
  separate phase). The runtime refusal message now reads
  "supported: 5.2, 5.3, 5.4, 5.5". The float-only seams, syntax rejection of
  the 5.3 integer operators, and the 5.2 stdlib roster are all V52-gated.

## Gate results (this run)

| Gate | Result |
|---|---|
| `cargo build -p lua-cli` | GREEN (warnings only) |
| `cargo test --workspace --features lua-rs-runtime/derive` | GREEN — 0 failures across 43 test-result groups; all 6 `v52_*` oracle tests pass |
| `check.sh 5.2` battery | **53 PASS / 1 FAIL** (only `module present`) |
| `check.sh 5.3` | 23 PASS / 0 FAIL (no regression) |
| `check.sh 5.4` | 7 PASS / 0 FAIL (no regression) |
| `check.sh 5.5` | 10 PASS / 0 FAIL (no regression) |

## Differential probing (diff_one.sh 5.2 vs lua5.2.4)

Spread covering float-only, roster, syntax, _ENV, coercion, errors:

MATCH (representative): integer-valued floats print without `.0` (`10/2`→`5`,
`2^2`→`4`, `tostring(1.0)`→`1.0`); float `for` loops; `3/2 7%3 2^10`;
`math.huge`/`-math.huge`/NaN; `1e100`/large literals; `2^53`/`2^63` tostring;
float table keys collapse (`t[1.0]` reachable via `t[1]`); `pairs` key types;
`string.format` `%d`/`%5.2f`/`%g`/`%e`/`%d`-truncation/`%d` strcoerce; `#`,
`upper`, `rep`; `_ENV == _G`, local-`_ENV` shadowing, `_ENV.x`; `select`;
string→float coercion (`"10"*"2"`, `"3.5"+0`, `10 .. 20`); `tonumber` with
base/hexfloat; `rawlen`/`rawequal`; `xpcall` with args; `bit32` full surface
(band/bor/bnot/lshift/rshift/extract/btest/lrotate); `utf8`/`table.move`/`warn`/
`coroutine.close`/`table.create` absent; `getfenv`/`setfenv` absent; `goto`;
all six 5.3 integer operators (`// & | << >> ~` binary+unary) correctly
rejected at parse; `<const>` rejected; `package.loaders`/`package.searchers`
present; deprecated math compat (`atan2`/`cosh`/`pow`/`log10`/`frexp`) present.

DIFF found — two classes, both pre-documented:

1. **`module` system (gap #1, documented).** `type(module)` → `nil` (ref:
   `function`); `type(package.seeall)` → `nil` (ref: `function`). The deprecated
   5.2 `module(...)`/`package.seeall` definition helpers are unimplemented.
   `require` itself is present and correct. This is the single `check.sh 5.2`
   failure.

2. **Error-message word order (gap #2, documented — cross-version, not a 5.2
   regression).** lua-rs emits the unified 5.3+ phrasing for several runtime
   errors where 5.2.4 used a different word order. Confirmed against the
   reference that 5.2 and 5.3 genuinely differ here, and that lua-rs matches
   5.3/5.4 — so this is one error-format seam rendering everything in modern
   style, not broken 5.2 behavior:

   | case | lua-rs (5.3+ style) | lua5.2.4 |
   |---|---|---|
   | index nil local | `attempt to index a nil value (local 't')` | `attempt to index local 't' (a nil value)` |
   | call nil local | `attempt to call a nil value (local 'x')` | `attempt to call local 'x' (a nil value)` |
   | call nil method | `attempt to call a nil value (method 'bad')` | `attempt to call method 'bad' (a nil value)` |
   | call nil field/global | `... a nil value (field/global 'X')` | `... field/global 'X' (a nil value)` |
   | `error({table})` | `(error object is a table value)` | `(no error message)` |
   | `pairs(nil)` | `bad argument #1 to 'for iterator' ... [C]: in function 'next'` | `bad argument #1 to 'pairs' ...` |

   The documented gap #2 names only the "attempt to call" variant; the index /
   `error({table})` / `pairs` attribution variants I found are the same seam and
   should be folded into that accounting if it is ever fixed.

## Smoke (5.1-test scripts under LUA_RS_VERSION=5.2 vs lua5.2.4)

13 scripts run. 9 full MATCH: factorial, fibfor, sort, table, life, printf,
hello, bisect, globals. 4 DIFF, all explained:
- `fib` — clock/timing output only (non-deterministic; behavior identical).
- `sieve`, `env`, `readonly` — same gap #2 error word order. Underlying
  behavior is correct for 5.2 (`getfenv` absent → these scripts are *expected*
  to error; only the message wording differs).

## Honesty assessment

V52 was lifted, and the implementer's `5.2-syntax-roster.md` "Known gaps"
section documents exactly the two gap classes I independently reproduced. The
gaps are real, narrow, and correctly characterized as (1) a dead deprecated
feature left unimplemented rather than stubbed, and (2) a pre-existing
cross-version error-message-format difference that is byte-identical to the C
reference on 5.4 too. No 5.2 behavior leaks into the modern versions (all
V52-gated; 5.3/5.4/5.5 batteries clean). One doc nit: the doc cites
`check.sh 5.2` as 49/50; the battery has since been expanded to 54 cases and is
now 53/1, but the single failure is still `module`, so the claim holds in spirit.

## VERDICT

**PASS** — Lua 5.2 is faithful enough to mark supported, with no
modern-version regression. The two remaining DIFFs (`module` system; 5.2-style
error-message word order) are real, documented, narrow, and non-behavioral
for the float-only number model, _ENV globals, syntax rejection, and stdlib
roster that define the 5.2 bridge. Recommend keeping the two documented gaps
visible (and expanding gap #2 to list the index / `error(table)` / `pairs`
variants); they do not warrant withholding "supported" status.
