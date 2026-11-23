# Shared-core triage 2 — items B, E, C (stdlib-err)

Read-only triage. Every claim below was reproduced against the unmodified
reference binaries in `/tmp/lua-refs/bin` via
`specs/oracle/diff_one.sh <ver> '<code>'`. No source was edited.

Reference binaries: lua5.3.6, lua5.4.7, lua5.5.0 (5.1.5 / 5.2.4 also present).
lua-rs binary: `target/debug/lua-rs`, version via `LUA_RS_VERSION`.

---

## Item B — `luaL_argerror` funcname / value / position omission

### Affected versions
5.3, 5.4, 5.5 (all three; this is a shared-core defect).

### Exact reference strings (confirmed)

`collectgarbage("bogusopt")` — all three versions:

```
PROG: (command line):1: bad argument #1 to 'collectgarbage' (invalid option 'bogusopt')
```

lua-rs emits instead (all three):

```
PROG: bad argument #1 (invalid option)
```

Three independent omissions in that one line:
1. the `(command line):1:` position prefix (from `luaL_where`),
2. the `to 'collectgarbage'` function-name clause,
3. the offending value `'bogusopt'` inside the parentheses.

`utf8.offset("abc", 0, 0)`:

| ver | reference | lua-rs |
|---|---|---|
| 5.3 | `... bad argument #3 to 'offset' (position out of **range**)` | `bad argument #3 (position out of **bounds**)` |
| 5.4 | `... bad argument #3 to 'offset' (position out of **bounds**)` | `bad argument #3 (position out of bounds)` |
| 5.5 | `... bad argument #3 to 'offset' (position out of bounds)` | same omission |

Note the **5.3-only wording** difference: ref says `out of range`, 5.4/5.5 say
`out of bounds`. lua-rs hardcodes `out of bounds`, so even after the
funcname/position fix, 5.3 stays wrong on the inner word.

`string.format("%200d", 1)` (width too long):

| ver | reference | lua-rs |
|---|---|---|
| 5.3 | `... invalid format (width or precision too long)` | `invalid conversion specification` |
| 5.4 | `... invalid conversion specification: '%200d'` | `invalid conversion specification` |
| 5.5 | `... invalid conversion specification: '%200d'` | `invalid conversion specification` |

Two distinct issues here, on top of the position prefix:
- 5.4/5.5 ref appends `: '%200d'` (the offending spec). lua-rs omits it.
- 5.3 uses an entirely different message (old `scanformat` path:
  `invalid format (width or precision too long)`) vs the 5.4+ `checkformat`
  path. lua-rs only implements the 5.4+ wording for all versions.

`string.format("%d", "x")` (type mismatch) already **MATCHes** on all three —
that path routes through the faithful `auxlib::type_error_arg → arg_error`.

### Responsible code & root cause

There are **two parallel arg-error mechanisms** and the GC/option/utf8/format
callsites use the wrong one:

- Faithful path (already correct): `crates/lua-stdlib/src/auxlib.rs`
  - `arg_error` (line 342) — already builds `bad argument #N to '<fn>' (<extra>)`,
    walks the stack via `get_info`/`push_global_func_name` for the name.
  - `check_option` (line 568) — already builds `invalid option '<value>'` with the
    value interpolated.
  - `lua_error`/`push_where` (line 427/448) — already prepend `<src>:<line>:`.
  These are the C-faithful translations and they are correct.

- Stub path (the bug): the `LuaState` trait method `check_arg_option` resolves
  to `crates/lua-stdlib/src/state_stub.rs:659`, which on failure calls
  `LuaError::arg_error(arg, "invalid option")` — a fast constructor in
  `crates/lua-types/src/error.rs:74`:
  ```rust
  pub fn arg_error(narg: i32, msg: &str) -> Self {
      LuaError::runtime(format_args!("bad argument #{} ({})", narg, msg))
  }
  ```
  This constructor (a) hardcodes the bare `bad argument #N (msg)` shape with no
  `to '<fn>'`, (b) has no `LuaState`, so it cannot run `luaL_where` (no position
  prefix) nor walk the call stack for the function name, and (c) is passed the
  literal `"invalid option"` with no value, dropping `'bogusopt'`.

Callsites that bypass the faithful path via `LuaError::arg_error`:
- `collectgarbage` → `base.rs:420` `state.check_arg_option(...)` → `state_stub.rs:659`.
- utf8.offset → `utf8_lib.rs:194/198/203/345/349` call `LuaError::arg_error(..)` directly.
- string.format width → `string_lib.rs:1693/1711/2036` raise
  `LuaError::runtime("invalid conversion specification")` directly (no value,
  no `arg_error` at all).
- Many other `LuaError::arg_error(..)` sites exist (base.rs, debug_lib.rs,
  type_fn) and share the same omission whenever they fire.

### Verdict: CLEAR-CUT (cross-cutting, but localized seams)

Not architectural. The faithful machinery already exists in `auxlib.rs`; the
fix is to route the stub callsites through it. Two edit seams:

1. **The shared constructor** `lua-types/src/error.rs:74` cannot self-enrich
   (no state). The faithful behavior must come from a state-aware helper. The
   cleanest seam: make `state_stub::check_option` call the auxlib
   `check_option`/`arg_error` (which already produce value + funcname + position)
   instead of `LuaError::arg_error`; likewise have utf8/format callsites call
   `auxlib::arg_error(state, arg, msg)` so `luaL_where` + name-walk run.
2. **Per-version wording**: gate utf8.offset inner word
   (`range` on 5.3, `bounds` on 5.4/5.5) and the format-width message
   (`invalid format (width or precision too long)` on 5.3 vs
   `invalid conversion specification: '<spec>'` on 5.4/5.5, including the spec)
   on `state.global().lua_version`.

Existing CI guard to update: `multiversion_oracle.rs:780/782` currently asserts
the substring `"invalid option"` only — it passes today *because* the message is
truncated. Tighten it to the full reference string when fixing.

---

## Item E — `print` must call the global `tostring` (5.3 only)

### Affected versions
**5.3 ONLY.** 5.4 and 5.5 already MATCH.

### Confirmed behavior

| code | 5.3 ref | 5.3 lua-rs | 5.4/5.5 |
|---|---|---|---|
| `tostring=nil; print(1)` | errors `attempt to call a nil value` (exit 1) | prints `1` (exit 0) | MATCH (both prints) |
| `tostring=function(x) return "X"..x end; print(7)` | `X7` | `7` | MATCH (both `7`) |
| global tostring override + `__tostring` value | `OVERRIDE` | `META` | MATCH |

### Root cause (a genuine cross-version mechanism split)

- Lua **5.3** `luaB_print` (lbaselib.c) fetches the **global** `tostring`
  (via an upvalue) and *calls* it on each argument — so redefining global
  `tostring` changes `print`, and a `nil` global makes `print` raise
  `attempt to call a nil value`.
- Lua **5.4 / 5.5** `luaB_print` uses `luaL_tolstring` directly: it honors the
  `__tostring` / `__name` metafields but **ignores** the global `tostring`.

lua-rs implements only the 5.4/5.5 mechanism on all versions:
`crates/lua-stdlib/src/base.rs:189` `print_fn` always calls
`state.to_display_string(i)` (= `luaL_tolstring`). It never consults the global
`tostring`, so on 5.3 it neither errors on nil nor respects an override.

### Verdict: CLEAR-CUT, localized

Single function, version-gated. Edit seam: `base.rs:print_fn`. On
`lua_version == V53`, replicate 5.3 `luaB_print`: look up global `tostring`,
push it, push the argument, `pcall`/call it (raising
`attempt to call a nil value` if it is nil), and write the string result.
Keep the existing `to_display_string` path for 5.4/5.5. (Faithful note: 5.3
print uses the *upvalue* captured at lib-open time, not a live `_ENV` lookup;
in practice both observe the same global table, so the global-fetch is
behaviorally equivalent for the oracle.)

---

## Item C — default GC mode (5.4 / 5.5 default to generational)

### Affected versions
5.4, 5.5. (5.3 has no mode switching — `incremental`/`generational` are invalid
options there, confirmed.)

### Confirmed reference behavior (the divergence IS observable)

`collectgarbage("incremental")` / `("generational")` return the **previous**
mode as a string. The first call therefore reveals the default:

```
5.4.7:  print(collectgarbage("incremental")) -> generational
5.5.0:  print(collectgarbage("incremental")) -> generational
```

Full sequence on both 5.4.7 and 5.5.0:
```
first ->incremental : generational   (default was generational)
then  ->generational : incremental
then  ->incremental  : generational
```

lua-rs reports `incremental` as the default on both 5.4 and 5.5:
```
OURS 5.4/5.5: print(collectgarbage("incremental")) -> incremental
sequence OURS: incremental | incremental   (REF: generational | incremental)
```

After the first explicit switch the sequences re-converge — the only divergence
is the **initial mode string**.

`collectgarbage("isrunning")` returns `true` on all three for both ref and
lua-rs — already MATCH; not part of this defect.

(Aside: the Lua 5.4 *manual* text says incremental is the default; this pinned
5.4.7 build reports generational. The oracle is the oracle — match the binary.)

### Responsible code

- Initial mode: `crates/lua-vm/src/state.rs:4590` `gckind: GcKind::Incremental as u8`
  in the global-state constructor. This is the single source of the default.
- Mode read/return: `crates/lua-vm/src/api.rs:2020` (Gen) and `:2031` (Inc)
  compute `old_mode` from `state.global().is_gen_mode()` (which reads `gckind`,
  `state.rs:1456`), then `base.rs:67 push_mode` maps 10→`generational`,
  11→`incremental`.

### Is it observable/testable without retuning the real collector? — YES for the
string, but flipping the default is NOT behavior-free.

`gckind` is read in only **three** places, all in `api.rs`: the two mode-switch
arms (2020/2031) and the `step` arm at `api.rs:1979`. There is **no
generational collector** in `lua-gc` (no `youngcollection` / minor-collection /
`genstep` logic anywhere). So `gckind` is almost purely cosmetic — *except* the
`step` arm at `api.rs:1979`:
```rust
if state.global().is_gen_mode() {
    state.gc().prune_weak_tables_mark_only();
}
```
If the default flips to `Generational`, `collectgarbage("step")` would take that
weak-table-pruning branch from the very first call on 5.4/5.5, a real (if small)
collector-behavior change with no faithful generational machinery behind it.

### Verdict: RISKY — document, do not naively flip the constant.

Matching the *reported* default string is trivial (it is just a field), but the
honest fix is not "change `GcKind::Incremental` → `Generational` at state.rs:4590"
because that also rewires the live `step` branch toward a generational collector
lua-rs does not actually have.

Two defensible options for the implementer (decide deliberately):
- **(a) String-only faithfulness:** keep the real collector incremental, but make
  the *default mode reported* by `collectgarbage` queries be `generational` on
  5.4/5.5 — e.g. version-gate the initial `gckind` to Generational AND make the
  `step` weak-prune branch independent of the default (guard on whether an
  explicit incremental switch has happened, or simply always prune as the
  incremental collector already does). This matches the oracle string without
  pretending to have a generational collector. Verify `gc.lua` / `step` parity
  after.
- **(b) Document as a known divergence** in the GC-mode reporting and leave the
  default incremental, on the grounds that faking generational defaults invites
  drift. Given the project rule "if RISKY, document precisely rather than fake
  it," (b) is the conservative default; (a) is acceptable only if `step`/`gc.lua`
  oracle parity is re-confirmed on 5.4 and 5.5.

Edit seam if pursuing (a): `crates/lua-vm/src/state.rs:4590` (version-gate the
init — note the constructor may not know the version at that point; check whether
`lua_version` is set before `gckind`), plus `crates/lua-vm/src/api.rs:1979`
(decouple the `step` weak-prune from the default mode). No `lua-gc` change is
needed for the string, which is exactly why it is tempting and why it must be
done carefully rather than by flipping the enum.

---

## One-line-per-item summary

- **B** — versions 5.3/5.4/5.5; CLEAR-CUT (cross-cutting); seam: route
  `state_stub::check_option` + utf8.offset + string.format-width callsites
  through `auxlib::arg_error`/`check_option` instead of the state-less
  `LuaError::arg_error` (`lua-types/src/error.rs:74`); add per-version wording
  (utf8 range/bounds; format-width message + offending spec).
- **E** — version 5.3 ONLY (5.4/5.5 already match); CLEAR-CUT; seam:
  version-gate `base.rs:print_fn` to fetch+call the global `tostring` on 5.3
  (error if nil), keep `to_display_string` for 5.4/5.5.
- **C** — versions 5.4/5.5; RISKY; observable (mode-switch returns prior mode,
  first call reveals default = `generational`); seam: `state.rs:4590` initial
  `gckind` + `api.rs:1979` step weak-prune branch; recommend DOCUMENT (or
  string-only fix that decouples `step` from the default) — do not flip the enum
  blindly, lua-rs has no real generational collector.
