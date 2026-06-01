# Repo-strengthening ideas (from the 2026-06-01 issue-triage pass)

Captured while triaging #92–#97 + #20. Nothing here is implemented — these are
proposals. Items are tagged **[significant]** (wants your sign-off) or
**[cheap]** (safe to land on request). Ordered by leverage.

---

## 1. [significant] Pin all five version reference binaries as a real oracle

**The gap.** Only `reference/lua-5.4.7/` is checked into the repo. Yet every
version-specific issue (#92/#95/#96/#97 and 5.5 work) is really a *cross-version
behavioral diff*, and the only way to settle "what does the reference actually
do on 5.1/5.2/5.3/5.5" today is the binaries sitting in `/tmp/lua-refs/bin/`
(`lua5.1.5 lua5.2.4 lua5.3.6 lua5.4.7 lua5.5.0`). `/tmp` is ephemeral — those
vanish on reboot, and every triage agent had to rediscover/rebuild them.

**Proposal.** Pin 5.1.5 / 5.2.4 / 5.3.6 / 5.5.0-work the way 5.4.7 is pinned:
record commits + build commands in `harness/source.toml`, build into
`reference/lua-5.x/`, and extend `parity_check.sh` so `REF` can select the
version matching `LUA_RS_VERSION`. That turns "I think the reference does X" into
a runnable multi-version behavioral oracle — the single biggest force-multiplier
for the open version-gate issues.

**Why significant.** Adds vendored upstream source + build steps + CI weight,
and a 5.5 *work* release isn't frozen (see #20 — must pin one snapshot). Wants a
deliberate call on which 5.5 work version to pin.

---

## 2. [cheap] `harness/ver_diff.sh` — one-shot cross-version snippet differ

Every triage agent hand-rolled the same loop: run a snippet under each
`LUA_RS_VERSION` and eyeball it against each reference binary. Make it a tool
(the "custom subsystem tester" discipline from the root CLAUDE.md, applied to
version parity):

```bash
# usage: ver_diff.sh '<lua snippet>'   [versions...]
# prints, per version: lua-rs output | reference output | MATCH/DIFF
```

Resolve references from `reference/lua-5.x/` once item #1 lands; until then fall
back to `${LUA_REFS:-/tmp/lua-refs/bin}`. Pure read-only, ~30 lines, no risk.
This is the fast inner loop for all five version-gate issues.

---

## 3. [cheap] Regression tests to add to `crates/lua-rs-runtime/tests/multiversion_oracle.rs`

All five issues have a clean, minimal repro. These should become pinned tests.
**Caveat:** they encode currently-*failing* behavior, so land each either (a)
together with its fix, or (b) now as `#[ignore = "tracks #NN"]` to pin intent.
Recommend (b) for #97 specifically — a high-priority silent-wrong-answer bug
deserves a committed, named guard even before the fix.

- **#97** (5.1–5.4): derived `__le` survives a yield in `__lt`.
  `mt={__lt=function(a,b) coroutine.yield(10); return a.val<b.val end}`; assert
  `a<=b == true`, `b<=a == false` driven through `coroutine.wrap`.
- **#96** (5.3 vs 5.4): closures built in a loop over one shared upvalue compare
  `==` on 5.3, `~=` on 5.4/5.5.
- **#95** (all versions): `-e 'break'` error wording per version (table in the
  issue comment). Good first-issue companion test.
- **#94** (5.5): `function f(...t) t[1]=99; return ... end; f(1,2,3)` → `99 2 3`.
- **#92** (5.1–5.3 vs 5.4+): line-hook sequence for the *single-line*
  `for i=1,4 do a=1 end` — 5.1–5.3 fire one extra event per back-edge.
  (NB: needs the single-line body; a multi-line body does not exercise the rule —
  this is the gap noted in the #92 triage comment.)

---

## 3b. [significant] GC parity is softer than the green checkmark — see #104

The 2026-06-01 GC-faithfulness audit found that `gc.lua`'s behavioral MATCH rests
on **simulated** memory observables, not real accounting: `api.rs:2073` halves
`totalbytes` after a cycle and `api.rs:2004` refills it to a 32 KB baseline,
both explicitly to make `gcinfo() < x` assertions hold. The collector itself is a
real tri-color mark-and-sweep, but write barriers are no-ops, finalizers run
outside the collector, and generational mode is a flag with no engine. **Do not
treat gc.lua's MATCH as evidence the GC is faithful.** Tracked as #104 (real byte
accounting), which is a prerequisite for #93 (generational). This is the clearest
case found of the "output matched but mechanism didn't" failure mode the harness
philosophy warns about.

## 4. [cheap] Label taxonomy (done this pass — recorded for continuity)

Created `5.1 5.2 5.3 5.4 5.5`, `priority: high|medium|low`, `architectural`.
Applied across #20, #92–#97. If you dislike the scheme it's trivially reversible
(`gh label delete`). Future version-specific issues should carry a version tag +
a priority by default.

---

## Triage outcome summary (2026-06-01)

| # | Title (short) | Verified state | Labels |
|---|---|---|---|
| 97 | `__le`-from-`__lt` across yield | **STILL-OPEN, confirmed bug** (inverted result; the "now MATCHes" claim was stale) | bug, 5.1–5.4, priority: high |
| 96 | 5.3 loop-closure `==` caching | STILL-OPEN (5.3 only) | bug, 5.3, priority: medium |
| 95 | `break` outside-loop wording | STILL-OPEN (4/5 versions diverge) | enhancement, good first issue, 5.1/5.2/5.3/5.5, priority: low |
| 94 | 5.5 `...t` shared storage | STILL-OPEN, partial (parse ok, aliasing missing) | enhancement, 5.5, priority: medium |
| 92 | line-hook back-edge / TEST-JMP | STILL-OPEN, documented-deferred (single-line repro unconfirmed) | bug, architectural, 5.1/5.2/5.3/5.5, priority: medium |
| 93 | generational GC + default mode | STILL-OPEN, architectural (no gen collector exists) | enhancement, architectural, 5.4/5.5, priority: medium |
| 20 | Support Lua 5.5 (tracker) | OPEN epic, partially underway | enhancement, architectural, 5.5, priority: low |
| 90 | Windows compile | left untouched — owner working it | — |

**Headline:** nothing was already-fixed; #97 (the "maybe already closed" one) is
the most urgent — a silent wrong answer on a core VM path with a small, known fix.
