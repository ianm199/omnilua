# Confirm #76 — `math.type` / `math.tointeger` return `false` instead of `nil`

**Status: CONFIRMED, current, reproduces on every version. CLEAR-CUT to fix.**

## What the bug is

On a failure (non-number argument to `math.type`, or a value not convertible to
an integer for `math.tointeger`), our implementation pushes `false`. Every
default-build reference returns `nil`. The "PORT NOTE" comments in our source
that justify pushing `false` are **factually wrong** about `luaL_pushfail`.

## Repros + our-vs-reference outputs

Captured with `specs/oracle/diff_one.sh <ver> <code>` (our version-selected
`lua-rs` vs the matching unmodified `make macosx` reference binary). Every row
below is a DIFF (exit codes match; values differ).

| code | ours (all vers) | ref 5.3.6 | ref 5.4.7 | ref 5.5.0 |
|---|---|---|---|---|
| `print(math.type("x"))` | `false` | `nil` | `nil` | `nil` |
| `print(math.type(true))` | `false` | `nil` | `nil` | `nil` |
| `print(math.tointeger(3.5))` | `false` | `nil` | `nil` | `nil` |
| `print(math.tointeger(2^63))` | `false` | `nil` | `nil` | `nil` |

Sanity rows that already MATCH (not part of the bug): `math.tointeger("7")`
returns `7` on every version (valid conversion → integer path, never touches the
fail branch).

5.1/5.2 references are not relevant: `math.type` and `math.tointeger` do not
exist before 5.3, so there is no failure-sentinel to match.

## Why it is CLEAR-CUT (not contract-dependent)

Verified at C-source level in the pinned references, not inferred:

- **5.3** (`lmathlib.c`): both functions call `lua_pushnil(L)` explicitly on the
  fail branch. Unconditional nil.
- **5.4** (`lmathlib.c`): both call `luaL_pushfail(L)`. In `lauxlib.h` for
  5.4.7, `#define luaL_pushfail(L) lua_pushnil(L)` — unconditional, no
  `#if`. So 5.4 = nil.
- **5.5** (`lmathlib.c`): both call `luaL_pushfail(L)`. In `lauxlib.h` for
  5.5.0 it is guarded: `lua_pushboolean(L, 0)` **only if `LUA_FAILISFALSE` is
  defined**, else `lua_pushnil(L)`. The default `make macosx` build does not
  define `LUA_FAILISFALSE` (our oracle contract = unmodified build), so 5.5 =
  nil.

`nil` is the correct answer for all three contract targets. There is no version
under which the oracle expects `false`, so a single shared-core fix to push
`nil` matches **every** reference. The `LUA_FAILISFALSE` knob is the only thing
that would change this, and the oracle contract pins it off; if a 5.5
`LUA_FAILISFALSE` build is ever a target it would be a separate, opt-in
behavior — not in scope and not what users get by default.

The current `false` behavior is also internally inconsistent: it produces a
*truthy-context* divergence (`if math.tointeger(x) then` takes the wrong branch
under our impl vs reference), so it is a real behavioral bug, not cosmetic.

## Impl location(s)

`crates/lua-stdlib/src/math_lib.rs`:

- `math_toint` — line **246**: `state.push(LuaValue::Bool(false));`
  (comment line 245: "PORT NOTE: luaL_pushfail in Lua 5.4 pushes false (not nil)." — incorrect)
- `math_type` — line **430**: `state.push(LuaValue::Bool(false));`
  (comment line 429: "PORT NOTE: luaL_pushfail pushes false in Lua 5.4.4+." — incorrect)

## Intended fix

In both functions, replace the failure push with `nil` and correct the
misleading comments:

- line 245-246 (`math_toint`): replace `LuaValue::Bool(false)` with
  `LuaValue::Nil`; comment should read that `luaL_pushfail` is `lua_pushnil` in
  the default 5.3/5.4/5.5 builds (only `false` under `LUA_FAILISFALSE`, which the
  oracle contract pins off).
- line 429-430 (`math_type`): same change.

`check_any(1)?` stays (argument-presence check is correct and version-agnostic).

This is shared-core (one backend serves all versions today), and the fix is
correct for all three references simultaneously — no per-version branching
needed.

## CI assertions to add

Extend `crates/lua-rs-runtime/tests/multiversion_oracle.rs` using the existing
`eq` helper and `Lua::new_versioned`. `tostring(nil)` renders `"nil"`. Add a test
covering all three versions:

```rust
/// #76: math.type / math.tointeger return `nil` (not `false`) on failure.
/// luaL_pushfail = lua_pushnil in the default 5.3/5.4/5.5 builds (oracle
/// contract pins LUA_FAILISFALSE off). Pre-existing 5.4 port bug.
#[test]
fn issue76_math_fail_returns_nil() {
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(v, "return math.type('x')",        "nil");
        eq(v, "return math.type(true)",       "nil");
        eq(v, "return math.tointeger(3.5)",   "nil");
        eq(v, "return math.tointeger(2^63)",  "nil");
        // guard the success paths still work (regression fence):
        eq(v, "return math.tointeger('7')",   "7");
        eq(v, "return math.type(1)",          "integer");
        eq(v, "return math.type(1.0)",        "float");
    }
}
```

(Optional truthiness fence to lock the *semantic* intent, not just tostring:
`eq(v, "return math.type('x') == nil", "true")` and
`eq(v, "if math.tointeger(3.5) then return 'truthy' else return 'falsey' end", "falsey")`.)

## Gate after fixing

`cargo build --workspace` ;
`cargo test --workspace --features lua-rs-runtime/derive` ;
`specs/oracle/check.sh 5.4` / `5.3` / `5.5`. The shared-core change must keep
every version green.
