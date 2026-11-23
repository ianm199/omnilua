# confirm-79.md — Error-message fidelity cluster (#79, R-D/E/F/G)

**Verdict: CONFIRMED.** All five sub-items reproduce as genuine, current
divergences against the pinned reference binaries on **5.4**, and (where
relevant) cross-version on 5.3 / 5.5 / 5.1 / 5.2. Verified `2026-05-31` with a
fresh `cargo build -p lua-cli` (`target/debug/lua-rs`) via
`specs/oracle/diff_one.sh` and direct binary invocation.

Headline repro:
```
diff_one.sh 5.4 'print(pcall(string.char,256))'
  OURS: false  bad argument #1 (value out of range)
  REF : false  bad argument #1 to 'string.char' (value out of range)
```

Four of five sub-items are **CLEAR-CUT** (the references agree across every
version we have, modulo per-version *wording* that we already version-gate
elsewhere). One — (d), the `[C]: in ?` traceback tail — is **RISKY /
architectural** (it requires invoking the main chunk beneath a C CallInfo frame,
i.e. a `pmain`-as-C-closure restructure of the CLI).

---

## Sub-item (a) — bad-argument errors drop `to '<fnname>'` and say `got nil` vs `got no value`

**CLEAR-CUT.** Two distinct defects in one path.

### (a1) Missing `to '<fnname>'` qualifier
```
diff_one.sh 5.4 'print(pcall(string.char,256))'
  OURS: false  bad argument #1 (value out of range)
  REF : false  bad argument #1 to 'string.char' (value out of range)
```
Cross-version reference behavior:
- 5.2.4 / 5.3.6 / 5.4.7 / 5.5.0 all print `to 'string.char'`.
- 5.1.5 prints `to '?'` (no `__name`/global-name resolution) — so the *form* is
  version-stable from 5.2 on; 5.1 would want `'?'`.

Root cause: `string.char` (and `utf8.char`) raise via the **static**
`LuaError::arg_error(narg, msg)` constructor, which has no `&LuaState` and
therefore cannot resolve the calling function's name. The state-aware auxlib
`arg_error` (which *does* emit `to '<fn>'`) is never reached on this path.

- `crates/lua-types/src/error.rs:74` — `LuaError::arg_error` → emits bare
  `bad argument #{n} ({msg})`.
- `crates/lua-stdlib/src/string_lib.rs:423` — `str_char` calls
  `LuaError::arg_error(i, "value out of range")`.
- `crates/lua-stdlib/src/utf8_lib.rs:293` — same, `utf_char`.
- Correct path already exists: `crates/lua-stdlib/src/auxlib.rs:331`
  (`arg_error`) and `:376` (`type_error_arg`) build the enriched
  `bad argument #{n} to '{fname}' ({msg})` via `get_info("n")` /
  `push_global_func_name`.

### (a2) `got nil` vs `got no value` for an absent argument
```
diff_one.sh 5.4 'print(pcall(string.sub))'
  OURS: false  bad argument #1 to 'string.sub' (string expected, got nil)
  REF : false  bad argument #1 to 'string.sub' (string expected, got no value)
diff_one.sh 5.4 'print(pcall(string.rep,"x"))'
  OURS: ...#2 to 'string.rep' (number expected, got nil)
  REF : ...#2 to 'string.rep' (number expected, got no value)
```
Universal across 5.1.5 / 5.2.4 / 5.3.6 / 5.4.7 / 5.5.0 (all print `got no
value`). C's `luaL_typeerror`/`tag_error` use `(arg > L->top - (ci->func+1))`
to detect an *absent* argument and substitute `"no value"` for the type name.

Root cause: `type_error_arg` computes the type name from
`state.type_name_at(arg)`, which reports `nil` for both a real nil and an
out-of-range (absent) argument — it never distinguishes "no value".

- `crates/lua-stdlib/src/auxlib.rs:376` (`type_error_arg`, line 386-394 type
  derivation) — must check whether `arg` is beyond the current frame's argument
  count and substitute `b"no value"`.
- Same fix conceptually applies to the inline `expected, got` sites in
  `crates/lua-vm/src/api.rs:512/544/569` and `crates/lua-stdlib/src/state_stub.rs:472/538/554/569`.

---

## Sub-item (b) — length / concat / string-arith-coercion errors drop `(command line):N:`

**CLEAR-CUT.** The location prefix is universal (present in 5.1.5 → 5.5.0).
Confirmed it is *subsystem-specific*: arith-on-nil, index-nil, call-nil, and
comparison errors already carry the prefix; only these three paths drop it.

```
diff_one.sh 5.4 'print(pcall(function() return #nil end))'
  OURS: false  attempt to get length of a nil value
  REF : false  (command line):1: attempt to get length of a nil value

diff_one.sh 5.4 'print(pcall(function() return {}..{} end))'
  OURS: false  attempt to concatenate a table value
  REF : false  (command line):1: attempt to concatenate a table value

diff_one.sh 5.4 'print(pcall(function() return "abc"+1 end))'
  OURS: false  attempt to add a 'string' with a 'number'
  REF : false  (command line):1: attempt to add a 'string' with a 'number'
```

Root cause: these callsites build the message with the **static, non-prefixing**
constructors (`LuaError::type_error`, `LuaError::concat_error`) or, for the
string-arith path, a bare `LuaError::runtime` — instead of the state-aware
`debug::type_error` / `prefixed_runtime` (the `luaG_runerror` equivalent that
prepends `short_src:line:`), or for the C-stdlib path the auxlib
`lua_error`/`push_where` (the `luaL_error` equivalent).

- **Length:** `crates/lua-vm/src/vm.rs:1295` and
  `crates/lua-vm/src/state.rs:2819, 2969` — use `LuaError::type_error(other,
  "get length of")` (static, no prefix). The prefixing variant is
  `crates/lua-vm/src/debug.rs:1324` (`type_error`, via `prefixed_runtime`).
- **Concat:** `crates/lua-vm/src/tagmethods.rs:493` — `LuaError::concat_error`
  (static, `crates/lua-types/src/error.rs:57`, no prefix). Needs to route
  through the state-aware concat error path so `prefixed_runtime` runs.
- **String-arith coercion failure:** `crates/lua-stdlib/src/string_lib.rs:504`
  (`trymt`) raises a bare `LuaError::runtime(...)`. Should raise via auxlib
  `lua_error` (`crates/lua-stdlib/src/auxlib.rs:435`, which calls `push_where`),
  exactly as C's `luaL_error` does. (See (c) for the operand bug on the same
  line.)

NB on wording for the string-arith family across versions: 5.4/5.5 use
`attempt to <op> a 'X' with a 'Y'`; 5.1/5.2/5.3 use `attempt to perform
arithmetic on a <type> value`. The string lib is built from each version's own
`lstrlib.c`, so the prefix fix must not perturb the per-version wording — add
the prefix only, mirroring each version's `luaL_error`.

---

## Sub-item (c) — arith/unary metamethod-failure mislabels operand types

**CLEAR-CUT** (a plain stack-management bug). Two visible symptoms, one cause.

```
diff_one.sh 5.4 'return -"x"'
  OURS: PROG: attempt to unm a 'string' with a 'function' | ...
  REF : PROG: (command line):1: attempt to unm a 'string' with a 'string' | ... | [C]: in ?

diff_one.sh 5.4 'print(pcall(function() return {}-"y" end))'
  OURS: false  attempt to sub a 'string' with a 'function'
  REF : false  (command line):1: attempt to sub a 'table' with a 'string'
```
(For 5.3 the *whole message* is `attempt to perform arithmetic on a table
value` — different wording, but ours is still wrong there too because the
underlying stack corruption feeds the wrong value to the formatter.)

Root cause — `crates/lua-stdlib/src/string_lib.rs:494` (`trymt`), lines
500-509. C's `trymt` short-circuits:
```c
if (lua_type(L,2)==LUA_TSTRING || !luaL_getmetafield(L,2,mtname))
   luaL_error(L, "attempt to %s a '%s' with a '%s'", mtname+2,
              luaL_typename(L,-2), luaL_typename(L,-1));
```
The `||` means `luaL_getmetafield` is **never called when arg2 is a string**.
Our Rust evaluates `state.get_meta_field(2, mtname)?` *unconditionally* on line
501 and binds it to `has_mm` before the `if`. When arg2 is a string, that call
finds the string metatable's own `__sub`/`__unm`/etc. and **pushes the
metamethod function onto the stack**. The error path then reads
`type_name_at(-2)` / `type_name_at(-1)`, which now point at `[arg2, pushed_fn]`
instead of `[arg1, arg2]` — hence the spurious `'function'` and the shifted
first operand. For `-"x"` (unm, one real operand duplicated to two by
`set_top(2)`) the same shift produces `'string' with a 'function'`.

Intended fix: make the metafield lookup lazy / short-circuited exactly like C —
do **not** call `get_meta_field` when `t2_is_string` is true, and when it *is*
called and pushes a value that we then reject, balance the stack before
formatting. Concretely, restructure to:
```
if t2_is_string || !state.get_meta_field(2, mtname)? {
    // stack is still [arg1, arg2]; format with -2 / -1
    return Err(via auxlib::lua_error(...));  // also fixes (b) prefix here
}
```
This single change fixes the operand mislabel (c) *and*, by routing through
`auxlib::lua_error`, the missing prefix (b) on the string-arith path.

---

## Sub-item (d) — uncaught errors omit the trailing `[C]: in ?` traceback frame

**RISKY / ARCHITECTURAL** (do not fix to match without the CLI restructure).
Real, deterministic, uniform across all versions.

```
$ lua5.4.7 -e 'error("boom")'
  (command line):1: boom
  stack traceback:
    [C]: in function 'error'
    (command line):1: in main chunk
    [C]: in ?            <-- present
$ LUA_RS_VERSION=5.4 lua-rs -e 'error("boom")'
  (command line):1: boom
  stack traceback:
    [C]: in function 'error'
    (command line):1: in main chunk
                        <-- MISSING
```
(5.5 reference also renames the error frame to `in global 'error'` — that is a
separate, already-tracked 5.5 frame-naming gap, not part of (d).)

Root cause: in C, the main chunk is run *inside* `pmain`, which is itself a C
closure pushed with `lua_pushcfunction(L,&pmain)` and invoked by `lua_pcall`
from `main()`. That `pmain` C CallInfo sits between `base_ci` and the main
chunk, and `lua_getstack` returns it as the last frame → `[C]: in ?`. Our CLI
`run()` (the `pmain` analogue) is a plain Rust function called directly from
`main` — there is **no C CallInfo above the main chunk**, so `get_stack`
correctly stops one frame early.

- `crates/lua-cli/src/interp.rs:613` (`run`, our `pmain`) is *not* pushed as a C
  closure / invoked via `pcall`; the main chunk is the bottom Lua frame.
- `crates/lua-vm/src/debug.rs:457` (`get_stack`) and `:477` (`is_base_ci` stop)
  faithfully mirror C `lua_getstack` (`ldebug.c:160`, which also excludes
  `base_ci`) — so `get_stack` is **correct** and must NOT be changed. The frame
  is genuinely absent from our CI chain.

Why risky: to produce `[C]: in ?` faithfully we must run the main chunk beneath
a real C CallInfo (push `run`/the chunk-runner as a C closure and `pcall` it, or
otherwise synthesize a base C frame). That touches the CallInfo chain and the
CLI entry path — higher blast radius than the wording fixes, and it interacts
with how `docall`/`msghandler` already wrap the call. Recommend a separate,
isolated change with its own oracle pass; do NOT bundle it with (a)/(b)/(c)/(e).

---

## Sub-item (e) — `table.concat` invalid-value error leaks the internal byte-array repr

**CLEAR-CUT.** Two defects: the byte-array leak and the missing prefix.
```
diff_one.sh 5.4 'print(pcall(table.concat,{ {} }))'
  OURS: false  invalid value ([116, 97, 98, 108, 101]) at index 1 in table for 'concat'
  REF : false  invalid value (table) at index 1 in table for 'concat'
```
`[116, 97, 98, 108, 101]` is the ASCII byte spelling of `table` — the formatter
debug-prints the type-name byte slice. Universal across versions.

Also confirmed (call-shape dependent): called via `pcall(table.concat, ...)` the
reference has **no** prefix (C function frame, no line); called from a Lua
function (`pcall(function() return table.concat({{}}) end)`) the reference
prints the `(command line):1:` prefix. So the prefix here is correct-by-context,
exactly like C's `luaL_error`.

Root cause — `crates/lua-stdlib/src/table_lib.rs:351`:
```rust
let type_name = state.type_name_str_at(-1);
return Err(LuaError::runtime(format_args!(
    "invalid value ({:?}) at index {} in table for 'concat'", type_name, idx)));
```
`{:?}` debug-formats the `&[u8]` as a byte array; it must be rendered as text
(`BStr(type_name)` / `{}`), and the error must route through `auxlib::lua_error`
(`crates/lua-stdlib/src/auxlib.rs:435`) so the location prefix appears when a
Lua caller is present, mirroring C's `luaL_error` at `lstrlib`/`ltablib.c`.

---

## Classification summary

| Sub-item | Defect | Verdict | Primary location |
|---|---|---|---|
| (a1) | missing `to '<fn>'` | CLEAR-CUT | `lua-types/src/error.rs:74`; callers `string_lib.rs:423`, `utf8_lib.rs:293` |
| (a2) | `got nil` vs `got no value` | CLEAR-CUT | `lua-stdlib/src/auxlib.rs:386-394` |
| (b) | missing `(command line):N:` on `#`/`..`/string-arith | CLEAR-CUT | `vm.rs:1295`, `state.rs:2819,2969`, `tagmethods.rs:493`, `string_lib.rs:504` |
| (c) | metamethod-failure mislabels operands | CLEAR-CUT | `string_lib.rs:494-509` (`trymt`) |
| (d) | missing `[C]: in ?` traceback tail | RISKY/ARCHITECTURAL | `interp.rs:613` (`run`/pmain not a C frame); `get_stack` is correct |
| (e) | `table.concat` leaks byte-array repr (+prefix) | CLEAR-CUT | `table_lib.rs:351` |

Per-version wording caveats (do not regress these while fixing):
- (a1) 5.1 wants `to '?'`, 5.2+ want the resolved name.
- (b)/(c) string-arith wording: 5.1/5.2/5.3 = `perform arithmetic on a <type>
  value`; 5.4/5.5 = `<op> a 'X' with a 'Y'`. Add prefix only; keep wording.
- (a2)/(e) wording is version-stable; safe everywhere.

None of these are CONTRACT-DEPENDENT on the reference binaries' compat flags
(unlike R-C `__le`-from-`__lt`). The `to '<fn>'`, `got no value`, location
prefix, plain type name, and operand-type labels are all default behavior of
every stock build 5.1–5.5.

---

## Intended fix (summary)

1. **(a1)** Route `str_char`/`utf_char` range errors through the state-aware
   auxlib `arg_error` (or a new `state`-taking `LuaError` helper) so the
   function name is resolved. Audit other `LuaError::arg_error`/`type_arg_error`
   static-constructor callsites for the same gap.
2. **(a2)** In `type_error_arg`, detect an absent argument
   (`arg > nargs_in_current_frame`) and substitute `b"no value"` for the type
   name; thread the same check into the inline `api.rs`/`state_stub.rs` sites.
3. **(b)** Switch length (`vm.rs:1295`, `state.rs:2819,2969`) and concat
   (`tagmethods.rs:493`) to the prefixing `debug::type_error` / a
   `prefixed_runtime`-based concat error; switch `trymt` (string-arith) to
   `auxlib::lua_error`.
4. **(c)** Short-circuit the metafield lookup in `trymt` so `get_meta_field` is
   not called when arg2 is a string, keeping `[arg1,arg2]` on the stack for the
   formatter (and balancing the stack when a pushed metamethod is rejected).
5. **(e)** Render the type name as text (`{}` + `BStr`) and raise via
   `auxlib::lua_error` for the contextual prefix.
6. **(d)** Separate change: run the main chunk beneath a synthesized base C
   CallInfo (push the chunk-runner as a C closure / `pcall` it from a `pmain`
   C-closure entry) so `get_stack` enumerates the `[C]: in ?` frame. Gate with
   its own oracle pass.

Shared-core caution: (b) and (c) touch code paths exercised by 5.3/5.4/5.5; each
fix must match **every** version reference (preserve per-version wording above),
verified with `check.sh 5.4 5.3 5.5`.

---

## CI test assertions to add

Extend `crates/lua-rs-runtime/tests/multiversion_oracle.rs` using the existing
`run`/`err_contains` helpers. The `run` wrapper `pcall`s the snippet, so it
captures the error *message* — sufficient for (a)/(b)/(c)/(e). Sub-item (d)
asserts the *uncaught top-level traceback*, which `pcall` swallows; it needs a
separate CLI-level test (spawn `target/debug/lua-rs -e ...`, assert stderr
contains the trailing `\t[C]: in ?`), so it is listed separately and should be
added only alongside the (d) fix.

```rust
// ── #79 error-message fidelity (R-D/E/F/G) ──────────────────────────────

#[test]
fn v54_argerror_to_fnname() {
    // (a1) bad-argument carries `to '<fn>'`
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return string.char(256)", "to 'string.char'");
        err_contains(v, "return string.char(256)", "value out of range");
    }
}

#[test]
fn v54_argerror_no_value() {
    // (a2) absent argument => `got no value`, not `got nil`
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return string.sub()", "got no value");
        err_contains(v, "return string.rep('x')", "got no value");
    }
}

#[test]
fn v54_length_concat_location_prefix() {
    // (b) `#`, `..` carry the chunk-location prefix.
    // The wrapper chunk name is the load() chunk, so assert the generic ":" form
    // plus the message; tighten to the exact `[string ...]:N:` if the harness
    // chunk name is pinned.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        // message body present
        err_contains(v, "return #nil", "attempt to get length of a nil value");
        err_contains(v, "return ({})..({})", "attempt to concatenate a table value");
        // prefix present (a ':<digits>:' appears before the message)
        let e = run(v, "return #nil").unwrap_err();
        assert!(e.contains(':') && e.find("attempt").map_or(false,
            |i| e[..i].contains(':')), "v{v:?} #nil missing location prefix: {e}");
    }
}

#[test]
fn v54_string_arith_coercion_failure() {
    // (b)+(c) string-arith failure: prefix present, operands labeled correctly.
    // 5.4/5.5 use `<op> a 'X' with a 'Y'`.
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return ({}) - 'y'", "attempt to sub a 'table' with a 'string'");
        err_contains(v, "return -'x'", "attempt to unm a 'string' with a 'string'");
    }
    // 5.3 uses the legacy wording.
    err_contains(LuaVersion::V53, "return ({}) - 'y'",
        "attempt to perform arithmetic on a table value");
}

#[test]
fn v54_table_concat_invalid_value_type_name() {
    // (e) plain type name, no byte-array leak.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return table.concat({ {} })",
            "invalid value (table) at index 1 in table for 'concat'");
        // negative guard: the byte-array repr must NOT appear.
        let e = run(v, "return table.concat({ {} })").unwrap_err();
        assert!(!e.contains('['), "v{v:?} concat leaked byte-array: {e}");
    }
}

// (d) — uncaught traceback tail `[C]: in ?` — ADD WITH THE (d) FIX ONLY.
// Not expressible via the pcall-based `run` helper. Implement as a CLI test:
// spawn `target/debug/lua-rs -e 'error("boom")'`, assert stderr ends with a
// line `\t[C]: in ?` (matching lua5.4.7 / lua5.3.6 / lua5.5.0).
```

Final-gate reminder (all must stay green):
`cargo build --workspace` ;
`cargo test --workspace --features lua-rs-runtime/derive` ;
`specs/oracle/check.sh 5.4 5.3 5.5`.
