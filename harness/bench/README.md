# lua-rs benchmark harness

Side-by-side performance characterization of `lua-rs` against pinned upstream
Lua 5.4.7. Modeled on the bench shape used in `redis-rs-port` and
`nginx-rs-port`, adapted for an interpreter (no servers, no protocol — just
"run this `.lua` workload through both binaries, measure").

The unit of measurement is **the ratio** (lua-rs / reference), not absolute
throughput. A standalone "lua-rs runs at X ops/s" number tells you nothing
about the port; the ratio against the reference C interpreter under the same
workload is the only fair comparison.

## What's wired

```
harness/bench/
├── README.md            <- this file
├── compare.sh           <- main ledgered bench: run all workloads vs reference
├── gc-profile.sh        <- end-of-run collector counters
├── opcode-profile.sh    <- feature-gated opcode execution counters
├── profile-inventory.sh <- repo + host profiler/tool availability
├── profile-hotspots.sh  <- macOS sample wrapper + VM execute attribution
├── scaling-check.py     <- complexity gate: flag superlinear (O(n^2)) behavior
├── value-layout.sh      <- Rust-vs-C value/frame/object layout probe
├── vm-execute-attribution.py <- source-region parser for sample output
├── workloads/           <- self-contained .lua microbenchmarks (timed vs C)
│   ├── binarytrees.lua  <- GC pressure (CLBG-style)
│   ├── closure_ops.lua  <- closure allocation + upvalue access
│   ├── fibonacci.lua    <- recursive call dispatch + small-int math
│   ├── gc_pressure.lua  <- allocation/collection throughput under churn
│   ├── mandelbrot.lua   <- float math + nested loops
│   ├── mandelbrot_long.lua <- longer float math profile target
│   ├── string_ops.lua   <- concat/find/gsub/byte ops
│   ├── string_ops_long.lua <- longer byte-string profile target
│   ├── table_hash_pressure.lua <- hash-part insertion (#38 regression guard)
│   ├── table_ops.lua    <- table insert/remove/iterate, array + hash
│   └── table_ops_long.lua <- longer table insert/remove/iterate target
└── scaling/             <- size-parameterized workloads for scaling-check.py
    ├── array_insert.lua
    ├── gc_churn.lua
    ├── hash_insert.lua
    └── pairs_iter.lua
```

`compare.sh` measures the lua-rs/C ratio at one size. `scaling-check.py`
(`make scaling`) is complementary: it runs each `scaling/` workload at 1x..8x
of a base size and fits the complexity exponent, failing if an operation that
should be linear goes superlinear. That gate catches O(n^2) regressions a
single-size ratio would miss (it is what would have caught the #38 table bug).

`profile-inventory.sh` is the cheap first command for a new performance
session. It prints which repo probes exist and which host profilers are
available on the machine, including `sample`, `xctrace`, `leaks`, DTrace,
`inferno-flamegraph`, `samply`, and Linux `perf`.

Generated artifacts land under `results/` and `profiles/` (gitignored).
The static dashboard at `history/index.html` IS tracked so it can be viewed
directly from GitHub via [raw.githack.com][dash] or by opening the file
locally in a browser.

[dash]: https://raw.githack.com/ianm199/lua-rs/main/harness/bench/history/index.html

Every workload is **deterministic** — same output on every run, same on
both interpreters. The compare runner asserts checksum equality (any drift
fails the bench, doubling as a correctness oracle).

## How to run

```bash
# build both binaries first
make macosx -C reference/lua-5.4.7   # produces reference/lua-5.4.7/src/lua
cargo build --release -p lua-cli     # produces target/release/lua-rs

# all workloads, best-of-5
bash harness/bench/compare.sh

# subset, fewer runs (smoke)
bash harness/bench/compare.sh --runs 2 --workloads fibonacci,mandelbrot
```

Output:
- `harness/bench/results/<UTC>-<sha>-compare.tsv` (header + per-workload rows)
- `harness/bench/results/<UTC>-<sha>-compare.json` (machine-readable)
- Appends 2 rows per workload (`wall_ratio`, `rss_ratio`) to
  `harness/evidence/ledger.jsonl` so the dashboard can plot trends

To rebuild the dashboard after a bench run:

```bash
python3 harness/bench/history.py        # writes harness/bench/history/index.html
python3 harness/bench/history.py --open # also opens it in your browser
```

## How to read the numbers

`wall_ratio` is the headline. It is best-of-N wall-clock for lua-rs divided
by best-of-N wall-clock for reference. **Lower is better.**

Best-of-N (not mean) is the standard interpreter-benchmark convention. It
filters out scheduling jitter without smearing real performance differences.

`rss_ratio` is max-RSS lua-rs / max-RSS reference. Memory overhead at peak.

Hardware + commit fingerprint is in the TSV header. **Do not merge runs
from different machines** — apples to oranges. For the current scorecard and
running optimization journal, read `docs/PERFORMANCE_PRINCIPLES.md` and
`docs/MATCHING_C_PERFORMANCE.md`.

## Probe vs ledgered bench split (when we add probes)

`compare.sh` is a **ledgered** bench: every run produces evidence that
should be commitable history. Numbers move with optimization work.

Probes are different — they answer narrow questions during exploration
("does throughput improve with N? does max-RSS scale with payload? where
are the allocation hot stacks?") and write to `profiles/` (gitignored).
**Probes never write ledger rows.** Treat their output as telemetry, not
evidence. This is the redis-rs-port convention; we follow it here.

`profile-hotspots.sh` is the macOS wall-clock sampler integration
(`/usr/bin/sample`). It normally samples a named file under
`harness/bench/workloads/`:

```bash
bash harness/bench/profile-hotspots.sh string_ops_long 6
PROFILE_REPEAT=30 bash harness/bench/profile-hotspots.sh closure_ops 8
```

For workloads that are too short to survive the sampler's startup delay,
prefer `PROFILE_REPEAT=N`; the runner labels the artifact as
`<workload>_x<N>` and executes the same workload file in a loop. If you need a
custom probe shape, pass an eval payload and use the first argument only as the
artifact label:

```bash
PROFILE_LUA_EVAL='for i=1,100 do dofile("harness/bench/workloads/gc_pressure.lua") end' \
  bash harness/bench/profile-hotspots.sh gc_pressure_x100 6
```

A calltree/xctrace runner can be added when the hotspot summary is not enough
to explain a packet.

When the raw sample contains `lua_vm::vm::execute`, the same runner also writes
`vm-execute.txt` next to `summary.txt`. That report is produced by
`vm-execute-attribution.py`, which parses the raw call graph and buckets
`execute` source-line samples into frame setup, dispatch fetch, opcode arms,
return re-entry, and unknown-inlined regions. The headline table uses
self-samples, so an outer `OP_CALL` frame is not charged for time spent in the
nested callee's active VM frame.

This is still sampling telemetry, not exact per-op timing. It is useful for
distinguishing "all time vanished into `vm::execute`" from concrete buckets
such as dispatch, `OP_CALL`, `OP_GETUPVAL`, and `OP_SETUPVAL`. Use
`opcode-profile.sh` when you need execution counts; use `vm-execute.txt` when
you need approximate time attribution inside the interpreter loop.

`opcode-profile.sh` is a feature-gated VM opcode counter for cases where stack
sampling collapses into `vm::execute`:

```bash
bash harness/bench/opcode-profile.sh fibonacci
PROFILE_REPEAT=10 bash harness/bench/opcode-profile.sh closure_ops
```

It builds `lua-rs` with `--features opcode-profile`, writes
`profiles/opcode-profile/<UTC>-<sha>-<label>/opcodes.tsv`, and overwrites
`target/release/lua-rs` with the instrumented binary. Rebuild a normal release
binary before running `compare.sh`.

`gc-profile.sh` runs the normal release binary and writes end-of-run collector
counters:

```bash
bash harness/bench/gc-profile.sh gc_pressure
PROFILE_REPEAT=10 bash harness/bench/gc-profile.sh binarytrees
```

It writes `profiles/gc-profile/<UTC>-<sha>-<label>/gc.tsv`. The report covers
collection counts, heap cohorts, latest mark/sweep counters, grayagain count,
and intern-table size. It is useful when `/usr/bin/sample` says a GC phase is
hot but cannot explain how many objects the phase is visiting or freeing.

`value-layout.sh` is a representation probe, not a benchmark:

```bash
bash harness/bench/value-layout.sh
```

It compiles a tiny Rust example plus a temporary C probe against
`reference/lua-5.4.7/src` and prints `impl/type/size_bytes/align_bytes` rows
for value, stack, frame, table, closure, userdata, proto, and upvalue
structures. Use it before making claims about safe-Rust value layout or
unsafe representation ceilings.

## Reproducibility rules

- Always run with the matching `target/release/lua-rs` build (NOT `target/debug`)
- Always run from a clean working tree (no in-flight edits to runtime crates)
- Do not run other CPU-heavy work in parallel
- Record the hardware fingerprint from the TSV header when sharing numbers

## Current follow-ups

1. The latest string-key lesson is the two-operand concat fast path after the
   post-v0.0.27 profiling wave; see `docs/MATCHING_C_PERFORMANCE.md`.
2. `profile-hotspots.sh` is wired for `/usr/bin/sample` summaries and supports
   `PROFILE_REPEAT=N` for scaled short-workload probes. `PROFILE_LUA_EVAL`
   remains available for custom probe shapes. When `vm::execute` dominates,
   inspect the adjacent `vm-execute.txt` before adding deeper profiler tooling.
3. `opcode-profile.sh` covers per-op counts when stack samples flatten into
   `vm::execute`; it does not provide per-op timing. Pair it with
   `vm-execute.txt` when opcode frequency and sampled time diverge.
4. `gc-profile.sh` covers end-of-run collector counters. It does not provide
   allocation stack attribution or cumulative per-phase timing.
5. `compare.sh` appends ledger rows directly. Typed bench runner entries in
   `harness/runners.toml` are still useful future cleanup, but not required
   for evidence-backed perf work.
6. `profile-inventory.sh` and `value-layout.sh` are telemetry probes. They do
   not write ledger rows and should be cited as design evidence, not speed
   claims.
7. Backfill remains future work for answering "when did this regress?" across
   older commits.
8. Keep `results/` and `profiles/` generated artifacts ignored unless a run is
   deliberately promoted into committed evidence.
