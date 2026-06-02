---
title: Ghost Abstraction Register
version: 1
updated: 2026-05-19
format: bidirectional-check-v1
---

# Ghost Abstraction Register

Each entry below is a named ghost abstraction: a temporary or no-op
implementation that was correct during an earlier phase but must be replaced
before the port is semantically complete. Entries carry enough metadata for
`harness/check_ghost_abstractions.sh` to detect drift in both directions —
new code landing on top of a ghost, and ghosts whose patterns no longer
appear in the codebase (ready for retirement).

Naming convention: kebab-case ids are the machine-readable primary key.
Pattern regex values use ERE (grep -E) syntax, escaped for shell use.

## Session 2026-05-19 Late retirements

- `flat-table-grow-cap` → **retired**. Canonical `LuaTable` now lives in `lua-types/src/table.rs` with interior mutability; the FLAT_TABLE_GROW_CAP hack in `state.rs::LuaTableRefExt` is gone; `TOTAL_GROW_CAP=1<<20` in lua-types replaces it.
- `reset-thread-phase-a-skip` → **retired**. `state.rs::reset_thread` now calls `do_::close_protected` + `do_::set_error_obj`, matching C-Lua's `luaE_resetthread`. Discovered + fixed by H-3a.
- `thread-reachability-promise` → **retired**. F-1.c wired `LuaValue::Thread` tracing plus a post-mark fixed-point hook that traces only reachable suspended coroutine stacks.

## Outstanding active entries by priority

| Priority | Ghost | Cost to retire |
|---|---|---|
| 2 | `gc-barrier-noops` | $30 — wire `luaC_barrier`/`luaC_barrierback` under incremental GC |
| 3 | `gc-phase-predicates-always-constant` | $20 — wire `keep_invariant`/`is_sweep_phase` against real `gcstate` |
| 4 | `dual-instruction-type` | $15 — unify `Instruction` between lua-types and lua-code |
| 4 | `dual-lex-types` | $80 — same pattern as the LuaTable refactor: canonicalize `LexBuffer`/`LexState`/`ZIO` |
| 4 | `dual-luadebug-type` | $30 — unify `LuaDebug` |
| 5 | `extension-trait-shim-layer` | death-by-1000-cuts, retire opportunistically |
| 5 | `state-stub-trait-defaults-with-todo` | active by design |
| 5 | `scattered-phase-b-todos-in-code` | one remaining at auxlib.rs:1373 ($5 sonnet) |

## Flat Table Grow Cap

```yaml
name: flat-table-grow-cap
files:
  - crates/lua-vm/src/state.rs:2190
  - crates/lua-vm/src/state.rs:2200
  - crates/lua-vm/src/state.rs:2226
  - crates/lua-vm/src/state.rs:2233
  - crates/lua-types/src/value.rs:122
patterns:
  - "Vec<.K,V.> placeholder"
  - "flat Vec<.K,V.>"
  - "single Vec.*placeholder.*no separate"
  - "Phase B.*LuaTable is a flat"
canonical_owner: crates/lua-vm/src/table.rs:229
why: Phase-B LuaTable in lua-types is a Vec<(K,V)> with no array/hash split; the real array+hash impl lives in lua-vm/src/table.rs and is not yet wired to the types-crate path.
retirement_trigger: Canonical vm::table::LuaTable (array+hash) wired into LuaTableRefExt and all direct lua-types::LuaTable dispatch removed.
test_gate: nextvar.lua reaches past for-loop init; heavy.lua PASSes under cargo test.
priority: 1
status: retired (2026-05-19, canonical LuaTable now lives in lua-types/src/table.rs with interior mutability; TOTAL_GROW_CAP replaces FLAT_TABLE_GROW_CAP)
```

The Phase-B `LuaTable` in `crates/lua-types/src/value.rs` is a single
`Vec<(K,V)>` behind a `RefCell`. It has no separate array part, no hash
part, and no growth backpressure. Large tables hit O(n) scans on every
read/write and there is no `LUA_ERRMEM` guard. The real implementation
already exists in `crates/lua-vm/src/table.rs` with a full array+hash
layout matching `struct Table` in `lobject.h`, but it is not reachable
from the lua-types dispatch path that most stdlib code hits.

The parallel table-refactor agent is actively reconciling these two impls.
When it lands, every pattern grep for `Vec<(K,V)> placeholder` should return
zero hits. At that point this entry can be promoted to `retired`.

Reference C source: `ltable.c`, `struct Table` / `luaH_resize` in `lobject.h`.

---

## GC Barrier No-ops

```yaml
name: gc-barrier-noops
files:
  - crates/lua-vm/src/state.rs:2789
  - crates/lua-vm/src/state.rs:2795
  - crates/lua-vm/src/state.rs:2801
  - crates/lua-vm/src/state.rs:2806
  - crates/lua-vm/src/state.rs:2340
  - crates/lua-vm/src/state.rs:2341
patterns:
  - "pub fn barrier\b.*\{ *\}"
  - "pub fn barrier_back\b.*\{ *\}"
  - "pub fn obj_barrier\b.*\{ *\}"
  - "gc_barrier_back.*phase-b no-op"
  - "gc_barrier_upval.*phase-b no-op"
canonical_owner: crates/lua-vm/src/state.rs:2789
why: Write barriers are Phase-D infrastructure; luaC_barrier and luaC_barrierback are never wired during incremental GC bringup so all four barrier variants are empty bodies.
retirement_trigger: Real luaC_barrier / luaC_barrierback implementations under incremental GC (Phase D state machine).
test_gate: gengc.lua PASSes, proving the barrier is doing real work even if generational mode itself is shimmed.
priority: 2
status: active
```

In C-Lua, `luaC_barrier` (forward barrier) and `luaC_barrierback` (backward
barrier) maintain the tri-color invariant during incremental GC. A black
object must not point to a white object — the barrier either re-grays the
parent (`barrierback`) or immediately marks the child (`barrier`). All four
variants (`barrier`, `barrier_back`, `obj_barrier`, `obj_barrier_back`) are
currently empty `{}` bodies on the `GcState` type. There is also a pair of
`gc_barrier_back` and `gc_barrier_upval` no-ops on `LuaState` itself (lines
2340–2341). Without real barriers, incremental GC will silently miss
write-after-mark mutations.

Reference C source: `lgc.c::luaC_barrier`, `lgc.c::luaC_barrierback`.

---

## GC Phase Predicates Always Constant

```yaml
name: gc-phase-predicates-always-constant
files:
  - crates/lua-vm/src/state.rs:1083
  - crates/lua-vm/src/state.rs:1092
patterns:
  - "pub fn keep_invariant"
  - "pub fn is_sweep_phase"
  - "TODO.*Phase D.*check gcstate"
canonical_owner: crates/lua-vm/src/state.rs:1083
why: Phase-D predicates that gate write-barrier and finalizer behavior; never wired because the GC state machine is not yet ported.
retirement_trigger: GC state machine fully ported; gcstate field drives real transitions.
test_gate: Same as gc-barrier-noops (gengc.lua PASSes).
priority: 2
status: active
```

`GlobalState::keep_invariant()` returns `false` unconditionally (should
check that `gcstate` is in a propagation phase). `GlobalState::is_sweep_phase()`
returns `false` unconditionally (should check for `GCSswpallgc` and related
states). These predicates gate whether `luaC_barrier` does real work and
whether finalizers can be scheduled. Because both return false, no barrier
logic fires and no finalizer path is guarded correctly, even once the barrier
bodies above are filled in.

Reference C macros: `keepinvariant(g)`, `issweepphase(g)` in `lgc.h`.

---

## GcWeak Always Returns Some

```yaml
name: gcweak-always-some
files:
  - crates/lua-types/src/gc.rs
  - crates/lua-gc/src/heap.rs
patterns:
  - "pub fn upgrade.*-> Option"
  - "contains_allocation"
canonical_owner: crates/lua-types/src/gc.rs
why: Retired. Heap-tracked GcWeak handles remember their heap, target identity, and heap allocation token; upgrade returns None after sweep removes that exact allocation.
retirement_trigger: Met by heap identity/token checks in GcWeak::upgrade.
test_gate: lua-types weak upgrade test plus weak-table canaries.
priority: 4
status: retired
```

`GcWeak<T>` now stores the target identity, heap allocation token, and the
heap that was active when the handle was created. `upgrade()` asks that heap
whether the same identity/token pair is still live; once sweep removes the
box, upgrade returns `None` without dereferencing the freed pointer. The
token prevents allocator address reuse from reviving a stale weak handle.
Legacy uncollected boxes still upgrade forever, matching their
process-lifetime allocation model.

Reference C source: `lstate.h` weak-pointer handling, `lgc.c` weak table
clearing in `clearbykeys`.

---

## Thread Reachability Promise

```yaml
name: thread-reachability-promise
files:
  - crates/lua-vm/src/trace_impls.rs:105
  - crates/lua-vm/src/state.rs:924
patterns:
  - "for entry in self\.threads\.values"
  - "entry\.state\.try_borrow"
canonical_owner: crates/lua-vm/src/trace_impls.rs:105
why: The GlobalState::trace impl traces all threads entries unconditionally, but C-Lua only traces threads reachable from the program; an unreachable suspended coroutine should be collectable.
retirement_trigger: Hook implementation verified — only reachable threads (reached via stack or coroutine value chain) are traced; canary test covering coroutine reachability passes.
test_gate: A dedicated canary that creates an unreachable suspended coroutine and asserts it is collected (collectgarbage() returns it as dead).
priority: 3
status: retired (2026-05-19, F-1.c implemented reachable-thread fixed-point trace hook)
```

In `crates/lua-vm/src/trace_impls.rs` the `GlobalState::trace` implementation
iterates `self.threads.values()` and traces every entry unconditionally
(lines 105–110). In C-Lua (`lgc.c::traversethread` and the main mark loop),
a suspended coroutine that is not reachable from the main stack or any live
closure is left untraced and collected. The current port keeps all threads
alive as strong roots for the lifetime of `GlobalState`, which means a
leaked coroutine is a permanent memory leak. The comment at line 147 of
`trace_impls.rs` describes the intended post-mark hook for finalizers but
the analogous hook for threads is absent.

Reference C source: `lgc.c::propagatemark`, `lgc.c::traversethread`,
thread handling in `ldo.c::luaD_closethread`.

---

## Extension Trait Shim Layer

```yaml
name: extension-trait-shim-layer
files:
  - crates/lua-vm/src/state.rs:346
  - crates/lua-vm/src/state.rs:411
  - crates/lua-vm/src/state.rs:436
  - crates/lua-vm/src/state.rs:468
  - crates/lua-vm/src/state.rs:507
  - crates/lua-vm/src/state.rs:520
  - crates/lua-vm/src/state.rs:531
  - crates/lua-vm/src/state.rs:541
  - crates/lua-vm/src/state.rs:555
patterns:
  - "pub trait LuaTableRefExt"
  - "pub trait LuaUserDataRefExt"
  - "pub trait LuaStringRefExt"
  - "pub trait LuaLClosureRefExt"
  - "pub trait LuaClosureExt"
  - "pub trait LuaProtoExt"
  - "pub trait LuaValueExt"
  - "pub trait LuaTypeExt"
  - "pub trait StackIdxExt"
canonical_owner: crates/lua-vm/src/state.rs:346
why: Reconciliation surface for adapting lua-types-crate values to vm-crate methods; exists because the types crate cannot depend on the vm crate.
retirement_trigger: Each trait shrinks to zero methods as canonical methods land on the types-crate types directly, or the crate boundary is resolved.
test_gate: None per-trait; verify each retirement via cargo build and smoke set.
priority: 5
status: active
```

Nine extension traits in `state.rs` — `LuaValueExt`, `LuaTypeExt`,
`StackIdxExt`, `LuaTableRefExt`, `LuaUserDataRefExt`, `LuaStringRefExt`,
`LuaLClosureRefExt`, `LuaClosureExt`, and `LuaProtoExt` — exist because the
`lua-types` crate cannot depend on `lua-vm`, so methods that need both
cannot live on the types-crate types directly. As the canonical API
stabilises, methods should migrate from these traits to inherent methods on
the types themselves (or to a dedicated adapter layer). Until then the traits
are legitimate plumbing, not bugs. The risk is methods being added here
rather than fixed upstream, slowly enlarging the shim layer indefinitely.

---

## State Stub Trait Defaults With Todo

```yaml
name: state-stub-trait-defaults-with-todo
files:
  - crates/lua-stdlib/src/state_stub.rs:98
  - crates/lua-stdlib/src/state_stub.rs:99
  - crates/lua-stdlib/src/state_stub.rs:100
  - crates/lua-vm/src/api.rs:511
  - crates/lua-vm/src/api.rs:518
  - crates/lua-parse/src/lib.rs:451
  - crates/lua-parse/src/lib.rs:542
  - crates/lua-lex/src/lib.rs:1936
  - harness/reconcile_types.sh:96
  - harness/reconcile_types.sh:107
patterns:
  - "todo!.*phase-b-reconcile"
  - "TODO_ARCH.*phase-b-reconcile"
canonical_owner: crates/lua-stdlib/src/state_stub.rs:91
why: Trait defaults for LuaStateStubExt that must be overridden by inherent methods on LuaState; the todo!() fires only if an override is missing, which is correct behavior.
retirement_trigger: Every method in the trait has a corresponding inherent method on LuaState; the trait can then be deleted.
test_gate: Build and smoke set; a missing override surfaces as a panic at first invocation.
priority: 3
status: active
```

`crates/lua-stdlib/src/state_stub.rs` defines `LuaStateStubExt`, a large
extension trait whose bodies are all `todo!("phase-b-reconcile: …")`. The
design is intentional: Rust method resolution prefers inherent methods over
trait methods, so any method that has been migrated to `LuaState` itself
silently takes precedence. The `todo!()` is a canary — it fires if and only
if an override is absent. This entry exists to declare the pattern as
expected, not to label it a bug. The work to retire it is landing inherent
methods one at a time until the trait has zero surviving `todo!()` bodies.

Reference: `crates/lua-stdlib/src/state_stub.rs` module-level doc comment.

---

## Reset Thread Phase A Skip

```yaml
name: reset-thread-phase-a-skip
files:
  - crates/lua-vm/src/state.rs:3523
patterns:
  - "For Phase A, skip the actual close"
canonical_owner: crates/lua-vm/src/state.rs:3521
why: Phase-A placeholder skipped the actual close of to-be-closed upvalues in reset_thread because ldo.c was not yet ported.
retirement_trigger: H-3a fix landed; close_protected called from do_.rs wired into reset_thread.
test_gate: coroutine.lua PASS (exercises coroutine close on error path).
priority: 1
status: retired (2026-05-19, H-3a integration wired close_protected and set_error_obj into reset_thread)
```

The `reset_thread` function at line 3523 carries a `TODO(port)` comment:
`For Phase A, skip the actual close (upvalue closing requires ldo.c)`. The
upvalue-closing call to `luaD_closeprotected` is absent. Until `do_.rs`
provides `close_protected`, coroutines that raise errors while holding
to-be-closed upvalues will not run the `__close` metamethod. This entry was
originally expected to be `retired` (H-3a fix) but the pattern still hits
in the current tree, so it remains `active` until the grep returns zero.

Reference C source: `ldo.c::lua_resetthread`, `ldo.c::luaD_closeprotected`.

---

## Interned String Strong Root Override

```yaml
name: interned-string-strong-root-override
files:
  - crates/lua-vm/src/trace_impls.rs
  - crates/lua-vm/src/state.rs
patterns:
  - "record_live_interned_strings"
  - "retain_live_interned_strings"
canonical_owner: crates/lua-vm/src/state.rs
why: Retired. The interned_lt short-string cache is now weak: root tracing skips it, post-mark records marked interned strings, and unreachable cache entries are removed.
retirement_trigger: Met by post-mark interned-string pruning in full, minor, incremental atomic, and mark-only weak cleanup paths.
test_gate: VM tests that keep a rooted short string cached and collect an unreferenced short string/cache entry.
priority: 4
status: retired
```

`GlobalState::interned_lt` holds the per-process short-string identity cache.
In C-Lua, `strt` (the hash table of interned strings) is treated as a weak
table during GC: entries are cleared by `clearbykeys` during the atomic
phase, not marked as roots. The port now follows that shape for
`interned_lt`: `GlobalState::trace` skips the cache, the collector's
post-mark hooks record cache entries whose strings were marked by real roots,
and the post-collection cleanup removes unmarked entries by pointer identity.
This avoids pinning every short string ever interned while preventing stale
cache entries from being reused after sweep.

Reference C source: `lgc.c::clearbykeys`, `lstring.c` intern table (`strt`).

---

## Opcode Reexport Shim

```yaml
name: opcode-reexport-shim
files:
  - crates/lua-parse/src/lib.rs:451
  - crates/lua-parse/src/lib.rs:542
patterns:
  - "TODO_ARCH.*phase-b-reconcile.*re-export"
  - "TODO_ARCH.*phase-b-reconcile.*canonical OpCode"
  - "TODO_ARCH.*phase-b-reconcile.*canonical LuaState"
canonical_owner: crates/lua-code/src/opcodes.rs:1
why: lua-parse re-exports OpCode from lua-code and LuaState from lua-vm via TODO_ARCH markers; these are phase-b-reconcile shims not yet resolved into clean crate boundaries.
retirement_trigger: lua-parse uses lua-code and lua-vm as direct dependencies with no shim re-exports; the TODO_ARCH comments disappear.
test_gate: cargo build with no warnings from the re-export lines; smoke set passes.
priority: 5
status: active
```

Two `TODO_ARCH(phase-b-reconcile)` re-exports exist in
`crates/lua-parse/src/lib.rs`: `pub use lua_code::opcodes::OpCode` (line 451)
and `pub use lua_vm::state::LuaState` (line 542). These are documented as
temporary architectural shims that exist so `lua-parse` call sites can
compile while the canonical crate dependencies are finalised. They are not
dangerous but do mean that `lua-parse` callers see `OpCode` as originating
from `lua-parse` rather than `lua-code`, making the dependency graph
misleading.

---

## Dual Instruction Type

```yaml
name: dual-instruction-type
files:
  - crates/lua-types/src/opcode.rs:1
  - crates/lua-code/src/opcodes.rs:1
patterns:
  - "pub struct Instruction"
canonical_owner: crates/lua-code/src/opcodes.rs:1
why: Instruction is defined in both lua-types and lua-code; the lua-types copy is a Phase-A placeholder that predates the canonical lua-code opcodes crate.
retirement_trigger: lua-types::opcode::Instruction removed; all call sites use lua-code::opcodes::Instruction.
test_gate: cargo build; smoke set passes.
priority: 3
status: active
```

`pub struct Instruction` exists in both `crates/lua-types/src/opcode.rs`
and `crates/lua-code/src/opcodes.rs`. The canonical home is `lua-code`;
the `lua-types` copy is a Phase-A leftover. Dispatch may silently hit the
wrong struct if a consumer does `use lua_types::opcode::Instruction` instead
of `use lua_code::opcodes::Instruction`.

---

## Dual Lex Types

```yaml
name: dual-lex-types
files:
  - crates/lua-lex/src/lib.rs:1
  - crates/lua-vm/src/zio.rs:1
  - crates/lua-parse/src/lib.rs:1
patterns:
  - "pub struct LexBuffer"
  - "pub struct LexState"
  - "pub struct ZIO"
canonical_owner: crates/lua-lex/src/lib.rs:1
why: LexBuffer, LexState, and ZIO are defined in both lua-lex and lua-vm/lua-parse as Phase-A stubs; canonical types belong in lua-lex.
retirement_trigger: lua-vm/zio.rs and lua-parse stub LexState removed; all consumers use lua-lex types.
test_gate: cargo build; lexer tests pass.
priority: 4
status: active
```

`LexBuffer` and `ZIO` exist in both `crates/lua-lex/src/lib.rs` and
`crates/lua-vm/src/zio.rs`. `LexState` exists in both
`crates/lua-lex/src/lib.rs` and `crates/lua-parse/src/lib.rs`. These are
Phase-A stubs created before `lua-lex` was fleshed out. The canonical
location is `lua-lex`; the duplicates in `lua-vm` and `lua-parse` are
ghosts.

---

## Dual LuaDebug Type

```yaml
name: dual-luadebug-type
files:
  - crates/lua-stdlib/src/state_stub.rs:1
  - crates/lua-vm/src/debug.rs:1
patterns:
  - "pub struct LuaDebug"
canonical_owner: crates/lua-vm/src/debug.rs:1
why: LuaDebug is defined in both state_stub.rs (Phase-A shim) and debug.rs (canonical); the stub shadowed the canonical type during Phase-A stdlib translation.
retirement_trigger: state_stub.rs LuaDebug removed; all debug-lib call sites use lua_vm::debug::LuaDebug.
test_gate: cargo build; debug library functions callable from Lua.
priority: 4
status: active
```

`pub struct LuaDebug` appears in both `crates/lua-stdlib/src/state_stub.rs`
and `crates/lua-vm/src/debug.rs`. The canonical home is `debug.rs`; the
`state_stub.rs` copy is a Phase-A shim added so stdlib code compiled while
`debug.rs` was being ported. They may have diverged in structure.

---

## Scattered Phase-B Todos In Code

```yaml
name: scattered-phase-b-todos-in-code
files:
  - crates/lua-vm/src/object.rs:270
  - crates/lua-stdlib/src/auxlib.rs:1373
  - crates/lua-vm/src/tagmethods.rs:255
  - crates/lua-vm/src/tagmethods.rs:266
patterns:
  - "todo!.*phase-b: TagMethod"
  - "todo!.*phase-b: LuaState"
  - "todo!.*phase-b: LuaTable"
canonical_owner: crates/lua-vm/src/state.rs:312
why: A small number of todo!("phase-b:") callsites exist outside state.rs and state_stub.rs where specific cross-crate helpers are missing.
retirement_trigger: Each todo!() replaced by a real implementation; the files list shrinks to zero.
test_gate: cargo build with no remaining todo! panics; affected stdlib paths exercised by smoke set.
priority: 3
status: active
```

Three files outside the main `state.rs` / `state_stub.rs` pair carry
`todo!("phase-b: …")` markers that fire if reached at runtime:

- `object.rs:270` — `TagMethod::from_arith_op` conversion needed for
  arithmetic metamethod dispatch.
- `auxlib.rs:1373` — `LuaState::new()` used by `luaL_newstate` in the
  auxiliary library.
- `tagmethods.rs:255` — `LuaTable::get_short_str` needed for metamethod
  lookups via the flat placeholder table.

These are real runtime panics waiting to happen on any path that exercises
the relevant operations. `tagmethods.rs:266` is in a comment and does not
fire, but is tracked here for completeness.

---

## Retirement Plan

### Top-5 Ghosts by Retirement Leverage

**1. Dual LuaTable Implementation (flat-table-grow-cap + extension-trait-shim-layer surface)**

This is the single most impactful retirement because it unblocks everything
downstream: `nextvar.lua`, `heavy.lua`, and any program that creates tables
larger than a few hundred entries. The parallel table-refactor agent is
already in flight. Retirement looks like: delete `crates/lua-types/src/value.rs::LuaTable`
entirely and route all table dispatch through `crates/lua-vm/src/table.rs::LuaTable`
via `GcRef`. The `LuaTableRefExt` trait methods in `state.rs` become thin
delegation wrappers until the vm-crate table API is stable, then they too
disappear. Estimated cost: 2–4 agent sessions; the hard part is updating
every call site that currently constructs `LuaTable::placeholder()` from
the types crate.

**2. GC Barriers (gc-barrier-noops + gc-phase-predicates-always-constant)**

These two ghosts are coupled — `keep_invariant()` must return `true` during
propagation phases or the barrier bodies are unreachable dead code even after
they're filled in. Retirement means: (a) port the GC state machine so
`gcstate` drives `keep_invariant` and `is_sweep_phase`, then (b) implement
the four barrier bodies to re-gray or immediately mark as appropriate.
Without real barriers, any future incremental GC work will be silently
incorrect. Estimated cost: 3–5 agent sessions — one for the state machine,
one for each barrier family, one for integration testing via `gengc.lua`.

**3. Thread Reachability Promise (thread-reachability-promise)**

Relevant to the `aux_resume` upvalue-flush fix already in flight. The fix
ensures coroutine close paths run `__close` correctly, but if the collecting
side never actually collects unreachable coroutines the fix is invisible.
Retirement: remove the unconditional `for entry in self.threads.values()`
trace and instead trace only threads reachable via the live value graph; add
a post-mark hook analogous to the finalizer hook that removes dead thread
registry entries. Estimated cost: 1–2 agent sessions; the shape is already
visible in the finalizer post-mark hook at line 2507 of `state.rs`.

**4. GcWeak Always Returns Some (gcweak-always-some) — retired**

Heap-tracked `GcWeak` handles now upgrade only while their target identity and
heap allocation token are still live on the owning heap. This retires the
always-`Some` behavior for weak-table registry entries. The separate
`interned-string-strong-root-override` ghost remains active.

**5. Extension Trait Shim Layer (extension-trait-shim-layer)**

Death by 1000 cuts — retire opportunistically. Each time an agent is already
working in `state.rs`, move one or two methods from the `*RefExt` traits to
inherent methods on the types-crate types (or delete them if the canonical
vm-crate method covers it). There is no single "retirement event" here; the
goal is zero traits remaining in `state.rs` that have `TODO(phase-b)` in
their doc. Estimated cost: negligible per method; cumulative cost spread
across many sessions.
