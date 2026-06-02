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
├── scaling-check.py     <- complexity gate: flag superlinear (O(n^2)) behavior
├── workloads/           <- self-contained .lua microbenchmarks (timed vs C)
│   ├── binarytrees.lua  <- GC pressure (CLBG-style)
│   ├── closure_ops.lua  <- closure allocation + upvalue access
│   ├── fibonacci.lua    <- recursive call dispatch + small-int math
│   ├── gc_pressure.lua  <- allocation/collection throughput under churn
│   ├── mandelbrot.lua   <- float math + nested loops
│   ├── string_ops.lua   <- concat/find/gsub/byte ops
│   ├── table_hash_pressure.lua <- hash-part insertion (#38 regression guard)
│   └── table_ops.lua    <- table insert/remove/iterate, array + hash
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
```

For workloads that are too short to survive the sampler's startup delay, pass
an eval payload and use the first argument only as the artifact label:

```bash
PROFILE_LUA_EVAL='for i=1,100 do dofile("harness/bench/workloads/gc_pressure.lua") end' \
  bash harness/bench/profile-hotspots.sh gc_pressure_x100 6
```

A calltree/xctrace runner can be added when the hotspot summary is not enough
to explain a packet.

## Reproducibility rules

- Always run with the matching `target/release/lua-rs` build (NOT `target/debug`)
- Always run from a clean working tree (no in-flight edits to runtime crates)
- Do not run other CPU-heavy work in parallel
- Record the hardware fingerprint from the TSV header when sharing numbers

## Current follow-ups

1. The latest table lesson is PR #120's typed write-barrier fast path; see
   `docs/MATCHING_C_PERFORMANCE.md`.
2. `profile-hotspots.sh` is wired for `/usr/bin/sample` summaries and supports
   `PROFILE_LUA_EVAL` for scaled short-workload probes. Add a calltree/xctrace
   runner only when a packet needs deeper attribution.
3. `compare.sh` appends ledger rows directly. Typed bench runner entries in
   `harness/runners.toml` are still useful future cleanup, but not required
   for evidence-backed perf work.
4. Backfill remains future work for answering "when did this regress?" across
   older commits.
5. Keep `results/` and `profiles/` generated artifacts ignored unless a run is
   deliberately promoted into committed evidence.
