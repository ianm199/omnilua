# Issue burndown spec — 2026-06-11

Owner: Fable (planning + verification). Execution: Opus subagents per packet.
Issues: #139 (5.1 order-TM gate), #134 (call-shaped perf residual), #113
(re-scoped to RSS). Baseline evidence: PGO matrix
`harness/bench/results/20260610T220702Z-c797118-compare.tsv` (commit c797118,
post-v0.0.33).

Status checklist (tick only with evidence paths):

- [x] T1: #139 fix landed — V51 mixed-type order comparisons raise without
      consulting `__lt`/`__le`; oracle parity on 5.1 AND 5.2–5.5 unchanged
      (commit 1ee624c: check.sh 57/54/23/7/10 pass across 5.1–5.5,
      multiversion_oracle 165 pass, run_official_all 44/44, 10-case
      byte-for-byte ref matrix vs /tmp/lua-refs/bin/lua5.1.5)
- [ ] T2-B: coroutine resume/yield allocator-traffic diet landed
      (`coroutine_pingpong` improves, canaries + official coroutine tests green)
- [ ] T2-A: pretailcall `clear_stack_range` verdict recorded (keep / remove /
      narrow) with rooting-safety evidence
- [ ] T2-C: frame re-entry / `prep_call_info` diet landed (`call_return_shapes`
      improves)
- [ ] T2-D: `finish_get` method-lookup diet landed (`method_calls` improves)
- [x] T3: #113 retitled to the RSS target with measured size table
      (issue #113 comment 2026-06-11, candidate ladder + done condition posted)

## Roles and routing

Per `../CLAUDE.md` model routing: Fable does planning, design sign-off, and
deep debugging only. Opus agents execute bounded packets. Every packet carries
its own gates; an agent reports evidence, it does not self-certify. Bench
numbers measured while another agent is compiling are provisional — final
numbers are re-measured on a quiet machine before any PR claims them.

The iteration ladder applies: rung 2 (`multiversion_oracle`) and rung 3
(`cargo test -p lua-vm`) are the inner loop; `run_canaries.sh` for anything
touching stack/GC; full PR gate (rung 6) once per branch, not per edit.

## Track 1 — #139: 5.1 mixed-type order comparisons (correctness)

### Reference semantics (verified against /tmp/lua-refs/bin/lua5.1.5)

5.1.5 `luaV_lessthan`/`lessequal` check `ttype(l) != ttype(r)` FIRST and raise
`luaG_ordererror` before any fast path or TM lookup:

| snippet (5.1) | reference output |
|---|---|
| `t<2`, t has `__lt` | `false  attempt to compare table with number` |
| `2<t`, t has `__lt` | `false  attempt to compare number with table` |
| `t<=2`, t has `__le` | `false  attempt to compare table with number` |
| `1<"hello"` | `false  attempt to compare number with string` |
| `t1<t2`, same mt `__lt` | `true  true` (same type: TM consulted) |
| `a<=b`, same mt only `__lt` | derived `not __lt(b,a)` (5.1 derivation kept) |
| `{}<{}` no mt | `false  attempt to compare two table values` |

5.2.4 succeeds (`true true`) on all the mixed-type rows: the gate is exactly
`version == V51`. lua-rs currently returns `true` on all four divergent rows
under `LUA_RS_VERSION=5.1`.

### Design

All five fallback sites funnel through one choke point,
`tagmethods.rs:738 call_order_tm` (sites: `less_than_others` vm.rs:1243,
`less_equal_others` vm.rs:1266, OP_LT vm.rs:2852, OP_LE vm.rs:2881,
`order_imm_slow` vm.rs:3711 → `call_orderi_tm` tagmethods.rs:804). Both-number
and both-string operands never reach it (handled by fast paths), so a V51 gate
at the top of `call_order_tm` reproduces C's check order exactly:

- if `lua_version == V51` and the two operands' Lua type tags differ
  (Int and Float are the SAME tag; use a Lua-type comparison, not the Rust
  enum discriminant), return `order_error(state, p1, p2)`
  (debug.rs:1526 — already produces the exact 5.1 wording) before any TM
  lookup, including the 5.1 `__le`-from-`__lt` derivation already gated at
  tagmethods.rs:767.
- `call_order_tm` is cold; a direct `state.global().lua_version` check is the
  idiom already used at tagmethods.rs:767-774. Do NOT add a per-opcode branch
  to the dispatch loop.

Trap to verify: the shared compiler emits OP_LtI/LeI/GtI/GeI for 5.1 chunks
(cg_emit_order, lib.rs:1339), so `t < 2` takes the immediate path. The
`call_orderi_tm` reconstruction must yield the same operand order and type
names as the reference for both normal and `inv` (GTI/GEI) forms — the test
matrix below covers constant and non-constant operands in both directions.

### Tests

1. `crates/lua-rs-runtime/tests/multiversion_oracle.rs` — new test
   `v51_mixed_type_order_errors_not_metamethod`, using `err_contains(V51, ...)`
   for: `t<2`, `2<t`, `t<=2`, `2<=t`, `t<"x"`, `"x"<t`, and the non-constant
   form `local n=2 ... t<n` (OP_LT path). Plus non-regression `eq` rows:
   same-type `t1<t2` with `__lt` still true on V51; mixed `t<2` still true on
   V52/V53/V54/V55.
2. `specs/oracle/check.sh` — add the two `run` rows in the 5.1 block
   (lines 163-239): `__lt` mixed-type and `__le` mixed-type pcall prints.
3. Spot-verify with `specs/oracle/diff_one.sh 5.1 '<snippet>'` (must flip
   DIFF → OK).

### Gates (in order)

```
cargo build -p lua-cli -q
cargo test -p lua-rs-runtime --test multiversion_oracle
specs/oracle/check.sh 5.1 && specs/oracle/check.sh 5.2 && specs/oracle/check.sh 5.3 \
  && specs/oracle/check.sh 5.4 && specs/oracle/check.sh 5.5
harness/run_official_test.sh reference/lua-c/testes/events.lua   (TM behavior, 5.4)
```
PR gate before push: `cargo test --workspace` + `harness/run_official_all.sh`.

Branch: `fix/139-v51-order-tm-gate`. Done: checklist T1 ticked with the five
check.sh outputs and the multiversion_oracle run recorded in the PR body.

## Track 2 — #134 residual: call-shaped wall-time diet

Baseline rows (PGO, c797118): `method_calls` 2.17x, `coroutine_pingpong`
2.00x, `call_return_shapes` 1.74x, `closure_ops` 1.80x. Profile evidence
(fresh, 2026-06-11, frame-pointer release build, /usr/bin/sample):

- `call_return_shapes`: ~52% of wall is frame machinery — OP_CALL 16.8%,
  FRAME_SETUP 15.6%, OP_TAILCALL 6.9%, OP_RETURN* ~10%, RETURN_REENTRY 3.7%.
- `method_calls`: `finish_get` 16.8% + `TableInner::get_str_value` 6.7%
  (every `obj:method()` misses on the instance and walks `__index`), OP_SELF
  11.3%, OP_CALL 9.1%, FRAME_SETUP 6.2%.
- `coroutine_pingpong`: ~17.8% in libsystem_malloc (`_nanov2_free` 8.1%,
  `nanov2_malloc_type` 3.8%, `_malloc_zone_malloc` 3.5%, `_free` 2.6%) +
  memset/memcmp; `push_parent_gc_snapshot` 5.0% and `RwLock::replace`
  (external_roots) 3.4% named directly.

Known structural diffs vs C (`ldo.c` luaD_precall/poscall): CallInfo 72B vs
64B; `prep_call_info` writes a 32-byte `CallInfoFrame` enum where C writes 4
scalars (do_.rs:685); frame re-entry re-matches the closure off the stack and
recomputes base every `'startfunc`/`'returning` entry (vm.rs:1891-1909) where C
keeps locals across the goto; per-tailcall `clear_stack_range` (do_.rs:796)
that C does not do; bounds-checked `get_at`/`set_at` vs raw StkId derefs.

Sub-packets, ordered by risk (lowest first). Each is one Opus agent, one
branch, sequential within a worktree.

### T2-B — coroutine resume/yield allocator diet (execute now)

Anchor: `crates/lua-vm` coroutine path — `push_parent_gc_snapshot` /
`pop_parent_gc_snapshot` (found via coro_lib / do_.rs resume path), the
`external_roots` RwLock replace, and `aux_resume`. PERFORMANCE_PRINCIPLES.md
("Audit port scaffolding", 2026-06-10) already blessed the direction: "the
GC-root copy was needed; the malloc/free pair was not."

Hypothesis: per-resume snapshot Vec alloc/free pairs and RwLock churn are the
~18% allocator pole. Fix shape: pool/reuse the snapshot buffers (clear+reuse a
collector- or state-owned buffer, same pattern as `marker_pool` in
heap.rs:1434); do not change WHAT is rooted, only where the buffer's storage
lives. Any change to rooting content itself is out of scope.

Gates: `cargo test -p lua-vm`, `cargo test -p lua-rs-runtime --test
multiversion_oracle`, `harness/canaries/gc/run_canaries.sh`,
`harness/run_official_test.sh reference/lua-c/testes/coroutine.lua`, then
`bash harness/bench/compare.sh --runs 5 --workloads coroutine_pingpong,closure_ops,fibonacci`
(fibonacci as a no-regression control). Target: coroutine_pingpong ≤ 1.7x
stock-build equivalent (provisional numbers acceptable from the agent; Fable
re-measures quiet before PR).

### T2-A — pretailcall `clear_stack_range` verdict (investigate, do NOT land without sign-off)

do_.rs:796 clears `[live_top, new_ci_top)` on every tailcall; C leaves the
reserved tail dirty. BUT: post-#140 exact rooting traces `[0..top)` and the
VM sets `top = ci_top` on several slow paths, so dirty slots in that range
could be traced — stale `GcRef`s there are exactly the UAF class #140 just
fixed. The clear may be load-bearing. Deliverable is a verdict memo, not a
diff: `git log -S clear_stack_range` provenance, whether traced range can see
the dirty slice (read the current trace bound logic), and if removal is safe,
what evidence proves it (GC canaries in quarantine mode + an ASAN battery run
of calls/coroutine official tests). Fable signs off before any removal lands.

### T2-C — frame re-entry + `prep_call_info` diet (after T2-B, design review required)

Anchor: vm.rs:1891-1909 (re-entry re-derivation), do_.rs:668-691
(`prep_call_info` 32B enum write), `Option<CallInfoIdx>` expects on pop
(vm.rs:3164, do_.rs:658). Direction: keep `code`/`base`/closure resolution in
locals across the dispatch loop the way C keeps them across `goto startfunc`,
and shrink the per-call CallInfoFrame write to the fields C writes. This is
deep VM surgery on the hottest loop: Opus drafts, Fable reviews the design
diff before it merges. Expected: the 15.6% FRAME_SETUP region compresses;
target call_return_shapes ≤ 1.55x.

### T2-D — `finish_get` method-lookup diet (after T2-C)

Anchor: vm.rs:1009-1056 (`finish_get` — MAX_TAG_LOOP frame, metatable borrow
traffic, `clone()`s to drop borrows), table.rs:1119 (`get_str_value`). C pays
the same algorithm via a tighter `luaV_finishget`. Direction: specialize the
one-level `__index`-is-a-table hop (the overwhelmingly common method-dispatch
shape) to a borrow-free fast path before entering the generic loop. No
semantic change: metamethod chain depth, error wording, and `__index`
function/table dispatch order must be byte-identical. Target: method_calls
≤ 1.85x.

## Track 3 — #113 re-scope to RSS (issue hygiene, no code)

The wall regression #113 was opened for is resolved (binarytrees 2.47x at
triage → 1.59x in the c797118 matrix). Surviving problem is RSS: closure_ops
4.19x, binarytrees 2.51x, string_format_mixed 2.12x, table_hash_pressure
2.09x, concat_chain 2.02x.

Measured representation sizes (value_layout, c797118) vs C 5.4.7:

| object | lua-rs | C | notes |
|---|---:|---:|---|
| GcHeader | 40 B | ~10 B CommonHeader | |
| GcBox<LuaTable> | 128 B | ~56 B | post-R2 diet (was 144) |
| TableNode | 40 B | 32 B | |
| GcBox<UpVal> | 104 B | ~40 B | dual Cell fast-path + RefCell<UpValState> mirror (upval.rs:22-39) |
| LClosure | 72 B box + separate upvals Vec | ~40 B + flex array | per-closure Vec header + extra alloc |
| CallInfo | 72 B | 64 B | wall concern, not RSS |

Plus: `allocation_tokens` side table ~50 B/object (PERFORMANCE_MODEL.md
candidate 10); `InternedStringMap` buckets never shrink (state.rs:130-207);
three mallocs per non-empty table (GC_ALLOC_PLAN.md cause 2).

Action: retitle #113 to the RSS target and post the table above with the
candidate ladder: (1) UpVal mirror removal — migrate remaining `slot()`
consumers to the Cell fields, the closure-RSS lever and likely a wall win too;
(2) candidate 9 table representation diet (Vec→Box<[T]>, PERF_PUSH_SPEC W2.3
follow-on); (3) candidate 10 allocation_tokens redesign; (4) intern-map shrink
policy. Done condition for the retitled issue: RSS ≤ 2.0x on closure_ops,
binarytrees, table_hash_pressure, concat_chain, string_format_mixed, best-of-5.

## Sequencing

1. T1 (#139) — main worktree, branch `fix/139-v51-order-tm-gate` (this spec
   rides on the same branch). PR when gates green.
2. T2-B + T2-A-investigation — isolated worktree, branch
   `perf/coroutine-snapshot-pool`, parallel with T1 (no file overlap:
   T1 touches tagmethods.rs/tests; T2-B touches coroutine/GC-snapshot path).
3. T3 — gh issue edit, immediate.
4. T2-C, then T2-D — sequential after T2-B merges (all touch vm.rs/do_.rs).

One branch per worktree, never two agents in one worktree, benches re-run
quiet before any number is quoted in a PR.
