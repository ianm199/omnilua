# Lua System Deep Dive

Status date: 2026-05-18.

This document is a current map of the Lua port: how Lua itself works, how this
Rust workspace models it, what has been built so far, what is still shimmed,
and how to reason about future work without getting lost in the translated
surface area.

It intentionally complements the phase docs. Some older notes describe the
state before Phase D-1 landed; this file describes the checked-in workspace as
of the date above.

## One-Screen Model

Lua is a small language, but the implementation is a dense VM:

```text
source bytes
  -> lexer
  -> parser / code generator
  -> Proto bytecode object
  -> Lua closure
  -> stack-based VM dispatch
  -> Lua values, tables, strings, closures, userdata, coroutines
  -> GC traces everything reachable from the active state and global state
```

The Rust port is split into crates by subsystem:

```text
lua-cli
  -> lua-vm
       -> lua-parse
       -> lua-code
       -> lua-lex
       -> lua-types
       -> lua-gc
  -> lua-stdlib
       -> lua-vm through state_stub re-export shim
       -> lua-types
```

The intended invariant is: every cross-cutting Lua type has one canonical
definition. `LuaValue`, `LuaString`, `LuaProto`, `LuaClosure`, `LuaTable`,
`LuaState`, and `LuaError` must not be locally redefined in downstream crates.
We already hit this failure mode once when `lua-stdlib` grew a fake
`LuaState`; today `state_stub.rs` is a compatibility shim that re-exports the
canonical `lua_vm::state::LuaState`, not a second state type.

## How Lua Works

### Values

Lua runtime values are tagged values. In C-Lua this is `TValue`: a payload plus
a type tag. In this Rust port the main surface is `lua_types::value::LuaValue`.

The important value categories are:

- `nil`: absence of value, also deletes table keys.
- booleans.
- integers and floats. Lua 5.4 has both integer and floating-point numeric
  subtypes, with conversion rules in arithmetic, comparison, and string
  parsing.
- strings. Short strings are interned; long strings are heap allocated and not
  necessarily interned.
- tables. They are the only general object/map/list data structure in Lua.
- functions. Either Lua closures over bytecode prototypes or C/Rust native
  functions.
- userdata. Opaque host data with metatables.
- threads. Lua coroutines, not OS threads.

Lua semantics lean heavily on pointer identity for heap values. Two strings
with the same bytes may be the same interned object; two tables with the same
contents are different values. That is why the port has to treat `GcRef<T>`
identity as a first-class concept.

### Tables

Tables are both arrays and hash maps. C-Lua stores an array part for dense
integer keys and a hash part for everything else. This port's `LuaTable` is in
`lua-types` and currently exposes enough behavior for the VM and stdlib tests:
raw get/set, short-string lookup, integer lookup, `next`, metatable access, and
table identity.

The table surface matters because many unrelated Lua features route through it:

- globals are fields in the global environment table.
- modules are tables.
- objects are usually tables with metatables.
- metatable lookup drives arithmetic metamethods, `__index`, `__newindex`,
  length, calls, comparison, finalization, and weak-table behavior.
- iteration uses the table `next` operation and must match Lua's edge cases.

The current port has a pragmatic table implementation. It is good enough for a
large part of the official suite, but weak tables, exact rehash behavior, and
some `next` edge cases remain part of the hard compatibility surface.

### Strings

Lua strings are byte strings, not Unicode strings. This is important. The Rust
port should treat Lua string data as `&[u8]` / `Vec<u8>` / `RedisString`-style
byte buffers, not `String`, except where it is only formatting diagnostics.

C-Lua interns short strings in the global string table. The port maintains
`GlobalState::interned_lt` so repeated intern operations return the same
`GcRef<LuaString>`. This is not just an optimization: parser constants,
locals, upvalues, table short-string lookup, and identity tests depend on it.

Long strings are allowed to be distinct objects even when their bytes match.
The VM string module has notes around this because string concatenation must
not accidentally intern every long-string result.

### Functions, Protos, and Upvalues

Lua source compiles into a `Proto`: bytecode instructions, constants, nested
protos, local-variable metadata, line info, and upvalue descriptors.

At runtime a Lua function is an `LClosure`:

```text
LuaLClosure
  proto: GcRef<LuaProto>
  upvals: Vec<RefCell<GcRef<UpVal>>>
```

Upvalues are the captured variables that let closures outlive the stack frame
where the variable was created. C-Lua represents open upvalues as links into
the stack and closes them when a scope exits. The Rust port has `UpVal` and
`UpValState`, with enough machinery to pass closure-oriented tests that have
landed. This remains a correctness-sensitive subsystem because bugs often
show up as wrong locals, wrong closure captures, or stack lifetime problems far
from the source of the mistake.

### The VM

Lua is a register VM, not a JVM-style operand-stack VM. A function frame has a
base register, the bytecode indexes registers relative to that base, and calls
reserve stack ranges for arguments/results.

The execution shape is:

```text
LuaState.stack: Vec<StackValue>
LuaState.call_info: Vec<CallInfo>
CallInfo:
  func register
  top register
  saved pc
  result count
  call status flags

VM loop:
  fetch Instruction from current Proto
  decode opcode fields
  operate on registers/constants/upvalues/tables
  maybe call Lua function or C/Rust function
  maybe return/yield/error
```

The crate split is:

- `lua-types/src/opcode.rs`: canonical opcode and instruction vocabulary that
  other crates should converge on.
- `lua-code`: codegen and opcode helper surface.
- `lua-vm/src/vm.rs`: dispatch and execution behavior.
- `lua-vm/src/api.rs`: C API-shaped operations like pushing values, calling,
  protected calls, loading, GC controls, and stack operations.
- `lua-vm/src/state.rs`: `LuaState`, `GlobalState`, call frames, stack, GC
  handle, registry, hooks, and state initialization.

### The State Model

C-Lua has two related state concepts:

```text
lua_State       per thread/coroutine: stack, call frames, open upvalues
global_State    shared universe: registry, strings, metatables, GC heap
```

The C macro `G(L)` gets from a thread state to the global state. In Rust:

```text
LuaState {
  global: Rc<RefCell<GlobalState>>,
  stack,
  call_info,
  open_upvalues,
  status,
  hooks,
  ...
}

GlobalState {
  registry,
  string pools,
  metatables,
  c function registry,
  parser/file-loader hooks,
  heap: lua_gc::Heap,
  ...
}
```

Coroutines should eventually be multiple `LuaState`s sharing one
`GlobalState`. That is why the GC architecture is one heap per global state,
not one heap per OS thread. A thread-local heap is used only as a scoped
migration bridge so old allocation call sites can find the active heap.

## Workspace Map

### `lua-types`

Canonical definitions for cross-crate Lua vocabulary:

- `LuaValue`
- `LuaString`
- `LuaTable`
- `LuaProto`
- `LuaClosure`, `LuaLClosure`, `LuaCClosure`
- `UpVal`
- `LuaUserData`
- `LuaError`
- status/type/opcode/tag-method vocabulary
- `GcRef<T>` and `GcWeak<T>`
- `Trace` impls for GC-rooted types

This crate is the foundation crate. It should not depend on `lua-vm`.
Anything that needs `LuaState` cannot be defined here directly without
creating a dependency cycle. The way out is either:

- keep the type state-free in `lua-types`, or
- define the state-facing adapter in `lua-vm`, or
- use a registry/index indirection, as the C-function registry does.

### `lua-gc`

The trusted GC kernel:

- `Gc<T>`: pointer-sized copy handle.
- `GcBox<T>`: allocation header plus value.
- `GcHeader`: color, finalization bit, intrusive allgc link, byte size.
- `Trace`: trait implemented by every GC-rooted type.
- `Marker`: tracing visitor and gray queue.
- `Heap`: owns the allgc list, byte counters, collection state, and sweep.
- `HeapGuard`: scoped TLS stack pointing at the currently active heap.

Unsafe code is intentionally concentrated here. The goal is not zero unsafe in
the whole project; the goal is a tiny, auditable unsafe kernel with a budget
and tests.

Current important point: `lua_types::gc::GcRef<T>` is now a newtype around
`lua_gc::Gc<T>`, not `Rc<T>`. `GcRef::new(value)` calls
`lua_gc::with_current_heap(...)`; inside a `HeapGuard` it allocates into the
active heap, otherwise it uses `Gc::new_uncollected(value)` as a
bootstrap/leak fallback.

That means the old "paper-only GC" diagnosis is no longer accurate in the
simple form. The current state is better:

- the heap exists on `GlobalState`;
- `GcRef` points at `lua_gc::Gc`;
- `pcall_k` installs a `HeapGuard`;
- full collection can trace `GlobalState` plus active `LuaState`;
- `collectgarbage("collect")` calls through to `Heap::full_collect`;
- weak refs are placeholder semantics;
- finalizers are deferred;
- generational/incremental precision is not complete;
- any allocation outside a guard is still uncollected.

The next correctness question is not "does anything allocate into the heap?"
It is "are all runtime allocation entry points covered by `HeapGuard`, are all
roots traced, and are all barrier/finalizer/weak-table semantics correct?"

### `lua-vm`

The runtime core:

- state creation and initialization;
- stack and call-frame management;
- C API-shaped functions;
- bytecode dispatch;
- table/string helpers;
- debug and dump/undump surfaces;
- parser/file-loader hooks;
- GC handle bridging `collectgarbage` to `lua-gc`.

`LuaState` and `GlobalState` live here. This is the crate where allocator
policy, coroutine strategy, stack discipline, and API compatibility decisions
belong.

`state.rs` now has Phase D state-owned allocation helpers such as
`new_proto`, `new_lclosure`, and upvalue allocation. Some comments still speak
as if `GcRef` were `Rc` because they were written during the migration; read
the implementation, not only the historical comments.

### `lua-lex`, `lua-code`, and `lua-parse`

These crates cover the front end:

```text
lua-lex    tokens from source bytes
lua-code   codegen/opcode helper surface
lua-parse  source -> LuaProto
```

The parser entry used by `lua-cli` is:

```rust
lua_parse::parse(state, DynData::default(), source, name, firstchar)
```

The parser returns a boxed proto, and the CLI wraps it in a Lua closure. Some
of that bootstrap path still calls `GcRef::new` directly; those direct calls
are acceptable only if they run under a `HeapGuard` or are explicitly
bootstrap-only.

Good candidates for static generation live here:

- opcode names and opcode metadata from Lua headers;
- token names and keyword tables;
- parser recovery tables if we want more deterministic translation;
- macro-derived constants that are currently hand-carried.

### `lua-stdlib`

The standard library crate maps Lua C stdlib files to Rust modules:

- `base.rs`
- `string_lib.rs`
- `table_lib.rs`
- `math_lib.rs`
- `io_lib.rs`
- `os_lib.rs`
- `utf8_lib.rs`
- `debug_lib.rs`
- `coro_lib.rs`
- `loadlib.rs`
- `auxlib.rs`
- `init.rs`

The important architectural correction is `state_stub.rs`. It used to be the
kind of local fake type that can make a crate compile while breaking the
workspace. It is now a reconcile shim:

```text
lua_stdlib::state_stub::LuaState == pub use lua_vm::state::LuaState
```

The extension trait in that file supplies missing methods for translated
stdlib code while the real `LuaState` API catches up. Inherent methods on
`LuaState` win when both exist. Over time, the extension trait should shrink
to zero and disappear.

This is a good example of the right escape hatch: keep the compatibility
surface local, but do not duplicate the canonical type.

### `lua-coro`

Coroutine support is separated because it is an architectural decision, not
just another stdlib function. Lua coroutines are userland threads that share a
global state and heap but have separate stacks/call frames.

The hard decision is stackful switching vs continuation/trampoline design:

- A C-faithful port naturally wants stackful coroutines.
- Rust stackful switching typically uses a crate like `corosensei` or a
  similar context-switch mechanism.
- A fully branded GC approach like `gc-arena` tends to push toward a stackless
  VM/trampoline design.

The current port has not completed Phase E coroutines. Tests such as
`literals` and `nextvar` are known to depend on real coroutine behavior.

### `lua-cli`

Minimal executable:

```text
new_state()
set parser_hook
set file_loader_hook
open_libs()
load_string()
pcall_k()
```

It is not a complete `lua.c` clone. It is a narrow integration executable whose
job is to force the crates to wire together and expose type/architecture
breakage that per-crate `cargo check` can miss.

## Execution Pipeline

For `lua-rs 'print("hello")'`, the expected flow is:

```text
lua-cli main
  new_state()
    creates LuaState and GlobalState
    initializes registry, globals, string cache, GC heap
  open_libs(&mut state)
    stdlib init registers base/string/table/math/etc.
  load_string(&mut state, source)
    auxlib invokes parser hook
    lua-parse builds LuaProto
    CLI wraps Proto in LuaLClosure
    closure is pushed onto stack
  pcall_k(&mut state, 0, MULTRET, ...)
    installs HeapGuard
    enters protected call
    VM dispatch runs closure bytecode
    native print is called through stdlib/C-function bridge
```

The key property is that this path crosses every major crate. This is why the
CLI is more valuable than it looks: per-crate checks can pass while the first
real consumer reveals incompatible type islands.

## Garbage Collection: Current Design

### C-Lua's Choice

C-Lua passes `lua_State *L` almost everywhere. Allocators use `G(L)` to reach
the shared global state and heap. C makes this cheap because raw pointers and
macros hide the dependency.

Rust makes the dependency visible. Constructors that used to be pure now need
some route to the heap:

- explicit `&mut LuaState`;
- explicit `&Heap`;
- a scoped TLS bridge;
- or a larger branded-lifetime GC architecture.

The locked direction here is one heap per `GlobalState`, with a TLS bridge only
as migration machinery.

### Current Rust Shape

```text
GlobalState.heap: lua_gc::Heap

pcall_k:
  let _heap_guard = HeapGuard::push(&global.heap)
  run protected call

GcRef::new(value):
  with_current_heap(|heap| if heap exists:
    heap.allocate(value)
  else:
    Gc::new_uncollected(value))

collectgarbage("collect"):
  state.gc().full_collect()
    roots = { &GlobalState, &LuaState }
    heap.full_collect(&roots)
```

### Soundness Invariant

`Gc<T>` is a raw, copyable handle under a safe wrapper. The safety story relies
on a safepoint rule:

```text
No collection may run while untraced temporary Gc<T> handles are live outside
the root graph.
```

The practical version:

- `Heap::allocate` should not collect.
- collection happens only at explicit safepoints (`collectgarbage`, later VM
  dispatch boundaries);
- every live heap reference reachable from the stack, call frames, globals,
  registry, metatables, intern pools, upvalues, and closures must be traced;
- unsafe stays inside `lua-gc`.

### Unsafe Audit Snapshot

Current unsafe budgets:

- `lua-gc`: 13 counted sites, all in `heap.rs`;
- `lua-cli`: 5 counted sites for the `libloading` dynamic-library backend;
- `lua-coro`: 0, with `unsafe_code = "forbid"` until a concrete stackful
  backend lands.

What was retired during the audit:

- the public `current_heap() -> Option<&'static Heap>` lifetime lie became
  closure-scoped `with_current_heap(...)`;
- reference-only `gc.rs`/`mem.rs` partial ports were removed from
  `crates/lua-gc/src/` so source scans only count compiled code;
- `lua-coro` no longer carries a speculative unsafe budget;
- `Heap::allocate` sets `header.next` while the `Box` is still owned, before
  `Box::into_raw`, avoiding an unnecessary raw-pointer write.

The remaining `lua-gc` unsafe sites are the expected trusted kernel:

- converting the scoped `HeapGuard` TLS pointer back to `&Heap`;
- dereferencing `Gc<T>` handles to reach their `GcBox<T>`;
- dereferencing gray-queue and allgc-list `NonNull<GcBox<dyn Trace>>` nodes;
- maintaining the sweep cursor, which points at an intrusive `next` cell;
- reclaiming unreachable boxes with `Box::from_raw`.

Reduction rules for future audits:

- never budget dead or uncompiled files; delete or move reference material out
  of `src/`;
- never return a fake `'static` reference from TLS or registry state; use a
  closure-scoped API so the borrow cannot escape;
- before using raw pointers, ask whether the value is still owned as a `Box` or
  ordinary Rust reference, and do the mutation there instead;
- raise `harness/unsafe-budgets.toml` only in the same patch that introduces a
  real compiled unsafe site and its `SAFETY` argument;
- after changing this surface, run `.claude/hooks/unsafe-budget.sh`,
  `cargo test -p lua-gc`, and the official suite.

### Known Gaps

- `GcWeak<T>` is a placeholder. `upgrade()` always returns `Some`.
- weak table semantics are not complete.
- finalizers (`__gc`) are not the full Lua finalization/resurrection model.
- generational mode is not implemented as real generational GC.
- write barriers exist as scaffolding, but correctness still depends on the
  exact mutation sites being covered.
- bootstrap allocations outside a `HeapGuard` use `new_uncollected`.

These gaps do not invalidate Rust memory safety by themselves. They are
semantic/runtime-compatibility gaps. The memory-safety risk lives in the GC
unsafe kernel; that is why the unsafe budget and root tracing tests matter.

## Harness and Agent Method

The harness strategy that has worked:

```text
1. Translate a bounded subsystem.
2. cargo check the crate/workspace.
3. Run integration/official tests.
4. Capture the first panic, wrong output, or compile failure.
5. Dispatch a focused agent with the trace and a narrow write scope.
6. Reject regressions.
7. Record architectural blockers separately from grindable bugs.
```

Important harness lessons from this repo:

- Per-crate `cargo check` is not enough. It misses duplicated nominal types.
- Workspace `cargo check` is still not enough. If no consumer wires the two
  sides together, incompatible islands can coexist.
- A CLI or integration binary is an architectural test.
- A type vocabulary registry should reject fake duplicate definitions.
- Unsafe budgets are useful because they force `unsafe` into named crates.
- Static generation is better than asking an LLM to recopy tables.
- Agent loops hit a ceiling when failures require architectural decisions
  rather than local bug fixes.

Current measured/observed state from recent runs:

- `cargo check --workspace` passes, with many stdlib warnings.
- The official suite was recently at 17/30 passing after Phase D progress.
- Newly passing tests in that overnight run included `bitwise`, `closure`,
  `events`, `goto`, `pm`, and `sort`.
- Remaining official failures cluster around real GC edge cases, coroutines,
  debug library depth, files/io/os gaps, string patterns, math/stdlib corners,
  compound statement/codegen bugs, and call/tail-call behavior.

## What Is Built So Far

Substantial pieces that are real:

- multi-crate Rust workspace;
- canonical type crate;
- parser/lexer/codegen pipeline sufficient for nontrivial programs;
- VM state, stack, call info, protected call path;
- many Lua value operations;
- table/string/function/upvalue model;
- standard library modules with many functions ported;
- CLI integration path;
- mark-sweep GC crate and `GcRef` backend swap to `lua_gc::Gc`;
- `HeapGuard` stack bridge;
- root tracing for major state/value types;
- official and frontier harness loops;
- type-duplication lesson encoded in process, if not yet fully automated.

Substantial pieces still partial:

- full coroutine semantics;
- full debug hooks and `lua_getinfo`/`lua_sethook` parity;
- full `io`/`os` behavior;
- exact string pattern matching;
- exact table weak behavior;
- finalizers;
- generational GC;
- full API parity;
- removal of stdlib extension-trait reconcile shim;
- static generation for all duplicated C tables/constants.

## How To Understand Failures

Classify every failure before assigning it to agents:

1. **Compile surface failure.**
   Missing method, wrong signature, trait bound, or crate dependency. Usually
   agent-grindable unless it implies a new dependency edge.

2. **Type vocabulary failure.**
   A crate invented a local `LuaState`, `LuaTable`, `Instruction`, etc. This
   is not a local compile bug. It is an architecture violation.

3. **Semantic local bug.**
   One opcode, one stdlib function, one conversion rule. Good agent target.

4. **Cross-subsystem semantic bug.**
   Example: `pcall` plus stack unwinding plus error object plus traceback.
   Needs a human-designed target before agents are useful.

5. **Architecture-blocked failure.**
   GC, coroutines, debug hooks, finalizers, weak tables. Do not burn loop
   budget pretending these are one-file fixes.

6. **Harness artifact.**
   Timeout wrapper, stuck agent, bad test isolation, wrong oracle. Fix the
   harness before interpreting the result.

## Near-Term Engineering Priorities

1. Reconcile the current Phase D docs with the actual D-1e state so future
   agents do not follow stale `Rc` instructions.

2. Audit `GcRef::new` call sites. There are still direct call sites. Decide
   which are allowed bootstrap allocations and which should move to
   `LuaState` allocation helpers.

3. Add a harness check that bans new direct `GcRef::new` outside an explicit
   whitelist.

4. Expand GC root tests. In particular: table cycles, closure/upvalue cycles,
   registry roots, stack roots, metatable roots, interned strings, and userdata
   finalizer placeholders.

5. Finish stdlib-state reconciliation. `state_stub.rs` should keep
   re-exporting canonical `LuaState`, but the extension trait should shrink.

6. Make coroutine architecture decision before dispatching more agents against
   coroutine-dependent tests.

7. Static-generate opcode/token/config tables rather than letting agents
   hand-maintain copied constants.

8. Keep the CLI and official tests as required integration gates. They are the
   cheapest signal for cross-crate architecture drift.

## Long-Term Shape

The credible end state is:

```text
lua-types
  canonical values and bytecode vocabulary

lua-gc
  small unsafe kernel, tested and budgeted

lua-vm
  one canonical LuaState / GlobalState / VM / API

lua-parse + lua-code + lua-lex
  generated/static where possible, hand-written where semantic

lua-stdlib
  no local state shims except imports of canonical state

lua-coro
  explicit coroutine implementation, chosen once

harness
  official suite, frontier tests, type-vocab guard, unsafe budget,
  regression abort, trace-driven agent dispatch
```

The port should not aim to be "Rust-flavored Lua" first. It should first be a
faithful Lua 5.4 runtime with a narrow trusted unsafe core. Once compatibility
is stable, Rust becomes useful because future changes can be made behind
stronger tests, clearer ownership, and better architectural boundaries than
the C original exposes.
