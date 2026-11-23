# Phase C Report — finishing Lua 5.5 (issue #20)

Branch: `finish-5.5` (off `main`, v0.0.21). Oracle: the unmodified `make
macosx` reference binaries in `/tmp/lua-refs/bin` — `lua5.5.0` as the truth
teller, `lua5.4.7` / `lua5.3.6` as cross-version non-regression refs. Method and
preamble per the engine contract in `specs/oracle/CONTRACT.md`. Every expected
value below was captured from a reference binary via `specs/oracle/diff_one.sh`.

This phase closed the headline 5.5 long-tail: the `global`-declaration family
(prefixed `<const>`, `global function`, the `*` wildcard coexisting with named
declarations, block-scoped strict mode), the 5.5 stdlib/error deltas
(`utf8.offset` 2nd return, `collectgarbage` option set + `"param"` API,
`<no error object>`, `[C]: in global 'fn'` namewhat), and the attribute
message-text bugs (Div.3/Div.4). It also found and fixed two divergences NOT in
the original plan: the `global<const> namelist` parser ambiguity and the
prefixed-attribute local-variable form (`local <const> a, b`), both new 5.5
grammar that the original parser rejected.

---

## Official 5.5 suite parity: before vs after

28 files swept (excluding harness/heavy/fs files `all`, `heavy`, `verybig`,
`cstack`, `memerr`, `files`, `main`). "Byte-identical" = normalized stdout +
stderr + exit-code match (heap addresses, benchmark msec, and unseeded-RNG seed
lines normalized). `constructs` and `math` are RNG-noise-only divergences
(`math.random`-seeded lines), counted as effective matches.

| Metric | Before (phase start) | After |
|---|---|---|
| Byte-identical (strict) | 9 | 13 |
| Effective (incl. RNG-only `math`/`constructs`) | ~11 | **15** |
| Real divergences | ~16 | **12** |

### Divergences by category: before -> after

| Category | Before | After | Status |
|---|---|---|---|
| global-decl parser (`global<const>` namelist, `global function`, prefixed local attr) | ~7 files cascading | **0** | RESOLVED — math, goto, db, coroutine, calls(early), closure(early), locals(early) all parse cleanly now |
| global-decl semantics (`*` wildcard voided by named decl) | calls, closure, locals | **0** | RESOLVED — wildcard now coexists with named decls, block-scoped |
| attribute message text (Div.3 `<close>`, Div.4 `unknown attribute`) | 3 forms (incl. 5.4) | **0** | RESOLVED via `lua_lex::sem_error` |
| stdlib-5.5 (utf8.offset 2nd ret, collectgarbage opts/param) | 2 | **0** | RESOLVED (landed earlier this phase) |
| error/namewhat (`<no error object>`, `[C]: in global`) | 2 | **0** | RESOLVED (landed earlier this phase) |
| named-vararg `...t`/`...` ALIASING | 1 (vararg.lua:111) | 1 | DEFERRED (architectural, documented) |
| table.create GC pre-reservation | 1 (sort.lua:22) | 1 | DEFERRED (GC byte-accounting stub) |
| relational-key index codegen (`_ENV[1<2]`) | 1 (closure.lua:10) | 1 | NEW root cause found; multi-version codegen bug |
| coroutine.close(main) | 1 (coroutine.lua:163) | 1 | stdlib gap |
| debug line-hook fidelity | 1 (db.lua:28) | 1 | debug gap (shared with 5.3) |
| other (errors checkmessage, goto:427 const, literals locale, nextvar __pairs tbc) | several | several | mixed, see below |

### After-fix remaining real DIFFs (12)

| File | First divergence | Category | Scope |
|---|---|---|---|
| closure | `:10` `_ENV[1<2]` "attempt to index a number value" | codegen | **multi-version** (also 5.3); upvalue indexed by a relational expr; sole blocker on closure.lua |
| sort | `:22` `table.create(N)` memdiff < 4N | GC-accounting | `create_table` does not reserve real array bytes AND `gc_count` is a Phase-A stub (136 B vs ref 900 KB) |
| vararg | `:111` `...t` mutation observable through `...` | named-vararg aliasing | DEFERRED (per 5.5-lang.md): needs the virtual vararg-table proto field |
| locals | `:314` `function (...t)` in a `<close>` mm | named-vararg aliasing | same root as vararg:111 |
| coroutine | `:163` `coroutine.close(main)` must error w/ "main" | stdlib gap | close on the main coroutine |
| db | `:28` `sethook(f,"l")` line trace | debug gap | line-hook fidelity (also 5.3) |
| errors | `:30` `checkmessage` assertion | error-wording | a `string.find(m, msg)` message-substring mismatch downstream |
| goto | `:427` `attempt to assign to const variable 'b'` | const/scope | a const-var assignment that the ref allows (closure/scope interaction) |
| literals | locale `pt_BR` line + `\u{}` upper bound | locale/lexer | env-dependent locale line; `\u{110000}` bound (shared w/ 5.3) |
| nextvar | `:919` `__pairs` 4th to-be-closed value not closed | tbc cascade | `__pairs` returning a 4th `<close>` value |
| tpack | a `string.unpack`/`packsize` bounds check deep in the file | stdlib-gap | (isolated `i6`/`packsize` MATCH; failure is deeper) |
| calls | `:212` assertion | other | deep; parse-clean now (was `:35`) |

---

## What landed (with gate results)

Two commits on `finish-5.5` this synthesis pass, on top of the four already
present (`global-guard`, `named-varargs`, `stdlib-5.5`, `error-namewhat`):

| Commit | Category | Summary |
|---|---|---|
| (this pass) | global-decl long tail | prefixed-`<const>` namelist, `global function`, local prefixed attribute, `*`-wildcard coexistence (block-scoped), Div.3/Div.4 attribute message text |

Specifically:

1. **`globalstat` restructure** (`crates/lua-parse/src/lib.rs`): parse the
   optional leading attribute FIRST as the declaration default (`get_global_attribute`),
   then branch on `*` vs name list. `global <const> a, b` is now a const name
   list (was wrongly forced to `*`). Mirrors upstream `globalstat`/`globalnames`.
2. **`global function NAME body`** form (upstream `globalfunc`): declares the
   global, compiles the body, runs the already-defined guard, stores the closure.
   Statement-dispatcher lookahead now accepts `TK_FUNCTION` after `global`.
3. **`localstat` prefixed attribute** (5.5 only): `local <const> a, b` applies
   the leading attribute as each variable's default. 5.4/5.3 keep rejecting the
   prefix form (`<name> expected near '<'`) exactly as the reference does.
4. **`global *` wildcard as a block-scoped flag** (`global_wildcard`): the
   wildcard now COEXISTS with named `global` declarations — a later `global name`
   no longer voids global-by-default (the `calls.lua`/`closure.lua`/`locals.lua`
   strict-scope cascade). Saved/restored on block entry/exit alongside
   `declared_globals`, so `do global * end` does not leak.
5. **Attribute message text** (Div.3/Div.4) via a new `lua_lex::sem_error`
   (location prefix, NO `near` suffix; mirrors upstream `luaK_semerror`):
   `unknown attribute 'foo'` now gets the `(command line):N:` prefix on locals
   and loses the spurious `near '='` on globals; `<close>` on a global rewords to
   `global variables cannot be to-be-closed`.

**Gate — all green after the fixes:**

| Battery | Result |
|---|---|
| `cargo build --workspace` | green |
| `cargo test --workspace --features lua-rs-runtime/derive` | 0 failures |
| `multiversion_oracle.rs` | **47 passed, 0 failed** (was 42; +5 v55_* tests) |
| `check.sh 5.5` | 10 passed, 0 failed |
| `check.sh 5.4` | 7 passed, 0 failed (no regression) |
| `check.sh 5.3` | 23 passed, 0 failed (no regression) |

**5.4/5.3 confirmed unaffected** — all 5.5 grammar is gated to `LuaVersion::V55`:
`global x = 1`, `global function f()`, `local <const> x` (prefix) all stay the
exact reference parse error on 5.4/5.3 (`diff_one.sh` MATCH); the `sem_error`
message helper is shared but the local-attribute path matches on BOTH 5.4 and 5.5.

### New CI assertions (`multiversion_oracle.rs`)

`v55_global_prefixed_const_namelist`, `v55_global_function_form` (incl.
already-defined guard), `v55_global_wildcard_coexists_with_named_decl`,
`v55_local_prefixed_attribute` (incl. the 5.4 prefix-rejection guard),
`v55_attribute_message_text` (Div.3/Div.4, incl. the 5.4-shared local form). All
values captured from `lua5.5.0` / `lua5.4.7`.

---

## What remains for full 5.5 parity (prioritized)

1. **`_ENV[<relational-expr>]` index codegen bug (multi-version).** `_ENV[1<2]`
   raises "attempt to index a number value" on 5.5 AND 5.3 (but MATCHes on 5.4,
   and a plain `t[1<2]` MATCHes on all). The sole blocker on `closure.lua`, and a
   genuine codegen defect (an upvalue indexed by a register holding a
   relational/boolean result is mis-lowered). Highest-leverage: it is a real
   correctness bug, not a wording nit, and it spans versions.
2. **Named-vararg `...t` / `...` aliasing.** The deferred architectural item
   (`specs/followup/5.5-lang.md` §2a): the pragmatic always-materialize lowering
   makes `t` and `...` independent, but upstream shares one storage object.
   Blocks `vararg.lua:111` and `locals.lua:314`. Re-entry: add a proto field for
   the vararg-table register; redirect `OP_VARARG` to that table; drop the
   snapshot copy.
3. **`table.create` real array reservation + GC byte accounting.** `sort.lua:22`
   asserts `collectgarbage("count")` rises by >=4·N after `table.create(N)`. Needs
   `create_table`/`resize` to reserve real array bytes AND the GC `count` stub
   (`gc_count`, Phase-A) to reflect it. A GC-accounting project, not a wording fix.
4. **`coroutine.close(main)` guard** (`coroutine.lua:163`): closing the main
   coroutine must error with "main" in the message. Contained stdlib fix.
5. **debug line-hook fidelity** (`db.lua:28`, shared with 5.3) and **`\u{}` upper
   bound in the lexer** (`literals.lua`, shared with 5.3): independent contained
   fixes already on the 5.3 backlog.
6. **`__pairs` 4th to-be-closed value** (`nextvar.lua:919`): a `<close>` value
   returned from `__pairs` not being closed in that path.

---

## Updated 5.5 oracle-battery count

- `check.sh 5.5`: **10 passed, 0 failed** (battery green).
- `multiversion_oracle.rs`: **47 tests passing** (was 42 at phase start; +5 this
  pass for the global-decl long tail).
- Official 5.5 suite sweep: **15/27 effective byte-identical** (13 strict +
  `math` + `constructs`, the latter two RNG-noise-only), up from ~11 effective at
  phase start.

## The single most valuable remaining item

**The `_ENV[<relational-expr>]` index codegen bug.** It is a real correctness
defect (not a message nit), it spans 5.5 AND 5.3, it is the lone blocker on
`closure.lua`, and unlike the deferred items (which need new proto fields or GC
accounting) it is a localized register-lowering fix in the indexed-access codegen.
