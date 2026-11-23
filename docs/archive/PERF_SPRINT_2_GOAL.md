# GOAL: perf sprint 2 — tooling, UpVal diet, setters, GC alloc design, safety-tax ablation

This is the standing goal document for the sprint. The supervising agent reads
this END TO END before doing anything, then `CLAUDE.md`, `../CLAUDE.md`, and
`docs/ISSUE_BURNDOWN_SPEC.md` — the burndown spec is the proven template for
this work: per-packet specs, evidence-ticked checklists, subagent execution
with the supervisor doing design sign-off and verification. Create
`docs/PERF_SPRINT_2_SPEC.md` in that same style before writing any code, claim
a row on `../AGENT_COORDINATION_BOARD.md`, and keep one branch per worktree
(never two agents in one tree). Each landed item is its own PR with gates in
the body.

## Non-negotiable measurement protocol (history: T2-C/T2-D were dropped by it)

- Freeze a baseline release binary from origin/main BEFORE each packet's edits.
- Wall = interleaved A/B only (alternate base/candidate, 8-loop aggregates for
  sub-0.5s workloads, ≥4 rounds, judge min-ratio). Noise floors on this rig:
  ±1% measurement, ±2-3% code-layout. Any sub-5% wall claim needs a second axis.
- The arbiter is deterministic instruction count (`harness/bench/instr-count.sh`,
  callgrind in the container): instruction-removal packets must show Ir DOWN on
  their primary target or the code is REVERTED and the packet recorded
  RESOLVED-NEGATIVE in the spec. Latency packets (locks/allocator) are Ir-flat
  by nature — for those, wall + a removed-work argument suffices.
- Profiles are evidence about ONE commit: never spec a packet from a profile
  whose build sha ≠ current main. Re-profile first.
- A neutral result is reported as neutral. Honest negatives are deliverables.
- Per-packet gates minimum: `cargo test -p lua-vm`, `cargo test -p lua-rs-runtime
  --test multiversion_oracle`, `harness/canaries/gc/run_canaries.sh` (all PASS),
  relevant official tests via `harness/run_official_test.sh`, and
  `cargo check -p lua-vm --target wasm32-unknown-unknown` (a 64-bit-only
  assertion broke the wasm CI gate on 2026-06-11; never again). PR gate:
  `cargo test --workspace` + `harness/run_official_all.sh`.
- Repo style is hook-enforced: no inline `//` comments (doc-comments only),
  PORT STATUS trailers preserved, no String/&str for Lua data, `unsafe` budgeted
  in `harness/unsafe-budgets.toml`.

## T0 — Tooling sprint (DO FIRST; everything after gets cheaper)

1. `harness/bench/instr-count.sh`: add a branch-simulation mode
   (`--branch-sim` → valgrind `--branch-sim=yes`, report Bc/Bcm per workload).
   This is the missing CPI arbiter — deterministic branch-miss counts settle
   "was it the branch?" questions that wall time on macOS/arm64 cannot
   (a call-free control once moved 12% on pure code layout).
2. Fix the bash-3.2 `set -u` crash in that script (`"${EXTRA_MOUNT[@]}"` on an
   empty array — use the `${arr[@]+"${arr[@]}"}` idiom).
3. Make `harness/bench/profile-hotspots.sh` agent-safe: it stalls under the
   agent harness (detached `sleep` killer subshell). Time-box this; if not
   fixable cleanly, document the manual `/usr/bin/sample` fallback in the
   script header.
4. Script the heap-profile diff: a wrapper that runs the dhat/counting-allocator
   build (see `harness/bench/table-bytes.sh` for prior art) for a named workload
   at two commits and emits alloc-count / bytes-per-block deltas. T1 and T3 use
   it.
5. Write `docs/MEASUREMENT_PROTOCOL.md` codifying the protocol block above,
   plus: PGO is worth ~0.1-0.2 of wall ratio; trust `_long` workloads; never
   bench under load. Link it from `CLAUDE.md`'s ladder section.
6. Extract a reusable perf-packet prompt template (frozen baseline, interleave,
   Ir/branch-sim arbiter, revert validation, drop-if-neutral, honest-negative
   reporting) into `../port-harness/templates/c-to-rust/perf-packet.md`, with a
   pointer back to `docs/ISSUE_BURNDOWN_SPEC.md` as the worked example.

## T1 — #113 ladder rung 1: UpVal mirror removal (RSS lever)

`crates/lua-types/src/upval.rs:22-39`: `UpVal` carries BOTH Cell-tagged
fast-path fields AND a `RefCell<UpValState>` mirror kept for legacy `slot()`
consumers → GcBox<UpVal> is 104 B vs C's ~40 B, and closure_ops RSS is 4.18x.
Migrate every `slot()` consumer (grep crates/ — coro_lib's open-upvalue
snapshot reads it; there will be others) to the Cell fields, delete the mirror.
Success bar: `value_layout` shows GcBox<UpVal> ≤ 64 B; closure_ops RSS ratio
drops measurably (compare.sh --runs 5); zero behavior change. Danger zone:
cross-thread upvalue flush (`cross_thread_upvals`) and upvalue close — run the
canaries AND `LUA_RS_GC_QUARANTINE=1` on coroutine.lua + closure-heavy official
tests (locals.lua, closure.lua). Post a progress comment on issue #113 after
merge with the new size table.

## T2 — Setter-family packet (worst wall rows)

table_setfield_same 2.17, table_seti_same 1.97, global_settabup_same 1.90,
table_settable_string_key 1.75 (stock matrix 20260611T164856Z-b0e68f8). PR #137
took the first pass; profile FRESH on current main before speccing (rule
above). Likely suspects: write-barrier checks, metatable-presence re-checks per
write, hash main-position recompute on the `same-key` shapes. Success bar: Ir
down on ≥2 of the 4 target rows, no control regression (fibonacci, mandelbrot,
table_field_index), canaries + quarantine green (barrier changes are
GC-correctness-sensitive — treat like rooting work).

## T3 — GC/allocator architecture (design memo first, ONE bounded step after)

The alloc-shaped tail (gc_pressure 1.98, concat_chain 2.02, binarytrees 1.77)
and RSS converge on `docs/GC_ALLOC_PLAN.md` causes 2/3: three mallocs per
non-empty table, no pooling, plain Box per GcBox. Deliverable A: a design memo
(supervisor-grade, not delegated) quantified with T0's dhat tooling, comparing:
size-class free lists for GcBoxes; Vec→Box<[T]> for table array/node parts
(PERFORMANCE_MODEL.md candidate 9); sweep-time buffer pooling; pacer cadence
tuning. Note the prior SmallVec rejection (GC_ALLOC_PLAN "inline-storage
lesson") — don't relitigate it. Deliverable B: implement ONLY the memo's
top-ranked bounded step, full battery (canaries, quarantine+stress, ASAN run
per `harness/asan-stress.sh` if sweep/rooting-adjacent). Anything deeper waits
for explicit human sign-off — say so in the PR.

## T4 — Safety-tax ablation (measurement ONLY, never ships)

Build a cargo-feature-gated variant (`perf-ablation-unchecked-stack`) replacing
the bounds-checked stack accessors (`get_at`/`set_at`/`set_top`, see
docs/MATCHING_C_PERFORMANCE.md "accessor" rows) with unchecked access, purely
to measure how much of the residual ~1.47x is safety tax. Rules: the feature
NEVER merges to default builds — keep the diff on branch
`ablation/unchecked-stack` unmerged; temporarily raise the lua-vm unsafe budget
ON THAT BRANCH ONLY; what merges to main is a docs PR writing the measured
matrix delta into `docs/PERFORMANCE_MODEL.md` ("safety tax = X% of wall on row
Y") plus the branch name for reproduction. This number decides when wall-time
work stops being worth it — that's its entire purpose.

## End state

All five ticked in `docs/PERF_SPRINT_2_SPEC.md` with evidence paths (including
any honest negatives), CHANGELOG Unreleased entries, a closing full
`compare.sh --runs 5` matrix committed to the bench ledger, coordination-board
row closed. If any packet's gates go red and resist two fix attempts, or a
quarantine/ASAN signature appears anywhere, halt that packet and report rather
than improvise.
