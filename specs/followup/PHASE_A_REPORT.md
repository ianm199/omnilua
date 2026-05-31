# Phase A Report — pre-existing 5.4 port bugs (#76–#79)

Scope: the four pre-existing 5.4 port bugs the multiversion adversarial sweep
surfaced (R-A..R-G), all verified present *before* the 5.3/5.4/5.5 multiversion
work shipped (v0.0.19). This phase confirmed each against the oracle, fixed the
clear-cut ones, and escalated the contract-dependent one.

Branch: `mv-followup`. Fix commits: `1bfa0c5` (#76), `29cc321` (#77),
`78d4986` (#79). #78 intentionally has **no** commit (deferred).

Evidence: `specs/oracle/diff_one.sh`, `specs/oracle/check.sh {5.4,5.3,5.5}`,
and `crates/lua-rs-runtime/tests/multiversion_oracle.rs`. Confirm specs:
`specs/followup/confirm-{76,77,78,79}.md`.

---

## #76 — `math.type` / `math.tointeger` return `false` instead of `nil` on failure

- **Confirmed?** Yes. Reproduced on 5.3/5.4/5.5; every default reference returns
  `nil` (`luaL_pushfail` = `lua_pushnil` in the contract builds; `LUA_FAILISFALSE`
  is pinned off). The "PORT NOTE" comments justifying `false` were factually wrong.
- **Fixed?** Yes. `crates/lua-stdlib/src/math_lib.rs` — both fail branches now push
  `LuaValue::Nil`; misleading comments corrected. CI: `issue76_*` added.
- **Gate:** build green; tests green; `diff_one.sh {5.4,5.3,5.5}` on
  `math.type("x")` / `math.tointeger(3.5)` all **MATCH**.
- Classification: CLEAR-CUT, shared-core, matched every version.

## #77 — `string.find` returns a spurious trailing empty value (R-A)

- **Confirmed?** Yes. Pattern path + zero explicit captures returned arity 3
  (extra empty string) vs reference arity 2, cross-version 5.1–5.5. Root cause:
  the port dropped C's `(ms->level == 0 && s)` guard in `push_captures`.
- **Fixed?** Yes. `crates/lua-stdlib/src/string_lib.rs` — `find` arm no longer
  synthesizes a whole-match value when there are no captures; `match`/`gmatch`/
  `gsub` untouched. CI: `issue77_*` added (arity + capture-still-works guards).
- **Gate:** build/tests green; `select("#", string.find("hello","l+"))` **MATCH**
  on 5.4/5.3/5.5.
- Classification: CLEAR-CUT, shared-core, matched every version.

## #78 — `__le` not derived from `__lt` (R-C) — DEFERRED FOR DECISION

- **Confirmed?** Yes. With only `__lt` defined, `a <= b` must derive `not (b < a)`
  in the default 5.1–5.4 builds; 5.5 removed it. We raise on all versions: correct
  for 5.5, **wrong for 5.3 and 5.4** (`diff_one.sh` shows 5.3/5.4 DIFF, 5.5 MATCH).
- **Fixed?** No — intentionally deferred. No commit, no CI test added (the spec's
  proposed tests would fail today; adding them would red the gate).
- **Contract analysis:** the fallback exists only because the pinned `make macosx`
  builds ship `LUA_COMPAT_LT_LE` ON (via the `LUA_COMPAT_5_3` umbrella). The bare
  5.4 language spec does not mandate it and 5.4's own manual deprecates it. Fixing
  means flipping the *headline 5.4 port* from "raises" to "returns a value" to
  mirror a compat-shim default — exactly the `LUA_COMPAT_*` shim class PORTING.md
  §13 chose to omit. A correct fix is a version-gated branch in
  `crates/lua-vm/src/tagmethods.rs` `call_order_tm` (derive for V51–V54, keep
  raising for V55; the immediate `OP_LEI/GEI` path inherits it via
  `call_orderi_tm`).
- **Recommendation:** Fix it, version-gated to V51–V54. CONTRACT.md's target is
  "what real users run," and the stock binary is the binding oracle; this is a
  genuine value-vs-error divergence, and the cost is one localized gated branch.
  But this is a policy call (treat the compat build as contract vs. decline to
  mirror a compat shim), so it stays a human decision, not an agent auto-fix.

## #79 — Error-message fidelity cluster (R-D/E/F/G)

Five sub-items. Four clear-cut sub-items fixed; one architectural sub-item deferred.

- **(a1)** missing `to '<fn>'` on `string.char`/`utf8.char` range errors — FIXED.
- **(a2)** `got nil` vs `got no value` for absent args — FIXED (auxlib +
  api.rs/state_stub.rs absent-arg detection).
- **(b)** missing `(command line):N:` prefix on `#` / `..` / string-arith — FIXED
  for the routed paths.
- **(c)** arith/unary metamethod-failure mislabeled operand types (stack
  corruption from an eager `get_meta_field`) — FIXED; `trymt` now short-circuits
  exactly like C's `||`.
- **(e)** `table.concat` leaked the internal byte-array repr (`[116, 97, ...]`)
  and dropped the prefix — FIXED (renders the type name as text, routes through
  `auxlib::lua_error`).
- **(d)** uncaught-error traceback omits the trailing `[C]: in ?` frame —
  **DEFERRED (architectural).** Our CLI `run` (the `pmain` analogue) is a plain
  Rust fn, so there is no C CallInfo above the main chunk; `get_stack` is correct
  and must not change. Faithful fix requires running the main chunk beneath a
  synthesized base C frame — higher blast radius, its own oracle pass. `diff_one.sh
  5.4 'error("boom")'` still DIFFs on exactly this tail line.
- **Gate:** build/tests green; `diff_one.sh 5.4` for (a1)(a2)(b-len)(b-concat)(c)(e)
  all **MATCH**; (a1)/(e) also MATCH on 5.3/5.5. CI: `v54_*` / `v_*` tests added
  (with byte-array negative guard). (d) CI deliberately not added.

### Error-cluster sub-items left for Phase B

1. **#79(d)** — `[C]: in ?` traceback tail (architectural; pmain-as-C-closure).
2. **#79(c)/(b) on 5.3** — string-arith failure on 5.3 still emits the 5.4-style
   `attempt to sub a 'table' with a 'string'` instead of 5.3's legacy
   `attempt to perform arithmetic on a table value`. `trymt`
   (`string_lib.rs:507`) hardcodes the 5.4/5.5 wording with no version gate. This
   was **explicitly descoped** out of #79 (see the comment on
   `v54_v55_string_arith_coercion_failure`, which only asserts V54/V55); it is a
   known per-version-wording leftover, not a regression, and `check.sh 5.3` does
   not exercise it, so the gate stays green. Version-gating string-arith
   error wording is Phase-B (finishing 5.3) work.

---

## Current oracle status (this build)

| Battery | Result |
|---|---|
| `check.sh 5.4` | 7 passed, 0 failed |
| `check.sh 5.3` | 23 passed, 0 failed |
| `check.sh 5.5` | 10 passed, 0 failed |
| `cargo build --workspace` | green |
| `cargo test --workspace --features lua-rs-runtime/derive` | all green (multiversion_oracle: 20/20) |

Known open DIFFs outside the green battery (tracked, not gated): #78 on 5.3/5.4,
#79(d) on all versions, #79 string-arith wording on 5.3.

## Hand-off to Phase B (finishing 5.3)

- **Decide #78** (the only deferred-for-decision item): if "fix," implement the
  version-gated `__le`-from-`__lt` derivation (V51–V54 derive, V55 raise) in
  `tagmethods.rs::call_order_tm` and land the confirm-78 CI tests.
- **#79 string-arith wording on 5.3**: version-gate `trymt`'s message so 5.3 emits
  `attempt to perform arithmetic on a <type> value`. Natural fit for the 5.3 pass.
- **#79(d)**: schedule the pmain-as-C-closure CLI restructure as its own isolated
  change with a dedicated oracle pass (spawn-the-binary CLI test for the
  `\t[C]: in ?` tail); do not bundle with wording fixes.
