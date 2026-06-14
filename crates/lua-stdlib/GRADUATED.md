# Graduated: lua-stdlib

Per-module graduation declarations for the standard library
(Idiomatization Sprint 2 — stdlib / Phase 2). Plan of record:
`docs/IDIOMATIZATION_ROADMAP.md`, `docs/IDIOMATIZATION_SPRINT_2_SPEC.md`.

Phase 2 is different from Sprint 1 in one decisive way: **there is no structural
(bytecode-parity) oracle.** stdlib emits behavior, not bytecode. The only net is
the behavioral suite, and it is coarser. The discipline each module follows is:
strengthen the behavioral net FIRST, idiomatize ONLY what the net covers, and
leave net-uncovered algorithm code LOAD-BEARING (extract/rename, never refactor).

---

## math

Status: graduated 2026-06-14 (Sprint 2, Phase P2a — the first Phase-2 module,
the first subsystem with no structural oracle). Branch of record: `idiom/math`.

### What "graduated" means here

`crates/lua-stdlib/src/math_lib.rs` was originally a line-by-line port of
PUC-Rio `lmathlib.c`. As of P2a the C correspondence is intentionally gone for
the idiomatized surface: the 8 `PORT NOTE` blocks, the `lmathlib.c:NNN` /
C-internals references, the `LUAMOD_API → pub` / `I2UInt` macro-correspondence
notes, the stale `to_integer_opt` `TODO`, and the dead Phase-A scaffolding (the
`MATHLIB` static + `LibReg` struct, ~120 lines that the live registration path
never used) have all been removed. The `PORT STATUS` trailer was condensed to
current state and now points at the behavioral net rather than at a C line count.

Do **not** open `lmathlib.c` to reason about the idiomatized functions — but DO
keep reading the inline comments that remain: they describe **behavior**
(per-version gates and the subnormal bit-math), not C structure, and they were
KEPT on purpose (see "Left load-bearing" below).

### The oracle that now guards it (behavioral-only — no bytecode parity)

A change to math is verified only when all are green (the P2a gate):

1. **Behavioral suite.** `cargo test -p omnilua --test multiversion_oracle`
   (169 — the prior 165 plus the four net-strengthening assertions added FIRST in
   P2a), the full official suite (`harness/run_official_all.sh`, math.lua PASS),
   and the version-gated batteries (`specs/oracle/check.sh 5.1`..`5.5` at the
   baseline 57/54/23/7/10, 0 fail).
2. **The crate's own fast net, new in P2a:** `cargo test -p lua-stdlib --test
   math_float_only` (2 white-box tests) — the first test module in this crate.
   It reaches through the `omnilua` `Value` enum (a dev-dependency) at the raw
   `Value::Integer` vs `Value::Number` distinction, which Lua's own `type()`
   **cannot observe** under the 5.1/5.2 float-only model.
3. **Crate gates:** `cargo test --workspace`, `cargo check -p lua-stdlib
   --target wasm32-unknown-unknown`. `unsafe` blocks: 0 (unchanged).

#### The net had to be STRENGTHENED before idiomatizing — the Phase-2 story

This is the key Phase-2 methodology output. At baseline the net was **strong for
pure algebra but gappy for the PRNG / subnormals / float-only invariants**, so
the FIRST three commits added tests (each proven non-tautological by mutation,
each green against the un-idiomatized baseline):

- **PRNG sequence (the #1 gap).** The old oracle pinned ONE draw per seed, so a
  2nd-call or projection-path divergence would pass undetected. Added multi-call
  no-reseed sequence pins (float draws, `random(lo,hi)` positive and signed
  ranges, the two-seed-word `randomseed(n1,n2)` form) on **5.4 and 5.5** — the
  only bit-exact path. (5.1/5.2/5.3 wrap host C `rand()`, a documented allowed
  divergence; see "Honest-negative" below.)
- **FloatOnly type invariant.** Under 5.1/5.2 every `math.random` result must be
  a `Float`, never an `Int` — invisible to `type()`. The white-box test above
  pins it; a mutation that pushed `Int` was caught (`Integer(8)`).
- **Subnormal ldexp/frexp.** Promoted the subnormal edges (`ldexp(1.0,-1074)`,
  underflow-to-+0.0, smallest normal, overflow-to-inf, `frexp` of the smallest
  subnormal/normal incl. sign) into the standard gate, on every version that
  exposes the functions (5.3/5.4/5.5). A naive `x * 2f64.powi(e)` was caught
  (yields `0.0`).

### Left load-bearing — extract/rename only, NEVER refactored (the net does not cover these)

- **The xoshiro256\*\* PRNG core** (`next_rand`, `rand_to_float`, `project`,
  `set_seed_words`) — now grouped, byte-for-byte unchanged, into a private
  `mod xoshiro`. The sequence is pinned bit-exact, so ANY reordering of the
  arithmetic diverges; the module doc says so explicitly. The grouping is the
  ONLY change — no algorithm line moved.
- **`ldexp` bit-scaling and `frexp` mantissa/exponent split.** Subnormal-correct
  by construction (bounded power-of-two chunking; the subnormal-input scale-up
  branch in frexp). A naive simplification loses subnormals. The inline comments
  explaining the bit-math were KEPT.
- **ALL version gates — NOT consolidated.** `is_v53`, `float_only`, the
  `empty_arg` index, the compat-math roster (`atan2`/`cosh`/`sinh`/`tanh`/`pow`/
  `log10`, 5.1–5.4 only), `frexp`/`ldexp` surviving into 5.5, `math.log`'s
  base-arg 5.1-vs-5.2+ split, `math.mod` 5.1 alias, the 5.1/5.2 nil-registration
  of `type`/`tointeger`/`ult`/`maxinteger`/`mininteger`, and the `randomseed`
  return-count / require-seed / auto-seed splits are each checked **per version
  in isolation** by the oracle. Collapsing them can silently break a
  never-construct-`Int` or wrong-arg-index invariant. The inline comments on each
  gate describe behavior (oracle-verified against the reference binaries), not C.

### Honest-negative recorded in this graduation

The PRNG **sequence** is bit-exact only on the xoshiro256\*\* path (5.4/5.5). On
5.1/5.2/5.3 the reference wraps the host C `rand()`/`random()`, whose byte stream
is platform-dependent — a KNOWN, DOCUMENTED allowed divergence
(`specs/followup/5.1-numbers-prng.md`, `specs/research/5.3-upstream-delta.md`).
The spec asked for "at least one of 5.1/5.3" sequence pins; the faithful answer
is that neither is pinnable to the reference (pinning our own output would be
tautological, which the phase forbids), so their **contract** (range/type/shape/
arg-error) is pinned instead and the **sequence** is intentionally not. This is
the right reason to STOP short on that blind spot, not a gap in coverage.

### Recipes harvested

See the "Recipe ledger" → "### P2a — math" in
`docs/IDIOMATIZATION_SPRINT_2_SPEC.md`.

---

## table

Status: graduated 2026-06-14 (Sprint 2, Phase P2b — the second Phase-2 module).
Branch of record: `idiom/table`.

### What "graduated" means here — and the headline finding

`crates/lua-stdlib/src/table_lib.rs` (a port of `ltablib.c`) arrived at P2b
**already mostly idiomatic**: 0 `unsafe`, helpers already extracted, no dead
scaffolding. Its behavioral net, by contrast, was **marginal** — ~75% of
standard paths sampled, but edge cases and version seams thin. So — exactly like
the Phase-1 parser, and reinforcing the P2a lesson from the opposite direction —
**the Phase-2 value here was net-strengthening, not idiomatization.** The
idiomatization on top was deliberately thin (crutch removal + safe
renames/doc-repair); the real deliverable is the strengthened net, and the real
finding is the **second Phase-2 data point**: *idiomatization debt is not
uniform — and neither is net strength*. A module can be clean code with a weak
net; the honest move is to invert the usual rich-rewrite instinct and spend the
budget on the net.

The net-strengthening even **caught a real behavioral bug** the weak net was
hiding (see below) — the clearest possible proof that the net was the thing
worth touching.

### The oracle that now guards it (behavioral-only — no bytecode parity)

A change to table is verified only when all are green (the P2b gate):

1. **Behavioral suite.** `cargo test -p omnilua --test multiversion_oracle`
   (**178** — the prior 169 plus the nine net-strengthening assertions added
   FIRST in P2b), the table-heavy official files run directly
   (`harness/run_official_test.sh reference/lua-5.4.7-tests/sort.lua` and
   `nextvar.lua`, both PASS — they pin sort/insert/remove/move/pack/unpack), and
   the version-gated batteries (`specs/oracle/check.sh 5.1`..`5.5` at the
   baseline 57/54/23/7/10, 0 fail).
2. **Crate gates:** `cargo test -p lua-stdlib`, `cargo test --workspace`,
   `cargo check -p lua-stdlib --target wasm32-unknown-unknown`. `unsafe` blocks:
   **0** (unchanged).

#### The net had to be STRENGTHENED before idiomatizing — the four gaps closed

At baseline the net sampled the standard paths but left four real holes. The
first commits added tests pinning REFERENCE behavior (captured from
`/tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7,5.0}`):

- **`table.remove` out-of-bounds gate (a REAL BUG the net was hiding).** The old
  net only checked the 5.3-vs-5.4 arg index, so two divergences hid behind it:
  our impl errored on **5.1** (where legacy `ltablib.c` has NO bounds check —
  out-of-range silently removes nothing and returns ZERO results) and reported
  arg **#2 on 5.2** (the reference reports **#1** on both 5.2 and 5.3). The new
  cross-version test pins every cell of the matrix and **FAILED at baseline**,
  proving it pins reference behavior; the faithful three-way gate
  (5.1 inert / 5.2+5.3 arg #1 / 5.4+5.5 arg #2) was then landed in the same
  commit.
- **5.1 `__len` bypass.** Under 5.1, `table.insert` (and `#`) use the primitive
  length and IGNORE a table `__len` metamethod; 5.2+ honors it. Pinned both
  directions (was roster-check only).
- **pack/unpack boundaries.** `table.pack`'s `.n` counts all args including
  holes/trailing nils; `table.unpack` raises "too many results to unpack" at the
  INT_MAX span and at the i64-extreme wrap rather than looping. Pinned (was
  untested).
- **`table.move` overlap + metamethod order.** Overlapping in-place moves copy
  in collision-safe order (forward when the destination is clear, backward
  otherwise); reads drive `__index` and writes drive `__newindex` interleaved
  one element at a time. Pinned both the result and the call order (was
  untested).

### Left load-bearing — extract/rename only, NEVER refactored (the net does not cover these)

- **The quicksort core** — `partition` (Sedgewick median-of-three with the
  comparator-callback inner loops), `aux_sort` (recurse-smaller / tail-loop-
  larger with pivot re-randomization), `sort_comp`, `choose_pivot`, `set2`,
  `randomize_pivot`. Left **entirely untouched** — docs, C-evidence blocks, and
  the stack-evolution annotations all kept verbatim. The behavioral net pins the
  OBSERVABLE sort contract (stability via a descending comparator, invalid-order
  detection, mixed-type compare error, array-too-big) but **cannot** observe the
  partition-internal comparator-callback-during-GC safety — that is a
  load-bearing region the behavioral net does not fully guard (an honest
  limitation, the table analogue of math's un-pinnable 5.1/5.2 PRNG).
- **The wrapping-subtract index bounds checks** in insert/remove/move/unpack —
  the unsigned `(pos - 1) < bound` idiom that rejects `pos <= 0` and overflow in
  one comparison. Left as-is (no proven-equivalent helper was extracted, because
  extracting one without a dedicated equivalence test would itself be the
  unverified churn this phase forbids).
- **ALL version gates — NOT consolidated.** The `open_table` per-version roster
  (`move` 5.3+, `pack`/`unpack` 5.2+, `create` 5.5, the 5.1 legacy
  `getn`/`setn`/`maxn`/`foreach`/`foreachi`), the three-way `remove` arg-gate,
  and the `__len` semantics are each checked per-version in isolation by the
  oracle. The per-version roster-delta comments were KEPT.

### Honest-negative recorded in this graduation

The sort partition's internal invariant — that the user comparator callback
cannot corrupt the partition state even if it triggers a GC or mutates the array
mid-sort — is **not behaviorally observable** and so cannot be reference-pinned
(it is the table analogue of math's platform-dependent 5.1/5.2 PRNG, which P2a
also declined to pin tautologically). The net pins the externally-visible sort
contract and STOPS there; the quicksort core is fenced off as load-bearing
rather than papered with a self-referential test. This is a correct STOP on a
blind spot, not a coverage gap.

### Recipes harvested

See the "Recipe ledger" → "### P2b — table" in
`docs/IDIOMATIZATION_SPRINT_2_SPEC.md`.

---

## string

Status: graduated 2026-06-14 (Sprint 2, Phase P2c — the HOT Phase-2 capstone, and
the FIRST module guarded by a PERF arbiter on top of the behavioral oracle).
Branch of record: `idiom/string`.

### What "graduated" means here — and the headline finding

`crates/lua-stdlib/src/string_lib.rs` (a ~3150-line port of `lstrlib.c`) contains
the recursive **pattern matcher**, the hot inner loop of every `string.*` pattern
operation. The honest finding — internalized from recon and confirmed by the
arbiter — is that **the matcher is already idiomatic Rust and CPI-load-bearing**:
the `goto`→`'outer: loop` tail-call translation is correct and perf-critical, and
there is NO perf-neutral idiomatization available inside it. So P2c is NOT a
matcher rewrite. It is the **third "leave load-bearing" data point** (after math's
xoshiro and table's quicksort), and the strongest, because the matcher is the
only load-bearing core *exercised on a perf-measured workload* — so "do not touch
it" is enforced by the Ir arbiter, not merely argued.

The capstone deliverable is the **perf-arbiter veto loop demonstrated end-to-end**
(see "The perf arbiter that now guards it" below): a first cut that regressed the
matcher's instruction count was CAUGHT by the arbiter and driven back to flat.

### The oracle that now guards it (behavioral suite + the NEW perf arbiter)

A change to string is verified only when ALL are green (the P2c gate):

1. **Behavioral suite.** `cargo test -p omnilua --test multiversion_oracle`
   (**181** — the prior 178 plus the three P2c net-strengthening assertions added
   FIRST), the matcher-heavy official files run directly
   (`harness/run_official_test.sh reference/lua-5.4.7-tests/strings.lua` and
   `pm.lua`, both PASS), and the version-gated batteries
   (`specs/oracle/check.sh 5.1`..`5.5` at the baseline 57/54/23/7/10, 0 fail).
2. **The PERF arbiter (new in P2c).** `bash harness/bench/instr-count.sh
   --workloads string_ops,string_ops_long --branch-sim` must come back **Ir
   FLAT** vs the frozen baseline (within the measured baseline-rebuild floor,
   ~0.09%), and Bc flat. An idiomatization that keeps behavior identical but
   regresses the matcher's instruction count or branch behavior is a NO-GO. The
   supervisor re-runs the authoritative Ir + a cold-machine wall A/B before merge.
3. **Crate gates:** `cargo test -p lua-stdlib`, `cargo test --workspace`,
   `cargo check -p lua-stdlib --target wasm32-unknown-unknown`. `unsafe` blocks:
   **0** (unchanged).

#### The net had to be STRENGTHENED before idiomatizing — and it caught two bugs

The behavioral net (pm.lua) exercises the matcher heavily but never HITS its
danger-zone edges. The first three commits added reference-pinned tests for those
edges; **two FAILED at baseline**, catching real 5.1/5.2 pre-5.3.3 divergences:

- **`matchdepth` / "pattern too complex" (a REAL BUG).** The `MAXCCALLS` (200)
  recursion guard was added in **5.2** — 5.1's `lstrlib.c` `match()` has no depth
  counter at all (verified: zero `MAXCCALLS`/`matchdepth` in the 5.1.5 source), so
  a too-deep pattern simply MATCHES on 5.1. Our impl applied the bound to 5.1 too
  and raised "pattern too complex". The test FAILED at baseline.
- **Empty-match advance / the 5.3.3 change (a REAL BUG).** The `e != lastmatch`
  empty-match de-dup is present in 5.3+ and absent in 5.1/5.2, so
  `gsub(" *","-")` must DOUBLE to `-a--b--c-d-` on 5.1/5.2 (vs `-a-b-c-d-` on
  5.3+) and `gmatch("%a*")` must emit spurious empty captures. Our impl deduped
  on all versions. The test FAILED at baseline.
- **Capture overflow.** >32 captures → "too many captures" on every version (not
  version-gated). Green tripwire at baseline.

Both bugs were fixed in the same area via two single-source version helpers,
`matcher_bounds_depth` and `matcher_dedups_empty_match`, verified against all five
reference binaries and the check.sh×5 baseline. Pin reference, fix the impl —
never weaken the test to the impl's wrong output.

#### The perf arbiter that now guards it — the veto loop that fired

The first cut of the empty-match fix regressed gmatch **Ir +0.33–0.54%** on the
matcher workloads (a `state.global()` RefCell borrow + per-match version branches
in `gmatch_aux`, which the Lua `for` loop calls once per match — 5M times on
`string_ops_long`). The baseline-vs-baseline rebuild floor was measured at
**±0.01–0.09%**, proving the regression was signal, not layout noise. It was
driven FLAT (**Ir −0.11% / −0.08%**, within the floor, stable across two builds)
by: restoring the `MatchState` release layout (dropping a debug-only field that
shifted hot offsets); hoisting the version reads to iterator creation; and
**specializing the gmatch step on a `const DEDUP: bool`** (`gmatch_aux` /
`gmatch_aux_legacy`, picked at creation) so monomorphization makes the 5.3+ step's
codegen byte-identical to the single-version baseline — version-gating behavior on
a hot path at ZERO instruction cost. The Ir-flat verdict is the proof the whole
packet (including the matcher) is perf-neutral.

### Left load-bearing — the matcher, untouched in code (only documented)

- **The recursive pattern matcher** — `match_pat` and its helpers (`singlematch`,
  `match_class`, `matchbracketclass`, `classend`, `max_expand`, `min_expand`,
  `start_capture`, `end_capture`, `match_capture`, `matchbalance`,
  `handle_class_with_suffix`). The `goto`→`'outer: loop` tail-call translation,
  the per-byte `match ms.pat[p]` dispatch, and the recursion structure are HOT and
  CPI-critical. **No code line in the matcher changed** — the only P2c edits there
  are doc-comments (an enriched `match_pat` contract note). No local was renamed:
  the names are already clear and faithful, so renaming would be churn on a clean
  load-bearing core. The Ir-flat verdict confirms it is untouched.
- **The version seams are gated OFF the hot path.** `matcher_bounds_depth` (5.1
  has no `MAXCCALLS`) and `matcher_dedups_empty_match` (the 5.3.3 rule) are
  resolved once at the cold entry / iterator-creation point; the per-match step
  never re-reads the version. The 5.1 no-bound case initializes `matchdepth` to a
  high sentinel so the hot `< 0` check stays byte-identical.

### Honest-negatives recorded in this graduation

- **The matcher is idiomatized AROUND, not WITHIN — and that is the SUCCESS.**
  There is no perf-neutral idiomatization inside the hot matcher; the packet
  idiomatized the cold utilities, gated the version seams, and proved neutrality.
  "Matcher load-bearing, idiomatized around it, perf-neutral confirmed" is the
  intended outcome, not a shortfall.
- **A real C seam left ungated because it is unobservable.** The 5.4+
  `changed`/return-original gsub optimization (return the original string object
  when nothing changed) is invisible through the language — strings intern, so a
  rebuilt identical string is `rawequal` to the original. Left applied to all
  versions (behavior-identical). The inverse of the two bugs above: a seam real in
  C but unobservable, where the correct amount of gating is zero (the matcher
  analogue of math's un-pinnable PRNG sequence / table's partition-internal
  invariant).

### Recipes harvested

See the "Recipe ledger" → "### P2c — string" in
`docs/IDIOMATIZATION_SPRINT_2_SPEC.md` — including the capstone methodology
recipe, **"the perf arbiter as a veto gate — idiomatize AROUND a hot load-bearing
core, prove neutrality with Ir/branch-sim, revert any regression."**
