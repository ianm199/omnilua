# Stuck: `reference/lua-c/testes/gc.lua`

**Status:** do not blindly agent-loop this. The GC has advanced past the doc's last snapshot, but `collectgarbage("step", n)` still has no real budget model.

## Current failure

Failure line depends on `_port` mode. Under our CLI:

```
testing incremental garbage collection
creating many objects
functions with errors
long strings
steps
steps (2)
lua: pcall_k failed: Runtime: reference/lua-c/testes/gc.lua:201: assertion failed!
```

Line 201 is `assert(dosteps(10) < dosteps(2))` — gated on `if not _port then`, so under the official harness with `_port=true` this path is skipped and the real failure is somewhere deeper. The doc's earlier "current failure at line 201" was true for the bare-CLI run but stale under the official harness.

## What's actually in tree (correcting the earlier draft)

The GC has had substantial work since the earlier doc was written:

- `crates/lua-gc/src/heap.rs:558` — `step_with_post_mark` exists, but it's threshold-triggered full collection, not true incremental work
- `crates/lua-vm/src/state.rs:2189` — `check_step` just calls `collect_via_heap(false)` (essentially a full collect)
- `crates/lua-vm/src/state.rs:2320` — `GcHandle::step()` is still a no-op
- `crates/lua-vm/src/api.rs:1960` — `GcArgs::Step { data }` parsed but `data` (the size parameter) is ignored

Plus the reachability/ephemeron/finalizer work that did land:

- Weak-table registry snapshots (D-2)
- Ephemeron convergence pass for `__mode='k'`
- `to_be_finalized` queue (O2-D1 finalizer-reachability fix)
- Post-mark hooks for both weak-table sweep and finalizer queueing

## Why not loop-grind this

Every prior round fixed a real bug — D-1e, D-1f, D-2, finalizer reachability, OP_SELF codegen, etc. The codebase is meaningfully better after eight gc.lua commits. But the test still fails because **it tests ~14 distinct GC behaviors and the step-budget one is the next blocker**, and that's an architectural gap, not a fixable-by-one-agent bug.

Concretely, real Lua's `lgc.c:luaC_step` runs `singlestep()` in a loop bounded by `g->GCstepmul * data` until either the cycle ends or the work budget runs out. We have nothing that scales work by `data`. Asking an agent to "fix gc.lua" reliably produces another real-but-tangential commit and no movement on the official harness.

## Three options, ranked

1. **Add gc.lua to `SKIP_TESTS` until Phase E.** Stop the bleed on opus spend. Document that whole-test "passing" is the wrong success metric for multi-layer GC tests. ★ recommended for the autonomous run.
2. **Carve `gc.lua` into harness-visible subtests.** Each `do ... end` block in gc.lua tests one feature. Run them as separate programs so the basic ones (weak tables, finalizers, simple collection) can register as wins and only the step-budget / generational ones stay stuck.
3. **Human-design an incremental budget model**, then let agents implement the mechanical pieces. Roughly:
   - `Heap::step(work_budget: usize) -> StepStatus` returning {complete, more}
   - Plumb `data` from `GcArgs::Step` through `collectgarbage("step", n)`
   - Add `gc_stepmul` multiplier
   1-2 days of human design + 2-3 agent runs to implement.

## What past agents have tried

| Commit | What landed |
|---|---|
| `9355448` (O2-D1, opus, this session) | Finalizer-reachability fix — replaced `strong_count()==1` gate with `to_be_finalized` queue |
| `66ccd3d` | 9-line cleanup in `state.rs` |
| `ff171af` | Phase D-2: reachability-based weak-table sweep |
| `5e34ef4` | Phase D-1f: five mark-sweep bugs |
| `80d5fc0` | Phase D-1e: `GcRef<T>` inner: `Rc<T>` → `Gc<T>` |
| `2809f30`, `1ca9c4c`, `64e74d7`, `6158afd` | earlier debug rounds |

All real wins, none of them moved gc.lua from fail to pass.

## Files most touched

`crates/lua-vm/src/api.rs`, `crates/lua-vm/src/state.rs`, `crates/lua-types/src/trace_impls.rs`, `crates/lua-gc/src/heap.rs`.
