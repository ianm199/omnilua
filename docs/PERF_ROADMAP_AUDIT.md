# Perf roadmap audit — is it executed? (2026-06-13)

Answer to "besides NaN-boxing/unsafe, is the perf roadmap executed?": **the WALL
half is essentially exhausted of safe levers; the RSS half is NOT — there are
four safe, planned, deferred packets, one of which no prior doc had named.**
Plus one safe wall lever remains (fasttm). This audit is grounded in the
2026-06-13 exhaustive recon (read-only, every thread verified against source).

Measurement discipline for everything below: `docs/MEASUREMENT_PROTOCOL.md`
(frozen baseline, Ir/branch-sim/heap-diff arbiters, drop-if-neutral).

## Wall (instruction/CPI) roadmap — DONE except two ceilings

Landed and verified: opcode specialization (ADDI/MULK/BANDK/EQK…, v0.0.31),
call-frame `CallInfoFrame` flatten (burndown T2-C2), `is_collectable`
tag-layout reorder (overnight T5a — `is_collectable` 11→6 instructions, a real
−1 Bc/call on method_calls), concat string-churn (overnight T4 — Ir −13.5%).

Remaining wall, by category:
- **fasttm absence cache (SAFE, undone)** — see Thread D below. The one safe
  wall lever left. The `TableFlags` field and `invalidate_tm_cache` skeleton
  exist but are INERT: `fast_tm_table` (state.rs:3691) re-walks the
  metatable's hash on every miss-path access where C skips via a cached bit,
  and the VM-side `invalidate_tm_cache` (state.rs:1067) is an empty no-op.
- **Result→unwind error propagation** — BEING MEASURED (overnight T5b
  prototype, `docs/UNWIND_ERROR_PROTOTYPE.md` when it lands). The safety-tax
  ablation named `Result` plumbing as a residual idiom-tax layer; T5b sizes
  whether the VM-wide conversion is worth it. Go/no-go pending.
- **Dispatch CPI ceiling (LANGUAGE-CONSTRAINED)** — the branch-sim arbiter
  showed fibonacci carries ~3.3× C's indirect-branch mispredicts: the
  `match`-based dispatch loop vs C's computed-goto, where each opcode handler
  has its own indirect jump the predictor learns per-site. Rust has no stable
  computed-goto or guaranteed tail-call dispatch (`become` is unstable). Not a
  clean safe win; this is the structural floor.

The safety-tax ablation (burndown T4) already proved the rest: ≥1.9× C
instructions remain after deleting ALL bounds/RefCell checks, so the residual
is representation/idiom. Closing it below ~1.2× needs NaN-boxing (excluded).

## RSS roadmap — NOT executed; the live grind set

Landed: UpVal mirror removal (sprint-2 T1, box 104→72 B, closure_ops RSS
−8.3%), lazy weak-token registration (sprint-2 T3b, peak live −10/−12% on
alloc rows), table metatable-slot diet (R2, box 144→128 B).

**Remaining SAFE RSS packets (ranked, the tonight grind):**

1. **B — closure `upvals` `Vec→Box<[T]>`** (running 2026-06-13). Recon
   confirmed the upvalue count is fixed at creation and never resized
   (`set_upval` is a `Cell::set` into an existing slot, not a push). Clean
   construction-only change; −8 B/closure box; targets closure_ops RSS ~2.98×.
2. **F1b — `LuaString` `Rc<[u8]>→Box<[u8]>`** (queued). THE UNNAMED LEVER no
   doc called: an `Rc<[u8]>` co-locates a 16-byte refcount header with the
   payload, so every string heap allocation carries that header on top of its
   `GcBox<LuaString>`; strings are immutable and GC-owned (shared via `GcRef`,
   not `Rc`), so `Box<[u8]>` drops the 16 B header per string + the refcount
   inc/dec on `clone`. Safe — unlike the full inline-TString single-allocation,
   which needs the excluded unsafe DST surgery. Risk: audit `LuaString::clone`
   callsites (the derive) to confirm none clone the value rather than the
   `GcRef`. Evidence is heap-diff (bench host) — queued behind T5b.
3. **A — table `array`/`node` `Vec→Box<[T]>`** (running 2026-06-13; candidate
   9 / GC_ALLOC_DESIGN_MEMO R4). Recon decisively confirmed growth is
   rehash/resize-boundary only (no `array.push`, no per-element `node.push`),
   so it's faithful to C and clean; −16 B/table box. The memo's "sequence
   after T2" note is moot — T2 (setter family) was RESOLVED-NEGATIVE with zero
   `.rs` diff, so table.rs is unconflicted.
4. **C — InternedStringMap shrink-when-sparse** (queued, minor). Buckets only
   grow/double, never shrink (state.rs:130-207); ~24 B × stale-bucket-count
   retained after a transient short-string population drops. Safe if the
   shrink check is deferred to the end of the dead-string removal batch (not
   per-removal → avoids O(live²)); C's `luaS_resize` shrinks too, so it's
   C-faithful. Low value (the bench set's string churn is mostly long,
   un-interned strings).

## Excluded (NaN-boxing / unsafe) — out of scope by user direction

- **GcHeader sub-40 B (R5)** — REJECTED: hot-field packing already measured
  +4% Ir; the remaining 32 B is two `dyn Trace` fat pointers; going lower
  needs thin-pointer + vtable-recovery unsafe.
- **CallInfo 72→64 B (T2-C2 Option B)** — needs a `repr(C) union` and a
  lua-vm unsafe-budget raise from 0; gated behind a control-isolated wall win
  that the ablation showed doesn't exist.
- **Size-class free lists for GcBoxes (R3)** — allocator surgery, must coexist
  with the quarantine HDR_FREED tripwire, partly unsafe; deferred pending
  human sign-off.
- **Inline-TString single allocation (F1 full)** — the other half of F1b;
  needs an unsized `GcBox<header + [u8]>` DST, unsafe pointer coercion.
- **Computed-goto / tail-call dispatch** — language-constrained (see wall
  ceiling above), not achievable in stable safe Rust.

## Assess-only — not a grind packet

- **E — GC pacer cadence (R6).** Recon found a strategic gap, not a quick win:
  the default (Incremental) production path does a STOP-THE-WORLD
  `full_collect_with_post_mark` on every threshold breach (heap.rs:2042); the
  genuinely-incremental budgeted stepper exists but is wired only to
  `collectgarbage("step")`, never to the automatic VM cadence. Wiring the real
  incremental stepper into the default path is the C-faithful design but a
  multi-day, correctness-critical change (the barrier invariant must hold
  across many partial steps). The known #113 generational-pacing regression is
  in `generational_step_with_major` but off the default path (generational is
  opt-in via `collectgarbage`). Memo R6 also warns cadence tuning measures the
  allocator until the alloc count drops. Verdict: assess, don't grind tonight.

## Verdict

The roadmap is **not** fully executed. Tonight's grind closes the safe RSS
levers (B, A running; F1b, C queued) and, if T5b's Result-tax is large enough,
opens the one remaining safe wall lever beyond fasttm. After this grind, the
only perf work left is either language-constrained (dispatch), correctness-
critical multi-day (incremental pacer), or behind the excluded NaN-boxing/
unsafe line — i.e. the safe, bounded perf roadmap will be genuinely exhausted.

(Status section appended by the supervisor as packets land.)

## Status — overnight 2026-06-13 grind COMPLETE

The safe/bounded perf roadmap is now executed. Outcome of every thread:

**Landed (6 PRs):**
- concat string-churn (#176) — Ir −13.5%, allocations −43.5% (13.9M→7.8M/run).
- `is_collectable` tag-layout reorder (#177) — 11→6 instructions, −1 Bc/call.
- closure `upvals` `Box<[T]>` (#180) — LuaLClosure 32→24 B.
- table `array`/`node` `Box<[T]>` / candidate 9 (#181) — GcBox<LuaTable> 128→112 B.
- `LuaString` `Rc<[u8]>→Box<[u8]>` (#182) — −16 B refcount header per string.
- InternedStringMap shrink-when-sparse (#184) — bounds worst-case intern memory
  (C-faithful `luaS_resize`); synthetic test proves 64→2048→64 bucket reclaim.

**Measured negatives (the equally-valuable half):**
- Result→unwind conversion (#183 memo, prototype branch `proto/unwind-errors-
  tableset`) — Result-tax ~1.5 Ir/write, below the layout floor; **NO-GO**.
  Corrects the earlier ~20-35% estimate: the compiler already elides the
  always-`Ok` happy path.
- fasttm absence cache (Thread D) — **DROPPED**. Invalidation fully proven
  correct (events.lua metamethod oracle + 8 invalidation tests across 5
  versions, incl. the subtle nil-repopulation case), but the Ir arbiter showed
  it *regresses* representative rows (metatable_index_chain +1.35%, method_calls
  +1.04%) to win only a synthetic absent-TM probe (−14.5%). Our per-access
  machinery is heavier than C's near-free bitmask, so the faithful C
  optimization does not transfer. Reverted; tree byte-identical to main.

**Cumulative deterministic check (machine-immune Ir, dec6a11 → HEAD):**
even compute rows the night did not target improved — numeric_mixed −2.72% Ir,
compare_immediates −2.47% Ir (the tag-layout reorder reaching the arithmetic
metamethod-check path).

**Matrix caveat:** the closing single-run wall matrix (20260613T065155Z,
overall 1.48) is NOT trustworthy — the machine ran heavy all night, so absolute
wall is thermally inflated; the *same* binary's compute rows show LOWER Ir than
pre-overnight, proving the wall "regressions" are machine-state, not code. A
clean wall before/after needs a cold-machine best-of-N re-run. The structural
RSS wins are guaranteed by the type changes (value_layout, deterministic); the
matrix RSS column still shows them through the noise (binarytrees 2.69→2.22,
string_ops 1.57→1.28, sort_seeded 1.07→0.68).

**Verdict:** the safe, bounded perf roadmap is genuinely exhausted. What remains
is only NaN-boxing / unsafe (excluded by direction) and the multi-day,
correctness-critical incremental-pacer rewrite (assess-only). No further safe
perf grind exists to pull.

## CORRECTION (2026-06-13, cold-machine re-measure): T5a tag-layout REVERTED

The overnight status above reported the T5a `LuaValue` tag-layout reorder (#177)
as a clean win. **That was wrong.** A cold-machine matrix + direct
binary-vs-binary A/B showed T5a is a net-negative TRADEOFF:

- It hurts arithmetic ~10-15% wall: numeric_mixed pre 3.91s → with-T5a 4.33s →
  reverted 3.93s; compare_immediates 8.29 → 9.54 → 8.37. (bitwise_mixed,
  fibonacci, mandelbrot regressed similarly in the matrix.)
- It helps tables/methods ~4-6%: metatable_index_chain 1.80 → 1.69 → 1.82;
  method_calls gains, but stays better-than-baseline without T5a (the table/RSS
  packets carry it).

Arithmetic is pervasive and the regression is larger, so T5a is reverted
(commit reverting 5e0a45f). All OTHER overnight work stands (concat, the three
`Box` RSS diets, intern-shrink — clean wins).

**Why the Ir arbiter missed it — the methodology gap:** T5a passed its gate
because its arbiter was instruction count, and Ir *fell* (numeric_mixed −2.7%).
But the regression is **CPI/code-layout** — the discriminant reorder reshuffled
the arithmetic type-dispatch codegen so the same (fewer) instructions run
slower. Deterministic Ir is blind to this. For any change that affects codegen
LAYOUT (enum-variant reorders, struct field reorders, repr changes), a
cold-machine WALL check is mandatory — Ir alone is insufficient. This is the
one class where the Ir arbiter must not be trusted on its own.
