# Idiomatization Sprint 1 — live spec + recipe ledger

Owner: Fable (supervisor: design sign-off + verification). Execution: Opus
subagents per subsystem. Plan of record: `docs/IDIOMATIZATION_ROADMAP.md`
(Stage-2; read first). This is the live checklist, the Phase-0 scaffolding
(recipe format + graduation template + gate template), and the recipe ledger.

Baseline (2026-06-13): bytecode-parity oracle GREEN across the bench corpus at
main 5abb986; `omnilua` 0.2.0; official suite passing.

## Checklist (tick only with evidence)

- [ ] P0: scaffolding — recipe format, graduation declaration template, gate
      template (this doc)
- [ ] P1a: LEXER (lua-lex) idiomatized + merged — bytecode parity (broad
      corpus) + behavioral suite green; recipe entries + graduation doc
- [ ] P1b: PARSER (lua-parse) idiomatized + merged (if P1a clean)
- [ ] P1c: CODEGEN (lua-code) idiomatized + merged (if P1a/P1b clean)
- [ ] REFLECT: `docs/IDIOMATIZATION_REFLECTION_1.md` written with Phase-2
      go/no-go (REQUIRED before any Phase-2 work)
- [ ] CLOSE: all PRs merged CI-green; board row closed

## Phase-0 scaffolding

### The gate (Phase 1 — bytecode-parity-net subsystems)
A subsystem is idiomatized only when ALL are green:
1. **Bytecode parity**: `python3 harness/bench/bytecode-parity.py <targets>`
   byte-identical to `luac -l -l` (allowlist `bytecode-parity-allow.txt`
   unchanged — do NOT add entries to dodge a regression). Run against BOTH the
   bench corpus AND a broad set of official-test `.lua` files (lexer/parser
   need wide token/grammar coverage; pass the file list as argv).
2. **Behavioral suite**: `harness/run_official_all.sh` (full pass) +
   `cargo test -p omnilua --test multiversion_oracle` (165) + the
   lexical-error/line-number behavioral tests (errors.lua, the syntax-error
   and line-attribution cases, `specs/oracle/check.sh 5.1`..`5.5`).
3. Crate gates: `cargo test -p <crate>`, `cargo test --workspace`,
   `cargo check --target wasm32-unknown-unknown`.
These subsystems are COLD (run at load, not per-op) — no perf arbiter needed;
bytecode parity is the structural oracle and it SURVIVES idiomatizing the
producer (you change the internals; the emitted bytecode must not move).

### Recipe-catalogue format
Each idiomatization records, in this doc's "Recipe ledger" section, entries of:
- **Pattern name** (e.g. `c-charptr-scan -> peekable-iterator`)
- **Before** (the C-port shape, 1-3 lines) → **After** (the idiomatic shape)
- **Behavioral invariant that replaced the structural one**: what you now
  trust instead of "matches llex.c line N" (e.g. "token stream yields identical
  bytecode; lexical errors byte-identical per errors.lua").
- **Caveats / where it doesn't apply.**

### Graduation declaration (per subsystem)
On merge, each idiomatized subsystem gets a short `## Graduated: <crate>` note
(in its crate CLAUDE.md or a `GRADUATED.md`) stating: the C correspondence is
intentionally gone; the oracle that now guards it; what a future debugger should
trust instead of the C source. This is the load-bearing artifact — it tells the
next person the structural crutch is removed and what replaced it.

## Recipe ledger
(append transformation recipes here as subsystems graduate)

## Verdict ledger
(append per-subsystem outcomes — graduated OR honest-negative-with-reason)
