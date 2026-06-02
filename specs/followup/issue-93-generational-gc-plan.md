# Issue #93: Generational GC Plan

Status audited on `issue-93-gc-current` after the minor-traversal and
normal-list cohort-sweep checkpoints.

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

## Current Progress

The current tree is no longer only startup/default scaffolding:

- `collectgarbage("step", ...)` branches into `generational_step()` when the
  declared mode is generational or `lastatomic != 0`.
- `GlobalState::is_gen_mode()` now matches the upstream declared-mode rule:
  `gckind == Generational || lastatomic != 0`.
- `keep_invariant()` and `is_sweep_phase()` read real heap state.
- VM stores route through active forward/backward barriers, including table
  writes, userdata uservalues, closure upvalues, and cross-thread upvalues.
- `lua-gc` has dual-white colors and generational age metadata; the canary and
  testC paths exercise age/color transitions instead of only relying on the
  official skipped path.
- Lua-visible byte APIs now read collector-owned heap bytes. The old
  `api.rs` `totalbytes` refill/halve shims and the long-string side tracker are
  gone; `collectgarbage("count")`, `gcinfo()`, debt, estimate, and pacing all
  use heap byte accounting.
- Payload bytes are charged/refunded for tables, strings, userdata payloads and
  uservalues, C-closure upvalue vectors, Lua-closure upvalue slots, and compiled
  proto vectors.
- `lua-gc::MarkerStats` and `SweepStats` record mark/drain work and sweep work
  split by young vs old age. `T.gcstats()` exposes those counters, and GC
  canary pass rows can now carry `METRIC ...` telemetry instead of only
  PASS/FAIL.
- Minor collection now runs the marker in a young-generation mode: plain
  `G_OLD` objects are counted as live but are not drained, while `G_OLD0`,
  `G_OLD1`, `G_TOUCHED1`, and `G_TOUCHED2` objects are explicitly revisited.
  The first telemetry canary dropped from `tracedold=246` to `tracedold=1`.
- The revisit set is now collector-owned. Forward/backward barriers and minor
  sweep record objects that must be revisited by the next young collection, so
  minor marking no longer scans `allgc` just to discover `OLD0`/`OLD1`/touched
  objects.
- The normal `allgc` list now has collector cursors for `survival`, `old1`,
  `reallyold`, and `firstold1`. Young sweep walks the nursery and survival
  ranges instead of the full old tail, and the telemetry canary now asserts
  `sweepvisitedold=0` while still recording touched-object revisit work.
- Full/incremental major sweep now corrects generation cursors when it frees a
  cursor object, and a new regression covers the generational black-to-major
  white reset so old objects can die during a major collection.
- Minor weak/ephemeron/finalizer cleanup uses age-aware liveness, so objects
  deliberately skipped because they are old are not misclassified as dead.
- Finalizer registry telemetry now splits pending and to-be-finalized objects
  by young vs old age (`pendingfinyoung`, `pendingfinold`, `tobefinyoung`,
  `tobefinold`) and exposes registry cohort counts (`finobjnew`,
  `finobjsur`, `finobjold1`, `finobjrold`, `finobjscan`). Minor finalizer
  marking now snapshots only the registry's new/survival suffix, matching
  C-Lua's "scan until `finobjold1`" shape for the current overlay model.
  `canary_m_testc_finalizer_cohorts.lua` pins that rooted old finalizers stay
  outside the minor scan while a young unreachable finalizer moves to
  to-be-finalized-young.
- Weak table registry selection is now collector-crate owned through
  `lua-gc::WeakRegistry<T>`. The VM still provides the table-specific
  ephemeron/prune hooks, but dedupe, dead weak-handle dropping, live snapshots,
  retain-by-live-identity, and `T.gcstats()` telemetry (`weaklive`, `weakdead`,
  `weakretained`) are centralized. `canary_n_testc_weak_registry.lua` pins that
  rooted weak tables are snapshotted/retained while weak-only entries clear.
- Internal testC telemetry exists for GC state, age/color, type counts, warning
  capture, and memory accounting. Both normal and `LUA_RS_TESTC=1` official
  `gc.lua`/`gengc.lua` currently pass.

The real generational collector is still not complete:

- Normal-list minor marking and sweeping are now cursor-bounded for the current
  allgc architecture, but this is still not exact C-Lua parity: touched objects
  are held in a collector-owned revisit vector instead of an intrusive
  `grayagain` list.
- Finalizers now live behind a generic `lua-gc::FinalizerRegistry<T>` with
  pending/to-be-finalized list mechanics, `finobjsur`/`finobjold1`/`finobjrold`
  cohort boundaries, and minor-scan selection owned by the collector crate.
  `lua-vm::FinalizerObject` implements the small `FinalizerEntry` trait, and
  the heap header's finalized bit now mirrors C-Lua's `FINALIZEDBIT` while an
  object is registered in pending/to-be-finalized lists. Remaining parity gap:
  finalizable objects are still an overlay on the heap's `allgc` chain, not a
  true intrusive `finobj`/`tobefnz` ownership split.
- Weak/ephemeron handling is correct enough for the current gates. Weak-table
  registry mechanics are now collector-owned, but weak/ephemeron table
  classification and mark/prune processing still run through VM post-mark hooks
  instead of intrusive `weak` / `ephemeron` / `allweak` lists.
- `GlobalState.totalbytes` has been removed; `gettotalbytes` maps to
  collector-owned heap bytes through `GlobalState::total_bytes()`.

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

- Done: delete the `api.rs` `totalbytes` refill and halve simulations.
- Done: make `collectgarbage("count")`, `gcinfo()`, `GCdebt`, `GCestimate`,
  and heap pacing agree on the same live-byte source.
- Done: retire the long-string side tracker into normal collector accounting.
- Done: preserve the uncollected-box guard so unswept boxes cannot create
  permanent byte drift.
- Done: charge/refund payload accounting for userdata, closures/upvalues,
  protos, tables, strings, and the currently identified GC-owned backing
  buffers.
- Done: remove the unused `GlobalState.totalbytes` compatibility shadow.

Verification:

- `cargo test -p lua-gc`
- new VM/runtime tests proving bytes return to baseline after
  allocate/grow/drop/full-GC
- official `gc.lua` and `gengc.lua` without API-visible accounting shims
- GC canaries in both public modes
- `canary_k_testc_accounting.lua`, which compares `collectgarbage("count")`
  against `T.totalmem()` through long-string charge, table-buffer charge,
  userdata charge, and post-sweep refund
- repeated allocation stress showing plateau, not monotonic drift

## Iteration Discipline

Each remaining collector slice should keep the loop short:

1. Add or tighten a canary, unit test, or testC telemetry point that fails for
   the missing behavior.
2. Make the narrow collector/runtime change needed for that signal.
3. Run the single targeted canary or test first.
4. Run the focused package gate touched by the change.
5. Run the full GC gate set only at milestone boundaries.

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
- Done as structural slices: replace scattered raw VM vectors with a single
  `lua-gc::FinalizerRegistry<T>` abstraction, and move pending-to-finalized
  promotion mechanics into that registry.
- Done for the registry overlay: track finalizable cohorts equivalent to
  `finobjsur`, `finobjold1`, and `finobjrold`, and use them to bound minor
  pending-finalizer scans.
- Done for the registry overlay: use the heap header finalized bit as
  C-Lua's `FINALIZEDBIT`, setting it on pending/to-be-finalized registration
  and clearing it when an object is popped for its `__gc` call.
- Separate, mark, run, and sweep finalizers from collector phases.
- Preserve per-version finalizer error behavior.

Verification:

- existing multiversion `__gc` tests
- `canary_m_testc_finalizer_cohorts.lua` for pending/to-be-finalized age splits
  and registry cohort/minor-scan boundaries
- new tests for order, resurrection, errors, and finalizer-list age movement
- official `gc.lua` finalizer sections

### 5. Add Ages, Dual Whites, and Cohorts

Goal: give objects enough metadata to make generational invariants real.

Deliverables:

- Extend `GcHeader` beyond `Color::{White, Gray, Black}` to represent dual
  whites, black/gray state, finalized, and age bits.
- Done: add normal-list cohort boundaries equivalent to `survival`, `old1`,
  `reallyold`, and `firstold1`.
- Make new allocations enter the correct age for the current collector state.
- Done for the normal allgc list: `sweep2old`-equivalent promotion and
  cursor-bounded young `sweepgen`.
- Remaining: exact `correctgraylist(s)`/`markold` parity for intrusive
  `grayagain` and weak lists, plus a true `finobj`/`tobefnz` split instead of
  the current finalizer registry overlay on `allgc`.
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
  generation cycle. Done for normal-list sweep and finalizer registry cohorts;
  still missing intrusive weak/gray/finalizer list parity.
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
- Done for registry mechanics: collector-owned weak snapshots dedupe entries,
  drop stale weak handles, and retain only tables that stayed live through the
  mark/prune pass.
- Weak string/key byte reclamation is collector-owned, not API cleanup.

Verification:

- official weak-table blocks in `gc.lua`
- generational weak-key/weak-value tests with old containers and young entries
- repeated minor collections do not leak young objects reachable only through
  weak paths
- `canary_n_testc_weak_registry.lua` for registry live/dead/retained telemetry

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
collector remains open. Accounting, barriers, age/cohort metadata, minor/major
policy, and collector-owned finalizer registry mechanics are now active in the
branch. The remaining critical path is exact grayagain/weak/ephemeron ownership,
true intrusive `finobj`/`tobefnz` separation, and a final full-suite sweep that
proves the meaningful `gengc.lua` assertions stay exercised.
