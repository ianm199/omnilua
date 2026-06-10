# Generalizable Rust perf tooling — open-source extraction candidates

Captured 2026-06-10 after the W2.3/codegen-parity wave, while the scars are
fresh. The thesis mirrors the port-harness product thesis: the custom tooling
we built to chase C parity in safe Rust solves problems the Rust ecosystem
has no good answer for. Ranked by (size of ecosystem gap) x (generality).

## 1. Displacement-aware A/B gate (working name: `cargo-ab` / `abgate`)

**The gap.** No existing tool answers "did my change regress this BINARY on
these workloads — and is the regression real work or layout luck?" with a
CI-gateable verdict. hyperfine times commands but has no verdict protocol.
criterion is in-process microbenches. iai-callgrind is the closest prior art
(callgrind counts) but is a Linux-only libtest harness, not a whole-binary
cross-platform A/B tool — and nothing anywhere does the triage step.

**What we built that proves it works** (battle-tested across ~15 packets):

- Wall half (`harness/bench/compare_bins.sh`): interleaved A/B pairs
  (thermal/clock drift hits both sides symmetrically), auto repeat-calibration
  to >=0.5 s samples, median-of-pair-ratios + fraction-faster, verdict bands
  (improved / regressed-minor / regressed / inconclusive), `--gate`/`--strict`
  tolerance policy, provenance (diff sha256, binary sha256s, dirty flag),
  output-equality as a built-in correctness oracle, calibration/match caches,
  per-sample process-group watchdogs, one-measurement-at-a-time host guard.
- Counts half (`harness/bench/instr-count.sh`): deterministic instruction
  counts via cachegrind `--cache-sim=no` in a Linux container — works on
  macOS/ARM hosts where valgrind does not run natively; per-iteration budgets
  from a workload manifest; differential probes (below).
- The triage protocol joining them: `wall_ratio = Ir_ratio x CPI_ratio`.
  Wall up + instructions flat = code-layout displacement (waivable; PGO
  erases it). Instructions up = real work (block). Live demo from 2026-06-10:
  `table_seti_same` +9.8% wall on +0.7% instructions — every existing tool
  fails that CI gate; the protocol correctly classified it.

**v1 shape.** Single Rust CLI (the bash/python here is repo-entangled glue):
point it at two binaries or two git refs + a workload manifest; native
valgrind on Linux, container fallback elsewhere; JSON + human table out;
exit code for CI; regression classification in the report. Fold in the
host-lock and watchdog. Loud doc requirement: workloads must be
deterministic (fixed seeds, no wall-clock dependence) or the counts half is
meaningless. Estimated 1-2 focused weeks.

## 2. `heapdiff` — comparative allocation decomposition

**The gap.** dhat-rs produces single-run profiles; there is no differ. The
question that actually drives optimization is comparative: "between these
two binaries (or vs this C reference), what changed per callsite in
allocation count / total bytes / peak, and is a memory gap CHURN or OBJECT
SIZE?"

**What we built**: `harness/bench/table-bytes.sh` (per-shape live
bytes-per-object, both implementations, baseline-subtracted) + the
dhat-vs-counting-allocator method. One afternoon of it redirected the whole
RSS strategy (allocation counts were at parity with C; objects were ~3x
bigger) and found a ~50 B/object side-table residue nobody suspected.

**v1 shape.** Take two dhat JSON files (or run the binaries), emit
per-callsite deltas + a count-vs-size decomposition table; optional
"per-scenario" mode that runs shape snippets and reports bytes/instance.
A few days of work. Pairs with #1: the A/B gate flags an RSS verdict,
heapdiff explains it.

## 3. The methodology as published content

The patterns are as valuable as the tools and travel further with the essay.
Already written up internally (`docs/PERFORMANCE_PRINCIPLES.md` patterns
section, `docs/PERFORMANCE_MODEL.md`):

- Recount-before-bench: falsify candidates with deterministic counts before
  paying for wall-clock A/Bs.
- Differential probes: `Ir(workload) - Ir(loop_only)` = exact per-feature
  instruction budgets, for both your binary and a reference.
- Displacement waivers must be proven (flat recount), never argued.
- Single-bounds-check register windows; port-scaffolding audits ("does the
  reference implementation do this work at all?").
- The rejected-experiments registry: machine-readable falsified-optimization
  log with mechanisms and retry conditions — anti-re-derivation memory,
  especially relevant for agent-driven development.

This also feeds the lua-rs visibility strategy: "the tooling we built
chasing C parity in safe Rust" is a strong story for the same audience as
the wasm playground wedge.

## Explicitly NOT worth extracting

- **PGO pipeline** — `cargo-pgo` exists and is good. Our only addition is
  pinned-training-set discipline + variant-labeled results (doc note, not a
  tool).
- **Dashboards/ledgers** — bencher.dev, CodSpeed et al. own this. Our novel
  bit is the variant-labeling rule (never silently compare PGO history
  against stock history).
- **Bytecode/structural parity gating** — that is port-harness product
  material (oracles against a reference implementation), already on the
  extraction roadmap. Lesson to carry: the gate's own regex misread luac's
  RETURN0/1 for a day — verify the verifier.

## Extraction discipline

Same proof-by-consumption pattern as the port-harness: extract the tool,
then port THIS repo's harness to consume the extracted version before
announcing anything. lua-rs-port is the first customer and the regression
test.
