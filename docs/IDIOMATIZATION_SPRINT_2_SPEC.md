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
- [x] P2a: MATH (`math_lib`) — net-strengthened FIRST (PRNG sequence + FloatOnly
      + subnormal, all proven non-tautological and green at baseline), then the
      well-covered pure functions idiomatized; PRNG/ldexp/frexp/version-gates left
      load-bearing; behavioral suite green (oracle 169, math_float_only 2,
      official suite incl. math.lua, check.sh 5.1-5.5 0-fail, workspace, wasm,
      unsafe 0); recipes + verdict below; graduation in
      `crates/lua-stdlib/GRADUATED.md`. Branch `idiom/math` (supervisor PRs).
- [x] P2b: TABLE (`table_lib`) — module arrived ALREADY mostly idiomatic (0
      unsafe, helpers extracted) with a MARGINAL net, so the Phase-2 value was
      net-strengthening, not idiomatization (the second Phase-2 data point). Net
      strengthened FIRST (9 oracle assertions across the 4 recon gaps + sort
      contract, reference-pinned); the `remove` arg-gate test FAILED at baseline
      and caught a real 5.1/5.2 divergence, fixed in the same commit. Then thin
      Tier-1 only: crutch removal (12 stale `TODO(port)`, C-source blocks, 4 C-
      correspondence PORT NOTEs, dangling doc summaries, condensed trailer) +
      `aux_getn`→`check_table_and_get_len` rename; the quicksort core + all
      version gates left load-bearing. Behavioral suite green (oracle 178,
      sort.lua + nextvar.lua PASS, check.sh 5.1-5.5 0-fail at 57/54/23/7/10,
      workspace, wasm, unsafe 0); recipes + verdict below; graduation in
      `crates/lua-stdlib/GRADUATED.md`. Branch `idiom/table` (supervisor PRs).
- [x] P2c: STRING (`string_lib`) — the HOT Phase-2 capstone, the FIRST module
      carrying a PERF arbiter (Ir + branch-sim) ON TOP of the behavioral oracle.
      Net strengthened FIRST (3 `test(string)` commits, reference-pinned: the
      pattern-too-complex gate, the 5.3.3 empty-match advance rule, the
      capture-overflow tripwire); TWO of the three FAILED at baseline, catching
      real 5.1/5.2 pre-5.3.3 divergences (the impl applied the 5.2+ matchdepth
      guard and the 5.3+ empty-match dedup to ALL versions). Fixed in the same
      area via two single-source version helpers (`matcher_bounds_depth`,
      `matcher_dedups_empty_match`); the hot recursive matcher left LOAD-BEARING
      (algorithm/recursion/`'outer: loop`/dispatch untouched, doc-comment only —
      the analogue of math's xoshiro / table's quicksort). Cold-utils crutch
      removal (6 PORT NOTEs, 3 dead format consts, condensed trailer + header).
      **THE CAPSTONE: the perf-arbiter veto loop ran for real** — the first cut
      regressed gmatch Ir +0.33-0.54% (a RefCell-borrow + per-match version
      branches on the 5M-match hot path); proven above the baseline-rebuild
      floor (~0.09%), then driven FLAT (-0.11%/-0.08%, within floor) by a
      const-generic gmatch split + depth-bound fold. Behavioral suite green
      (oracle 181, strings.lua + pm.lua PASS, check.sh 5.1-5.5 0-fail at
      57/54/23/7/10, workspace, wasm, unsafe 0); recipes + verdict below;
      graduation in `crates/lua-stdlib/GRADUATED.md`. Branch `idiom/string`
      (supervisor verifies + runs the AUTHORITATIVE Ir/cold-wall arbiter + PRs).
- [ ] (then, as gates + budget allow: `os` date/time arithmetic — cold/pure)
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

### P2a — math (`math_lib`), 2026-06-14

`crates/lua-stdlib/src/math_lib.rs` is a ~1043-line port of `lmathlib.c`. The
defining Phase-2 lesson landed here: **the net had to be strengthened before the
code could be safely touched.** The behavioral suite was strong for pure algebra
but had three real holes (PRNG sequence, the float-only Int invariant, subnormal
edges) — so the first three commits were `test(math): ...`, each proven
non-tautological by mutation and green against the un-idiomatized baseline,
*before* any `idiom(math): ...` commit. That ordering is the deliverable.

---

**Recipe: `strengthen the behavioral net FIRST` (the Phase-2 precondition in
practice)**
- Pattern: a stdlib module has no structural oracle; some of its behavior is
  pinned only thinly (one sample) or not at all (an invariant `type()` can't see).
  Idiomatizing against that thin net is the one thing Phase 2 forbids.
- Action, in order, as their own commits before touching the module:
  1. **Find the holes by category:** sampled-not-sequenced (PRNG), invisible-to-
     the-language invariants (float-only never-construct-`Int`), and
     buried-edge (subnormal ldexp/frexp pinned in only one place).
  2. **Capture REFERENCE behavior** from the version-suffixed ref binaries
     (`/tmp/lua-refs/bin/lua5.x`) — never the impl's own output (tautological).
     For a sequence, seed once and pin N consecutive draws; for an invisible
     invariant, reach through a lower-level view than the language exposes; for a
     buried edge, promote it into the standard gate so it runs every time.
  3. **Prove each test real by mutation:** break the thing it guards, watch it
     FAIL, restore, watch it PASS. (Here: corrupt a sequence digit; force the
     float-only push to `Int`; swap `ldexp` for naive `x*2f64.powi(e)`.)
  4. **Confirm green at the un-idiomatized baseline** — that proves the net is a
     real net, not a description of the change you're about to make.
- Invariant that replaced the (absent) structural one: oracle 169 (was 165) +
  `cargo test -p lua-stdlib --test math_float_only` (2) + math.lua + check.sh×5.
- Caveat: where a blind spot CANNOT be strengthened against the reference, STOP
  and record an honest-negative — do **not** pin the impl's own output to make the
  number go up. (Here: the 5.1/5.2/5.3 PRNG sequence wraps host C `rand()` — a
  documented platform-dependent divergence; only 5.4/5.5 xoshiro is bit-pinnable.)

**Recipe: `white-box test for an invariant the language can't observe`**
- Pattern: an invariant is real (the reference upholds it) but invisible through
  the language's own introspection — e.g. under 5.1/5.2 `math.random` must yield a
  `Float` not an `Int`, yet those versions have no `math.type` and `type()` says
  `"number"` for both.
- Action: place the test in the consuming crate but reach one layer below the
  language. Here a `tests/` file in `lua-stdlib` takes `omnilua` as a
  **dev-dependency** (omnilua depends on lua-stdlib, so a normal dep would cycle —
  a dev-dep does not, since it is outside the build graph) and inspects the raw
  `Value::Integer` vs `Value::Number` returned by `eval`. Add a CONTRAST case from
  a version where the subtype IS the other one (5.4 `random(5,8)` → `Integer`) so
  the test is proven to exercise the gate, not a coincidental absence.
- Invariant that now guards it: `v51_v52_random_results_are_always_float` +
  `v54_random_interval_is_integer_subtype`.
- Caveat: this is the only way to net a float-only Int invariant; the behavioral
  oracle alone is blind to it. Keep the contrast case or a future change could
  make BOTH versions wrong and still pass.

**Recipe: `name the recurring arg-type dispatch / push helper`**
- Pattern: the same `if matches!(state.value_at(N), LuaValue::Int(_))` int-vs-float
  branch recurs at many sites; a "push Int when it fits exactly, else Float"
  helper carries inline-commented magic bounds.
- Action: extract `arg_is_int(state, n)` (7 call sites → reads as intent);
  rename `push_num_int` → `push_int_or_float` and lift its `i64::MIN as f64` /
  `-(i64::MIN as f64)` bounds into `const I64_MIN_AS_F64` /
  `I64_MAX_PLUS_1_AS_F64` with a doc-comment explaining the half-open test
  (`i64::MAX` is not exactly representable in `f64`). Also name the
  frame-relative `set_top(state, 1)` "return the arg unchanged" idiom as
  `keep_first_arg`, folding the triplicated relative-vs-absolute inline comment
  into one doc.
- Invariant that guards it: pure renames/extractions — the whole behavioral net
  (oracle 169, math.lua, check.sh×5) is unmoved.
- Caveat: net was already STRONG for these pure paths (abs/floor/ceil/modf/type
  are well-covered) — no strengthening needed first; this is the easy half.

**Recipe: `group the load-bearing algorithm into a named private module`**
- Pattern: a cluster of pure functions IS the load-bearing core (here xoshiro256\*\*
  `next_rand`/`rand_to_float`/`project`/`set_seed_words`, pinned bit-exact). You
  want to make "do not refactor this" structurally obvious without touching it.
- Action: wrap them (plus only the constants they use) in a private `mod xoshiro`,
  `pub(super)`, callers reach them as `xoshiro::*`. **Move/group ONLY — every
  function body byte-identical, nothing reordered.** The module doc states the
  contract ("pinned by the sequence tests; do not reorder the arithmetic").
- Invariant that guards it: the PRNG-sequence pins (strengthened in this packet)
  — they are the tripwire that proves the grouping changed no arithmetic.
- Caveat: this is the Phase-2 form of "leave load-bearing." Do it only AFTER the
  sequence net exists; without the tripwire, grouping a bit-exact algorithm is
  unverifiable. Distinguish from P1's "extract for readability" — here the extract
  is to *fence off* code the coarse net cannot fully prove.

**Recipe: `crutch removal on a behavioral-only module`**
- Same as Sprint 1's recipe, with a Phase-2 keep-list twist. Removed: 8 `PORT
  NOTE`s, the `lmathlib.c`/`LUAMOD_API`/`I2UInt` C-correspondence notes, a stale
  `to_integer_opt` `TODO` (the signature it predicted already exists — verified by
  grep, the parser-packet discipline), ~120 lines of dead `MATHLIB`/`LibReg`
  Phase-A scaffolding the live `MATHLIB_FUNCS` path never used, and the
  C-line-count `PORT STATUS` trailer (condensed to point at the net).
- **Keep-list, sharper here because the net is coarser:** EVERY version-gate
  comment (each gate is oracle-checked per-version in isolation — a collapsed gate
  silently breaks a never-construct-`Int` or wrong-arg-index invariant) and the
  ldexp/frexp subnormal bit-math comments. Distinguish *"this is what the C did"*
  (delete) from *"this is the per-version behavior / the subnormal correctness
  reason"* (keep). One genuine `TODO` survives (the thread-local→per-state PRNG
  migration) — kept because it is real deferred behavior, not stale scaffolding.
- Caveat: comment-only but gate it like code (a blank line detaches a `///`).

### P2b — table (`table_lib`), 2026-06-14

`crates/lua-stdlib/src/table_lib.rs` is a ~1054-line port of `ltablib.c`. It
arrived **already mostly idiomatic** (0 `unsafe`, helpers extracted, no dead
scaffolding) with a **marginal** behavioral net. That inverts P2a: math needed a
rich-ish idiomatization on a strong-for-algebra net; table needed almost no
idiomatization but a strengthened net. **The Phase-2 value here was
net-strengthening, not idiomatization** — the second Phase-2 data point, and the
sharper statement of the governing lesson: *idiomatization debt is not uniform —
and neither is net strength*. The honest discipline is to read which of the two
is the binding constraint for THIS module and spend the budget there, rather
than reflexively rewriting clean code.

---

**Recipe: `net-strengthening that catches a bug the weak net was hiding`**
- Pattern: a stdlib module looks behaviorally done (official suite green, roster
  checks pass), but the net only SAMPLES the version seams. When you pin the full
  version matrix of a seam against the reference, an assertion FAILS at baseline —
  the net was hiding a real divergence, not merely under-describing a correct one.
- Concrete: `table.remove` out-of-bounds. The old net checked only the
  5.3-vs-5.4 arg index. Pinning all five versions exposed that our impl errored
  on **5.1** (legacy `ltablib.c` does NO bounds check — out-of-range silently
  removes nothing and returns ZERO results) and reported arg **#2 on 5.2** (ref:
  **#1**, same as 5.3). The new `v_remove_out_of_bounds_arg_gate_crossversion`
  FAILED at baseline (caught the 5.1 case), proving non-tautological; the
  faithful three-way gate (5.1 inert / 5.2+5.3 #1 / 5.4+5.5 #2) — confirmed
  against the 5.1.5/5.2.4/5.3.6/5.4.7/5.5.0 `tremove` sources — landed in the
  same commit, FAIL→PASS.
- Discipline twist vs the packet brief: the brief expected every net test to be
  "green at baseline" (the recon believed the impl matched). When a pinned-to-
  reference test instead FAILS, that is the net doing its job — the resolution is
  to pin reference and fix the impl, NOT to weaken the test to the impl's output
  (that would be the tautology the phase forbids). Keep the test commit's message
  explicit that it caught a divergence.
- Invariant that now guards it: the full-matrix remove test (5.1 inert + zero
  results; 5.2/5.3 arg #1; 5.4/5.5 arg #2), green.

**Recipe: `pin the well-behaved-but-untested edges (green at baseline)`**
- Pattern: most net gaps are NOT bugs — they are correct paths nobody pinned. The
  value is converting "works today, unguarded" into "guarded," so a future
  idiomatization (or a shared-core change) can't silently break them.
- Action: capture each edge from the reference binaries and assert it; confirm
  green at baseline (proving the net describes the reference, and the impl already
  matches). Closed here: 5.1 `__len`-bypass-on-insert vs 5.2+ honoring it (a
  contrast pair, so the gate is provably exercised); `pack.n` counts holes/nils;
  `unpack` empty-range / INT_MAX-span / i64-extreme-wrap boundaries (split 5.2-
  literal cases from 5.3+ `math.maxinteger` cases — the integer subtype is a 5.3
  addition); `move` overlap copy-direction + interleaved `__index`/`__newindex`
  order (forward AND backward); the observable sort contract.
- Caveat: a "green at baseline" net test is still a real net — it is the tripwire
  that lets the idiomatization that follows be verified rather than described.
  Distinguish it from a tautology: it pins the REFERENCE (an independent oracle),
  not the impl's own output.

**Recipe: `thin Tier-1 on an already-idiomatic module — crutch removal + safe rename`**
- Pattern: when the code is already clean, idiomatization is NOT a rewrite. It is
  removing the Phase-A scaffolding crutches and a few intent-clarifying renames —
  and nothing else. Forcing a "rich" rewrite here would be the churn the brief
  warns against.
- Action: (1) remove the 12 stale `TODO(port)` "verify method name" notes — each
  named a `LuaState` method that now exists and is called (proven stale by
  compilation + the green net, the parser-packet discipline). (2) Delete the
  embedded `/// static ... { ... }` C-source blocks and the 4 pure-C-
  correspondence PORT NOTEs from the idiomatized functions, repairing each
  function's orphaned first-line doc fragment into a real summary that points at
  the test now pinning it. (3) Rename `aux_getn` → `check_table_and_get_len` (it
  both type-checks and returns the border — the C name hid that), at the 4 Rust
  call sites only, leaving the C function name verbatim where it refers to
  upstream. (4) Condense the stale PORT STATUS trailer to the math template's
  form (unsafe=0, the one genuine deferred item, the net, the kept perf note).
- Keep-list (sharper because the net is coarse): the genuine deferred-behavior
  note (`check_tab` error-path stack cleanup, rephrased from the stale "Phase B
  should…" to current truth); EVERY per-version roster-delta comment; the
  `remove` version-gate C evidence (5.4.7 + 5.1.5 `tremove` side-by-side — that is
  the per-version-behavior REASON, not generic crutch); and the ENTIRE sort
  cluster's docs + C blocks UNTOUCHED.
- Invariant that guards it: pure comment/rename changes — oracle 178, sort.lua +
  nextvar.lua, check.sh×5 all unmoved.

**Recipe: `fence off the load-bearing algorithm by leaving it ALONE`**
- Pattern: P2a's "group into a private module" fenced off math's xoshiro core.
  table's quicksort core (partition / aux_sort / sort_comp / choose_pivot / set2 /
  randomize_pivot) is the same kind of net-uncoverable algorithm — but here the
  faithful move was even more conservative: leave it **entirely untouched**, docs
  and C-evidence blocks included, rather than re-group it.
- Why not extract/group like math? Two reasons. (a) The cluster is already
  visually contiguous under a `// ─── Quicksort ───` banner — grouping would be
  churn for no readability gain. (b) The net is WEAKER here than for the PRNG: the
  PRNG sequence is bit-pinnable (the tripwire that made math's regrouping
  verifiable), but the partition's internal comparator-callback-during-GC safety
  is NOT behaviorally observable, so there is no tripwire that would catch a
  regrouping bug. With no tripwire, the correct amount of change to a load-bearing
  region is ZERO.
- Invariant: the OBSERVABLE sort contract (stability, invalid-order detection,
  array-too-big, mixed-type compare) is pinned; the partition internals are
  declared load-bearing and not touched.
- Caveat: this is the strongest form of "leave load-bearing" — when even the
  fence-off refactor (regroup/rename) lacks a net to verify it, don't do it.

### P2c — string (`string_lib`), 2026-06-14

`crates/lua-stdlib/src/string_lib.rs` is a ~3150-line port of `lstrlib.c` and the
HOT Phase-2 capstone — the FIRST module whose gate is the **perf arbiter (Ir +
branch-sim) layered on top of the behavioral oracle**. The honest framing going
in (and confirmed): the recursive pattern matcher is *already idiomatic Rust and
CPI-load-bearing* — the `goto`→`'outer: loop` tail-call translation is correct
and perf-critical, with NO perf-neutral idiomatization available inside it. So
P2c is NOT a matcher rewrite. It is: strengthen the net for the matcher's weak
spots, idiomatize the cold utilities around the hot core, leave the matcher
load-bearing, and **prove the whole packet perf-neutral with the Ir/branch-sim
veto gate**. The capstone deliverable is *demonstrating that veto loop running
for real* — including catching and reverting a regression.

---

**Recipe: `the perf arbiter as a veto gate — idiomatize AROUND a hot load-bearing
core, prove neutrality with Ir/branch-sim, revert any regression` (THE P2c point)**
- Pattern: a module has a HOT core whose codegen is perf-critical (the matcher,
  the analogue of math's xoshiro / table's quicksort). You must change *around*
  it — net tests, version gates, cold-utility cleanup — without moving the hot
  core's instruction count or branch behavior. The behavioral oracle says
  "still correct"; it says NOTHING about "still fast." The perf arbiter is the
  second, independent truth-teller, and it has VETO power.
- Action, the loop that actually ran here:
  1. **Freeze a release baseline BEFORE any edit** (`cp target/release/omnilua
     /tmp/lua-rs-string-base`), record its sha. Ensure `Cargo.lock` exists (the
     instr-count container mounts `/src:ro` and needs it).
  2. Do the behavioral work (net + fixes + cold idiomatization), gating each
     step behaviorally green.
  3. **Run the Ir arbiter on the matcher-heavy workloads** (`instr-count.sh
     --workloads string_ops,string_ops_long --branch-sim`) and diff the `rs`
     row vs baseline. The first cut here came back **Ir +0.33–0.54%** — a real
     regression.
  4. **Establish the noise floor before believing a small delta.** Rebuild the
     SAME baseline commit twice and measure the Ir spread (here **±0.01–0.09%**,
     layout-only). +0.33% was 4–50× the floor → REAL, not rebuild noise. This
     step is non-negotiable: Ir is "effectively exact" for a fixed binary, but
     a rebuild reshuffles layout by up to ~0.5%, so a sub-floor delta is noise
     and a supra-floor delta is signal — you cannot tell which without the floor.
  5. **Localize and kill the regression** (this is the veto loop):
     - The regression was per-MATCH work in `gmatch_aux`, which the Lua `for`
       loop calls once per match (5M times on the long workload). Three causes,
       fixed in order, each re-measured: (a) an added `MatchState` field that
       shifted hot struct offsets — dropped (release layout restored); (b) a
       `state.global()` **RefCell borrow** read per match — hoisted to iterator
       creation; (c) the residual per-match runtime branch on the version flag —
       killed by **specializing the step on a `const DEDUP: bool`** (two thin
       closures, `gmatch_aux`/`gmatch_aux_legacy`, picked at creation) so
       monomorphization folds the seam to constants and the 5.3+ step's codegen
       is byte-identical to the single-version baseline; plus folding the
       depth-bound select (`DEDUP=true ⟹ bound_depth=true`, a compile-time
       constant) with `#[inline]` on `MatchState::new`.
     - Final: **Ir −0.11% / −0.08%**, both WITHIN the rebuild floor → FLAT.
       Confirmed stable across two final-branch builds (spread 0.003%/0.02%).
- Invariant: the perf arbiter VERDICT (Ir flat within the measured rebuild floor,
  Bc flat) is the second gate the module must pass, alongside the behavioral
  suite. The number that enters the spec is re-measured by the supervisor on a
  cold machine (the authoritative Ir + cold-wall arbiter).
- Caveat — the const-generic specialization is the reusable trick: when a hot
  path must branch on a per-instance-constant seam, lift the seam to a `const`
  type parameter and bind it where the instance is created (here: which closure
  to register), NOT a runtime field the hot loop re-reads. The compiler then
  produces one specialized, branch-free path per value. This is how you gate
  behavior by version on a hot path at ZERO instruction cost.

**Recipe: `net-strengthening that catches a version-seam bug the matcher hid`**
- Same shape as P2b's `table.remove` finding, on the matcher. The behavioral net
  (pm.lua) exercises the matcher heavily but never HITS its danger-zone edges, so
  the edges were unguarded — and two of them hid real 5.1/5.2 divergences:
  - **`matchdepth` / "pattern too complex".** Pinning the bound across versions
    exposed that our impl raised on **5.1**, where `lstrlib.c` `match()` has NO
    depth counter (`MAXCCALLS` was a 5.2 addition; verified zero in the 5.1.5
    source) and a too-deep pattern simply MATCHES. The test FAILED at baseline.
  - **Empty-match advance (the 5.3.3 change).** Pinning `gsub(" *","-")` and
    `gmatch("%a*")` per-version exposed that our impl applied the 5.3+
    `e != lastmatch` de-dup to **5.1/5.2**, which lack it — so `gsub` should
    DOUBLE to `-a--b--c-d-` and `gmatch` should emit spurious empty captures.
    The test FAILED at baseline.
  - **Capture overflow** (>32 → "too many captures"): NOT version-gated, green
    at baseline — a clean tripwire.
- Resolution (the discipline, same as P2b): pin REFERENCE, fix the impl, never
  weaken the test to the impl's wrong output. Both fixes are single-source
  version helpers (`matcher_bounds_depth`, `matcher_dedups_empty_match`); the
  matcher recursion itself is untouched (the 5.1 no-bound case is `matchdepth`
  initialized to a high sentinel, so the hot `< 0` check stays byte-identical).
- Caveat — the `changed`/return-original gsub optimization (5.4+) is NOT
  behaviorally observable (strings intern, so a rebuilt identical string is
  `rawequal` to the original), so it is left applied to all versions: an
  honest "this seam is real in C but invisible through the language, do not
  bother gating it" — the inverse of the bugs above (seams that ARE observable).

**Recipe: `treat the hot matcher as LOAD-BEARING — doc-comment only, prove it`**
- Pattern: P2a fenced math's xoshiro into a private module; P2b left table's
  quicksort entirely untouched. The matcher is a third point: it is the HOT inner
  loop of every `string.*` pattern op, and unlike those two it is *exercised on a
  perf-measured workload*, so "leave it alone" is enforced by the Ir arbiter, not
  just by argument.
- Action: the ONLY hot-path edits made were (a) an enriched `match_pat`
  doc-comment recording the load-bearing contract in-place (do not extract
  helpers / convert the loop / replace the dispatch) and (b) reframing one
  helper's doc. NO code line in the matcher changed; no local was renamed (the
  names — `b`/`e`, `cont`, `count`, `level`, `what`, `s`/`p`/`ep` — are already
  clear and faithful, so renaming would be churn on the load-bearing core). The
  Ir-flat verdict is the *proof* the matcher is untouched: a hot-core edit would
  have moved string_ops_long's 21.8B-instruction count past the floor.
- Caveat: the spec permitted local renames as "Ir-neutral by construction." The
  honest finding is that even an admissible edit is not worth making on a clean
  load-bearing core — the correct amount of change to the matcher was effectively
  ZERO code, ONLY docs (the P2b "leave load-bearing alone" lesson, now with a
  perf tripwire that would catch a violation).

## Verdict ledger
(append per-module outcomes — graduated OR honest-negative-with-reason)

### P2a — math: GRADUATED (2026-06-14)

Graduated. Net strengthened first (3 `test(math)` commits, all non-tautological
and green at baseline), then 6 `idiom(math)` transformations, each gated
behavioral-green. Final state: oracle 169, `lua-stdlib` `math_float_only` 2,
official suite incl. math.lua PASS, check.sh 5.1-5.5 at baseline 57/54/23/7/10
(0 fail), workspace green, wasm check OK, **unsafe blocks 0**. Graduation doc:
`crates/lua-stdlib/GRADUATED.md` "math". Branch `idiom/math` (supervisor verifies
+ PRs).

**Honest-negative (within an otherwise-graduated module):** the spec asked for a
seeded-sequence pin on "at least one of 5.1/5.3 (the older PRNG path)." Neither is
pinnable to the reference: 5.1/5.2/5.3 wrap the host C `rand()`/`random()`, whose
byte stream is platform-dependent — a KNOWN, DOCUMENTED allowed divergence
(`specs/followup/5.1-numbers-prng.md`, `specs/research/5.3-upstream-delta.md`;
re-confirmed empirically here: our 5.3 output uses xoshiro and diverges from the
5.3.6 reference for every tested seed). Pinning our own 5.3 output would be
tautological — the one move this phase forbids. Resolution: bit-pin the xoshiro
path (5.4 + 5.5) where it IS exact, and pin the 5.1/5.2/5.3 **contract**
(range/type/shape/arg-error) rather than the sequence. This is a correct STOP on
a blind spot, not a coverage gap.

**Out of scope, noted not fixed:** `math.fmod(x,0)` error omits the function name
(`bad argument #2 (zero)` vs `... to 'fmod' (zero)`) — a shared-core arg-naming
gap tracked separately.

### P2b — table: GRADUATED (2026-06-14)

Graduated. The module arrived **already mostly idiomatic** (0 `unsafe`, helpers
extracted) with a **marginal** net, so — inverting P2a — the value was
net-strengthening, not idiomatization (the **second Phase-2 data point**:
idiomatization debt is not uniform, and neither is net strength). Net
strengthened FIRST: 9 oracle assertions (oracle 169 → 178) across the four recon
gaps + the observable sort contract, each pinning the version-suffixed reference
binaries. Then thin Tier-1 only (crutch removal + the `aux_getn` rename); the
quicksort core and all version gates left load-bearing. Final state: oracle 178,
sort.lua + nextvar.lua PASS, check.sh 5.1-5.5 at baseline 57/54/23/7/10 (0 fail),
workspace green, wasm check OK, **unsafe blocks 0**. Graduation doc:
`crates/lua-stdlib/GRADUATED.md` "table". Branch `idiom/table` (supervisor
verifies + PRs).

**Bug caught by the strengthened net (fixed in this packet):** `table.remove`
out-of-bounds was gated only `V53→arg1 else arg2`. The full-matrix pin exposed
two divergences from the reference — **5.1** must NOT bounds-check at all (legacy
`ltablib.c`: out-of-range silently removes nothing, returns ZERO results; our
impl raised), and **5.2** must report arg **#1** (our impl reported #2). The new
test FAILED at baseline, proving it pins reference; the faithful three-way gate
(5.1 inert / 5.2+5.3 #1 / 5.4+5.5 #2) was landed in the same commit, verified
against all five `tremove` sources + binaries and the check.sh×5 baseline.

**Honest-negative (within an otherwise-graduated module):** the sort partition's
*internal* invariant — the comparator callback cannot corrupt partition state
even if it triggers a GC or mutates the array mid-sort — is **not behaviorally
observable** and so cannot be reference-pinned (the table analogue of math's
platform-dependent 5.1/5.2 PRNG sequence). The net pins the observable sort
contract (stability, invalid-order detection, mixed-type compare, array-too-big)
and STOPS; the quicksort core is fenced off as load-bearing and left ENTIRELY
untouched. Unlike math's xoshiro (regrouped into a private module under cover of
the bit-exact sequence tripwire), the partition was NOT even regrouped — with no
behavioral tripwire to verify a regroup, the correct change to a load-bearing
region is zero.

**Tier-2 loop refactor declined (honest-negative):** the repeated
wrapping-subtract bounds idiom (`(pos - 1) < bound`) in insert/remove/move/unpack
was a candidate to factor into a shared range-check helper. Declined: extracting
it without a dedicated equivalence unit test (which the brief required as the
precondition) would itself be unverified churn on edge-case logic the coarse net
guards only partially. Left as-is.

### P2c — string: GRADUATED (2026-06-14)

Graduated — the HOT Phase-2 capstone, and the first module gated by the **perf
arbiter (Ir + branch-sim) on top of the behavioral oracle**. The headline is not
"the matcher was idiomatized" (it wasn't — it's already idiomatic and
load-bearing) but **"the perf-arbiter veto loop ran end-to-end and worked."**

Net strengthened FIRST: 3 `test(string)` commits (oracle 178 → 181), each pinning
the version-suffixed reference binaries on a matcher danger-zone pm.lua doesn't
exercise — pattern-too-complex, the 5.3.3 empty-match advance rule, capture
overflow. TWO **FAILED at baseline**, catching real 5.1/5.2 pre-5.3.3 divergences
(see the bug note below). Then: the version fixes via two single-source helpers;
crutch removal on the cold surface (6 PORT NOTEs, 3 dead format consts, condensed
trailer + module header); the hot matcher left load-bearing (doc-comment only, no
code line moved). Final state: oracle 181, strings.lua + pm.lua PASS, check.sh
5.1-5.5 at baseline 57/54/23/7/10 (0 fail), workspace green, wasm OK, **unsafe
blocks 0**. Graduation doc: `crates/lua-stdlib/GRADUATED.md` "string". Branch
`idiom/string` (supervisor verifies + runs the AUTHORITATIVE Ir/cold-wall arbiter
+ PRs).

**The perf-arbiter VETO that fired (the capstone evidence):** the first cut of the
empty-match fix regressed gmatch **Ir +0.33–0.54%** on `string_ops`/`string_ops_long`
— a real per-match cost (a `state.global()` RefCell borrow + runtime version
branches in `gmatch_aux`, which the Lua `for` loop calls once per match, 5M times
on the long workload). The baseline-vs-baseline rebuild floor was measured at
**±0.01–0.09%**, proving the +0.33% was signal not layout noise. It was driven
FLAT (**Ir −0.11% / −0.08%**, within the floor, stable across two builds) by:
restoring the `MatchState` release layout; hoisting the version reads to iterator
creation; and **specializing `gmatch` on a `const DEDUP: bool` step** (two
closures picked at creation) so the 5.3+ path's codegen is byte-identical to the
single-version baseline — version-gating behavior on a hot path at ZERO
instruction cost. This is the module's reusable trick and the literal
demonstration of the gate's veto power.

**Bug caught by the strengthened net (fixed in this packet):** our matcher applied
two 5.2+/5.3+ behaviors to ALL versions. (1) The `MAXCCALLS` "pattern too complex"
guard was added in **5.2** — 5.1's `lstrlib.c` `match()` has no depth counter, so
a too-deep pattern MATCHES on 5.1; our impl raised. (2) The 5.3.3 `e != lastmatch`
empty-match de-dup is absent on **5.1/5.2**, so `gsub(" *","-")` must double to
`-a--b--c-d-` and `gmatch("%a*")` must emit spurious empty captures; our impl
deduped everywhere. Both tests FAILED at baseline (pinning reference), both fixed
in the same area via `matcher_bounds_depth`/`matcher_dedups_empty_match`, verified
against all five reference binaries + the check.sh×5 baseline.

**Honest-negative — the matcher is load-bearing, idiomatized AROUND not WITHIN
(a SUCCESS, not a shortfall):** the recursive matcher (`match_pat` + helpers, the
`goto`→`'outer: loop` tail-call translation, the per-byte dispatch) is already
idiomatic Rust and CPI-critical. There is NO perf-neutral idiomatization inside
it — the analogue of math's xoshiro and table's quicksort, but stronger because
it's *exercised on a perf-measured workload*, so "leave it alone" is enforced by
the Ir arbiter, not just argued. The packet idiomatized the cold utilities, gated
the version seams off the hot path, and proved neutrality; the matcher's code is
byte-for-byte unchanged (only doc-comments). The Ir-flat verdict IS the proof.

**Honest-negative — a real C seam left ungated because it's unobservable:** the
5.4+ `changed`/return-original gsub optimization (return the original string
object when nothing changed) is not behaviorally observable — strings intern, so
a rebuilt identical string is `rawequal` to the original. It is left applied to
all versions (behavior-identical). This is the inverse of the two bugs above
(observable seams that had to be gated): a seam that is real in C but invisible
through the language, where the correct amount of gating is zero.
