# Spec #233 — Userdata uservalues (`set_user_value` / `user_value`)

Status: design, pre-implementation. Reviewer focus: **GC write-barrier correctness**
(a missing/wrong barrier under the generational collector is an invisible
use-after-free — the highest-stakes failure mode in this codebase).

## Problem

A host can attach a Rust payload to a userdata (`host_value`) but cannot attach
arbitrary *Lua* values to it. Stock Lua 5.4 has `lua_setiuservalue` /
`lua_getiuservalue` (1-based slots); mlua exposes `set_user_value` /
`nth_user_value`. omniLua's `RawLuaUserData.uv: RefCell<Vec<LuaValue>>` exists
(`crates/lua-types/src/userdata.rs:14`) but is unreachable from the embedding API.

## Substrate (verified)

- Embedding `Lua::create_userdata::<T>` (`crates/lua-rs-runtime/src/lib.rs:1244`)
  constructs `RawLuaUserData` directly with `uv: RefCell::new(Vec::new())`
  (lib.rs:1286–1291) — **zero uservalue slots**.
- `lua_vm::api::set_i_uservalue(state, idx, n)` (`api.rs:1960`):
  - reads userdata at stack index `idx`, value from top of stack;
  - **does NOT grow** `uv` — returns `false` if `n < 1 || n > uv.len()`;
  - on success stores `uv[n-1] = val` then calls `state.gc().barrier_back(ud, &val)`
    (api.rs:1973) and pops the value.
- `lua_vm::api::get_i_uservalue(state, idx, n)` (`api.rs:1431`): pushes `uv[n-1]`
  (or Nil if out of range), returns its `LuaType`. Read-only, no barrier.
- **Barrier correctness** (the load-bearing analysis): `state.gc().barrier_back`
  (`state.rs:5669`) → `barrier_any` (downcasts `GcRef<LuaUserData>`, state.rs:4550)
  → `barrier_lua_value` (state.rs:4432): if `!child.is_collectable()` returns early
  (scalars safe); if generational+Backward runs `generational_backward_barrier`
  (state.rs:4444) **before** the child-type dispatch; then `barrier_gc_child`
  → `heap.barrier_back` (incremental tri-color) (heap.rs:2033). Covers
  scalar / string / table / function / userdata / thread children, in both
  incremental and generational modes. **No barrier hole in the existing path.**

## Design — pre-allocated fixed slots (the only barrier-safe option)

Reuse the existing, proven, barriered `set_i_uservalue`/`get_i_uservalue` and
**never grow `uv` from the runtime crate** (growing would require a hand-rolled
barrier — exactly the risk we refuse). Slots are fixed at creation, matching
C-Lua's `lua_newuserdatauv(L, size, nuvalue)`.

### API

```rust
impl Lua {
    /// Create a userdata with `nuvalue` Lua uservalue slots (1-based), each
    /// initialized to nil. (`create_userdata` keeps 0 slots, unchanged.)
    pub fn create_userdata_with_uservalues<T: UserData + 'static>(
        &self, data: T, nuvalue: usize,
    ) -> Result<AnyUserData>;
}

impl AnyUserData {
    /// Set the `n`-th uservalue (1-based). Errors if `n` exceeds the slot count
    /// the userdata was created with.
    pub fn set_user_value<V: IntoLua>(&self, n: usize, value: V) -> Result<()>;
    /// Read the `n`-th uservalue (1-based); nil if unset / out of range.
    pub fn user_value<V: FromLua>(&self, n: usize) -> Result<V>;
}
```

### Implementation sketch

`create_userdata_with_uservalues` is `create_userdata` with the single change
`uv: RefCell::new(vec![LuaValue::Nil; nuvalue])` at the construction site.

`set_user_value` (drives the barriered VM API, mirroring `Function::dump`'s
push/pop pattern):
```
with_state(|state| {
    push the userdata onto the stack            // raw_for_lua(self.root)
    push the marshalled value                   // value.into_lua(self).to_raw_for_lua
    let ok = lua_vm::api::set_i_uservalue(state, ud_idx, n as i32)?;  // barriers internally
    restore top
    if !ok { Err("uservalue index {n} out of range (userdata has N slots)") } else { Ok(()) }
})
```
`user_value` pushes the userdata, calls `get_i_uservalue(state, ud_idx, n)`,
pops the pushed value, converts via `Value::from_raw` + `FromLua`.

## Correctness argument (the thing to adversarially check)

1. The runtime crate performs **no direct `uv` write** — every store goes through
   `set_i_uservalue`, which barriers. ⇒ no hand-rolled barrier to get wrong.
2. Slots are fixed at creation, so `set_i_uservalue` never needs to grow ⇒ its
   `n <= nuvalue` guard is satisfied for valid `n`, and we surface an error (not a
   silent no-op) when `n` is too large.
3. Scalars are barrier-exempt by `barrier_lua_value`'s early return ⇒ storing a
   number/bool/nil is safe and a no-op for the collector.

## Out of scope (deferred)

- Growing the uservalue count after creation (would need a barriered grow path).
- The metamethod-breadth half of #233 (bitwise `__band`…, `__idiv`, `__close`,
  `__name`) and the derive extensions — separate, non-memory-safety work.

## Test plan (tier 2/3)

`crates/lua-rs-runtime/tests/uservalues.rs`:
- round-trip a string / table / nil through slot 1; multiple slots independent.
- set a **table** uservalue, drop all other handles, `gc_collect()`, then read it
  back intact — proves the barrier kept the child alive (the regression a missing
  barrier would cause).
- `n` beyond the slot count errors (not silent).
- a userdata from plain `create_userdata` (0 slots) errors on `set_user_value(1,…)`.
- version-invariant: behaves identically on a 5.1 and a 5.4 instance.

Oracle gate: full `cargo test -p omnilua` + `multiversion_oracle` green; the
GC canaries (`harness/canaries/gc/run_canaries.sh`) green, since this touches the
userdata/barrier surface.

## Open questions for the reviewer

- Is `vec![LuaValue::Nil; nuvalue]` at construction the *complete* requirement, or
  does the userdata's GC registration also need to know its uvalue count anywhere
  else (accounting, tracing)? Confirm the userdata `Trace` impl already traverses
  `uv` (it must, or even the stdlib `setiuservalue` path would leak/UAF).
- Any path where `set_i_uservalue`'s barrier is skipped (e.g. the userdata itself
  is white/unlinked at set time)? The C invariant is the parent is already linked.

## Codex review reconciliation (VERDICT: REVISE — adopted)

Central store path **confirmed sound** (`set_i_uservalue` → `barrier_back` →
generational backward barrier before child dispatch, state.rs:4444). Three fixes:

1. **GC accounting (High).** A non-empty preallocated `uv` must charge the heap:
   `LuaUserData::buffer_bytes()` includes `uv` capacity (userdata.rs:61) and the VM
   constructor calls `account_buffer` (api.rs:870); the runtime constructor skips it
   only because `uv` is empty today (lib.rs:1285). The new path must call
   `userdata.account_buffer(userdata.buffer_bytes() as isize)` **under the heap guard**.
2. **Checked index (Medium).** Use `i32::try_from(n)` (reject overflow) before the VM
   helper — `n as i32` lets a huge `usize` wrap to a valid slot. Cap `nuvalue` too.
3. **Real barrier test (High).** The "set/drop/`gc_collect`/read-back" test is
   meaningless — a full collect marks `uv` via `Trace` even with the barrier removed.
   Gate on the **GC canaries** (`harness/canaries/gc/run_canaries.sh`, which include
   `canary_j_testc_sweep_uservalue_barrier.lua` — black/old-userdata + white/young-child),
   and add an API test that exercises set-then-step rather than full-collect.
