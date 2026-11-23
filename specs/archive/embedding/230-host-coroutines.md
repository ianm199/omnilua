# Spec #230 — Host-driven coroutines (`create_thread` / `resume` / `status`)

Status: design, pre-implementation. **The riskiest of the batch.** Reviewer focus:
**do NOT hand-roll a partial resume** — the stdlib `aux_resume` does cross-thread
upvalue snapshot/flush and a GC snapshot that a naive `lua_resume` call omits;
skipping them corrupts upvalues / GC state.

## Problem

`Thread` is an opaque `Value::Thread` wrapper with no methods (lib.rs:2253). A host
cannot create, resume, or query a coroutine from Rust — only Lua-level
`coroutine.*` works. mlua exposes `Thread::resume`/`status`.

## Substrate (verified)

- `lua_vm::do_::lua_resume(state, from, nargs, &mut nres) -> LuaStatus`
  (`do_.rs:1322`): the **low-level** resume. Args must already be on the
  coroutine's own stack; `nres` out = yielded/returned count; status is
  `Ok` (finished) / `Yield` (suspended) / error.
- `lua_vm::state::new_thread(state, Some(function_value))` (`state.rs:6144`):
  creates a suspended coroutine with `function_value` on its stack and pushes
  `Value::Thread` onto the parent stack.
- **The safe driver is the stdlib's `aux_resume`** (`coro_lib.rs:283`), used by
  `coroutine.resume` (`co_resume`, coro_lib.rs:644). Beyond `lua_resume` it does:
  1. transfer args parent-stack → coroutine-stack (separate stacks; copy via buffer);
  2. **open-upvalue snapshot** parent → `GlobalState.cross_thread_upvals`
     (coro_lib.rs:307–318) and **flush back** after (coro_lib.rs:398–411) — missing
     this leaves stale upvalues;
  3. `push_parent_gc_snapshot` (coro_lib.rs:320) for GC/borrow safety;
  4. `try_borrow_mut` the coroutine — **already-borrowed = currently running**
     (COS_NORM) → error;
  5. switch `current_thread_id`, call `lua_resume`, restore;
  6. collect results coroutine-stack → parent buffer.
- Status: `LuaStatus` (`lua-types/src/status.rs`); host-facing coroutine status via
  `aux_status` (coro_lib.rs:222) → `COS_RUN/DEAD/YIELD/NORM`; `co_status`
  (coro_lib.rs:758) returns `"running"/"dead"/"suspended"/"normal"`.

## Design — drive resume through the existing safe machinery, not `lua_resume` raw

The cleanest, lowest-risk approach is to **reuse `coroutine.resume`'s machinery**
rather than replicate `aux_resume`'s snapshot/flush in the runtime crate. Two
candidate implementations:

### Option A (preferred) — call the stdlib `co_resume` path on the parent stack
`Thread::resume(args)`:
1. on the parent `Lua` state, push the `Thread` value, then push the marshalled
   `args`;
2. invoke the same code `coroutine.resume` runs (`aux_resume`/`co_resume`),
   reusing all its snapshot/flush/borrow safety;
3. read back the `(true, ...results)` / `(false, err)` it leaves and convert to
   `Result<Variadic<Value>>`.

Requires `aux_resume` (or a thin wrapper) be callable from `lua-rs-runtime`.
`co_resume` is `pub` but expects to be invoked as a registered C function with a
Lua calling convention. The spec's open question: expose a small
`lua_stdlib::coro_lib::host_resume(parent, thread, args) -> Result<(LuaStatus, Vec<LuaValue>)>`
wrapper around `aux_resume` so the runtime drives the *exact* safe path.

### Option B (fallback) — `lua_resume` + replicate the snapshot/flush
Only if A is infeasible. Replicate, in order: borrow-check, arg transfer, upval
snapshot, `push_parent_gc_snapshot`, `lua_resume`, result collection, **upval
flush**. High risk of omitting a step — **discouraged**; flagged for the reviewer
to confirm A is viable so we avoid B.

### API

```rust
impl Lua {
    pub fn create_thread(&self, body: Function) -> Result<Thread>;
}
impl Thread {
    /// Resume, passing `args` (first resume → function arguments; later →
    /// values returned from the matching `coroutine.yield`). Returns the values
    /// passed to `yield` (still suspended) or the function's return values
    /// (finished). A runtime error in the coroutine is returned as `Err`.
    pub fn resume<A: IntoLuaMulti, R: FromLuaMulti>(&self, args: A) -> Result<R>;
    pub fn status(&self) -> ThreadStatus;   // Suspended | Running | Normal | Dead
}

pub enum ThreadStatus { Suspended, Running, Normal, Dead }
```

`ThreadStatus` is a new public enum mapping `COS_*`. `resume` distinguishing
"yielded" vs "finished" beyond the values is deferred (mlua returns just the
values; a later `resume_status` can expose the discriminant if needed).

## Risks for the reviewer (the crux)

1. **Cross-thread upvalue coherence** — the single most important correctness
   point. Confirm Option A reuses `aux_resume`'s snapshot (coro_lib.rs:307) AND
   flush (coro_lib.rs:398); a host resume that skips the flush corrupts the parent's
   open upvalues. Reject any design that calls `lua_resume` without it.
2. **Borrow/re-entrancy** — resuming an already-running coroutine must error
   (try_borrow_mut fails, COS_NORM), not panic. Confirm the runtime path surfaces
   this as a clean `Err`, mirroring `cannot resume non-suspended coroutine`.
3. **Provenance** — a `Thread` resumed against a different `Lua` instance must
   error (reuse the existing different-state guard).
4. **GC snapshot** — `push_parent_gc_snapshot` must wrap the resume; confirm
   Option A inherits it.
5. **Error object lifetime** — the coroutine's error value lives on the coroutine
   stack; it must be copied/rooted before the coroutine stack is cleaned
   (coro_lib.rs handles this; confirm the wrapper preserves it).

## Test plan

`crates/lua-rs-runtime/tests/host_coroutines.rs`:
- create a thread from a yielding function; resume across several steps passing in
  and getting out values; observe `Suspended`→`Dead`.
- a coroutine that reads/writes an **upvalue** of an enclosing function across a
  yield — value coherent after host-driven resume (the upval-flush regression test).
- resuming a finished thread errors; `status()` transitions correct.
- a runtime error inside the coroutine surfaces as `Err`, thread becomes `Dead`.
- parity: the same coroutine driven purely in Lua (`coroutine.create`+`resume`)
  yields identical observable results.

Oracle gate: `multiversion_oracle` green, the coroutine official tests
(`coroutine.lua` via the harness) green, full `cargo test -p omnilua` green.

## Open questions for the reviewer

- Is exposing a `lua_stdlib::coro_lib::host_resume` wrapper around `aux_resume`
  (Option A) the right seam, or is there an even lower-risk reuse? Strongly prefer
  reusing aux_resume over replicating it.
- Does the active-thread switch (`current_thread_id`) need any runtime-crate-side
  bookkeeping, or is it fully internal to aux_resume?
- Should v1 ship `create_thread` + `resume` + `status` only, deferring
  yield-from-host (a host C function yielding) — which is a separate, harder
  problem? Proposed: yes, defer host-side yield.

## Codex review reconciliation (VERDICT: REVISE — deeper than this batch)

Codex confirmed Option A is the right direction but only after non-trivial work,
and ruled Option B out entirely. Required before implementation:

- **Runtime `active_state` bookkeeping (High).** A Rust callback that re-enters `Lua`
  from inside a resumed coroutine uses `LuaInner.active_state` (lib.rs:399/919), which
  falls back to the *parent*, not the coroutine `LuaState`; a nested `resume` then
  snapshots/flushes the wrong stack + upvalues. Host resume must set `active_state` to
  the coroutine state for the duration of the resume.
- **Reject host-returned `Yield` (High).** A Rust callback returning `LuaError::Yield`
  via `?` makes `lua_resume` see `Yield` without `lua_yieldk`'s setup (do_.rs:1417/1366)
  → corruption. v1 translates callback-returned `Yield` to a runtime error.
- **Root the error payload before cleanup/GC (High).** Mirror `Function::call`
  (lib.rs:2036) — `capture_error_in_state` the coroutine error before the coroutine
  stack is cleaned. Test: `error({})` + forced GC + inspect.
- **Provenance (Medium).** `LuaThread` carries only an `id` (value.rs:150); cross-state
  ids collide. Guard via `RootedValue::raw_for_lua`; wrapper is same-state-only.
- **Version checks (Medium).** `create_thread` must mirror `co_create` (rejects C/Rust
  bodies on 5.1, coro_lib.rs:708).
- **Dedicated wrapper, not raw `co_resume` (Medium).** `co_resume` leaves results in
  C-call convention with the arg frame present; add `lua_stdlib::coro_lib::host_resume`
  around `aux_resume` with saved-top restore + result extraction + error capture;
  feature-gate on `coroutine`.
- **Pre-existing bug surfaced (filed separately).** Main thread isn't in
  `GlobalState.threads` (state.rs:1808), so `aux_resume` reports it dead while
  `aux_status` says normal → `coroutine.resume(coroutine.running())` returns
  `"cannot resume dead coroutine"` but the 5.4 oracle expects `"cannot resume
  non-suspended coroutine"`.

**Conclusion:** #230 is a coordinated VM+stdlib+runtime effort, not part of this
embedding batch. This spec stands as the design once the above land.
