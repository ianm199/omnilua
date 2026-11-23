# Overnight session spec — 2026-06-12 → 13

## STATUS: COMPLETE (2026-06-13). 13 PRs merged (#173-#184 + this summary).
T1 Bevy demo (#175, compiles to wasm — wedge proven) · T2 content drafts
(#173) · T3 release dry-run GO (#174) · T4 concat perf (#176, Ir -13.5%) ·
T5a tag-layout (#177) · T5b unwind NO-GO memo (#183) · plus the discovered
RSS grind: closure-box (#180), table-box (#181), string-box (#182), intern-
shrink (#184); fasttm DROPPED (measured negative). Full outcomes +
roadmap verdict in docs/PERF_ROADMAP_AUDIT.md. Hard line held: nothing
tagged/published/posted; v0.1.0 tag + 2 content drafts await the user.

Owner: Fable (supervision, merges, review). Execution: Opus subagents.
User is asleep; everything here is reviewable in the morning.

## HARD BOUNDARY (holds all night, no exceptions)

- **No `git tag` / no `cargo publish` / no `npm publish`** — the v0.1.0 tag is
  the user's irreversible call per RELEASING.md.
- **No posting anything public** (no HN/Reddit/social, no PRs to external
  repos). Drafts land as in-repo markdown for the user to post.
- Everything else: normal PR-per-packet, merge when gates green + reviewed.
- The measurement protocol (`docs/MEASUREMENT_PROTOCOL.md`) governs every perf
  claim: frozen baseline, Ir/branch-sim arbiter, drop-if-neutral, honest
  negatives are deliverables.

## Tracks

| # | Track | Surface | Bench host? | Risk |
|---|---|---|---|---|
| T1 | Bevy wedge demo | new `examples/bevy/` (NON-workspace member) | no | open-ended |
| T2 | Launch content drafts | new `docs/posts/*.md` | no | none |
| T3 | Release dry-run report | read-only + `docs/RELEASE_DRYRUN.md` | no | none |
| T4 | Perf: concat string churn | lua-stdlib/lua-vm concat path | YES | bounded |
| T5a | Repr rung 1: tag layout | lua-types `LuaValue` discriminants | YES (after T4) | bounded |
| T5b | Repr rung 2: unwind-error PROTOTYPE | one subsystem, unmerged branch | YES (after T5a) | measurement only |

## Conflict / sequencing

- T1, T2, T3 are file-disjoint from core and from each other → run in parallel
  immediately (wave 1), alongside T4.
- T4 owns the core string/concat path AND the bench host in wave 1. T4's
  arbiters (instr-count, heap-diff) are deterministic, so T1/T3 builds running
  concurrently don't corrupt them.
- **T5a conflicts with T4** (both touch lua-types value + hot VM tag reads) and
  needs the bench host → runs AFTER T4 merges (wave 2).
- T5b (unwind-error) is a huge cross-cutting change; it is a **measurement
  prototype on ONE subsystem only**, on an unmerged branch like the T4
  safety-tax ablation — NOT a full conversion, NOT merged. Runs after T5a.

## Per-track briefs

### T1 — Bevy wedge demo
A minimal, self-contained Bevy example proving "Lua scripting that follows your
game to the browser." Scope: `examples/bevy/` with its OWN Cargo.toml, excluded
from the workspace (do NOT add bevy to the core workspace — it bloats CI).
A tiny app: a Bevy system that loads a Lua script via the `omnilua` embedding
crate (path dep) and the script drives some entity state (move a sprite, mutate
a resource) hot-reloadable. If a native `cargo run` works, attempt a
`wasm32-unknown-unknown` build and document it. README in the example dir.
NOTE the existing external `bevy-lua-rs-starter` repo (linked from the main
README) and report whether this should fold into updating that vs stay fresh —
do NOT touch the external repo. STOP and document if Bevy API/version
integration thrashes rather than burning the night on it.

### T2 — Launch content drafts
`docs/posts/safety-tax.md` — the "we measured what memory safety costs" post,
assembled from `docs/PERFORMANCE_MODEL.md` §Safety-tax ablation + the perf
sprint story (T2/T4 negatives, the ≥1.9x-after-full-ablation result, the
nginx-implication). Honest, receipts-driven, no hype. Plus
`docs/posts/show-hn-playground.md` — a short Show HN / r/rust launch blurb for
the five-versions-live playground. Drafts only; the user posts.

### T3 — Release dry-run report
Make the morning `git tag v0.1.0` one clean command. Verify the RELEASING.md
path end-to-end WITHOUT publishing: `cargo publish --dry-run` across the
publish set in dependency order (leaves first), surface any packaging error
(missing license file, path-dep leakage, oversized include, missing
description — should be clean post-#171). Confirm the npm package builds and
`npm publish --dry-run` is clean. Confirm the deprecation-pointer plan for the
old `lua-rs-runtime`/`lua-cli` names. Write findings + the exact ordered
publish command list to `docs/RELEASE_DRYRUN.md`. Read-only re: the registry.

### T4 — Perf: concat string churn
Per `docs/GC_ALLOC_DESIGN_MEMO.md` R2: concat_chain allocates 13.9M blocks/run
(~38 B avg) vs C's one TString per result. Recon the concat path (OP_CONCAT in
lua-vm, string build/intern in lua-stdlib + lua-types), find the per-concat
allocation count, reduce it (build into a reused buffer, intern once) without
changing semantics. Arbiter: Ir DOWN on concat_chain + heap-diff blocks DOWN;
controls (fibonacci, string_ops) no regression; canaries + quarantine green.
Drop-if-neutral.

### T5a — Repr rung 1: tag layout
Reorder/renumber `LuaValue`'s enum discriminants (lua-types/value.rs) so the
collectable variants form a contiguous range / share a bit, collapsing the
multi-compare `is_collectable()` (and adjacent tag tests) toward C's single
bit-test. Safe Rust, explicit discriminants. Arbiter: Ir + branch-sim DOWN on
the rows T2/T4 fingered (setter family, method_calls); no control regression;
full oracle + canary battery (this touches the value representation — treat as
correctness-sensitive). Expected win is small (1-3% Ir); honest-negative is a
fine outcome and still documents the lever.

### T5b — Repr rung 2: unwind-error PROTOTYPE (measure only, never merge)
Convert ONE hot subsystem's error path (recommend the table get/set chain,
where T2 has exact per-write budgets) from `Result`-threading to
panic/`catch_unwind`-based propagation caught at the pcall boundary, on branch
`proto/unwind-errors-tableset` (NEVER merged). Measure the per-operation Ir
delta vs baseline to size the full conversion. Deliverable: a memo
`docs/UNWIND_ERROR_PROTOTYPE.md` with the measured Result-tax and a
go/no-go recommendation for the full arc. This is the "prototype before
committing weeks" step.

## Morning deliverables

Merged-or-ready PRs for T1–T4 + T5a (or evidence-backed negatives), the two
content drafts, the release dry-run report, the unwind-error prototype memo,
and a status summary at the top of this spec. The v0.1.0 tag waits for the user.
