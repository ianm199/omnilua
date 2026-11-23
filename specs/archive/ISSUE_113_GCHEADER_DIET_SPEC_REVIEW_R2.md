# Issue #113 spec — adversarial review round 2

Reviewer: OpenAI Codex (gpt-5.6-sol, xhigh), read-only, 2026-07-12, on spec rev-2.
Verdict: REVISE — W1/W3-deletion/pointer-width/measurement plan materially improved; W2 needs one more pass (destruction ownership + tombstone policy).

Rev‑2 is substantially more honest than rev‑1, and Wave 1 is credible. Wave 2 is still not implementation-ready. The two-phase release design has a real correctness hole, and the tombstone design lacks the invariants needed to remain bounded.

## Blocking findings

1. The two-phase scan/release is not payload-`Drop` reentrancy-safe.

Rev‑2 tombstones and fully accounts an entire dead batch before releasing its first element ([spec:328](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:328>)). Releasing outside the `RefCell` borrow prevents a borrow panic, but that is only one part of reentrancy safety.

Consider dead objects A and B:

1. Scan tombstones both, refunds both sizes, removes both tokens, and decrements both object counts.
2. `Drop` for A runs.
3. A holds `Gc<B>` and a `Weak<Heap>`, upgrades the heap, and calls `B.account_buffer(heap, n)`.
4. `account_buffer` still succeeds because `HDR_COLLECTED` remains set ([heap.rs:997](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/crates/lua-gc/src/heap.rs:997>)).
5. B’s header and heap bytes increase, but B has already received its sweep refund and its later `release_box` performs no second refund.

The pacer is now permanently wrong. Current sweep avoids this particular batch window: it unlinks/accounts and immediately releases each object before inspecting the next ([heap.rs:2711](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/crates/lua-gc/src/heap.rs:2711>)).

More severe variants exist:

- A’s destructor starts a nested collection with B in its root set. B is absent from every owner vector, can be traced, and is then freed by the outer dead-list.
- A’s destructor calls public `drop_all`.
- A’s destructor panics, leaking every remaining tombstoned raw pointer in the scratch list.

Therefore the exactly-one-owner invariant is false during release: dead pointers are in none of the five claimed structures ([spec:435](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:435>)).

Rev‑3 needs an explicit in-flight ownership state plus panic safety, and a collector-reentrancy policy. A defensible design would at least:

- prohibit/defer nested collection and teardown while destruction is active;
- permit allocation only under a precisely stated rule;
- either release one scanned object at a time or prevent already-accounted peers from accepting `account_buffer`;
- own pending dead pointers through an unwind-safe drain guard.

2. Tombstone growth is unbounded below the pacer’s visibility.

All three owner moves are public and can cycle one object:

`allgc → finobj → tobefnz → allgc`

Under W2, every round creates three tombstones and three tail appends. During `Pause`, there is no mandatory compaction—only an optional future `start_cycle` compaction above an unspecified threshold ([spec:295](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:295>)). Because owner capacity is explicitly excluded from pacer bytes, this growth need never trigger a collection.

Consequences include unbounded capacity with one live object, increasingly expensive linear membership searches, and eventual O(churn²) behavior. The claim that a move remains “the same O(n) as today” is misleading: current `n` is live chain length; W2’s `n` is historical uncompacted slot count.

There are also specification-level omissions:

- `tobefnz` is declared as a plain `Vec<Option<_>>` with no tombstone counter ([spec:268](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:268>)), yet the mutation rule says its removals increment `tombstones`.
- General compaction says compact the entire vector and recount boundaries; minor sweep instead says compact only the scanned slice ([spec:422](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:422>)). Those are different algorithms.
- While tombstones exist, `reallyold`, `old1`, and `survival` must be defined as physical boundary indices, not live counts. That invariant is unstated.
- `promote_all_to_old → reallyold = live_len` is wrong when holes precede a live tail unless promotion first performs whole-vector compaction.
- `Vec::retain` removes scan holes but does not reduce capacity, so it does not address retained-memory high-water.

A concrete density threshold, mandatory no-cursor compaction/reuse policy, and adversarial repeated-move test are required.

3. The `FinalizerRegistry` precedent does not transfer as claimed.

The registry supplies a useful precedent only for dense cohort rotation. It:

- has a dense `Vec`;
- immediately rebuilds it during removal ([heap.rs:646](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/crates/lua-gc/src/heap.rs:646>));
- has no tombstones, incremental cursor, fixed watermark, scratch dead ownership, or destructor reentrancy;
- tracks cohorts only for `pending`, not `to_be_finalized`.

Its `finish_minor_collection` arithmetic ([heap.rs:713](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/crates/lua-gc/src/heap.rs:713>)) can inform W2 after compaction, but it is not an “exact cohort-prefix-counter scheme” precedent for the hard parts of OwnerVec. The claimed low novelty should be withdrawn.

4. The grayagain argument contradicts W1 and the code.

W1 correctly specifies that every cross-list move deletes grayagain membership ([spec:187](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:187>)). Current `unlink_from_list` always calls `correct_generation_pointers`, whose final duty is `unlink_grayagain` ([heap.rs:1918](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/crates/lua-gc/src/heap.rs:1918>)).

W2 nevertheless justifies deleting `firstold1` by asserting a moved `Old1` object retains a grayagain entry ([spec:371](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:371>)). It does not.

`firstold1` is indeed currently unread by collector decisions, so its mechanical deletion may still be valid. But the stated proof is invalid. G3 should use a graph-shaped case: a moved transitional object with young descendants reachable through an exact-`Old` parent, not merely “next minor is sane.”

5. The cursor interaction table incorrectly generalizes recoloring.

`move_finobj_to_tobefnz` deliberately does not recolor, both today ([heap.rs:2018](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/crates/lua-gc/src/heap.rs:2018>)) and in rev‑2. Yet the ahead-of-cursor row says “the `is_sweep()` recolor” protects every move. Its safety instead depends on the object having been marked before promotion and, when applicable, being swept later in `tobefnz`.

The table and M1–M3 tests need separate expectations for all three transitions.

6. Teardown repeats the release mistake.

Taking each vector and then dropping its contents avoids a live borrow, but a destructor that allocates into a vector already taken by `drop_all` leaves a new allocation behind; the function later zeros accounting without revisiting that vector. Current `drop_all` is public ([heap.rs:2999](</Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113spec/crates/lua-gc/src/heap.rs:2999>)), so “weak upgrade fails during `Heap::drop`” is insufficient. Teardown needs an explicit no-reentrancy guard or drain-until-stable behavior.

## Disposition audit

| R1 item | Assessment |
|---|---|
| 1, 2, 8, 11, RSS | Honestly incorporated |
| 3 | Mostly incorporated; move/recolor table still overgeneralizes |
| 4 | Strategy selected, but compaction and density rules remain incomplete |
| 5 | Not incorporated beyond avoiding a `RefCell` borrow panic |
| 6 | Header age is correctly authoritative; precedent and grayagain proof are overstated |
| 7 | Typed registry as FIFO authority is credible; synchronization should assert move success |
| 9 | Exclusion from pacer bytes is an honest choice; the fallback slack charge remains underspecified |
| 10 | Not incorporated during scratch release or reentrant teardown |

Wave 1, deletion of Wave 3, pointer-width accounting, and the measurement plan are all materially improved. Wave 2 needs another revision centered on destruction ownership/reentrancy and a bounded tombstone policy.

VERDICT: REVISE