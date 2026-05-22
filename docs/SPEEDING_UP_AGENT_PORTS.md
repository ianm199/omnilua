# Speeding Up Agent-Driven Ports

Notes from inspecting the Phase A Lua port traces. The short version: the run is
slow mostly because each worker asks the model to generate a huge Rust file in
one shot. Local hooks, rustc, and filesystem work are not the primary bottleneck.

## What the Traces Show

Across the 19 traced translator runs:

- Total sequential wall time: about 5.0 hours.
- Total API time: about 4.8 hours.
- Median file time: about 13.7 minutes.
- P90 file time: about 30 minutes.
- Output throughput was usually around 60-75 Sonnet output tokens/sec.
- Large one-shot writes dominated: `vm.rs` was about 137k chars, `state.rs`
  84k, `llex` 81k, `table.rs` 76k, `api.rs` 67k.

That means most of the time is not in post-agent validation. It is model time:
planning, generating, streaming large `Write` calls, then sometimes spending more
turns on checks or repair.

## Current Speed Limits

Plain parallelism helps, but only until the largest single file becomes the
tail. With the current whole-file task shape, a greedy scheduling estimate from
the observed durations is:

| Workers | Approx. wall time |
|---:|---:|
| 2 | 2.5 hours |
| 4 | 78 minutes |
| 8 | 40 minutes |
| 12+ | 32 minutes |

The 12+ worker floor exists because the slowest file still takes about 31-32
minutes by itself. More workers cannot fix a giant single task.

## Main Problems

### 1. Whole-file units are too large

Files like `lapi.c`, `lparser.c`, `ltable.c`, `llex.c`, and `lvm.c` are too big
for one translator invocation. The result is slow, fragile, and expensive:

- big files hit max-output failures;
- big files hit max-budget failures after useful work was already done;
- one socket/API failure can waste a 20-30 minute run;
- the largest task determines the parallel wall-clock floor.

The fix is structural: split large files into sections or function ranges.

### 2. Output volume is inflated

The "C source as adjacent comments" trick is useful for review, but it is
expensive. It increases output tokens, file size, stream time, and chance of
output-limit failures.

For Phase A, prefer source references:

```rust
// C: lvm.c:1234-1288
```

instead of embedding large C snippets next to every translated block. Full
adjacent C comments can be reserved for hairy functions or later review builds.

### 3. Agents reread too much context

The traces show repeated source reads and repeated analysis reads. Some runs also
start with bad absolute paths such as `/Users/ianmclaughlin/ANALYSES/...` before
correcting to the repo path.

The fanout layer should prepare a precise work packet so the model starts with
exactly what it needs:

- source slice;
- direct header slices;
- relevant `ANALYSES` rows;
- target file path;
- target insertion marker;
- current local type/API surface, if needed.

Then the prompt can explicitly say not to read broad files unless blocked.

### 4. Budget and output caps need to match file size

Several traces show work completing or nearly completing but still ending with
`error_max_budget_usd`. A single flat cap is the wrong control.

Use size-based budgets:

| C file size | Suggested cap |
|---:|---:|
| under 300 LoC | $1.50 |
| 300-800 LoC | $3.00 |
| 800+ LoC | $5.00-$6.00 |

This does not make an individual large run faster, but it avoids wasting a long
run that fails near the finish line.

### 5. Prompt rules are not enforcement

The translator instructions already say to split big writes, but traces show
giant one-shot `Write` calls anyway. Important performance policy has to live in
the orchestrator, not only in the prompt.

Examples:

- refuse whole-file tasks over a LoC threshold;
- generate section tasks automatically;
- reject `Write` calls over a size threshold if the CLI/hook layer can see them;
- require target markers for section tasks;
- record section-level success independently.

## Recommended Long-Term Shape

### Section-level fanout

For any C file over about 500 LoC:

1. Build a symbol/function index.
2. Group functions into chunks of about 150-300 C LoC.
3. Create an empty Rust target file with stable section markers.
4. Run translator workers per section.
5. Have a stitcher pass normalize imports, trailers, and module-level types.
6. Run compiler-fixer after the full file is assembled.

This turns one 30-minute failure into several 3-8 minute tasks. It also makes
parallelism much more effective because the slowest file no longer dominates the
whole run.

### Two-pass translation

Use two explicit passes instead of asking one agent to design and emit
everything:

1. **Shape pass:** emits structs, enums, function signatures, markers, TODOs,
   and source line references. Small output, fast, easy to review.
2. **Body pass:** fills one function group at a time.

This reduces one-shot planning pressure and makes retries cheap.

### Smaller comments by default

Use concise source references everywhere, with full C snippets only when the
translation is non-obvious. A good default:

```rust
// C: ltable.c:486-530, luaH_get
```

Full C-as-comments should be opt-in for complex control flow, pointer tricks,
or places where Phase B reviewers need side-by-side context.

### Precomputed work packets

Add a `harness/packets/` step that writes one JSON or markdown packet per task.
The packet should include all high-value context and nothing else. The agent
should read the packet first and only reach for the repo if the packet is
insufficient.

This cuts repeated reads, bad paths, and prompt-cache churn.

### Fast failure before API work

Before launching a translator task:

- check target is not already real-ported;
- check output cap env is visible;
- check allowed tools match the commands the prompt asks for;
- check budget for the file size;
- check the source slice is below the chunk threshold.

This avoids the common expensive failures: output cap, budget cap, denied tools,
and giant over-threshold writes.

## Refinements Validated Mid-Phase-B

Three points worth pinning down, with live evidence from the Phase B
compiler-fixer dispatch.

### Shape-then-body needs a *frozen* API contract

Pure section fanout without coordination causes naming drift: six agents
translating sections of `lapi.c` will independently invent
`state.get_top()` vs `state.top()` vs `state.stack_top()` for the same
concept. A stitcher pass that only normalizes imports cannot catch this.

The shape pass must commit a binding public-API contract — signatures,
field names, error-constructor names — before any body agent starts.
Body agents work against a fixed interface they cannot rename. The
trailer/imports stitcher is for things that don't matter; the contract
is for things that do.

### Smaller chunks unlock model tiering

Whole-file translation needed Sonnet because the agent had to hold ~2000
lines of C plus cross-file context in its head while generating ~4000
lines of Rust. A 150-LoC section against a frozen shape contract is
much closer to grunt work — Haiku can do it.

Combined effect: 8x from parallelism × 3-5x from model tiering. Cost
falls more than wall time does, which matters once you're paying.

### Cut on syntactic gutters, not LoC

"150-300 C LoC per chunk" is the right magnitude, but cuts must land in
syntactic gaps — between functions, between a `#define` block and the
functions that use it, between an `LUAI_FUNC` and its tightly-coupled
static helpers. Naïve `split -l 250` would slice a function in half.

Use clangd or tree-sitter to find natural cut points. The pre-computed
ANALYSES step already has the symbol/range data — extend it with
"chunk plans" alongside the macros/types/error-sites TSVs.

### Phase B is already proving the section thesis

Live evidence from the parallel compiler-fixer dispatch:

| Crate     | Errors | Tool uses | Wall time | Tokens |
|-----------|-------:|----------:|----------:|-------:|
| lua-parse |      6 |        19 |       80s |    30k |
| lua-code  |     13 |        75 |    7 min  |    99k |
| lua-vm    |    731 |     (running) | 25-40 min | 300-600k |

lua-parse and lua-code were *naturally scoped* (one crate each, narrow
problem) and finished fast. lua-vm got all 731 errors in one agent
because we didn't pre-section it. The right v2 dispatch would have been
one agent per high-error file:

- `api.rs` (187 errors) → agent 1
- `debug.rs` (197 errors) → agent 2
- `do_.rs` (197 errors) → agent 3
- `tagmethods.rs` (63), `vm.rs` (29), `object.rs` (29), small files → agent 4

Four parallel 5-8 min runs instead of one serial 25-40 min run. Same
total token cost, much lower wall time. **Section fanout applies to
compiler-fixing, not just translation** — anywhere errors cluster by
file, scope each file to its own worker.

## Expected Speedup

Conservative expectation:

- 4-8x wall-clock from parallel workers.
- 2-3x from smaller outputs, fewer rereads, fewer failed giant writes.

Together, a 5-hour sequential whole-file Phase A pass should be able to become a
15-25 minute 8-worker run. With aggressive section fanout and no rate-limit
pressure, 8-15 minutes is plausible for a comparable pass.

The realistic target is not 100x. The generated Rust volume is real. But
10-20x wall-clock improvement over naive sequential whole-file translation is a
reasonable long-term target.

## Immediate Next Changes

In priority order. Item 1 is the cheap unblocker that prevents the most
expensive failure modes; it should ship before any chunking work.

1. **Preflight checks** in `fanout.sh` (afternoon's work, immediate ROI).
   Before launching any translator task, fail-fast on:
   - target already real-ported (already done)
   - `CLAUDE_CODE_MAX_OUTPUT_TOKENS` not set or below 64000
   - `--allowedTools` missing commands the translator prompt asks for
   - budget too low for the file's LoC
   - source slice over the chunk threshold (once chunking exists)

2. **Size-based budgets** (5 lines of bash). $1.50 / $3 / $5 by LoC bucket.
3. **Source line references by default** in translator prompts. Reserve
   adjacent C-as-comments for hairy functions; cuts output tokens 30-40%.
4. **Work packets** in `harness/packets/` — pre-built JSON/markdown per task
   with source slice + relevant ANALYSES rows + target marker. Stops the
   repeated source rereads we saw in traces.
5. **Chunking mode** for files >500 LoC. Cut on syntactic gutters (clangd
   or tree-sitter), not LoC. Builds on packets — each chunk is a packet.
6. **Shape-then-body two-pass** with frozen API contract from shape pass.
   This is the architectural upgrade that makes chunking coherent.
7. **Section-scoped compiler-fixer dispatch** in Phase B+. One agent per
   high-error file, in parallel — same pattern as translation.

