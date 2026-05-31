export const meta = {
  name: 'shared-core-fidelity-batch',
  description: 'Shared-core fidelity batch: cross-version defects surfaced across Phases B/C. Triage each remaining item (clear-cut vs architectural) against the references, fix the clear-cut ones sequentially with CI tests, document the architectural ones precisely.',
  phases: [
    { title: 'Triage', detail: 'parallel read-only: confirm each remaining item current behavior across 5.3/5.4/5.5 + clear-cut/architectural verdict' },
    { title: 'Fix', detail: 'sequential oracle-gated fixes for the clear-cut items + CI tests' },
    { title: 'Synthesize', detail: 'what landed, what remains architectural, updated parity across all versions' },
  ],
}

const ROOT = '/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port/.claude/worktrees/git-issues'

const CTX = [
  'Repo: ianm199/lua-rs (pure-Rust Lua), branch shared-core-fidelity (off main, v0.0.21+). Goal: close CROSS-VERSION fidelity defects surfaced across Phases B and C. These are NOT version-specific roster items — they are bugs in the shared core that diverge from the reference on 2-3 versions at once. Fixing one improves parity on multiple versions.',
  '',
  'THE ENGINE (follow it; do not re-invent):',
  '- Oracle = the unmodified make-macosx reference binaries in /tmp/lua-refs/bin (lua5.3.6 / lua5.4.7 / lua5.5.0; lua5.1.5 / lua5.2.4 also present). A shared-core change MUST match EVERY affected version reference, not just one.',
  '- Differential oracle: ' + ROOT + '/specs/oracle/diff_one.sh <5.3|5.4|5.5> "<lua>" prints MATCH or a DIFF block.',
  '- Official tests: /tmp/lua-refs/lua-5.3.4-tests/*.lua and /tmp/lua-refs/lua-5.5.0-tests/*.lua (preamble: _soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}). Absolute lua-rs path: ' + ROOT + '/target/debug/lua-rs, version via LUA_RS_VERSION=5.3|5.4|5.5. The CLI runs the main chunk beneath a pmain C frame (tracebacks include the trailing [C]: in ? frame).',
  '- Adversarial-first: derive cases from the upstream manuals / official tests / probing the reference binaries, NOT from our Rust source.',
  '- Prior context: ' + ROOT + '/specs/followup/PHASE_B_REPORT.md and PHASE_C_REPORT.md list these items in their "what remains" sections.',
  '- CI tests: extend ' + ROOT + '/crates/lua-rs-runtime/tests/multiversion_oracle.rs (in-process Lua::new_versioned + load+pcall wrapper) for value/error behavior; ' + ROOT + '/crates/lua-cli/tests/traceback_oracle.rs (spawn-the-binary) for CLI traceback/message behavior.',
  '- Gate (must stay green): cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive ; ' + ROOT + '/specs/oracle/check.sh {5.4,5.3,5.5}. NO version may regress.',
  '',
  'THE ITEMS (confirm each against the references first; some may be partly fixed already):',
  'A. _ENV[<relational-expr>] index codegen bug: `_ENV[1<2]` raises "attempt to index a number value" on 5.5 AND 5.3 (MATCHes on 5.4); plain `t[1<2]` matches everywhere. An upvalue indexed by a register holding a boolean/relational result is mis-lowered. Sole blocker on official closure.lua. LIKELY CLEAR-CUT and localized (codegen in crates/lua-code or crates/lua-parse expr lowering). HIGH PRIORITY.',
  'B. luaL_argerror funcname/value omission (cross-version, 5.3/5.4/5.5): `bad argument #1 (invalid option)` should be `bad argument #1 to \'collectgarbage\' (invalid option \'setpause\')`; also affects utf8.offset and string.format width/precision wording (missing `to \'<fn>\'` and the offending value). In crates/lua-stdlib/src/auxlib.rs (arg_error / push_global_func_name) + the checkoption call sites. Confirm exact reference strings for several callsites.',
  'C. GC default mode: lua-rs defaults to incremental on all versions, but BOTH reference 5.4 and 5.5 default to GENERATIONAL (collectgarbage("incremental")/("generational") and the default mode differ). Confirm exact reference behavior (what collectgarbage(\'isrunning\')/mode queries return by default) across 5.3/5.4/5.5. May be RISKY (real collector behavior) — if so, document precisely rather than fake it.',
  'D. \\u{...} upper bound in the lexer: 5.x accepts up to 0x7FFFFFFF and errors "UTF-8 value too large" above; lua-rs accepts 0x110000+. Blocks literals.lua. Confirm per-version (5.1-5.5) and fix in crates/lua-lex. LIKELY CLEAR-CUT.',
  'E. print must call the global tostring (error if nil): lua-rs formats internally; reference errors "attempt to call a nil value" when tostring is shadowed nil. Blocks calls.lua. In crates/lua-stdlib base print. LIKELY CLEAR-CUT.',
  'F. string.unpack "c0" bounds: `string.unpack("c0", x, 0)` should raise "initial position out of string". Blocks tpack.lua. In crates/lua-stdlib string pack/unpack. LIKELY CLEAR-CUT.',
  'G. __le-from-__lt across a coroutine yield: coroutine.lua:599. #78 already derives __le from __lt on 5.1-5.4; this case is about the derivation surviving a yield boundary. CONFIRM whether it is clear-cut or architectural (coroutine state across the tagmethod call).',
  'H. (architectural candidates — CONFIRM then most likely DOCUMENT, do not half-implement): goto label scoping in disjoint/nested blocks (goto.lua), loop-built-closure equality caching (closure.lua:48), __gc finalizer error propagation (gc.lua:360), debug line-hook fidelity (db.lua), named-vararg t/... aliasing (needs a new proto field). For each, confirm the exact divergence and write a precise re-entry note; only implement if it turns out genuinely localized.',
].join('\n')

phase('Triage')
const triageAreas = [
  ['codegen-lex', 'Triage items A (_ENV[relational] codegen), D (\\u{} upper bound), F (string.unpack c0 bounds). For each: reproduce the exact divergence on every affected version via diff_one.sh / official test, locate the responsible code, and give a clear-cut-vs-architectural verdict with the exact edit seam. Write findings to ' + ROOT + '/specs/followup/sharedcore-triage-1.md.'],
  ['stdlib-err', 'Triage items B (argerror funcname/value omission), E (print->global tostring), C (GC default mode generational). For each: exact reference strings/behavior across 5.3/5.4/5.5, the responsible code, clear-cut-vs-architectural verdict + edit seam. For C specifically, determine if matching the default generational mode is observable/testable without retuning the real collector, and whether it is safe. Write ' + ROOT + '/specs/followup/sharedcore-triage-2.md.'],
  ['coro-arch', 'Triage item G (__le-from-__lt across yield) and the item-H architectural candidates (goto label scoping, loop-closure equality, __gc error propagation, debug line-hook, named-vararg aliasing). For each: exact divergence + affected versions + a clear-cut-vs-architectural verdict. Be honest: which are genuinely localized vs need a structural change. Write ' + ROOT + '/specs/followup/sharedcore-triage-3.md with a precise re-entry note for each architectural one.'],
]
const triage = await parallel(triageAreas.map(function (a) {
  const key = a[0], desc = a[1]
  return function () {
    return agent(CTX + '\n\nTRIAGE (' + key + ') — READ-ONLY (run binaries, read code; do NOT edit). ' + desc + '\n\nEverything must be reproduced against the reference binaries. Return a ~10-line summary: per item, affected versions, clear-cut or architectural, and the edit seam.',
      { label: 'triage:' + key, phase: 'Triage', agentType: 'general-purpose' })
  }
}))

phase('Fix')
const fixOrder = [
  ['env-relational-index', 'Item A: fix the _ENV[<relational-expr>] (and any boolean-result-as-index) codegen bug so `_ENV[1<2]` indexes correctly on 5.5/5.3 (and stays correct on 5.4). Per sharedcore-triage-1.md. This is a real correctness defect — verify the fix on plain tables and _ENV across all versions.'],
  ['lexer-unicode-bound', 'Item D: \\u{...} upper bound = 0x7FFFFFFF, "UTF-8 value too large" above, per sharedcore-triage-1.md. Match every version reference (5.1-5.5 share this).'],
  ['string-unpack-c0', 'Item F: string.unpack "c0" / position bounds raise "initial position out of string" per sharedcore-triage-1.md.'],
  ['argerror-funcname', 'Item B: restore the `to \'<fn>\'` qualifier and the offending value/option in luaL_argerror / checkoption messages, matching the reference across 5.3/5.4/5.5, per sharedcore-triage-2.md. This is cross-version — verify collectgarbage invalid-option, utf8.offset, and string.format width/precision callsites all improve and none regress.'],
  ['print-global-tostring', 'Item E: print must call the global tostring (erroring if it is nil), per sharedcore-triage-2.md. Match the reference message across versions.'],
  ['clearcut-remainder', 'Implement any OTHER items that the triage docs (sharedcore-triage-1/2/3.md) marked CLEAR-CUT and not yet done in this Fix phase (e.g. item C GC default mode IF triage deemed it safe/clear-cut, item G __le-across-yield IF localized). Skip anything triage marked architectural — those are documented only. State explicitly which items you implemented and which you skipped and why.'],
]
const fixes = []
for (const entry of fixOrder) {
  const key = entry[0], what = entry[1]
  const r = await agent(CTX + '\n\nFIX (' + key + '). ' + what + '\n\nRead the relevant sharedcore-triage-*.md first. If triage marked this item architectural/risky, DO NOT force it — document and skip. Otherwise implement, add CI assertions (multiversion_oracle.rs for value/error behavior across the affected versions including a guard that unaffected versions are unchanged; traceback_oracle.rs for CLI messages), then GATE: cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive (0 failures) ; check.sh 5.4 AND 5.3 AND 5.5 (all green, no regression) ; reproduce fixed cases via diff_one.sh. Commit on shared-core-fidelity: git add -A && git commit -m "fix(core): ' + key + ' ...". Return what landed, gate results, anything deferred.',
    { label: 'fix:' + key, phase: 'Fix', agentType: 'general-purpose' })
  fixes.push(r)
}

phase('Synthesize')
const report = await agent(CTX + '\n\nSYNTHESIS. Read the triage docs and fix results. Re-run the official suite sweeps for 5.3 and 5.5 (lua-rs vs the matching reference) to measure parity AFTER the fixes; spot-check 5.4 did not regress. Write ' + ROOT + '/specs/followup/SHARED_CORE_REPORT.md: per item — landed or deferred (with a precise re-entry note for each architectural deferral), gate results, and the before/after byte-identical counts for 5.3 and 5.5. Confirm all three versions green via check.sh + full cargo test. Return a ~15-line executive summary: what landed, what remains architectural, the updated parity numbers, and the single most valuable remaining cross-version item.',
  { label: 'synthesize', phase: 'Synthesize', agentType: 'general-purpose' })

return { triage, fixes, report }
