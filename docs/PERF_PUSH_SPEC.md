# Perf push spec — measurement floor first, tradeoffs last

Status: EXECUTING (approved 2026-06-09; phases P0/P1/P2.1-2.3/P3/P4.1/P6/P7
done; Wave 2 in progress — see §Wave 2 and the acceptance checklist for live
state). Derived from the 2026-06-09 deep audit of `docs/PERFORMANCE_MODEL.md`,
`docs/MATCHING_C_PERFORMANCE.md`, `harness/bench/*`, and the
`lua-vm`/`lua-types` hot paths.

Execution ledger: ~45 commits on main since `v0.0.32` through 2026-06-10.
Open work, priority order: W2.3 RSS/alloc parity; codegen divergence packets
(task 11, incl. a CORRECTNESS look at the RETURN-specialization class);
dispatch-preamble attribution (needs callgrind on a bigger box); W2.2
call/return diet; P4.2-4.4 build-knob experiments (fat-LTO / fast-alloc /
panic-abort — still unmeasured); W2.5/P5 safety-tax ablation.

This spec is the work plan that turns the audit into packets. It follows the
house rules: every packet has a gate, the oracle is the only truth-teller, no
new `unsafe` in shipped crates, no benchmark-only fast paths. Read alongside
`docs/PERFORMANCE_MODEL.md` (current bottleneck model) and
`docs/PERFORMANCE_PRINCIPLES.md` (rules).

## 0. Goal, definition of done, non-goals

**Goal.** Before any safe-Rust tradeoff conversation, (a) make every
performance number trustworthy at the precision we act on, (b) add the
decomposition layer that splits "we execute more instructions" from "our
instructions stall more", (c) collect the cheap structural wins the audit
found, and (d) produce an empirical price list for each safety mechanism so
"we've hit the safe-Rust floor" becomes a measured claim, not a feeling.

**Definition of done.**

- Every tall-pole ranking in `PERFORMANCE_MODEL.md` is backed by samples
  ≥0.5 s at the measured binary, with provenance (diff hash + binary hashes)
  in the artifact header.
- For each matrix workload we can print: wall ratio, instruction-count ratio,
  and the residual CPI factor — for lua-rs *and* the C reference.
- A conflicting A/B verdict (like the 2026-06-09 0.880-vs-0.988
  `global_settabup_same` pair) is resolvable with one deterministic recount.
- Codegen parity with `luac -l` is a mechanical gate, not a discovery.
- PGO, fat LTO, fast-alloc, and panic-abort each have a measured verdict.
- A committed safety-tax table prices bounds checks, `RefCell` borrows, and
  dispatch shape per workload.
- The model docs are corrected and have one source of truth per fact.

**Non-goals.** No JIT. No `unsafe` in shipped runtime crates (the price-list
branch never merges). No changes to the pinned reference source used by
oracles or ledgered benches. No new benchmark workload that a real Lua program
wouldn't exercise.

## 1. Audit facts this spec is built on

Each later phase cites these. Verified 2026-06-09 against the tree at
`d0dc949`:

1. **`StackValue.tbc_delta` is dead.** Defined at
   `crates/lua-vm/src/state.rs:414-417`; the only non-constructor reference is
   a write of `0` at `state.rs:2517`; nothing reads it. The real to-be-closed
   mechanism is `LuaState.tbclist: Vec<StackIdx>` (`state.rs:2021`,
   `crates/lua-vm/src/func.rs:293`). Every stack slot is 24 B where C's is
   16 B, for a field that does nothing. `StackValue` does not leak outside
   `lua-vm`.
2. **The tall-pole table was measured on 50–70 ms samples.** In
   `20260609T163417Z-1dee1e5-bin-ab.tsv` the C wall for `table_seti_same`,
   `table_setfield_same`, `global_settabup_same`, `gc_pressure` is
   0.02–0.07 s, read at centisecond resolution with process startup included.
   The `repeat_each 5` reruns (0.6–0.8 s samples) are the valid shape.
3. **Headline Linux ratios come from `ubuntu-latest`** (ephemeral hardware,
   `.github/workflows/release.yml`), so cross-release Linux trends conflate
   instance drift with code drift.
4. **`compare.sh`/`compare_bins.sh` run all-A-then-all-B** with no
   interleaving, no warmup, no provenance of the working-tree diff or binary
   hashes, and `compare.sh` has no `--repeat-each`.
5. **No instruction-count or hardware-counter tooling exists.** macOS
   `sample` + opcode counts is the finest grain. The C reference is never
   profiled or counted. "Code layout moved unrelated workloads" is currently
   unfalsifiable.
6. **`table_ops_long` is at 0.43x** (2.3x faster than C) after the v0.0.32
   table-set fast paths — the matrix now mixes VM-parity rows with
   stdlib-algorithm-divergence rows without labeling them.
7. **C's `trap` serves two masters** — hooks *and* stack-reallocation pointer
   revalidation (`reference/lua-5.4.7/src/lvm.c:1139`, `ldo.c:193`). The Rust
   stack is index-based, so the realloc half does not apply; per-`CallInfo`
   trap reads are partially C-pointer-machinery mimicry.
8. **Commit `d0dc949` ("agent: auto-commit at stop") landed an unvalidated
   experiment.** It bundles the hookmask-guarded trap refresh (a shape both
   docs record as rejected; the broad variant regressed `numeric_mixed` 11%)
   with an unrelated good change (passing `code: &[Instruction]` into
   `finish_order_imm_jump` instead of re-fetching via `proto_code`), plus
   scratch `harness/impl/official/*.stop.out` files that the harness guide
   says must never be committed.
9. **`GcRef` is `Copy`** (`crates/lua-types/src/gc.rs:163`) and `LuaValue` is
   a 16-byte `Copy` enum; `PERFORMANCE_PRINCIPLES.md` §5 still describes a
   refcount bump that does not exist. The real per-op tax is interior
   mutability: `LuaTable` carries `RefCell`/`Cell` layers
   (`crates/lua-types/src/table.rs:1365-1390`) and every setter borrows
   per call (`table.rs:1533` → `:984`).
10. **Allocator/heap tooling already half-exists and is undocumented:**
    `lua-cli` has `fast-alloc` (mimalloc) and `dhat-heap` features with
    `#[global_allocator]` gates (`crates/lua-cli/src/main.rs:1436-1447`,
    `crates/lua-cli/Cargo.toml`). No doc or bench script mentions either; the
    model doc claims there is "no allocation-stack profiler" on macOS while
    dhat is wired.
11. **`catch_unwind` is confined to `lua-rs-runtime`**
    (`crates/lua-rs-runtime/src/lib.rs`), and `lua-cli` does not depend on
    that crate — so a panic-abort *CLI* build is testable without touching
    embedding semantics.
12. Host inventory (this M3 Max): `docker` and `xctrace` present;
    `valgrind`/`hyperfine`/`samply`/`luac`-on-PATH absent; reference `luac`
    built at `reference/lua-5.4.7/src/luac`; `llvm-profdata` present in the
    rustup toolchain.

## 2. Phase P0 — settle the tree (blocker, do first)

**P0.1 Adjudicate `d0dc949`'s vm.rs change.** Split the two mechanisms:

- Build HEAD and three variants: parent (`1dee1e5`), code-slice-only, and
  guard-only. The code-slice fetch removal is expected to be a pure win; the
  hookmask guard is a narrowed retry of a rejected shape and must prove
  itself.
- Gate per variant: `cargo test -p lua-vm --lib`, `make conformance`, then
  P1-style A/B at ≥0.5 s samples on
  `compare_immediates,global_settabup_same,numeric_mixed,binarytrees,fibonacci`
  (the workloads the rejected broad variant moved).
- Decision rule: keep a mechanism only on an `improved` verdict (P1.4
  definition) repeated twice; otherwise `git revert` that half with a commit
  message citing this spec and the rejected-experiments entry. Default on
  inconclusive: revert the guard, keep the code-slice change if it is at
  least neutral.

**P0.2 Untrack scratch oracle outputs.** `git rm --cached
harness/impl/official/*.out`, extend `.gitignore`, and add the same exclusion
to the stop-hook staging step (see P7.4). These files churn on every smoke run
and poison auto-commits.

**P0.3 Record the rejected-experiment retry.** Create
`harness/rejected-experiments.jsonl` (schema in P7.3) and backfill the entries
already named in the two perf docs, including the trap-guard lineage and
today's artifacts.

Estimated effort: half a session. Everything else is blocked on P0.1 because
all later A/B baselines must be a tree whose perf content is validated.

## 3. Phase P1 — measurement integrity (the runners)

All changes apply to both `harness/bench/compare.sh` and
`harness/bench/compare_bins.sh` unless noted. Ledger semantics are unchanged:
`compare.sh` stays the only ledger writer.

**P1.1 Interleave A/B.** Replace all-A-then-all-B with per-pair alternation:
for run i in 1..N, time A then B (same order every pair). Thermal and clock
drift then hit both binaries symmetrically. Keep per-pair samples for P1.4.

**P1.2 Provenance stamps.** TSV/JSON headers gain: `dirty: yes|no`
(`git status --porcelain` non-empty), `diff_sha256: <first 12 of sha256 of
git diff HEAD>` when dirty, `bin_a_sha256` / `bin_b_sha256` (first 12), and
the effective repeat factor per workload. Artifacts from a dirty tree are
fine for probes; the stamp makes them attributable afterward.

**P1.3 Minimum-sample enforcement.** Per workload: one calibration run; if
wall < `MIN_SAMPLE_S` (default 0.5, env-overridable), set
`repeat_each = ceil(MIN_SAMPLE_S / wall)` automatically. `compare.sh` gains
`--repeat-each` (auto by default) with the repeat count recorded per row. A
row whose *repeated* sample still lands under 0.2 s is reported with ratio
field `short` instead of a number. This kills the centisecond-quantization
class of false poles (audit fact 2).

**P1.4 Statistics + verdict.** From the per-pair samples report, per workload:
best-of-N ratio (back-compat headline), median-of-pairs ratio, and
`frac_pairs_improved`. Emit a machine verdict per workload and for the run:
`improved` iff median ≤ 0.99 and frac ≥ 0.7; `regressed` iff median ≥ 1.01
and frac ≤ 0.3; else `inconclusive`. `compare_bins.sh` exits non-zero on
`regressed` when `--gate` is passed, so packet gates stop being eyeball
calls.

**P1.5 Workload manifest + row classes.** New
`harness/bench/workloads/manifest.tsv` with columns:
`workload  class  iterations  notes`. `class ∈ {vm-parity, stdlib,
macro, startup}`. Runners print the class per row; `history.py` carries it
through to the dashboard. Add `startup_empty.lua` (prints one OK line) as the
`startup` row so short-row reasoning can subtract process startup explicitly.
Classify `table_ops_long` and `table_ops` as `stdlib` (audit fact 6) and file
a small investigation note for *why* we are 2.3x faster there (expected:
shift loops bypass C's per-element `lua_geti`/`lua_seti` API layering —
confirm and document, since an aggregate row that fast can hide VM
regressions).

**P1.6 Re-baseline.** After P1.1–P1.5 land (and after P3), run the full
matrix on the M3 at enforced durations, promote that TSV into
`harness/evidence/` as a committed artifact, and rewrite the "Current
Position" table in `PERFORMANCE_MODEL.md` from it. The tall-pole ranking may
genuinely change; all later packet priorities follow the re-baselined table.
CI (`ubuntu-latest`) wall-clock is demoted to smoke from this point: the
dashboard keeps plotting it, but the model doc stops citing it as the
canonical ranking (audit fact 3). The pinned M3 is the wall-clock trend
instrument; CI's regression gate moves to instruction counts (P2.1).

**P1.7 Runner self-guards.** Both runners run the
`pgrep -fl 'compare_bins|compare.sh|profile-hotspots|target/release/lua-rs'`
check themselves and abort if another measurement process is live, instead of
relying on agents remembering the workflow step.

Estimated effort: 1–1.5 sessions. P1.1–P1.4 are one coherent edit per script;
P1.5–P1.6 are mechanical.

## 4. Phase P2 — the decomposition layer

The organizing identity: `wall_ratio = Ir_ratio × CPI_ratio` (instructions
executed × cycles per instruction). Different factors demand different
medicine; today they are indistinguishable (audit fact 5).

**P2.1 Deterministic instruction counts — `harness/bench/instr-count.sh`.**
Callgrind is simulation-based: counts are ~0.1%-stable, immune to thermal and
scheduler noise, and work fine inside VMs. valgrind does not run on macOS
arm64, so runs happen in a `--platform linux/arm64` Docker container (docker
verified present):

- Image: debian stable + `valgrind` + build essentials + rustup, built once
  and cached; a named volume holds the cargo registry/target dir so rebuilds
  are incremental.
- The script builds linux-arm64 `lua-rs` (release profile) and the reference
  (`make linux`, its stock `-O2`) inside the container, then runs
  `valgrind --tool=callgrind` per workload per binary.
- Output: `results/<ts>-<sha>-instr.tsv` with
  `workload  binary  Ir_total`, plus raw `callgrind.out.*` under
  `profiles/instr/` for `callgrind_annotate` drill-down.
- Companion `instr-diff.sh <out_a> <out_b>`: top-N per-function Ir deltas
  between two runs — the deterministic A/B arbiter for small packets.
- Acceptance: two consecutive runs of the same binary differ <0.5% Ir on
  every workload. Document the expected ~20–100x slowdown (a 2.5 s workload
  takes minutes; targeted packet runs use `--workloads`, the full matrix is a
  nightly/manual sweep).
- Follow-on (separate packet): a CI job running the same container per PR on
  changed-relevant workloads, gating on >1% Ir regression — this replaces
  wall-clock as the CI signal (audit fact 3).

**P2.2 Per-iteration instruction budgets — `instr-budget.py`.** Using the
manifest's `iterations` column:
`Ir_per_iter = (Ir(workload) − Ir(startup_empty)) / iterations`, computed for
both binaries. Deliverable: a committed table in
`docs/PERFORMANCE_MODEL.md` — per workload: C Ir/iter, rs Ir/iter, Ir ratio,
wall ratio, residual CPI factor. This is the artifact that turns packets into
"remove ~N instructions/iteration" with mechanical verification by recount,
and it tells us per workload whether the gap is work-count or stalls.

**P2.3 Static bytecode-parity gate.** The #134 immediate/K-opcode misses were
found the expensive way (runtime counts + ratios); a listing diff catches the
whole divergence class at rung-1 cost:

- Add a dev-only listing binary: `cargo run -p lua-code --example listing --
  <file.lua>` printing, per function: pc, mnemonic, operands, k-flag —
  normalized to match what `bytecode-parity.py` extracts from
  `reference/lua-5.4.7/src/luac -l -l` output (`opcode_names` already exists
  in `lua-code`).
- `harness/bench/bytecode-parity.sh` diffs the normalized streams for every
  workload (and any `.lua` passed). Known, accepted divergences live in a
  checked-in allowlist with one-line justifications.
- Wire as `make bytecode-parity`; add to the PR checklist for
  `lua-parse`/`lua-code` changes. Exit non-zero on any non-allowlisted
  divergence.

**P2.4 Differential C profile — `profile-ref.sh`.** Sample the *target*, not
just our port. Copy `reference/lua-5.4.7/src` to a scratch build dir (never
rebuild the oracle binary in place — the pinned `-O2` oracle stays
byte-stable), build with `-O2 -g -fno-omit-frame-pointer`, and run the same
`/usr/bin/sample` + summary pipeline `profile-hotspots.sh` uses. Deliverable:
side-by-side top-frame tables (C vs rs) for each tall pole, operationalizing
"what does C do implicitly that we do explicitly".

**P2.5 macOS hardware counters — `profile-counters.sh` (best-effort).** Wrap
`xctrace record --template 'CPU Counters'` (xctrace verified present) +
`xctrace export` to a small table per run: instructions, cycles (→ IPC),
branch mispredicts, L1d misses. M-series event names and export schemas are
finicky; this tier is explicitly best-effort, validated once by checking its
instruction counts against P2.1's Ir within ~5%. Deliverable: a one-time
inventory table for the tall poles classifying each as
front-end/branch-bound, memory-bound, or instruction-count-bound — used to
re-rank packets and to finally explain the Linux-vs-M3 ratio split with data.

**P2.6 Allocation-volume parity.** Splits "we allocate more" from "our
objects are bigger" (104 B vs 56 B tables, measured) from "the collector
retains longer" — three different packets currently sharing one RSS number
(binarytrees RSS ratio 4.17x):

- Our side: `alloc-profile.sh` runs the existing `dhat-heap` build (audit
  fact 10 — it is already wired, just unscripted and undocumented) per GC
  workload; record total allocations, total bytes, peak, top stacks.
- C side: a counting `l_alloc` patch applied to a `/tmp` *copy* of the
  reference source (the pinned oracle tree is never modified), printing
  alloc-count/byte totals at exit.
- Deliverable: alloc parity table for `binarytrees`, `gc_pressure`,
  `closure_ops` + an RSS decomposition note in the model doc. RSS becomes a
  first-class modeled quantity instead of an unexplained column.

Estimated effort: P2.1+P2.2 1–1.5 sessions; P2.3 0.5–1; P2.4 0.25; P2.5
0.5–1 (may stall on xctrace quirks — timebox it); P2.6 0.5–1.

## 5. Phase P3 — the free representation win: delete `tbc_delta`

Remove the field (audit fact 1): `StackValue` becomes `{ val: LuaValue }` =
16 B, C parity, −33% bytes on every stack copy, grow, and scan.

- Mechanical change: drop the field, fix the 4 constructor sites and the one
  dead write. `StackValue` is `lua-vm`-internal (verified by grep), so no
  cross-crate fallout. Keep the wrapper struct for now (a later cleanup may
  collapse `Vec<StackValue>` to `Vec<LuaValue>`; not this packet).
- Gate: `cargo test -p lua-vm -p lua-types --lib`, `make conformance`, GC
  canaries (`harness/canaries/gc/run_canaries.sh`), full-matrix
  `compare_bins.sh --gate` vs the P0-settled base, and a P2.1 `instr-diff`
  showing Ir drop concentrated in stack-traffic functions. Watch
  `call_return_shapes`, `fibonacci`, `closure_ops`, and the setter rows.
- Risk: low. The TBC semantics live entirely in `tbclist`; `lua.lua`/locals
  close paths in the official suite are the canary. If anything regresses,
  the field was not dead and the audit fact gets corrected in the model doc.
- Model doc follow-up: move "StackValue 24 B vs 16 B" from the
  representation-ceiling list to fixed; add the meta-lesson (audit the
  ceiling list — one entry was a dead-code deletion misfiled as future
  unsafe work).

Estimated effort: 0.5 session including the A/B. Sequence after P1 so the
verdict comes from the upgraded runner (and ideally after P2.1 so the recount
confirms the mechanism).

## 6. Phase P4 — zero-source-change build knobs

Each is an experiment with a recorded verdict in
`docs/MATCHING_C_PERFORMANCE.md`; none ships without its own decision packet.
All A/Bs use the P1 runner at enforced durations; PGO additionally gets a
P2.1 recount (PGO should not change Ir much — if wall improves with flat Ir,
that is the layout/CPI story confirmed).

**P4.1 PGO.** `harness/bench/build-pgo.sh`:
`-Cprofile-generate` build → training run (full bench matrix + a slice of the
official suite for realistic dispatch mixes) → `llvm-profdata merge` (rustup
toolchain path, verified present) → `-Cprofile-use` rebuild → A/B vs stock
release. Interpreters are the canonical PGO winner (typically 5–15%), and it
directly attacks the recurring "layout moved unrelated workloads" failure
mode by making hot-path layout deliberate. If the win is ≥5% median across
the matrix, file a follow-up packet to wire PGO into the release pipeline
(build-time cost and reproducibility discussion live there, plus
ledger-labeling so PGO'd ratios are never silently compared against
non-PGO history; intellectual-honesty note: also measure a PGO'd C reference
once, clearly labeled, while the stock `-O2` reference remains the official
target).

**P4.2 `lto = "fat"`.** One-line A/B vs current `thin` (codegen-units is
already 1). Record compile-time cost alongside the perf verdict.

**P4.3 `fast-alloc` (mimalloc).** Already wired (audit fact 10) — measure it:
A/B `--features fast-alloc` across the matrix, with special attention to
`binarytrees`/`gc_pressure`/RSS. Decision note must address fairness: C uses
system malloc; if fast-alloc ever becomes default, ledger rows must carry an
allocator label.

**P4.4 `panic = "abort"` for the CLI.** Add
`[profile.release-abort]` inheriting release with `panic = "abort"`; build
only `lua-cli` with it (`catch_unwind` is confined to `lua-rs-runtime`,
which the CLI does not depend on — audit fact 11). A/B. If it wins
meaningfully, the ship decision weighs CLI-only distribution complexity;
embedders keep unwind.

Estimated effort: 1 session for all four experiments; ship decisions are
separate packets.

## 7. Phase P5 — the safety-tax price list (diagnostic branch, never merges)

The data-driven trigger for any future tradeoff conversation: measure what
each safety mechanism actually costs, per workload, on a branch that is
explicitly diagnostic.

- Branch `diag/safety-tax` from the post-P3 baseline. In-branch only: raise
  the `lua-vm` unsafe budget in `harness/unsafe-budgets.toml` and mark the
  branch clearly in its README header as never-merge.
- Ablation A — bounds checks: feature `diag-unchecked-stack` swapping the
  stack-slot and code-fetch indexing to `get_unchecked` equivalents. If a
  small accessor seam is needed first, land that seam on main as a
  perf-neutral refactor (verified by P1 A/B + P2.1 recount) before branching,
  so the ablation diff stays mechanical.
- Ablation B — interior mutability: feature `diag-unchecked-borrow` replacing
  the `LuaTable` `RefCell`/`Cell` borrow paths with unchecked variants.
- Ablation C (stretch) — dispatch shape: only if a cheap experiment exists
  (e.g. a one-deep hot/cold opcode split already rejected once); otherwise
  rely on the PGO result + literature estimate and say so.
- Measure: full matrix wall (P1 runner) + Ir (P2.1) per ablation,
  individually and stacked. Deliverable: a committed table in
  `docs/MATCHING_C_PERFORMANCE.md` — per workload, the % wall and % Ir
  attributable to bounds checks, borrows, and (estimated) dispatch shape.
- Interpretation rule, stated up front: the price list is for *measurement*.
  A workload whose remaining gap equals its measured safety taxes is at the
  safe-Rust floor; further effort there is tradeoff territory and stops by
  policy. A workload whose gap exceeds its taxes still has recoverable
  structure.

Estimated effort: 1–2 sessions. Sequence after P2.1 exists (the recount is
what makes ablation numbers trustworthy) and after P3/P4 so the baseline is
final.

## 8. Phase P6 — workload-matrix expansion

Everything currently measured is a metamethod-*absent* fast path; real
programs live on the paths below. All new workloads: deterministic output,
checksum-asserted, parity-verified against the reference, manifest entries
with class + iteration counts.

| workload | class | exercises |
|---|---|---|
| `method_calls.lua` | vm-parity | `OP_SELF`, `obj:method()` dispatch |
| `metatable_index_chain.lua` | vm-parity | `__index` table chains, 2-level inheritance reads/writes |
| `pcall_error.lua` | vm-parity | pcall/error throw + recovery loop |
| `varargs_spread.lua` | vm-parity | `...` pack/spread, `select('#', ...)` |
| `coroutine_pingpong.lua` | vm-parity | resume/yield switch cost |
| `string_format_mixed.lua` | stdlib | `string.format`, number→string |
| `concat_chain.lua` | vm-parity | `OP_CONCAT` multi-operand chains |
| `sort_seeded.lua` | stdlib | `table.sort` with deterministic comparator |
| `json_roundtrip.lua` | macro | self-contained pure-Lua encode/decode — a real-program opcode mix |
| `startup_empty.lua` | startup | process + runtime init only (P1.5) |

Acceptance: each row produces stable ratios at enforced durations and a
bytecode-parity pass (P2.3). Expect new poles (the metamethod-present and
pcall rows have never been measured); they enter the re-ranked model with
P2.2 budgets attached.

Estimated effort: 1 session.

## 9. Phase P7 — docs and process hardening

**P7.1 `PERFORMANCE_MODEL.md` corrections.** Fold in: audit facts 1/2/6/7/10
(dead field → fixed by P3; sub-100 ms caveat + re-baselined table; row
classes; trap's dual role in C and what that means for the Rust port's trap
machinery — the structural question is which re-reads exist only for C's
pointer revalidation; dhat/mimalloc exist — the "no allocation-stack profiler
on macOS" gap statement is wrong as written). Add the new tools (P2.x) to the
tooling map with one-line "question → tool" rows. Re-rank packet candidates
from the re-baseline + budgets.

**P7.2 `PERFORMANCE_PRINCIPLES.md` corrections.** Rewrite §5
("Reference-counted values") to the real cost model: `GcRef`/`LuaValue` are
`Copy`; copies are 16-byte memcpys; the tax is `RefCell`/`Cell` borrows on
shared GC structures and the helper-boundary `Result` round-trips. Remove the
v0.0.31-era scorecard from PRINCIPLES entirely — scorecards live in exactly
one place (MODEL §Current Position, regenerated from promoted evidence; a
later nicety is `history.py` emitting that fragment so it cannot rot by
hand). PRINCIPLES keeps rules and patterns only.

**P7.3 Rejected-experiments registry.** `harness/rejected-experiments.jsonl`,
one JSON object per line:
`{id, date, mechanism, files, symbols, workloads_regressed, evidence,
reason, retry_conditions}`. Rules added to MODEL: a packet brief that
re-touches a registered mechanism must cite the id and state what is
different this time; A/B verdicts append to the registry on rejection.
Backfill from both perf docs (P0.3). Optional later hardening: a PreToolUse
warning hook when an edit touches registered symbols — not in this push.

**P7.4 Stop-hook containment.** The auto-commit at `d0dc949` is the failure
case: an unvalidated perf experiment + scratch artifacts swept onto `main`.
Changes to `harness/stop-hook.sh`:

- Never stage `harness/impl/**` outputs or `harness/bench/{results,profiles}`
  (today only gitignore protects some of these; the hook excludes them
  explicitly).
- When `crates/**` is dirty and a marker file `harness/.perf-experiment`
  exists, refuse the auto-commit and leave the tree dirty with a message
  naming the marker. `compare_bins.sh` creates the marker when invoked from a
  dirty tree; an `improved`/`regressed` verdict with `--gate` removes it.
  This keeps the safety-net property (work is never silently lost — the hook
  still blocks rather than discards) while stopping unvalidated perf diffs
  from landing bundled with unrelated files.
- Restate the existing rule in MODEL: perf agents work in their own worktree.

**P7.5 (optional, parallelizable) Typed bench runners.** The existing model
doc packet 5: `runners.toml` entries for bench/profile/instr runners with
resource locks (`benchmark-host`, `profiler`, `instr-container`). Unblocked
by nothing; do it when an agent is otherwise idle.

Estimated effort: 1 session (P7.1–P7.4), P7.5 separate.

## 10. Execution order and parallelization

```
P0 (settle tree)                                  ← blocker, first
 └─ P1 (runner integrity)                         ← unblocks all verdicts
     ├─ P2.1+P2.2 (instr counts + budgets)        ┐ parallel worktrees OK:
     ├─ P2.3 (bytecode parity)                    │ static/deterministic work
     ├─ P6  (new workloads, authoring)            ┘ does not touch the bench host
     ├─ P3  (tbc_delta)                           ← needs bench host
     └─ P1.6 (re-baseline)                        ← after P3 lands
         ├─ P4 (PGO / fat-LTO / fast-alloc / panic-abort)
         ├─ P2.4–P2.6 (C profile, counters, alloc parity)
         └─ P5 (safety-tax price list)            ← last; needs P2.1 + final baseline
P7 (docs/process)                                 ← P7.2/P7.3/P7.4 anytime after P0;
                                                    P7.1 after P1.6
```

Hard host rule (extends the existing one): **one measurement process of any
kind at a time** — wall-clock runs, sample profiles, and callgrind container
runs all distort each other. Static work (P2.3, P6 authoring, P7 docs)
parallelizes freely in separate worktrees per the repo's worktree rules.

Total estimate: 8–11 focused sessions. The payoff curve is front-loaded: P0+P1
alone fix the false-pole and unattributable-verdict problems; P2.1+P2.2 change
what a packet *is* (predicted instruction delta, verified by recount).

## 11. Risks

- **Callgrind container friction** (toolchain in image, arm64 valgrind
  quirks): timebox P2.1 bring-up to one session; fallback is running the
  identical script on a Linux CI runner first (callgrind is noise-immune
  there) and doing local bring-up second.
- **xctrace export instability**: P2.5 is best-effort and timeboxed; the push
  succeeds without it (P2.1 covers the Ir factor; CPI is then inferred).
- **Re-baseline reshuffles priorities**: expected, not a risk — but it means
  no deep setter-opcode packet should start before P1.6.
- **PGO reproducibility**: training-set drift changes layout run to run; the
  build script pins the training command list in-repo.
- **Stop-hook changes weaken the safety net**: P7.4 keeps "block, don't
  discard" semantics; nothing is auto-reverted.
- **`tbc_delta` turns out not-dead**: the gate (official suite + canaries)
  catches it; the audit fact gets corrected rather than worked around.

## 12. Acceptance checklist (the push is done when)

- [x] `d0dc949`'s two mechanisms each have a recorded verdict; rejected parts reverted; registry entry written.
- [x] Runners interleave, stamp provenance, enforce ≥0.5 s samples, and emit machine verdicts; `compare.sh` has auto `--repeat-each`.
- [x] Re-baselined Current Position table committed from promoted evidence; row classes labeled; CI wall-clock demoted to smoke.
- [x] `instr-count.sh` determinism proven (<0.5% spread); per-iteration budget table (C vs rs) committed for every matrix workload.
- [x] Bytecode-parity gate green over all workloads (or allowlisted with justifications) and wired as a make target.
- [x] `StackValue` is 16 bytes; matrix + Ir evidence linked in the commit.
- [ ] PGO, fat-LTO, fast-alloc, panic-abort: PGO verdict recorded AND SHIPPED; the other three remain unmeasured.
- [ ] Safety-tax price list committed; per-workload "at floor / not at floor" classification written into the model doc.
- [x] New workload rows live with budgets; metamethod-present and pcall paths measured for the first time.
- [x] Model/principles docs corrected (GcRef Copy, dhat/mimalloc, trap dual-role, single scorecard); rejected-experiments registry active; stop-hook contains scratch + unvalidated perf diffs.

## Wave 2 (2026-06-10, unattended execution plan)

State at wave start: PGO overall 1.41x (artifact 20260610T001xZ, variant=pgo),
no row above 2.08x. All waves below follow the packet gate; instruction
recounts arbitrate every boundary verdict. Commit deliberately and often;
hold `harness/.perf-experiment` whenever the tree is dirty at a turn
boundary.

### W2.1 Setter-family instruction diet (tallest recoverable band)
`table_setfield_same` 2.02 / `global_settabup_same` 1.88 / `table_seti_same`
1.66 PGO. Budget says C does `t[1]=i` in 61 Ir/iter, we use 157. Method:
**differential workloads** — `wave2/loop_only.lua` (bare numeric for) vs the
setter rows; Ir(setter) − Ir(loop) isolates per-iteration opcode cost for
BOTH binaries with cachegrind totals only (no callgrind needed). Then audit
the rs-side path (`OP_SETI`/`OP_SETFIELD` arm → `raw_set_*` →
`try_update_*`) against the C macro chain and remove nameable work:
RefCell borrow shape, helper-boundary `Result` round trips, double
metatable/dummy checks. Target: setter Ir/iter under ~110. Gate: recount +
gated A/B + 44/44.

### W2.2 Call/return + method dispatch diet
`method_calls` 2.05 / `call_return_shapes` 1.70 PGO; fibonacci Ir ratio
2.32. Same differential method with a `call_only.lua` micro-workload.
Audit `precall`/`poscall`/`OP_SELF` arms. The journal's rejected naive
attempts (registry: return-reentry-cleanup, callinfo-cached-closure,
naive-exact-lua-call-fastpath) define forbidden shapes — cite before
retrying anything adjacent.

### W2.3 RSS / allocation parity (P2.6)
binarytrees RSS 4.2x, closure_ops 5.2x. Run the dhat-heap build per GC
workload; build the counting-allocator patched C reference in /tmp;
produce the decomposition table (alloc count vs object size vs retention).
Fix the single top finding if bounded (e.g. Vec->Box<[T]> in LuaTable).

### W2.4 Bytecode-parity gate (P2.3, plumbing)
`lua-code` listing example + `bytecode-parity.sh` diffing against
`luac -l -l` for every workload; `make bytecode-parity`. Static, cheap,
prevents codegen-divergence regressions.

### W2.5 Safety-tax ablation (P5) — only if W2.1/W2.2 land first
Branch `diag/safety-tax`, never merges: price bounds checks and RefCell
borrows per workload via feature-gated unchecked variants. Produces the
empirical floor for the 1.4-1.7 band.

### W2.6 Wrap
Fresh stock+PGO matrices, model doc Current Position refresh, dashboard,
board + memory updates, clean tree.
