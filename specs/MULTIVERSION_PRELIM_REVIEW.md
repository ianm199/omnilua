# Self-review: preliminary 5.3 + 5.5 (PR #75)

A critical read of my own branch. Written to be useful to whoever picks this up,
not to flatter the work. Severity is about "would this bite someone," not effort.

## Verdict

As a spike it does its job: it proves the version seam end-to-end and the
diff-against-real-reference-binaries oracle, and the 5.5 global-declaration core
is genuinely implemented (compile-time enforcement, reusing existing `_ENV`
codegen, no new opcodes — that part I'm happy with). But the review turned up one
real correctness bug, one actively-misleading behavior, and a verification gap
that together mean this is *not* a "small step from done" — it's an honest spike
with sharp edges. Most of my confidence (the "23/10/7 oracle") was narrower than
it looked.

## What's solid

- **5.5 undeclared-name enforcement is correct for chunk-level decls** and matches
  the reference byte-for-byte, including the `luaK_semerror`-style "no near-token"
  message. The insight that it needs no new opcodes (pure compile-time check +
  existing `_ENV.name` codegen) is right and is why it landed cleanly.
- **OpCode consolidation is clean and discriminant-preserving** — `string.dump`
  round-trips byte-identical to 5.4.7. Picking `lua-vm` as canonical (it's already
  below `lua-code` in the dep graph) was the right direction.
- **5.4 parity was proven, not asserted** — full official-suite diff against a
  pre-change build; all diffs were noise.
- **String-coercion gate is correctly localized** to the string arithmetic
  metamethod, so bitwise ops are untouched and 5.4 is provably unaffected.

## High severity

### H1. 5.5 `global` is block-scoped upstream; ours is chunk-wide and permanent
Real 5.5 scopes a `global` declaration (and the strict mode it triggers) to the
enclosing block, like a local. Ours stores `global_strict` + `declared_globals`
on `LexState` for the whole parse and never unwinds them at block exit.

```
do global x end; y = 5; print(y)
  ref 5.5: 5            -- strict mode ended with the do-block; y is implicit-global
  ours:    error "variable 'y' not declared"
```

This is a genuine fidelity bug, and the more important meta-point: **my 10/10 5.5
oracle missed it because every battery snippet used a top-level `global`.** The
battery had a coverage hole exactly where the model is subtlest. Fix is real work:
track `global_strict`/`declared_globals` per-block and save/restore on block
enter/exit (mirror how `nactvar`/locals unwind), not a flag on `LexState`.

### H2. 5.1 / 5.2 actively masquerade as themselves
`LUA_RS_VERSION=5.1` reports `_VERSION == "Lua 5.1"` but runs the 5.4 core:
`math.type(3)` → `integer` (5.1 has no integer subtype and no `math.type`), `3//2`
and `goto` both work (neither exists in 5.1). So we hand back a binary that *claims*
to be 5.1 and silently isn't. That's worse than not supporting it. Until the
legacy family is real, `LuaVersion::V51`/`V52` should refuse at construction (or
the CLI should reject them) rather than relabel 5.4.

### H3. The real verification isn't in CI
The strong evidence (`specs/oracle/check.sh`, 23/10/7 vs the reference binaries)
is a shell script nothing runs automatically. The CI-gated integration tests
(`multiversion_53.rs`/`_55.rs`) are thinner — version strings, presence/absence,
parse-checks — and notably **don't assert the 5.5 undeclared-name *enforcement*
or the const-global rejection at all** (just that a `global` statement parses).
So the headline feature has no regression guard in `cargo test`. The oracle battery
should become Rust integration tests (or CI should run check.sh against pinned
reference binaries), and they should cover the enforcement + block scoping (H1).

## Medium severity

### M1. The `Engine` enum is vestigial — two half-built seams coexist
`enum Engine` exists with one real arm and `for_version` does `_ => Engine::V54`,
so it never dispatches on version. The actual seam is the `lua_version` flag on
`GlobalState`. A reader can't tell which is "the" mechanism, and the dead enum
implies an architecture that isn't there. Either delete the `Engine` scaffold for
now, or commit to it and route version behavior through it. Right now it's
misleading dead code.

### M2. The seam is a runtime flag, not the design doc's `Engine`/`Semantics`
`WEBLUA_MULTIVERSION_API_SPEC.md` argues for `enum Engine` with `#[cfg]` variants
and/or a monomorphized `Semantics` type so version differences fold away at
compile time. What shipped is a single engine reading a runtime `LuaVersion` flag
at each seam. That's *fine right now* — every current seam is in a cold path
(parser, stdlib registration; the one arith-coercion gate is in the string
metamethod, not the VM loop), so there's no hot-path `if version` cost. But it's a
different architecture than the doc sells, and it won't stay free if a version
difference ever lands in the dispatch loop. Worth an explicit decision: update the
doc to bless the flag, or migrate to the typed seam before logic spreads. (I lean
"flag is correct for now," but it should be a choice, not drift.)

### M3. `global_const_name` taxes every expression for a rare feature
I added `Option<GcRef<LuaString>>` to `ExprPayload`, which is cloned per expression
in the parser, solely to flag declared-const globals so `check_readonly` can reject
assignment. Every expression in every parse now carries (and moves) that field for
a feature that fires on a handful of `<const>` globals. A side-channel (check the
name against `declared_globals` at the assignment site) would avoid taxing the hot
path. Minor, but it's the kind of thing that's annoying to undo later.

### M4. `from_u32` re-duplicates the decode it was meant to de-duplicate
The consolidation removed lua-code's enum, but I added an 83-arm `from_u32` to
lua-vm that mirrors the 83-arm match already in `InstructionExt::opcode`. So lua-vm
now has two hand-maintained 83-arm opcode matches. Should be one source (have
`opcode()` delegate to `from_u32`, or vice versa).

## Low severity

- **L1.** `singlevar` does a linear `declared_globals.iter().any()` per free name in
  strict mode — O(globals × uses). Fine for small chunks; a set keyed by interned
  pointer would be cheap to switch to.
- **L2.** `LUA_RS_VERSION` env var is an unprincipled way to select version (global,
  stringly-typed). It was a stopgap for oracle diffing; a real `--lua-version` CLI
  flag was in the plan and should replace it.
- **L3.** Scope-mixing: the branch dedents stray C-signature lines in doc comments
  of unrelated files (`io_lib`/`os_lib`/`loadlib`/`vm`/`state`/`do_`) to get
  `cargo test --workspace` green. Defensible, but it's incidental cleanup riding in
  a feature PR; arguably its own commit/PR. (It is at least isolated to one commit.)
- **L4.** `bit32` argument handling uses `check_integer` then `as u32`. Worth a test
  for 5.2/5.3 edge semantics (integral floats accepted, mod-2^32 reduction, error on
  non-integral float) against the reference — I verified values but not the arg-type
  error paths.

## If this graduates from a spike

In rough priority: (1) fix block scoping (H1) and add block-scoped tests; (2) make
V51/V52 refuse (H2); (3) move the oracle into CI-gated tests covering enforcement
(H3); (4) resolve the `Engine`-vs-flag incoherence (M1/M2) with an explicit
decision; (5) clean up M3/M4. None of that changes the verdict that the *approach*
(shared modern core, version-gated seams, reference-binary oracle) is sound — it's
the execution edges that need work before it's more than exploratory.
