# Spec — Async integration (`create_async_function` / `call_async`)

Status: **DESIGN ONLY — not implemented.** Issue: to be filed (see the serde PR
that lands alongside this doc). This is the correctness-sensitive tier: it crosses
the GC ↔ coroutine ↔ boundary-`RefCell` seam, the project's highest-stakes area.
Per the standing workflow it must get a cross-model adversarial review (codex,
read-only) **before** any code is written. The "Open questions for review" section
at the end is the review's entry point.

## Problem

A host embedding omniLua in an async Rust service (tokio/async-std) wants Lua
scripts to call host functions that do real I/O — network, DB, timers — without
blocking the executor thread. mlua offers this via its `async` feature:
`Lua::create_async_function` (a Rust `async` closure callable from Lua) and
`Function::call_async` / `Chunk::eval_async` (drive Lua from Rust, awaiting host
async fns). omniLua has none of it (verified: zero `create_async`/`Future`/
`async fn` in `crates/lua-rs-runtime/src`).

The standard mechanism: when Lua calls an async host fn, the running coroutine
**yields**, carrying a request to the host; a Rust **driver** polls the
corresponding future and **resumes** the coroutine with the result when ready.
The script reads as if it blocks; the OS thread does not.

## Substrate (verified on `origin/main` @ d225f470)

The machinery that would be a deep VM rewrite *if absent* already exists. This is
a **boundary integration, not a VM change.**

- **Yield-from-native + continuations.** `lua_yieldk(state, nresults, k, ctx)`
  (`crates/lua-vm/src/do_.rs:1417`) suspends with a continuation. The slot is
  `LuaKFunction = fn(&mut LuaState, status: i32, ctx: isize) -> Result<usize, LuaError>`
  (`crates/lua-vm/src/state.rs:660`); set via `set_u_c_k` / `set_u_c_ctx`
  (`state.rs:829`/`:834`), read via `u_c_k` / `u_c_ctx` (`state.rs:820`/`:815`).
  **Note the shape: a bare `fn` pointer plus a single `isize` of context — it
  cannot hold a Rust `Future` or a closure.** This constraint drives the design.
- **Resume loop.** `lua_resume` (`do_.rs:1322`) → `resume_coroutine` (`do_.rs:1257`)
  runs the VM synchronously until the coroutine yields or returns, then returns
  control to the caller with the yielded/returned values. `aux_resume`
  (`crates/lua-stdlib/src/coro_lib.rs:283`) transfers args/results across the
  parent↔child stacks; `aux_status` (`coro_lib.rs:222`) classifies suspended vs
  dead.
- **Suspended-stack rooting.** While a child runs, the parent's live stack and
  open upvalues are snapshotted into `g.suspended_parent_stacks` /
  `g.suspended_parent_open_upvals` and traced (`coro_lib.rs:451`).
- **Host coroutine API (#230)** — `create_thread` / `Thread::resume` /
  `Thread::status` (`specs/embedding/230-host-coroutines.md`). This is the
  **prerequisite**: async is "drive a `Thread` from a future." (#230 ships on
  `feat/embedding-hard-tier`; async must land on a base that includes it.)
- **Boundary re-entrancy.** `Lua` holds the VM behind `state: RefCell<LuaState>`
  plus an `active_state: Cell<*mut LuaState>` raw pointer for re-entry
  (`crates/lua-rs-runtime/src/lib.rs:428`). `with_state` (`lib.rs:1007`) borrows
  the cell for the *duration of one closure*; the audited unsafe re-entry bridge
  is `active_state_mut` (`lib.rs:1024`, SAFETY at `:1030`).
- **Rooting.** Every handle owns a `RootedValue` (`lib.rs` rooting machinery)
  anchored in the external root set, traced every collection; `Drop` unroots.

## Design

### Public API surface

```rust
impl Lua {
    /// Register a Rust async fn callable from Lua. Calling it from a coroutine
    /// suspends that coroutine until the future resolves.
    pub fn create_async_function<A, R, F, Fut>(&self, f: F) -> Result<Function>
    where
        A: FromLuaMulti, R: IntoLuaMulti,
        F: Fn(Lua, A) -> Fut + 'static,
        Fut: Future<Output = Result<R>> + 'static;
}

impl Function { pub async fn call_async<A: IntoLuaMulti, R: FromLuaMulti>(&self, args: A) -> Result<R>; }
impl Chunk    { pub async fn eval_async<R: FromLuaMulti>(self) -> Result<R>;
                pub async fn exec_async(self) -> Result<()>; }
```

All futures are `!Send` (see Constraints).

### Mechanism — the token-registry + passthrough-continuation pattern

The `isize` continuation context cannot hold a future. The design sidesteps this
entirely: **the future never lives inside the VM; the driver owns it.**

1. **Registration.** `create_async_function(f)` boxes `f` into a per-instance
   registry (`Vec<Box<dyn Fn(Lua, Vec<Value>) -> LocalBoxFuture<Result<Vec<Value>>>>>`
   on the Rust side of `LuaInner`) and returns a native function whose token is
   the registry index.
2. **Call → yield.** When Lua calls it, the native trampoline packages the call
   args, then calls `lua_yieldk(state, 0, k = passthrough, ctx = token_index)` and
   returns `Err(LuaError::Yield)`. The coroutine suspends; control returns to the
   driver. The args travel out as the yield payload (extracted to Rust-owned
   `Value`s / `RootedValue`s before the borrow is released).
3. **Driver awaits — outside any VM borrow.** `call_async`/`eval_async` runs a
   loop: `resume(thread, feed)` (a short `with_state`), observe `status()`. If
   `Suspended` with an async-yield payload, look up the registry closure by token,
   build `fut = closure(lua, args)`, and `fut.await` — **with no `with_state`
   borrow held.** The await happens between resumes, in the driver's own frame.
4. **Resume with the result.** The awaited `Result<Vec<Value>>` becomes the feed
   for the next `resume`; the `passthrough` continuation returns those values as
   the async fn's results, and the script proceeds.
5. Loop until `status() == Dead`; convert the final results to `R`.

The continuation `k` is a fixed `fn` that returns the resume-args as the call's
results (`status` arg distinguishes normal resume from error injection). No
per-call state lives in `k`/`ctx` beyond the token index.

## Why this is not a VM rewrite

The yield/continuation/resume substrate is already present and exercised by
`coroutine.*` and #230. Async adds: (a) a Rust-side registry of boxed async
closures, (b) a native trampoline that yields a token, (c) one fixed passthrough
continuation, (d) an async driver loop in the embedding layer. No opcode-loop,
GC-collector, or `unsafe` changes are anticipated. The VM stays a synchronous
loop that can only suspend at yield points — which is exactly where async fns
yield.

## Hard parts & proposed solutions

1. **Yield-from-native is not exposed in `create_function`.** The current
   trampoline (`rust_callback_trampoline`) runs callbacks to completion and never
   sets a continuation. Solution: a separate `create_async_function` registration
   path that installs the passthrough continuation via `set_u_c_k`/`set_u_c_ctx`
   and returns `Err(Yield)`. Do **not** retrofit the sync path.
2. **`ctx: isize` cannot hold a future.** Solved by the registry: `ctx` carries
   only a token index; the future is built and owned by the driver. (Design choice
   over the alternative of a side-table keyed by thread id, which is heavier.)
3. **`with_state` `RefCell` must not be held across `.await`.** This is a design
   invariant, not a runtime risk: the driver awaits *between* resumes, when no
   `with_state` guard exists. Each `resume` is its own short borrow. A borrow held
   across `.await` would deadlock the next re-entrant `with_state` and is forbidden
   by construction; the review must confirm no code path holds it.
4. **GC rooting across `.await`** — the soundness crux; see next section.
5. **`!Send` / executor.** `Lua` is `Rc<RefCell<…>>` → `!Send`/`!Sync`; the driver
   future is `!Send`. Requires a single-threaded executor (tokio `LocalSet`,
   `tokio::task::spawn_local`, or any `LocalSet`-style runtime). Document; ship a
   small `LocalSet` example, stay executor-agnostic otherwise. Async does **not**
   serve the wasm wedge (no executor on bare `wasm32-unknown-unknown`); its value
   is native server embedders.
6. **Yield across `pcall` / version semantics.** 5.1 cannot yield across a `pcall`
   boundary; 5.2+ can via continuations. An async fn called inside `pcall` on 5.1
   would error. The multi-version core must gate or clearly error. Open question.
7. **Error propagation.** A future returning `Err` resumes the thread in the error
   path so the Lua side sees a catchable error (or the driver surfaces it). Define
   the exact contract; reuse the #229 traceback machinery if capture is on.
8. **Cancellation / drop.** Dropping the driver future mid-await leaves the
   coroutine suspended; it is still rooted (no UAF), reclaimed when the `Lua`
   drops. Document as a leak-until-instance-drop, not unsoundness.

## Soundness argument — rooting a suspended coroutine across `.await`

Claim: a coroutine suspended at an async yield, and every Lua object it references,
survives any GC that runs while the driver awaits.

- The driver holds the coroutine as a `Thread` handle = a `RootedValue` for the
  coroutine's whole lifetime. The external root set is traced every collection
  (`lib.rs` rooting), so the `LuaThread` GC object is never swept while the driver
  future is alive.
- A `LuaThread`'s own stack (its in-flight args, locals, and the yielded values)
  is reachable from the thread object and traced as part of tracing the thread.
  Thus everything the suspended frame needs is kept alive transitively by the one
  rooted `Thread`.
- The async closure's captured state is Rust-owned (heap, not GC) and outlives the
  await by ownership.
- Call args that the host closure needs after the await are extracted to
  Rust-owned `Value`s (each its own `RootedValue`) **before** the borrow is
  released, so they do not depend on the coroutine stack surviving in any
  particular shape.

Therefore no `with_state` borrow and no GC root is required *during* the await
beyond the rooted `Thread` (+ any extracted arg roots) the driver already holds.
**Review must verify**: (a) the `LuaThread` stack is actually walked by the
tracer in both incremental and generational modes; (b) `suspended_parent_stacks`
interaction is correct when the *parent* is itself the driver (a host-driven
thread, not a Lua `coroutine.resume`); (c) no path drops the arg roots early.

## Constraints summary

- Single-threaded executor only (`!Send`). Ship a tokio `LocalSet` example.
- Not for wasm; native-server feature. Gate behind an `async` cargo feature with
  optional `futures`/`futures-util` (for `LocalBoxFuture`) — never pulling tokio
  into the core.
- Interaction with the sandbox instruction budget: define whether an await pauses
  the budget (it should — wall-clock spent awaiting is not Lua instructions).

## Test plan

- Ready future: async fn returning an already-resolved value → script gets it.
- Pending future: async fn awaiting a `oneshot`/timer → coroutine suspends, driver
  resumes, script continues. Use an in-memory channel so the test is deterministic
  (the `conn_transport_kit` philosophy: no real sockets).
- Sequential awaits in one script; values thread through correctly.
- Error path: future returns `Err` → Lua `pcall` catches it (5.2+); 5.1 behavior
  pinned against the reference.
- Nested: async fn → calls a Lua fn → calls another async fn (nested yields).
- GC stress: force a full collection while a coroutine is suspended awaiting;
  assert no UAF (GC canary in both modes + Miri if available).
- Concurrency: N async Lua coroutines driven on one `LocalSet`.
- Off-by-default: with the feature disabled, the build and the oracle are
  byte-identical.

## Open questions for codex-review

1. Is the **token-registry + passthrough-continuation** design correct and
   minimal, versus a thread-id-keyed future side-table? Any case where the
   passthrough `k` must carry more than the token?
2. **Yield-across-`pcall`** per version: which versions permit it through
   `lua_yieldk`, and should `create_async_function` hard-error on 5.1 inside a
   non-yieldable frame, or is there a faithful path?
3. **Rooting proof**: confirm the suspended `LuaThread`'s stack is traced in
   incremental *and* generational modes via the rooted `Thread`. Cite the trace
   path. Is there any window where the coroutine is suspended but transiently
   unrooted between `resume` returning and the driver re-taking ownership?
4. **`suspended_parent_stacks`** semantics when the resumer is the Rust driver
   rather than a Lua `coroutine.resume` — is the snapshot/restore still balanced?
5. **Sandbox**: must the instruction budget and any wall-clock deadline pause
   across an await? What is the correct interaction with `Lua::sandboxed`?
6. **Executor surface**: generic over executor, or ship a concrete tokio
   `LocalSet` helper plus the trait? What is the minimal dependency footprint?
7. **Error contract**: exact mapping of a future `Err` onto the Lua error path,
   and its interaction with #229 traceback capture.
