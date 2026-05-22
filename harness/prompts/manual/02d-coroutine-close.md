# Phase E-4: coroutine.close + to-be-closed variables

**Order: after 02a, 02b, 02c.** Can run in parallel with 02c if you're careful about merge order — they touch overlapping files (coro_lib.rs, state.rs).

Authoritative design: `docs/LUA_PHASE_E_RUNTIME_SPEC.md` Part 1, especially "Coroutine Implementation Slices" §5 (lines 371-374).

## Prerequisites (verify on entry)

```bash
git log --oneline main | grep -E "thread identity|resume/yield"
```

02a and 02b must already be on main. 02c is preferred but not strictly required — this slice doesn't depend on cross-thread upvalues.

## Task

Implement `coroutine.close(co)` properly and ensure `__close` metamethods run on suspended or dead coroutines that own to-be-closed (`<close>`) variables.

C-Lua reference: `lcorolib.c::luaB_close`, `ldo.c::luaE_resetthread`, `ldo.c::luaF_close`.

Behavior contract:

1. `coroutine.close(co)` is valid only when `co` is **suspended** (`Yield` status) or **dead** (`Ok` with no active frames). Calling it on a *running* or *normal* (parent of running) coroutine must raise: `attempt to close a normal coroutine`.

2. On valid call:
   - Run all `__close` metamethods for any to-be-closed variables remaining on the coroutine's stack, in LIFO order.
   - Reset the coroutine's status to `Ok`.
   - Empty its stack.
   - If any `__close` raises, `coroutine.close` returns `(false, errobj)`; otherwise `(true)`.

3. A *dead* coroutine that died via an uncaught error should have `coroutine.close` propagate the original error message — see C-Lua's exact behavior for the (false, errobj) shape.

## Current state to investigate (READ FIRST)

- `crates/lua-stdlib/src/coro_lib.rs` — find `coro_close` (or whatever the function is named). Slice 02a left it as a stub-error. Wire it up here.
- `crates/lua-vm/src/state.rs` — look for `close_thread` / `reset_thread` (per the spec doc's "Coroutine Implementation Slices" §5 mention). May be skeleton/stub.
- `crates/lua-vm/src/func.rs` — `luaF_close` translation. Search for `tbc` or `to_be_closed` or `tbc_delta` (per state.rs `StackValue.tbc_delta` field).
- `crates/lua-vm/src/do_.rs` — error propagation through close paths.

## Scope

- `crates/lua-stdlib/src/coro_lib.rs` — implement `coro_close` (full), wire to a new `LuaState::close_thread` helper
- `crates/lua-vm/src/state.rs` — `close_thread` / `reset_thread` real impl, status transitions
- `crates/lua-vm/src/func.rs` — ensure `luaF_close` handles the "iterate from top down, call each __close, propagate errors as you go" loop correctly for non-current threads (analogous to the cross-thread upvalue fix in 02c)
- Tests: targeted Lua repros

## Acceptance test

```bash
target/debug/lua-rs -e '
-- Close a suspended coroutine — __close should fire on tbc local
local closed = {}
local mt = {__close = function(o) table.insert(closed, o.id) end}
local co = coroutine.create(function()
  local _ <close> = setmetatable({id="a"}, mt)
  local _ <close> = setmetatable({id="b"}, mt)
  coroutine.yield()
end)
local ok = coroutine.resume(co)
assert(ok and coroutine.status(co) == "suspended")
local ok, err = coroutine.close(co)
assert(ok, "close failed: "..tostring(err))
assert(coroutine.status(co) == "dead")
-- LIFO: b closed first, then a
assert(closed[1] == "b" and closed[2] == "a", "order: "..table.concat(closed,","))
print("close on suspended ok")

-- Close on dead coroutine succeeds with no side-effects
local co2 = coroutine.create(function() end)
local ok = coroutine.resume(co2)
assert(coroutine.status(co2) == "dead")
local ok2 = coroutine.close(co2)
assert(ok2)
print("close on dead ok")

-- Close on running coroutine errors
local co3 = coroutine.create(function() coroutine.close(coroutine.running()) end)
local ok, err = coroutine.resume(co3)
assert(not ok and string.find(tostring(err), "normal coroutine") or string.find(tostring(err), "running"))
print("close on running rejected ok")

-- Close propagates __close errors
local co4 = coroutine.create(function()
  local _ <close> = setmetatable({}, {__close = function() error("boom") end})
  coroutine.yield()
end)
coroutine.resume(co4)
local ok, err = coroutine.close(co4)
assert(not ok and string.find(tostring(err), "boom"))
print("close error propagation ok")
'
```

All four sections must print their `ok` line.

Also run `reference/lua-c/testes/coroutine.lua` again; report new failure line (after 02c lands).

## What NOT to do

- Don't add native stack switching — this is still the heap-backed coroutine path
- Don't add `unsafe`
- Don't try to handle every edge case of `__close` chaining error semantics; the C-Lua-spec compliant version is enough
- Don't touch `lua-coro` crate — that's slice 02e if needed
- Don't break basic `coroutine.resume`/`yield` (slice 02b's acceptance must still pass after this slice)

## Report on exit

- Acceptance test result (each of the 4 sections)
- Files changed + LOC delta
- New coroutine.lua failure line
- Any other tests in mega_loop's PROGS that now pass that didn't before
- Notes on tricky parts (especially the "close while target is suspended on a different thread's stack" loop)

Budget: $20.
