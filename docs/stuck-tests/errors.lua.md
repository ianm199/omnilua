# Stuck: `reference/lua-c/testes/errors.lua`

**Status:** diagnosable now. The earlier doc said "instrument first"; that's been done. We have the exact failing tuple.

## Current failure — the exact case

First failing `checkmessage(prog, expected)` call:

```lua
prog     = table.sort({1,2,3}, table.sort)
expected substring = 'table.sort'
actual message     = bad argument #1 to '?' (table expected, got number)
```

So the runtime is catching the right semantic error: `table.sort` is being passed as the comparator, and when the sort engine invokes it as `cmp(a, b)` with two numbers, arg #1 is wrong (`number`, expected `table`). The bug is **name attribution** — the error says `'?'` instead of `'table.sort'`.

## What's actually wrong

The comparator is called from `sort_comp` (`crates/lua-stdlib/src/table_lib.rs:525`) via `state.push_value_at(2); state.call(2, 1)`. That call site invokes `table.sort` **as a value, not by name**, so by the time `arg_error_impl` (`crates/lua-vm/src/debug.rs:118`) tries to resolve a name via `get_info(state, b"n", &mut ar)`, `ar.namewhat` is `nil`/empty and the format string falls back to `'?'`.

C-Lua's `lauxlib.c:luaL_argerror` handles this by walking the debug info up one frame and using the C function's registered library name. We need the equivalent: for C/Rust functions that have a registered name in `package.loaded` or a known library table, resolve it.

## Suggested prompt for the next agent (sonnet or opus)

> Fix function-name attribution for C/Rust library functions invoked
> indirectly as values, so that `bad argument #1 to '?'` becomes
> `bad argument #1 to 'table.sort'` (etc.).
>
> Repro (do not modify the test file):
>
>   target/debug/lua-rs -e 'table.sort({1,2,3}, table.sort)'
>
> Expected: error message contains the substring `table.sort`.
>
> Likely fix site: `crates/lua-vm/src/debug.rs:118` `arg_error_impl`. When
> `get_info(state, b"n", &mut ar)` leaves `ar.namewhat` empty, look up the
> calling function's registered name. Reference C-Lua: `lauxlib.c:luaL_argerror`
> and the `findfield` helper in `ldblib.c` that walks `_G` / `package.loaded`
> to resolve a function value back to its dotted name.
>
> Smoke-test errors.lua afterwards — the failure point should move past
> line 30. Don't try to "fix errors.lua"; just fix this attribution case.

This is a tight, single-fix prompt. Should be a sonnet-tier job — opus is overkill.

## What past agents have tried

Five commits over the project (`f53fb1d`, `f289e4e`, `9a1c490`, `949cffa`, `71b48a8`, `67f00a4`). The O2 opus run fixed a real codegen bug in `cg_self` for high-index method constants (OP_SELF with `k_idx > MAXINDEXRK`). That advanced past one `checkmessage`, but the next one (the `table.sort` case above) hit immediately.

The recurring failure mode is: agent runs errors.lua → sees generic "assertion failed at line 30" → spends $10 of opus budget bisecting which of ~40 checkmessage calls is failing → finally finds the wording mismatch → fixes it → next round, same drill on the next mismatch. Pre-instrumenting (as we just did) cuts that loop entirely.

## Files most touched

`crates/lua-vm/src/debug.rs` (`arg_error_impl`, `get_info` consumers), `crates/lua-vm/src/state.rs` (`obj_type_name`), `crates/lua-parse/src/lib.rs` (codegen edges).
