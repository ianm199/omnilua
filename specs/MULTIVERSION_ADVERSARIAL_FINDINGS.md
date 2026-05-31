# Multiversion adversarial findings (5.3 / 5.4 / 5.5 vs reference C binaries)

Synthesis of all per-category hunts in `specs/adversarial/*.md`, cross-checked
against the self-review `specs/MULTIVERSION_PRELIM_REVIEW.md`. Every divergence
cited as confirmed below was reproduced through
`specs/oracle/diff_one.sh <ver> '<code>'` against the pinned reference binaries
(`/tmp/lua-refs/bin/lua5.3.6`, `lua5.4.7`, `lua5.5.0`). The helper normalizes
program-path and heap-address noise; exit codes are compared too.

A spot re-verification of the most consequential claims (all seven 5.4
regressions, the 5.5 lexer panic, 5.5 block-scope/initializer/for-const,
5.5 float round-trip, 5.3 compat-math, 5.3 bitwise coercion) was run directly
and all reproduced — see "Verification note" at the end.

---

## 1. Executive summary

**Total cases run across all categories: ~1,150 distinct snippets** (the largest
single batch, `fuzz_misc`, is ~310 snippets × 3 versions ≈ 928 version-cases; the
counts below are distinct-snippet counts per category).

| Category | Version(s) | Run | MATCH | DIFF |
|---|---|---|---|---|
| reg54_core | 5.4 | 268 | 241 | 27 |
| reg54_lang | 5.4 | 112 | ~91 | 21 |
| num53 | 5.3 | ~95 | ~78 | ~17 |
| metaerr53 | 5.3 | 84 | 47 | 37 |
| stdlib53 | 5.3 | 150 | 114 | 36 |
| globals55_scope | 5.5 | 60 | 41 | 19 |
| globals55_const | 5.5 | 53 | 24 | 29 |
| other55 | 5.5 | 64 | 18 | 46 |
| suite_slices | 5.3/5.4/5.5 | ~57 files+snippets | 5.4 clean | 5.3/5.5 fidelity |
| fuzz_misc | 5.3/5.4/5.5 | ~310 (≈928 cases) | 811 | 117 |

**Approximate MATCH rate per version** (distinct-snippet basis, error-formatting
families counted once per occurrence):

- **5.4: ~88–90% MATCH.** The numeric/arithmetic/closure/metamethod/coroutine
  *core* is clean. All 5.4 DIFFs collapse into ~5 root causes, almost all in the
  error-message / value-representation layer — plus two genuine behavioral bugs
  (`string.find` extra return, `__le`-from-`__lt`) and one wrong-value bug
  (`math.type`/`math.tointeger` return `false`).
- **5.3: ~75–80% MATCH.** Number-model core is excellent; gaps are the missing
  compat-math family, missing core string coercion in arith/bitwise, `__le`-from
  -`__lt`, and the (shared) error-formatting family.
- **5.5: ~50–65% MATCH** and the lowest. The headline 5.5 features are
  partially or incorrectly implemented: block-scoping of `global` is wrong,
  `global` initializers are dropped, the for-loop control variable is not
  read-only, float `tostring` doesn't round-trip, `global` cannot be used as an
  identifier (and *panics*), and several new APIs (`...t` varargs,
  `utf8.offset` 2nd return, `collectgarbage("param")`) are absent.

**Confirmed divergences by class:**

- **REGRESSION (5.4 differs from lua5.4.7): YES — confirmed, multiple.**
  Distinct root causes: ~7 families (see §2). These are cross-version bugs that
  happen to also fire under 5.4; the multiversion work did not *newly* break the
  numeric core, but it shipped with 5.4 in a state that diverges from the 5.4
  reference on error formatting, two metamethod/library behaviors, and one
  wrong-value bug.
- **FIDELITY (5.3/5.5 version-specific gaps):** large, dominated by a handful of
  central gaps (§3) plus a long tail of error-wording mismatches.
- **NOISE:** essentially none in the curated snippet sets (RNG/timing/locale only
  in the whole-file `suite_slices` sweep, correctly quarantined there). The
  pervasive missing `[C]: in ?` traceback frame is **deterministic, not noise** —
  it is a real reproducible DIFF on every uncaught error in every version.

---

## 2. REGRESSIONS first (5.4 divergence from lua5.4.7) — most serious

The honest verdict: **the 5.4 numeric/language core was NOT regressed** — integer/
float arithmetic, `//`/`%`/`^`, overflow wrap, bitwise, comparisons, all `math.*`
numeric results, `string.format` conversions, `string.pack`/`unpack`, closures,
upvalues, every binary/unary metamethod dispatch, pcall/xpcall/coroutine shapes,
`<const>`/`<close>`, goto, varargs, and OOP dispatch all MATCH lua5.4.7. The
prelim review's "5.4 parity was proven" claim holds *for the core*.

**However, 5.4 is not byte-clean against its own reference.** The confirmed 5.4
regressions, by severity:

### R-A (value-correctness) — `string.find` returns a spurious extra empty value
A pattern containing magic chars but no explicit captures yields a third trailing
empty-string return.
```
diff_one.sh 5.4 'print(string.find("hello","l+"))'
  OURS: 3  4  (trailing empty 3rd value; select('#')==3)
  REF : 3  4
```
This is a real value bug, version-independent, and the most dangerous 5.4
regression because it silently corrupts return arity that programs branch on.

### R-B (wrong value) — `math.type` / `math.tointeger` return boolean `false`, not `nil`
```
diff_one.sh 5.4 'local x=math.type("3"); print(x, type(x))'
  OURS: false  boolean
  REF : nil    nil
diff_one.sh 5.4 'print(math.tointeger(3.5))'   # OURS false / REF nil
```
The manual specifies these return a `fail` (nil). `false ~= nil` breaks
`== nil` guards and prints differently. Cross-version (also wrong in 5.3/5.5).

### R-C (behavioral) — `__le` not derived from `__lt`
This reference `lua5.4.7` binary (built with `LUA_COMPAT_LT_LE`) evaluates
`a<=b` as `not (b<a)` when `__le` is absent but `__lt` exists. Ours raises.
```
diff_one.sh 5.4 'local a=setmetatable({},{__lt=function() return true end}); print(a<=a)'
  OURS: (command line):1: attempt to compare two table values   (raises, exit 1)
  REF : false                                                   (exit 0)
```
NB: standard 5.4 *removed* `LUA_COMPAT_LT_LE`; this divergence exists because the
pinned reference binary was compiled with that compat flag ON. Real against this
oracle, but its severity depends on whether we treat the compat-built binary as
the contract (see §5).

### R-D (content bug) — arith/unary error reports wrong 2nd-operand type
On the string-coercion-failure metamethod path, the *second* operand's type is
mislabeled (`'function'` instead of its real type); unary `unm` invents a bogus
`'function'` second operand; `{}-"y"` swaps/garbles the leading type.
```
diff_one.sh 5.4 'return -"x"'
  OURS: attempt to unm a 'string' with a 'function'
  REF : (command line):1: attempt to unm a 'string' with a 'string'
```

### R-E (missing `to '<fn>'` in argument errors) — pervasive
```
diff_one.sh 5.4 'print(pcall(string.char,256))'
  OURS: bad argument #1 (value out of range)
  REF : bad argument #1 to 'string.char' (value out of range)
```
Affects every `luaL_argerror` callsite (string/math/table/select/tonumber/…).
Also: an entirely-absent argument prints `got nil` where the ref prints
`got no value`.

### R-F (missing `(command line):N:` location prefix) — selective
Length (`#`), concatenation (`..`), and the string-arithmetic-coercion failure
path drop the chunk-location prefix. Arith-on-nil, index-nil, call-nil, and
comparison errors *do* carry it — so this is subsystem-specific, not global.
```
diff_one.sh 5.4 'print(pcall(function() return #nil end))'
  OURS: attempt to get length of a nil value
  REF : (command line):1: attempt to get length of a nil value
```

### R-G (traceback tail) — missing trailing `[C]: in ?`
Every uncaught top-level error's traceback omits the final `\t[C]: in ?` frame
that lua5.4.7 prints. Uniform, deterministic, cosmetic-ish but it breaks any test
that asserts full traceback text.

Also minor: `table.concat` invalid-value error leaks our internal byte-array
representation (`invalid value ([116,97,...])`) instead of `invalid value (table)`.

---

## 3. Fidelity gaps (5.3 / 5.5), deduped and prioritized by centrality

### CENTRAL (break otherwise-valid programs)

**F1 — 5.5 `global` declaration is chunk-wide + permanent, not block-scoped.**
The dominant 5.5 defect. An explicit `global` decl should void implicit `global *`
only for the *remainder of its enclosing block*; we leak it to the whole chunk and
out of nested functions, so subsequent free names (even `print`) wrongly become
compile errors.
```
diff_one.sh 5.5 'do global Y; Y=1 end; print("after")'
  OURS: (command line):1: variable 'print' not declared
  REF : after
```
Severity: HIGH, central. Confirms prelim H1 exactly.

**F2 — 5.5 `global x = expr` silently discards the initializer.** Every
initialized global reads back `nil`; when the value is then used, we raise
spurious nil-operand runtime errors where the reference computes the result.
```
diff_one.sh 5.5 'global x = 7; global print; print(x)'   # OURS nil / REF 7
diff_one.sh 5.5 'global print; global f <const> = function() return 42 end; print(f())'
  OURS: attempt to call a nil value (global 'f')   REF: 42
```
Severity: HIGH, central — breaks the primary documented use of the feature.

**F3 — 5.5 for-loop control variable is not read-only.** The headline 5.5
semantic change. Numeric (int+float), the first generic-for var, inside functions
and `load`, even self-assign — all should be a compile error; we accept them.
```
diff_one.sh 5.5 'for i=1,3 do i=i+1 end'
  OURS: (ok, exit 0)   REF: attempt to assign to const variable 'i' (exit 1)
```
Severity: HIGH, central.

**F4 — 5.5 float `tostring` uses `%.14g`, not the round-trip `%.17g`.** Pervasive:
`1/3`, `math.pi`, `0.1+0.2`, `2^53`, and large int-valued floats (which 5.5 prints
in fixed-point `N.0` form). Confirmed 5.5-specific (our 5.4 correctly matches
5.4's `%.14g`).
```
diff_one.sh 5.5 'print(1/3)'   # OURS 0.33333333333333 / REF 0.33333333333333331
```
Severity: HIGH, central — hits essentially every non-trivial float under 5.5.

**F5 — 5.3 whole compat-math family missing.** `math.atan2/cosh/sinh/tanh/pow/
log10/ldexp/frexp` all error "attempt to call a nil value". Stock 5.3 ships these
(`LUA_COMPAT_MATHLIB` default ON).
```
diff_one.sh 5.3 'print(math.pow(2,10))'   # OURS error / REF 1024.0
```
Severity: HIGH, central for 5.3.

**F6 — 5.3 string→number coercion missing in the core arith *and* bitwise path.**
We use the 5.4 string-library-metamethod model unconditionally, so 5.3 bitwise on
numeric strings errors (should compute) and 5.3 arith-on-bad-string emits the
5.4-style metamethod message instead of 5.3's "perform arithmetic on a string
value".
```
diff_one.sh 5.3 'print("3" & 5)'   # OURS error / REF 1
diff_one.sh 5.3 'print("abc"+1)'   # OURS 5.4-metamethod msg / REF "...arithmetic on a string value"
```
Severity: HIGH for 5.3 — `errors.lua`/`bitwise.lua` assert on these exact strings.

**F7 — 5.3/5.4 lost `__le`-from-`__lt` derivation (= R-C above for 5.4).** Also a
5.3 central metamethod behavior (`LUA_COMPAT_LT_LE`), hit by events.lua and
coroutine.lua. We behave like 5.5 (correctly error) for all three; 5.3 and 5.4
references derive it.

### SECONDARY (correctness, narrower surface)

**F8 — 5.5 `global` used as an ordinary identifier PANICS the lexer (CRASH).**
`local global = 5`, `print(global)`, `{global=5}` all abort with a Rust panic
(`lua-lex/src/lib.rs:662`, index OOB, exit 101). The reference (`LUA_COMPAT_GLOBAL`
on) treats `global` as a normal name.
```
diff_one.sh 5.5 'local global = 5'   # OURS panic exit 101 / REF 5
```
Severity: HIGH — it is a panic, `global` is a common variable name, and it taints
the 5.5 compat story. Highest-priority bug after the central semantic gaps.

**F9 — 5.5 missing/incorrect new APIs:** named vararg tables `function f(...t)`
not parsed (`')' expected near 't'`); `utf8.offset` missing its new 2nd return
(end byte position); `collectgarbage("param",...)` rejected, obsolete `"setpause"`
still accepted, wrong prior-mode strings; `global function` form not parsed;
`global <const> *` const not enforced; `error(nil)` not replaced by
`<no error object>`. Each is a distinct, niche-to-moderate fidelity gap.

**F10 — 5.3 missing `__ipairs` metamethod, lenient `\u{}` (>0x10FFFF accepted),
`%q` of inf/nan emits 5.4 form, closure-sharing equality, `print` not routing
through global `tostring`, label-parser false duplicate, `string.unpack` lenient
bounds.** A long tail of 5.3-specific behaviors not wired into the 5.3 backend.

### TAIL (error-wording fidelity, shared across versions)

**F11 — shared error-formatting family** (also the bulk of the 5.4 regressions
R-E/R-F/R-G): missing `to '<fn>'`, missing `(command line):N:` prefix on
length/concat/string-arith, missing `[C]: in ?` traceback tail, `got nil` vs
`got no value`, plus 5.3-specific name annotations (`(local 'x')`, `(field 'a')`,
`(upvalue 'up')`) and 5.5-specific frame renaming (`in global 'x'`, `in field
'char'`). Individually niche; collectively they dominate the raw DIFF counts and
will fail any test suite that asserts on error text (`errors.lua` does).

---

## 4. Cross-check against MULTIVERSION_PRELIM_REVIEW.md

Adversarial testing **CONFIRMED** the review's predicted high-severity issues and
found several it missed.

| Prelim claim | Adversarial result |
|---|---|
| **H1** 5.5 `global` block-scoping wrong (chunk-wide/permanent) | **CONFIRMED, central** (F1). 13+ snippets reproduce the scope leak across do/while/for/repeat/if/function boundaries. The review's worry that the original battery missed it (every battery snippet used top-level `global`) was exactly right. |
| **H2** 5.1/5.2 masquerade as 5.4 | Not directly re-tested here (no 5.1/5.2 reference binary in `/tmp/lua-refs`); the adversarial hunt was scoped to 5.3/5.4/5.5. Remains an open, unverified-but-credible concern. |
| **H3** real verification (`check.sh` 23/10/7) not in CI; enforcement untested | **Strongly reinforced.** The 23/10/7 battery missed F1 (scope leak), F2 (dropped initializer), F3 (for-const), F8 (lexer panic), F4 (float round-trip) — all central. The "10/10 5.5 oracle" was far narrower than it looked. |
| **5.4 parity "proven, not asserted"** | **Confirmed for the core, refuted for the edges.** Numeric/language core is genuinely clean, but 5.4 is *not* byte-clean: `string.find` arity bug (R-A), `math.type` false (R-B), `__le`-from-`__lt` (R-C), wrong-operand-type (R-D), and the error-formatting family (R-E/F/G). |
| **"String-coercion gate correctly localized; 5.4 provably unaffected"** | Partly contradicted: the localization left 5.3 *without* core arith/bitwise string coercion (F6), and the 5.4 string-arith error path still drops the location prefix and mislabels operands (R-D/R-F). |

**NEW issues the review did not call out:**

- **N1.** `global` as an identifier **panics** the lexer (F8) — a crash, not just a
  semantic gap. The review flagged scope but not the lexer reservation overrun.
- **N2.** 5.5 `global x = expr` drops the initializer entirely (F2) — central, and
  orthogonal to the block-scoping issue.
- **N3.** 5.5 for-loop control-var read-only enforcement absent (F3) — a headline
  5.5 change the review didn't mention.
- **N4.** `string.find` spurious extra return (R-A) — a cross-version value bug.
- **N5.** `math.type`/`math.tointeger` return `false` not `nil` (R-B) — cross-version.
- **N6.** Lost `__le`-from-`__lt` for 5.3/5.4 (R-C/F7).
- **N7.** 5.5 float round-trip `tostring` (F4) and 5.3 compat-math family (F5).

---

## 5. Coverage statement (honest about blind spots)

**Exercised well:** 5.4 numeric/arith/bitwise/format/comparison core (deep);
5.4 language constructs (closures, metamethods, coroutines, `<const>`/`<close>`,
goto, varargs, OOP); 5.3 number model and stdlib (math/bit32/utf8/pack/string);
5.5 `global` statement in all its forms (scope, const, init, redeclare, syntax
errors), for-const, float formatting, new-API presence; cross-version error
wording; curated official-test-file slices for all three versions.

**Not exercised / remaining blind spots:**
- **5.1 / 5.2 (prelim H2) — not tested at all.** No reference binary available in
  `/tmp/lua-refs`. The masquerade concern is unverified.
- **The compat-flag ambiguity of the reference binaries themselves.** The two 5.5
  category files disagree on whether `lua5.5.0` was built with `LUA_COMPAT_GLOBAL`
  on or off, and `lua5.4.7` clearly has `LUA_COMPAT_LT_LE` on (R-C). Some FIDELITY
  verdicts (especially `__le`-from-`__lt`, and `global`-as-identifier) are
  contingent on the compat-build choice of the oracle, not on the bare language
  spec. **Before treating these as must-fix, confirm what compat flags the pinned
  reference binaries were built with** — that decides whether they are "the
  contract" or oracle artifacts.
- GC timing, weak tables, `__gc` ordering, finalizer semantics — only lightly
  touched.
- Long-running / stateful programs, large-data, locale-dependent paths (skipped as
  host-noise), and the C API surface (only Lua-level behavior was diffed).
- Performance/hot-path behavior of the runtime-flag seam (prelim M2) — not a
  differential-output concern, untested here.

---

## 6. Prioritized "fix-before-trustworthy" list

1. **Fix the 5.5 lexer panic on `global` as an identifier (F8/N1).** A crash is
   never acceptable; `global` is a common name. Cheap and highest-leverage.
2. **5.5 `global` block-scoping (F1/H1).** Track `global_strict`/`declared_globals`
   per-block, save/restore on block enter/exit (mirror local `nactvar` unwind).
   The central correctness gap.
3. **5.5 `global` initializer (F2/N2).** Emit the assignment; today it is dropped.
4. **5.5 for-loop control-var read-only (F3/N3).** Compile-time const enforcement.
5. **`math.type`/`math.tointeger` return `nil` not `false` (R-B).** One-line-ish,
   fixes a cross-version (incl. 5.4) wrong-value bug.
6. **`string.find` extra empty return (R-A).** Cross-version arity-correctness bug.
7. **Error-formatting family (R-E/R-F/R-G/F11).** Add `to '<fn>'`, the
   `(command line):N:` prefix on length/concat/string-arith, the `[C]: in ?`
   traceback tail, `got no value`, and name annotations. Single largest DIFF
   source; required to pass `errors.lua` on any version. Fix R-D (wrong 2nd-operand
   type) in the same pass.
8. **5.5 float round-trip `tostring` (F4).** Version-gate `%.17g`-equivalent.
9. **5.3 compat-math family (F5)** and **5.3 core string coercion in arith/bitwise
   (F6).** Both central to 5.3 fidelity.
10. **Resolve the reference-binary compat-flag question (§5)** before committing to
    `__le`-from-`__lt` (R-C/F7) and `global`-as-identifier as contractual.
11. **Move the oracle into CI (H3)**, asserting the enforcement paths the original
    battery missed (block-scoping, initializer, for-const).
12. **Verify or refuse 5.1/5.2 (H2)** — out of this hunt's scope, still open.

---

## Verification note

Independently re-reproduced via `diff_one.sh` during synthesis (all DIFF as
reported): 5.4 — `math.type`, `math.tointeger`, `string.char` argerror, `#nil`
prefix, `-"x"` operand type, `string.find` arity, `__le`-from-`__lt` (and
confirmed `lua5.4.7` itself prints `false`); 5.5 — `local global` panic, do-block
scope leak, dropped initializer, for-const, `1/3` round-trip; 5.3 — `math.pow`
missing, `"3" & 5` coercion. No reported headline divergence failed to reproduce.
