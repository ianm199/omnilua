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
- [x] T2-B: coroutine resume/yield allocator-traffic diet landed — PR #147,
      honest result WALL-NEUTRAL (quiet interleaved A/B 3.40s vs 3.40s over
      20-run aggregates): the dominant snapshot-buffer pole was already fixed
      on main by 04cd144 (2026-06-09); the four remaining per-resume Vec
      buffers are now pooled (canaries 36/0, quarantine clean). Discovered
      follow-up: T2-B2 below.
- [ ] T2-B2: per-resume panic-hook machinery diet (design-gated; see below)
- [x] T2-A: pretailcall `clear_stack_range` verdict recorded: KEEP +
      document (see T2-A section; full analysis 2026-06-11)
- [x] T2-C: frame re-entry / `prep_call_info` diet RESOLVED-NEGATIVE with
      evidence (2026-06-11): A1 hoist/cold-split and A2 partial-update both
      REGRESS call_return_shapes 1.02-1.03x (revert-validated back to 0.999);
      A3 Option::expect already compiles to a shared cold symbol. The frame
      pole is structural in the CallInfoFrame discriminant branch → T2-C2.
      Surviving docs commit: PR #148.
- [x] T2-C2: CallInfoFrame flatten Option A landed (commit b93ec81): branch
      deleted at the source, CallInfo stays 72 B (compile-time assert), zero
      new unsafe, debug_assert tripwires on every accessor, trap →
      CIST_TRAP bit (frame-reuse reset validated byte-identical vs ref C).
      Evidence: targets call_return_shapes −8.6% / method_calls −4.4% wall
      reproducibly with EXACTLY FLAT Ir (cachegrind), but the call-free
      mandelbrot control also moved ~12% (Ir +0.55%) — the wall win is
      layout-entangled on this macOS/arm64 rig, not cleanly isolable to
      branch-removal. Gates: lua-vm 23/0, oracle green, canaries 36/0,
      official calls/events/coroutine/errors/db/locals 6/6, workspace 0 fail.
      VERDICT: Option B (union, 64 B, unsafe budget 0→≥6) NOT escalated —
      no control-isolated wall win, and the 8 B/frame RSS is not
      independently requested. Reopen via the #113 diet ladder or with a
      Linux perf-stat branch-miss measurement.
- [x] T2-D: `finish_get` method-lookup diet RESOLVED-NEGATIVE with Ir evidence
      (2026-06-11): the diet (hoist the loop-invariant `__index` interned-name
      read out of the per-hop loop; fold `table_metatable` + `fast_tm_table`
      into one `index_tm_with_name`) moves Ir the WRONG way on the primary
      target. Deterministic callgrind rs Ir/iter vs origin/main b23eec8:
      method_calls **+3.0 (+0.22%, regressed)**, metatable_index_chain
      −16.0 (−0.96%, improved), table_field_index / fibonacci flat (never
      enter `finish_get`). The hoist pays off only for multi-hop chains; for
      the 1-hop `Klass.__index = Klass` method-dispatch shape (the named
      target) the loop runs once, so hoisting saves nothing and adds cross-
      loop liveness for `index_name`. A no-hoist fold (name read lazily in
      the table branch) zeroes both deltas (+1.0 / +4.0 Ir/iter — noise),
      confirming the win was entirely the hoist and the cost entirely
      method_calls. Root cause is structural: `GcRef`/`LuaValue` are `Copy`,
      so the loop's `.clone()`s are bitwise copies, not refcount traffic, and
      the metatable read is a `Cell::get`; there is no "borrow gymnastics /
      clone overhead" to remove — the spec's premise (clone/borrow waste)
      does not hold for this `Copy` value representation. The genuine work
      (borrow metatable → hash-walk `__index` → borrow index table →
      hash-walk key) is what C pays too. Wall A/B (8-loop, 4-round,
      min-ratio) corroborated noise: method_calls 1.0083, metatable_chain
      1.0299, table_field 1.0030, all layout-entangled per the T2-C lesson.
      VERDICT: dropped, no code change; branch tip == origin/main.
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

### T2-B2 — per-resume panic-hook diet (next pingpong lever, design-gated)

Found during T2-B. `aux_resume` (coro_lib.rs ~235-255) wraps every resume in:
`take_hook()` → `Arc::new(Mutex::new(Some(prev)))` → `set_hook(Box::new(...))`
→ resume → `take_hook()` → restore `set_hook(prev)`. That is 3-4 heap
allocations plus four global hook-lock operations per resume, purely to
suppress the default panic printer for `LuaThreadClose` unwind payloads
(coroutine.close's internal unwind), chaining everything else to the previous
hook.

Design of record (Fable, 2026-06-11): install ONE process-global hook via
`OnceLock` at first resume, capturing the then-current hook and chaining to
it; gate the LuaThreadClose suppression on a `thread_local!` suppress
counter incremented/decremented around the `catch_unwind` (counter, not bool,
for nested resumes). Per-resume cost drops to two TLS counter writes; the
scoping semantics (suppression active only inside a resume window) are
preserved exactly. Known tradeoff to document: an embedder calling
`std::panic::set_hook` AFTER lua-rs's first resume displaces our chained hook
permanently (today per-resume reinstall wins each window); acceptable, note in
the doc-comment. Gates: T2-B gate set + a test that a panic raised inside a
resumed Rust hostfunc still reaches the default/previous hook (not swallowed),
plus quiet A/B on coroutine_pingpong.

### T2-A — pretailcall `clear_stack_range` verdict (investigate, do NOT land without sign-off)

do_.rs:796 clears `[live_top, new_ci_top)` on every tailcall; C leaves the
reserved tail dirty.

**VERDICT (2026-06-11): KEEP, and document.** Investigation findings:

- Provenance: added in `11dfb50` (2026-05-24, generic test-greening commit, no
  root-cause link); its sibling `prep_call_info` clear was removed the same
  day in `97b3c4c`. Predates the abe2b52 exact-rooting rewrite, whose design
  ("the return hot path clears nothing", EXACT_ROOTING_SPEC.md:263-273) makes
  it redundant *in the common case*.
- BUT a real residual window exists: the ci_top-raising slow paths
  (OP_LT/OP_LE → order metamethod, vm.rs:2851/2880 → frame pushed at ci_top →
  collect) run the per-collect dead-tail clear and the trace off the SAME
  raised top within one collect, so an uncleared `[live_top, new_ci_top)` gap
  WOULD be traced there — stale GcRefs in it are the #140 UAF class, and no
  standing assertion guards the window.
- Cost of the clear: `fsize - narg1` slots ≈ 16-32 B per tailcall via a scalar
  nil loop (state.rs:2631-2633) — single-digit percent of the 6.9% OP_TAILCALL
  region at most; the abe2b52-era A/B on call_return_shapes was inconclusive.
- Decision: expected win is within noise; proof-of-safety requires a new
  targeted canary (collect inside an order-TM called from a fresh tail-called
  frame with polluted reserved tail) + quarantine/ASAN battery. Cost/benefit
  says keep. Reopen only if a future profile shows the clear itself as a
  measured pole, and then only with that battery.
- Follow-up folded into T2-C (same files): fix the stale `clear_stack_range`
  docstring (state.rs:2620-2622 still describes the pre-abe2b52 conservative
  trace model) and add a doc-comment at the do_.rs:796 callsite recording this
  verdict so the divergence from C's luaD_pretailcall is no longer
  undocumented.

### T2-C — frame re-entry + `prep_call_info` diet (after T2-B, design review required)

Anchor: vm.rs:1891-1909 (re-entry re-derivation), do_.rs:668-691
(`prep_call_info` 32B enum write), `Option<CallInfoIdx>` expects on pop
(vm.rs:3164, do_.rs:658). Direction: keep `code`/`base`/closure resolution in
locals across the dispatch loop the way C keeps them across `goto startfunc`,
and shrink the per-call CallInfoFrame write to the fields C writes. This is
deep VM surgery on the hottest loop: Opus drafts, Fable reviews the design
diff before it merges. Expected: the 15.6% FRAME_SETUP region compresses;
target call_return_shapes ≤ 1.55x.

### T2-C2 — CallInfoFrame flatten, Option A (approved)

T2-C's A/B data proved the FRAME_SETUP pole is the `CallInfoFrame` enum's
discriminant branch (state.rs ~565-582): every hot accessor (`saved_pc`,
`set_saved_pc`, `nextra_args`, `ci_trap`, `set_trap`) matches the tag before
touching a field, and source-level tricks around it regress. The fix is to
delete the branch: replace the 2-variant enum with a plain struct whose
fields are always present (`savedpc`, `nextraargs`, `k`, `old_errfunc`,
`ctx` — 32 B payload, CallInfo stays 72 B), and fold `trap` into a new
`CIST_TRAP` callstatus bit (bits 14/15 free; C stores trap in callstatus
too). All accessors become plain field reads / one-mask bit ops. Zero new
unsafe.

Sign-off conditions (Fable, 2026-06-11):
- Every accessor that was variant-guarded carries a `debug_assert!` on the
  frame kind (`CIST_C` bit), replacing the enum's wrong-variant panic
  tripwire at zero release cost.
- ~22 `CallInfoFrame::` sites change: state.rs accessors/constructors
  (:599-:743, :2913, :5297, :5470), do_.rs (:685-688, :1100-1101, :1399),
  tagmethods.rs (:857, :957), api.rs (:1819, :1890, :1898).
- savedpc write-before-reentry ordering is byte-identical; db.lua +
  errors.lua official tests are the line-attribution canaries.
- Arbiter for sub-1% wall deltas: `harness/bench/instr-count.sh` (Ir is
  load-immune); judge wall on interleaved A/B min-ratio past the ±1% floor.
- Option B (repr(C) union overlay, CallInfo 64 B, lua-vm unsafe budget
  0 → ≥6) is NOT approved yet: escalate only if Option A's wall win lands
  and the 8 B/frame RSS is independently wanted (then it joins the #113
  diet ladder).

### T2-D — `finish_get` method-lookup diet (after T2-C) — RESOLVED-NEGATIVE

Anchor: vm.rs:1009-1056 (`finish_get` — MAX_TAG_LOOP frame, metatable borrow
traffic, `clone()`s to drop borrows), table.rs:1119 (`get_str_value`). C pays
the same algorithm via a tighter `luaV_finishget`. Direction tried: hoist the
loop-invariant `__index` interned-name read out of the per-hop loop and fold
the per-hop `table_metatable` + `fast_tm_table` pair into one
`index_tm_with_name`, preserving the loop-iteration-per-hop structure so chain
depth, error wording, and function/table dispatch order stay byte-identical.

Outcome (2026-06-11, see checklist entry for the full Ir table): the diet
moves the named target the WRONG way — method_calls **+0.22% Ir**
(deterministic callgrind), while only the multi-hop metatable_index_chain
improves (−0.96%). The premise in the original direction — that the loop
wastes work on borrows and `clone()`s — does not hold: `GcRef`/`LuaValue` are
`Copy` (lua-types/src/gc.rs:163, value.rs:12), so the loop's clones are
bitwise copies and the metatable read is a `Cell::get`; the irreducible cost
is the two hash-walks (metatable `__index`, then the index table's key) that C
pays identically. No borrow-free fast path can remove the borrows of the two
tables it must actually read. Per the spec's own arbiter rule (Ir must go DOWN
on method_calls or the change is dropped), the change is **dropped** — branch
`perf/finish-get-diet` carries only this docs note; tip == origin/main.

Harness lesson: when a perf direction is premised on "remove clone/borrow
overhead," first confirm the value representation is NOT `Copy`. For lua-rs's
`Copy` value enum, `.clone()` is free and the optimizer already elides the
redundant copies — the diet has nothing to remove. The Ir arbiter is what
made this droppable cleanly where wall (all four workloads +0.3–3.0%,
layout-entangled per T2-C) could not have settled it.

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
