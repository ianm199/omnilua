# Stuck tests

Programs in `reference/lua-c/testes/` that the autonomous harness has plateaued on. Each doc captures the **current** failure point, what past agents tried (transcript pointers), the actual code state, and a focused next-agent prompt.

## Priority order (refreshed 2026-05-18 evening, post-cherry-pick of 42ff10f)

| # | Test | Why agents plateaued | Next move |
|---|---|---|---|
| 1 | [files.lua](files.lua.md) | 5 borrow-split stubs in `io_lib.rs`. Earlier doc said *"add RefCell"* — already done. Existing `LSTREAM_REGISTRY: Rc<RefCell<LStream>>` pattern just needs to be applied to each stub. | **Best next Opus target.** Focused prompt: finish `f_seek`, `f_flush`, `f_write`, `f_read`, `io_read` using the existing pattern. |
| 2 | [errors.lua](errors.lua.md) | Each `checkmessage` mismatch is its own one-line wording bug; agents kept rediscovering which one. | **Diagnosable now.** Exact failing tuple captured: `table.sort({1,2,3}, table.sort)` expects substring `'table.sort'`, we emit `'?'`. Name-attribution fix in `arg_error_impl`. Sonnet-tier. |
| 3 | [gc.lua](gc.lua.md) | `collectgarbage("step", n)` ignores `n`; no real budget model. GC has advanced through D-1e/D-1f/D-2/finalizer reachability, but each agent round fixes a real bug *underneath* gc.lua and the test still doesn't pass. | **Do not agent-loop.** Add to `SKIP_TESTS`, or split into harness-visible subtests, or scope a human-designed budget model. |

## Why these plateaued (v4 stuck-skip in action)

The harness escalated each from Sonnet → Opus, ran ≥2 Opus rounds, observed no whole-test pass-count progress (despite real commits landing), and marked them `[skip stuck prog (no progress 2 rounds)]`. That's the intended behavior — stop pouring $10/round into agent attempts that keep peeling layers without converging. Captured in `docs/RETROSPECTIVE_AND_PRODUCTIZATION.md` §11.

## How agents *do* make progress on these — without passing them

`git log --grep=gc.lua` shows eight commits. Each fixed a real bug (D-1e, D-1f, D-2 weak-table sweep, finalizer reachability, OP_SELF codegen…) and advanced the test by one phase. The test still fails — but the codebase is meaningfully better. "Stuck" is misleading: these are **multi-layer** tests where every layer fix is its own valuable PR. The harness can't see that, because it only counts whole-test passes.

## How to use these docs

When dispatching the next agent on one of these files, **paste the "Suggested prompt" block from the per-test doc verbatim** as the user message. The prompts are written to:

- name the exact site (file + line + function)
- give the working pattern to copy from (`io_write` for files.lua, `lauxlib.c:luaL_argerror` for errors.lua)
- forbid the distractions past agents fell into ("do not redesign userdata", "do not try to fix errors.lua broadly")
- include a smoke-test command to confirm the fix moved the failure point
