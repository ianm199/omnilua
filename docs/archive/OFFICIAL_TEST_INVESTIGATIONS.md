# Official-test failure investigations

Living document tracking the deep-dive into each failing test in `harness/run_official_all.sh`'s 44-test suite. Latest completed all-suite measurement (2026-05-24, `RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=90 ./harness/run_official_all.sh`): **44/44 (100%)** — 44 pass, 0 fail, 0 timeout.

Each section records: what the test exercises, the precise failure, upstream (lua-c) data flow for the same source, our current behavior, the divergence, attempted fixes, and what's still required.

---

## Tracking convention

| status | meaning |
|---|---|
| **OPEN** | actively investigating |
| **DIAGNOSED** | root cause understood; fix not yet attempted |
| **PARTIAL** | some fixes landed but test still fails |
| **FIXED** | test passes in HEAD |
| **PARKED** | known-cause but out of scope (e.g. needs a different subsystem) |

---

## `db.lua` — debug library, hooks, coroutine traceback, finalizers

**Status**: FIXED — `RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=45 ./harness/run_official_test.sh reference/lua-c/testes/db.lua` passes, and `db.lua` passes through the `all.lua` bytecode dump/load path.

### Resolution summary

The final successful path was not one bug; it was a layered official-test ladder:

1. Fixed stale line attribution for the large-gap hook traces by using the RHS/current token line where the bytecode emission had been pinned to an earlier operator line.
2. Fixed debug local/upvalue semantics: varargs, non-current temporaries, open upvalue access, count-hook multi-return preservation, and cross-thread coroutine debug APIs.
3. Reduced the heap abort to `harness/repro/db_hook_getlocal_coroutine_traceback_repro.lua`, then used ASan. The visible crash was traceback string allocation, but the real UAF was a parent-thread open `UpVal` swept during `coroutine.resume`. The fix roots suspended parent open-upvalue handles alongside the existing suspended parent stack-value snapshots.
4. Fixed finalizer debug naming: finalizer calls temporarily mark the caller frame with `CIST_FIN`, so `debug.getinfo(1)` inside `__gc` reports `namewhat="metamethod"` and `name="__gc"`.
5. Fixed traceback truncation: after emitting the `"... (skipping N levels)"` line, the traceback builder must switch from the first display window to `LEVELS2`; otherwise deep tracebacks can loop.
6. Fixed stripped debug-info behavior: absent Lua upvalue names report `"(no name)"`, and absent proto source reports as `"=?"`, whose short source is `"?"`.
7. Fixed the `all.lua` dump/load-only failure: `LuaValue::full_type_tag()` must write variant tags, not base tags. Otherwise `string.dump` writes floats as integer constants and `2^24 - 1` reloads as the raw f64 bit pattern.
8. Fixed the final `all.lua` weak-table panic by unlinking weak-pruned hash nodes from table collision chains. Clearing a hash key to `nil` without repairing `next` offsets makes a chained node look reusable and can also make `next(t, k)` repeatedly match stale equal long strings.

Reusable instructions and the full case study are saved in `docs/DEBUGGING_STRATEGIES.md` under "Official-Test Crash/Hang Ladder".

### Historical line-hook investigation notes

The notes below are retained because they explain the earlier line-attribution
work that became one layer of the final `db.lua` fix. Some phrasing refers to
the state before the later GC/finalizer/traceback fixes.

### What it tests

`debug.sethook(f, "l")` fires `f` on every line executed. The test compares the fired-line sequence against a hand-written expected list. Each test() invocation runs a small Lua chunk and asserts the trace matches.

### Failure point

`db.lua:28` — `assert(l == line, "wrong trace!!")` inside the test() helper.

The helper is called many times; the failure was whichever test() invocation hit a trace mismatch first. Before the later `db.lua` fixes, only the **gap-test loop at line 207** still failed. That loop generates 256 test() invocations with `\n×i / \n×j` injections inside `a = b[1] + b[1]` to stress line attribution across large source gaps.

### Root cause (the systemic finding)

**lua-c attributes every instruction's line via `savelineinfo(fs, f, fs->ls->lastline)` at emit time**, called from `luaK_code` (lcode.c:388). The line is read DYNAMICALLY at the moment the instruction is added to the proto.

**Our parser threads a `line` parameter through 30+ codegen call sites**, captured at various points before the actual emission. For simple statements this matches lua-c. For nested expressions where emit happens many tokens after capture, the saved `line` is stale.

### lua-c data flow for `a = b[1] + b[1]`

`luac -l -l` dump:
```
1 [1] VARARGPREP   0       ; line 1
2 [1] NEWTABLE     0 0 1   ; line 1
3 [1] EXTRAARG     0       ; line 1
4 [1] LOADI        1 10    ; line 1
5 [1] SETLIST      0 1 0   ; line 1
6 [3] GETI         1 0 1   ; line 3 — first b[1]
7 [4] GETI         2 0 1   ; line 4 — second b[1]
8 [3] ADD          1 1 2   ; line 3 — operator
9 [3] MMBIN        1 2 6   ; line 3
10 [4] SETTABUP    0 0 1   ; line 4 — store back to a
11 [5] LOADI       0 4     ; line 5 — b = 4
12 [5] RETURN      1 1 1   ; line 5
```

Key dynamic behavior:
- The first GETI is emitted AFTER `+` is consumed → `lastline = 3` → instruction at line 3
- The second GETI is emitted AFTER second `b[1]` is parsed → `lastline = 4` → instruction at line 4
- ADD/MMBIN emit at current `lastline = 4`, but lua-c calls `luaK_fixline(fs, line)` after emission to OVERRIDE the line to the saved operator line (3)
- SETTABUP emits at current `lastline = 4`

### Our data flow

We capture `let line = ls.linenumber;` BEFORE `lex_next` consumes the operator → `line = 3` (operator line). This is correct for the operator-line capture but is then passed all the way down to `cg_discharge_vars` for both operands.

Result: our second GETI also emits at line 3, missing the line-4 hook fire.

### Surgical fixes that landed

| commit | site | from | to |
|---|---|---|---|
| `deacc5e` | `adjust_assign` (lib.rs:2443) | `linenumber` | `lastline` |
| `deacc5e` | `leave_block` OP_CLOSE (lib.rs:2898) | `linenumber` | `lastline` |
| `deacc5e` | `whilestat` back-jump (lib.rs:3895) | `linenumber` | `lastline` |

These three flipped these patterns to pass:
- `local a` (with no initializer) now fires hook for line 1
- While-loop back-jump no longer fires the END line on every iteration
- `do...end` blocks attribute their OP_CLOSE to the END's line, not the next-statement's line

All simple test() cases (lines 124–188 in db.lua) pass standalone.

### What was still required at that point

The gap-test loop needs lua-c's dynamic-lastline-at-emit behavior. Either:

1. **Structural emit_inst refactor**: change `emit_inst(fs, line, inst)` to ignore the threaded `line` and read `fs.last_token_line` (mirrored from `ls.lastline` on every `sync_from_lex`). Plus add a `luaK_fixline`-equivalent to override the line of just-emitted instructions for binop tag-method line attribution.
2. **Per-site audit**: walk every `cg_discharge_vars` / `cg_exp_to_*` callsite and change it to use the *current* `ls.lastline` instead of the threaded `line`. ~30 sites.

### Attempted but reverted

- **Bulk `linenumber → lastline` replace (2026-05-24)**: regressed `errors.lua` from PASS→FAIL on `lineerror` cases that depend on operator-line capture for binop tag-method attribution. Specifically the binop site in `subexpr` (lib.rs:3645) needs `linenumber` (operator's line, captured BEFORE consuming) — bulk replace got the line of the LEFT OPERAND instead.
- **Add `fs.last_token_line` field + read from `emit_inst`**: introduced a deep crash (`internal: execute called on non-Lua frame`) in db.lua. Not yet diagnosed; probably caused a wrong abslineinfo offset that mis-set CallInfo state somewhere.

---

## `gc.lua:469` — weak-table-of-long-strings sweep

**Status**: FIXED — `RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=60 ./harness/run_official_test.sh reference/lua-c/testes/gc.lua` passes.

### What it tests

Lua 5.4's incremental GC handling of weak tables (`__mode = "kv"`) holding long-string keys mapped to mixed values (numbers, tables). After collect, only entries with non-collectable values should survive.

### Failure points

This block exposed two distinct bugs:

- `gc.lua:465` expects post-collect memory to keep exactly one 4 MB long-string key alive.
- `gc.lua:469` expects `next(a, k) == nil` after the surviving long-string key is returned.

### Standalone repro behaves correctly

```lua
local m = collectgarbage("count")              -- ≈ 0.3 KB
local a = setmetatable({}, {__mode = "kv"})
a[string.rep("a", 2^22)] = 25                  -- long string -> number
a[string.rep("b", 2^22)] = {}                  -- long string -> table
a[{}] = 14                                      -- table -> number
-- check post-insert count > m + 8192 ✓ (8192.8 KB)
collectgarbage()
-- check post-collect 4096 ≤ count < 8192 ✓ (4096.7 KB)
```

Our output:
```
baseline m = 0.3017578125 KB
after insert: diff = 8192.81 KB (passes assert > 8192)
after collect: diff = 4096.68 KB (passes assert 4096 ≤ ... < 8192)
surviving entries in a: { key=4194304-char string, val=25 }
```

### Resolution

The final fix was table-shape correctness, not memory accounting:

- Weak-table pruning must clear entries whose weak key or weak value was not marked.
- If a hash node is removed, its collision chain must be repaired. A node with a stale `next` offset cannot simply be marked free by setting `key = nil`.
- Removing a chain node now either patches the predecessor's `next`, or moves the successor into the removed head slot and rewrites the successor's relative `next` offset.
- This also prevents `next(a, k)` from re-finding a stale equal long-string key and returning the same live entry again.

### Verification

The focused weak-table toy now returns only the `"a"` long-string key, `next(a, k)` returns nil, lookup of the cleared `"b"` key returns nil, and the final collect empties the table.

`gc.lua` passes standalone and inside `all.lua`.

---

## `gengc.lua:122` — generational weak-table cleanup

**Status**: FIXED — `RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=60 ./harness/run_official_test.sh reference/lua-c/testes/gengc.lua` passes.

### Root cause

`collectgarbage("generational")` followed by explicit `collectgarbage("step", 0)` must still perform the atomic weak-table cleanup that removes all-weak entries whose targets were not marked. Our generational mode does not model object ages yet, and the initial shortcut of doing a full collect on every gen step was too aggressive: it collected suspended coroutine/open-upvalue state that should survive.

### Fix landed

Added a mark-only weak cleanup for generational steps:

- `lua-gc::Heap::mark_only_with_post_mark` traces roots and runs the post-mark hook without sweeping objects.
- `GcHandle::prune_weak_tables_mark_only` snapshots weak tables, traces reachable threads, converges ephemerons, and prunes dead weak entries.
- `collectgarbage("step", ...)` invokes this cleanup in generational mode after the incremental step.

This gives `gengc.lua` the weak-table semantics it needs without pretending we have a complete age/barrier implementation.

---

## `coroutine.lua:319` — xpcall + yield + error

**Status**: FIXED — `coroutine.lua` now passes in the full official sweep.

### Root cause

lua-c's `luaG_errormsg` invokes the message handler at the error-RAISE site (inside the longjmp path), so by the time `finishpcallk` sees the error during recovery, the message has already been transformed to `handler(err)`.

Our Rust error propagation uses `Result::Err` and has no equivalent chokepoint at raise. The synchronous-error path in `pcall_k` (api.rs:1944) handles the handler call inline. But the yield-then-error path (`pcall_k` Err(Yield) at api.rs:1930) propagates Yield without restoring state and the eventual resume-time error skips the handler entirely — `msg` reaches the test as the raw error table instead of `g(err)`.

### Fix landed

`crates/lua-vm/src/do_.rs` `finish_pcallk` — after `set_error_obj` writes the error to `func_idx`, invoke the handler (via `state.errfunc`, which is preserved across yield since the Yield path never restored it) and replace the error value. Mirrors lua-c's `luaG_errormsg` semantically — different timing (recover instead of raise), equivalent post-condition. The synchronous-error path clears `CIST_YPCALL` before returning so it never reaches `finish_pcallk` — no double-invoke.

### Verification

Standalone repro `/tmp/coro_repro.lua` returns `r=false, msg=240` (was `msg=table`). errors.lua's xpcall-based `checkerror` cases unaffected (verified via Tier 2 regression check).

### Follow-up result

The later per-coroutine hook failure also cleared after the coroutine close/recovery fixes below; `coroutine.lua` passes as of the 40/44 run.

---

## `locals.lua:974` — `<close>` across coroutine exit paths

**Status**: FIXED — `locals.lua` passes standalone and in the full 44-test sweep.

### What it tests

Lua 5.4's `local x <close> = ...` semantics: when scope exits (normally OR via error), each `<close>` variable's `__close` metamethod fires in LIFO order. The test stresses the case where each `__close` itself contains `coroutine.yield(...)`.

```lua
local function foo (err)
  local z <close> = func2close(function(_, msg)
    assert(...)
    coroutine.yield("z")     -- yield from inside __close
  end)
  local y <close> = func2close(function(_, msg)
    coroutine.yield("y")
    if err then error(err + 20) end
  end)
  local x <close> = func2close(function(_, msg)
    coroutine.yield("x")
  end)
  if err == 10 then error(err) else return 10, 20 end
end
```

### What works

Without yields, our `<close>` ordering is correct:

```
foo raising error(10)
x close: msg=10, err=10   <- x fires first (LIFO ✓)
y close: msg=10, err=10   <- y fires second; raises error(30)
z close: msg=30, err=10   <- z fires last with the updated error
final: false, 30
```

### What broke

With yields inside __close (the actual test), our impl reports z firing FIRST (wrong order) on the third foo case (`pcall(foo, 10)`). The first `co()` call after entering this scenario returns `false, "...assertion failed!"` where the failing assert is z's `assert(msg == nil or msg == err + 20)` — z saw `msg=10` (the original error), not `msg=30` (the y-substituted error). This means z fired before y had a chance to run and substitute.

### Actual root cause

The close-list cursor hypothesis was wrong. Lua C does not need an extra cursor: `luaF_close` pops each `tbclist` entry before calling its `__close`, and that popped list is the continuation state.

The real divergence was in the yieldable `lua_pcallk` path. Our Rust `pcall_k` wrapped the yieldable call in `raw_run_protected`, caught body errors locally, cleared `CIST_YPCALL`, and ran `close_protected` as a conventional non-yieldable pcall. Lua C does the opposite: yieldable `lua_pcallk` calls `luaD_call` directly because `lua_resume` is already the protected boundary. Real errors must unwind to `precover`, with `CIST_YPCALL` still set, so `finishpcallk` can close pending `<close>` variables yieldably and preserve close-error substitution.

### Fix landed

- `crates/lua-vm/src/api.rs`: yieldable `pcall_k` now calls `do_::call` directly and leaves `CIST_YPCALL` installed on real errors.
- `crates/lua-stdlib/src/base.rs`: `pcall`/`xpcall` now rethrow yieldable protected-call errors instead of converting them to `(false, err)` at the wrong stack boundary.
- `crates/lua-stdlib/src/coro_lib.rs`: `coroutine.wrap` now closes an errored wrapped coroutine via the existing reset/close path, matching `luaB_auxwrap`'s `lua_closethread`.
- `crates/lua-vm/src/func.rs`: stale closed upvalues in the Rust Vec-backed open-upvalue list are unlinked during close instead of panicking.

Verification:
- `harness/repro/locals_close_yield_repro.lua` covers the exact 12-resume close-yield sequence plus wrapped-coroutine close-on-error.
- Focused verification for the close/yield path passed; the final full sweep also passes `locals`.

---

## `cstack` — Rust stack overflow during 1000-coroutine close cascade

**Status**: FIXED — `cstack.lua` passes standalone and in the full 44-test sweep.

`coro_lib.rs` `co_close` now bounds Rust stack via `inc_c_stack`, and the wrapped-coroutine/error-close fixes addressed the earlier cascade instability.

---

## `calls`, `calls_head313`, `calls_isolated_tmp` — call semantics timeout bucket

**Status**: FIXED — all three pass in the full 44-test sweep.

The timeout bucket disappeared after the coroutine/error-recovery, GC, and weak-table fixes; no separate call-semantics patch was needed for these targets.

---

## `all` — meta-runner

**Status**: FIXED — `all.lua` passes in the aggregate.

The last `all.lua`-only blockers were:

- `string.dump`/`load` numeric corruption from writing base type tags instead of variant tags.
- A weak-table hash-chain panic after `db.lua` advanced through stripped-debug-info checks.
- A `gc.lua:469` repeat-entry failure from weak-pruned long-string tombstones, fixed by unlinking removed hash nodes.

---

## The structural emit_inst design (what would actually fix this layer)

For a future session, the right systemic fix:

1. Add `fs.last_token_line: i32` field, mirrored from `ls.lastline` in `sync_from_lex` and in each `lex_next` / `lex_lookahead` call site (currently `sync_from_lex` already runs after both).

2. Change `emit_inst` to use `fs.last_token_line` for the line attribution, completely ignoring the threaded `line` parameter (or using it only as an override hint).

3. Add a `cg_fixline(fs, line)` helper that overrides the line of the just-emitted instruction (the latest entry in `f.lineinfo` / `f.abslineinfo`). Mirrors lua-c's `luaK_fixline`.

4. In `cg_posfix_fold`'s binop emission and a few other sites where lua-c explicitly fixlines, call `cg_fixline` after the emission.

When attempted naively (just changing emit_inst), this introduced a deep crash because the `abslineinfo` records were mis-aligned vs the threaded line — needs careful audit of `previousline` updates and `iwthabs` accounting against the new effective_line.

Estimated effort: 4–6 hours of careful work + extensive testing.

---

## Latest full-sweep checkpoint

Final 2026-05-24 sweep is **44/44**:

- 44 pass.
- 0 fail.
- 0 timeout.
- Command: `RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=90 ./harness/run_official_all.sh`.
- Evidence: `harness/impl/official/run_all.tsv`.

## Earlier progress checkpoint

Started the session at **37/44** deterministic. Ended at **40/44**:

- `locals.lua` passes.
- `coroutine.lua` passes.
- `cstack.lua` passes.
- Full sweep: 40 pass, 3 fail (`all`, `db`, `gengc`), 1 timeout (`gc`).

Later work took the suite from 40/44 to 44/44 by fixing `db`, `gc`, `gengc`, and the `all.lua` dump/load path.
