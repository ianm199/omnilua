# Graduated: lua-parse

Status: graduated 2026-06-14 (Idiomatization Sprint 1, Phase P1b — and P1c,
which is **subsumed** here). Branch of record: `idiom/parser`. Plan:
`docs/IDIOMATIZATION_ROADMAP.md`, `docs/IDIOMATIZATION_SPRINT_1_SPEC.md`.

## What "graduated" means here

`crates/lua-parse/src/lib.rs` was originally a port of PUC-Rio `lparser.c` +
`lparser.h` **with `lcode.c` codegen folded in** (the recursive-descent parser
and the single-pass bytecode generator live in one crate; the standalone
`lua-code` crate is only the opcode tables / `Instruction` encoding). As of
Sprint 1 the C correspondence is **intentionally gone**: the `# C source` /
Phase-A design-note blocks, the `lparser.c:NNN` / `lcode.c:NNN` line references,
the `macros.tsv` / `types.tsv` annotations, the `lparser.h` enum-correspondence
notes, the dead "resolve in Phase B" import stubs, and the C-correspondence
`PORT NOTE`s have all been removed or rewritten as behavioral docs. Do **not**
open `lparser.c` or `lcode.c` to reason about this file — the structural mapping
no longer holds and chasing it will mislead you.

This crate was **already mostly idiomatic** before Sprint 1 (Result/`?`,
`Option<Box>` chains, real enums, ZERO unsafe). The graduation work was narrow
by design: idiomatize the genuine C-residue, leave the already-idiomatic
majority and the hot codegen core alone, and record honest-negatives where
idiomatization wasn't worth a bytecode-parity risk.

## The oracle that now guards it

Behaviour is held by three nets, strongest first. A change to this crate is
verified only when all are green (the Sprint-1 gate):

1. **Bytecode parity (structural, strongest).** This crate's whole job is to
   emit bytecode; the output must equal `luac -l -l` byte-for-byte. Crucially
   this oracle **survives idiomatizing the producer**: rewrite the parser/codegen
   internals however you like; if the emitted bytecode does not move, you are
   provably behavior-preserving.
   - `python3 harness/bench/bytecode-parity.py` (bench corpus — all-OK).
   - `python3 harness/bench/bytecode-parity.py reference/lua-5.4.7-tests/*.lua`
     (broad grammar coverage). The broad corpus has **pre-existing codegen-level
     divergences** (constant folding, `LOADNIL` coalescing) that are NOT
     parser-introduced; the invariant is that the per-file divergent-op **counts
     do not change** — a count that moves means real codegen output moved.
   - The allowlist `harness/bench/bytecode-parity-allow.txt` is **empty** and
     must stay empty; do NOT add an entry to dodge a regression.
2. **Behavioral suite (output parity).** `cargo test -p omnilua --test
   multiversion_oracle` (165), the full official suite
   (`harness/run_official_all.sh`, 33/33), and the version-gated batteries
   (`specs/oracle/check.sh 5.1`..`5.5` = 57/54/23/7/10, 0 fail).
3. **Grammar/codegen-edge behavioral tests.** `literals.lua`, `errors.lua`, and
   the parser-heavy official files (`constructs.lua`, `goto.lua`, `locals.lua`,
   `attrib.lua`) — the files that exercise exactly the delicate surface
   idiomatization could break (error wording, line attribution, scoping).

Plus the crate's own fast net, new in Sprint 1: **`cargo test -p lua-parse`** (4
unit tests) — the tier-2 inner loop. It drives the codegen primitives directly:
the `JumpList` cursor yields the same pcs as a manual chain walk (+ empty/single
+ `cg_concat` tail-link), and the operator-to-`BinOpr` mapping indexes the
`PRIORITY` precedence table consistently.

## What a future debugger should trust instead of lparser.c / lcode.c

- **The version gates are the behavioral invariant.** One core parses 5.1–5.5;
  the differences are gated on `state.global().lua_version` (live) and the
  version snapshot the lexer carries. The load-bearing gates (all KEPT, with
  their explanatory comments):
  - The 5.4-only VJMP `exp2val` upstream-bug gate (5.4 keeps the bug; 5.3/5.5
    fixed).
  - Float-only 5.1/5.2 (`parser_is_float_only`): no bitwise operators.
  - Goto/label scoping: block-scoped pre-5.4 vs function-wide 5.4+.
  - `<const>`/`<close>` attributes (5.4+); the 5.5 `global` keyword, strict
    scoping, for-var const-ness, and `if`-branch condition-line attribution.
  Do not "simplify" a version gate away — it is the whole point of the multi-
  version core.
- **The boundaries are stable contracts, not enums to "finish."** Token kinds
  cross from `lua-lex` as plain `i32` (`TK_*`); the error formatters and this
  parser read those codes directly, so they stay `i32`. `LexState` *embeds*
  `lua_lex::LexState` (field `lex`) and drives it via `lex_next`/`lex_lookahead`.
  `parse()` returns `Box<LuaProto>`; the caller wraps it into a closure for the
  GC. These are the public surface — byte-stable on purpose.
- **The emit/register/jump/line-info/constant-fold CORE is deliberately
  structurally faithful** (the hot-loop exception). The `cg_*` functions'
  instruction-emission order, register-allocation LIFO discipline,
  constant-table insertion order (with the pre-add dedup find), jump-offset
  computation (`offset = dest - (pc+1)`), constant-folding rules, the
  `OP_CONCAT` prev-instruction merge, and the relative-vs-absolute line-info
  encoding are exactly what bytecode parity pins. Idiomatize *around* this core
  (e.g. the jump-list *walks* became the `JumpList` cursor), never *through* it
  (the jump *offset math* is untouched). If you must change this core, the
  bytecode-parity counts are your tripwire.
- **`ExprPayload` and `VarDesc` are flat structs, not enums — on purpose.** Only
  the field(s) named for the active `ExprKind` / `VarKind` are meaningful. The
  "convert to a tagged enum" recipe is a **recorded honest-negative**: the kind
  and the payload are set in separate statements throughout codegen and several
  helpers take them as separate arguments, so a faithful enum conversion is an
  un-gate-able big-bang. The invariant "which field each kind uses" is held by
  the bytecode-parity oracle, not the type system. See the Sprint-1 verdict
  ledger for the full reasoning.
- **8 `TODO(port)` markers are GENUINE, not crutch.** They flag real
  deferred-codegen behavior (integer stack-check, debug-var startpc, local
  const-fold, explicit `fix_line`, GC proto allocation, single-vs-multret arg,
  an unnamed-var defensive branch). The bytecode-parity oracle proves they do
  not move output on the corpus, so they document a real, invisible divergence
  from full `lcode.c` — do not delete them thinking they are stale.

## Recipes harvested

See the "Recipe ledger" → "### P1b — lua-parse" in
`docs/IDIOMATIZATION_SPRINT_1_SPEC.md` for the reusable before→after patterns:
manual reverse index walk → `.rev()` range iterator; sentinel-terminated chain
walk → named lending cursor; and the parser-specific crutch-removal judgment
calls (strip the `file.c:NNN` coordinate but keep the behavioral prose;
distinguish stale from genuine `TODO(port)`). The verdict ledger records the two
honest-negatives (enter/leave → RAII, and the `ExprPayload` enum).
