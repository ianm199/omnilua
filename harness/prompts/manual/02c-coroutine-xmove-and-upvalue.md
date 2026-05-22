# Phase E-3: xmove + cross-thread upvalue fix

**Order: next after slices 02a + 02b (both already on main).**

Authoritative design: `docs/LUA_PHASE_E_RUNTIME_SPEC.md` Part 1, especially "Coroutine Implementation Slices" §2. Plus the prior slices already merged.

## Prerequisites (verify on entry)

```bash
git log --oneline main | grep -E "thread identity|resume/yield"
target/debug/lua-rs -e 'local co = coroutine.create(function() coroutine.yield(1,2) end); print(coroutine.resume(co))'
```

Both must succeed. You should see commits `1dbaa1d` (thread identity) and `452a433` (resume/yield) on main, and the resume call should print `true  1  2`.

## Task

Fix two things in one slice:

1. **Cross-thread upvalue access (the bug 02b flagged)** — `LuaState::upvalue_get` and `upvalue_set` read/write `self.stack[idx]` from the *current* thread, ignoring the `thread_id` carried in `UpValState::Open { thread_id, idx }`. So when a coroutine reads an upvalue captured from the parent thread, it indexes its own stack at that slot — reading garbage.

2. **`xmove` between threads** — move N stack values from one `LuaState` to another, both sharing the same `GlobalState`. C-Lua: `lua_xmove`. Currently stubbed or missing.

## Concrete fix sites (READ THESE FIRST before exploring)

### Cross-thread upvalue bug

`crates/lua-vm/src/state.rs:1833-1855` — `upvalue_get` and `upvalue_set`.

Current `upvalue_get` (line 1838):
```rust
lua_types::UpValState::Open { thread_id: _, idx } => self.stack[idx.0 as usize].val.clone(),
```

The `thread_id: _` discards the thread identifier and reads from `self.stack` (current thread). Same bug in `upvalue_set` at line 1848 (`self.set_at(idx, val)`).

Fix: when `thread_id != self.current_thread_id()`, resolve the owning thread via `GlobalState::threads`. Slice 02a stored child threads as `Rc<RefCell<LuaState>>` (02b changed that from `GcRef<LuaState>`) — borrow the target thread and read/write its `stack[idx]`.

Beware:
- `self.global()` returns a borrow; `thread.borrow()` is a second borrow on a different RefCell. Should be safe if you drop the global borrow before borrowing the thread.
- Main thread (id 0) is NOT stored in `GlobalState::threads` (per 02a's design). If `thread_id == 0`, the originating thread is the main thread — there must be a separate handle for that (check `main_thread_id` / how 02a resolves main).
- Closures captured in the main thread reading from inside a coroutine still must read main's stack, not the current coroutine's.
- When current_thread_id == thread_id, the fast path is `self.stack[idx]` — preserve that.

### xmove

C-Lua reference: `lapi.c::lua_xmove`. The op:
1. Both threads must share the same `lua_State -> global_State`.
2. Pop N values from source's top.
3. Push them onto target's top.
4. Leave source's stack top decremented by N, target's top incremented by N.

Implementation: search `crates/lua-vm/src/api.rs` for any `xmove` stub. If not present, add a free function `lua_xmove(from: &mut LuaState, to: &mut LuaState, n: i32)`. Argument may need to be `&mut self` on one and `&mut LuaState` on the other — keep both borrows out of the `GlobalState::threads` borrow path (i.e. get raw `Rc<RefCell<LuaState>>` and borrow them mutably for the duration). Single-threaded so the borrows are uncontested.

Coroutine `coroutine.create` and `coroutine.resume` should now use `xmove` internally where they manually copy stack slots between threads (look for `value_at` + `push` patterns in `coro_lib.rs`).

## Scope

- `crates/lua-vm/src/state.rs` — `upvalue_get` / `upvalue_set` rewrite (~30 LOC delta)
- `crates/lua-vm/src/api.rs` — add `lua_xmove` (~40 LOC)
- `crates/lua-stdlib/src/coro_lib.rs` — use `lua_xmove` where coroutine stack-shuffling currently happens manually
- Audit any other `self.stack[idx]` access in upvalue / call-frame paths that might also need cross-thread routing (grep for `UpValState::Open`)

## Acceptance test

```bash
target/debug/lua-rs -e '
-- Cross-thread upvalue: coroutine reads upvalue captured from parent
local x = 100
local co = coroutine.create(function()
  for i = 1, 3 do
    coroutine.yield(x + i)
    x = x + 1000
  end
end)
local ok, a = coroutine.resume(co); assert(ok and a == 101, "got "..tostring(a))
local ok, b = coroutine.resume(co); assert(ok and b == 1102, "got "..tostring(b))
local ok, c = coroutine.resume(co); assert(ok and c == 2103, "got "..tostring(c))
print("upvalue cross-thread ok")

-- xmove smoke
-- Direct lua-side test is harder since xmove is a C-API op; coroutine.create
-- itself uses it. If coroutine.create + resume works with multi-value args
-- and the function reads them correctly, xmove is working.
local co2 = coroutine.create(function(a, b, c)
  return a + b + c
end)
local ok, r = coroutine.resume(co2, 10, 20, 30)
assert(ok and r == 60, "got "..tostring(r))
print("multi-arg resume ok")
'
```

Also run `reference/lua-c/testes/coroutine.lua` and report new failure line (was 56 before — should advance significantly).

## What NOT to do

- Don't introduce a new "thread context" parameter to every state method — keep the routing localized to upvalue ops
- Don't break the single-thread-fast-path: if `thread_id == self.current_thread_id()`, read `self.stack` directly (avoid the `GlobalState::threads` lookup)
- Don't add `unsafe`
- Don't try to make all of `coroutine.lua` pass; just move the failure point and confirm cross-thread upvalues work
- Don't touch `lua-coro` — slice 02e (corosensei) is separate

## Report on exit

- Acceptance test result
- Files changed + LOC delta
- New `coroutine.lua` failure line (was 56)
- Any tests in mega_loop's PROGS that now pass that didn't before
- Whether xmove found other internal callers in coro_lib that you migrated
- Any cross-thread access patterns OTHER than upvalues that you flagged but didn't fix

Budget: $20. This is a narrow surgical fix, not a refactor.
