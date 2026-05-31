# Confirm #78 — `__le` not derived from `__lt`

**Status: CONFIRMED, current divergence. Classification: CONTRACT-DEPENDENT / RISKY — DO NOT auto-fix. Escalate to human.**

When a metatable defines `__lt` but not `__le`, evaluating `a <= b` must (in the
default `make macosx` builds of 5.1–5.4) derive the result as `not (b < a)`,
calling `__lt(b, a)`. Lua 5.5 removed this (`LUA_COMPAT_LT_LE` gone). Our
implementation raises `attempt to compare two table values` on **all** versions.
So we are correct for 5.5 and wrong for 5.3 and 5.4.

This is pre-existing 5.4 port debt (the fallback was deliberately omitted, not a
multiversion regression) — see the PORT NOTE at `tagmethods.rs:584`.

## Exact repros (live, this build — `target/debug/lua-rs` + `/tmp/lua-refs/bin/*`)

### Repro A — `a <= a`, only `__lt` defined
`local a=setmetatable({},{__lt=function() return true end}); print(a<=a)`

| Version | OURS | REFERENCE | Verdict |
|---|---|---|---|
| 5.1 (ref only) | n/a | `false` | ref derives |
| 5.2 (ref only) | n/a | `false` | ref derives |
| 5.3 | `(command line):1: attempt to compare two table values` (exit 1) | `false` (exit 0) | **DIFF** |
| 5.4 | `(command line):1: attempt to compare two table values` (exit 1) | `false` (exit 0) | **DIFF** |
| 5.5 | `... attempt to compare two table values` (exit 1) | same error (exit 1) | MATCH (modulo R-G `[C]: in ?` tail) |

(`a <= a` derives `not (a < a)` = `not true` = `false`.)

### Repro B — asymmetric `__lt`, two distinct tables
`local mt={__lt=function(x,y) return rawlen(x)<rawlen(y) end}; local a=setmetatable({1},mt); local b=setmetatable({1,2},mt); print(a<=b, b<=a)`

| Version | OURS | REFERENCE |
|---|---|---|
| 5.3 | raises (exit 1) | `true	false` (exit 0) |
| 5.4 | raises (exit 1) | `true	false` (exit 0) |
| 5.5 | raises (exit 1) | raises (exit 1) — MATCH |

### Swap-order confirmation (proves the derivation is `not (b < a)`, not `not (a < b)`)
`/tmp/lua-refs/bin/lua5.4.7 -e 'local mt={__lt=function(x,y) print("lt",x.n,y.n); return x.n<y.n end}; local a=setmetatable({n=1},mt); local b=setmetatable({n=2},mt); print("res",a<=b)'`
prints `lt 2 1` then `res true` → it called `__lt(b,a)` = `2<1` = false → `not false` = true.

### Sanity — explicit `__le` is NOT broken (all MATCH)
`local a=setmetatable({},{__le=function() return "LE" end, __lt=function() return "LT" end}); print(a<=a)` → MATCH on 5.3 / 5.4 / 5.5. The divergence is *only* the derived fallback when `__le` is absent.

## Which references derive it (per `specs/oracle/CONTRACT.md`, empirically reconfirmed)

| Behavior | 5.1.5 | 5.2.4 | 5.3.6 | 5.4.7 | 5.5.0 |
|---|---|---|---|---|---|
| `__le` from `__lt` (only `__lt` defined) | yes | yes | **yes** | **yes** | no (removed) |

So the correct contract is: **derive for V51/V52/V53/V54; raise for V55.**

## Classification: CONTRACT-DEPENDENT / RISKY (clear-cut = NO)

Per `CONTRACT.md` lines 40–43: "`__le`-from-`__lt` is part of the 5.3 AND 5.4
contract (both default builds derive it). Our impl errors instead. Because this
is a single cross-version behavior that also affects the 5.4 baseline, it is
**pre-existing 5.4 port debt**, tracked separately from the 5.3/5.5 multiversion
work, not fixed in this branch."

Why NOT clear-cut / why a human must decide:

1. **It changes 5.4 behavior that we currently advertise as "error".** Today
   lua-rs 5.4 *raises* on `a<=a`; matching the reference flips that to a
   *value*. That is a semantic change to the headline 5.4 port, not a pure
   bug-narrowing fix.
2. **The contract is oracle-build-specific.** The fallback only exists because
   the pinned `make macosx` builds happen to ship `LUA_COMPAT_LT_LE` ON (it
   defaults on via the `LUA_COMPAT_5_3` umbrella). The bare 5.4 language spec
   does *not* mandate it; 5.4's own manual deprecates it. We would be encoding
   a compat-shim default as if it were core semantics — exactly the
   `LUA_COMPAT_*` shim PORTING.md §13 chose to omit (see `tagmethods.rs:556`).
3. **Cross-version blast radius.** A shared-core fix must match EVERY version's
   reference: V51/V52/V53/V54 derive, V55 must keep raising. A naive "always
   derive" fix would regress 5.5 to a DIFF. The fix is therefore a
   version-gated branch, with its own correctness surface (the `not (b < a)`
   operand-swap order, recursion/`__lt` re-dispatch, immediate-operand paths in
   `call_orderi_tm`).
4. **Adversarial findings concur (R-C, §2 and §6 item 10):** severity "depends
   on whether we treat the compat-built binary as the contract"; listed under
   "Resolve the reference-binary compat-flag question *before* committing."

Recommendation: **human decision required.** The only question is policy: do we
treat the default `make macosx` build as the binding contract (then fix, gated
to V51–V54) or treat `LUA_COMPAT_LT_LE` as an oracle artifact we decline to
mirror (then close #78 as won't-fix and document the deviation)? The
CONTRACT.md target ("what real users run") argues for fixing; PORTING.md §13's
no-shim stance argues for declining. Recommend fixing, gated, because it is a
genuine value-vs-error divergence against the stated oracle and the cost is one
localized version-gated branch — but this is a contract call, not an agent call.

## Precise impl location

- **`crates/lua-vm/src/tagmethods.rs:557` — `call_order_tm`** is the single
  dispatch point. The fallback is deliberately omitted at **`tagmethods.rs:584`**
  (`// PORT NOTE: LUA_COMPAT_LT_LE block skipped`), and the rationale comment is
  at **`tagmethods.rs:555–556`**. After the metamethod lookup fails
  (`call_bin_tm` returns `false`), it goes straight to
  `Err(order_error(...))` at **`tagmethods.rs:586`**.
- Callers (no change needed, context): `crates/lua-vm/src/vm.rs:1048`
  (`less_equal_others` → `call_order_tm(l, r, TagMethod::Le)`),
  `vm.rs:2374`/`vm.rs:2402` (immediate-operand `OP_LEI`/`OP_GEI` →
  `order_imm_slow` → `call_orderi_tm` at `tagmethods.rs:598`, which also routes
  through `call_order_tm`).
- Version source of truth: **`state.lua_version: lua_types::LuaVersion`**
  (`crates/lua-vm/src/state.rs:1003`); variants in
  `crates/lua-types/src/version.rs:31` (`V51`/`V52`/`V53`/`V54`/`V55`).

## Intended fix (if approved — version-gated, faithful to C `luaT_callorderTM`)

In `call_order_tm`, only when `event == TagMethod::Le` and the `__le` lookup
failed (the `call_bin_tm` returned `false` branch), and only when
`state.lua_version` is one of `V51|V52|V53|V54`: retry as `__lt` with operands
swapped and negate — i.e. `return Ok(!call_order_tm(state, p2, p1, TagMethod::Lt)?)`.
If that inner `__lt` dispatch also finds no metamethod it raises, matching C
(which sets `L->ci->callstatus |= CIST_LEQ` and re-enters `lessthanothers`). For
`V55`, fall through to `order_error` unchanged. The immediate path
(`call_orderi_tm`) inherits the fix automatically since it delegates to
`call_order_tm`. Note C's compat block raises a *different* message on total
absence ("attempt to compare two ... values") via the `__lt` path — verify the
raised text still matches the reference after the swap.

## Exact CI test assertions to add (`crates/lua-rs-runtime/tests/multiversion_oracle.rs`)

Using the existing `eq` / `err_contains` helpers (`Lua::new_versioned` +
load+pcall wrapper). These encode the reference outputs and are the regression
gate; they will FAIL today (documenting the open bug) until #78 is fixed:

```rust
// #78: __le derived from __lt (LUA_COMPAT_LT_LE). Derived in default
// make-macosx builds of 5.1–5.4; removed in 5.5. Reference outputs captured
// via specs/oracle/diff_one.sh. See specs/followup/confirm-78.md.
const LE_FROM_LT_A: &str =
    "local a=setmetatable({},{__lt=function() return true end}); return a<=a";
const LE_FROM_LT_B: &str =
    "local mt={__lt=function(x,y) return rawlen(x)<rawlen(y) end}; \
     local a=setmetatable({1},mt); local b=setmetatable({1,2},mt); \
     return tostring(a<=b)..','..tostring(b<=a)";

// 5.3 and 5.4: reference DERIVES `a<=b` as `not (b<a)`.
eq(LuaVersion::V53, LE_FROM_LT_A, "false");
eq(LuaVersion::V54, LE_FROM_LT_A, "false");
eq(LuaVersion::V53, LE_FROM_LT_B, "true,false");
eq(LuaVersion::V54, LE_FROM_LT_B, "true,false");

// 5.5: reference REMOVED the fallback — must still raise (guards against an
// over-broad "always derive" fix regressing 5.5).
err_contains(LuaVersion::V55, LE_FROM_LT_A, "attempt to compare two table values");

// Sanity: explicit __le must keep winning on every version (not regressed).
eq(LuaVersion::V53,
   "local a=setmetatable({},{__le=function() return 'LE' end, __lt=function() return 'LT' end}); return a<=a",
   "LE");
eq(LuaVersion::V54,
   "local a=setmetatable({},{__le=function() return 'LE' end, __lt=function() return 'LT' end}); return a<=a",
   "LE");
eq(LuaVersion::V55,
   "local a=setmetatable({},{__le=function() return 'LE' end, __lt=function() return 'LT' end}); return a<=a",
   "LE");
```

(If `Lua::new_versioned` does not yet expose `V51`/`V52`, the 5.1/5.2 leg is
covered by direct reference invocation above; the CI gate covers 5.3/5.4/5.5,
which is where lua-rs has a version-selected path.)

## Gate to run after any fix (must ALL stay green)

```
cargo build --workspace
cargo test --workspace --features lua-rs-runtime/derive
specs/oracle/check.sh 5.4 && specs/oracle/check.sh 5.3 && specs/oracle/check.sh 5.5
```
A shared-core change must match every version's reference, not just one.
