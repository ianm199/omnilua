# Hard-Problems Report

Branch: `hard-problems` (off `main`). Oracle: the unmodified reference binaries
in `/tmp/lua-refs/bin` (`lua5.1.5` / `lua5.2.4` / `lua5.3.6` / `lua5.4.7` /
`lua5.5.0`). Every expected value below was captured from a reference binary via
`specs/oracle/diff_one.sh` / `check.sh`. This batch attacks the architectural
deferrals accumulated across prior phases. Honesty rule observed: each item is
either FIXED (with gate evidence) or DOCUMENTED (with a self-contained re-entry
note); nothing is half-implemented or guessed.

---

## Gate results (all green, no version regressed)

| Battery | Result |
|---|---|
| `cargo build --workspace` | green |
| `cargo test --workspace --features lua-rs-runtime/derive` | 43 test binaries, **0 failures** |
| `multiversion_oracle.rs` | **111 passed**, 0 failed (was 110) |
| `traceback_oracle.rs` | **13 passed**, 0 failed (was 11) |
| `check.sh 5.1` | 55 passed, 0 failed |
| `check.sh 5.2` | 54 passed, 0 failed |
| `check.sh 5.3` | 23 passed, 0 failed |
| `check.sh 5.4` | 7 passed, 0 failed |
| `check.sh 5.5` | 10 passed, 0 failed |

---

## Item 1 — table-resize PANIC (robustness, highest priority) — **FIXED (already landed)**

The downward-array-resize panic (`crates/lua-types/src/table.rs:594`,
`index out of bounds: len 8, index 8`) was already fixed on this branch in
commit `738c8ea` ("fix(core): table-resize-panic — clamp downward-resize
migration to physical array length"). The migration loop now clamps to the
physical array length:

```rust
let migrate_end = (old_asize as usize).min(self.array.len());
```

**Evidence.** `nextvar.lua` (5.5) no longer panics — it runs cleanly through the
table-resize stress block and now stops at an unrelated assertion (`nextvar.lua:919`,
a `<close>` to-be-closed-variable issue, distinct from the resize panic). The
reference reaches `OK`; lua-rs reaches line 919. The robustness goal (a panic is
worse than a mismatch) is satisfied: there is no panic on any version.

**Residual (out of scope, separate issue):** `nextvar.lua:919 assert(closed)` —
a `<close>` finalization-ordering matter unrelated to array resizing.

---

## Item 2 — goto label scoping in disjoint/nested blocks — **FIXED**

Two parts: (a) the label-scope rule, and (b) the error-message location prefix.

### (a) Label scope rule (already landed, verified)
Version-gated label scoping was already implemented in
`crates/lua-parse/src/lib.rs` (`checkrepeated`, `findlabel_for_goto`,
`movegotosout`, all gated on `V52|V53` to scan the *current block* via
`bl.firstlabel`; 5.4/5.5 keep the function-wide scan). Verified:

- `::l3:: print("a"); do goto l3; ::l3:: end; print("ok")` →
  ACCEPTED on 5.2/5.3, REJECTED on 5.4/5.5 — matches all references.

### (b) Error-message location prefix (fixed this pass)
The two goto/label errors (`label '<n>' already defined on line N` and
`no visible label '<n>' for <goto> at line N`) were built with a bare
`LuaError::syntax(format_args!(...))`, which omitted the `chunkname:line:`
location that upstream's `luaK_semerror` → `luaX_syntaxerror` prepends. The
reference emits e.g. `(command line):1: label 'l3' already defined on line 1`;
lua-rs emitted only the body.

**Fix.** Routed both messages through `lua_lex::lex_error(&mut ls.lex, msg, 0)`
(the semantic-error path that prepends `chunk_id(source):linenumber:`, with
token `0` so no `near` suffix). Changed `checkrepeated` and `undef_goto`
signatures to `&mut LexState`. Files: `crates/lua-parse/src/lib.rs`
(`undef_goto`, `checkrepeated`). Verified across all versions and for multi-line
file inputs (prefix line = current parse line, matching the reference).

**Evidence.**
- `diff_one.sh 5.4 'goto nowhere'` → MATCH (was DIFF: missing prefix).
- `diff_one.sh {5.4,5.5} '::l3:: ...; do goto l3; ::l3:: end'` → MATCH.
- `goto.lua` (5.3) advances past the label-scoping blocker; now fails later at
  line 180 (closure upvalue-id caching, a separate documented item). 5.5 fails
  at 427 (`global<const>` assignability, separate). The label blocker is gone.

**CI guard.** `multiversion_oracle.rs::goto_label_errors_carry_chunkname_line_prefix`
asserts both messages carry a `<chunk>:<line>:` prefix on 5.2–5.5.

**Found-but-not-fixed (separate item, NOT in scope):** `break`-outside-loop
*wording* diverges by version — 5.2/5.3 `<break> at line N not inside a loop`,
5.4 `break outside loop at line N` (rs matches), 5.1 `no loop to break near
'end'`, 5.5 `break outside loop near 'break'`. The prefix fix improved all of
these (the location is now present), but the body wording and the token-vs-line
form (5.1/5.5 use `near '<tok>'`, raised eagerly in `breakstat`, not deferred to
goto-resolution) is a distinct multi-version item. Pre-existing; not gated by
`check.sh`. See issue-title list below.

---

## Item 3 — `__gc` finalizer error propagation + warning — **FIXED**

Two halves; one was already landed, the other was wired this pass.

### 5.2/5.3 propagation (already landed, verified)
Commit `d879604` already wraps an erroring `__gc` as
`error in __gc metamethod (<msg>)` and re-throws it out of `collectgarbage` on
5.2/5.3, parking it in `gc_finalizer_error` for the `collectgarbage` builtin to
re-raise (`crates/lua-vm/src/api.rs` `run_pending_finalizers_inner` +
`crates/lua-stdlib/src/state_stub.rs:1154`). Verified `pcall(collectgarbage)`
returns `false` with the wrapped message on 5.2/5.3, matching the reference.

### 5.4/5.5 warning emission (fixed this pass — root cause: warn subsystem unwired)
The 5.4/5.5 disposition (`luaE_warnerror` → `Lua warning: error in __gc (...)`
on stderr, `pcall` still returns ok) emitted **nothing**. Root cause was broader
than `__gc`: the entire warning subsystem was unwired — `GlobalState::warnf`
defaulted to `None` and the default `warnfoff`/`warnfon`/`warnfcont` chain
(upstream `lauxlib.c`) was never installed, so even the plain `warn()` builtin
printed nothing.

**Fix.** Ported the default warning state machine into
`crates/lua-vm/src/state.rs`:
- Added `WarnMode { Off, On, Cont }` and a `GlobalState::warn_mode` field
  (default `Off`, matching post-`luaL_openlibs`).
- Rewrote `warning(state, msg, tocont)` so that, when no custom `warnf` is
  installed, it runs `default_warn`: a faithful port of `checkcontrol` (`@on`/
  `@off`) + `warnfon`/`warnfcont` — prints `Lua warning: ` to begin a message,
  appends parts, finishes with `\n`, and threads on/off/continuation state
  through `warn_mode`.

**Evidence.**
- `warn("@on"); warn("hello"); warn("a","b","c"); warn("@off"); warn("nope")` →
  stderr matches the reference byte-for-byte on 5.4/5.5 (`Lua warning: hello`,
  `Lua warning: abc`, nothing after `@off`).
- `/tmp/gcwarn.lua` (`warn("@on")` + erroring `__gc` + `pcall(collectgarbage)`)
  → `Lua warning: error in __gc (...:2: @boom@)` on stderr, `pcall` returns ok —
  matches reference on 5.4/5.5.
- Without `@on`, nothing prints (warnings default off) — matches reference.

**CI guard.** `traceback_oracle.rs::warn_subsystem_via_cli` drives the full
`@on`/parts/`@off`/`__gc`-error sequence through the spawned CLI and diffs both
stdout and stderr against the reference binary on 5.4/5.5.

---

## Item 4 — debug line-hook fidelity — **DOCUMENTED (genuinely deep, two-crate version-gated change)**

Investigated and confirmed; not attempted, per the honesty rule and the explicit
"attempt only if localized, else DOCUMENT" instruction. The divergence is real,
version-specific, and has two *independent* root causes — neither in the hook
logic itself — each carrying cross-version line-number regression risk.

### Confirmed divergences (reproduced this pass)
**Cause 1 — back-edge fire rule changed at 5.4 (affects 5.1/5.2/5.3).**
`for i=1,4 do local a=1 end` traced with `sethook(f,"l")`:
- ref 5.1/5.2/5.3: `3,3,3,3,3,4` (fires on *every* backward jump).
- ref 5.4/5.5: `3,3,3,3,4` (fires only when the line actually changed).
- lua-rs (all versions): `3,3,3,3,4` — implements the 5.4 rule everywhere, so it
  is **wrong on 5.1/5.2/5.3** (one missing back-edge event per iteration).

**Cause 2 — codegen line-attribution of conditional TEST/JMP changed at 5.5
(affects 5.5).** A multi-line `if / <cond> / then / ... / else / ... / end`:
- ref ≤5.4 attributes the condition `TEST`/`JMP` to the `then` line → a line
  event fires for it.
- ref 5.5 folds them onto the condition-expression line → no separate event.
- lua-rs emits the `then`-line event on all versions, so it is **wrong on 5.5**
  (one extra event). (`luac -l` per version confirms the differing line on the
  `TEST`/`JMP` instructions; db.lua's own version-matched expected arrays encode
  the split — 5.3.4-tests vs 5.5.0-tests differ.)

rs is therefore a *hybrid*: it matches 5.4 on both cases but no single other
version on both.

### Why DOCUMENTED, not fixed
A correct fix is a **two-part, version-gated change across two crates**:
- (a) Version-gate codegen line-attribution for the `if`/`while` conditional
  `TEST`/`JMP`: 5.2/5.3 attribute to the `then`/`do` line; 5.4/5.5 fold onto the
  condition-expression line. Seam: `cond` / `test_then_block` in
  `crates/lua-parse/src/lib.rs` and the `luaK_fixline` TODO (~line 4607).
  Verify with the C `luac -l` in `/tmp/lua-refs/lua-5.x/src/luac` per version.
- (b) Version-gate the back-edge rule in `crates/lua-vm/src/debug.rs`
  `trace_exec`: 5.1/5.2/5.3 fire on every backward jump (`npci <= oldpc`)
  unconditionally (old `luaG_traceexec`); keep the current 5.4 `changed_line`
  path for 5.4/5.5.

Each part risks regressing line-number reporting everywhere it is consumed —
error locations, tracebacks, `getinfo("l").currentline` — across all five
versions. That is exactly the "structural change too large to land with full
oracle verification this pass" the honesty rule says to document. The minimal
per-version repro harnesses (`/tmp/forloop.lua` for cause 1, `/tmp/mlif.lua` for
cause 2) pin the exact expected sequences with zero harness noise and are the
inner loop a future pass should develop against, plus a per-version
expected-trace table before touching either seam. Drive db.lua from its real
file path with the version-matched test directory (5.3.4-tests vs 5.5.0-tests);
the expected arrays differ. Tree left clean (no probe edits to crates).

---

## Always-documented items (not attempted — confirmed deep)

### Generational-GC default mode — **DOCUMENTED**
5.4 and 5.5 default to the **generational** collector; `collectgarbage` mode
queries (`"incremental"`/`"generational"`/`"isrunning"`) reflect this. lua-rs
runs the **incremental** collector on all versions; no generational collector
exists in `crates/lua-gc` (the `GcKind::Generational` enum variant is present
but there is no generational implementation behind it). This is genuine
collector behavior, not a wording swap — reporting the mode while running the
wrong collector would be a lie. Re-entry: implement a generational mode in
`crates/lua-gc` (minor/major collection, age fields, the `genminormul`/
`genmajormul` pacing params already surfaced by the 5.5 param API), wire the
default per version, and only then make the mode query report it. Risky; large.

### Named-vararg `...t` / `...` aliasing — **DOCUMENTED**
5.5's always-materialize lowering makes a named vararg table `t` and `...`
independent storage; upstream shares one storage object. (Note: the no-table
named-vararg accessor form `function(t, ...v)` returning `v[1]` etc. *is* already
implemented and correct on 5.5 — `/tmp/namedva.lua` matches. The gap is the
*aliasing* of the materialized table with `...`.) Affects `vararg.lua:111`,
`locals.lua:314` (5.5). Re-entry: add a proto field for the vararg-table
register, redirect `OP_VARARG` to read it, and drop the snapshot copy so `t` and
`...` reference one object. (Carried from `5.5-lang.md` §2a.) VM/codegen-level.

---

## Issue-title list (architectural remainder, for filing)

1. **debug: version-gate line-hook fidelity (back-edge rule + conditional
   TEST/JMP line attribution)** — rs is a 5.4 hybrid; wrong on 5.1/5.2/5.3
   (missing per-iteration back-edge events) and 5.5 (extra `then`-line event).
   Two-crate, version-gated change; regression risk across all line-number
   consumers. (Item 4.)
2. **gc: implement generational collector and per-version default mode** — 5.4/5.5
   default to generational; rs runs incremental everywhere. Needs a real
   generational collector in `lua-gc` before the mode query can honestly report
   it.
3. **5.5: alias named-vararg table with `...` (shared storage)** — materialized
   named-vararg table and `...` are independent in rs; upstream shares one
   object. Needs a proto field for the vararg-table register.
4. **parser: version-gate `break`-outside-loop error wording** — 5.1 `no loop to
   break near '<tok>'`, 5.2/5.3 `<break> at line N not inside a loop`, 5.4
   `break outside loop at line N`, 5.5 `break outside loop near 'break'`. rs
   emits the 5.4 form on all versions; 5.1/5.5 also need the eager `near '<tok>'`
   form raised in `breakstat`. (Found while fixing item 2's prefix.)
5. **vm: 5.3-only loop-built-closure equality caching** (`closure.lua:48`) — 5.3
   caches and returns the same `LClosure` for closures with identical upvalue
   sets; 5.4+ removed the cache (rs matches 5.4/5.5). Needs a per-proto LClosure
   cache keyed on the upvalue set in the `OP_CLOSURE` path, 5.3-gated.
   (Carried; not in this batch's item list.)
6. **vm/coroutine: `__le`-from-`__lt` derivation across a yield** (5.3/5.4) — the
   derived swapped-operand `__lt` call loses both the yield and the `not(b<a)`
   inversion when the metamethod yields. Needs a resume continuation in the
   order-metamethod path. (Item G; carried.)
