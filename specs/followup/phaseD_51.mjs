export const meta = {
  name: 'phaseD-5.1-legacy',
  description: 'Phase D part 2: Lua 5.1 (the legacy family) — fenv globals via Option B on the modern core, 5.1 stdlib roster, 5.1 metamethod diffs (__len-on-tables inert), 5.1 syntax gates. Lift the V51 refusal only if it passes the oracle battery + 5.1.5 example scripts vs lua5.1.5 with no regression to 5.2/5.3/5.4/5.5.',
  phases: [
    { title: 'Design', detail: 'read-only: design fenv/getfenv/setfenv (Option B); catalogue roster + syntax + metamethod diffs + PRNG vs lua5.1.5' },
    { title: 'Implement', detail: 'sequential oracle-gated: fenv globals, syntax gates, roster, metamethod diffs, lift V51' },
    { title: 'Verify', detail: 'read-only: 5.1 battery + 5.1.5 example scripts vs lua5.1.5; confirm 5.2-5.5 unregressed; honest verdict' },
  ],
}

const ROOT = '/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port/.claude/worktrees/git-issues'

const CTX = [
  'Repo: ianm199/lua-rs (pure-Rust Lua), branch finish-5.1 (off main). Goal: add Lua 5.1 support — the LEGACY family. 5.2 is ALREADY DONE and merged (float-only numbers on the modern _ENV core, V52 supported). 5.1 = 5.2 MINUS _ENV PLUS fenv globals, MINUS goto, PLUS the 5.1 stdlib roster, PLUS the 5.1 metamethod differences. The float-only number model is shared with 5.2 and already works under V51 (number_model()==FloatOnly) — reuse it.',
  '',
  'AUTHORITATIVE SPEC: ' + ROOT + '/specs/LUA_5_1_PORT_SPEC.md (read §2.1-§2.4 fully) and ' + ROOT + '/specs/research/5.1-5.2-upstream.md. Oracle pin: /tmp/lua-refs/bin/lua5.1.5.',
  '',
  'THE HARD AXIS — fenv globals (§2.2). Use OPTION B (lower-at-load into table access, reuse the modern _ENV machinery): a per-closure environment that the lowered free-name table-access reads, plus getfenv/setfenv mutating it. The modern parser already threads _ENV as an upvalue and resolves free names via GETTABUP/SETTABUP — under V51, that _ENV upvalue IS the closure environment, and getfenv/setfenv read/write it. Required observable surface (match lua5.1.5): getfenv([f|level]) returns env of function/level (running fn if 0/absent); setfenv(f,t) sets env of a Lua function or stack level; setfenv(0,t) sets the running thread env; a closure can have a DIFFERENT env than _G. C-function environments (LUA_ENVIRONINDEX) may be a DOCUMENTED GAP if no consumer needs it — note it, do not silently fake it. Do NOT build a whole second VM ISA.',
  '',
  'METAMETHOD DIFFS (§2.4) — the silent-failure traps: (1) __len on TABLES does nothing in 5.1 — `#t` uses primitive length, never consults __len; the # dispatch must branch on version. THIS IS THE #1 SOURCE OF SILENT 5.1 FAILURES. (2) no __pairs/__ipairs. (3) no __gc on tables (userdata only). (4) __lt-derives-__le fallback KEPT (already gated for 5.1-5.4). (5) arithmetic always float-dispatched (shared with 5.2).',
  '',
  'ROSTER (§2.3): loadstring present + load takes a reader fn only; unpack as a GLOBAL (table.unpack ABSENT); table.getn/setn/maxn/foreach/foreachi present; module/package.seeall/package.loaders present; string.gfind alias; math.log 1-arg (no base), math.log10/atan2/pow present, NO math.type; gcinfo(); newproxy([bool|proxy]) (userdata-with-metatable idiom for __gc/__len); arg table; xpcall(f,h) CANNOT take extra args; coroutine.running returns nil in main; bit32 ABSENT (5.2-only!); _VERSION="Lua 5.1". SYNTAX absent: goto/labels, //, bitwise, string.pack, utf8, <const>/<close>, \\x/\\z/hex-float, p-exponent. Hex INTEGER literals (0x1F) present (lower to f64).',
  '',
  'PRNG NOTE: 5.1 math.random/randomseed use C rand() — the exact byte sequence is host/platform-dependent and CANNOT be portably bit-matched in Rust. Treat math.random SEQUENCE divergence as a KNOWN DOCUMENTED divergence (like the existing math.lua random-seed line), NOT a gate failure. What MUST match: math.random()  in [0,1), math.random(n) in [1,n] integer-valued-float, argument errors, randomseed accepting a number. Do not reuse 5.4 xoshiro claims of parity; just ensure the contract/range is right.',
  '',
  'THE ENGINE:',
  '- Oracle = /tmp/lua-refs/bin/lua5.1.5 (also lua5.2.4 etc for no-regression). The official 5.1 conformance suite is NOT bundled; the oracle is a hand-built battery + the example scripts in /tmp/lua-refs/lua-5.1.5/test/*.lua (many are real 5.1 programs: life.lua, sort.lua, fib.lua, env.lua, trace-globals.lua, etc.) + adversarial probing from the 5.1 manual (https://www.lua.org/manual/5.1/). diff_one.sh/check.sh already accept 5.1 (added in the 5.2 phase).',
  '- The seam: LuaVersion (crates/lua-types/src/version.rs) has V51 with FloatOnly; is_supported() currently EXCLUDES V51. Lifting = add to is_supported() + the runtime new_versioned guard (crates/lua-rs-runtime/src/lib.rs) + the CLI LUA_RS_VERSION parse.',
  '- CI: extend ' + ROOT + '/crates/lua-rs-runtime/tests/multiversion_oracle.rs with v51_* tests (Lua::new_versioned(LuaVersion::V51) + the load+pcall wrapper). Capture expected values from lua5.1.5.',
  '- Gate: cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive ; ' + ROOT + '/specs/oracle/check.sh {5.2,5.3,5.4,5.5} (no regression) + a new check.sh 5.1 battery. Modern + 5.2 must not regress (all V51 behavior is V51-gated).',
  '- HONESTY RULE: lift V51 ONLY if the battery passes broadly AND the example scripts run faithfully. fenv and __len-on-tables are the highest-risk; if a sub-area cannot reach parity cleanly, leave V51 refused OR mark 5.1 alpha and document the EXACT gaps. Do not ship a 5.1 that masquerades as working. Report faithfully — RNG-sequence divergence is the one allowed documented exception.',
].join('\n')

phase('Design')
const designAreas = [
  ['fenv', 'FENV GLOBALS (§2.2), the hard axis. Probe lua5.1.5 exhaustively: getfenv(0)/getfenv(1)/getfenv(f)/getfenv() return values; setfenv(f,t) then call f and confirm free names resolve in t; setfenv(0,t) effect on the running chunk; a closure with a non-_G env; the error cases (setfenv on a C function, bad level). Read how the modern parser threads _ENV (crates/lua-parse/src/lib.rs envn ~L408, GETTABUP/SETTABUP) and how a closure stores upvalues, so getfenv/setfenv can read/write the _ENV upvalue under V51. Decide the EXACT Option-B mechanism (where the per-closure env lives, how getfenv/setfenv reach it, what LUA_ENVIRONINDEX gap to document). Write ' + ROOT + '/specs/followup/5.1-fenv.md: the mechanism, the impl seams, the observable test cases, and any documented gap.'],
  ['roster_syntax', 'ROSTER + SYNTAX + METAMETHODS (§2.3, §2.4). Probe lua5.1.5 for the full present/absent roster (loadstring, global unpack, table.getn/foreach/maxn, module, string.gfind, math.log 1-arg, math.log10/atan2/pow, gcinfo, newproxy, arg, xpcall-no-extra-args, coroutine.running-nil-in-main, bit32 ABSENT, _VERSION) and the syntax that must be REJECTED (goto, //, bitwise, <const>/<close>, \\x/\\z escapes, p-exponent, string.pack) with exact messages. CRITICAL: confirm __len-on-tables is INERT on 5.1 (`setmetatable({},{__len=function() return 99 end})` then `#t` → primitive length, not 99) and that lua-rs currently consults __len (so it WILL diverge). Write ' + ROOT + '/specs/followup/5.1-roster-syntax.md: present/absent lists, messages, the __len/__pairs/__gc metamethod gates, impl seams (init.rs roster gate, parser syntax gates, the # dispatch in the VM).'],
  ['numbers_prng', 'NUMBER EDGES + PRNG + misc. Confirm the float-only behavior already works under V51 (reuse the 5.2 work — spot-check .0 suppression, float-only arith, %d truncation, math.floor returns float, hex-int literal). Probe lua5.1.5 for: math.random/randomseed CONTRACT (ranges, arg errors — NOT the exact sequence), tostring of numbers, the arg table shape, xpcall arity error, load-vs-loadstring behavior. Write ' + ROOT + '/specs/followup/5.1-numbers-prng.md: what is already correct under V51 (inherited from 5.2), what is 5.1-specific, the PRNG contract (and the documented sequence-divergence), impl seams.'],
]
const design = await parallel(designAreas.map(function (a) {
  const key = a[0], desc = a[1]
  return function () {
    return agent(CTX + '\n\nDESIGN (' + key + ') — READ-ONLY (run binaries, read code; do NOT edit source — a scratch is_supported tweak you REVERT is ok for probing). ' + desc + '\n\nEverything reproduced against lua5.1.5. State whether Option B / gate-based approach is sufficient for your area or whether any case forces a deeper change. Return a ~10-line summary + top fixes.',
      { label: 'design:' + key, phase: 'Design', agentType: 'general-purpose' })
  }
}))

phase('Implement')
const fixOrder = [
  ['fenv-globals', 'Implement 5.1 fenv globals via Option B (per specs/followup/5.1-fenv.md): getfenv/setfenv reading/writing the per-closure _ENV upvalue under V51, with the observable surface matching lua5.1.5 (getfenv levels, setfenv(0,t), distinct closure env). Document the LUA_ENVIRONINDEX/C-function-env gap explicitly. V52-V55 unaffected (they have no getfenv/setfenv).'],
  ['metamethod-diffs', 'Implement the 5.1 metamethod gates (per specs/followup/5.1-roster-syntax.md §2.4): __len on TABLES inert under V51 (#t = primitive length, never consult __len — branch the # dispatch in the VM on version); no __pairs/__ipairs dispatch under V51; no __gc on tables under V51. THE __len ONE IS CRITICAL. 5.2-5.5 keep current behavior. newproxy userdata still honors __len/__gc.'],
  ['syntax-roster', 'Implement the 5.1 syntax gates (reject goto/labels, //, bitwise, <const>/<close>, \\x/\\z escapes, p-exponent, string.pack — match lua5.1.5 messages) and the 5.1 stdlib roster (per specs/followup/5.1-roster-syntax.md): loadstring + reader-only load, global unpack (table.unpack absent), table.getn/setn/maxn/foreach/foreachi, module/package.seeall/package.loaders, string.gfind, math.log 1-arg + log10/atan2/pow (no math.type), gcinfo, newproxy, arg table, xpcall-no-extra-args, coroutine.running-nil-in-main, bit32 ABSENT, _VERSION="Lua 5.1". Mirror the existing version roster-gate pattern. 5.2-5.5 unchanged.'],
  ['prng-misc', 'Implement the 5.1 math.random/randomseed CONTRACT (ranges/arg-errors per specs/followup/5.1-numbers-prng.md) — do NOT attempt to bit-match the C rand() sequence (document that divergence). Handle any remaining 5.1-specific number/misc items. Then verify the float-only behavior inherited from 5.2 holds under V51.'],
  ['lift-v51', 'IF the prior steps pass the battery and example scripts: lift the V51 refusal — add V51 to is_supported(), the runtime guards, and CLI LUA_RS_VERSION=5.1. Run a spread of /tmp/lua-refs/lua-5.1.5/test/*.lua under LUA_RS_VERSION=5.1 vs lua5.1.5 and diff. If the battery does NOT broadly pass, DO NOT lift — document the gaps and leave V51 refused/alpha. State clearly which you did and why.'],
]
const fixes = []
for (const entry of fixOrder) {
  const key = entry[0], what = entry[1]
  const r = await agent(CTX + '\n\nIMPLEMENT (' + key + '). ' + what + '\n\nRead the relevant specs/followup/5.1-*.md first. Add CI assertions to crates/lua-rs-runtime/tests/multiversion_oracle.rs (v51_* behavior + guards that 5.2-5.5 are unchanged), then GATE: cargo build --workspace ; cargo test --workspace --features lua-rs-runtime/derive (0 failures) ; check.sh 5.2 AND 5.3 AND 5.4 AND 5.5 (no regression) AND check.sh 5.1 (battery) ; reproduce fixed cases via diff_one.sh 5.1 vs lua5.1.5. If risky/ambiguous, STOP and document rather than guess. Commit on finish-5.1: git add -A && git commit -m "feat(5.1): ' + key + ' ...". Return what landed, gate results, anything deferred.',
    { label: 'impl:' + key, phase: 'Implement', agentType: 'general-purpose' })
  fixes.push(r)
}

phase('Verify')
const verify = await agent(CTX + '\n\nVERIFY (READ-ONLY: run binaries/tests, read code; do NOT edit). Independently judge the 5.1 backend:\n' +
  '1. Run the full check.sh 5.1 battery; report pass/fail vs lua5.1.5.\n' +
  '2. Run a spread of cases via diff_one.sh 5.1: fenv (getfenv/setfenv/distinct-env), __len-on-tables inert, roster present/absent, rejected syntax, float-only numbers, _VERSION, xpcall arity, global unpack. MATCH/DIFF per case.\n' +
  '3. Run the /tmp/lua-refs/lua-5.1.5/test/*.lua example scripts under LUA_RS_VERSION=5.1 vs lua5.1.5 (exclude RNG-dependent output). Report MATCH/DIFF; RNG-sequence diffs are allowed-documented, everything else is a real DIFF.\n' +
  '4. Confirm NO regression: check.sh 5.2/5.3/5.4/5.5 + full cargo test.\n' +
  'Return a verdict table + a one-line PASS/FAIL on "Lua 5.1 is faithful enough to mark supported, with no other-version regression, RNG-sequence excepted". Be honest — if V51 was lifted but cases DIFF, list them; if correctly left refused/alpha, confirm the gaps are real. Write ' + ROOT + '/specs/followup/PHASE_D_5.1_REPORT.md with the before/after and verdict.',
  { label: 'verify:5.1', phase: 'Verify', agentType: 'general-purpose' })

return { design, fixes, verify }
