# Issue #93: Generational GC Plan

Status audited on `main` after PR #109 (`5309cd1`).

## What #109 Fixed

PR #109 fixed the observable startup-mode half of #93:

- Hosted Lua 5.4 and 5.5 switch to reported generational mode after
  `open_libs`, matching the upstream standalone startup path in
  `reference/lua-5.4.7/src/lua.c`.
- Raw `new_state()` still initializes `gckind` as incremental, matching
  `reference/lua-5.4.7/src/lstate.c`.
- `multiversion_oracle.rs` now verifies the mode round trip:
  `generational|incremental|generational`.

This does not implement generational collection.

## Spot-Check Findings

The current public surface still has only generational scaffolding:

- `GcKind::{Incremental, Generational}` exists.
- `collectgarbage("generational"|"incremental")` switches the mode flag.
- `genminormul`, `genmajormul`, `lastatomic`, and `gc_estimate`-like fields
  exist in `GlobalState`.
- `GcArgs::Step` still always calls `state.gc().incremental_step(work_units)`.
- Generational mode only adds `prune_weak_tables_mark_only()` after that
  incremental step.
- `GlobalState::is_gen_mode()` checks only `gckind == Generational`; upstream
  `isdecGCmodegen(g)` is true when `gckind == KGC_GEN || lastatomic != 0`.
- `keep_invariant()` and `is_sweep_phase()` are hardcoded `false`.
- `LuaStateGc::barrier`, `barrier_back`, `obj_barrier`, and
  `obj_barrier_back` are no-op shims, even though `lua-gc::Heap::barrier`
  has a lower-level forward-barrier primitive.
- `GcHeader` has `Color::{White, Gray, Black}` and a reserved `finalized` flag,
  but no dual-white bits and no generational age bits.
- `gengc.lua` is not a strong proof: `harness/impl/official/gengc.out`
  records `testC not active`, and the important `T.gcage`/`T.gccolor`
  assertions in `reference/lua-c/testes/gengc.lua` are skipped.

#104 is a prerequisite, but its status is nuanced:

- Some real heap accounting primitives already exist:
  `Gc::account_buffer`, `Heap::adjust_bytes`, `GcHeader.size: Cell<usize>`,
  a 1 MB threshold floor, and `lua-gc` unit tests for buffer refund/no-op on
  uncollected boxes.
- Some table buffer bytes are already charged through `LuaTable::buffer_bytes()`
  at construction.
- The Lua-visible API still has Phase-B simulations: `api.rs` refills
  `totalbytes` to a 32 KB baseline after collect and halves `totalbytes` after
  completed steps.
- `collectgarbage("count")` still combines `heap.bytes_used()` with the
  hand-maintained `gc_tracked_long_strings` tracker instead of one
  collector-owned allocation ledger.

So the correct #104 framing is not "no accounting exists"; it is "finish and
unify accounting, then delete the public observable simulations."

## Upstream Pieces to Port

The real 5.4 generational collector is not a small branch in `step`; it is a
set of coupled mechanisms in `lgc.c`, `lgc.h`, and `lstate.h`:

- State predicates: `keepinvariant(g)`, `issweepphase(g)`,
  `isdecGCmodegen(g)`.
- Two-white color bits plus black/gray state.
- Age bits: `G_NEW`, `G_SURVIVAL`, `G_OLD0`, `G_OLD1`, `G_OLD`,
  `G_TOUCHED1`, `G_TOUCHED2`.
- Cohort cursors on normal and finalizer lists:
  `survival`, `old1`, `reallyold`, `firstold1`, `finobjsur`,
  `finobjold1`, `finobjrold`.
- Barriers:
  `luaC_barrier_` marks white children and promotes to `G_OLD0` when needed;
  `luaC_barrierback_` links old touched parents into `grayagain` and advances
  touched ages.
- Mode transitions:
  `entergen`, `atomic2gen`, `enterinc`, `luaC_changemode`.
- Collection policy:
  `youngcollection`, `fullgen`, `stepgenfull`, `genstep`, `setminordebt`,
  `setpause`.
- Finalizer lists:
  `finobj`, `tobefnz`, and their generational cohort boundaries.
- Weak/ephemeron processing through atomic and gray-list correction.

## Implementation Sequence

### 1. Finish #104 Accounting

Goal: one real collector-owned byte model.

Deliverables:

- Delete the `api.rs` `totalbytes` refill and halve simulations.
- Make `collectgarbage("count")`, `gcinfo()`, `GCdebt`, `totalbytes`,
  `GCestimate`, and heap pacing agree on the same live-byte source.
- Finish payload accounting for tables, strings, userdata, closures/upvalues,
  and any other GC-owned buffers.
- Retire or subsume `gc_tracked_long_strings` into normal collector accounting.
- Preserve the existing uncollected-box guard so unswept boxes cannot create
  permanent byte drift.

Verification:

- `cargo test -p lua-gc`
- new VM/runtime tests proving bytes return to baseline after
  allocate/grow/drop/full-GC
- official `gc.lua` and `gengc.lua` without API-visible accounting shims
- GC canaries in both public modes
- repeated allocation stress showing plateau, not monotonic drift

### 2. Make Collector State Predicates Real

Goal: make the current collector phases observable to barrier logic.

Deliverables:

- Replace the coarse `gcstate` byte mirror with named states equivalent to
  `GCSpropagate`, `GCSenteratomic`, `GCSatomic`, `GCSswpallgc`,
  `GCSswpfinobj`, `GCSswptobefnz`, `GCSswpend`, `GCScallfin`, `GCSpause`.
- Implement `keep_invariant()` as `gcstate <= GCSatomic`.
- Implement `is_sweep_phase()` as `GCSswpallgc <= gcstate <= GCSswpend`.
- Implement declared generational mode as upstream does:
  `gckind == Generational || lastatomic != 0`.
- Add regression tests for these predicates across incremental step, sweep,
  pause, declared generational mode, and bad-major fallback.

### 3. Port Real Barriers Before Minor Collection

Goal: make old/black objects safe when the VM mutates them.

Deliverables:

- Replace no-op VM barrier shims with calls that inspect object color and age.
- Audit every existing call site against upstream `luaC_barrier*` usage:
  table writes, metatable writes, upvalue closure, userdata uservalues,
  proto/string/object installation, and API stores.
- Keep the lower-level heap barrier only as an implementation primitive; the VM
  needs Lua-object-aware barrier behavior, including age changes.
- Add grayagain/touched tracking that can support both incremental and
  generational correction.

Verification:

- GC canaries, especially table barrier and coroutine/upvalue cases
- new tests for black/old parent to white/young child references through table,
  upvalue, metatable, userdata, and coroutine paths
- official `gc.lua` and available trace/weak tests

### 4. Move Finalizers Into the Collector

Goal: stop treating finalization as an after-the-fact API drain.

Deliverables:

- Model `finobj` and `tobefnz` lists.
- Track finalizable cohorts: `finobjsur`, `finobjold1`, `finobjrold`.
- Use the reserved finalized state consistently.
- Separate, mark, run, and sweep finalizers from collector phases.
- Preserve per-version finalizer error behavior.

Verification:

- existing multiversion `__gc` tests
- new tests for order, resurrection, errors, and finalizer-list age movement
- official `gc.lua` finalizer sections

### 5. Add Ages, Dual Whites, and Cohorts

Goal: give objects enough metadata to make generational invariants real.

Deliverables:

- Extend `GcHeader` beyond `Color::{White, Gray, Black}` to represent dual
  whites, black/gray state, finalized, and age bits.
- Add normal-list cohort boundaries equivalent to `survival`, `old1`,
  `reallyold`, and `firstold1`.
- Make new allocations enter the correct age for the current collector state.
- Implement `sweep2old`, `sweepgen`, `correctgraylist(s)`, and `markold`.
- Ensure `enterinc` clears ages and cohort pointers safely.

Verification:

- internal age/color tests matching the first half of `gengc.lua`
- no regression in incremental mode
- tests for removing objects around `firstold1`/cohort boundaries

### 6. Implement Real Generational Steps

Goal: replace "incremental step plus weak prune" with minor/major policy.

Deliverables:

- `entergen` runs to an atomic boundary, sweeps survivors old, sets minor debt,
  and only then declares generational mode active.
- `youngcollection` marks OLD1 and touched objects, runs atomic processing,
  sweeps nursery/survival cohorts, updates finalizer cohorts, and finishes the
  generation cycle.
- `fullgen` and `stepgenfull` handle major and bad-major behavior.
- `genstep` chooses minor vs major using real bytes, `GCestimate`,
  `lastatomic`, `genminormul`, and `genmajormul`.
- `collectgarbage("step", 0)` matches reference behavior in declared
  generational mode.

Verification:

- deterministic minor/major scheduling tests
- `collectgarbage("param", ...)` behavior remains correct for 5.5
- official `gc.lua`, `gengc.lua`, canaries, and `lua-gc` tests

### 7. Finish Weak/Ephemeron Behavior Under Generations

Goal: weak tables and ephemerons must participate in minor and major cycles.

Deliverables:

- Weak values, weak keys, and ephemeron fixed-point processing run at the right
  atomic points.
- Old weak tables touched by young entries are revisited correctly.
- Weak string/key byte reclamation is collector-owned, not API cleanup.

Verification:

- official weak-table blocks in `gc.lua`
- generational weak-key/weak-value tests with old containers and young entries
- repeated minor collections do not leak young objects reachable only through
  weak paths

### 8. Build a `testC` Equivalent

Goal: stop letting `gengc.lua` pass by skipping its strongest assertions.

Deliverables:

- Add an internal-only harness/helper exposing safe equivalents of `T.gcage`
  and `T.gccolor`.
- Run the meaningful `gengc.lua` age/color object graphs against lua-rs, or
  port them into Rust tests.
- Cover table, metatable, userdata uservalue, upvalue, touched object,
  finalizer, weak-table, and mode-transition cases.
- Keep this helper out of normal public runtime surfaces.

Verification:

- the harness fails against the current scaffold
- it passes only after ages, barriers, cohorts, finalizers, and genstep are real

## Final Close Gate

#93 should close only when all of this is true:

- #109's startup default remains green for hosted 5.4/5.5.
- Incremental mode remains selectable and correct.
- Declared generational mode performs real minor/major collection.
- `isdecGCmodegen` semantics match upstream, including `lastatomic != 0`.
- #104 API-visible simulations are gone.
- Barriers, finalizers, weak tables, byte accounting, ages, and cohort
  transitions are covered by tests.
- `gengc.lua` meaningful age/color assertions are exercised, not skipped.

Final command set:

- `bash .claude/hooks/unsafe-budget.sh`
- `cargo test -p lua-gc`
- `cargo test -p lua-rs-runtime --test multiversion_oracle`
- `./harness/canaries/gc/run_canaries.sh`
- `TEST_TIMEOUT_S=60 ./harness/run_official_test.sh reference/lua-c/testes/gc.lua`
- `TEST_TIMEOUT_S=60 ./harness/run_official_test.sh reference/lua-c/testes/gengc.lua`
- full official-suite sweep for 5.4 and 5.5 after collector changes land

## PR/Issue Summary

#109 fixed only the default-mode startup bug in #93. The real generational
collector remains open. The critical path is #104 accounting first, then real
phase predicates and barriers, collector-owned finalizers, dual-white/age/cohort
metadata, minor/major generational policy, weak/ephemeron integration, and a
`testC`-equivalent harness that proves the meaningful `gengc.lua` assertions.
