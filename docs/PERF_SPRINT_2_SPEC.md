# Perf sprint 2 spec — started 2026-06-11

Owner: Fable (supervisor: design sign-off + verification). Execution: Opus
subagents per packet. Goal of record: `docs/PERF_SPRINT_2_GOAL.md` — read it
first; this file is the live checklist and verdict ledger, in the style of
`docs/ISSUE_BURNDOWN_SPEC.md`. Baseline evidence: stock matrix
`harness/bench/results/20260611T164856Z-b0e68f8-compare.tsv` (overall 1.47).
Board claim: `../AGENT_COORDINATION_BOARD.md` Active Work row (Fable,
2026-06-11).

Status checklist (tick only with evidence paths):

- [x] T0.1 `instr-count.sh --branch-sim` (Bc/Bcm/Bi/Bim; tool is cachegrind,
      header corrected) — PR #158; first run surfaced fibonacci Bim 3.3x vs C
      (bytecode dispatch), the CPI gap now measurable
- [x] T0.2 bash-3.2 `set -u` EXTRA_MOUNT fix; audit found no other
      empty-array bugs in harness/bench — PR #158
- [x] T0.3 `profile-hotspots.sh` agent-stall FIXED (detached watchdog
      inherited stdout/stderr and held the pipe open; fds detached, watchdog
      reaped via pkill -P) — validated under the agent harness, PR #158
- [x] T0.4 `heap-diff.sh` landed with exact-zero null test — PR #158; first
      real use produced T1's causal evidence below
- [x] T0.5 `docs/MEASUREMENT_PROTOCOL.md` written (supervisor-authored,
      2026-06-11), linked from `CLAUDE.md` §Benchmarks
- [x] T0.6 `port-harness/templates/c-to-rust/perf-packet.md` extracted
      (port-harness commit 90239a5, green proof = ISSUE_BURNDOWN_SPEC.md)
- [x] T1 UpVal mirror removal landed (commit 03c4468): UpVal 64→32 B,
      GcBox<UpVal> 104→72 B (value_layout; the goal doc's "≤64 B" bar was a
      supervisor arithmetic error — 72 B is the floor with the 40 B GcHeader,
      whose diet belongs to T3/#113). closure_ops causal chain: heap-diff
      total bytes −7.89% / peak live −10.86% with alloc count unchanged
      (100k upvals × 32 B = the measured −3,199,936 B), process max-RSS
      40.7→37.3 MB (−8.3%, interleaved ×3). Gates: oracle 165/0, canaries
      36/0, quarantine clean on coroutine/locals/closure, workspace 0 fail,
      wasm check green. UpValState deleted from the public API (0.0.x).
      #113 progress comment: pending below.
- [x] T2 setter family — RESOLVED-NEGATIVE (no in-scope Ir win exists;
      verdict-ledger entry below). FRESH branch-sim profile on 5727ee4
      established the gap is structural safety/representation tax, not
      removable instruction-removal under the packet's no-unsafe / no-layout
      constraints. Two candidate changes built, gated green, measured, and
      reverted per drop-if-neutral. Branch `perf/setter-family-diet`.
- [x] T3a GC/alloc design memo written: `docs/GC_ALLOC_DESIGN_MEMO.md`
      (supervisor-authored, dhat absolutes @ 5727ee4: concat_chain 13.9M
      blocks/38 B avg is the standout anomaly; binarytrees peak 36 MB live).
      Ranking: R1 lazy weak-token registration APPROVED as T3b; R2 concat
      string-churn packet filed as discovered follow-up; R3 size-class
      pooling deferred (needs human sign-off, quarantine interplay); R4
      candidate-9 table parts stays on the #113 ladder behind T2; R5
      GcHeader sub-40 diet REJECTED (hot-field packing already measured
      +4% Ir; remaining 32 B is two fat pointers); R6 pacer tuning deferred.
- [x] T3b lazy weak-token registration KEPT (PR #162, commit 13d9e52): Ir
      −2.59/−3.69/−2.94/−2.63% on gc_pressure/concat_chain/binarytrees/
      table_hash_pressure with fibonacci control at −0.000%; heap-diff peak
      live −12.4%/−10.0%; all weak-table canaries + quarantine green; new
      address-reuse token test. Memo R1 sizing confirmed.
- [x] T4 safety-tax ablation MEASURED on branch `ablation/unchecked-stack`
      (pushed unmerged for reproducibility; never merges). Two-axis
      attribution + branch-sim cross-check + quiet wall A/B written into
      `docs/PERFORMANCE_MODEL.md` §"Safety-tax ablation": safety checks =
      5–15.5% of Ir, ~0% of reliable wall (Bcm≈0 — perfectly predicted;
      ablated builds were wall-NEUTRAL-to-SLOWER), and Ir ratios remain
      ≥1.9x C after FULL ablation → the residual gap is representation/
      idiom, not safety. Evidence: results/20260611T*-t4-*.tsv (6 runs).
- [x] CLOSE: CHANGELOG Unreleased entries added; closing matrix
      20260611T200946Z-2a10e04 (overall 1.47; binarytrees 1.74→1.58,
      gc_pressure 1.98→1.81, concat_chain 2.02→1.82, closure_ops RSS
      4.18→2.98, table_hash_pressure RSS 2.14→1.86) committed to the bench
      ledger; board row moved to Recently Completed 2026-06-11.

## Protocol

The measurement protocol, gates, and stop conditions are normative in
`docs/PERF_SPRINT_2_GOAL.md` §"Non-negotiable measurement protocol" — they are
not restated here. Two local notes:

- Frozen baselines this sprint: name them `/tmp/lua-rs-s2-<packet>-base` and
  record the build sha next to each tick.
- Bench-host rule: one measurement process at a time across all repos;
  implementation agents may report provisional numbers under load, but every
  number that appears in a PR body or this checklist is re-measured quiet by
  the supervisor.

## Verdict ledger

(append per-packet outcomes here as they land — kept verdicts AND honest
negatives, with evidence paths)

### T2 setter family — RESOLVED-NEGATIVE (2026-06-11, branch `perf/setter-family-diet`)

**Verdict: no in-scope instruction-removal win exists on the four target rows.**
Two candidate changes were implemented, gated green (oracle 165/165), measured
with the deterministic Ir + branch-sim arbiters, and reverted per drop-if-neutral.
The setter-family gap is structural safety/representation tax that the packet's
constraints (zero new `unsafe`, no `LuaValue`/table layout change — the latter is
an explicit STOP condition) put out of reach. The right owners are T4 (safety-tax
ablation, never-merges) and T3 (representation).

Baseline (frozen `/tmp/lua-rs-s2-t2-base`, sha 5727ee4, same-source rebuild
`e4797e3`, protocol-valid). Per-iteration budgets, `--branch-sim`
(`harness/bench/results/20260611T181830Z-5727ee4-instr.tsv`):

| row | C Ir/it | rs Ir/it | Ir ratio | C Bc/it | rs Bc/it | rs Bcm/it |
|---|---|---|---|---|---|---|
| table_setfield_same | 77 | 183 | 2.38 | 9 | 30 | 0 |
| table_seti_same | 61 | 154 | 2.52 | 8 | 22 | 0 |
| global_settabup_same | 77 | 189 | 2.45 | 9–12 | 31–32 | 0 |
| table_settable_string_key | 93 | 199 | 2.14 | 11 | 32 | 0 |

**Classification (per MEASUREMENT_PROTOCOL §model):** instruction-removal, NOT
CPI. Bcm ≈ 0/iter on every row (branches predict perfectly) — the gap is wasted
*work* (~100 extra Ir, ~21 extra conditional branches per write vs C), not stalls.
So `instr-count.sh` Ir is the arbiter, with Bc as a layout-immune cross-check
(a removed branch shows in Bc regardless of code placement; Ir alone is hostage
to the ±2–3% whole-binary layout floor — a call-free control once moved 12% wall
on layout, and here fibonacci/mandelbrot, which share no setter code, swung
±1–2.4% Ir between builds).

Where the ~21 extra branches/write live, and why each is out of scope:
1. `RefCell<TableInner>::borrow_mut()` per write (C has no borrow flag) —
   removable only via `unsafe` (forbidden) or a layout change (T3).
2. Bounds-checked `Vec`/stack accessors: `try_update_int`'s array store does an
   `alimit` semantic check AND a `Vec` index bounds check (`alimit ≤ array.len()`
   is a runtime invariant LLVM cannot prove, so the second check stays);
   `get_at`/`set_at` likewise. This is exactly the T4 ablation target — never
   merges.
3. `LuaValue::is_collectable()` is a multi-variant `matches!` (discriminants
   {4,5,6,7,9}) → ~2 conditional branches, vs C's `iscollectable` single
   `rawtt(v) & BIT_ISCOLLECTABLE` bit-test (0 branches). Closing it needs a
   `LuaValue` representation change (collectable bit / variant reorder) — T3
   territory and the packet's STOP condition.
4. `Result<(),LuaValue>` plumbing through the `try_update_* → raw_set_* → match`
   layers (C returns a raw `TValue*`). Inlines partially; the residual
   miss-arm match is a branch C folds into a pointer comparison.

Candidate changes tested and dropped:

- **C1 node-walk single-match** (`TableNode::short_str_key()` collapsing
  `key_is_short_str()` + `key_string()` — two enum-tag reads → one, matching C's
  `keyisshrstr(n) ? keystrval(n) : NULL`). Real but tiny: **−1 Bc/iter** on the
  two string rows (setfield 30→29, settable 32→31; table_seti_same int path
  unchanged at 22, as predicted — confirms the −1 is the removed string-walk
  match), Bcm flat. Ir effect (+2/iter) is below the layout floor → fails the
  mechanical bar (Ir down on ≥2 rows). Reverted. (A legitimate ~3-line
  simplification + 2 dead-accessor deletions if landed as non-perf cleanup.)
- **C2 C-faithful fastget→finishfastset restructure** of OP_SETFIELD +
  OP_SETTABUP: try an in-place overwrite first (a new `try_overwrite_short_str`
  with C's `!isempty(slot)` semantics — a present key with a *nil* value is a
  MISS so `__newindex` still fires; verified byte-identical to lua5.4.7 on the
  nil-slot `__newindex` edge case), checking `has_metatable` only on the miss.
  Removes `has_metatable` from the hit path — but the added `!isempty` check
  trades 1:1 for it. **Bc/iter IDENTICAL to baseline on all four rows
  (30/22/32/32)** → net-zero branch change, confirmed by the layout-immune
  arbiter; Ir swings were pure layout noise (fibonacci −2.4% in the same build,
  shares no setter code). Reverted.

Load-bearing discovery: **our no-metatable setter fast path is already at
branch-parity with C's fast path** — we skip C's `isempty` probe (safe without a
metatable, both outcomes just store) in exchange for the `has_metatable` branch.
Restructuring to mirror C's exact shape is therefore a wash. The remaining ratio
is not setter-logic; it is the safety/representation tax T4 measures and T3
addresses.

Evidence TSVs (`harness/bench/results/`):
`20260611T175932Z-e4797e3-t2-baseline.tsv` (Ir baseline),
`20260611T181830Z-5727ee4-instr.tsv` (branch-sim baseline),
`20260611T182125Z-…` (C1 branch-sim), `20260611T182748Z-…` (C2 branch-sim).
Gates on each candidate: `cargo test -p lua-types` + `-p lua-vm` green,
multiversion_oracle 165/165. No control regression observed beyond layout noise.

### T5a — repr rung 1: `LuaValue` collectable-range reorder — KEPT (2026-06-13, branch `overnight/repr-tag-layout`)

**Verdict: the discriminant reorder closes T2's `is_collectable` lever — KEEP.**
This is the direct follow-on to T2's finding #3 (`is_collectable()` is a
multi-variant `matches!` over the split collectable set `{4,5,6,7,9}`, lowering
to a bitmask-constant test vs C's single `BIT_ISCOLLECTABLE` bit-test). The fix:
reorder the `LuaValue` variants so the five GC-managed variants (`Str`/`Table`/
`Function`/`UserData`/`Thread`) form a contiguous tail, moving the scalar
`LightUserData` up out of the middle of that block. With contiguous
discriminants the name-based `is_collectable()` lowers to one fused niche-decode
+ range compare (`sub`/`cmp`/`ccmp`/`cset`, 6 insns) instead of the prior
`…/mov #752/lsr/and` bitmask test (11 insns) — verified in standalone codegen.

**Crucially NOT `#[repr(u8)]` + explicit `= N`** (which the rung-1 brief
suggested): a primitive `repr` forces a dedicated tag byte, defeating the niche
packing that keeps `LuaValue` at 16 bytes — it bloats to **24 bytes** (the
largest payload `LuaClosure` is itself 16 B with no spare niche). The
`const _: () = assert!(size_of::<LuaValue>() == 16)` (64-bit-gated) caught this.
**Declaration order gives the identical codegen win at zero size cost**, so the
landed change is a pure variant reorder under the default repr. Zero new
`unsafe`; the contiguity invariant is pinned by a unit test
(`collectable_variants_are_a_contiguous_range`).

Baseline frozen at `origin/main` (ccb8a3a):
`harness/bench/results/20260613T043832Z-ccb8a3a-t5a-base.tsv`; candidate
`20260613T044607Z-ccb8a3a-t5a-cand.tsv`. Per-iteration budgets, `--branch-sim`:

| row | Ir/it base→cand | Bc/it base→cand | total Ir Δ | total Bcm Δ |
|---|---|---|---|---|
| table_setfield_same | 183 → **182** (−1) | 30 → 30 (flat) | −0.55% | −2.27% |
| table_seti_same | 154 → **147** (−7) | 22 → 22 (flat) | −4.54% | −2.48% |
| method_calls | 1370.84 → **1358.83** (−12.0) | 202.13 → **201.13** (−1) | −0.88% | −33.2% |
| gc_pressure (control-ish) | n/a (no iter count) | — | −0.22% | −1.84% |
| fibonacci (control) | n/a | Bc +190 on 11.7e9 = **0.000%** | −1.54% (layout) | +100% (layout) |

**Classification:** instruction-removal (Ir), confirmed layout-immune on
targets. The per-iteration budgets land on exact integers — these are real
deleted instructions, not the ±2-3% whole-binary layout floor. Setter rows lose
Ir with Bc/it flat (the bitmask `mov/lsr/and` work removed without changing
branch count); `method_calls` additionally loses **−1 Bc/call** — a genuinely
removed conditional branch on the upvalue-barrier `is_collectable` (the
layout-immune smoking gun). The fibonacci control's −1.54% Ir is the known
layout floor (it shares no `is_collectable` code; its Bc is exactly flat at
+190/11.7e9, so no control branch logic moved — not a real win, not a
regression; its Bcm doubling is a relayout artifact of the predictor model, for
which Bc not Bcm is the trustworthy axis). Expected-small win (1-4.5% Ir on
targets), in the predicted direction → KEPT.

Reach of the lever (every callsite the reorder optimizes, from a workspace
sweep): 5 per-write setter-path callsites (`vm.rs` OP_SET* barriers + `state.rs`
`raw_set`), 3 upvalue/method-dispatch barriers (`vm.rs` OP_SETUPVAL +
`state.rs` `upvalue_set`), 3 cold GC-barrier guards. The setter and method
workloads exercise exactly these.

Gates (full correctness battery — value representation is correctness-critical):
`cargo test -p lua-types`/`-p lua-vm`/`-p lua-stdlib` green; multiversion_oracle
**165/165**; GC canaries **all PASS** (incremental + generational, 0 FAIL);
gc.lua + nextvar.lua + events.lua **PASS plain AND under
`LUA_RS_GC_QUARANTINE=1`** (GC marking reads the value tags — proves the reorder
didn't break collectable detection in sweep); `cargo test --workspace` 0 fail;
`cargo check -p lua-vm --target wasm32-unknown-unknown` green (size assert
correctly 64-bit-gated); `specs/oracle/check.sh 5.1` 57/0 (the type-tag-sensitive
version). `size_of::<LuaValue>()` unchanged at 16 B.
