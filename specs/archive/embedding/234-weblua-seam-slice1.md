# Spec #234 (slice 1) — WebLua number-model seam + typed divergence

Status: design, pre-implementation. The full design lives in
`specs/WEBLUA_MULTIVERSION_API_SPEC.md` (Engine/Backend dispatch, §4–6). This spec
carves the **first implementable slice** that delivers value without the
`enum Engine` refactor: a single-source-of-truth host-boundary number seam and the
two typed-divergence error variants. Reviewer focus: **don't change the behavior of
existing `FromLua<i64>` / `IntoLua` callers** (semver-visible, oracle-checked).

## Why a slice

The existing WebLua spec stages the work (§6): (1) consolidate OpCode, (2) introduce
`enum Engine` + `Backend`, (3+) per-version backends. Steps 1–2 are a large
behavior-preserving refactor. But two pieces are independently valuable and small:

1. The **number-model marshaling seam** (spec §2.2) — currently `marshal_from`
   (#235) coerces `Integer→Float` for FloatOnly targets *inline* (`coerce_int`,
   lib.rs) with **silent widening**, and `FromLua<i64>` has its own ad-hoc rule.
   Centralize both into one seam with the spec's "correct or loud" policy.
2. The **typed divergence** error variants (spec §3.4): `Unsupported` and
   `LossyIntConversion`.

The `enum Engine` backbone (slice 2) is **out of scope here** and remains its own
focused effort.

## Substrate (verified)

- `LuaVersion::number_model()` → `NumberModel::{FloatOnly, Dual}`
  (`lua-types/src/version.rs`).
- `LuaError` (`lua-types/src/error.rs:13`) is the cross-crate error enum; it is the
  natural home for new variants but is **not** currently `#[non_exhaustive]` — adding
  variants is semver-visible (acceptable in 0.x; mark `#[non_exhaustive]` now to
  avoid future breaks).
- `marshal_from`'s `coerce_int(dst, i)` (lib.rs, from #235): `FloatOnly ⇒
  Number(i as f64)` silently; `Dual ⇒ Integer(i)`.
- `FromLua<i64>` and `IntoLua` impls live in `crates/lua-rs-runtime/src/lib.rs`
  (~2800–3160). **Current `FromLua<i64>` behavior on a float input must be read
  before changing it.**

## Design

### 1. Error variants (lua-types)

```rust
#[non_exhaustive]
pub enum LuaError {
    // …existing…
    /// A feature absent on the active backend was requested (runtime analog of
    /// mlua's #[cfg] removal).
    Unsupported { feature: &'static str, version: LuaVersion },
    /// A host i64 with no exact f64 representation crossed into a FloatOnly
    /// backend under LossyIntPolicy::ErrorOnInexact.
    LossyIntConversion { value: i64, version: LuaVersion },
}
```
`message_lossy` (error.rs:157) gains arms for both; the `Display`/`message_lossy`
text is new (no oracle asserts on these — they don't exist yet).

### 2. Per-instance policy

```rust
pub enum LossyIntPolicy { ErrorOnInexact, Truncate }   // default ErrorOnInexact

impl Lua { pub fn set_lossy_int_policy(&self, p: LossyIntPolicy); }
```
Stored on `LuaInner` (runtime crate), read at the seam.

### 3. The seam — single source of truth

```rust
/// Lower a host i64 into the active number model. THE ingest rule.
fn lower_host_int(version: LuaVersion, policy: LossyIntPolicy, i: i64) -> Result<Value> {
    match version.number_model() {
        NumberModel::Dual => Ok(Value::Integer(i)),       // exact round-trip
        NumberModel::FloatOnly => {
            let f = i as f64;
            if f as i64 == i { Ok(Value::Number(f)) }     // exact (|i| ≤ 2^53)
            else { match policy {
                LossyIntPolicy::Truncate => Ok(Value::Number(f)),
                LossyIntPolicy::ErrorOnInexact =>
                    Err(LuaError::LossyIntConversion { value: i, version }.into()),
            }}
        }
    }
}
```
- `marshal_from`'s `coerce_int` is **replaced by** `lower_host_int` (now exact-or-error
  by default instead of silently lossy).
- `IntoLua for i64` routes through `lower_host_int` (host→Lua ingest).
- `FromLua for i64` egress rule (spec §2.2.2): accept `Integer(i)`; accept
  `Number(f)` **iff** `f.fract()==0 && in range` else `FromLuaConversion` error; **no
  truncation**. ONLY adopt this if it does not regress current callers (see risk).

## Risks for the reviewer (the crux)

1. **`FromLua<i64>` semantics change is the danger.** If the current impl truncates
   floats, switching to exact-or-error is *stricter* and could break callers/tests.
   If it already errors on non-integral floats, the change is a no-op or additive.
   The spec REQUIRES reading the current impl first and only tightening if the
   oracle + full suite stay green. Prefer: keep accepting `Integer`, ADD
   integral-float acceptance, and only reject non-integral if that matches today.
2. **`#[non_exhaustive]` on `LuaError`** forces all `match` sites to add a wildcard
   arm — find every exhaustive match on `LuaError` across crates and confirm they
   compile (or already have `_`). This is a mechanical but wide change.
3. **`marshal_from` behavior change**: v1 silently widened; now large ints error by
   default. Update #235's tests/docs; this is the intended hardening but it is a
   behavior change for that one path.
4. Keep it **backend-free**: this slice must NOT introduce `enum Engine`; it reads
   `self.version()` directly, exactly as the rest of the cold-path seams do today.

## Test plan

`crates/lua-rs-runtime/tests/number_seam.rs`:
- `2^53` exact → Number on a 5.1 instance (ok); `2^53+1` → `LossyIntConversion`
  under default policy; `Truncate` policy widens it.
- `Dual` instance keeps `Integer` exactly.
- `FromLua<i64>` accepts `3.0`, rejects `3.5`, accepts `3` — and an explicit test
  that this matches pre-change behavior for every case the old impl already handled.
- `marshal_from` of a huge int into 5.1 now errors (updated #235 expectation).

Oracle gate: `multiversion_oracle` byte-identical (number formatting/`math.type`
behavior is internal to the VM, untouched by this host-boundary seam), full
`cargo test -p omnilua` + workspace green.

## Open questions for the reviewer

- Is tightening `FromLua<i64>` worth the regression risk, or should slice 1 ship
  only the ingest seam (`lower_host_int` + variants + policy) and leave egress
  `FromLua<i64>` exactly as-is? (Leaning: ship ingest + variants; touch egress only
  if it's already exact-or-error.)
- `Unsupported` has no producer in slice 1 (no feature gating yet) — add the variant
  now (so the type is stable) but wire producers in slice 2? Or defer the variant
  until there's a caller? (Leaning: add now, it's the stable surface the spec wants.)
- Should `set_lossy_int_policy` instead be a `LuaBuilder` option (per WebLua §1.4)?
  We have no builder yet; a setter is the minimal step. Confirm acceptable.

## Codex review reconciliation (VERDICT: REVISE — descope adopted)

The review pared this to a minimal, correct core and caught a latent bug:

- **Right choke point (High).** Lower in `Value::to_raw_for_lua` (lib.rs:1819), the
  real host→Lua push path — today it pushes `Integer` as `Int` *regardless of
  version*, so a 5.1 (float-only) instance stores a host `i64` as a true integer (a
  latent bug). NOT in `IntoLua<i64>` (misses manually-built `Value::Integer`). Reuse
  the helper from `marshal_value`.
- **Leave `FromLua<i64>` alone (High).** It already accepts integral floats and
  rejects non-integral (lib.rs:2892); only out-of-range floats would change. Egress
  unchanged this slice. (`LuaError::FromLuaConversion` doesn't exist here anyway.)
- **Exactness check was wrong (High).** `f as i64 == i` accepts inexact values —
  Rust float→int **saturates** (`i64::MAX as f64 as i64 == i64::MAX`). Reuse the VM's
  guarded float↔int helper (state.rs:894). Rename `Truncate` (`i64 as f64` rounds).
- **Don't flip `marshal_from`'s default (Medium).** Silent-widen → error is breaking;
  strict is **opt-in** via `LossyIntPolicy`, default preserves current behavior.
- **Defer `Unsupported` (Medium).** Premature (no producer) and the WebLua example is
  oracle-wrong — absent 5.1 stdlib globals must fail as a normal Lua runtime error,
  not a typed feature error. Also defer `#[non_exhaustive]` until a real producer.
- **No new error variant yet (Medium).** The `ErrorOnInexact` path raises a normal
  `LuaError::runtime` (correct `pcall`/`to_status`/`into_value` semantics for free).
- **Engine slicing confirmed coherent.** The playbook (§117) reads a runtime version
  flag at production sites, not a `dyn` Engine — so the seam is a flag-read at
  `to_raw_for_lua`. `enum Engine` stays deferred.

**Revised slice 1 = ** `LossyIntPolicy` (default `WidenLossy`) + a shared,
guarded `lower_host_int` helper used by `to_raw_for_lua` + `marshal_value`. Nothing
else (no variants, no `FromLua`/`IntoLua` changes, no `Unsupported`).
