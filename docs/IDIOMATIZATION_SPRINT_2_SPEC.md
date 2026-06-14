# Idiomatization Sprint 2 — stdlib (Phase 2): live spec + recipe ledger

Owner: Fable (supervisor: design sign-off + verification). Execution: Opus
subagents per module. Plan of record: `IDIOMATIZATION_ROADMAP.md` Phase 2 + the
**Phase-2 GO** in `IDIOMATIZATION_REFLECTION_1.md §7`. Reuses Sprint 1's
recipe-catalogue format + graduation-declaration template
(`IDIOMATIZATION_SPRINT_1_SPEC.md` → "Phase-0 scaffolding").

## The shift from Sprint 1: the structural oracle is GONE

Sprint 1 (lexer/parser) had **bytecode parity** — a near-total, bisectable net
that survived idiomatizing the producer. stdlib emits *behavior*, not bytecode.
**The only net is the behavioral suite, and it is coarser.** This sprint's whole
discipline is managing that loss. The governing rule (from the reflection):

> Before idiomatizing any stdlib module, verify its behavioral coverage is
> strong enough to stand alone. A module with thin coverage has a weak net;
> **strengthening the net (adding oracle tests) is the FIRST transformation, not
> an afterthought.** Code the net does NOT cover (algorithm-exact PRNG, subnormal
> bit-math) is treated as load-bearing: EXTRACT/rename, never refactor.

## The Phase-2 gate (behavioral-only)

A module is idiomatized only when ALL are green:

1. **Coverage precondition (do FIRST, before any idiomatization).** Produce a
   per-public-function map: WELL-COVERED / WEAKLY-COVERED / UNCOVERED by the
   behavioral net (`run_official_all` module file + `multiversion_oracle` +
   `check.sh` ×5). For every WEAK/UNCOVERED function that idiomatization will
   touch, **add the missing test to the standard gate first** (a sequence test, a
   type-invariant test, an edge-input test). Land the net-strengthening tests as
   their own commit(s) so they are seen to FAIL-then-PASS only against the
   reference behavior — never tautological.
2. **Behavioral suite green:** the module's official test file PASS (via
   `run_official_all`), `multiversion_oracle` (current 165, plus the new
   net-strengthening assertions), `check.sh 5.1`..`5.5` at baseline.
3. **Crate gates:** `cargo test -p lua-stdlib`, `cargo test --workspace`,
   `cargo check --target wasm32-unknown-unknown`.
4. **Perf arbiter — hot modules ONLY.** Cold modules (math, table, os date/time)
   need no arbiter. The string-pattern matcher IS hot → it carries the Ir +
   cold-machine-wall arbiter (`docs/MEASUREMENT_PROTOCOL.md`, the T5a lesson);
   an idiomatization that regresses CPI is a no-go even if behavior is identical.

Load-bearing invariants preserved byte-identical, same as Sprint 1: all version
gates, exact error wording, no public-API change, `unsafe` reduced-never-added.

## Recipe / graduation format

Unchanged from Sprint 1 (`IDIOMATIZATION_SPRINT_1_SPEC.md`). Each module appends
recipes to this doc's "Recipe ledger", a verdict to the "Verdict ledger", and
gets a `crates/lua-stdlib/<module>.GRADUATED.md` (or a shared
`lua-stdlib/GRADUATED.md` with a per-module section) stating which behavioral
net now guards it and which algorithm code was left load-bearing.

## Checklist (tick only with evidence)

- [x] P2.0: scaffolding — this spec (Phase-2 gate + coverage-precondition rule);
      math coverage-precondition verdict recorded below from the 2026-06-14 recon
- [ ] P2a: MATH (`math_lib`) — net-strengthened FIRST, then the well-covered pure
      functions idiomatized; PRNG/ldexp/frexp/version-gates left load-bearing;
      merged with behavioral suite green + recipes + graduation
- [ ] (then, as gates + budget allow: `table` (sort/nextvar nets), `os` date/time
      arithmetic — both cold/pure)
- [ ] (LAST, separate, with the perf arbiter: the `string` pattern matcher)
- [ ] CLOSE: PRs merged CI-green; board row updated

## P2a — math: coverage-precondition verdict (recon 2026-06-14)

The behavioral net is **strong for pure algebra, gappy for PRNG / subnormals /
version-gate invariants.** The packet must STRENGTHEN before it idiomatizes.

**WELL-COVERED → SAFE to idiomatize** (arg-type dispatch helpers, naming,
const-naming the bit-pattern magic numbers, crutch removal): `abs`, `sin`/`cos`/
`tan`/`asin`/`acos`/`atan`, `sqrt`, `exp`, `log`, `deg`/`rad`, `floor`/`ceil`,
`modf`, `min`/`max`, `ult`, `type`, `tointeger`, `huge`/`pi`/`maxinteger`/
`mininteger`.

**WEAK / UNCOVERED → net-strengthen FIRST, then leave the algorithm
LOAD-BEARING (extract/rename only, do NOT refactor the math):**
- `math.random`/`randomseed`: the oracle pins ONE seed point — it would not catch
  a 2nd-call sequence divergence. **Add a multi-call no-reseed sequence assertion
  (per version where the PRNG differs — 5.4/5.5 xoshiro256\*\*) + a FloatOnly
  invariant test** (under 5.1/5.2 every `math.random` result must be `Float`,
  never `Int` — invisible to `type()` but a real invariant). Then do not touch
  `next_rand`/`project`.
- `ldexp`/`frexp`: subnormal edges (`ldexp(1.0,-1074)==5e-324`) are only in
  `multiversion_oracle`, not `math.lua`. **Promote the subnormal assertions into
  the standard gate.** Then leave the bit-scaling untouched.
- Version gates (`is_v53`, `float_only`, `empty_arg`, the compat-math roster,
  `math.log` base arg, `math.mod` 5.1 alias, the 5.1/5.2 nil-registration of
  5.3+ helpers): each is checked per-version in isolation, so **do NOT
  consolidate** — collapsing them can silently break a never-construct-Int or
  wrong-arg-index invariant. Keep them explicit `if` branches.

**Known adjacent bug (NOT in scope, note it):** `math.fmod(x,0)` error omits the
function name (`bad argument #2 (zero)` vs `... to 'fmod' (zero)`) — a shared-core
arg-naming gap, tracked separately; do not "fix" it inside this packet.

## Recipe ledger
(append transformation recipes here as modules graduate)

## Verdict ledger
(append per-module outcomes — graduated OR honest-negative-with-reason)
