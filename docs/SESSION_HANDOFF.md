# Session Handoff — lua-rs-port

Last session ended: **2026-05-20**. Last commit: `cad3abe` (F-1: 33→38/44).

## State of the world

- **Official suite: 38/44 PASS (86%)** on `./harness/run_official_all.sh`. Run from `lua-rs-port/` root.
- **`cargo build` clean**, all 102 warnings are pre-existing in lua-stdlib (mostly unused imports — cleanable separately).
- **Smoke set: 6/6 PASS** (`./harness/run_one_test.sh reference/lua-c/testes/{strings,closure,tracegc,big,sort,math}.lua`).
- **LuaRocks 3.11.1 runs `--version` end-to-end** (still — not regressed). Reproduce:
  ```bash
  curl -sSL https://luarocks.org/releases/luarocks-3.11.1.tar.gz | tar xz -C /tmp
  tail -n +2 /tmp/luarocks-3.11.1/src/bin/luarocks > /tmp/luarocks_noshebang.lua
  LUA_PATH="/tmp/luarocks-3.11.1/src/?.lua;/tmp/luarocks-3.11.1/src/?/init.lua" \
    ./target/debug/lua-rs -e 'arg={[0]="luarocks","--version"}; dofile("/tmp/luarocks_noshebang.lua")'
  ```
  Currently fails at `error in error handling` on the exit teardown path — not regressed, just not yet fixed.

## The 6 remaining failures

Three families, each with concrete file:line entry points.

### Family A — Yield through C-frames (2 tests, ~$70)

The C-Lua resume path through frames marked `CIST_YPCALL` / `CIST_CLSRET` is incomplete. F-3.a/b landed the `pcall_k`/`call_k` yieldable branches; the resume side that continues those frames via `finishCcall(L, ci)` needs work.

**`coroutine.lua:327`** — yield-through-pcall.
- Test pattern around `coroutine.lua:282-330`: `coroutine.create(pcall)` where the inner fn yields.
- Where to look: `crates/lua-vm/src/api.rs` (resume entry, particularly the in-progress C-frame handling) and `crates/lua-vm/src/do_.rs` (finishCcall analog).
- C-Lua reference: `ldo.c::lua_resume` lines ~575-625, `if (s == LUA_YIELD) finishCcall(L, ci);`.
- **STALL WARNING**: The R-β agent stalled (600s no-progress) chasing this with too-broad scope. Output cut off mid-investigation about an `errfunc=5` save/restore confusion across xpcall. Next agent should scope to ONE assertion (e.g. only flip the `coroutine.lua:282-289` subtest first; the surrounding test code already passes).

**`locals.lua:982`** — yield inside `__close` metamethod.
- Test pattern: `__close` does `coroutine.yield(...)` then returns.
- Needs the `CIST_CLSRET` callstatus flag wired and the OP_CLOSE / OP_TBC path to honor it on resume.
- Where to look: `crates/lua-vm/src/vm.rs` (OP_TBC, OP_CLOSE dispatch), `crates/lua-vm/src/do_.rs` (close_protected re-entry).
- **STALL WARNING**: R-γ also stalled. Same broad-scope issue. Next agent should target ONLY `locals.lua:975-985` first.

**Cost:** $30-50 each, opus. Prompt them as "make this one assertion at line N pass, not the whole subsystem."

### Family B — GC reachability + barriers (3 tests, ~$70-100)

**`gc.lua` TIMEOUT** — fails in "self-referenced threads" section. GC walk does not converge on cycles.
- Where to look: `crates/lua-gc/src/heap.rs` (the trace fixed-point), `crates/lua-vm/src/trace_impls.rs::impl Trace for GlobalState` (the thread visiting block).
- Likely fix: in the post-mark fixed-point hook, detect when nothing new is marked in an iteration and exit, plus correctly drain `twups` / `cross_thread_upvals` mirrors so the cycle resolves.
- **Cost:** $40 opus.

**`gengc.lua:130`** — real GC barrier issue (verified NOT a generational-GC bug by `harness/canaries/gc/canary_b_coro_upvalue.lua` reproducing under incremental mode).
- This needs `gc-barrier-noops` and `gc-phase-predicates-always-constant` ghosts retired (see `docs/GHOST_ABSTRACTION_REGISTER.md`).
- C-Lua reference: `lgc.c::luaC_barrier`, `lgc.c::luaC_barrierback`, `lgc.h::keepinvariant` / `issweepphase` macros.
- Current barriers in `crates/lua-vm/src/state.rs:2789-2806` are empty `{}` bodies. `keep_invariant()` and `is_sweep_phase()` always return `false`.
- **Cost:** $40-60 opus.

**`all.lua` TIMEOUT** — composite runner; downstream of `gc.lua`. **Will flip automatically when `gc.lua` does.** No standalone work needed.

### Family C — Debug library (1 test, ~$25)

**`db.lua:28`** — line-hook event timing.
- Test: `debug.sethook(f, "l")` followed by `load(s)()` then `debug.sethook()`. Asserts `#l == 0` — meaning the line-hook callback either fired or recorded events when it shouldn't have.
- Where to look: `crates/lua-vm/src/debug.rs::trace_exec` (the line-hook dispatch) and the recently-added C-frame guard at `trace_exec` line ~1916. The guard may be too lax or fire at wrong times.
- **Cost:** $25 opus.

## Recommended dispatch order

In parallel worktrees, after one fresh `cargo build` and `./harness/run_official_all.sh` sanity check:

1. **`db:28`** ($25 opus) — smallest, most isolated. +1 to **39/44**.
2. **`gc.lua` TIMEOUT** ($40 opus) — flips `all` automatically. +2 to **41/44**.
3. **`gengc:130`** ($40-60 opus) — retires 2 ghost entries, foundational for any future GC work. +1 to **42/44**.
4. **`coroutine:327` and `locals:982`** ($30-50 each, opus, in parallel) — both yield-through-C. Each must scope to ONE assertion. +2 to **44/44**.

**Total budget to 44/44:** ~$170-225.

**Realistic outcome:** 42/44 likely; 44/44 requires the yield-through-C work to land cleanly, which is the hardest remaining surface and has stalled an agent before. Budget mental floor at 42/44.

## Critical rules for any next agent

These are project-wide and the audit will catch violations:

- **No inline `//` comments.** Doc strings only. The CLAUDE.md at `/Users/ianmclaughlin/.claude/CLAUDE.md` enforces this.
- **No fallback patterns** (`x || y || z`). Single source of truth — if data may be missing, fix the data path.
- **No new `unsafe`** outside `lua-gc` (and `lua-cli` with its FFI budget). `lua-coro` currently has a zero budget; raise it only with a concrete stackful backend. `unsafe_code = "forbid"` is the workspace default.
- **No `String`/`&str` for Lua data** — use `&[u8]` / `Vec<u8>` / `LuaString`.
- **No `tokio` / `rayon` / `std::process` / `std::fs` / `std::net`** outside `lua-cli`. The hook pattern (e.g. `PopenHook`, `FileOpenHook`, `OsExecuteHook` in `state.rs`) is how stdlib reaches the OS.
- **Never edit** `reference/lua-c/testes/`. Tests are the oracle.
- **Never `--no-verify`** on commits. The Stop hook auto-commits and gates on the smoke set.

## Infrastructure available

- **Ghost abstraction register**: `docs/GHOST_ABSTRACTION_REGISTER.md` (11 active entries). Run `./harness/check_ghost_abstractions.sh --dry-run` to see drift. PreToolUse hook wrapper is staged at `.claude/hooks/check-ghost-abstractions.sh` but NOT yet wired into `.claude/settings.json` — wire it before next agent dispatch if you want it gating.
- **GC canaries**: `harness/canaries/gc/` — 5 canaries with dual-mode runner. **Run these BEFORE any GC-related agent dispatch.** Specifically `canary_b_coro_upvalue.lua` was the diagnostic that proved gengc.lua isn't a gen-GC bug.
- **Smoke set**: `./harness/run_one_test.sh reference/lua-c/testes/<name>.lua` for any single test. 6-test smoke set is `{strings, closure, tracegc, big, sort, math}`.
- **Stop-hook gate**: `harness/stop-hook.sh` runs the smoke set on agent Stop events. Auto-commits per `harness/baseline-smoke.tsv`.
- **Worktree-isolated parallel agents**: use the `Agent` tool with `isolation: "worktree"` + `run_in_background: true`. Pattern proven through this session.

## Specific scoping advice for the next session

**For the yield-through-C agents (R-β / R-γ-style work):**

Don't scope as "make `coroutine.lua` pass" or "fix yield-through-pcall everywhere." That's what stalled the agents last session. Instead, scope as:

> "Make `coroutine.lua:282-289` pass. This subtest creates a coroutine with `coroutine.wrap(function() assert(not pcall(table.sort, ...)); coroutine.yield(20) end)` and asserts `co() == 20`. The current failure is at `coroutine.lua:327` — go figure out which earlier assertion in the same do-block is actually triggering and fix the smallest possible thing. Do not refactor the errfunc system. If you can't make the subtest pass in 45 minutes, report what you found and stop."

The agents stall when they spread across the entire `errfunc` save/restore + CallInfo flag wiring + `finishCcall` re-entry path at once. Force them to bisect.

**For the GC barrier work:**

Don't ask for "implement gengc." Ask for:

> "Wire `luaC_barrier` and `luaC_barrierback` under incremental GC. The bodies at `crates/lua-vm/src/state.rs:2789-2806` are currently `{}`. Match C-Lua's `lgc.c::luaC_barrier_` exactly. Also implement `keep_invariant()` and `is_sweep_phase()` based on the `gcstate` field. Don't touch generational-mode code. Verify with `./harness/canaries/gc/run_canaries.sh` (all 5 still pass) and `./harness/run_official_all.sh` (≥38/44, target +1 for gengc)."

## Don'ts

- **Don't touch `crates/lua-types/src/table.rs`** — the canonical LuaTable just landed this session. It's working.
- **Don't bring back `FLAT_TABLE_GROW_CAP`** — retired. Use `TOTAL_GROW_CAP` in lua-types if you need a cap.
- **Don't add inline `//` comments** to any file — every fix has accidentally added some; the project rule is strict.
- **Don't commit `harness/impl/official/*.out`** — those regenerate every suite run. The `.tsv` is the canonical scoring artifact.
- **Don't poll or sleep waiting on background agents.** Use `Agent` with `run_in_background: true` and you'll be notified on completion.

## Cost summary this session

| Agent / batch | Status | Net flips |
|---|---|---|
| R-α aux_resume upvalue | landed, canary_b PASS, gengc advanced :99→:130 | 0 (but huge structural fix) |
| R-β yield-through-pcall | stalled at 600s | 0 (advanced coroutine :282→:327) |
| R-γ yield-in-close | stalled at 600s | 0 (locals stayed at :982) |
| Small-fixes batch (files/errors/db) | partial | bonus prefixing work landed |
| Table refactor | landed | 0 (eliminated FLAT_TABLE_GROW_CAP) |
| Ghost audit | landed | 0 (3 entries retired so far) |
| Canary set | landed | 0 (saved $200 of misdirected gengc work) |
| **User-driven F-1 finish** | landed at `cad3abe` | **+5 (files/errors/nextvar/cstack/literals)** |
| **Net** | **33 → 38/44** | ~$450 spend total |

## Final advice for the next agent

The work is well-bounded now. The remaining failures are individually scoped and each has a known C-Lua reference for the fix. The biggest risk is repeating the R-β/R-γ stall — agents tried to fix entire subsystems instead of one assertion. Scope tighter, dispatch in parallel, integrate carefully, run canaries before any GC work.

You're 5-7 fixes from 44/44. None of them require Phase D-3 generational GC. The canary set proved that. Spend the budget on focused yield-through-C and GC-barrier work, in that order, and you'll land it.
