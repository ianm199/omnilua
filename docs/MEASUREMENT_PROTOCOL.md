# Measurement protocol — how a perf claim becomes true here

A perf change without this protocol is **unverified**, the same way a behavior
change without the oracle is unverified. This document codifies the protocol
that ran the 2026-06-11 burndown (`docs/ISSUE_BURNDOWN_SPEC.md` — read its
verdict entries as worked examples). It exists because this rig (macOS/arm64,
Apple M3 Max) cannot attribute small wall-time deltas, and because subagents
will rationalize noise into wins unless the success bar is mechanical.

## The model: wall = instructions × CPI

Every win belongs to one of three classes, and each class has a different
arbiter. Classify the packet BEFORE measuring, because the classes read
opposite signatures as success:

| Win class | What moved | Signature | Arbiter |
|---|---|---|---|
| Instruction removal (fast paths, dead work) | fewer retired instructions | Ir DOWN on the target | `instr-count.sh` (deterministic callgrind) |
| CPI / branch (deleted mispredicting branches, layout) | same instructions, fewer stalls | Ir FLAT, wall down, Bcm down | `instr-count.sh --branch-sim` (Bc/Bcm) |
| Latency scaffolding (locks, allocator round-trips) | same instructions, less wait | Ir FLAT, wall down a lot | wall + a removed-work argument (enumerate the lock/alloc ops deleted) |

Worked examples from the burndown: T2-D was an instruction-removal hypothesis
killed by Ir going UP (+0.22%) — the assumed clone overhead didn't exist
(`GcRef`/`LuaValue` are `Copy`). T2-C2 was a branch win: targets −8.6%/−4.4%
wall with Ir exactly flat. T2-B2 was a latency win: pingpong −32% wall, Ir
−0.012% — four global hook-lock ops and 3-4 allocations per resume deleted.

## The wall protocol (when you must measure wall at all)

1. **Freeze the baseline**: build a release binary from origin/main BEFORE any
   edits, copy it out of `target/` (e.g. `/tmp/lua-rs-<packet>-base`), record
   its build sha. Same-source rebuilds are valid baselines (verified
   2026-06-11: frozen vs fresh-rebuilt baseline gave identical ratios).
2. **Interleave**: alternate base/candidate within each round, never
   back-to-back blocks. Sub-0.5s workloads run as 8-loop aggregates under
   `/usr/bin/time -p`. ≥4 rounds; judge the min-ratio.
3. **Quiet machine for final numbers.** Implementation agents may report
   provisional interleaved numbers under load; every number that enters a PR
   body or a spec checklist is re-measured quiet by the supervisor.
4. **Revert validation** for any surprising result: revert the change, rebuild
   from clean, confirm the ratio returns to ~1.0. This is what proved T2-C's
   regressions were real and not drift.

## Noise floors (this rig — re-derive if the host changes)

- Measurement floor: **±1%** (base-vs-base interleaved min-ratio spread).
- Code-layout floor: **±2-3%**, occasionally far worse — a call-free compute
  control once moved **12% wall** from a whole-crate layout shift (Ir +0.55%).
  Corollary: **any sub-5% wall claim needs a second axis** (Ir or Bcm), and a
  moving control invalidates per-target wall attribution for that build pair.
- Ir floor: ~single-digit counts per billion — effectively exact.
- PGO is worth ~0.1-0.2 of wall ratio overall (stock 1.58 / PGO 1.38 at
  v0.0.33). Never compare a stock number against a PGO number as if same-build.
- Trust `_long` workloads; sub-100ms rows are startup-dominated. The matrix's
  `repeat` column shows the auto-calibration.

## Rules that exist because they were violated once

- **Profiles are evidence about ONE commit.** A packet was specced (T2-B) off
  a profile predating a fix already on main, and came back wall-neutral.
  Check the profile's build sha against current main before speccing; if it
  doesn't match, re-profile.
- **Drop-if-neutral.** A neutral result is reported as neutral and the code is
  reverted — honest negatives are deliverables and get recorded in the spec's
  verdict ledger. Do not keep changes "because they should help."
- **Bench host is exclusive**: one measurement process at a time across all
  repos on this machine (see `../AGENT_COORDINATION_BOARD.md`). Parallel cert
  generation and parallel benches both fabricate regressions.
- **The wasm gate is part of the ladder**: `cargo check -p lua-vm --target
  wasm32-unknown-unknown` before any PR — size/layout assertions are 64-bit
  claims unless gated (`#[cfg(target_pointer_width = "64")]`), and this broke
  main's CI once (fixed in PR #153).


## The Ir arbiter's blind spot: CPI / code layout (added 2026-06-13)

Deterministic instruction count (Ir) is the arbiter for instruction-removal and
the cross-check for branch wins — but it is BLIND to CPI/code-layout effects.
A change can reduce Ir yet run SLOWER because it reshuffled codegen (i-cache,
branch alignment, type-dispatch layout). This bit us: the T5a `LuaValue`
discriminant reorder cut Ir on arithmetic rows ~2.7% but regressed their WALL
~10-15% (reverted). RULE: any change that alters codegen LAYOUT — enum-variant
reorders, struct field order, `repr`, anything that moves the discriminant or
shifts hot-path branch targets — REQUIRES a cold-machine wall A/B, not just Ir.
Ir-down is necessary but NOT sufficient for these. Pure logic changes that don't
move layout can still trust Ir.

## Tool index

- `harness/bench/instr-count.sh` — deterministic Ir (callgrind, container);
  `--branch-sim` adds Bc/Bcm (the CPI arbiter).
- `harness/bench/compare.sh --runs 5` — the ratio matrix vs reference C;
  ledger rows are committed at milestones.
- `harness/bench/heap-diff.sh` — alloc-count / bytes-per-block delta between
  two commits for one workload (RSS packets).
- `cargo run -p lua-vm --example value_layout` / `value-layout.sh` — struct
  byte sizes vs C's (`table-bytes.sh` for per-shape heap detail).
- `harness/bench/profile-hotspots.sh` + `vm-execute-attribution.py` — sampled
  hotspots and per-opcode-region attribution inside `vm::execute` (see script
  header for the agent-harness caveat/fallback).
- `harness/canaries/gc/run_canaries.sh`, `LUA_RS_GC_QUARANTINE=1`,
  `harness/asan-stress.sh` — the correctness battery; required whenever a perf
  change touches GC, barriers, stack rooting, or sweep paths.

## When to stop

The project parity threshold is wall ≤ 1.5×; the matrix crossed it on
2026-06-11 (overall 1.47 stock). Past that line, tail-row wall work competes
against RSS work and strategic work and usually loses — get the safety-tax
ablation number (`docs/PERF_SPRINT_2_GOAL.md` §T4) before sponsoring any
further wall packet, because it bounds what the remaining tail can ever yield.
