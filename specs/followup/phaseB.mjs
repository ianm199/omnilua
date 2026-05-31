export const meta = {
  name: 'mv-phaseB-finish-5.3',
  description: 'Phase B: finish Lua 5.3 (#19). Discover all current 5.3 divergences vs lua5.3.6, fix the clear-cut categories (compat-math, string coercion in bitwise, error wording), add CI tests, drive official-5.3 slices toward parity.',
  phases: [
    { title: 'Discover', detail: 'parallel read-only cataloguing of 5.3 divergences: official-suite slices + compat-math + bitwise-coercion/error-wording' },
    { title: 'Fix', detail: 'sequential oracle-gated fixes by category + CI tests' },
    { title: 'Synthesize', detail: '5.3 parity report: before/after divergences, what remains' },
  ],
}

const ROOT = '/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port/.claude/worktrees/git-issues'

const CTX = [
  'Repo: ianm199/lua-rs (pure-Rust Lua), branch mv-5.3-finish (off main, v0.0.20). Goal: finish Lua 5.3 (issue #19). 5.3 currently runs on the shared modern core with a few version-gated deltas; this phase closes the long tail toward parity with lua5.3.6.',
  '',
  'THE ENGINE (follow it; do not re-invent):',
  '- Oracle = the unmodified make-macosx reference binaries in /tmp/lua-refs/bin (esp. lua5.3.6; also lua5.4.7 and lua5.5.0 to confirm no cross-version regression). Contract pinned in ' + ROOT + '/specs/oracle/CONTRACT.md (default builds are binding; LUA_COMPAT_MATHLIB is ON in 5.3, so compat-math IS part of the 5.3 contract).',
  '- Differential oracle: ' + ROOT + '/specs/oracle/diff_one.sh <5.3|5.4|5.5> "<lua>" prints MATCH or a DIFF block.',
  '- Official 5.3 tests: /tmp/lua-refs/lua-5.3.4-tests/*.lua (run with preamble: _soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}). Use an ABSOLUTE path to lua-rs since you cd into the test dir: ' + ROOT + '/target/debug/lua-rs, selected via LUA_RS_VERSION=5.3.',
  '- Adversarial-first: derive cases from the upstream 5.3 manual / official tests / probing lua5.3.6, NOT from our Rust source.',
  '- Research already done: ' + ROOT + '/specs/research/5.3-upstream-delta.md and ' + ROOT + '/specs/LUA_5_3_AND_5_5_PORT_SPEC.md.',
  '- CI tests: extend ' + ROOT + '/crates/lua-rs-runtime/tests/multiversion_oracle.rs (Lua::new_versioned + the load+pcall wrapper already there).',
  '- Gate (must stay green; a shared-core change must match EVERY version reference, not just 5.3): cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive ; ' + ROOT + '/specs/oracle/check.sh {5.4,5.3,5.5}. 5.4 must not regress.',
].join('\n')

phase('Discover')
const areas = [
  ['suite', 'OFFICIAL 5.3 SUITE SWEEP. Run each /tmp/lua-refs/lua-5.3.4-tests/*.lua through LUA_RS_VERSION=5.3 lua-rs vs lua5.3.6 (with the preamble; cd into the test dir; absolute lua-rs path). For each file report: byte-identical, or the FIRST divergence (file:line + ours vs ref). Categorize every divergence (compat-math, string-coercion-in-bitwise, error-wording, number-model, stdlib-gap, other). Focus on files exercising implemented surface (numbers, strings, bitwise, math, vararg, locals, constructs, closures, events). Write ' + ROOT + '/specs/followup/5.3-divergences.md with a categorized table and a parity estimate.'],
  ['math', 'COMPAT-MATH + math edges. 5.3 ships (LUA_COMPAT_MATHLIB on) math.atan2/cosh/sinh/tanh/pow/log10/ldexp/frexp and one-arg math.log with a base; confirm which are missing/wrong vs lua5.3.6 and the exact expected values/signatures. Also probe math.fmod/modf/huge edge behaviors. Write ' + ROOT + '/specs/followup/5.3-math.md: exact missing entries, expected values, the file to edit (crates/lua-stdlib/src/math_lib.rs — mirror the existing bit32/warn roster gate), and CI assertions.'],
  ['coerce_err', 'STRING-IN-BITWISE COERCION + ERROR WORDING. (1) 5.3 coerces numeric strings in core bitwise ops ("3" & 5 -> 1, "0xff"|0, ~"5", "8">>"1"); we error. Pinpoint the path (the bitwise metamethods / lua-vm bitwise; note the arith string->float coercion already added in crates/lua-stdlib/src/string_lib.rs). (2) 5.3-specific error wording where it differs from 5.4 (e.g. arithmetic-on-non-coercible-string says "attempt to perform arithmetic on a string value" in 5.3 vs the 5.4 metamethod-style message). Probe lua5.3.6 for exact strings. Write ' + ROOT + '/specs/followup/5.3-coerce-err.md: repros, expected, impl location, clear-cut vs risky, CI assertions.'],
]
const disc = await parallel(areas.map(function (a) {
  const key = a[0], desc = a[1]
  return function () {
    return agent(CTX + '\n\nDISCOVER (' + key + ') — READ-ONLY (run binaries, read code; do NOT edit). ' + desc + '\n\nEverything reported must be reproduced with diff_one.sh or direct invocation against lua5.3.6. Return a ~10-line summary: how many divergences, the categories, and the top 5 clear-cut fixes.',
      { label: 'discover:' + key, phase: 'Discover', agentType: 'general-purpose' })
  }
}))

phase('Fix')
const fixOrder = [
  ['compat-math', 'Add the 5.3 compat-math family (per specs/followup/5.3-math.md): the missing math.* functions, version-gated to 5.1/5.2/5.3 (LUA_COMPAT_MATHLIB), in crates/lua-stdlib/src/math_lib.rs mirroring the existing roster gate. Match lua5.3.6 values exactly. Must remain ABSENT under 5.4/5.5.'],
  ['bitwise-coercion', 'Make 5.3 coerce numeric strings in core bitwise ops (per specs/followup/5.3-coerce-err.md), matching lua5.3.6, while 5.4/5.5 keep erroring. Reuse/extend the version-gated string coercion already in the codebase. Watch bit32 (32-bit) vs native 64-bit operator semantics.'],
  ['error-wording', 'Fix the CLEAR-CUT 5.3-specific error wording differences (per specs/followup/5.3-coerce-err.md and 5.3-divergences.md). Do the safe ones (match lua5.3.6 exactly); leave anything risky/architectural documented. Do NOT regress 5.4/5.5 wording.'],
]
const fixes = []
for (const entry of fixOrder) {
  const key = entry[0], what = entry[1]
  const r = await agent(CTX + '\n\nFIX (' + key + '). ' + what + '\n\nRead the relevant specs/followup/5.3-*.md first. Implement, add CI assertions to crates/lua-rs-runtime/tests/multiversion_oracle.rs (assert the 5.3 behavior AND that 5.4/5.5 are unchanged where relevant), then GATE: cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive (0 failures) ; check.sh for 5.4 and 5.3 and 5.5 (all green) ; reproduce the fixed cases via diff_one.sh. If a part is risky/ambiguous, STOP and document rather than guess. Commit on mv-5.3-finish: git add -A && git commit -m "feat(5.3): ' + key + ' ...". Return what landed, gate results, anything deferred.',
    { label: 'fix:' + key, phase: 'Fix', agentType: 'general-purpose' })
  fixes.push(r)
}

phase('Synthesize')
const report = await agent(CTX + '\n\nSYNTHESIS. Read the discover specs and fix results. Re-run the official 5.3 suite sweep (lua-rs 5.3 vs lua5.3.6) to measure parity AFTER the fixes. Write ' + ROOT + '/specs/followup/PHASE_B_REPORT.md: divergences before vs after (by category), what landed (with gate results), what remains for full 5.3 parity (prioritized), and the updated 5.3 oracle-battery count. Confirm 5.4/5.5 unaffected. Return a ~15-line executive summary including the before/after 5.3 parity numbers and the single most valuable remaining item.',
  { label: 'synthesize', phase: 'Synthesize', agentType: 'general-purpose' })

return { disc, fixes, report }
