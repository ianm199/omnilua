export const meta = {
  name: 'fix-79d-traceback-frame',
  description: 'Fix #79(d): uncaught top-level errors must emit the trailing [C]: in ? traceback frame, cross-version (5.3/5.4/5.5). Run the CLI main chunk beneath a synthesized pmain C-closure frame so the stack walk reaches it.',
  phases: [
    { title: 'Design', detail: 'read-only: confirm exact reference traceback across versions + entry points; choose the minimal mechanism' },
    { title: 'Implement', detail: 'pmain-as-C-closure (or base-CallInfo) restructure + spawn-the-binary oracle test + gate all versions' },
    { title: 'Verify', detail: 'read-only cross-version CLI-stderr diff vs reference; confirm no traceback regression' },
  ],
}

const ROOT = '/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port/.claude/worktrees/git-issues'

const CTX = [
  'Repo: ianm199/lua-rs (pure-Rust Lua), branch fix-79d-traceback (off main @ v0.0.21). Goal: fix issue #79(d) — the LAST open item of #79.',
  '',
  'THE DEFECT (already diagnosed; confirm, do not re-derive from scratch):',
  '- Uncaught top-level errors are MISSING the trailing `[C]: in ?` traceback frame that the reference emits. It is the sole blocker on official math.lua and a line-noise contributor on EVERY error-raising suite file across all versions.',
  '- The traceback BUILDER is correct: ' + ROOT + '/crates/lua-stdlib/src/auxlib.rs::traceback (line ~261) walks get_stack(level) and renders a C frame as `[C]: in ?` via push_func_name. It would emit the frame IF one existed in the call stack.',
  '- The gap: the CLI runs the main chunk via run() -> dofile/dostring -> docall (crates/lua-cli/src/interp.rs ~line 394) -> api::pcall_k(chunk). There is NO base C-closure CallInfo beneath the chunk. Reference lua.c pushes `pmain` (a C function) and lua_pcall()s IT; pmain then does the arg handling + docall, so the chunk runs BENEATH pmain on the call stack, and an uncaught error walks down to pmain -> `[C]: in ?`.',
  '- THE FIX (per the approved plan): restructure the CLI so the main work runs beneath a synthesized base C frame — the pmain-as-C-closure pattern from lua.c. Mirror lua.c: push a C closure (pmain) carrying argc/argv, lua_pcall it, and move the existing arg-parse/dofile/dostring/REPL orchestration INTO pmain. Keep the stack-walker (get_stack / get_info / push_func_name) UNCHANGED — it is correct.',
  '',
  'THE ENGINE (follow it; do not re-invent):',
  '- Oracle = unmodified make-macosx reference binaries in /tmp/lua-refs/bin (lua5.3.6 / lua5.4.7 / lua5.5.0). This is a CLI-level concern (traceback printed to stderr by the lua binary), NOT an in-process API concern — so probe by RUNNING the binaries on a temp .lua file and comparing stderr, e.g. `/tmp/lua-refs/bin/lua5.4.7 /tmp/x.lua` vs `LUA_RS_VERSION=5.4 ' + ROOT + '/target/debug/lua-rs /tmp/x.lua`.',
  '- diff_one.sh ' + ROOT + '/specs/oracle/diff_one.sh <ver> "<lua>" exists but uses -e; for traceback fidelity prefer running an actual file AND -e AND stdin, since entry point affects the chunk name shown in the frame.',
  '- CI test: this needs a CLI-level (spawn-the-binary) oracle test, because the `[C]: in ?` frame only appears in the CLI traceback path, not the in-process load+pcall wrapper in multiversion_oracle.rs. Add a spawn-the-binary integration test (a new tests/*.rs in crate lua-cli, or extend an existing CLI test harness) that runs target/debug/lua-rs on a temp script that errors, and asserts the stderr ends with the `[C]: in ?` frame, for 5.3/5.4/5.5. Normalize paths/addresses as the existing oracle scripts do.',
  '- Also keep ' + ROOT + '/crates/lua-rs-runtime/tests/multiversion_oracle.rs green (it should be unaffected).',
  '- Gate (must stay green): cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive ; ' + ROOT + '/specs/oracle/check.sh {5.4,5.3,5.5}. 5.4 MUST NOT regress.',
  '- Additional gate specific to this fix: the official suites that assert on tracebacks must not regress. Spot-check by running, via the CLI on real files, a few official tests (math.lua should ADVANCE; errors.lua, calls.lua should not regress) against the reference and comparing the FIRST divergence line. The harness scripts in ' + ROOT + '/harness can run official tests.',
  '- IMPORTANT subtlety to confirm against the reference: does the trailing `[C]: in ?` frame appear for a file run, a -e string, and an interactive REPL line? Match the reference EXACTLY for each entry point (the frame, the chunk name in the preceding frame, and whether REPL differs). Confirm what the reference prints before implementing.',
].join('\n')

phase('Design')
const design = await agent(
  CTX + '\n\nDESIGN (READ-ONLY: run binaries, read code; do NOT edit). Produce a precise implementation design:\n' +
  '1. Capture the EXACT reference traceback (lua5.4.7, lua5.3.6, lua5.5.0) for an uncaught error via (a) a script file, (b) `-e`, (c) piped stdin, (d) an interactive REPL line. Show the full stderr for each. Identify exactly where `[C]: in ?` appears and what the frame above it says (chunk name, "in main chunk", "in function", etc.).\n' +
  '2. Read crates/lua-cli/src/interp.rs (run, pmain orchestration, docall, dofile, dostring, runargs) and crates/lua-cli/src/repl.rs, plus how api::pcall_k / CallInfo / get_stack set up and walk the call stack in crates/lua-vm (do_.rs, debug.rs). Determine the minimal faithful mechanism: a true pmain C-closure pcall (mirroring lua.c) vs. synthesizing a base CallInfo. State which, with the exact functions/types to touch and why.\n' +
  '3. Note any cross-cutting risk: does adding a base C frame shift level numbering anywhere (debug.traceback default level, error location prefixes, xpcall handlers)? List what must be re-verified.\n' +
  'Write ' + ROOT + '/specs/followup/79d-design.md with: per-version expected stderr, the chosen mechanism, the exact edit plan (files/functions), the CLI-level oracle test design, and the regression-risk checklist. Return a ~12-line summary.',
  { label: 'design:79d', phase: 'Design', agentType: 'general-purpose' })

phase('Implement')
const impl = await agent(
  CTX + '\n\nIMPLEMENT. Read ' + ROOT + '/specs/followup/79d-design.md and follow its chosen mechanism. Implement the pmain-as-C-closure (or base-CallInfo) restructure so uncaught top-level errors emit the trailing `[C]: in ?` frame, matching the reference EXACTLY for each entry point (file / -e / stdin / REPL) on 5.3/5.4/5.5. Do NOT change the stack-walker logic (get_stack/get_info/push_func_name) — it is correct. Add the CLI-level spawn-the-binary oracle test described in the design. Then GATE: cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive (0 failures) ; check.sh 5.4 AND 5.3 AND 5.5 (all green, 5.4 no regression) ; and confirm the fixed cases match the reference stderr via spawning both binaries. Spot-check official math.lua (should advance) and errors.lua/calls.lua (no regression) through the CLI vs reference. If something is ambiguous or risky, STOP and document rather than guess. Commit on fix-79d-traceback: git add -A && git commit -m "fix(cli): emit trailing [C]: in ? traceback frame for uncaught errors (#79(d))". Return what landed, gate results, exact before/after stderr for one case per version, and anything deferred.',
  { label: 'impl:79d', phase: 'Implement', agentType: 'general-purpose' })

phase('Verify')
const verify = await agent(
  CTX + '\n\nVERIFY (READ-ONLY: run binaries/tests, read code; do NOT edit). Independently confirm the fix:\n' +
  '1. For each version 5.3/5.4/5.5, run several uncaught-error cases (a runtime error in a function called from the main chunk; an error at top level; a nested call; an error inside pcall that is re-raised) through BOTH the CLI (LUA_RS_VERSION=<v> target/debug/lua-rs on a temp file) and the matching reference binary. Diff stderr (normalize prog path + heap addresses). Report MATCH/DIFF per case.\n' +
  '2. Confirm no regression: run check.sh for 5.4/5.3/5.5 and `cargo test --workspace --features lua-rs-runtime/derive`; report counts.\n' +
  '3. Run official math.lua, errors.lua, calls.lua through the CLI vs the version reference; report the first divergence line for each and whether it improved/held/regressed vs the pre-fix state recorded in specs/followup/PHASE_B_REPORT.md.\n' +
  'Return a verdict table (case -> MATCH/DIFF) plus a one-line PASS/FAIL on "the #79(d) frame is now faithful cross-version with no regression". Do not rationalize a pass — if any case DIFFs, say FAIL and show it.',
  { label: 'verify:79d', phase: 'Verify', agentType: 'general-purpose' })

return { design, impl, verify }
