export const meta = {
  name: 'hard-problems-batch',
  description: 'Architectural deferrals from Phases B/C/shared-core: fix the table-resize panic (robustness), attempt goto label scoping, __gc finalizer error propagation, and debug line-hook fidelity. Fix what is tractable with oracle gating; document + flag the genuinely-deep for issue filing.',
  phases: [
    { title: 'Fix', detail: 'sequential: attempt each architectural item, fix-or-document, oracle-gated, CI tests' },
    { title: 'Synthesize', detail: 'what landed, what remains genuinely architectural (with re-entry notes to file as issues)' },
  ],
}

const ROOT = '/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port/.claude/worktrees/git-issues'

const CTX = [
  'Repo: ianm199/lua-rs (pure-Rust Lua), branch hard-problems (off main). All 5 versions (5.1-5.5) are now supported and oracle-green. This batch attacks the ARCHITECTURAL deferrals accumulated across the prior phases — the items repeatedly flagged "fix-or-document, do not half-implement". Precise re-entry notes are in ' + ROOT + '/specs/followup/SHARED_CORE_REPORT.md (bottom) and sharedcore-triage-3.md.',
  '',
  'THE ENGINE:',
  '- Oracle = /tmp/lua-refs/bin/lua5.1.5 / lua5.2.4 / lua5.3.6 / lua5.4.7 / lua5.5.0. diff_one.sh <ver> "<lua>" and check.sh <ver> work for all 5 versions.',
  '- Official tests: /tmp/lua-refs/lua-5.3.4-tests/*.lua and lua-5.5.0-tests/*.lua (preamble: _soft=true; _port=true; _nomsg=true; _U=false; arg=arg or {}). Absolute lua-rs path: ' + ROOT + '/target/debug/lua-rs, LUA_RS_VERSION=<v>.',
  '- CI: multiversion_oracle.rs (in-process), traceback_oracle.rs (spawn-the-binary).',
  '- Gate (must stay green, NO version regresses): cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive ; check.sh 5.1/5.2/5.3/5.4/5.5.',
  '- HONESTY RULE: each item is fix-or-document. If an item needs a structural change too large/risky to land cleanly with full oracle verification, STOP, revert any probe edits, and write a precise re-entry note instead of shipping a partial/guessed fix. Report faithfully which you fixed and which you documented.',
  '',
  'THE ITEMS (re-entry notes from SHARED_CORE_REPORT.md):',
  '1. table-resize PANIC (robustness — HIGHEST PRIORITY, a panic is worse than a mismatch). crates/lua-types/src/table.rs:594 panics ("index out of bounds: len 8, index 8") on a downward array resize (nextvar.lua, 5.5): the migration loop iterates new_asize..old_asize indexing self.array[i], but old_asize (from set_limit_to_size) can exceed self.array.len(). Re-entry: re-establish the set_limit_to_size => self.array.len() invariant, or clamp the migration loop to the physical array length, mirroring upstream luaH_resize (migrates over the old PHYSICAL array). LIKELY TRACTABLE.',
  '2. goto label scoping in disjoint/nested blocks (blocks goto.lua on 5.3 AND 5.5). rs label table is too global. Minimal repro: `::l3:: do goto l3; ::l3:: end` — rs incorrectly rejects "label already defined". The rule differs by version: 5.3 uses a DEFERRED findgotos model scanning only the current block (BlockCnt.firstlabel exists); 5.4+ scans the whole function. rs implements 5.4 eager semantics. Genuine parser-scope change in crates/lua-parse goto/label resolution, version-gated; interacts with <close>/goto-over-local. ATTEMPT; if the deferred-model rework is too invasive to verify cleanly, DOCUMENT.',
  '3. __gc finalizer error propagation (gc.lua:360). 5.3 PROPAGATES a finalizer error out of collectgarbage (rs returns ok=true, wrong); 5.4/5.5 CATCH it and emit "Lua warning: error in __gc (...)" to stderr (rs omits entirely). Needs protected finalizer calls with version-specific disposition. Seam: lua-gc finalizer loop + to_be_finalized drain + warn path. ATTEMPT the 5.4/5.5 warn + 5.3 propagate; if the protected-call plumbing across the GC boundary is too deep, DOCUMENT.',
  '4. debug line-hook fidelity (db.lua, multiple versions). sethook(f,"l") line-event timing/attribution: multi-line `if` traces diverge (ref `3,9,3,10,2,3,4,7,11` vs rs `3,10,2,3,4,7,11`). Instruction->line attribution + hook-fire timing rework; correct trace differs per version. Seam: debug.rs trace_exec + proto lineinfo. LIKELY DEEP — investigate, attempt only if localized, else DOCUMENT with a precise re-entry note.',
  '',
  'Also DOCUMENT (do not attempt — confirmed deep, just write/update re-entry notes for issue filing): generational-GC default mode (no generational collector exists in lua-gc); named-vararg t/... aliasing (needs a proto field for the vararg-table register).',
].join('\n')

phase('Fix')
const fixOrder = [
  ['table-resize-panic', 'Item 1: fix the table.rs:594 downward-resize panic. Mirror upstream luaH_resize — clamp the migration loop to the physical array length (or restore the set_limit_to_size => array.len() invariant). Add a regression test reproducing the nextvar.lua case (the exact table-shape that triggered the len-8/index-8 panic) in multiversion_oracle.rs or a lua-types unit test. This MUST be fixed — it is a panic in safe-Rust. Verify no version regresses and that official nextvar.lua advances on 5.5.'],
  ['goto-label-scoping', 'Item 2: goto label scoping. Probe lua5.3.6 and lua5.5.0 for the exact accept/reject behavior of disjoint/nested same-name labels (`::l3:: do goto l3; ::l3:: end`, forward/backward gotos, goto-into-block rejection, goto-over-local-into-scope rejection). Implement the correct per-block label scoping, version-gated where 5.3 vs 5.4+ differ. If the deferred-findgotos rework is too invasive to verify cleanly across all the goto.lua cases, STOP and write a precise re-entry note instead of a partial fix. Be honest about which you did.'],
  ['gc-finalizer-error', 'Item 3: __gc finalizer error propagation. Probe the references: 5.3 propagates the error out of collectgarbage; 5.4/5.5 catch + warn to stderr ("error in __gc metamethod (...)"). Implement protected finalizer calls with the version-specific disposition (propagate on 5.3, warn on 5.4/5.5). Use traceback_oracle.rs (spawn-the-binary) to assert the stderr warn and the 5.3 propagation. If the protected-call-across-GC plumbing is too deep to land cleanly, STOP and document precisely.'],
  ['debug-line-hook', 'Item 4: debug line-hook fidelity. Investigate the line-event timing/attribution divergence (db.lua sethook(f,"l")). If it is a localized fix in debug.rs trace_exec / proto lineinfo that you can verify against the per-version reference traces, implement it. If it requires a hook-fire-timing rework whose correct behavior differs per version and is hard to verify, STOP and write a precise re-entry note. Most likely DOCUMENT — do not guess.'],
]
const fixes = []
for (const entry of fixOrder) {
  const key = entry[0], what = entry[1]
  const r = await agent(CTX + '\n\nFIX (' + key + '). ' + what + '\n\nIf you implement: add CI assertions (multiversion_oracle.rs and/or traceback_oracle.rs), then GATE: cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive (0 failures) ; check.sh 5.1/5.2/5.3/5.4/5.5 (no regression) ; reproduce via diff_one.sh / official test. Commit on hard-problems: git add -A && git commit -m "fix(core): ' + key + ' ...". If you DOCUMENT instead: revert any probe edits (leave the tree clean for the next item), and return a precise re-entry note. Return clearly: FIXED (with gate results) or DOCUMENTED (with re-entry note), and anything deferred.',
    { label: 'fix:' + key, phase: 'Fix', agentType: 'general-purpose' })
  fixes.push(r)
}

phase('Synthesize')
const report = await agent(CTX + '\n\nSYNTHESIS. Read the fix results. Confirm the gate is green on all 5 versions (check.sh 5.1-5.5 + full cargo test). Write ' + ROOT + '/specs/followup/HARD_PROBLEMS_REPORT.md: per item — FIXED (with gate results + which official test advanced) or DOCUMENTED (with a precise, self-contained re-entry note suitable for a GitHub issue). Include the always-documented items (generational GC, named-vararg aliasing). Return a ~12-line executive summary: what landed, what remains architectural (as a clean list of issue-titles + one-line each, so the orchestrator can file them), and confirmation that no version regressed.',
  { label: 'synthesize', phase: 'Synthesize', agentType: 'general-purpose' })

return { fixes, report }
