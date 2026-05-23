# {{PORT_NAME}} Perf-Fixer Packet

You are running as the `test-fixer` role, **but this packet is performance
work, not correctness work.** Read the `PACKET_NOTE` below carefully — it
tells you which hot frame to attack and which profile artifact provides
the evidence.

## Required reading before you start

1. `docs/PERFORMANCE_PRINCIPLES.md` — the playbook
   (evidence-driven, gate-driven, dashboard-driven).
2. `docs/MATCHING_C_PERFORMANCE.md` — the lessons digest. The target is
   parity with reference Lua 5.4.7. The redis-rs-port sibling hit
   parity through this same methodology.
3. The profile artifact named in `PACKET_NOTE` below, under
   `harness/bench/profiles/`. The top of every such file is a human
   summary of top frames by % of wall-clock; the full call graph follows.
4. The C-Lua reference for the function you're attacking, under
   `reference/lua-c/`.

## Failing perf-parity fixtures (workloads with wall_ratio > 1.5x)

Latest oracle evidence blob:

`{{LATEST_ORACLE_BLOB}}`

{{FAILING_FIXTURES_SUMMARY}}

Packet-specific hypothesis and target:

{{PACKET_NOTE}}

Declared target files (changes outside this set will fail the chassis
out-of-scope-changes gate):

{{PACKET_TARGETS}}

Profile-evidence and reference paths (read in order):

{{SOURCE_RANGES}}

Affected capabilities and owners:

{{AFFECTED_CAPABILITIES}}

## Required process

Follow the gate exactly:

1. **Read the profile artifact** named in `PACKET_NOTE`. Note which
   frame is hot and what % of wall it consumed. Form a hypothesis
   BEFORE editing.
2. **Read the C-Lua reference** for the same function. Note whether
   C-Lua has a fast path for the case your hypothesis suggests we're
   missing. It usually does — that's the whole pattern.
3. **Make ONE focused change.** Smaller is better. Patterns that work:
     - "Use the fast path that already exists"
     - "Cache the precondition, don't disable the feature"
     - "Match upstream's structure"
     - "Don't clone where C uses pointers"
4. **Rebuild**: `cargo build --release -p lua-cli -q`. Must be clean.
5. **44/44**: `./harness/run_official_all.sh`. Must stay green.
6. **GC canaries** (if you touched GC code): `./harness/canaries/gc/run_canaries.sh`.
7. **Bench**: `./harness/bench/compare.sh --runs 5`. Confirm the
   targeted workload moved (ratio drops, or at least no regression)
   and others stay within ±10%.

The Stop hook will auto-commit on green correctness checks.

## Hard rules

- **44/44 stays green.** If you can't keep it passing, revert.
- **No benchmark-only fast paths.** If the workload path is unrealistic, fix is fraudulent.
- **No skipped semantic correctness.** Skip work that's provably
  unnecessary; never skip work that matters.
- **No new `unsafe`** outside `lua-gc` / `lua-coro`.
- **No `String` for Lua data.** Use `&[u8]` / `Vec<u8>` / `LuaString`.
- **Touch only the declared target files.**
- **No inline `//` comments**, doc strings only.
- **No fallback patterns** like `x || y || z`.

## Output contract

- `cargo build --release -p lua-cli -q` clean
- 44/44 passes; GC canaries pass if touched
- compare.sh shows the targeted workload improved (or unchanged)
- Touched `.rs` files have a refreshed `PORT STATUS` trailer

If the bench did not actually improve, the commit message should explain why
(e.g. "LLVM was already optimizing this path; no measurable change but the
code is cleaner"). Hygiene is fine; lying about perf is not.

Project root: `{{PROJECT_ROOT}}`
Prompt hash: `{{PROMPT_HASH}}`
Evidence path: `{{EVIDENCE_PATH}}`
