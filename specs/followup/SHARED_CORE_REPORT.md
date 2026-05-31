# Shared-Core Fidelity Report

Branch: `shared-core-fidelity` (off `main`, v0.0.21+). Oracle: the unmodified
`make macosx` reference binaries in `/tmp/lua-refs/bin` (`lua5.3.6` / `lua5.4.7`
/ `lua5.5.0`). Every expected value below was captured from a reference binary
via `specs/oracle/diff_one.sh`. The brief is cross-version fidelity: each fix
must match **every** affected version, never regress another.

This pass found that items A, B, D, E, F, and the `[C]: in ?` traceback frame
were already landed on the branch (commits `1c0ce30`, `0668e59`, `471d32d`,
`98c5be2`, `5fb5136`, `87b4ace`, `d4d1050`). Re-running the official suite
sweeps surfaced four NEW cross-version defects in the same families those items
were about — `utf8.char` ceiling, `string.format` flag-overflow wording,
`string.pack` `c<n>` size width, and `math.random` 5.3 interval guards — all of
which are now fixed and CI-guarded. The architectural remainder (G, H) is
documented with precise re-entry notes.

---

## Items A–F: status

| Item | State | Notes |
|---|---|---|
| A — `_ENV[<relational>]` index codegen | **LANDED (pre-pass)** `1c0ce30` | `_ENV[1<2]` now matches the reference on all three versions (5.3/5.5 → `nil`; 5.4's reference genuinely raises "attempt to index a number value", an upstream 5.4 bug — rs reproduces it faithfully). |
| B — `luaL_argerror` funcname/value/where | **LANDED (pre-pass)** `0668e59`,`87b4ace` | `collectgarbage`, `utf8.offset`, `string.format`, `string.rep`, `string.pack`/`unpack` argerror wording all MATCH cross-version. |
| C — GC default mode | **DEFERRED (documented)** | See "Item C" below — confirmed real collector-behavior, not faked. |
| D — `\u{}` lexer upper bound | **LANDED (pre-pass)** `471d32d` | Lexer caps at `0x7FFFFFFF` ("UTF-8 value too large" above) on all versions; matches. |
| E — `print` → global `tostring` | **LANDED (pre-pass)** `98c5be2` | 5.1/5.2/5.3 `print` calls the global `tostring` (errors if nil); 5.4/5.5 ignore it. Matches. |
| F — `string.unpack` `"c0"` bounds | **LANDED (pre-pass)** `5fb5136` | `string.unpack("c0", x, 0)` raises "initial position out of string"; matches. |

## NEW cross-version fixes landed this pass

| Fix | Versions | File | Summary |
|---|---|---|---|
| `utf8.char` codepoint ceiling | 5.3 vs 5.4/5.5 | `crates/lua-stdlib/src/utf8_lib.rs` | 5.3 rejects codepoints > `0x10FFFF` ("value out of range"); 5.4/5.5 accept up to `0x7FFFFFFF`. rs accepted everything. **Distinct from item D** (that is the *lexer* `\u{}` ceiling; this is the `utf8.char` *function*). Blocked `utf8.lua:151` on 5.3. |
| `string.format` flag-overflow wording | 5.3 vs 5.4/5.5 | `crates/lua-stdlib/src/string_lib.rs` | 5.3 `scanformat` raises "invalid format (repeated flags)" when the flag run reaches `sizeof(FLAGS) == 6` chars; 5.4/5.5 fold this into "invalid format (too long)". rs always emitted "(too long)". Blocked `strings.lua:303` on 5.3. |
| `string.pack`/`packsize` `c<n>` size width | 5.3/5.4 vs 5.5 | `crates/lua-stdlib/src/string_lib.rs` | 5.3/5.4 read the `c` size into a C `int` (a huge numeral overflows → trailing digit mis-read as "invalid format option '<d>'"); 5.5 widened `getnum` to `size_t`. Added the 5.5-only "result too long" (`pack`) and widened "format result too large" (`packsize`) running-total checks. Blocked `tpack.lua` on 5.5 (and `tpack` was also failing on 5.3 before — both pass now). |
| `math.random` 5.3 interval guards | 5.3 vs 5.4/5.5 | `crates/lua-stdlib/src/math_lib.rs` | 5.3 treats `random(N)` as `[1,N]` (so `random(0)` is empty `[1,0]`) and rejects width-overflowing intervals (`low >= 0 || up <= LUA_MAXINTEGER + low` else "interval too large"). 5.4/5.5 rewrote the generator around `project` bit-masks: `random(0)` returns a full-range integer, any interval is accepted. rs used the 5.4/5.5 algorithm for all versions. Blocked `math.lua` on 5.3. |

All four are version-gated; 5.4/5.5 (or 5.3, as appropriate) are confirmed
unaffected via `diff_one.sh` and CI guards.

### CI guards added (`crates/lua-rs-runtime/tests/multiversion_oracle.rs`)

`v53_utf8_char_caps_at_10ffff`, `v54_v55_utf8_char_caps_at_7fffffff`,
`v53_format_repeated_flags`, `v54_v55_format_too_long`,
`v53_v54_pack_csize_overflows_int`, `v55_pack_csize_wide`,
`v53_random_interval_guards`, `v54_v55_random_zero_and_full_range`. The
multiversion oracle went **47 → 67** passing tests.

---

## Architectural / deferred items (G, H, C) — re-entry notes

### Item C — GC default mode (DEFERRED, documented, do not fake)
Confirmed real: 5.4 and 5.5 default to the **generational** collector;
`collectgarbage("incremental")`/`("generational")` and the `("isrunning")`/mode
queries reflect this. lua-rs runs incremental on all versions. This is genuine
collector behavior, not a wording swap — faking the mode query while running the
wrong collector would be a lie. Re-entry: implement a generational mode in
`crates/lua-gc` and wire the default per version; only then make the mode query
report it. Risky; out of scope for a fidelity-wording pass.

### Item G — `__le`-from-`__lt` across a yield
The actual derivation-across-yield case (`coroutine.lua` mt with
`__lt`/yield, 5.3/5.4) now **MATCHes** — #78's `__le`-from-`__lt` derivation
survives the yield boundary. 5.5 removed `__le`-from-`__lt` derivation entirely,
so that path errors on both rs and ref. The one residual 5.5 DIFF is a **doubled
location prefix** the reference emits (`(command line):1: (command line):1:
attempt to compare two table values`) on the specific `x<=y` path where the
derivation is gone and the error propagates back through the coroutine boundary;
rs emits a single prefix. It reproduces only in that narrow construction
(isolated `x<x`-with-erroring-`__lt`, and plain `x<x`, both MATCH). Low value,
fragile reference quirk — DEFERRED.

### Item H — architectural candidates (all DEFERRED, documented)

- **goto label scoping in disjoint/nested blocks** (`goto.lua`): rs's label
  table is too global. Minimal repro: `::l3:: do goto l3; ::l3:: end` — rs
  errors "label 'l3' already defined" on **both** versions; the reference 5.3
  accepts it (inner `l3` is a distinct block scope) while 5.5 also errors. The
  scope rules themselves differ 5.3 vs 5.5. Re-entry: per-block label scope
  tracking in `crates/lua-parse` goto/label resolution, version-gated; interacts
  with the `<close>`/goto-over-local rules. Genuinely a parser-scope change, not
  a localized fix.
- **loop-built-closure equality caching** (`closure.lua:48`): **5.3-only**. With
  closures sharing identical upvalues (`function(x) return x + a + _ENV end` in a
  loop), 5.3 caches and returns the same `LClosure` (`a[3]==a[4]==a[5]` → true);
  5.4+ removed the cache (rs already matches 5.4/5.5 → false). Re-entry: a
  per-proto LClosure cache keyed on the upvalue set, in the closure-creation path
  (`crates/lua-vm` `OP_CLOSURE`), 5.3-gated. VM/GC-level.
- **`__gc` finalizer error propagation** (`gc.lua:360`): an erroring `__gc`
  finalizer's error is not surfaced as the reference does. GC-subsystem
  (`crates/lua-gc`), interacts with finalizer scheduling. Deferred.
- **debug line-hook fidelity** (`db.lua:28`, both versions): `sethook(f,"l")`
  line-trace events do not match the reference trace. Debug-subsystem; needs the
  per-instruction line-change hook dispatch. Deferred.
- **named-vararg `...t` / `...` aliasing** (`vararg.lua:111`, `locals.lua:314`,
  5.5): the always-materialize lowering makes `t` and `...` independent;
  upstream shares one storage object. Re-entry: a proto field for the
  vararg-table register, redirect `OP_VARARG`, drop the snapshot copy. (Carried
  from `5.5-lang.md` §2a.)

### NEW bug found (not on the item list): table-resize panic
`crates/lua-types/src/table.rs:594` **panics** (`index out of bounds: len 8,
index 8`) during a downward array resize on a specific `nextvar.lua` case (5.5):
the migration loop iterates `new_asize..old_asize` and indexes `self.array[i]`,
but `old_asize` (from `set_limit_to_size`) can exceed `self.array.len()`. A panic
is worse than a parity mismatch. Pre-existing (table.rs untouched this pass).
Re-entry: re-establish the `set_limit_to_size` ⇒ `self.array.len()` invariant or
clamp the migration loop to the physical array length, mirroring upstream
`luaH_resize` which migrates over the old *physical* array.

---

## Gate results (all green)

| Battery | Result |
|---|---|
| `cargo build --workspace` | green |
| `cargo test --workspace --features lua-rs-runtime/derive` | 43 test binaries, 0 failures |
| `multiversion_oracle.rs` | **67 passed, 0 failed** (was 47) |
| `traceback_oracle.rs` | 11 passed, 0 failed |
| `check.sh 5.3` | 23 passed, 0 failed |
| `check.sh 5.4` | 7 passed, 0 failed (no regression) |
| `check.sh 5.5` | 10 passed, 0 failed (no regression) |

## Official-suite parity: before vs after (this pass)

Sweep = each parity-meaningful test file run through `lua-rs(<ver>)` and the
matching reference with the standard preamble (`_soft=true; _port=true;
_nomsg=true; _U=false`), normalized (heap addresses, benchmark msec). Excludes
harness/heavy/fs files (`all`, `main`, `heavy`, `cstack`, `memerr`, `verybig`,
`big`, `files`, `api`, `gengc`, `tracegc`).

| Version | Before (pass start) | After (strict) | After (effective) | Newly passing |
|---|---|---|---|---|
| 5.3 | 10 byte-identical | **13** byte-identical | **15** (incl. `sort` RNG-count + `literals` locale noise) | `strings`, `utf8`, `tpack`, `math` |
| 5.5 | 10 byte-identical | **12** byte-identical | **14** (incl. `math` + `constructs` RNG-seed noise) | `tpack`, `constructs` (to noise-only) |

Remaining 5.3 real DIFFs: `calls` (string.dump bytecode header), `closure`
(closure caching, H), `coroutine` (5.5-derivation note), `db` (line hook, H),
`errors` (line attribution), `gc` (`__gc` error, H), `goto` (label scope, H).
Remaining 5.5 real DIFFs: `calls`, `coroutine` (`coroutine.close(main)`), `db`,
`errors` (checkmessage), `goto` (label scope / const), `locals` + `vararg`
(named-vararg aliasing), `nextvar` (table-resize panic), `sort` (table.create
GC accounting).
