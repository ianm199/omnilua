# Lua Phase E Runtime Architecture Spec

Status date: 2026-05-18.

This spec covers three decisions that should be settled before another large
agent phase lands on top of the Lua port:

1. coroutine execution,
2. incremental `collectgarbage("step", n)` semantics,
3. dynamic library loading for `package.loadlib` and C module searchers.

The common theme is that these are not isolated stdlib TODOs. Each one crosses
crate boundaries and changes what the harness must enforce. A one-shot agent can
implement a narrow slice after the design is settled; it should not be asked to
invent these designs while trying to make a test pass.

## Executive Decision

Recommended decisions:

| Area | Decision | Why |
|---|---|---|
| Coroutines | Implement faithful Lua coroutine semantics over heap-backed `LuaState` stacks first; use `corosensei` only if native Rust call-stack suspension is needed for yieldable host/C-call continuations. | C-Lua coroutines are VM threads sharing one `global_State`, not OS threads. The current Rust VM already stores Lua stack and `CallInfo` on heap structures, so most coroutine semantics should be modeled in the VM, not delegated wholesale to native stacks. |
| GC step budget | Add a real budgeted incremental step API, but do it as a pragmatic mark/sweep increment first, not full Lua generational parity. Define `n` as added KiB debt, then translate debt through `gcstepmul` into a bounded amount of mark/sweep work. | This matches the observable contract of Lua's `lua_gc(LUA_GCSTEP, n)` closely enough for the official tests and creates the right future shape for real generational GC. |
| Dynamic loading | Add an embedder dynamic-load hook in `lua-vm` and implement the CLI backend with `libloading`. Do not put `unsafe` in `lua-stdlib`. Treat C ABI compatibility as a separate milestone from loading a shared object. | `loadlib.c` is platform/host integration, like file I/O. Also, loading a symbol is not enough: a stock Lua C module expects the Lua C ABI, which this port does not yet expose. |

The ordering should be:

1. GC budget, because it is small, unlocks `gc.lua` progress, and reduces
   harness ambiguity.
2. Coroutines, because they affect VM call/yield semantics and the debug
   library.
3. Dynamic loading, because `package.loadlib` is operator-facing but does not
   block most official language tests unless C modules are in scope.

## Current Repo Facts

Relevant current surfaces:

- `crates/lua-coro/src/lib.rs` is a skeleton with no active unsafe budget.
  If a future stackful backend needs raw context switching, it must raise the
  crate's explicit unsafe budget in the same patch.
- `crates/lua-stdlib/src/coro_lib.rs` has the `lcorolib.c` stdlib shape, but
  execution operations are stubs or temporary emulation.
- `crates/lua-vm/src/do_.rs` already has translated `lua_resume`,
  `lua_yieldk`, continuation-unroll, and protected-call skeletons.
- `crates/lua-vm/src/state.rs` already has per-thread `LuaState`, shared
  `GlobalState`, `nCcalls`, `is_yieldable`, `new_thread`, and `close_thread`.
- `crates/lua-gc/src/heap.rs` is a stop-the-world tracing collector. Its
  `step_with_post_mark` is threshold-triggered full collection, not incremental
  work.
- `crates/lua-vm/src/api.rs` implements `GcArgs::Step { data }`, but `data`
  only affects `gc_debt`; the actual heap work is not budgeted by `data`.
- `crates/lua-stdlib/src/loadlib.rs` has faithful `loadlib.c` structure, but
  `lsys_load`, `lsys_sym`, and `lsys_unloadlib` are stubs because platform
  dynamic loading is unsafe and forbidden in `lua-stdlib`.
- `lua-cli` already owns filesystem hooks: `file_loader_hook`,
  `file_open_hook`, `file_remove_hook`. Dynamic loading should use the same
  capability pattern.

## Sources Compared

Local C-Lua sources used:

- `reference/lua-5.4.7/src/lcorolib.c`
- `reference/lua-5.4.7/src/ldo.c`
- `reference/lua-5.4.7/src/lapi.c`
- `reference/lua-5.4.7/src/lgc.c`
- `reference/lua-5.4.7/src/loadlib.c`
- `reference/lua-5.4.7/src/lstate.c`
- `reference/lua-5.4.7/src/lstate.h`

External dependency facts checked:

- `corosensei` 0.3.3 docs: stackful coroutine abstraction, supported targets,
  default guarded stacks, unwinding and cleanup-on-drop behavior.
  <https://docs.rs/corosensei/latest/corosensei/>
- `libloading` 0.9.0 docs: cross-platform dynamic library loading, unsafe
  `Library::new`, unsafe `Library::get`, lifetime binding between `Symbol` and
  `Library`, and platform caveats.
  <https://docs.rs/libloading/latest/libloading/>

## Part 1: Coroutines

### What C-Lua Actually Does

Lua coroutines are "threads" in the Lua sense, not OS threads.

In C-Lua:

- `lua_newthread(L)` allocates a new `lua_State` object.
- The new thread shares the same `global_State` as the parent.
- The new thread has its own Lua stack, `CallInfo` chain, open upvalues,
  `tbclist`, status byte, hook state, and `nCcalls`.
- `coroutine.create(f)` creates a new thread and moves `f` onto that thread's
  stack.
- `coroutine.resume(co, args...)` moves args from caller stack to coroutine
  stack, runs `lua_resume`, then moves yielded/returned results back.
- `coroutine.yield(values...)` sets the coroutine status to `LUA_YIELD`, stores
  `nyield`, and throws a yield signal to unwind to the resume boundary.
- Resumption either starts the coroutine body, resumes after a yield, runs C
  continuations, or finishes a partially yielded VM instruction via
  `luaV_finishOp`.
- `coroutine.close(co)` resets a suspended/dead coroutine and runs to-be-closed
  variables.

The crucial point: C-Lua does not need one OS thread per coroutine. It keeps Lua
activation state in `lua_State` and `CallInfo`. The native C stack is used while
executing, but Lua frames themselves live in the VM structures. Yielding unwinds
to a protected resume boundary via longjmp; resuming re-enters the VM using the
saved `CallInfo` and program counter.

### What The Rust Port Already Has

The port is closer to C-Lua's logical architecture than it may look:

- `LuaState.stack: Vec<StackValue>` is heap-backed.
- `LuaState.call_info: Vec<CallInfo>` is heap-backed.
- Lua `savedpc` is an index, not a native program counter.
- `vm::execute(state, ci)` is re-enterable in principle because it reads
  `savedpc` from `CallInfo`.
- `vm::finish_op` already exists for resuming opcodes interrupted by yields.
- `do_.rs` has translated `lua_resume`, `lua_yieldk`, `unroll`,
  `finish_ccall`, `finish_pcallk`, and `precover`.

This means the core Phase E work is not "invent coroutines." It is "connect the
already-translated coroutine control path to real thread values and teach the
VM to surface `LuaError::Yield` as a suspension rather than as a normal error."

### Important Correction: Corosensei Is Not The Whole Coroutine Design

Earlier notes say "Phase E wires real stackful coroutines via corosensei." That
may still be useful, but it should not be the first mental model.

There are two different stacks in play:

1. Lua VM stack: `LuaState.stack`, `CallInfo`, `savedpc`, open upvalues.
2. Native Rust call stack: recursive Rust calls during `state.call`,
   metamethods, stdlib functions, debug hooks, `pcallk` continuations, etc.

C-Lua's coroutine semantics are primarily about the first one. The second one
matters for yielding across C/Rust call frames. In C, that is handled by
`lua_yieldk`, non-yieldable call counters, continuations, and longjmp.

So the spec should be:

1. Make Lua thread identity real.
2. Make `lua_resume` / `lua_yieldk` work over heap-backed `LuaState`s.
3. Only introduce a native stack-switch backend for the parts that truly need
   preserving Rust call stacks across suspension.

### Why OS Threads Are The Wrong Main Architecture

OS threads look attractive because they avoid explicit unsafe stack switching.
For this repo, they are not actually the safe option.

The current runtime is intentionally single-threaded:

- `GcRef` and `LuaValue` are not `Send + Sync` surfaces.
- `GlobalState` is `Rc<RefCell<GlobalState>>`.
- file handles and other side tables use thread-local or `Rc<RefCell<_>>`
  patterns.
- the GC heap assumes one active mutator.
- Lua's own semantics assume only one coroutine runs at a time unless the host
  creates independent Lua states.

Implementing each Lua coroutine as an OS thread would force one of two bad
choices:

1. Make the whole VM `Send + Sync` by converting `Rc<RefCell<_>>` to
   `Arc<Mutex<_>>` or similar. That is a cross-workspace rewrite and replaces a
   small unsafe boundary with pervasive lock-ordering and reentrancy risk.
2. Keep most state thread-local and copy/serialize values across threads. That
   stops being faithful Lua coroutine semantics because tables, closures,
   userdata, strings, registry state, and GC identity must be shared.

OS threads also make GC harder: the collector would need to see all suspended
thread stacks and coordinate with parked OS threads. C-Lua simply walks all
`lua_State` stacks in one VM universe.

Conclusion: OS threads may be useful as a non-faithful compatibility mode for a
toy embedder, but not as Phase E's main architecture.

### Why Pure Stackless Is Also Not The Next Step

A fully stackless VM is the clean Rust answer in the abstract. Every operation
that can yield returns a `Poll`-like state, and the dispatcher is a trampoline.
This is how some Rust Lua implementations avoid native stack switching.

But converting this port now would mean rewriting the interpreter dispatch,
stdlib call surface, `pcallk`, metamethod calls, and debug hooks into an
explicit continuation machine. It is probably the best design for a greenfield
Rust Lua, not for this harness-generated C-to-Rust port where preserving the
shape of C-Lua is an explicit advantage.

### Recommended Coroutine Architecture

Represent the coroutine as a real VM thread first.

Add a canonical owned thread object, roughly:

```rust
pub struct LuaThread {
    state: RefCell<LuaState>,
    id: usize,
}
```

But do not put this new type ad hoc in `lua-stdlib`. It needs a canonical owner.
Given the current crate split:

- `lua_types::value::LuaThread` is currently only a placeholder value type.
- the real `LuaState` type lives in `lua-vm`.
- `LuaValue::Thread` currently expects `GcRef<lua_types::value::LuaThread>`.

There are two viable approaches:

1. Move the rich thread container into `lua-types` without depending on
   `lua-vm`: impossible if it directly contains `LuaState`.
2. Keep the rich thread container in `lua-vm`, and change the `LuaValue::Thread`
   payload to a VM-owned opaque thread handle with operations exposed through
   `state_stub`.

Recommendation: introduce a canonical `ThreadId` / `ThreadHandle` indirection.

```rust
pub struct LuaThread {
    id: usize,
}
```

Then keep the actual `LuaState` storage in `GlobalState`:

```rust
pub struct GlobalState {
    threads: Vec<GcRef<LuaState>>,
    current_thread_id: usize,
    main_thread_id: usize,
}
```

`LuaValue::Thread` can remain a lightweight identity value while `lua-vm`
resolves it through `GlobalState`. That avoids putting `LuaState` inside
`lua-types`, avoids a crate cycle, and keeps the "one canonical thread handle"
in the type vocabulary.

This also fixes current placeholder behavior in `new_thread`, where a real
`LuaState` is constructed and then discarded while a placeholder
`LuaThread::placeholder()` is pushed.

### Coroutine State Machine

A thread has these effective states:

- `Ok + no frames + function on stack`: initial suspended coroutine.
- `Ok + active frames`: normal/running depending on relation to caller.
- `Yield`: suspended after yield.
- error status: dead.
- `Ok + no stack values`: dead.

Implement `aux_status` against actual thread state:

```text
if target == current:
    running
else if target.status == Yield:
    suspended
else if target.status == Ok:
    if target has active frames:
        normal
    else if target stack top == 0:
        dead
    else:
        suspended
else:
    dead
```

This mirrors `lcorolib.c`.

### Yield Propagation In Rust

Current `lua_yieldk` returns `Err(LuaError::Yield)` for normal non-hook yields.
That is the right signal type, but the boundary handling must distinguish yield
from error:

- `LuaError::Yield` must not become a user-visible runtime error inside
  `coroutine.resume`.
- `lua_resume` should convert it to `LuaStatus::Yield` and leave yielded values
  on the coroutine stack.
- `pcall` / `pcallk` must treat yield according to Lua's non-yieldable counter
  and continuation rules.
- `call_no_yield` paths must continue rejecting yields across non-yieldable
  boundaries.

The danger is accidental `?` propagation through an outer API that treats every
`Err` as failure. Phase E should audit all `state.call`, `call_no_yield`,
`pcall_k`, metamethod-call, hook-call, and stdlib-call paths for this rule.

### Where Corosensei Fits

Use a backend boundary:

```rust
trait CoroutineBackend {
    fn start(&mut self, thread: ThreadId) -> ResumeOutcome;
    fn resume(&mut self, thread: ThreadId) -> ResumeOutcome;
    fn suspend(&mut self, values: usize) -> !;
    fn close(&mut self, thread: ThreadId) -> CloseOutcome;
}
```

Then implement two stages:

1. `VmThreadBackend`: no native stack switching. It relies on `LuaError::Yield`
   unwinding Rust frames back to the `lua_resume` boundary. This should be
   enough for pure Lua coroutines and many official tests.
2. `CorosenseiBackend`: only if real yieldable Rust/C-call continuations require
   preserving native stack frames.

Do not introduce `corosensei` directly into `lua-stdlib`. It belongs in
`lua-coro`, with a small safe facade consumed by `lua-vm`.

The `corosensei` crate is a plausible backend because it supports suspending
from any point in the coroutine call stack, has guarded default stacks, supports
major desktop/server targets, and unwinds suspended stacks on drop. But it
should remain a replaceable backend, not leak into the VM or stdlib API.

### Coroutine Unsafe Policy

Keep unsafe in `lua-coro`, not `lua-vm` or `lua-stdlib`.

The safety contract should be written in code comments next to each unsafe
boundary:

- a coroutine stack is only resumed by one parent at a time;
- no `RefCell` borrow into `GlobalState`, `LuaState`, table storage, or userdata
  may be held across a yield boundary;
- yielded values are copied/cloned onto the target Lua stack before switching;
- dropped suspended coroutines close to-be-closed variables or are marked dead;
- corosensei handles native stack cleanup, but Lua-level cleanup still runs via
  `close_thread` / `reset_thread`.

Add a hook check for `yield-across-borrow` patterns if practical: agents should
not introduce code that calls a potentially-yielding VM operation while holding
a `RefMut`.

### Coroutine Implementation Slices

Recommended agent slices:

1. **Thread identity and registry.**
   - Replace placeholder `LuaThread` usage with real thread IDs.
   - Store child `LuaState`s in `GlobalState`.
   - Implement lookup helpers in `lua-vm`.
   - No `corosensei` yet.

2. **`xmove`, `create`, `status`, `running`, `isyieldable`.**
   - Implement stack transfer between two `LuaState`s sharing one `GlobalState`.
   - Wire `coroutine.create`.
   - Make `aux_status` real.

3. **`resume` / `yield` for pure Lua.**
   - Make `aux_resume` call `lua_vm::do_::lua_resume`.
   - Treat `LuaError::Yield` as suspension at the right boundary.
   - Move yielded/returned values back to caller.

4. **`wrap` and error propagation.**
   - Replace direct-call emulation in `aux_wrap`.
   - Match C-Lua's `wrap` behavior: errors are raised, not returned as
     `(false, err)`.

5. **`close` and to-be-closed variables.**
   - Finish `close_thread` / `reset_thread`.
   - Run `__close` correctly for suspended/dead coroutines.

6. **Native stack backend only if needed.**
   - Add `lua-coro` backend using `corosensei`.
   - Keep the public VM API unchanged.

### Coroutine Harness Additions

Add targeted tests before full `coroutine.lua`:

- create/status initial suspended state;
- resume returning values;
- yield/resume value exchange;
- wrap error behavior;
- nested resume and `normal` status;
- yielding across non-yieldable `pcall`/C boundary errors;
- coroutine close runs `__close`;
- debug hook yield edge cases only after basic debug hooks are stable.

## Part 2: Incremental GC Budget

### What C-Lua Does

`lua_gc(L, LUA_GCSTEP, data)` in C:

- temporarily allows GC to run;
- if `data == 0`, clears debt and runs one basic GC step;
- otherwise adds `data * 1024` bytes to `GCdebt` and calls `luaC_checkGC`;
- returns `1` if the collector reached pause state at the end of the call.

The actual budget is in `lgc.c::incstep`:

```text
stepmul = gcstepmul | 1
debt = (GCdebt / WORK2MEM) * stepmul
stepsize = (1 << gcstepsize) / WORK2MEM * stepmul

loop:
    work = singlestep()
    debt -= work
until debt <= -stepsize or gcstate == pause

if pause:
    setpause()
else:
    GCdebt = (debt / stepmul) * WORK2MEM
```

`singlestep` is a state machine:

- pause -> restart collection;
- propagate -> mark one gray object;
- enteratomic -> run atomic;
- sweep allgc -> sweep a bounded chunk;
- sweep finalizable objects -> bounded chunk;
- sweep to-be-finalized -> bounded chunk;
- finish sweep;
- call a few finalizers;
- return to pause.

The key semantic is not that `n` directly means "do n objects." It means "add
n KiB of debt," and debt is converted to units of GC work through `gcstepmul`
and `gcstepsize`.

### What The Rust Port Does Now

Current Rust behavior is not budgeted:

- `api.rs::GcArgs::Step { data }` sets debt using `data * 1024`.
- `data == 0` calls `state.gc().step()`, but `GcHandle::step()` is a no-op.
- nonzero data calls `state.gc().check_step()`.
- `check_step()` calls `collect_via_heap(false)`.
- `collect_via_heap(false)` calls `Heap::step_with_post_mark`.
- `Heap::step_with_post_mark` checks `bytes >= threshold`; if true, it runs a
  full stop-the-world collection.

So `data` changes debt accounting but does not scale actual collector work.
This is why `gc.lua` step-sizing assertions plateau.

### Recommended Budget Semantics

Implement a pragmatic incremental collector with the same public shape as
C-Lua, without trying to finish full generational parity immediately.

Definitions:

- `data`: added KiB of GC debt, exactly like C-Lua.
- `WORK2MEM`: use `size_of::<LuaValue>()` or a fixed 16 bytes; choose one and
  document it.
- `stepmul`: `global.gcstepmul | 1`.
- `stepsize`: `1 << global.gcstepsize`, converted to work units.
- `work unit`: one gray object traced, one swept object visited, or a fixed
  finalizer cost.

Implement:

```rust
pub struct StepBudget {
    remaining_work: isize,
    max_credit: isize,
}

pub enum StepOutcome {
    Paused,
    InProgress,
    SkippedStopped,
}

impl Heap {
    pub fn incremental_step_with_post_mark(
        &self,
        roots: &dyn Trace,
        budget: StepBudget,
        post_mark: impl FnMut(&mut Marker),
    ) -> StepOutcome;
}
```

The first version may use a simplified state machine:

- `Idle/Pause`: initialize a marker, reset allgc colors, trace roots, leave gray
  queue stored on `Heap`, enter `Propagate`.
- `Propagate`: pop up to budget gray objects, tracing children.
- `Atomic`: run post-mark weak-table/finalizer hook to fixed point; enter
  sweep.
- `Sweep`: sweep up to budget allgc nodes.
- `Finalize`: schedule/run bounded finalizers if supported; otherwise move to
  pause.
- `Pause`: update threshold/debt and report cycle complete.

This needs `Marker` state to move from stack-local to heap-owned for in-progress
cycles. Today `Marker` is created inside `full_collect_with_post_mark`, so the
gray queue and visited set disappear after each call. That is fine for full
collect and impossible for true incremental stepping.

### Minimal Alternative For Official Tests

If full incremental state is too much for the next run, a smaller compatibility
shim can still be honest:

- `collectgarbage("step", n)` always runs a full collection when a cycle is
  needed;
- but the number of calls required to report cycle completion scales with `n`.

Do this by introducing a `manual_step_remaining_work` counter:

1. On first step of a cycle, estimate work as `heap.objects_len + roots_cost`.
2. Subtract `budget_from_data(n)` on each call.
3. Only run `full_collect_with_post_mark` and return `1` when the counter
   reaches zero.

This would satisfy step-sizing tests but is not a real incremental collector.
It should only be used if explicitly labeled as a transitional shim. The better
long-term path is the real heap-owned marker state above.

### Recommended GC Decision

Do not implement generational GC yet. Implement incremental pacing first.

Rationale:

- The official `collectgarbage("step", n)` observable only needs larger `n` to
  complete the cycle in fewer calls.
- The current `Heap` already has precise tracing, weak-table hooks, and sweep.
- Generational mode requires age bits, old cohorts, back barriers, touched lists,
  and a more complex invariant. That is a separate Phase D-3.
- A budgeted incremental collector is a strict improvement over the current
  threshold-full-collect behavior and gives future generational work a state
  machine to extend.

### GC Implementation Slices

1. **Expose heap work inventory.**
   - Add object count / approximate work count to `Heap`.
   - Add tests for stable counts after allocation/sweep.

2. **Move incremental state into `Heap`.**
   - Add `GcState::{Pause, Propagate, Atomic, Sweep, Finalize}`.
   - Store active `Marker` or equivalent visited/gray state in `Heap`.

3. **Budgeted propagate.**
   - Add `Marker::drain_gray_budget(max)` returning work performed.
   - Preserve remaining gray queue across calls.

4. **Budgeted sweep.**
   - Store sweep cursor.
   - Sweep at most N objects per call.

5. **Bridge weak-table/finalizer hook.**
   - Keep `collect_via_heap` as the owner of Lua-specific post-mark logic.
   - Invoke it during the atomic phase.

6. **Wire `GcArgs::Step`.**
   - Convert `data` to debt.
   - Convert debt to `StepBudget`.
   - Return `1` only if cycle reaches pause.

7. **Update docs and remove stale shim comments.**
   - `api.rs` still has Phase-B comments about simulated total bytes. Those
     must be replaced once real budgeted work exists.

### GC Harness Additions

Add a micro-test independent of all `gc.lua`:

```lua
collectgarbage("collect")
local function dosteps(n)
  local i = 0
  repeat
    i = i + 1
  until collectgarbage("step", n)
  return i
end
assert(dosteps(10) < dosteps(2))
```

Also add Rust unit tests for:

- budget 0 performs at least one unit of work;
- larger budget drains more gray objects than smaller budget;
- sweep can pause mid-list and resume;
- weak table pruning runs once per atomic phase;
- full collection remains equivalent to running incremental until pause.

## Part 3: Dynamic Library Loading

### What C-Lua Does

`loadlib.c` has three platform functions:

- `lsys_load`: `dlopen` / `LoadLibraryExA`;
- `lsys_sym`: `dlsym` / `GetProcAddress`;
- `lsys_unloadlib`: `dlclose` / `FreeLibrary`.

`lookforfunc(path, sym)`:

1. checks if the library is already loaded in the `CLIBS` registry table;
2. if not, loads it;
3. stores the handle in `CLIBS`;
4. if `sym == "*"`, returns `true`;
5. otherwise resolves the symbol and pushes it as a Lua C function;
6. on failure returns `(false, errmsg, "open" | "init")`.

`package.loadlib` is just the public wrapper. C module searchers use the same
path.

### Current Rust State

`loadlib.rs` translated the shape well, but the platform calls are stubs:

- `LibHandle(usize)` is a placeholder.
- `lsys_load` pushes `DLMSG` and returns `None`.
- `lsys_sym` pushes `DLMSG` and returns `None`.
- `lsys_unloadlib` does nothing.

This was correct under the "no unsafe in lua-stdlib" rule.

### The Hidden Bigger Issue: Lua C ABI

Dynamic loading has two layers:

1. loading a shared object and resolving a symbol;
2. calling that symbol with the ABI it expects.

Stock Lua binary modules export functions like:

```c
int luaopen_mymodule(lua_State *L);
```

That `lua_State *` is C-Lua's ABI. Our `LuaState` is a Rust struct with a Rust
API. A symbol loaded from a stock Lua C module cannot be safely called as:

```rust
fn(&mut LuaState) -> Result<usize, LuaError>
```

So `package.loadlib` can be unblocked in two different senses:

1. **Rust-native dynamic modules**: libraries compiled specifically against our
   Rust ABI, exporting a known `extern "C"` shim that returns a function pointer
   or registration table our VM understands.
2. **C-Lua ABI compatibility**: implement enough of the Lua C API ABI so
   existing `.so`/`.dylib`/`.dll` modules compiled for Lua 5.4 can run.

The second is much larger. It means exposing a stable C ABI facade for this
interpreter. That is valuable eventually, but it is not the same task as using
`dlopen`.

### Recommended Dynamic Loading Architecture

Keep `lua-stdlib` safe and platform-agnostic.

Add a dynamic loading capability to `GlobalState`, analogous to file hooks:

```rust
pub type DynLibLoadHook = fn(
    state: &mut LuaState,
    path: &[u8],
    see_global: bool,
) -> Result<DynLibId, LuaError>;

pub type DynLibSymbolHook = fn(
    state: &mut LuaState,
    handle: DynLibId,
    symbol: &[u8],
) -> Result<DynamicSymbol, LuaError>;

pub type DynLibUnloadHook = fn(handle: DynLibId);
```

But do not use raw `usize` handles casually. Handles must encode lifetime and
ownership. A better shape:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DynLibId(u64);

pub struct DynLibRegistry {
    libs: Vec<LoadedLib>,
}
```

The actual `libloading::Library` values should live in the embedder/backend
crate, likely `lua-cli` or a new `lua-loadlib` crate. `lua-stdlib` should only
store `DynLibId` in `CLIBS`.

### Backend Placement Options

#### Option A: `lua-cli` Hooks

Put the `libloading` backend in `lua-cli`, like file I/O.

Pros:

- follows existing hook pattern;
- keeps workspace unsafe budget concentrated at the embedding boundary;
- no new crate;
- easy to disable for sandboxed embeddings.

Cons:

- CLI owns library registry lifetime, which is awkward if non-CLI embedders
  want loadlib;
- tests that instantiate `LuaState` without CLI hooks get fallback behavior.

#### Option B: New `lua-loadlib` Crate

If reusable embedder loading becomes a requirement, create a crate with an
explicit unsafe budget and a safe facade:

```text
lua-stdlib -> lua-vm types only
lua-cli    -> lua-loadlib -> libloading
```

Pros:

- dedicated unsafe budget;
- reusable by embedders;
- cleaner API for loaded-library lifetime;
- easier to audit.

Cons:

- new crate and hook plumbing;
- still does not solve C-Lua ABI compatibility by itself.

Recommendation: start with Option A if the goal is official tests and CLI
behavior. Move to Option B once dynamic modules become product-facing.

### What To Return From `lsys_sym`

Do not pretend `dlsym` returns a Rust-native `fn(&mut LuaState)`.

Define explicit supported ABI modes:

```rust
pub enum DynamicSymbol {
    RustNative(fn(&mut LuaState) -> Result<usize, LuaError>),
    LuaCAbi(unsafe extern "C" fn(*mut LuaCState) -> libc::c_int),
    Unsupported { reason: Vec<u8> },
}
```

For the immediate port, support only `RustNative` modules or return a clear
error for C ABI modules:

```text
dynamic library loaded, but Lua C ABI modules are not supported by this build
```

This is more honest than resolving a C symbol and crashing.

### Unloading Policy

C-Lua unloads CLIBS entries from a `__gc` metamethod on the registry table.

Rust policy should be conservative:

- Do not unload a library while any function pointer from it may still be
  callable.
- Keep libraries alive until state close by default.
- `gctm` can drop registry entries at close, but explicit `Library::close` is
  optional and platform-dependent.

This is aligned with `libloading`'s safety model: the library must outlive any
symbol loaded from it. The simplest correct policy is to keep every loaded
library alive for the lifetime of the Lua state.

### Dynamic Loading Implementation Slices

1. **Add hook types and fields.**
   - Add `dynlib_load_hook`, `dynlib_symbol_hook`, `dynlib_unload_hook` to
     `GlobalState`.
   - Default to `None`.

2. **Replace local `lsys_*` stubs with hook calls.**
   - `lua-stdlib` still contains no unsafe.
   - Error behavior matches C fallback if hook is absent.

3. **Implement CLI backend with `libloading`.**
   - Store `Library` objects in a registry owned by CLI/backend.
   - Return stable `DynLibId`s.
   - Use `unsafe` only in backend.

4. **Gate ABI support.**
   - Initially support `sym == "*"` library loading and Rust-native symbols only
     if we define a Rust-native module ABI.
   - Return a clear `"init"` failure for stock Lua C ABI symbols until the ABI
     facade exists.

5. **Add tests.**
   - absent hook returns fallback error;
   - `package.loadlib(path, "*")` succeeds with backend hook and stores handle;
   - missing symbol returns `(false, msg, "init")`;
   - missing library returns `(false, msg, "open")`;
   - unloading happens at state close or is intentionally leaked.

## Cross-Cutting Harness Rules

These three areas need harness support so agents do not make architectural
escapes.

### Type Vocabulary Guard

Add or extend the type vocabulary registry with canonical owners:

| Name | Owner |
|---|---|
| `LuaThread` / `ThreadId` | `lua-types` or `lua-vm` by explicit decision |
| `DynLibId` | `lua-vm` or new `lua-loadlib` |
| `DynLibRegistry` | backend crate |
| `StepBudget` / `StepOutcome` | `lua-gc` |
| `CoroutineBackend` | `lua-coro` or `lua-vm` |

Reject local stubs in `lua-stdlib`.

### Unsafe Budget Guard

Expected unsafe locations:

- `lua-gc`: tracing heap and raw allocation/list internals;
- `lua-coro`: native stack switching only if needed;
- `lua-loadlib` or `lua-cli`: dynamic library backend.

No unsafe should be added to `lua-stdlib` for loadlib.

If dynamic loading goes into `lua-cli`, the unsafe-budget file must explicitly
grant a small budget there or define a new crate. Do not let agents bypass the
rule by casting function pointers through `usize` in safe-looking code.

### Stuck-Test Ratchets

Before launching broad test agents, split hard official tests into ratchets:

- `coroutine.lua` subsets: create/status, resume/yield, wrap, close, debug hook.
- `gc.lua` subsets: step-size, weak table pruning, finalizer scheduling,
  generational mode.
- `package/loadlib` subsets: pure Lua require, searchpath, absent dynamic loader,
  dynamic loader with hook.

The harness should count subtest progress, not only whole official-file pass.

## Concrete Next Prompts

### Prompt: GC Budget Design Implementation

```text
Implement budgeted incremental collectgarbage("step", n) semantics.

Scope:
- crates/lua-gc/src/heap.rs
- crates/lua-vm/src/api.rs
- crates/lua-vm/src/state.rs
- tests only as needed

Requirements:
- data in GcArgs::Step adds data * 1024 bytes of debt, matching Lua.
- larger data must perform more collector work than smaller data.
- collectgarbage("step", n) returns 1 only when a cycle reaches pause.
- Preserve existing weak-table/finalizer post-mark hook behavior.
- Do not implement generational GC.
- Do not add unsafe outside lua-gc.
- Add a focused test where dosteps(10) < dosteps(2).
```

### Prompt: Coroutine Thread Identity

```text
Implement real Lua thread identity and coroutine status without native stack
switching.

Scope:
- crates/lua-types/src/value.rs / thread value type as needed
- crates/lua-vm/src/state.rs
- crates/lua-stdlib/src/coro_lib.rs

Requirements:
- new_thread must store the created LuaState in GlobalState and push a real
  Thread handle, not a placeholder.
- coroutine.create moves the function onto the child thread stack.
- implement xmove between threads sharing the same GlobalState.
- implement coroutine.status, running, isyieldable for real thread handles.
- Do not add corosensei yet.
- Do not create a duplicate LuaState type.
```

### Prompt: Pure Lua Resume/Yield

```text
Wire coroutine.resume/yield for pure Lua coroutines using existing do_.rs
lua_resume/lua_yieldk skeletons.

Scope:
- crates/lua-vm/src/do_.rs
- crates/lua-stdlib/src/coro_lib.rs
- VM error/yield boundary handling as needed

Requirements:
- LuaError::Yield is a suspension signal at lua_resume boundaries, not a runtime
  error.
- coroutine.resume returns true + yielded/returned values on success.
- coroutine.resume returns false + error object on coroutine error.
- coroutine.wrap raises errors instead of returning false.
- Preserve non-yieldable C-call boundary errors.
- No native stack switching in this slice unless a test proves it is required.
```

### Prompt: Dynamic Loading Hook

```text
Move loadlib platform dynamic loading behind embedder hooks.

Scope:
- crates/lua-vm/src/state.rs
- crates/lua-stdlib/src/loadlib.rs
- crates/lua-cli/src/main.rs if implementing the CLI backend

Requirements:
- lua-stdlib stays safe and contains no dlopen/dlsym/libloading unsafe calls.
- GlobalState gets optional dynamic library load/symbol/unload hooks.
- package.loadlib preserves C-Lua return shape: function/true on success,
  false + message + "open" or "init" on failure.
- If hook is absent, behavior matches current fallback.
- If a stock Lua C ABI symbol is found but ABI is unsupported, return a clear
  "init" failure instead of calling it.
```

## Final Recommendation

The design posture should be: preserve C-Lua's semantic architecture, but place
Rust's unsafe and platform-specific parts behind explicit capability boundaries.

For coroutines, this means VM-thread semantics first and native stack switching
only where the Rust call stack truly requires it.

For GC, this means matching Lua's public pacing contract before trying to clone
the entire generational collector.

For dynamic loading, this means separating "can open a shared library" from
"can safely run arbitrary Lua C ABI modules." The first is a small host
capability. The second is a major compatibility project.

This gives the harness a clean shape: agents get small executable slices, hooks
reject local stubs and unsafe leaks, and hard official tests become ratcheted
subsystems rather than opaque budget sinks.
