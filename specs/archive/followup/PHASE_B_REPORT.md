# Phase B Report — finishing Lua 5.3 (issue #19)

Branch: `mv-5.3-finish` (off `main`, v0.0.20). Oracle: unmodified `make macosx`
`/tmp/lua-refs/bin/lua5.3.6` (with `lua5.4.7` / `lua5.5.0` as cross-version
non-regression refs). Method and preamble per the engine contract in
`specs/oracle/CONTRACT.md`.

This phase closed the clear-cut 5.3 long-tail categories surfaced by the Phase B
discover sweep (`specs/followup/5.3-divergences.md`, `5.3-math.md`,
`5.3-coerce-err.md`): the LUA_COMPAT_MATHLIB roster, string→integer coercion in
core bitwise ops, and 5.3-specific arithmetic/`for`-loop error wording. It also
caught and fixed a self-inflicted regression in the arith-wording change.

---

## What landed (with gate results)

Four commits on `mv-5.3-finish`:

| Commit | Category | Summary |
|---|---|---|
| `8d995be` | compat-math | LUA_COMPAT_MATHLIB roster: `atan2/cosh/sinh/tanh/pow/log10` (5.3/5.4) + `frexp/ldexp` (5.3/5.4/5.5), version-gated. |
| `0f8fe7d` | string-coercion-in-bitwise | 5.3 coerces numeric strings to integers in core `& \| ~ << >>` (and unary `~`); 5.4/5.5 keep erroring. |
| `7984643` | error-wording | 5.3 arith-on-string → `attempt to perform arithmetic on a <type> value (<varinfo>)`; 5.3 `for`-bound → `'for' <what> must be a number`. |
| *(this phase)* | error-wording fix | Made the arith-on-string intercept metamethod-aware (see "Regression caught" below). |

**Gate — all green after the fixes:**

| Battery | Result |
|---|---|
| `cargo build --workspace` | green |
| `cargo test --workspace --features lua-rs-runtime/derive` | 0 failures |
| `multiversion_oracle.rs` | 29 passed, 0 failed (was 20 at end of Phase A) |
| `check.sh 5.3` | 23 passed, 0 failed |
| `check.sh 5.4` | 7 passed, 0 failed (no regression) |
| `check.sh 5.5` | 10 passed, 0 failed (no regression) |

5.4/5.5 confirmed unaffected: the compat-math roster stays absent where it must
(`atan2` etc. nil on 5.5; all 8 nil-cross-checked), bitwise string coercion is
5.3-gated (`"3" & 5` still errors on 5.4/5.5), and the arith/`for` wording is
version-gated (5.4/5.5 keep `attempt to add a 'string' with a 'number'` and `bad
'for' limit (number expected, got string)`).

### Regression caught and fixed this phase

The Phase A arith-wording guard short-circuited to the 5.3 core error whenever an
arith op had a string operand that did not coerce — **before** checking whether
the other operand carried a genuine arith metamethod. This broke
`events.lua:139` (`b + '5'` where `b` is a table with `__add`): the reference
dispatches to `__add` and returns the table; we raised "perform arithmetic on a
table value". `events.lua` had been byte-identical before Phase A and would have
regressed to a hard failure.

Fix (`crates/lua-vm/src/tagmethods.rs`, `try_bin_tm`): the 5.3 intercept now
fires only when the non-string operand has **no** real arith metamethod
(`get_tm_by_obj`), and treats unary minus (`p1 == p2`) as having no "other
operand" so `-"x"` still takes the core path. Verified vs oracle:

- `t + "5"` / `"5" + t` (t has `__add`) → `42` on 5.3/5.4/5.5 (MATCH).
- `"abc" + 1`, `"abc" * 2`, `-"x"`, `local x="a"; x+1`, `aaa="z"; aaa+1`,
  `"2" * nil` → exact 5.3 core wording + correct varinfo (text MATCH; only the
  universal trailing `[C]: in ?` traceback frame differs — pre-existing noise).
- `"3" + 2`, `math.type("1"+"2")`, `-"5"` → success path preserved (MATCH).

CI regression guard added to `v53_arith_string_error_wording` (both operand
orders, plus the 5.4/5.5 dispatch case).

---

## Divergences: before vs after (official 5.3 suite, lua-rs 5.3 vs lua5.3.6)

27 files (drivers `all.lua`/`main.lua` excluded). "Byte-identical" = normalized
stdout+stderr+exit code match (heap addresses and benchmark msec normalized).

| Category (from discover) | Before | After | Notes |
|---|---|---|---|
| string-coercion-in-bitwise | 2 (bitwise, constructs) | **0** | RESOLVED — both files now byte-identical. |
| error-wording (arith/for) | folded into errors.lua | partially resolved | errors.lua clears the line-100 arith blocker and the line-287 `for` blocker; now stops at the line-298 call-line-attribution issue (shared-core, affects 5.4 too). |
| compat-math | 0 files broke in suite | 0 | math roster was an API-presence gap, not a suite-file failure; closed + cross-version verified. |
| (regression risk) events.lua | 0 | **0** | would have regressed to a hard failure; caught and fixed. |

**Byte-identical file count: 11 → 12** (+ `sort.lua` effectively identical, msec
noise only → 12 → 13 effective). The two that flipped to OK are `bitwise.lua`
and `constructs.lua` (the string-in-bitwise category). `events.lua` held at OK.

After-fix byte-identical set (12): api, attrib, big, **bitwise**, code,
**constructs**, events, locals, nextvar, pm, vararg, verybig. Plus sort (noise).

### Remaining DIFF files and their (unchanged) categories

| File | First divergence | Category | Scope note |
|---|---|---|---|
| errors.lua | :298 call-expression-split line attribution | line-info | shared-core; 5.4 also wrong; not 5.3-specific. |
| strings.lua | :298 `%100.3d` width/precision-too-long wording | error-wording (argerror) | needs cross-version format-spec error rework + missing location prefix. |
| utf8.lua | :117 `utf8.offset` OOB "out of range" + missing `to 'offset'` | error-wording (argerror) | cross-version argerror fn-name plumbing gap. |
| math.lua | :275 trailing `[C]: in ?` traceback frame | traceback | the universal cosmetic frame; only blocker on this file. |
| tpack.lua | :315 `string.unpack("c0", x, 0)` bounds check | stdlib-gap | "initial position out of string". |
| literals.lua | :75 `"\u{110000}"` accepted; ref errors | stdlib-gap (lexer) | accept up to 0x7FFFFFFF, "UTF-8 value too large" above. |
| calls.lua | :29 `print` with `tostring=nil` | stdlib-gap | `print` must call global `tostring` (error if nil). |
| db.lua | :28 line-hook (`sethook(f,"l")`) trace | stdlib-gap (debug) | debug line-hook fidelity. |
| files.lua | :415 file-iterator chunk reload semantics | other | `load(io.lines(file,"L"))()` chunk-name/reload. |
| goto.lua | :50/:71 same label in disjoint/nested blocks | other (label scope) | "already defined" false positive. |
| closure.lua | :48 loop-built closures with identical upvalues not `==` | other (closure caching) | closure equality. |
| coroutine.lua | :599 `__le`-from-`__lt` across a `yield` | other (`__le`/yield) | depends on #78 `__le`-from-`__lt`. |
| gc.lua | :360 erroring `__gc` finalizer not propagated | other (GC finalizer) | finalizer error propagation. |

---

## What remains for full 5.3 parity (prioritized)

1. **`[C]: in ?` trailing traceback frame (#79(d)).** Universal across every
   error path and every version; it is the *sole* blocker on `math.lua` and a
   line-noise contributor on every other error-raising file. The single
   highest-leverage remaining item — one architectural change (run the main
   chunk beneath a synthesized base C frame, the `pmain`-as-C-closure restructure)
   would flip `math.lua` and tighten byte-parity on errors/strings/utf8/literals/
   calls simultaneously. Must land as its own isolated change with a spawn-the-
   binary oracle test, NOT bundled with wording fixes (per Phase A hand-off).
2. **`__le`-from-`__lt` derivation (#78), gated V51–V54.** Unblocks
   `coroutine.lua:599`; it is a genuine value-vs-error divergence in the binding
   compat build. The deferred-for-decision item from Phase A — a localized gated
   branch in `tagmethods.rs::call_order_tm`. Decision still pending (treat the
   compat build as contract vs. decline to mirror a compat shim).
3. **argerror fn-name + location-prefix plumbing (cross-version).** Fixes
   `utf8.offset` and `string.format` width/precision wording on `utf8.lua` /
   `strings.lua` — but it is wrong on 5.4 too, so it is a shared-core argerror
   fix, not a 5.3 wording swap.
4. **stdlib gaps**: `string.unpack` `c0` bounds (`tpack.lua`), `\u{}` upper-bound
   in the lexer (`literals.lua`), `print`→global-`tostring` (`calls.lua`), debug
   line-hook fidelity (`db.lua`). Each is a contained, independent fix.
5. **other**: goto label scoping in disjoint/nested blocks, loop-closure equality
   caching, GC `__gc` finalizer error propagation. Larger, more localized to
   their subsystems.

---

## Updated 5.3 oracle-battery count

- `check.sh 5.3`: **23 passed, 0 failed** (unchanged count — the battery was
  already green; the suite-sweep parity is the moving metric).
- `multiversion_oracle.rs`: **29 tests passing** (was 20 at end of Phase A;
  +compat-math, +bitwise-coercion, +arith/for wording, +the metamethod-dispatch
  regression guard).
- Official 5.3 suite sweep: **12/27 byte-identical** (13 effective incl. sort),
  up from 11 (12 effective) before the Phase B fixes.
