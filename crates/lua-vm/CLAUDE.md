# lua-vm — the register VM and core runtime

The largest crate and the hot path. The bytecode interpreter, value operations,
arithmetic/comparison/metamethod dispatch, call/return, and the line/count hooks.
Read the root `../../CLAUDE.md` first.

## The hot-path rule (non-negotiable)

The bytecode dispatch loop (`src/vm.rs` `execute`, the big `'startfunc`/`loop`)
**must carry zero per-version cost**. Compute-bound code runs identically across
5.1–5.5 because the loop never branches on `lua_version`. When a version needs
different opcode semantics, **resolve the version once before the loop** into a
local `bool`/flag and branch on that, not on `state.global().lua_version` inside
the handler.

Canonical example: `legacy_for` (issue #92). Numeric `for` is compare-based on
≤5.3 and count-based on 5.4+. We resolve `let legacy_for = matches!(version,
V51|V52|V53)` once at the top of `execute`, then the `ForPrep`/`ForLoop` arms
pick `forprep_legacy`/`forloop_legacy` vs `forprep`/the count-based body. The
emitted bytecode is identical across versions — only the VM *interpretation* of
the same `Bx` differs.

## Where things live

- `execute` — the dispatch loop; `OpCode` arms. `trap` = per-frame hook flag;
  `savedpc` is set before any op that can re-enter (calls, hooks, FORPREP).
- `forprep` / `forprep_legacy` / `forloop_legacy` / `forlimit*` — numeric `for`.
- `raw_arith` / `number_to_str_buf` — arithmetic and float formatting; the
  **float-only arm** (5.1/5.2) lives here, gated by `number_model()`, not a
  `LuaValue` fork (the dual enum stays; behavior is gated at production sites).
- `src/debug.rs` — `trace_exec` (line/count hooks; the `npci <= oldpc ||
  changed_line` rule is a faithful 5.4 `luaG_traceexec`), `get_func_line`,
  the line-info decoder.
- Metamethod dispatch — `__len`/`__pairs`/`__gc`-on-tables are **inert in 5.1**;
  `__le`-from-`__lt` derivation is kept 5.1–5.4, dropped 5.5.

## GC interaction

Heap stores from the VM go through write barriers (`lua-gc`). When you add a
store that can place a young object into an old/black container, it must be
barriered — see `crates/lua-gc/CLAUDE.md`. A missing barrier is invisible until
the generational collector frees a still-reachable object.

## Gotchas

- `savedpc` must be current before a hook or a re-entrant call, or tracebacks and
  `currentline` point at the wrong instruction.
- Stack reallocation invalidates raw indices; re-fetch `base` after anything that
  can grow the stack.
- 5.4 is the canary: a shared-core change must leave 5.4 byte-identical to
  baseline (modulo RNG/PID/path noise). Run `specs/oracle/check.sh 5.4`.

## Test
`cargo test -p lua-vm`; behavior lives in
`crates/lua-rs-runtime/tests/multiversion_oracle.rs` (tier 2). For one divergence:
`specs/oracle/diff_one.sh <ver> "<snippet>"`.
