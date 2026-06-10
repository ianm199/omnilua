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

**2026-06-10 PM** — full stock + PGO matrices at `c1dfdc1` (artifacts
`20260610T143508Z-c1dfdc1-compare.tsv` stock,
`20260610T144112Z-c1dfdc1-compare.tsv` `variant=pgo`, both ledgered).
These INCLUDE the whole 06-09/10 packet wave: C-shaped stringtable,
coroutine pools, FORLOOP register window, full codegen parity
(unary folds, float-immediate compares, RK stores, NOT peephole), the
GC mark-buffer pool, and fat LTO (now the stock release profile).
Overall: **stock 1.54x, PGO 1.43x**:

| workload | stock | PGO | | workload | stock | PGO |
|---|---:|---:|---|---|---:|---:|
| method_calls | 1.80 | 2.14 | | varargs_spread | 1.68 | 1.63 |
| coroutine_pingpong | 2.02 | 2.12 | | call_return_shapes | 1.64 | 1.61 |
| global_settabup_same | 2.00 | 2.10 | | json_roundtrip | 1.84 | 1.50 |
| table_seti_same | 1.87 | 2.10 | | fibonacci | 1.64 | 1.47 |
| binarytrees | 2.17 | 2.05 | | string_format_mixed | 1.62 | 1.45 |
| table_setfield_same | 2.08 | 1.98 | | compare_immediates | 1.70 | 1.41 |
| concat_chain | 1.96 | 1.82 | | mandelbrot / _long | 1.47 | 1.41 |
| table_settable_string_key | 1.63 | 1.80 | | metatable_index_chain | 1.43 | 1.38 |
| gc_pressure | 1.98 | 1.78 | | loop_variants | 1.70 | 1.37 |
| bitwise_mixed | 1.85 | 1.73 | | string_ops | 1.46 | 1.26 |
| closure_ops | 1.69 | 1.69 | | string_ops_long | 1.28 | 1.11 |
| numeric_mixed | 1.83 | 1.65 | | sort_seeded | 1.52 | 1.02 |
| pcall_error | 1.77 | 1.63 | | table_hash_pressure | 0.93 | 0.82 |
| table_ops / _long | 0.43 | 0.42 | | | | |

Reading rules for this table:

- **Stock improved 1.65x -> 1.54x in one day** from the packet wave plus
  fat LTO; per-row examples vs the prior snapshot: method_calls
  2.35 -> 1.80, table_setfield 2.40 -> 2.08, mandelbrot 1.69 -> 1.47,
  call_return_shapes 2.11 -> 1.64. (Cross-snapshot stock drift caveat
  still applies to small deltas; these are large and packet-backed.)
- **PGO's margin narrowed and is now MIXED per-row**: overall still -7%
  vs stock (1.54 -> 1.43), but it REGRESSES eight rows vs stock
  (method_calls 1.80 -> 2.14, table_seti 1.87 -> 2.10,
  coroutine_pingpong, global_settabup, table_settable_string_key...).
  Last night PGO beat stock on every row; the difference is fat LTO in
  the base (stock captured much of PGO's layout win) plus a training
  set that predates the codegen-parity packets. Follow-up: retrain the
  pinned training set and re-evaluate per-row; if the setter-row
  regressions persist, consider per-row variant selection or accept the
  overall win.
- **Stock cross-snapshot comparisons remain unreliable for small
  deltas** (layout drift, proven by recounts). Judge packets by gated
  interleaved A/Bs and recounts; judge releases by full-matrix runs
  like this one.
- RSS is DECOMPOSED (W2.3): allocation counts at parity with C; objects
  ~3x bigger. See "RSS decomposition" under Current Findings; the table
  representation diet (candidate 9) is the lever.

### Prior snapshot — re-baselined 2026-06-09 (P1.6)

**Re-baselined 2026-06-09** (P1.6): Apple M3 Max, commit `87ef21f`, the first
scorecard with enforced >=0.5 s samples, interleaved pairs, and repeat
calibration (artifact `20260609T203851Z-87ef21f-compare.tsv`, ledgered).
Overall wall ratio **1.51x**. Sorted by median pair ratio:

P6 expansion rows (artifact `20260609T205627Z-933d82e-compare.tsv`, same
host/method) are merged below and marked NEW — the previously unmeasured
paths hid the two tallest poles on the board:

| workload | wall ratio | median | note |
|---|---:|---:|---|
| concat_chain | 2.54 | 2.54 | NEW tallest pole: short multi-operand `..` chains |
| coroutine_pingpong | 2.53 | 2.53 | NEW: resume/yield switch cost |
| table_setfield_same | 2.35 | 2.37 | existing short-string SETFIELD |
| global_settabup_same | 2.33 | 2.33 | _ENV SETTABUP write path |
| method_calls | 2.06 | 2.27 | NEW: OP_SELF `obj:method()` dispatch |
| binarytrees | 2.05 | 2.15 | GC family; RSS 4.5x |
| gc_pressure | 2.04 | 2.02 | GC family |
| json_roundtrip | 2.02 | 2.00 | NEW: pure-Lua macro bench — the "real program" number |
| table_seti_same | 1.95 | 1.97 | integer SETI existing slot |
| string_format_mixed | 1.90 | 1.89 | NEW |
| pcall_error | 1.90 | 1.90 | NEW: throw/unwind; RSS 2.3x |
| call_return_shapes | 1.86 | 1.90 | frame setup / return re-entry |
| closure_ops | 1.85 | 1.91 | upvalues; RSS 5.1x |
| table_settable_string_key | 1.83 | 1.84 | |
| varargs_spread | 1.77 | 1.74 | NEW |
| numeric_mixed | 1.77 | 1.71 | |
| bitwise_mixed | 1.71 | 1.70 | |
| string_ops | 1.69 | 1.69 | old "1.00x" row was startup quantization |
| sort_seeded | 1.68 | 1.72 | NEW |
| loop_variants | 1.65 | 1.65 | |
| fibonacci | 1.58 | 1.62 | |
| mandelbrot / _long | 1.53 | 1.53 | |
| metatable_index_chain | 1.52 | 1.50 | NEW: `__index` chain reads healthier than feared |
| compare_immediates | 1.48 | 1.49 | improved by 2026-06-09 packets |
| string_ops_long | 1.47 | 1.46 | |
| table_field_index | 1.19 | 1.14 | |
| table_hash_pressure | 1.07 | 1.02 | at parity |
| table_ops_long | 0.43 | 0.43 | stdlib-divergence row, not VM parity |
| table_ops | — | — | livelock FIXED 2026-06-09 (missing key barrier, ltable.c:717 parity); row re-baselines next compare.sh run |
| startup_empty | — | — | absolute startup constant only; excluded from ratios/ledger |

All ratios above are STOCK builds. The measured PGO lever (see Build-config
experiments) sits on top: estimated overall ~1.51x -> ~1.34x once shipped.
Note the PGO training set predates the NEW rows — the ship packet must
retrain on the expanded matrix.

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

**Decomposition (P2.1 rig, 2026-06-09, the most important measured fact in
this document):** `wall_ratio = Ir_ratio x CPI_ratio`, and the first
per-iteration instruction budgets show the C gap is almost pure INSTRUCTION
COUNT — our cycles-per-instruction is *better* than C's:

| workload | C Ir/iter | rs Ir/iter | Ir ratio | wall ratio | CPI factor |
|---|---:|---:|---:|---:|---:|
| concat_chain | 2,965 | 7,350 (was 8,100 pre-stringtable) | 2.48 | ~1.98 est | |
| table_seti_same | 61 | 157 | 2.57 | 1.95 | 0.76 |
| fibonacci | (totals) | | 2.32 | 1.58 | 0.68 |
| string_format_mixed | 12,286 | 20,336 | 1.66 | 1.90 | 1.14 |

Consequences: (a) we are not memory/branch-bound on these rows — we simply
execute ~2.3-2.7x more instructions, which is the recoverable-work bucket,
not a safety tax; (b) Apple-class IPC hides a third of the bloat, weaker CI
cores hide less — that is the whole Linux-vs-M3 ratio mystery; (c) packets
should now be stated as "remove ~N instructions/iteration" and verified by
recount (`instr-count.sh`). C does the entire `t[1] = i` loop iteration in
61 instructions; our extra 96 have names. (`string_format_mixed` is the
counterexample: better Ir ratio, worse CPI — formatting is the one row that
is genuinely stall-bound.)

**Differential probe budgets (2026-06-10, `harness/bench/probes/`):**
subtracting `loop_only` from a workload isolates per-opcode cost. Findings:

| component | C Ir/iter | rs Ir/iter | ratio |
|---|---:|---:|---:|
| bare FORLOOP tick | 24 | 61 (75 pre-window) | 2.54 |
| SETI on top | +37 | +82 | 2.2 |
| SETFIELD on top | +53 | +118 | 2.2 |
| CALL+RETURN+ADD on top | +191 | +425 | 2.2 |

The opcode surplus is UNIFORM (~2.2x) across families — there is no single
rotten opcode; the cost is spread across dispatch preamble, bounds checks,
borrow flags, and helper boundaries. The dispatch tick itself was the worst
outlier (3.12x) until the FORLOOP register window (`661ee2a`); its remaining
~37-instruction surplus over C needs per-function attribution (callgrind on
a >8GB-VM box) to cut further.

Two budget-reading caveats, both learned the hard way: (1) **C-side budgets
on STRING-KEY rows carry ±10% per-run noise** — C's hash seed is
per-process (address/time), so its collision chains differ per run; our
seed is fixed. Only int-key/loop/call C budgets are exact. (2) Micro-edits
inside the giant dispatch match can shift codegen of UNRELATED arms by a
few Ir (the rejected `setter-arm-direct-operand-reads` experiment) —
always recount the probe, never assume.

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
| Is a small wall delta real work or layout luck? | `bash harness/bench/instr-count.sh --workloads w1,w2` | deterministic Ir + per-iteration budgets (`results/*-instr*.tsv`) |
| What does one opcode cost in isolation? | instr-count over `harness/bench/probes/` pairs (e.g. `loop_only` vs a setter row) | differential Ir/iter for both binaries |
| Did codegen diverge from luac? | `make bytecode-parity` | per-workload mnemonic diff vs `luac -l -l`; allowlist only shrinks |
| Is the PGO build still healthy? | `make perf-pgo` | conformance-gated, variant=pgo ledger rows |
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

### Landed packets, 2026-06-09/10 wave (each gated + recount-arbitrated)

| packet | commit | result |
|---|---|---|
| GC key write barrier (CORRECTNESS) | `b5e65fb` | missing ltable.c:717 parity let the generational GC free live interned keys → livelock + silent-wrong-lookup risk; fixed, ~1-2% cost on insert rows (work C also pays) |
| tbc_delta deletion (P3) | `3ec9433` | stack slots 24 -> 16 B; 15/21 rows improved, call_return -9-11% |
| trap-guard adjudication (P0.1) | `2f236de` | narrowed hookmask guard kept (-2-5% dispatch rows); broad variant stays rejected |
| coroutine snapshot pools | `04cd144` | per-resume Vec alloc/free x2 removed; pingpong -12-16%; fibonacci blip PROVEN displacement (+296 of 82.3e9 Ir) |
| C-shaped stringtable | `534f5bb` | one-hash intern, zero-alloc hit, O(dead) GC removal; concat -22%, format -15%, hash-pressure -13%, string_ops -11% |
| FORLOOP register window | `661ee2a` | six bounds checks -> one; loop tick 75 -> 61 Ir, baked into every loop row |
| PGO shipped (P4.1) | `967b801` | release CI publishes conformance-gated, variant-labeled PGO ratios; median -11%, and it erases layout-displacement noise |
| bytecode-parity gate (P2.3) | `f20bdfb` | 31 rows EXACT opcode parity vs luac; 19 baselined divergences in 4 classes (see candidates) |

### RSS decomposition (W2.3, 2026-06-10): object size, not churn

Method: dhat-heap build (`--features dhat-heap`) vs a counting `l_alloc`
patched into a /tmp copy of the reference (never `reference/` itself).
Result table (total = lifetime allocation traffic):

| workload | allocs C / rs | bytes-per-block C / rs | peak C / rs |
|---|---|---|---|
| binarytrees | 6.31M / 6.32M (parity) | 58 / 185 (3.2x) | 15.3M / 43.8M (2.9x) |
| closure_ops | 200k / 301k (1.5x) | 50 / 166 (3.3x) | 10.1M / 31.4M (3.1x) |
| gc_pressure | 600k / 618k (parity) | 51 / 222 (4.4x) | 27K / 97K (3.6x) |

The "we allocate too often" hypothesis is dead: counts match C almost
exactly (binarytrees structurally two blocks per table on both sides).
The whole RSS gap is REPRESENTATION SIZE: a table object allocation
averages 176 B vs C's ~56 B `Table`, and the hash-part vector 99 B vs
C's right-sized node array. dhat per-callsite attribution
(`new_table_with_sizes` 555 MB, `set_node_vector` 314 MB of 1.17 GB
binarytrees traffic) plus two pure-churn offenders now fixed: the GC
mark phase allocated a fresh visited-set + gray-queue per cycle (396
buffers, 249 MB — pooled on the Heap, churn -23%), and
`sweep_young_range` temporaries (~15 MB, still open, same pooling
pattern). Remaining lever: the table representation diet (candidate 9).

### Table/global setters (HISTORICAL — pre-stringtable/window; kept for the method)

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

Ship status: SHIPPED 2026-06-10 (`967b801`) — release CI runs
`make perf-pgo` (conformance-gated, `variant=pgo` ledger rows; the
dashboard trend stays stock-only).

**mimalloc / `--features fast-alloc` (P4.3, 2026-06-10): measured, not
shipped.** A/B vs stock at 10 interleaved pairs: big wins on
allocation-heavy rows — `concat_chain` 0.77, `table_hash_pressure` 0.82,
`gc_pressure` 0.85, `string_ops_long` 0.85, `json_roundtrip` 0.94 — and
RSS drops of 21-43% everywhere it matters (`json` 0.57, `gc_pressure`
0.64, `string_ops_long` 0.59, `binarytrees` 0.78), which bites directly
into the W2.3 RSS-parity problem. Costs: `binarytrees` wall +4.8-5.1%
(10/10 pairs, real) and `fibonacci` +1-3% borderline. Verdict: a real
lever with one real trade; do NOT flip the default from this sample.
Next step if pursued: full matrix + PGO-on-mimalloc interplay, and check
whether the binarytrees regression survives PGO.

**fat LTO (P4.2, 2026-06-10): SHIPPED (`d6df7b4`).** A/B vs thin at 10
interleaved pairs, zero regressions: call_return_shapes -12%,
binarytrees / mandelbrot_long / table_seti_same / json_roundtrip /
string_ops_long -4-5%, fibonacci/gc_pressure flat; official suite 44/44
against the fat binary. No losing row, so unlike mimalloc it ships
immediately; the release profile is now `lto = "fat"` and PGO layers on
top of it.

**panic-abort CLI (P4.4, 2026-06-10): REJECTED** (registry
`panic-abort-cli-build`). pcall is Result-based and `catch_unwind` lives
only in lua-rs-runtime (not linked by the CLI), so semantics hold
(44/44) — but there is no unwind tax to reclaim on hot paths, and the
abort personality regressed `table_seti_same` +17% (0/10 pairs).

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
profile that shows the expected source bucket shrinking — and since the P2.1
rig exists, a recount showing the predicted Ir delta.

**Status 2026-06-10:** candidate 0 APPLIED; candidate 1's intern-churn half
is superseded by the stringtable (`534f5bb`) — its remaining substance is
the RefCell/helper-boundary diet, now quantified (SETI op = 82 Ir vs C 37).
Candidates 2-5 stand, with budgets attached. New candidates, evidence in
hand:

### 6. Codegen divergence packets (bytecode-parity allowlist, task 11)

Four classes from `make bytecode-parity`, hottest first: (a) C emits
LEI/LTI immediate compares where we emit LOADF+LE — mandelbrot's INNER
LOOP pays the extra op; (b) unfolded negative constants (LOADI+UNM for
`-1`, every descending for-step — cold); (c) extra constant
materialization (json_roundtrip carries 29 divergent ops — mine the diff);
(d) RETURN/RETURN0/RETURN1 selection differs from luac in closing returns
— **needs a correctness look** (the specializations differ in
vararg/close handling) before any perf work. Gate: allowlist counts drop,
parity stays green, conformance, targeted A/B.

### 7. Dispatch-preamble attribution

The loop tick still carries ~37 Ir over C with a near-minimal FORLOOP body.
opcode() decode is already a bounds-checked cast (vm.rs:50). The remaining
surplus needs per-function counts: callgrind on a machine whose docker VM
exceeds 8GB (it OOMs locally), or finer probes. Do NOT hand-guess here —
the rejected direct-operand-reads experiment shows micro-edits in the
dispatch match move unrelated arms.

### 8. RSS / allocation parity (spec W2.3) — DECOMPOSED 2026-06-10

Done: see "RSS decomposition" finding. The answer is object size, and the
follow-on packet is candidate 9. The mark-buffer pool landed; the
`sweep_young` scratch buffers (next_revisit Vec + IdentityHashSet,
~15 MB/run churn) are a small remaining pooling packet.

### 9-pre. DONE 2026-06-10: GcHeader diet (first slice of the diet)

Header 64 -> 40 bytes on every heap object — but from the COLD side
only: `type_name` (16 B, diagnostics-only) became a `Trace` trait
method; the three cold bool flags share one byte; pacer `size` is u32.
Color and age KEEP dedicated bytes: the first attempt packed them
C-`marked`-style and cost +4.2% Ir on gc_pressure (mask/RMW in the
mark/sweep loops; callgrind-localized; registry
`gcheader-pack-hot-fields`). `GcBox<LuaString>` 88 -> 64 (-27%),
`GcBox<LuaTable>` 168 -> 144; `table-bytes.sh` shows every shape -32 B.
Shipping-config (PGO) A/B: binarytrees/closure_ops/gc_pressure/
string_ops_long all IMPROVED with RSS -9..-15%; table_seti_same +1.8%
regressed-minor (tracked: ~+1-4 Ir/iter cross-crate inlining coupling);
fibonacci +3.1% wall on FLAT Ir (recounted twice) = data-placement
luck, waived per the displacement protocol.

### 10. Allocation-token side table (found via table-bytes residue)

Every `Heap::allocate` inserts (identity, token) into
`allocation_tokens: IdentityHashMap<usize>` — the ~50 B/object residue
the per-shape tool shows above the box size, plus a hashmap insert on
EVERY allocation (wall cost on alloc-heavy rows). It exists so weak
handles can validate targets across address reuse WITHOUT dereferencing
possibly-freed memory — moving the token into the header breaks the
freed-but-not-reused case (reading a freed header is UB), so any diet
here must rethink the weak-registry validation flow first. High value,
needs design.

### 9. Table representation diet (from the W2.3 decomposition)

A table allocation averages 176 B vs C's 56 B `Table` struct, and the
hash-part vector 99 B vs C's right-sized power-of-2 node array. Both
feed RSS ratios of 2.9-3.6x at alloc-count parity. Inventory the bytes:
GcBox header, RefCell/Cell flags, separate array/hash Vec headers
(ptr+cap+len x2), node layout vs C's 24 B `Node`. High-risk,
high-reward; any layout change must keep `value-layout.sh` and the GC
canaries green and re-run the full A/B matrix.

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
   moved. If the A/B shows a boundary verdict (any `regressed`/
   `regressed-minor` on a row the change does not execute), arbitrate by
   recount: `instr-count.sh` on that row before and after. Flat Ir + wall
   rise = layout displacement, waived with the numbers in the commit;
   rising Ir = real, the gate stands. Never argue a waiver without the
   recount — two 2026-06-09 precedents (fibonacci +4e-7% Ir at wall 1.030;
   the direct-reads rejection) define the protocol. Conversely, recount the
   target probe BEFORE paying for an A/B: a candidate whose Ir does not
   drop is dead on arrival.

   Marker protocol: hold `harness/.perf-experiment` (touch it yourself)
   whenever the tree is dirty across a turn/session boundary — the runners
   are ownership-aware and will not delete a marker they did not create.

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

