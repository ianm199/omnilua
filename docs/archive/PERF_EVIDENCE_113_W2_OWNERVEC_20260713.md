# Evidence: #113 Wave 2 (owner-class vectors) — measured-negative, closed unmerged

Verdict: **NO-GO for RSS, the wave's primary metric.** PR #268 closed
without merging; branch `omnilua-dev/issue-113-wave2-ownervec` (head
`be5142ed`) retained on the remote as the record. Wave 1 (#264, grayagain
link removal) remains shipped and RSS-positive everywhere.

## What was built (and proven correct)

The full rev-3 W2 design: three intrusive owner lists + seven generational
cursor cells replaced by tombstoned owner-class vectors with cohort
counters; `pending_release` sweep-time destruction ownership; `GcHeader`
24 → 8 B on 64-bit (compile-time proven). Correctness held everywhere the
suite can see: official 44/44, GC canaries 36/36 × both modes, quarantine
clean, workspace clean, zero new unsafe (net −2), one codex round with the
Critical fix landed, and six negatively-verified guards documenting the
load-bearing mechanisms (releasing-window inertness, per-object release
accounting, compaction bound, sweep recolor, backing-charge sync, drain
ordering). Codex fixes 3–7 were triaged but unstarted at the kill —
notably the recursive-`drop_all` stack-overflow hazard remains live ON THE
ARCHIVED BRANCH ONLY; anyone resurrecting it starts there.

## The numbers that closed it

RSS, maxrss best/interleaved ×5, sha-verified distinct release binaries,
after honestly charging owner-vector backing (16 B/slot) to the pacer:

| workload | main (A) | W2 (B) | verdict |
|---|---|---|---|
| binarytrees | 41.4–44.3 MB | 45.8–50.1 MB | **+10–18% WORSE, 5/5 pairs** |
| closure_ops | ~25.0 MB | ~22.7 MB | −8% better (bucket crossing) |
| table_ops_long | ~8.06 MB | ~8.40 MB | ~+4% worse |

Ir (deterministic cachegrind, rs rows, sha-stamped): binarytrees −2.74%
(faster), closure_ops −0.08%, mandelbrot control +0.10%.

## The mechanism (R2 review objection, confirmed empirically)

On 64-bit, an owner-vector slot for a `dyn Trace` box is a **fat pointer:
16 B — exactly the 16 B the header diet removed**. The design relocates
the link; it does not delete it. On top of parity comes strictly-positive
overhead: tombstone slack (≤ 25% density bound), compaction headroom
(shrink keeps 1.5× len), and peak coexistence during release drains.
High-churn workloads (binarytrees) pay all of it; closure_ops still wins
only because `GcBox<UpVal>` 56 → 40 B crosses a malloc size class — a
mechanism the vector spine gets no credit for (Wave 1's box shrink crossed
the same boundary class).

Secondary finding: shrinking *charged* box bytes while adding *uncharged*
side-structure memory shifted collection cadence (more live objects
admitted per cycle). Charging the backing (commit `7cabd12c`) corrected
cadence but cannot fix the relocation arithmetic.

## What is banked

- The 8 B header layout and the full owner-vector implementation, on the
  retained branch, with `owner_capacity_bytes()` / 
  `owner_backing_charged_bytes()` diagnostics for any future attribution.
- −2.74% Ir on binarytrees from linear slot scans — evidence that sweep
  locality is worth having if a thin-slot design ever exists (wasm32 slots
  are thin; the arithmetic differs there).
- Three named principles added to `docs/PERFORMANCE_PRINCIPLES.md`
  ("Patterns from the owner-vector negative").
- The next credible #113 lever: **allocator size-class analysis** — both
  RSS wins of this arc (UpVal diet, closure_ops here) came from bucket
  crossings, not from raw byte counts. Target boundaries deliberately.
