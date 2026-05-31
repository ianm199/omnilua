export const meta = {
  name: 'mv-phaseC-finish-5.5',
  description: 'Phase C: finish Lua 5.5 (#20). Discover all current 5.5 divergences vs lua5.5.0, fix the headline long-tail features (global-already-defined guard, named varargs, utf8.offset 2nd return, collectgarbage param, error(nil), namewhat) + CI tests, drive official-5.5 slices toward parity.',
  phases: [
    { title: 'Discover', detail: 'parallel read-only cataloguing of 5.5 divergences: official-suite sweep + language-feature probes + stdlib/error probes' },
    { title: 'Fix', detail: 'sequential oracle-gated fixes by category + CI tests' },
    { title: 'Synthesize', detail: '5.5 parity report: before/after divergences, what remains' },
  ],
}

const ROOT = '/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port/.claude/worktrees/git-issues'

const CTX = [
  'Repo: ianm199/lua-rs (pure-Rust Lua), branch finish-5.5 (off main, v0.0.21). Goal: finish Lua 5.5 (issue #20). 5.5 runs on the shared modern core with version-gated deltas (contextual global, block-scoped global decls, <const> globals, read-only for vars, round-trip float tostring, table.create). This phase closes the headline long tail toward parity with lua5.5.0.',
  '',
  'THE ENGINE (follow it; do not re-invent):',
  '- Oracle = the unmodified make-macosx reference binaries in /tmp/lua-refs/bin (esp. lua5.5.0; also lua5.4.7 and lua5.3.6 to confirm NO cross-version regression). Contract pinned in ' + ROOT + '/specs/oracle/CONTRACT.md.',
  '- Differential oracle: ' + ROOT + '/specs/oracle/diff_one.sh <5.3|5.4|5.5> "<lua>" prints MATCH or a DIFF block.',
  '- Official 5.5 tests: /tmp/lua-refs/lua-5.5.0-tests/*.lua (run with preamble: _soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}). Use an ABSOLUTE path to lua-rs since you cd into the test dir: ' + ROOT + '/target/debug/lua-rs, selected via LUA_RS_VERSION=5.5. The CLI now runs the main chunk beneath a pmain C frame (PR #84), so tracebacks include the trailing [C]: in ? frame.',
  '- Adversarial-first: derive cases from the upstream 5.5 manual / official tests / probing lua5.5.0, NOT from our Rust source.',
  '- Research already done: ' + ROOT + '/specs/LUA_5_3_AND_5_5_PORT_SPEC.md and ' + ROOT + '/specs/research (look for 5.5 deltas). Prior 5.5 oracle cases are in ' + ROOT + '/crates/lua-rs-runtime/tests/multiversion_oracle.rs (the v55_* tests).',
  '- CI tests: extend ' + ROOT + '/crates/lua-rs-runtime/tests/multiversion_oracle.rs (Lua::new_versioned + the load+pcall wrapper already there). For CLI-level traceback/namewhat behavior, the spawn-the-binary harness is ' + ROOT + '/crates/lua-cli/tests/traceback_oracle.rs.',
  '- Version seam: LuaVersion is at state.global().lua_version; parser deltas in crates/lua-parse/src/lib.rs; stdlib roster gates in crates/lua-stdlib/src; float/number in crates/lua-vm/src/object.rs.',
  '- Gate (must stay green; a shared-core change must match EVERY version reference, not just 5.5): cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive ; ' + ROOT + '/specs/oracle/check.sh {5.4,5.3,5.5}. 5.4 AND 5.3 must not regress.',
  '',
  'KNOWN 5.5 LONG-TAIL ITEMS (the plan + prior findings; confirm each against lua5.5.0, then fix the clear-cut ones):',
  '1. "global already defined" runtime/compile guard: `global x = expr` (or re-declaring) when x is already defined must error the way 5.5 does. Confirm the EXACT trigger (compile-time redeclare vs runtime non-nil) and message against lua5.5.0. File: crates/lua-parse/src/lib.rs globalstat.',
  '2. Named varargs: `function f(...t)` binds the extra args into table t. Confirm 5.5 syntax + semantics; parser + codegen.',
  '3. utf8.offset 2nd return value (5.5 returns the end position too). crates/lua-stdlib utf8 lib.',
  '4. collectgarbage("param", ...) — the 5.5 GC param API. Confirm names/return; crates/lua-stdlib gc/base + crates/lua-gc.',
  '5. error(nil) / error with non-string at level producing `<no error object>` in the CLI traceback. Confirm vs lua5.5.0.',
  '6. namewhat: 5.5 renders a global C function frame as `[C]: in global \'error\'` where lua-rs shows `[C]: in function \'error\'` (surfaced by the #79(d) verifier; in auxlib.rs::push_func_name). Confirm whether this is 5.5-only or all-version, and fix faithfully.',
  '7. _VERSION must be "Lua 5.5"; any other 5.5 stdlib roster deltas vs 5.4.',
].join('\n')

phase('Discover')
const areas = [
  ['suite', 'OFFICIAL 5.5 SUITE SWEEP. Run each /tmp/lua-refs/lua-5.5.0-tests/*.lua through LUA_RS_VERSION=5.5 lua-rs vs lua5.5.0 (with the preamble; cd into the test dir; absolute lua-rs path). For each file report: byte-identical, or the FIRST divergence (file:line + ours vs ref). Categorize every divergence (global-decl, named-varargs, stdlib-gap, error-wording, namewhat, number-model, other). Focus on files exercising implemented surface. Write ' + ROOT + '/specs/followup/5.5-divergences.md with a categorized table and a parity estimate (X/N byte-identical).'],
  ['lang', 'LANGUAGE FEATURES. Probe lua5.5.0 for the EXACT behavior of: (1) `global x = 1; global x = 2` and `global x; x=1` when x already global/defined — compile-time or runtime? exact message? (2) named varargs `function f(...t) return t end` and `local function g(...t)` — is `...t` valid 5.5 syntax? what does t contain? does `...` still work alongside? (3) `<const>`/`<close>` global edge cases. (4) for-loop control-var read-only edge cases. Compare each to lua-rs 5.5. Write ' + ROOT + '/specs/followup/5.5-lang.md: exact repros, expected, impl location (crates/lua-parse/src/lib.rs + codegen), clear-cut vs risky, CI assertions.'],
  ['stdlib_err', 'STDLIB + ERROR/TRACEBACK. Probe lua5.5.0 for: (1) utf8.offset return arity/values (2nd return). (2) collectgarbage("param", ...) and any 5.5 GC API names/returns; also collectgarbage("count")/("step") deltas. (3) error(nil) and error({}) — the CLI message/traceback (`<no error object>`?). (4) the namewhat `[C]: in global \'error\'` vs `in function \'error\'` — is it 5.5-only or also 5.4/5.3? (probe all three). (5) _VERSION string; any math/string/table roster deltas 5.4->5.5. Write ' + ROOT + '/specs/followup/5.5-stdlib-err.md: exact expected, impl location, clear-cut vs risky, CI assertions (note which need the spawn-the-binary CLI harness vs the in-process wrapper).'],
]
const disc = await parallel(areas.map(function (a) {
  const key = a[0], desc = a[1]
  return function () {
    return agent(CTX + '\n\nDISCOVER (' + key + ') — READ-ONLY (run binaries, read code; do NOT edit). ' + desc + '\n\nEverything reported must be reproduced with diff_one.sh or direct invocation against lua5.5.0 (and cross-checked on 5.4/5.3 where relevant). Return a ~10-line summary: how many divergences, the categories, and the top 5 clear-cut fixes.',
      { label: 'discover:' + key, phase: 'Discover', agentType: 'general-purpose' })
  }
}))

phase('Fix')
const fixOrder = [
  ['global-guard', 'Implement the 5.5 "global already defined" guard (per specs/followup/5.5-lang.md), matching lua5.5.0 exactly (trigger + message). 5.5-gated; 5.4/5.3 unaffected. File: crates/lua-parse/src/lib.rs globalstat.'],
  ['named-varargs', 'Implement 5.5 named varargs `function f(...t)` (per specs/followup/5.5-lang.md) IF it is clear-cut: parser accepts the syntax, codegen binds the extra args into table t. 5.5-gated; on 5.4/5.3 the syntax must still be a parse error matching the reference. If the codegen is architecturally risky, STOP and document precisely what is needed rather than half-implement.'],
  ['stdlib-5.5', 'Fix the clear-cut 5.5 stdlib items (per specs/followup/5.5-stdlib-err.md): utf8.offset 2nd return, collectgarbage("param"/...) surface, _VERSION, and any roster deltas. Match lua5.5.0; do not regress 5.4/5.3. Files in crates/lua-stdlib (+ crates/lua-gc for GC params).'],
  ['error-namewhat', 'Fix the clear-cut 5.5 error/traceback items (per specs/followup/5.5-stdlib-err.md): error(nil)->`<no error object>` if that is the divergence, and the namewhat `[C]: in global \'<fn>\'` rendering (apply to whichever versions the reference shows it; auxlib.rs::push_func_name). Use the spawn-the-binary CLI harness (crates/lua-cli/tests/traceback_oracle.rs) for traceback assertions. Do NOT regress 5.4/5.3 traceback output.'],
]
const fixes = []
for (const entry of fixOrder) {
  const key = entry[0], what = entry[1]
  const r = await agent(CTX + '\n\nFIX (' + key + '). ' + what + '\n\nRead the relevant specs/followup/5.5-*.md first. Implement, add CI assertions (in-process to multiversion_oracle.rs, or spawn-the-binary to traceback_oracle.rs as appropriate — assert the 5.5 behavior AND that 5.4/5.3 are unchanged where relevant), then GATE: cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive (0 failures) ; check.sh for 5.4 and 5.3 and 5.5 (all green) ; reproduce the fixed cases via diff_one.sh or the spawned binaries. If a part is risky/ambiguous/architectural, STOP and document rather than guess. Commit on finish-5.5: git add -A && git commit -m "feat(5.5): ' + key + ' ...". Return what landed, gate results, anything deferred.',
    { label: 'fix:' + key, phase: 'Fix', agentType: 'general-purpose' })
  fixes.push(r)
}

phase('Synthesize')
const report = await agent(CTX + '\n\nSYNTHESIS. Read the discover specs and fix results. Re-run the official 5.5 suite sweep (lua-rs 5.5 vs lua5.5.0) to measure parity AFTER the fixes. Write ' + ROOT + '/specs/followup/PHASE_C_REPORT.md: divergences before vs after (by category), what landed (with gate results), what remains for full 5.5 parity (prioritized), and the updated 5.5 oracle-battery count. Confirm 5.4/5.3 unaffected. Also re-run check.sh for all three versions and the full cargo test to confirm green. Return a ~15-line executive summary including the before/after 5.5 parity numbers and the single most valuable remaining item.',
  { label: 'synthesize', phase: 'Synthesize', agentType: 'general-purpose' })

return { disc, fixes, report }
