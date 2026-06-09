# lua-rs Performance Model and Agent Handoff

This is the current "start here" document for performance agents. It connects
the high-level performance principles, benchmark tools, current evidence, known
gaps, and packet candidates into one operating model.

Read this alongside:

- `docs/PERF_PUSH_SPEC.md` for the active 2026-06 work plan (phases P0–P7).
- `docs/PERFORMANCE_PRINCIPLES.md` for the rules and packet discipline.
- `docs/MATCHING_C_PERFORMANCE.md` for the longer research journal.
- `harness/bench/README.md` for exact benchmark/profiler usage.
- `harness/rejected-experiments.jsonl` before forming any packet that touches
  a previously tried mechanism.

## Current Position

**Re-baselined 2026-06-09** (P1.6): Apple M3 Max, commit `87ef21f`, the first
scorecard with enforced >=0.5 s samples, interleaved pairs, and repeat
calibration (artifact `20260609T203851Z-87ef21f-compare.tsv`, ledgered).
Overall wall ratio **1.51x**. Sorted by median pair ratio:

| workload | wall ratio | median | note |
|---|---:|---:|---|
| table_setfield_same | 2.35 | 2.37 | tallest pole: existing short-string SETFIELD |
| global_settabup_same | 2.33 | 2.33 | _ENV SETTABUP write path |
| binarytrees | 2.05 | 2.15 | GC family; RSS 4.5x |
| gc_pressure | 2.04 | 2.02 | GC family |
| table_seti_same | 1.95 | 1.97 | integer SETI existing slot |
| call_return_shapes | 1.86 | 1.90 | frame setup / return re-entry |
| closure_ops | 1.85 | 1.91 | upvalues; RSS 5.1x |
| table_settable_string_key | 1.83 | 1.84 | |
| numeric_mixed | 1.77 | 1.71 | |
| bitwise_mixed | 1.71 | 1.70 | |
| string_ops | 1.69 | 1.69 | old "1.00x" row was startup quantization |
| loop_variants | 1.65 | 1.65 | |
| fibonacci | 1.58 | 1.62 | |
| mandelbrot / _long | 1.53 | 1.53 | |
| compare_immediates | 1.48 | 1.49 | improved by 2026-06-09 packets |
| string_ops_long | 1.47 | 1.46 | |
| table_field_index | 1.19 | 1.14 | |
| table_hash_pressure | 1.07 | 1.02 | at parity |
| table_ops_long | 0.43 | 0.43 | stdlib-divergence row, not VM parity |
| table_ops | — | — | skipped: generational-GC LIVELOCK (tracked) |

RSS is now a first-class problem: `binarytrees` 4.5x and `closure_ops` 5.1x
memory ratios are unexplained pending the allocation-parity probe (spec P2.6).

The pre-re-baseline framing below is retained for history; its absolute
numbers came from 50-70 ms samples and ephemeral Linux CI hardware:

As of `v0.0.32` / commit `169b868` on the Linux release runner, the largest
reference-Lua gaps are not broad "table performance" or broad "Rust is slow"
claims. They are specific bytecode and representation shapes:

| Workload | Linux release ratio | Main bottleneck class |
|---|---:|---|
| `table_seti_same` | 3.40x | `SETI` existing integer writes + loop/dispatch |
| `table_setfield_same` | 3.33x | `SETFIELD` existing short-string writes |
| `global_settabup_same` | 3.14x | `_ENV`/global `SETTABUP`, upvalue + table write |
| `table_settable_string_key` | 2.86x | `SETTABLE` string-key writes |
| `closure_ops` | 2.76x | closure calls + upvalue traffic |
| `call_return_shapes` | 2.72x | call frame setup + return re-entry |
| `gc_pressure` | 2.67x | allocation/collector cadence |
| `binarytrees` | 2.61x | allocation + GC + traversal |
| `fibonacci` | 2.40x | call/return + upvalues + dispatch |

Local Apple M3 Max telemetry after the release dashboard commit
`1dee1e5` showed the same shape but different absolute ratios:

- artifact: `harness/bench/results/20260609T163417Z-1dee1e5-bin-ab.tsv`
- `global_settabup_same`: 2.667x
- `table_setfield_same`: 2.333x
- `table_seti_same`: 2.200x
- `table_settable_string_key`: 2.000x
- `call_return_shapes`: 2.045x
- `binarytrees`: 2.093x
- `closure_ops`: 1.941x
- `numeric_mixed`: 1.808x
- `table_ops_long`: 0.430x, faster than reference in that aggregate shape

Important reading: aggregate table workloads are not the problem anymore.
Specific existing-key setter opcodes are.

### Measurement caveats on the table above (2026-06-09 audit)

- **The Linux column comes from ephemeral `ubuntu-latest` runners**
  (`release.yml`); hardware varies per release, so cross-release Linux
  trends conflate instance drift with code drift. Treat Linux CI ratios as
  smoke. The pinned Apple M3 Max host is the wall-clock trend instrument.
- **The top setter rows were measured on 0.05–0.07 s samples** at
  centisecond timer resolution with process startup included
  (`20260609T163417Z` artifact: `table_seti_same` C wall 0.05 s,
  `gc_pressure` 0.02 s). The tall-pole *ranking* is provisional until the
  re-baseline at enforced ≥0.5 s samples (PERF_PUSH_SPEC P1.3/P1.6).
- **`table_ops_long` at 0.430x is not a VM-parity number.** We appear to be
  2.3x faster than C there because stdlib shift loops diverge from C's
  per-element API layering (pending confirmation). Label it a stdlib row;
  do not average it with VM rows or let it mask VM regressions.

## Mental Model

The remaining gap is mostly interpreter overhead, not a single algorithmic
failure. Use these buckets when classifying profiles:

- **Dispatch and trap checks.** `vm::execute` source-line attribution often
  shows `DISPATCH_FETCH`, `UNKNOWN_INLINED`, and hook/trap-adjacent lines. Treat
  broad dispatch edits as high risk because small code-layout changes can move
  unrelated workloads. Structural note: C's `trap` serves two masters — hooks
  *and* stack-reallocation pointer revalidation (`lvm.c:1139` "stack
  reallocation or hooks?", `ldo.c:193`). The Rust stack is index-based, so the
  realloc half does not apply here; some per-`CallInfo` trap reads exist only
  as C-pointer-machinery mimicry. Trap packets should ask "which re-reads
  serve a purpose we actually have" before guarding individual sites.
- **Call frame setup and return re-entry.** Recursive and closure-heavy
  workloads show time in `OP_CALL`, `FRAME_SETUP`, `OP_RETURN1`, and the ret
  label that restores the previous `CallInfo`.
- **Upvalue access.** `_ENV` globals and closure workloads pay `upvalue_get`
  costs. The current implementation already has a same-thread open-upvalue fast
  path; remaining cost is still visible in profiles.
- **Existing-key table writes.** The newest table setter workloads are
  measuring repeated writes to already-present keys. These should avoid generic
  insertion/rehash and metamethod work when the VM has already proved a plain
  table/no-metatable path.
- **Representation and borrow boundaries.** Look for `RefCell` borrows, helper
  calls that C-Lua expresses as macros, and type-erased helpers where the VM
  already knows the concrete type. Note `LuaValue` and `GcRef` are `Copy`
  (16-byte memcpys, no refcount) — copies are essentially free; the cost is
  borrow-flag traffic and miss-path value round-trips, not clones.
- **GC and allocation.** Use GC counters and allocation-oriented profiles before
  assuming a VM opcode is the bottleneck in allocation-heavy workloads.

## Tooling Map

Use the cheapest tool that answers the question. Do not run multiple
performance agents on the same host.

| Question | Tool | Output |
|---|---|---|
| Is correctness still valid? | `cargo test ...`, `make conformance`, `make conformance-55` | test/conformance output |
| How does current lua-rs compare to C-Lua? | `bash harness/bench/compare.sh` | `harness/bench/results/*-compare.{tsv,json}`, ledger rows |
| Did one local packet improve vs a saved lua-rs binary? | `bash harness/bench/compare_bins.sh --a /tmp/base --b target/release/lua-rs` | `harness/bench/results/*-bin-ab.{tsv,json}` |
| Is a short workload too noisy? | `compare_bins.sh --repeat-each N` | longer per-sample wall time |
| What host profilers are available? | `bash harness/bench/profile-inventory.sh` | inventory text |
| Where is wall time inside `vm::execute`? | `bash harness/bench/profile-hotspots.sh workload seconds` | `harness/bench/profiles/*/summary.txt`, `vm-execute.txt` |
| Which opcodes execute most often? | `bash harness/bench/opcode-profile.sh workload` | `harness/bench/profiles/opcode-profile/*/opcodes.tsv` |
| Is GC cadence the problem? | `bash harness/bench/gc-profile.sh workload` | `gc.tsv`, `gc-delta.tsv`, `gc-rates.tsv` |
| Is layout itself a limit? | `bash harness/bench/value-layout.sh` | value/frame/object size rows |
| Where do allocations come from? | `cargo build --release -p lua-cli --features dhat-heap`, run workload | dhat heap profile (counts/bytes/stacks) |
| Is the allocator a factor? | A/B a `--features fast-alloc` (mimalloc) build via `compare_bins.sh` | bin-ab artifacts |
| Did complexity regress? | `make scaling` / `harness/bench/scaling-check.py` | scaling report |
| What changed over commits? | `python3 harness/bench/history.py` | `harness/bench/history/index.html` |

Notes:

- `compare.sh` writes ledger rows and should be used for publishable
  reference-C history.
- `compare_bins.sh` is preferred for local packet validation because it does
  not mutate the ledger.
- Rebuild a normal release binary after `opcode-profile.sh`; that runner uses
  an instrumented build.
- When `vm-execute.txt` is missing useful source-line attribution, rebuild with:

```bash
CARGO_PROFILE_RELEASE_DEBUG=true \
RUSTFLAGS="-C force-frame-pointers=yes" \
  cargo build --release -p lua-cli
```

## Evidence Vocabulary

Agents should use these labels when reporting:

- **Measured fact:** a concrete number from a TSV/JSON/profile artifact.
- **Repeated signal:** the same ratio or profile shape across multiple runs.
- **Hypothesis:** an explanation tied to source anchors and profile artifacts.
- **Packet candidate:** a bounded change with correctness and benchmark gates.
- **Rejected experiment:** a tested idea that did not pass the gate.
- **Risk:** semantic, GC, hook, metatable, unsafe, or benchmark-integrity risk.

Do not present a hypothesis as a measured fact.

## Current Findings

### Table/global setters

Artifacts:

- `harness/bench/profiles/20260609T163726Z-1dee1e5-global_settabup_same_x100/vm-execute.txt`
- `harness/bench/profiles/20260609T163740Z-1dee1e5-table_setfield_same_x100/vm-execute.txt`
- `harness/bench/profiles/20260609T163757Z-1dee1e5-table_seti_same_x100/vm-execute.txt`

Measured shape:

- `global_settabup_same`: about 52% of samples in `OP_SETTABUP`, about 18% in
  `OP_FORLOOP`, about 16% in dispatch fetch. Top visible lines include
  `tbl.raw_set_short_str`, `state.upvalue_get`, barrier checks, and key/table
  checks.
- `table_setfield_same`: about half the samples are in `OP_SETFIELD`; the top
  line is again `tbl.raw_set_short_str`.
- `table_seti_same`: less short-string work; more direct integer/array update
  plus loop/dispatch overhead.

Interpretation:

- The issue is not "LuaTable is globally slow." It is repeated existing-key
  setter bytecode paying too much helper and representation overhead.
- `_ENV` writes add upvalue access on top of the table write cost.
- `SETI` needs its own packet; short-string hash optimizations do not explain
  the integer case.

### Calls, returns, upvalues

Artifacts:

- `harness/bench/profiles/20260609T164558Z-1dee1e5-fibonacci_x3/vm-execute.txt`
- `harness/bench/profiles/20260609T164607Z-1dee1e5-numeric_mixed_x20/vm-execute.txt`

Measured shape:

- `fibonacci`: `DISPATCH_FETCH`, `OP_CALL`, `FRAME_SETUP`, `OP_RETURN1`,
  `OP_GETUPVAL`, `OP_ADDI`, `OP_ADD`, `OP_LTI`, and return re-entry all show up.
- `numeric_mixed`: dispatch fetch, unknown inlined VM work, `FORLOOP`, `ADDI`,
  and `MULK` dominate.

Interpretation:

- Call/return and upvalue costs are still concrete, profile-visible targets.
- Numeric-loop work is sensitive to dispatch-loop layout. Treat broad branch
  edits skeptically and validate them against `numeric_mixed`.

### Build-config experiments (PERF_PUSH_SPEC P4)

**PGO (P4.1, 2026-06-09): the largest single lever measured to date.**
`harness/bench/build-pgo.sh` (pinned training set: full workload matrix +
calls/nextvar/strings/closure from the official suite). Full-matrix A/B vs
stock at 10 interleaved pairs (artifact
`20260609T204715Z-3f4088e-bin-ab.tsv`): 17/20 improved, median ~ -11%,
nearly all rows 10/10 pairs. `numeric_mixed` 0.768, `fibonacci` 0.786,
`loop_variants` 0.824, `gc_pressure` 0.874, `binarytrees` 0.879, setter
rows 0.89-0.91, `call_return_shapes` 0.907. Estimated vs-C overall moves
~1.51x -> ~1.34x.

Two model consequences:

1. **Layout displacement confirmed.** The P3 `numeric_mixed` +2-3% blip is
   not just erased but inverted to -23%. Sub-3% single-row verdicts on
   non-PGO binaries are layout luck; the gate tolerance policy is the right
   default until instruction counts arbitrate.
2. **One real trade:** `table_ops_long` +13.6% (0/10 pairs) — the profile
   deprioritizes its stdlib shift loops. That row is stdlib-divergence
   class where we are 2.3x FASTER than C, so it stays ~2.1x ahead even
   after the loss. Acceptable for shipping; revisit if the row class ever
   approaches parity from above.

Ship status: evidence supports wiring PGO into release builds (separate
packet: pipeline changes, ledger `variant` labeling so PGO ratios are never
silently compared against non-PGO history, reproducibility via the pinned
training set). Until that lands, ledgered compare.sh runs stay on stock
builds.

## Rejected Or Inconclusive Experiments

Machine-readable registry: **`harness/rejected-experiments.jsonl`** — one JSON
object per line with `{id, date, mechanism, files, symbols,
workloads_regressed, evidence, reason, retry_conditions}`. A packet brief that
re-touches a registered mechanism must cite its `id` and state what is
different this time. New rejections append to the registry.

These are useful because they define the edges of the search space:

- **Hash-node `Cell<LuaValue>` values.** This allowed existing short-string
  hash updates through a shared table borrow and improved some setters, but it
  regressed `table_field_index` read-side performance. Do not reintroduce it
  as a broad table representation change without stronger evidence.
- **Broad hook/trap guarding across branch/test opcodes.** This improved some
  table setter rows, but a repeated run regressed `numeric_mixed` by about 11%
  and `compare_immediates` by about 2%. Broad dispatch-shape edits are not
  safe by intuition alone. *Resolution 2026-06-09:* the **narrowed** variant
  (2 FORLOOP sites + `finish_order_imm_jump` only) was adjudicated under the
  P0.1 protocol and **kept** — it improved `compare_immediates` ~5%,
  `global_settabup_same` ~3-4%, `numeric_mixed` ~2%, `binarytrees` ~3% in two
  interleaved-pair rounds; the bundled `code: &[Instruction]` fetch change
  measured neutral on the opcodes it touches (its apparent
  `global_settabup_same` blip was layout displacement — that workload never
  executes the changed code). One round-1 `binarytrees` "regression" was
  traced to the stop hook building/smoking mid-bench; the P7.4 marker now
  prevents that class of contamination.
- **Arithmetic direct-K rewrites.** Earlier local arithmetic rewrites regressed
  numeric/fibonacci/bitwise neighbors and were reverted. Keep arithmetic work
  profile-led and per-opcode.

## Packet Candidates

Each candidate needs correctness first, then a targeted A/B, then at least one
profile that shows the expected source bucket shrinking.

### 0. Delete the dead `StackValue.tbc_delta` field — APPLIED 2026-06-09

Outcome: `StackValue` 24 -> 16 bytes (C slot parity). 15/21 workloads
improved — `call_return_shapes` -9-11%, `table_seti_same` -8.5%,
`global_settabup_same` -7-8%, `compare_immediates` -8%, matrix total -3.6%
(artifact `20260609T193931Z-2b482e7-bin-ab.tsv`). Correctness: 30/30 lib
tests, GC canaries, 44/44 official. Known cost, confirmed twice:
`numeric_mixed` +2-3.7% (layout displacement signature; recovery owned by
the PGO packet P4.1, arbitration by instruction counts P2.1). The
repeated-`dofile` `table_ops` livelock found during this gate is
pre-existing and tracked separately.

Original hypothesis (2026-06-09 audit): `StackValue.tbc_delta`
(`crates/lua-vm/src/state.rs:414-417`) is write-only — the only
non-constructor reference is a `= 0` write at `state.rs:2517`; the real
to-be-closed mechanism is `LuaState.tbclist: Vec<StackIdx>`
(`state.rs:2021`, `func.rs:293`). The field pads every stack slot to 24
bytes where C's is 16. Removing it cuts a third of the bytes on every stack
copy, grow, and scan, with zero semantic change. `StackValue` does not leak
outside `lua-vm`.

Source anchors:

- `crates/lua-vm/src/state.rs` `StackValue`, the 4 constructor sites, `:2517`

Gate:

```bash
cargo test -p lua-vm -p lua-types --lib
make conformance
./harness/canaries/gc/run_canaries.sh
bash harness/bench/compare_bins.sh --a /tmp/lua-rs-base --b target/release/lua-rs --gate
```

Watch `call_return_shapes`, `fibonacci`, `closure_ops`, and the setter rows.
Risk: low; if the official suite or canaries disagree, the field was not dead
and this entry gets corrected rather than worked around.

### 1. Existing short-string table setter fast path

Hypothesis: `SETFIELD`, `SETTABLE` string-key, and `_ENV` `SETTABUP` can avoid
some helper/boundary overhead on already-present short-string keys while
preserving barriers and metamethod semantics.

Source anchors:

- `crates/lua-vm/src/vm.rs` `OP_SETTABUP`, `OP_SETTABLE`, `OP_SETFIELD`
- `crates/lua-vm/src/state.rs` `LuaTableRefExt::raw_set_short_str`
- `crates/lua-types/src/table.rs` `try_update_short_str`

Gate:

```bash
cargo test -p lua-types -p lua-vm --lib
bash harness/bench/compare_bins.sh \
  --a /tmp/lua-rs-v0032-base \
  --b target/release/lua-rs \
  --runs 10 \
  --repeat-each 5 \
  --workloads global_settabup_same,table_setfield_same,table_settable_string_key,table_field_index
```

Risk: read-side table regressions, skipped GC barrier, skipped metamethod path.

### 2. `SETI` integer existing-key path

Hypothesis: `SETI` is not helped by short-string work; it needs a narrower
integer/array update audit and loop interaction check.

Source anchors:

- `crates/lua-vm/src/vm.rs` `OP_SETI`, `OP_FORLOOP`
- `crates/lua-types/src/table.rs` `try_update_int`, `try_raw_set_int_fast`

Gate:

```bash
bash harness/bench/compare_bins.sh \
  --a /tmp/lua-rs-v0032-base \
  --b target/release/lua-rs \
  --runs 10 \
  --repeat-each 5 \
  --workloads table_seti_same,numeric_mixed,loop_variants
```

Risk: array growth/accounting, nil delete semantics, code-layout regressions in
numeric loops.

### 3. Call/return frame re-entry

Hypothesis: `call_return_shapes` and `fibonacci` still pay avoidable frame
setup and return re-entry work after `RETURN0`/`RETURN1`.

Source anchors:

- `crates/lua-vm/src/vm.rs` `OP_CALL`, `OP_RETURN0`, `OP_RETURN1`, ret label
- `crates/lua-vm/src/do_.rs` `precall`, `poscall`, `prep_call_info`
- `crates/lua-vm/src/state.rs` `CallInfo` accessors

Gate:

```bash
bash harness/bench/compare_bins.sh \
  --a /tmp/lua-rs-v0032-base \
  --b target/release/lua-rs \
  --runs 7 \
  --workloads call_return_shapes,fibonacci,closure_ops
```

Risk: hooks, yields, tail calls, close/TBC return paths.

### 4. Upvalue traffic

Hypothesis: closure/global workloads still pay too much upvalue lookup work in
same-thread open-upvalue cases.

Source anchors:

- `crates/lua-vm/src/vm.rs` `OP_GETUPVAL`, `OP_SETUPVAL`, `OP_SETTABUP`
- `crates/lua-vm/src/state.rs` `upvalue_get`, `upvalue_set`
- `crates/lua-types/src/upvalue.rs`

Gate:

```bash
bash harness/bench/compare_bins.sh \
  --a /tmp/lua-rs-v0032-base \
  --b target/release/lua-rs \
  --runs 7 \
  --workloads closure_ops,fibonacci,global_settabup_same
```

Risk: coroutine-owned open upvalues, cross-thread mirrors, GC barriers on
closed upvalues.

### 5. Tooling packet: benchmark runner coherence

Hypothesis: performance work is harder to parallelize because `runners.toml`
only models official correctness runners; benchmark/profile runners are
documented shell commands but not typed runner entries.

Scope:

- Add typed runner entries for the bench matrix, direct binary A/B, hotspot
  profile, opcode profile, GC profile, scaling, and history rebuild.
- Add resource locks: `benchmark-host`, `bench-results`, `profiler`,
  `opcode-profile-build`.
- Keep generated `results/` and `profiles/` ignored unless explicitly promoted.

Gate:

```bash
python3 ../port-harness/loop/check-completion.py --project . --json
python3 ../port-harness/loop/parallel-plan.py --project . --selector manual --json
```

Risk: runner metadata drifting from the actual shell harness.

## Agent Workflow

For a new performance agent:

1. Recover state.

```bash
pwd
git status --short --branch
git log --oneline -8
ls -lt harness/bench/results | head
ls -lt harness/bench/profiles | head
```

2. Confirm there is no active benchmark runner.

```bash
pgrep -fl 'compare_bins|compare.sh|profile-hotspots|target/release/lua-rs'
```

3. Build a release binary and save a base before editing.

```bash
cargo build --release -p lua-cli
cp target/release/lua-rs /tmp/lua-rs-base
```

4. Choose one packet. Do not mix table representation, dispatch, call/return,
   and GC changes in one packet. Check `harness/rejected-experiments.jsonl`
   first; retrying a registered mechanism requires citing its id and what is
   different this time.

5. Run correctness first, then targeted A/B. Use `--repeat-each` for subsecond
   workloads.

6. If the A/B passes, run the matching profile and verify the predicted bucket
   moved.

7. Run final validation.

```bash
cargo fmt --check
git diff --check
cargo test -p lua-types -p lua-vm --lib
make conformance
```

8. Report measured facts, repeated signals, hypotheses, rejected experiments,
   risks, and exact artifact paths. Avoid conclusions based on one short run.

## Documentation Gaps To Keep Closing

- The current `harness/work-packets.jsonl` contains valuable older perf
  packets, but some are stale after recent landed work. New packet entries
  should reference current `v0.0.32` artifacts.
- `harness/runners.toml` does not yet model benchmark/profile runners.
- Allocation-stack profiling exists but is unscripted: `lua-cli` already has
  `--features dhat-heap` (dhat global allocator) and `--features fast-alloc`
  (mimalloc) wired (`crates/lua-cli/src/main.rs:1436-1447`). `alloc-profile.sh`
  and the C-side counting-allocator probe are specced in
  `PERF_PUSH_SPEC.md` P2.6.
- Backfill remains future work for "when did this regress?" questions across
  older commits.
- Generated profile and result artifacts are ignored, so durable reports must
  cite exact local paths or promote selected summaries into committed docs.

