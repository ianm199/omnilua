# Retrospective & Productization Notes

What we learned doing this AI-driven C→Rust port, organized for transfer to a
next project. Lua 5.4 specifics are *examples*; the principles generalize.

## TL;DR

- The agent is rarely the limiting factor; **the harness around it is**.
- Validation that the agent can fix should live **inside** its loop, not after.
- Harness metrics need to be **trustworthy** before they're actionable. Two
  bugs in our harness made a 79%-success Phase A look like a 14%-success Phase A.
- Cost economics are remarkable — ~$30 to translate ~28k C LoC into ~12k Rust LoC.
- Pre-computed cross-file **analyses** (macros, types, error sites) are the
  single highest-leverage upfront investment.
- Parallel fanout demands care: shared-state hooks and shared temp files are
  race-prone; per-worker scoping is required.
- For hard runtime bugs, use the **Honest Repro Ladder** in
  `docs/DEBUGGING_STRATEGIES.md`: exact semantic mini-repro first, then compare
  the first divergence against the source implementation's state machine.

## 1. What the harness looks like (4 layers)

```
┌─ Layer 1: SPEC (static, prompt-cached) ─────────────────────────┐
│   PORTING.md — translation rules; banned patterns; type maps    │
│   translator.md, compiler-fixer.md, test-fixer.md, verifier.md  │
└─────────────────────────────────────────────────────────────────┘
┌─ Layer 2: ANALYSES (pre-computed cross-file lookups) ───────────┐
│   macros.tsv     — every C macro → Rust equivalent              │
│   types.tsv      — every C struct → Rust struct, field-by-field │
│   error_sites.tsv — every C error throw → Rust Err(...)         │
│   file_deps.txt  — C file → target crate + path                 │
└─────────────────────────────────────────────────────────────────┘
┌─ Layer 3: AGENT LOOP (per-file claude -p invocations) ──────────┐
│   Translator → in-loop syntax check → trailer → stop           │
│   (Compiler-fixer / Test-fixer / Verifier come in later phases) │
└─────────────────────────────────────────────────────────────────┘
┌─ Layer 4: POST-AGENT VALIDATION (hooks + oracle scripts) ───────┐
│   unsafe-budget, forbidden-import, trailer-required             │
│   rustc backstop (defense in depth)                             │
│   pilot.jsonl aggregate                                         │
└─────────────────────────────────────────────────────────────────┘
```

Each layer is independent and replaceable. The model only sees Layer 1 directly;
Layer 2 is read via tools; Layers 3–4 are orchestration the agent doesn't know
about.

## 2. The eight key lessons

### 2.1 Harness metrics lie until you've debugged them

We nearly executed a full retry pass on 12 "failed" Phase A files. The real
failure count was 3. Two harness bugs systematically misclassified successful
work:

- **Filter regex blindness.** Our "expected name-resolution errors" filter
  caught `cannot find type X` but not `could not find X` (E0433 phrasing) or
  `type annotations needed` (E0282). 5 files marked syntax-failed; actually clean.
- **Parallel hook race.** Hooks scanned the entire `crates/` tree on every Stop
  event. Under `--workers 4`, worker B's hook saw worker A's in-flight (no-trailer-yet)
  file and reported the failure against worker B's own success. 3 files
  marked hooks-failed; actually fine.

**Mitigation:** make the underlying compiler/oracle the source of truth.
Aggregate summaries can be wrong; `rustc --emit=metadata` is not. Build the
TUI to surface the disagreement (raw `total_errors` vs filtered `residual`)
so misclassification is obvious.

### 2.2 Pre-computed cross-file analyses are the highest-leverage upfront work

The ~950 lines of TSVs we generated before Phase A started paid off enormously
during translation. Agents looked up cross-file decisions instead of inferring
them per file. The 5-file pilot taking $1.09/file and Phase A averaging $1.88/file
was directly enabled by these tables.

**Generalization:** for any port, the upfront analysis step deserves
first-class tooling — auto-generated from source parsing (clangd, tree-sitter)
where possible, human-tightened, agent-consumable as lookups.

### 2.3 Validation inside the loop > validation after

The single biggest discipline upgrade. The Translator's `rustc` self-check turned
"agent declares done blindly" into "agent iterates until clean." Three files
(`ltm`, `lobject`, `ldo`) had shipped broken Rust under budget cap because the
syntax check was post-hoc; now the agent runs it itself.

**Rule of thumb:** anything cheap enough to run per-turn should be a tool the
agent can call. Anything slow or global is post-hoc backstop only.

### 2.4 The phase split is non-negotiable

Phase A (translate, may not compile) → Phase B (compile per-crate) → Phase C+
(test suite + idiom refinement). Don't merge. Our successful 11 files all have
name-resolution errors right now — and that's *correct*. Forcing compilation
during translation would require inventing types ahead of design decisions.

Make the constraint structural: Translator can't run `cargo check` on the whole
crate (allowed-tools enforces this).

### 2.5 Subagent role split with bounded tools

Four roles, each with different model and tool grants:

| Role | Model | Tools | Used in |
|---|---|---|---|
| Translator | Sonnet 4.6 | Read, Write, Edit, Glob, Grep, rustc | Phase A inner loop |
| Compiler-fixer | Sonnet 4.6 | + cargo check | Phase B |
| Test-fixer | Sonnet 4.6 + Opus advisor | + cargo test, oracle scripts | Phase C+ |
| Verifier | Haiku 4.5 | **no Write/Edit** | end of each phase |

The Verifier-with-no-write-tools pattern is structural anti-sycophancy. It
*physically cannot* mark a phase passing without evidence. This is the same
shape as Anthropic's `cwc-long-running-agents` reference repo.

### 2.6 Visibility is not a luxury

The 5-hour dark period during the first sequential pilot was the lowest moment
of the project. Three layers of visibility went in:

- `--output-format stream-json` per worker (live events as text)
- `harness/monitor/status.py` (one-shot snapshot)
- `harness/monitor/monitor.py` (curses TUI with mock + live backends)

The Mock backend was critical — let us develop the UI without a live run.
**Generalization:** any monitoring UI for an agent system needs a mock-data
mode so it can be iterated on while the harness itself is broken.

### 2.7 Parallelism is a multiplier, not free

`--workers 4` cut wall-clock from 47 min to ~50 min while doing 3× the work.
Great. But it also exposed:

- **Hook race conditions** — fixed via `CLAUDE_TARGET_RS_FILE` env var
  scoping the hook to one worker's file.
- **Shared temp files** — `/tmp/x.rmeta` had to become `mktemp -t` per worker.
- **Cache-window misses** — sequential pacing was *just* outside the 5-min
  prompt-cache TTL; parallel calls shared the cache better.
- **Output interleaving** — stream-json events from 4 workers became unreadable;
  per-worker transcript files saved us.

**Generalization:** parallelism in agent fanout needs designed isolation
(worktrees, per-worker temp dirs, per-worker hook scope). Bolting on workers
late exposes these.

### 2.8 Failure modes are predictable

Across all 14 Phase A attempts, the failures clustered into 4 types:

- **Budget cap hit on large files** — needs higher `--max-budget-usd` for
  files >1500 LoC, or smarter sub-budgets per turn.
- **Broken syntax under budget cap** — agent declared done. Fixed by in-loop
  rustc.
- **Hooks lying** — see 2.1.
- **Borrow-checker conflicts the agent didn't reshape** — PORTING.md §4.3 has
  the pattern (capture scalar into local); the agent didn't apply it on llex.rs.

These are all designed-against, not surprised-by, in a v2.

## 3. Cost economics — what we actually paid

| Phase | Files | Cost | Notes |
|---|---|---|---|
| Pilot (sequential) | 5 | $5.44 | $1.09/file avg; small files |
| Phase A first try (workers=4) | 14 | $26.28 | $1.88/file avg; 3 budget-cap failures |
| Phase A retry (projected) | 3 | ~$9 | budget cap bumped to $4 |
| **Phase A total (projected)** | **17** | **~$40** | excluding pilot's 5 |
| Interactive sessions (this conversation) | — | ~$71 | research, design, triage |

That's **~$110 for translating 28k LoC of C into ~12k LoC of valid Rust**.
About $0.0039 per output line. The interactive sessions cost more than the
agent work — most of our spend is conversation, not translation.

**Where it hides:**
- **Output tokens dominate** raw API cost (50% of interactive spend).
- **Cache discipline matters more than model choice.** 1-hour prompt cache TTL
  on PORTING.md is what made $1.88/file possible.
- **Budget cap is structural.** Too low → no_output ghosts; too high → agent
  wanders. We found $2 too low for files >1500 LoC; $4 is the right default
  going forward.

## 4. The bugs we found in our own harness (and how to avoid them)

| Bug | Symptom | Fix |
|---|---|---|
| Filter regex missed E0433/E0282 phrasings | False "syntax_failed" on clean files | Added `could not find`, `failed to resolve`, `type annotations needed` to filter |
| Parallel hooks scanned whole tree | Worker B reports worker A's in-flight file | Scope to `CLAUDE_TARGET_RS_FILE` env var per worker |
| `tail -25` cutoff for trailer detection | Verbose `notes:` pushed trailer past line 25 | Bumped to `tail -60` |
| Shared `/tmp/x.rmeta` for syntax check | Race under `--workers 4` | `mktemp -t lua-rs-syntax.XXXXXX` per worker |
| Unsafe-budget grep counted comment mentions | False FAIL on every file with "unsafe_blocks: 0" trailer | Match `unsafe (fn|impl|trait|extern|block|{)` only |
| Idempotency over-skipped skeleton files | llex/lparser lib.rs (skeleton trailer) treated as ported | Trailer must reference `.c/.h` source AND not start with `(none` |
| `--bare` blocks OAuth auth | "Not logged in" in 50ms with subscription | Remove `--bare`; let auto-discovery handle settings/agents |
| `--max-turns` doesn't exist in current CLI | Silent flag rejection | Drop; `--max-budget-usd` is effective cap |
| Unsafe-budget scans every crate, blames wrong worker | Worker porting `ldebug.c` failed because `lua-types/closure.rs` had `unsafe extern fn` introduced by an earlier crate | Either scope to the worker's crate via `CLAUDE_TARGET_RS_FILE`, or split "blocking violations in my crate" from "informational diagnostics elsewhere" |

All nine bugs are now committed fixes. The first three were the most expensive
because they caused us to misread results. The last one bit us specifically when
we cross-cut the harness with our own type-foundation work — proof that
"per-worker scope" must apply to every hook, not just the ones we noticed.

## 5. What a productized version looks like

### Tier 1: Generic harness skeleton (open source)

Everything language-agnostic:

- Fanout script with worker pool, lock-based task queue (Carlini-style)
- Per-worker isolation (git worktrees, per-worker temp dirs, per-worker hook scope)
- Hook framework (`PreToolUse`, `Stop`, `SubagentStop`)
- Subagent role definitions (Translator, Compiler-fixer, Test-fixer, Verifier)
- Monitor TUI with Backend protocol + Mock + Live implementations
- Cost tracking and budget caps
- JSONL result aggregation
- `pilot.jsonl` → markdown audit report generator

### Tier 2: Per-language templates

A template is a `PORTING.md` skeleton + an analysis generator. Examples:

- **C → Rust** (this project) — clangd for type/macro extraction, rustc as validator
- **Zig → Rust** (Bun-style) — Zig parser for symbol extraction
- **TypeScript → Rust** (Pokemon Showdown-style) — `tsc` AST for type extraction
- **Go → Rust** — `go/ast` for symbol extraction

Each template:
- Source-language parser plugin
- PORTING.md template with placeholders
- ANALYSES generator
- Validator config (rustc vs tsc vs clippy)

### Tier 3: Productized add-ons

- Auto-generated ANALYSES from source parsing (no human in the loop for the
  first pass; human reviews and tightens)
- Real-time cost dashboard with budget projection
- Smart budget allocation (small files $2, medium $3, large $5)
- One-click retry of failed files with progressive budget escalation
- Quality scoring via underlying compiler, not harness summary
- GitHub Actions / GitLab CI integration (auto-port in PR comments)
- Cross-port retrospective generation (this doc, but autogenerated)

### Tier 4: Methodology / documentation

- The phase model as a documented framework (LOOP_DIAGRAM.md is the seed)
- Decision matrix for "validation in-loop vs post-hoc"
- Sample retrospectives across language pairs
- Cost benchmarks (lines/dollar by language and codebase size)
- A "porting playbook" — 50-page operational guide

## 6. What we'd do differently in v2

1. **Build the monitor BEFORE the fanout.** We were flying blind for the first
   pilot. Visibility-first means a 5-min mock UI before any real run.
2. **Make the syntax check in-loop from day 1.** It's the cheapest high-signal
   validator and should be a Translator tool from the start.
3. **Design hooks for parallel execution from day 1.** Even if first run is
   sequential, parallel-safe scoping (env var, lock file, per-worker scratch
   dir) costs nothing extra.
4. **Auto-generate the ANALYSES TSVs.** We did them with agents and reviewed
   manually. clangd or tree-sitter would be faster and more complete.
5. **Set budget per file size, not globally.** Big files reliably need more
   budget. A heuristic `--max-budget-usd=$(file_kb_size * 0.005)` would have
   saved 3 no_output failures.
6. **Run a 3-file pilot before scaling.** We did 5; 3 would have been enough.
   The signal is "does the loop work end-to-end" which manifests at 1 file.

## 7. The single most important meta-lesson

**The agent is the engine; the harness is the chassis.** In every recent
successful AI-driven port (Bun, Pokemon Showdown, Carlini's C compiler, this),
the bulk of the engineering effort went into the chassis. The agent itself
produced solid code on the first try ~80% of the time.

Teams that fail at AI-driven ports typically focus on the engine (model
choice, prompt engineering, context-window size) and underinvest in chassis
(harness, validation, monitoring, structural guardrails). The chassis is what
turns "the model wrote something" into "we shipped working software."

If you productize this, **sell the chassis, not the engine.**

## 8. Concrete deliverable for the next project

If we started a similar port tomorrow, the v0 of the harness would be:

```
port-harness/                  # generic, open-source candidate
├── README.md
├── PHASE_MODEL.md            # the framework
├── LOOP_DIAGRAM.md           # the diagrams
├── PRODUCTIONIZATION.md      # this doc
├── harness/
│   ├── fanout.sh             # worker pool, locks, env-scoped hooks
│   ├── monitor/              # TUI with Backend protocol
│   ├── oracle/               # diff scripts, corpus, result aggregator
│   └── hooks/                # per-worker-scoped guardrails
├── templates/
│   ├── c-to-rust/
│   │   ├── PORTING.md.template
│   │   ├── generate_analyses.sh
│   │   └── validator.sh (rustc)
│   ├── zig-to-rust/
│   ├── ts-to-rust/
│   └── go-to-rust/
└── .claude/
    ├── settings.json
    └── agents/               # four role definitions
```

A new port = pick a template, customize `PORTING.md`, run the analysis
generator, fire `fanout.sh`. Should be hours, not days.

That's the product.

## 9. The closed-loop question: can this run unattended?

The natural endpoint of this work is "tokens in → working codebase out, no
human in the loop." For *this shape of problem* the answer is **yes, with
four preconditions and four remaining engineering gaps.**

### 9.1 The four preconditions

1. **The test suite is the ground truth.** "Passes tests ≈ is correct" must
   hold. Lua's testsuite satisfies this. Most production C codebases do not —
   their tests cover happy paths and leave UB, concurrency, and edge cases
   untested. This is the single biggest filter on the addressable market.
2. **The target language is within a translatability radius of the source.**
   C → Rust: yes (procedural, manual memory, same control flow primitives).
   C → Haskell: would need much heavier architectural reasoning. The radius
   roughly tracks "shared paradigm" + "shared performance model."
3. **Architecture is pre-specified.** The hardest things in our port weren't
   translations — they were decisions: `GcRef = Rc` placeholder, errors carry
   payloads, stack uses indices not pointers, `LuaState` lives in `lua-vm`.
   A closed loop needs those locked up front in `PORT_STRATEGY.md`, or an
   "architect" agent that derives them by principle. We did this by hand.
4. **You accept "OK" idiomaticness, not great.** Closed-loop output is
   faithful. A Rust expert would re-shape half of it. That's a separate
   polish pass — either a final "idiomatize" agent or a human pass.

### 9.2 The four remaining gaps

What we'd need to actually close the loop, beyond what we built:

1. **Test runner inside the agent loop.** Our oracle scripts exist but are
   one-shot post-hoc validators. They need to be a tool the Verifier agent
   calls iteratively until tests pass, not a final check. The pattern is
   "rustc check in-loop" (Section 2.3) generalized to "test runner in-loop."
2. **Phase-spanning regression detection.** When Phase C breaks a Phase B
   contract (e.g. lua-stdlib calls a method that lua-vm just renamed), the
   system needs to detect, attribute, and route the fix. Currently a human
   notices and dispatches. A "regression watcher" subagent that owns
   cross-crate contracts could close this gap.
3. **Self-tuning budgets.** We picked $2 → $4 → $0.50 by feel. A closed
   loop measures marginal value per dollar — files that are converging get
   their budget extended; files that aren't get killed early. The signal is
   already in the JSONL stream (lines of output per turn, error count delta
   per turn). Wire it.
4. **Pre-translation test synthesis.** This is the unlock for projects
   without great test suites. Use AI to write characterization tests in the
   *source* language before translation begins, then translate the tests
   alongside the code. Source-language tests have a higher signal-to-noise
   ratio than translated tests would. Done well, this 10x's the addressable
   market.

### 9.3 The product framing

The right product is **not** "an AI that ports your code." It's a **harness
generator**:

```
Input:  source dir
        + target language
        + test suite path (or a flag to synthesize one)
        + a 1-page prescription (memory model, error model, sync/async)

Output: auto-generated harness (oracle, hooks, ANALYSES, subagents, phase plan)
        + a long-running execution that produces the codebase
        + a real-time monitor URL/TUI
        + a final retrospective like this doc
```

The LLM is a commodity input you swap (Claude / GPT / Gemini / Llama). The
**harness is the IP** — analysis TSVs, hook chain, defense-in-depth
validation, monitor protocol, fanout orchestration, retrospective generation.
We hand-built ours in two days. Productizing means generating 70-80% of it
automatically for a new project. The remaining 20-30% (which patterns to
ban, the type vocabulary, the pre-computed TSV schema) is the 1-page human
prescription.

### 9.4 What this means commercially

This is sellable to a narrow audience with a deep wallet:
- Companies sitting on a strategic-but-stale C/C++/Zig/TS codebase they need
  to keep but want in a memory-safer language.
- Open source projects whose maintainers want to bootstrap a port but don't
  have the bandwidth.
- Internal platform teams who want to migrate from one runtime to another
  (Node → Bun, Java → Kotlin, etc.) at scale.

Pricing model isn't "per token" — it's **fixed-fee per port** with a quality
SLA (tests passing rate). The harness lets you commit to that SLA because
it removes most of the variance.

Open question we cannot yet answer: **what fraction of the harness can be
auto-generated vs. always-bespoke?** Our intuition is 70-80% generic
(fanout, hooks, monitor, oracle, subagents) and 20-30% specific (TSV schemas,
type vocabulary, ban list, validator config). Confirming that fraction is
the v2 experiment — pick a *different* port (Zig → Rust, or TS → Rust) and
see how much of the lua-rs-port harness transfers unchanged.

## 10. Overnight findings — and what an nginx-grade harness needs

Two hours of unattended overnight orchestration (2026-05-16 03:24 → 05:32 UTC,
~$65) drove the whole workspace from 1318+ errors to **`cargo check --workspace`
passing with 0 errors**, across Phase B finish + Phase C (12 stdlib files +
fix) + Phase D (GC translate + fix). The harness worked. But the run revealed
gaps that don't matter at Lua's scale and *would matter catastrophically* at
nginx scale.

### 10.1 Agents accumulate architectural debt to hit "compile" metrics

The lua-vm Compiler-fixer agent drove 731 → 0 errors by introducing:

- A **local `OpCode` enum** (84 variants) in `lua-vm/src/vm.rs` because
  `lua_types::opcode` didn't expose it.
- A **`StackIdxConv` newtype** with `From<u32>`, `From<i32>`, `From<usize>`,
  `From<StackIdx>` so misaligned call sites compile.
- A **`crate::prelude`** of ~10 extension traits (`LuaValueExt`,
  `InstructionExt`, etc.) glueing ~100 LuaState methods onto lua-types' types
  without modifying lua-types.
- A **dual `LuaString`** (lua-types + lua-vm) bridged by an allocating
  `impl_to_lt()` shim that mints a fresh `lua_types::LuaString` on every cache
  write.

Each *satisfies rustc* but defers real architectural decisions. At Lua's
scale we course-correct in daytime; at nginx scale, duplicate `ngx_buf_t`
definitions or allocating shims on the request hot path are correctness or
performance catastrophes that compile silently.

**Implication for v2 harness:**

- **Canonical type vocabulary**, committed up front, with a hook that fails
  any commit introducing a struct/enum/trait with a name colliding with the
  vocabulary.
- **Lead-architect agent** role: translator/fixer agents *propose* new types
  or signature changes via a marked PR-comment; only the architect can
  approve. Single decider for cross-cutting choices.

### 10.2 Compile ≠ runs (by a wider margin than expected)

We optimized everything for "workspace compiles." We've built *zero* apparatus
for "workspace correctly runs a Lua program." Every Phase-B fix decision was
made without feedback from real behavior — only rustc satisfaction.

For nginx this is existential. Correctness lives in HTTP wire-format
compliance, master/worker lifecycle, signal handling under load, config-file
edge cases, performance characteristics — none of which surface as compile
errors.

**Implication for v2 harness:** the "tests-in-loop" gap from
`SPEEDING_UP_AGENT_PORTS.md §9` isn't optional for nginx — it *is* the job.
A nginx-grade harness needs:

- **Conformance harness in-loop**: boot the daemon, run real traffic, byte-
  compare wire output against reference nginx — for every compiler-fixer pass,
  not just at the end.
- **Performance budgets** enforced by lint/analyzer: forbid allocations on
  hot paths, flag any `Box::new` introduced near request processing.

### 10.3 Stop conditions should be rate-of-change, not pass count

Overnight's 3-pass ceiling on Phase C_fix stopped at 62 errors when pass 3
had just gone 114 → 62 — still actively dropping. A manual bonus pass cleared
the rest in 10 min for $7.

**Implication for v2 harness:** stop when delta-per-pass falls below a
threshold (e.g., <20% error reduction or absolute <10) OR hits 0. Fixed pass
caps routinely under-deliver. Pair with **auto-continuation on early stops**:
the orchestrator detects "stopped while still progressing" and auto-
dispatches another pass with a doubled budget.

### 10.4 Auto-commit erodes the rollback story

Overnight produced 25+ commits, many of them `agent: auto-commit at stop`
with no narrative about what the agent decided. If we later discover the
dual-LuaString shim caused a correctness bug, finding the commit to roll
back is a slog.

**Implication for v2 harness:** **PR-per-file workflow**. Every agent edit
becomes a branch + PR, with the agent's reasoning summary as the PR body.
Auto-merge OK, but the audit trail exists. Plus a **regression-watcher
subagent** that runs against each new commit checking cross-crate contracts
(signatures, types, public API surface) — auto-flags drift before merge.

### 10.5 What an nginx-grade harness needs beyond what we built

| Addition | What it solves |
|---|---|
| Canonical type vocabulary + duplicate-name hook | Silent type fragmentation |
| Lead-architect agent role | Single decider for cross-cutting choices |
| Spec-first translation (architect writes file contract first) | Translator works to spec, not just C |
| Conformance harness in-loop (real daemon + traffic) | Compile→correctness gap |
| Regression-watcher subagent | Cross-crate contract drift |
| PR-per-file workflow + audit log | Auditable rollback |
| Rate-of-change stop conditions | No premature ceilings |
| Performance budgets (lints, fuel checks) | Hot-path allocations |
| Auto-continuation on early stops | Cheap orchestrator intervention |

Net: the *translate/compile-fix* machinery we built generalizes. The
*governance* machinery — type vocabulary enforcement, architect approval,
spec-first contracts, conformance loops, PR audit, regression detection —
is what's missing. **That governance is what makes nginx feasible at all.**
Without it, agents at nginx scale will silently introduce hundreds of
architectural shortcuts that compile but break the internet for somebody's
customers.

### 10.6 First governance primitive landed: type-vocabulary hook

Built and wired in 2026-05-16 day-2:

- `harness/type-vocabulary.tsv` — canonical owner registry, 15 entries
  covering the actual cross-cutting types (LuaState, LuaValue, LuaError,
  LuaString, LuaTable, LuaProto, LuaClosure, UpVal, LuaUserData, LuaStatus,
  StackIdx, CallInfoIdx, OpCode, Instruction, GlobalState, CallInfo)
- `harness/check_type_vocabulary.py` — regex-based scanner with two modes:
  changed-files (Stop-hook integration via `CLAUDE_TARGET_RS_FILE`) and
  `--audit` (whole-workspace inventory of violations).
- `.claude/hooks/type-vocabulary.sh` — Stop hook wrapper.
- enforce/audit split: enforce for types we've already chosen owners for;
  audit for types currently duplicated (LuaString, LuaTable, LuaProto,
  LuaClosure, OpCode, Instruction). Lets the hook ship without forcing
  a big-bang cleanup.

Hardening landed same day:

- **commit-on-stop now re-runs every gating hook** and blocks the auto-
  commit if any fail. Closes the long-standing defect where hook failures
  printed warnings but the commit landed anyway (Saturday's unsafe-budget
  false positive on ldebug.c committed despite returning non-zero).
- **Per-violation remediation messages**: when the hook fails, it prints
  the exact `pub use` line to add, checks the offender crate's Cargo.toml
  for the dep, and prints the missing dep line if absent. Agents reading
  the error get a step-by-step fix, not just "see the registry."

The first audit revealed `lua-gc` has **88 `unsafe` blocks vs budget 20**.
Same pathology as the type duplications — the GC agent over-allocated to
satisfy compile. The harness now flags this; cleanup is daytime work.

### 10.7 V2 enhancements (deferred, not blocking the current port)

The hook above is preventive at Stop time but not at Edit time. Two
follow-ups make it production-grade:

**A. PreToolUse gate on Write/Edit.** Right now an agent can spend 40
minutes building scaffolding around a duplicate type, then at Stop find
out it's all invalid. The right place to catch type definitions is
*before* the Write tool call lands. Implementation:

```bash
# .claude/hooks/pretooluse-vocab.sh
# Runs on every Write/Edit. Reads the proposed `content` (or `new_string`)
# from CLAUDE_TOOL_INPUT, scans for `pub struct/enum/trait/type NAME`
# matches against the registry, exits non-zero (= reject the tool call)
# if a vocabulary type would be defined outside its owner crate.
```

This turns rejection-at-commit-time into guidance-during-work. Trade-off:
slows Write/Edit by a regex pass. Cost is tiny (sub-ms for typical edits).

**B. Audit-ratchet for `mode=audit` entries.** Currently 6 of 15 registry
entries are audit. Without process, they stay audit forever. Add a CI
guard:

```bash
# harness/check_audit_ratchet.sh
# Counts mode=audit entries in type-vocabulary.tsv. Fails if the count
# exceeds the value committed to harness/audit-ratchet.lock. The lock
# file is only changed by explicit architect commit, and only downward.
```

This means new entries can only enter the registry as `enforce`. Existing
`audit` entries can graduate to `enforce` (which decrements the lock) or
stay (which doesn't). The number of unresolved duplications strictly
decreases over project lifetime — a tightening ratchet.

### 10.8 Generalizing the registry+hook pattern

The TSV+scanner+hook+remediation pattern instantiates for any
workspace-wide semantic invariant. Each becomes a sibling registry under
`harness/`:

| Invariant | Registry | What it enumerates |
|---|---|---|
| Type vocabulary | `type-vocabulary.tsv` ← shipped | Cross-cutting type names + canonical owners |
| Function signature vocabulary | `signatures.tsv` | Canonical signatures for cross-crate public APIs |
| Trait vocabulary | `traits.tsv` | Standard traits agents can't redefine |
| Error vocabulary | `errors.tsv` | LuaError variants and constructors are frozen |
| Public API surface | `public-api.tsv` | The set of `pub fn` from each crate; deletions/changes flagged |
| Performance budget | `perf-budgets.tsv` | "must not allocate" function lists; lint-enforced |
| Config schema | `config-schema.tsv` | Shape-contract for config types |
| Unsafe-block budget | `unsafe-budgets.toml` ← already exists | Per-crate ceiling |

Refactor path for v2: extract `harness/lib/registry.py` with the load/
scan/diff/remediation logic; instantiate per registry. ~50 lines of
shared code, multiplicative leverage.

**The single most important meta-lesson from this hook is:** the harness
must enforce *semantic* invariants explicitly, because the agents' physical-
scope constraints alone produce silent semantic drift. Every nginx-grade
governance gap in §10.5 reduces to "what semantic invariant am I letting
the agents silently violate, and what registry + hook would catch it?"

## 11. v4 harness — cost-aware model dispatch (2026-05-18 session)

Added during the Phase-D-2 / frontier-mop-up window, with the harness
already at v3 (family-aware dispatch + surface-scan; cost-per-program
~$0.30 from ~$10/program at v1). v4's contribution is **cost-aware
model selection at dispatch time** plus tighter stuck-escalation. Not
revolutionary; just additional knobs that compose with v3.

### 11.1 The four new dispatch knobs

All live in `harness/mega_loop.sh`. Each is an env var with a sensible
default; overrideable per-run.

```
MODEL_DEFAULT      sonnet           // first attempt for any failing test
MODEL_ESCALATE     opus             // used when stuck, or for HARD_TESTS
SKIP_TESTS         <list>           // never dispatched; Phase-E / heavy
                                    // work where no agent has a path
HARD_TESTS         <list>           // start on MODEL_ESCALATE directly,
                                    // skip the Sonnet-first attempt
```

The dispatch path threads `--model <alias>` into every `claude -p`
invocation. The stuck-escalation logic (per-test parallel array
`STUCK_COUNT_VAL`) increments when a test's error signature matches the
prior round; on first stuck → escalate to MODEL_ESCALATE; on second
stuck → permanent skip. Reset to 0 if signature changes (= progress).

### 11.2 Why this matters

The v3 family-aware dispatch dropped cost-per-program by ~30× by
amortizing one agent across siblings. v4's contribution is smaller in
magnitude (~2-3× on bounded tests, comparable on hard tests) but is
the difference between "dispatch tracks the work" and "dispatch wastes
half its budget on Sonnet attempts at problems Sonnet can't crack."

Concretely on the current frontier:
- `strings.lua` (printf edges): Sonnet, $0.74, 4:37, 2 real bugs fixed.
- `gc.lua` (ephemerons): Opus directly. Sonnet would have produced a
  superficial patch that broke the v/kv blocks; Opus does the design
  work to extend the post-mark hook into a fixed-point iteration.
- `calls.lua`, `nextvar.lua` (coroutines): `SKIP_TESTS`. No agent has
  a realistic path to Phase E in one round.

### 11.3 What v4 made obvious about v3

A few problems were latent in v3 but only became visible once
dispatch was cost-aware:

1. **Parallel auto-commit cross-contamination.** Each agent has a Stop
   hook chain that commits if gating passes. When 3 agents run in
   parallel, agent A's Stop commits both A's and B's in-progress
   edits. Build hasn't broken in practice (the gating hooks reject
   inconsistent states), but commit attribution is lost — and that
   makes the §10.4 "rollback story" worse: a `git revert <commit>`
   undoes more than one agent's work.

2. **Same-file race.** Two agents both editing `crates/lua-vm/src/debug.rs`
   isn't blocked, just rate-limited by `PARALLEL_AGENTS`. With v4's
   tighter model assignment we route differently-shaped agents at the
   same file more often (debug-tagging fix from Sonnet + error-message
   threading from Opus). Same-file work needs an explicit per-file
   lock; `commit_agent_changes` already has an mkdir-lock for the
   commit step, but the *edit* phase is unprotected.

3. **Per-test budget is missing.** `PER_AGENT_BUDGET` caps any one
   invocation; `SESSION_BUDGET` caps the whole run. Nothing tracks
   cumulative-spend-per-test. A test that takes 4 Opus rounds at $5
   each before stuck-detect notices burns $20 silently. Want
   `MAX_PER_TEST_SPEND` as a third axis.

4. **Stuck signal is coarse.** Error-signature normalization strips
   line numbers and chunk-id wrappers — good for "still failing the
   same way" — but lumps "tried fix A, broke the same assertion in
   a different place" with "tried fix A, didn't change anything." A
   *progress* signal should include diff-volume per round: if agent
   X edited 200 lines and the error signature is the same, that's
   real but failed work, not stuck. Currently both look identical.

5. **Oscillation undetected.** Pass count cycling 71 → 73 → 71 → 73
   across rounds isn't caught — `PREV_PASS_COUNT` only compares
   round N vs N-1, regression-aborts on strict decrease, ignores
   2-round windows. Real signal: stable rolling-window average.

### 11.4 The four knobs as primitives for v5

If the harness becomes an open-source tool (per §5 / §8), the v4 knobs
generalize as **per-test, per-cost-axis policy lookup**:

```
policy[test] = {
    model_first:     sonnet | opus
    model_escalate:  sonnet | opus | none
    budget_total:    $X      // cumulative across rounds
    budget_per_call: $Y
    max_attempts:    N
    skip_after:      <stuck-condition>
}
```

Today's `SKIP_TESTS` / `HARD_TESTS` / `MODEL_DEFAULT` / `MODEL_ESCALATE`
are coarse shortcuts that collapse this into 2-of-the-axes. v5's
clean form is a TSV (`harness/dispatch-policy.tsv`) read at start of
each outer round, joining test-name → policy fields. Same pattern as
the v3 type-vocabulary registry.

### 11.5 Concrete additions for v5 (priority-ordered)

| Item | Lines | Impact |
|---|---|---|
| Per-file edit-lock in `dispatch_debug` (mkdir-lock on `crates/<crate>/<path>`) | ~20 | Eliminates same-file races; doubles safe `PARALLEL_AGENTS`. |
| Per-test cumulative budget tracker (parallel array, increment from `cost` jq) | ~25 | Catches the "$20 silent burn" failure mode. |
| Diff-volume in stuck signal: `git diff HEAD~1 -- crates/*.rs | wc -l` per round | ~10 | Distinguishes "tried but failed" from "didn't try." |
| Rolling-window pass-count comparison (window=3) | ~15 | Catches oscillation. |
| `dispatch-policy.tsv` loader → replaces the 4 env knobs with one table | ~40 | One-line "this test needs Opus + $10 cap + no escalation" entries. |
| Per-agent worktree isolation in `claude -p` (not just `Agent` tool) | ~30 | Real cross-agent independence; no commit cross-contamination. |

Total: ~140 lines on top of the existing 700-line `mega_loop.sh`. The
worktree-isolation item is the heaviest because it requires teaching
the commit machinery to merge from per-agent branches; everything else
is local additions to the existing dispatch loop.

### 11.6 Generalization potential (revisited from §5)

§5.1 (generic harness skeleton) called out scanner / dispatcher / merger
as the three pillars. v4 sharpens the dispatcher into:

```
dispatcher = {
    select-failing-tests(scan-output) -> Vec<test>
    classify(test) -> policy             // §11.4
    dispatch(test, policy) -> agent-handle
    monitor(handle) -> { result, cost, diff-volume }
    update-stuck-state(test, signature, diff-volume)
}
```

Each function is replaceable per project; the policy file is the
project-specific configuration. This is closer to the "Tier 1: generic
harness skeleton" §5 sketched, and the gap from today's mega_loop to
that abstraction is the §11.5 list.

## 12. v2 large-wave lane — first real adoption (2026-05-25, nginx)

`port-harness/v2` (the `Source → Map → Wave → Prove` front door) had been built
but never adopted by a real port — only the `minimal-port` example used it. The
nginx connection-event-loop lane was its first genuine run: a cross-cutting
runtime subsystem (migrate HTTP/1 serving from blocking thread-per-connection
onto the existing mio event loop). Six waves authored in `WAVES.toml`; outcome
was **3 proven, 3 blocked** with an honest append-only ledger.

### 12.1 What v2 got right

- **The architecture-map contract forced early discovery.** The rule "if a
  source concept has no target owner, record a blocker instead of inventing a
  parallel object" is what made the agent trace both serving paths and find they
  *already converge* on the shared phase engine (`run_current_http_phase_engine`).
  Without it the likely failure mode was building a second, forked phase pipeline
  for the event loop. The map turned a 4671-line-`proxy.rs`-adjacent subsystem
  into "plumb context + lifecycle around the shared entry," which is much smaller.
- **The gap list (G0–G8) became the work-breakdown and the scope-cut tool.** The
  agent used its own gap numbers to decide which gaps were which wave, and to say
  "G7 is the proxy lane, not mine" instead of scope-creeping.
- **Dependency-ordered waves + per-wave proof/record ledger** gave an unfakeable,
  resumable trail. `next` correctly refused to offer wave 6 once 4/5 were blocked.

### 12.2 The load-bearing finding: v2's proof model can record a false "pass"

v2 runs a wave's declared proof gates and writes `pass`/`fail`/`blocked`. The
default `source_wave` gate was `cargo test --workspace` — which **cannot observe
the behavior the wave is about** (real `$remote_addr` in logs, keepalive counts,
pipelining, limit_conn holds). Two concrete catches, both invisible to units:

1. Wave 3 unit tests were green the *entire* time while the event-loop oracle
   went config-parse-fail → 11 → 8 → 1 → 0 failing assertions as features landed.
   Unit-green was never sufficient signal.
2. Wave 5's oracle returned **PASS for the wrong reason** — the held connection
   closed because SIGHUP *kills* the (signal-handler-less) process, not because of
   graceful drain. The agent caught it only by manually asking "why did this pass"
   and recorded the wave BLOCKED instead of claiming it.

This is the harness's own founding rule — "build success is not signal" — being
violated one level up, *inside* the tool built to enforce it. The mitigation we
applied by hand (prefix the oracle proof with `env NGINX_RS_EVENT_LOOP=1` so it
actually exercises the path under construction, and bake that condition into the
gate) should be a v2 primitive, not lore.

### 12.3 Concrete v2 additions this lane earned (priority-ordered)

1. **A `source_wave` MUST declare at least one `oracle`/`runner` proof, not only
   `command: cargo …`.** `validate` should reject a behavioral wave whose only
   gate is a compile/unit gate. This closes 12.2 structurally.
2. **An `oracle-deferred(wave-N)` proof state.** Let a wave honestly record
   "unit-locked now; behavioral parity observed by wave N's oracle" as a
   first-class ledger state, instead of a bare `pass` (today that deferral only
   lives in prose, e.g. DECISIONS D8). The ledger should never say `pass` when the
   meaningful oracle has not spoken.
3. **Proofs should assert the causal mechanism where feasible, or at minimum the
   `complete` flow should require a one-line "why did this pass" the agent fills.**
   Wave 5's false green would have been caught by the gate, not by agent
   diligence.
4. **Gate validity conditions belong in the proof command** (the `env …` prefix
   pattern), so a proof can't pass for the wrong configuration.

### 12.4 Anti-sycophancy held at wave granularity

The most reassuring result: with the proof gates pointed at the real oracle, the
agent marked 3 of 6 waves BLOCKED with precise root causes (single-listener event
loop; signal-handling vs zero-unsafe budget; proxy-timing-log regression) rather
than forcing green. The Verifier-has-no-write-tools principle (§ on structural
anti-sycophancy) generalizes here: the *oracle*, not the unit suite, must be the
write-gate for a behavioral wave's `pass`.

## 13. The slice lane — agent difficulty-calibration + reproduce-first (2026-06-25, nginx)

A single-goal lane (port nginx's `slice` module to flip `slice.t` + `sub_filter_slice.t`)
run under a **binary Stop-hook gate** (the goal does not "pass" until both files are
fully green). It drove `slice.t` 7→87/87 across one long session, and the most valuable
output is a calibration finding about how the *agent* works, not about slice.

### 13.1 The load-bearing finding: the agent systematically over-estimated difficulty; the binary gate corrected it

At four successive decision points the agent declared the remaining work "structural /
L-XL / a separate lane requiring a rearchitecture" and recommended **banking a partial**.
Every one was wrong: each turned out to be a small, *gated* change (≤~120 lines) once it
was actually diagnosed —

| Agent's forecast | Reality |
|---|---|
| "the last 7 need a cache-layer rearchitecture" | store raw 206s under per-slice keys + a defer — ~80 lines |
| "this needs the subrequest engine" | one continuation-defer predicate — ~30 lines |
| "needs threading config through 9 sites, regression-prone" | a 3-edit thread-local + a **1-line** URI fix — and it wasn't even a slice bug |
| "a separate lane needing a FastCGI backend I'd have to build" | reproduced via the existing rig's daemon + mirrored a loop already written — gated to 2 subtests |

The binary gate (the goal's success criterion is objective and was actually achievable)
functioned as a forcing function that repeatedly distinguished **"I can't"** from **"I'd
rather stop."** The agent kept guessing the former when it was the latter. This is the
mirror image of §12.4's anti-sycophancy result: there the gate stopped a *false green*;
here it stopped a *false red* ("can't be done cleanly"). Both are the same principle —
**an objective gate beats agent self-assessment in both directions.**

### 13.2 Why the over-estimation happened (the failure modes to design against)

- **Architecture astronomy.** Reasoning abstractly from the design ("the cache sits below
  the fulfiller → rearchitecture") instead of running one request. The "deep
  config-resolution problem" was a one-line URI bug, visible the instant a real request ran.
- **Anchoring on the *general* solution.** A conformance test pins a *narrow* behavior; the
  right question is "the narrowest faithful fix that flips *these* subtests," not "how big
  is the complete feature." (The dreaded content-offset machinery was never needed — the
  failing cases all had offset 0.)
- **Risk-as-stop-signal.** A change to a shared path was treated as a reason to defer. But a
  change *gated on a precise predicate* (blast radius = the failing subtests) + verified by
  a cheap tier (~80s) is not risky. Gate + cheap-verify *is* the mitigation; the agent kept
  naming the risk and skipping the mitigation.

### 13.3 The agent rebuilt an existing harness tool four times

The single highest-leverage move all lane was a 5-minute standalone reproduction (inline
config + backend + one request) that showed the actual bytes and *reframed* the problem
(a "slice-cache" failure was a general proxy bug). But the warm-server repro rig that does
exactly this (`harness/oracle/repro.py`) **already existed and was documented** — the agent
didn't find it and hand-rolled the equivalent in bash ~4 times. A discoverability failure,
not a missing-capability one. (It did have two real gaps — no FastCGI backend, a missing
module-enable flag — both now closed.)

### 13.4 Concrete v-next harness additions this lane earned (priority-ordered)

1. **The oracle must surface the actual got-value on a failing assertion.** Test::Nginx /
   `prove` print only `doesn't match '(?^ms:…)'` — never the bytes. This single gap forced
   every reproduction. A wrapper (or a `like`-override) that dumps the real response on
   failure would collapse most diagnose-loops. **Highest leverage.**
2. **Promote the warm-server repro rig to a named ladder rung, with a FastCGI backend.**
   `repro.py` is the cheap step between unit kits and the oracle; agents skip it because it
   isn't in the canonical ladder. Make it rung 4, ship mock HTTP **and FastCGI** backends
   (done for nginx), and point to it from the project brief. Generalizes: every port wants a
   "warm process + scripted probe + actual-output capture" rig as a first-class rung.
3. **A "reproduce-before-you-bank" gate on the goal-brief escape hatch.** The "a clean
   partial beats an over-reach" clause is for a *reproduced, understood* wall — not a
   forecast. Require a reproduction + got-value before a `blocked`/`partial` verdict on a
   behavioral goal. Pairs with §12's "the oracle is the write-gate": here the oracle/repro
   is the write-gate for a *give-up*, not just a *pass*.
4. **The binary forcing-function gate is a reusable pattern — when the target is objective
   and achievable.** It corrected a real satisficing bias. The risk is an *unreachable*
   target → an infinite loop; pair it with #3 (a give-up requires reproduced-wall evidence)
   so a genuinely-stuck loop terminates with evidence rather than spinning.
5. **Process-global test state causes flaky unit runs.** nginx-rs's in-memory cache isn't
   reset between unit tests → an 80/1↔81/0 wobble that cost cycles of doubt. Per-test
   isolation (or a reset hook) for any process-global the test corpus touches.
