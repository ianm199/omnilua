# Backlog-crush campaign — 2026-07-15

Goal: drive the open-issue count to zero (or to a defensible parked state
with evidence), autonomously, under the supervisor+subagent pattern. The
supervisor (Fable) does the upfront planning here, builds/specs the custom
kits each ticket needs for a fast inner loop, runs the heavy gates + codex
rounds, and decides keep-vs-nuke by evidence. Honest negatives are
first-class outcomes — a well-evidenced "close as won't-fix" or
"measured-neutral, reverted" counts as crushing the ticket.

This file is the referenceable source of truth. It is updated as tickets
resolve. Do not trust a hardcoded status elsewhere — the GitHub issue state
and this table's "Outcome" column are authoritative.

## The open tickets (at campaign start)

| # | Title (short) | Class | Approach | Kit | Verdict authority |
|---|---|---|---|---|---|
| 267 | Gc boxes carry no owner identity | GC soundness | Implement spec-recommended **C/D** (deref-free guards + free HDR_FREED tripwire + u8 owner-id in padding + seal the raw surface). Spec PR #290 merges first as docs. | **stale_handle_kit** (new) | supervisor + codex |
| 282 | loadlib 5.1/5.2 path/env fidelity | oracle fidelity | Oracle-driven fidelity fixes vs the 5 reference binaries, follow-up to #273. | existing loadlib oracle probes | supervisor + oracle |
| 291 | FREELIST_REF collides with 5.5 mainthread slot | latent version bug | Version-gate FREELIST_REF cheaply; unreachable today (luaL_ref has no callers) so no test for the uncallable path — insurance only. Fold into #282's loadlib lane. | (none) | supervisor |
| 278 | Embedding-API stubs grab-bag | judgment / triage | Triage each sub-item into FIX / DOCUMENT / CLOSE by common sense (real embedding correctness → fix; niche/uncallable → document or close). Split the parse-codegen items to their own issue if kept. | **embedding_api_kit** (new, as needed) | supervisor |
| 113 | RSS object diet (parked, W2 killed) | perf, measure-first | **Analysis first, no blind surgery.** Build the size-class histogram tool; rank shrink candidates by malloc-bucket-crossing × population (the only lever that's actually paid off — see PERF_EVIDENCE_113_W2). Implement ONLY a concretely-measured bucket-crossing win; else document the finding and park with evidence. | **size_class_histogram** (new tool) | supervisor + Ir/RSS |

## Iteration-cycle analysis (where the inner loop lives, and the kit that speeds it)

The discipline (CLAUDE.md "custom subsystem testers"): before grinding a
subsystem against the slow end-to-end oracle, build a small in-memory
deterministic kit that exercises exactly that part. Per ticket:

- **#267** — the inner loop is *use-after-close / foreign-heap scenarios*.
  The full-VM oracle CANNOT easily produce a use-after-teardown (that's the
  whole reason the bug is invisible). So the inner loop must be a
  **stale_handle_kit**: construct a `Heap` + `Gc`/`GcRef` directly, script
  the F1/F2b/F2c/F4 failure cases from the spec (no-guard downgrade of a
  freed box; foreign-heap token mint; same-heap swept-then-re-downgrade;
  account_buffer on stale), and assert the guard/tripwire fires. Milliseconds,
  100%-reproducible, where a real socket/VM reproduces it never. This kit is
  the deliverable that makes the fix verifiable at all.

- **#282** — inner loop is `package.path`/`cpath`/env-precedence bytes vs
  each reference version. The kit already exists: `specs/oracle/diff_one.sh
  <ver>` + the hermetic `HostHooks::env` fake from #273's
  loadlib_strengthen.rs. Rung 3 (diff_one) is the loop; rung 6 (official +
  check.sh ×5) is the gate. No new kit.

- **#278** — inner loop is *per-API behavior from the host side*. Where an
  item is behaviorally checkable (to_close/TBC, to_cfunction, the "0x?"
  address placeholder), drive it from a Rust test comparing to reference
  semantics — an **embedding_api_kit** only if the existing
  crates/lua-rs-runtime/tests/ harnesses don't already reach it. Many items
  are triage-by-reading, not code (uncallable stubs → close).

- **#113** — inner loop for the ANALYSIS is a **size_class_histogram** tool
  (new): allocate a representative workload, dump every live `GcBox<T>`'s
  `size_of` against the platform allocator's size-class table (macOS
  nano/scalable zones: 16-byte quanta to 256, then 512-byte, etc.), and
  rank each object type by (bucket-step-if-shrunk × live-population). This
  is a READ-ONLY analysis kit — it decides whether any surgery is worth it
  BEFORE surgery. If it finds a win, the implement inner loop is instr-count
  (Ir, deterministic, cachegrind) + heap-diff for RSS, per MEASUREMENT_PROTOCOL.

## Keep-vs-nuke decision framework (the common-sense rules)

- **A change ships** iff: it fixes a confirmed-reachable bug OR removes dead
  code OR is oracle-verified fidelity, AND passes the full gate battery, AND
  a codex round finds no unaddressed correctness defect. Comment-only/doc
  changes need only the mechanical + build gates.
- **A change is nuked (reverted/closed)** iff: measured-neutral/negative on
  its own success metric (the W2 precedent), OR it targets an uncallable
  path where the fix adds risk without reachable benefit, OR the "bug" turns
  out to be correct-as-is on oracle inspection. A nuke with evidence closes
  the ticket as effectively as a merge.
- **An item is documented-and-parked** iff: it's a real gap but the fix is
  disproportionate to reach (niche embedding features, multi-day redesigns
  whose payoff isn't yet justified). Park with a precise doc + issue note,
  don't leave a silent gap.

## Wave plan (file-disjoint parallelism; supervisor merges + rebases)

**Wave A** (launch together — file-disjoint):
- #267 → Opus, owns lua-types/gc.rs + lua-gc/heap.rs + lua-rs-runtime seal
  surface; builds stale_handle_kit first.
- #282+#291 → Sonnet, owns lua-stdlib/loadlib.rs.
- #113-analysis → Opus, READ-ONLY (builds size_class_histogram, produces a
  findings doc + go/no-go verdict); no source mutation, so it cannot collide.

**Wave B** (after A resolves the file locks):
- #278 → triage lane; fixes land after #267 frees api.rs if they overlap;
  #291 already folded into #282.
- #113-implement → ONLY if the analysis returns a concrete bucket-crossing
  win; sequenced after #267 (both touch heap.rs).

## Operating rules for every lane (carried from the proven method)

Background+poll any command >2 min; incremental commits from edit one; phase
pings to main only; ≤400-line read slices; stage files explicitly (never
`git add -A`, never stage reference/lua-c or harness/impl/official/); NEVER
`git stash` in a worktree (shared refs/stash — CLAUDE.md); push via the
`gh auth token -u ianm199` one-shot credential override; agents live on
ladder rungs 1–4, supervisor runs official/Ir/RSS/codex/merge; stop-the-line
on any canary/official flip; codex fix-rounds capped at ~3 with triage
(FIX / DEFER-to-issue / REBUT).

## Outcome log (updated as tickets resolve)

- **#282 — CLOSED, no code.** All five gaps were already resolved by #273's
  completion; re-verified against live reference binaries. Editing correct
  code is pure risk.
- **#291 — FIXED (#293).** Version-gated FREELIST_REF; the fix corrected the
  ticket's own wrong assumption (real per-version values 0/0/0/3/1, verified
  against each lauxlib.c). Unreachable path (luaL_ref has no callers) — cheap
  insurance.
- **#113 — analysis DONE (#296), one candidate identified.** size_class_histogram
  tool + ranked findings: `GcBox<UpVal>` 56→48 is the ONLY box that both
  crosses a libmalloc class and has population (~100k on closure_ops, ~4–6%
  slot-byte projection). Everything else fills its class exactly → the rest
  of the GcBox diet is parked; remaining RSS gap is buffer representation (a
  separate track). Candidate 1 → Wave B, arbiter-gated (drop-if-neutral).
- **#267 — PARTIAL mitigation MERGED (#294).** The cheap C/D fix closes the
  no-guard release UAFs (F1/F3/F4) deref-free + byte-neutral — Codex-confirmed
  sound. The HDR_FREED tripwire + owner_gen attempt at the
  foreign/stale-after-sweep cases was NUKED (Codex: it re-derefs a freed
  header — a new UAF in the check). Those cases genuinely need option B
  (slot-indexed handles) → filed as **#295**. Both Codex r2 findings landed
  before merge: the spec carries a SUPERSEDED-IN-PART banner marking the
  tripwire REJECTED ("do not re-implement"), and `GcBox<u64>` has target-gated
  const size asserts (32B/24B) beside the `GcHeader` ones. Issue #267 stays
  open only as the option-B tracker (→ #295); the reachable bug is fixed.
- **#113 UpVal shrink (candidate 1) — MERGED (#298).**
  `struct UpVal { state: Cell<UpValState> }` with `enum UpValState { Open{...},
  Closed(LuaValue) }` collapses three Cells → one; UpVal 32→24, GcBox<UpVal>
  56→48 (crosses libmalloc 64→48). Measured **−16.71% closure_ops RSS**,
  +1.924% Ir, **wall FLAT** (best-of-7 A/B) → KEEP: the layout arbiter is wall,
  and CPI absorbs the Ir via better cache packing, so it's a pure RSS win.
  Codex r1 caught a real wasm32 u64-thread-id truncation (usize round-trip at
  the API boundary); agent threaded u64 end-to-end, size-neutral. Codex r2
  APPROVED (no residual narrowing on any path; close copies-before-transition;
  Cell sound; 48B const-asserted). Supervisor official 44/44. #113 stays open
  as the broader RSS-representation tracker — the histogram (#296) showed UpVal
  was the ONLY worth-it box shrink; remaining RSS gap is buffer representation,
  a separate track.
- **#278 embedding-API grab-bag — CLOSED, MERGED (#299).** Official 44/44 on
  branch, codex r3 APPROVED, issue auto-closed.
  register_c_function extraction + dead-code deletions Codex-clean from r1. The
  `debug.debug` "0x?" fix via `to_display_string` delegation exposed a real
  luaL_tolstring fidelity cluster; fixed faithfully in one pass (r1: 6 findings —
  #276-class gc-check, luaL_tolstring kind lookup, LightC real address,
  numeric→string slot, upval-count bounds, docs). r2 found the LightC address fix
  was incomplete (3 divergent pointer resolvers); r2 fix unified them into one
  `value_identity_pointer` (deleted `value_pointer`/`raw_value_pointer`), so the
  public handle `to_pointer()` now equals the VM `%p`. r3 APPROVED. Lesson: a
  "grab-bag" hid a real cross-path pointer-identity contract; the codex loop
  (3 rounds) is what surfaced it. Kept, not split.
- **#300 `<const>` folding — CLOSED, MERGED (#303).** Executed via
  deep-spec→codex→execute. The nvarstack/reglevel register-watermark refactor
  landed as a bisectable byte-identical commit (sha256-identical bytecode across
  56 dump entries), then the fold. Codex r1 found the 5.5 barrier only inspected
  the innermost function (enclosing `global x` failed to shadow a folded CTC) →
  fixed with a per-level recursive barrier walk mirroring reference 5.5. Official
  44/44, multiversion 185/185, all oracle cases match 5.4/5.5, pre-5.4 byte-
  identical. Filed **#304** (the analogous regular-local/UpVal shadowing bug the
  agent surfaced — same root, different resolution path, deferred to keep blast
  radius minimal). Lesson: the value-model workflow earned its keep — the spec
  review turned "wire two functions" into "a register-accounting refactor is the
  prerequisite," which is exactly what shipped.
- **#301 io/os errno fidelity — CLOSED, MERGED (#302).** Official 44/44 +
  check.sh ×5 (57/54/23/7/10, 0 failed) + wasm gate. Took 3 codex rounds (each
  caught a real defect: wasm compile break, os.remove errno-clobber, live
  errno-0 fallback, then a wasm-strerror regression the errno ABI introduced +
  the write-side tuple, then a Windows Win32-mistranslation). Applied keep-vs-
  nuke on round 3: fixed the one true regression (gate posix_strerror to
  unix+wasm) myself and SPLIT the two incomplete-feature/edge items (wasm errno-1
  ABI encoding, short-write tuple, Win32→errno normalization) to **#305** rather
  than let the grab-bag spiral. Source-breaking hook signature change
  (Result<_,LuaError> → io::Result) is CHANGELOG-noted. Root cause
  confirmed:
  the FileOpenHook/Remove/Rename type alias was `Result<_, LuaError>`, which
  structurally can't carry a numeric errno — so every hook had to stringify the
  io::Error (verbose `(os error N)` Display) and drop `raw_os_error()`, giving
  errno 0. Fix retypes the hooks to `io::Result` (a source-breaking public API
  change — accepted for fidelity, CHANGELOG-noted) + an exact `(os error N)`
  suffix-strip. Codex confirmed the core sound but found a wasm compile break
  (cfg(wasm32) hooks still on the old signature — the Homebrew-rustc-no-wasm-std
  gotcha means native build hid it), an `os.remove` errno-corruption (remove_dir
  clobbers the real unlink error), a still-reachable `unwrap_or(0)` fallback
  (no-fallback-rule violation), and a 5.1 io.input wording gap → fix-round.
- **#300 `<const>` compile-time-const folding — spec + execution dispatched
  (PR pending).** Deep-spec → adversarial-codex → execute (value-model workflow).
  The spec review PAID OFF: v1 assumed the port had all VCONST plumbing and just
  needed two functions wired; codex found two CRITICAL prerequisites — (1)
  `FuncState` can't reach `DynData.actvar`, so discharge can't read `const_val`
  (needs an `ExprPayload` const snapshot), and (2) codegen uses `nactvar` as the
  register watermark, but a folded const bumps `nactvar` without a register (needs
  `nvarstack`/reglevel decoupling — a real refactor). Spec v2
  (`specs/ISSUE_300_CONST_FOLDING_SPEC.md`) folds in all 6 findings + the GC
  verdict (no UAF — the loader stops GC over the whole parse window). Executing
  the watermark refactor as a standalone byte-identical step FIRST, then folding.

## Campaign end-state (2026-07-15)

Every ticket that was open at campaign start is resolved or parked with evidence
— the goal ("drive to zero OR to a defensible parked state with evidence") is met.

**Shipped this campaign:** #282 (closed, no-code), #291 (#293), #113-analysis
(#296), #267 partial mitigation (#294, then closed — reachable UAFs fixed,
residual == #295), #278 (#299), #113 UpVal candidate (#298), #300 `<const>`
folding (#303), #301 io/os errno fidelity (#302).

**Also SHIPPED (picked up after the initial end-state, at the user's "crush #304
next" call):**
- **#304 — CLOSED, MERGED (#307).** Turned out to be much bigger than one bug:
  the adversarial spec-review DISPROVED the naive post-hoc fix (it leaves phantom
  upvalues — `new_upvalue` mints the capture before any post-check runs) and
  specified the correct design — move the 5.5 global/local decision INSIDE
  `singlevaraux`, per-frame, threading a barrier-slice upper bound. The refactor
  fixes **four** real 5.5 divergences (captured-local shadowing, global-function
  precedence, named-vararg shadowing, `global function` phantom capture) + the
  `_ENV`-rejection cases, and REMOVES the post-hoc barrier arms,
  `ctc_shadowed_by_global` (the #300 helper, now subsumed), and the
  `global_function_names` stack. ≤5.4 byte-identical (sha256-proven), #300 CTC
  cases green, codex APPROVED (all 6 points, no findings), official 44/44. The
  spec-review paid for itself AGAIN — it caught that #304 was a resolution
  refactor, not a one-liner, and surfaced 3 extra divergences.

**Parked / documented children (the defensible remainder):**
- **#305** — errno-fidelity residuals split from #301: wasm errno-1 (EPERM) ABI
  encoding, short-write result tuple, Win32→CRT-errno normalization. Edge /
  other-platform; the Darwin-observable bug is fully fixed.
- **#295** — GC option-B slot-indexed handles (multi-day redesign; the only
  sound fix for the foreign/stale-after-sweep cases). Parked with a full spec.
- **#113** — remaining RSS gap is buffer representation, a separate track from the
  object-header diet (which is exhausted at UpVal per the #296 histogram).

**Release 0.7.0 — PREPPED, awaiting the maintainer (irreversible publish).**
**PR #306** stages the version bump 0.6.0 → 0.7.0 (breaking-change minor on the
0.x line, because #301's file-hook signature is source-breaking) + the CHANGELOG,
bundling **#298/#299/#300/#302/#304**. Tagging the merge SHA `v0.7.0` publishes
irreversibly to crates.io + npm — the maintainer approved *prepping* the release
but the tag push is theirs. The supervisor did NOT tag.

**Method notes (for the harness retrospective):** the supervisor+subagent +
codex-loop pattern held up across 4 parallel lanes. Every codex round earned its
cost — each caught a real defect that would otherwise have shipped (a wasm compile
break the native toolchain hid, an `os.remove` errno-clobber, an incomplete
three-way pointer-identity contract, an enclosing-function barrier gap). The
deep-spec→codex→execute discipline for #300 converted a wrong "wire two functions"
plan into the correct "register-accounting refactor is the prerequisite" — the
spec review paid for itself. Keep-vs-nuke was exercised as authority, not
ceremony: the UpVal RSS win was KEPT on a wall-flat/RSS-down measurement, and
#301's round-3 grab-bag spiral was cut by fixing the one true regression and
splitting the rest to #305.
