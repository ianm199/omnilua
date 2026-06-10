# GC root inventory — issue #140 exact-rooting audit (P1)

Status: 2026-06-10, branch `gc/exact-rooting-p0`. Companion spec:
`docs/EXACT_ROOTING_SPEC.md`. Instruments: `LUA_RS_GC_QUARANTINE=1`,
`LUA_RS_GC_STRESS=1`, the root-loss assert in `trace_reachable_threads`,
and `harness/asan-stress.sh` (ASAN). Line numbers are as of commit
`9807995` — re-grep before trusting them across refactors.

**The exactness statement under audit:** every object reachable by future
VM execution is reachable from the root trace at every `would_collect`
checkpoint. It has two sides: (a) every STORAGE location holding a
`GcRef` is traced, and (b) every READER's read-set is inside the
trace-set. Bug A was a (a)-failure (borrowed thread untraced); bug B is
an (a)/(b) mismatch on the stack (tracer stops at `top`/frame ranges,
readers and later cycles see stale slots).

## 1. Collection checkpoints (where a collect can fire)

Collection happens ONLY at explicit `gc_check_step`/`gc_cond_step`/
`check_step` call sites — allocation entry points (`intern_str`,
`new_table*`, `push_closure`) do NOT collect inline. The Rust-temporary
contract is therefore: **no checkpoint between allocation and rooting**,
audited per checkpoint, not per allocation.

| Crate / file | Sites | Context |
|---|---|---|
| lua-vm/vm.rs | ~2357, ~2791, ~3418, ~3532 | opcode arms: NEWTABLE, concat, CLOSURE, vararg PACK |
| lua-vm/do_.rs | 514, 708, 764, 833, 883, 912 | checkstackGCp, precall, do_call, returncall, stack realloc, frame setup |
| lua-vm/state.rs | 3959, 3984, 5546 | gc_check_step / gc_cond_step wrappers, new_thread |
| lua-vm/api.rs | 858, 988, 1091, 1102, 1108, 1120, 1193, 1331, 2277, 2309 | table ops, string building (`push_vfstring` 1120 = bug A's trigger), API boundaries |
| lua-vm/tagmethods.rs | 938, 969 | metamethod dispatch, get_varargs |
| lua-rs-runtime/lib.rs | 935, 952, 1039 | embedding entry points |

C's discipline at the equivalent sites is `checkGC(L, c)` (`lvm.c:1131`),
which SETS `L->top` to the live mark before the collect. Our checkpoints
do not currently fix up `top` — that is P2 option (d).

## 2. Storage inventory

Verdicts: OK = traced at every checkpoint, instrument-verified.
GAP = known uncovered path. DIVERGENT = safe but not C-faithful.

| # | Location | Traced by | Verdict | Evidence / canary |
|---|---|---|---|---|
| 1 | Stack slots `[0..top)` | `LuaState::trace` via `gc_trace_bound` (C `traversethread` parity) | OK (P2 option (d), 2026-06-10): tight bound + `clear_dead_stack_tail` before every collect + site-local savestate fixups (`get_varargs`, VarArgPack). Was bug B: the old frame-range walk traced stale slots because C's atomic dead-slice clear was never ported | battery config 2 — deterministic panic at `9807995`, clean after; ASAN UAF on db.wrap stress=1 gone |
| 2 | Debug-local heuristic | DELETED (P2 option (d)) | OK: the tight-bound + Protect-fixup pair makes `[0..top)` cover live locals at every collect point; the saved_pc heuristic is gone | same as row 1 |
| 3 | ci chain func slots | stack walk (each range starts at `ci.func`) | OK while thread traced; was the victim read of bug A when whole thread went untraced | canary_q |
| 4 | Open upvalues | `self.openupval` loop (trace_impls.rs:109) + `twups` + `cross_thread_upvals` (GlobalState) | OK | canary_b |
| 5 | Registry (`l_registry`), globals, loaded | GlobalState::trace :136,157-158 | OK | suite-wide |
| 6 | Hook closure | dual-rooted: `registry[HOOKKEY][thread]` (via l_registry) + Rust `Box` in `LuaState.hook` (debug.rs:446) | OK; residual: if the Box captures GcRefs and the HOOKKEY entry is removed first, the Box side is invisible to the GC — keep the registry entry the lifetime master | db.lua hook sections under battery |
| 7 | Coroutine LuaStates (registered threads) | `trace_reachable_threads` fixed-point (state.rs:4239) — only if thread VALUE marked AND `try_borrow` succeeds | **OK with assert**: borrow-failures must be ≤ parent snapshots (debug assert, state.rs:4271). Was bug A: debug-API borrows held across allocations un-rooted the thread silently. Fixed by `RootedThreadBorrow` (coro_lib.rs) | canary_q fails on parent commit; coroutine.lua battery-green |
| 8 | Resume-chain parent stacks | snapshot push/pop in `aux_resume` (coro_lib.rs:202+), traced at GlobalState::trace :223-232; pools always empty | OK; LIFO discipline documented on the guard | coroutine.lua, canary_b |
| 9 | Debug-API borrowed threads | `RootedThreadBorrow` snapshot (stack `[..top]` + open upvals; `resnapshot()` after lua_getinfo 'L'/'f' pushes) | OK as of `0677646`; snapshot is `[..top]` — beyond-top reads rely on row 1's eventual fix | canary_q |
| 10 | To-be-closed list | `tbclist` holds StackIdx only; values live on stack | OK | close canaries in suite |
| 11 | Metatables (per-type `mt[]` + table/userdata metatables) | GlobalState::trace :177-181; per-object via Trace impls | OK incl. self-cycles (marker visited-set dedup) | canary_e |
| 12 | Short-string intern (`interned_lt`) | NOT traced (weak by design); pruned post-mark by visited set BEFORE sweep, so `find()` never returns a doomed string | OK | canary_p (dead-key family) |
| 13 | `strcache` | traced as STRONG root (trace_impls.rs:196-200) | **DIVERGENT**: C clears stale cache entries in atomic instead; ours over-retains cached strings a cycle. Safe; revisit in P2 cleanup | gc.lua count assertions stay green |
| 14 | Weak tables (entries) | weak-mode trace skips weak edges; post-mark `prune_weak_dead_with_value` clears dead entries, TOMBSTONES keys of empty entries (clearkey parity), string-preserves Str keys/values | OK as of `1a04425`. Was bug 4: value-nil nodes skipped → dead key never tombstoned → `equal_key` deref of swept long string | canary_r fails on parent commit; gc.lua quarantine-green |
| 15 | Strong-table erased entries | `trace_entries_with_clearkey` tombstones nil-valued entries' keys during traversal | OK (`9c5125c`) | canary_p |
| 16 | Pending finalizers / to_be_finalized | pending NOT traced (by design — distinguishes reachable from finalizer-kept); `to_be_finalized` traced :213-215; during `__gc` call object+closure are stack-resident | OK | canary_c, canary_m, canary_o |
| 17 | Embedding handles (lua-rs-runtime) | `external_roots` traced :141-143; ScopedRef/Mut are transient borrows, nulled on scope drop | OK | runtime scope tests (Miri-covered) |
| 18 | Parser/lexer temporaries | `long_str_anchor` (lua-lex) anchors interned long strings for the parse session; protos anchor their constants; resulting closure pushed on stack | OK | suite compile paths under quarantine |
| 19 | Rust-frame temporaries in opcode arms / stdlib | nothing traces them; SAFE only because no checkpoint sits between alloc and rooting. ~30 hold-across-alloc sites, ~5 near checkpoints; worst: VarArgPack vm.rs:3518-3530 (`t`, `n_key` held across 3 allocs, checkpoint at :3532 AFTER rooting — currently correct but one re-order away from a bug) | OK-FRAGILE: contract enforced dynamically (stress+quarantine battery), not statically | battery config 2; P2(c) verdict: fix in place if any site regresses, no anchor API |
| 20 | Coroutine completion state | `reset_thread` (state.rs:5682): ci → base frame, stack cleared, top=1 — matches C's dead-coroutine shape | OK | coroutine.lua |
| 21 | Stack shrink sites | shrink only at close/reset paths (state.rs:5433, ci truncate :5274/:5321); C shrinks in atomic | OK | — |

## 3. Reader inventory (read-set vs trace-set)

| Reader | Reads | Inside trace-set? |
|---|---|---|
| VM opcode arms | registers `[base..ci.top)` via `get_at`/`set_at` | YES below `top` at checkpoints; the between-checkpoint window is safe by the checkpoint contract |
| `debug.rs` traceback/getinfo (`ci_lua_proto`, `funcname_from_call`) | `get_at(ci.func)` on ANY walkable ci of ANY thread | only if that thread is traced — was bug A. NOTE: `ci_lua_proto` panics on a non-closure slot; under P2(d)'s atomic clear it must tolerate nil |
| `debug.getlocal`/`setlocal` | slots up to `ci.top` (above top for active locals) | NO for above-top slots — same disease as row 2; P2(d) widens trace to cover |
| Table probe chains (`equal_key`, `key_is_short_str`, `main_position_from_node`) | node keys incl. dead ones | dead keys must be tombstoned (rows 14/15) and are then compared by pointer bits only; insert-relocation hashes only live (non-empty-valued) keys |
| `next`/iteration (`get_generic_slot_deadok`) | dead-key tombstones by pointer bits | YES (no deref) |
| Weak registry / finalizer post-mark hooks | identities + marks, not contents | YES (identity-only) |

## 4. Open items

1. **Bug B (row 1/2)** — the only open GAP. Fix = P2 option (d):
   checkpoint top-fixup (`checkGC(L,c)` parity) + clear-dead-tail in the
   atomic pass + collapse trace to `[0..top)` + delete the debug-local
   heuristic. Battery config 2 is the regression oracle.
2. **strcache over-retention (row 13)** — divergence, not a safety bug.
3. **Row 6 residual** — make the registry HOOKKEY entry the documented
   lifetime master for hook closures.
4. **Row 19 fragility** — re-audit whenever a checkpoint is added or
   moved; the battery is the tripwire.

## 5. Instrument map (how each row is checked)

- Quarantine (`LUA_RS_GC_QUARANTINE=1`): any deref of a swept box panics
  — catches every content-deref gap (rows 1, 7, 14 found/verified).
- Stress (`LUA_RS_GC_STRESS=1`): collect at every checkpoint — makes
  cadence-window bugs deterministic (row 1).
- Root-loss assert: borrow-failures ≤ snapshots at every collect (row 7).
- ASAN (`harness/asan-stress.sh --asan`): truth-teller for reads that
  bypass headers; confirmed row 1 (db.wrap stress=1) and the row 7/14
  fixes (all no-stress runs clean, 2026-06-10).
- Canaries: `canary_p` (row 15), `canary_q` (rows 7/9), `canary_r`
  (row 14), plus the long-standing a–o set; runner honors `LUA_RS_BIN`.
