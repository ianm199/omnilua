# harness — oracles, benchmarks, enforcement

The harness is how a change is *verified*, not just built. Read the root guide
(`../CLAUDE.md`) first.

## Two oracle families (know which question each answers)

1. **Official-suite parity (single-version, 5.4)** — runs the unmodified upstream
   Lua 5.4 suite against `lua-rs` and diffs stdout + exit code.
   - `harness/run_official_all.sh` — the full suite (the headline gate; reports
     the live pass count — don't hardcode it in docs).
   - `harness/run_official_test.sh reference/lua-c/testes/<t>.lua` — one file.
   - `harness/run_one_test.sh <t>` — smoke.
   - `harness/parity_check.sh` — diff one program vs the 5.4 reference binary.
   - Reference binary: `reference/lua-5.4.7/src/lua` (build:
     `make macosx -C reference/lua-5.4.7`; binary gitignored).
   - Scratch outputs land in `harness/impl/official/*.out` — **never commit
     them**; `run_all.tsv` is the scoring artifact.

2. **Multi-version snippet oracle (5.1–5.5)** — diffs a Lua snippet against a
   chosen version's reference binary.
   - `specs/oracle/diff_one.sh <ver> "<snippet>"` — one snippet vs that version.
   - `specs/oracle/check.sh <ver>` — that version's battery, `N passed / M failed`.
   - Reference binaries: `/tmp/lua-refs/bin/lua5.{1,2,3,4,5}.x`, **pinned in
     `specs/oracle/CONTRACT.md`**. `/tmp` is ephemeral — if cleared, rebuild from
     the CONTRACT recipe. `reference/lua-5.3.6/` is also vendored in-repo.

The in-process equivalent (no binary) is
`crates/lua-rs-runtime/tests/multiversion_oracle.rs` — the tier-2 inner loop.

## GC canaries

`harness/canaries/gc/run_canaries.sh` — fast, deterministic in-memory GC testers
(incremental + generational). Run on any GC/metamethod/table change *before* the
slow `gc.lua` oracle. This is the "build a custom subsystem tester" pattern.
Honors `LUA_RS_BIN`.

## GC rooting battery (`harness/asan-stress.sh`, issue #140)

Hunts use-after-sweep rooting bugs with three layered instruments:

- `LUA_RS_GC_QUARANTINE=1` (debug build) — sweep parks dead boxes instead of
  freeing; any later dereference is a deterministic Rust panic with a
  backtrace. Cadence-identical to a normal run, milliseconds to trip. **The
  inner loop for this bug class** — reach for it before ASAN.
- `+ LUA_RS_GC_STRESS=1` — collect at every checkpoint. Behavioral failures
  are expected under stress (cadence-sensitive asserts); only panic
  signatures count as findings.
- `--asan` — nightly AddressSanitizer build (commit-keyed cache), the
  truth-teller for reads that bypass the poisoned headers.

`harness/asan-stress.sh` runs quick by default (canaries + repro set);
`--full` adds the whole official suite under quarantine; `--quarantine-only`
is the CI gate (must stay clean). Stress-config findings are expected until
#140 bug B (frame-range stale slots) is fixed. Evidence lands in
`harness/evidence/rooting-battery/<stamp>/`.

**Release-profile suite** (`make conformance-release`) is part of `make test`:
optimized code re-rolls GC cadence, which is how #140 bug A segfaulted every
release run while the debug suite stayed green.

## Benchmarks (`harness/bench/`)

Measures the **lua-rs / reference-C ratio** (wall + RSS), never absolute numbers.
```bash
bash harness/bench/compare.sh                       # all workloads, best-of-5
bash harness/bench/compare.sh --runs 3 --workloads fibonacci,binarytrees
python3 harness/bench/history.py                     # rebuild harness/bench/history/index.html (tracked)
make scaling                                         # flag O(n^2) regressions
```
Trust the `_long` workloads; sub-100ms ones are startup-dominated. To pin a perf
regression: `git bisect run` a script that builds release and thresholds the
best-of-N wall time on one workload (how #113 was found). Results go to
`harness/bench/results/` (gitignored) + `harness/evidence/ledger.jsonl`.

## Enforcement (`.claude/hooks/`, wired in `.claude/settings.json`)

Mechanical guardrails that fail a tool call or Stop event. The Stop orchestrator
is `harness/stop-hook.sh` (re-runs unsafe-budget / forbidden-import /
type-vocabulary / trailer + a smoke gate + auto-commit). PreToolUse gates:
`verify-gate.sh`, `pretooluse-type-vocab.sh`, `pretooluse-no-gcref-new.sh`. The
unsafe ceilings are in `harness/unsafe-budgets.toml`.

## Source pin
`harness/source.toml` records the pinned upstream (Lua 5.4.7) and its build/test
commands. `make setup` recreates the `reference/lua-c/testes` symlink if missing.
