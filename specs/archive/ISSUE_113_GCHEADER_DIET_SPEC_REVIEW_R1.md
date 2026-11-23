# Issue #113 spec — adversarial review round 1

Reviewer: OpenAI Codex (gpt-5.6-sol, xhigh), read-only, 2026-07-12.
Verdict: REVISE. Spec rev-2 must incorporate these before execution.

## Review

The core idea is feasible without adding thin-pointer/vtable-recovery unsafe, but the specification is not implementation-ready. Wave 1 is plausible. Wave 2’s mutation, ordering, and accounting invariants are incorrect or unspecified, and Wave 3 is unnecessary.

### Blocking findings

1. **Wave 2 relocates a fat pointer; it does not eliminate its memory cost.**

Every live heterogeneous object still needs one `NonNull<GcBox<dyn Trace>>` somewhere so the heap can trace and destroy it. The current invariant requires every live box to be reachable exactly once ([heap.rs:25](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:25)). An owner `Vec` therefore consumes 16 bytes per live object on 64-bit targets, plus spare capacity.

Consequences:

- W1 can save approximately 16 bytes per ordinary object, with a 16-byte external slot only for gray-listed objects.
- W2 shrinks the box by another 16 bytes but adds a permanent 16-byte owner-vector slot. Its direct total-memory saving is approximately zero before allocator-size-class effects.
- The resulting 8-byte `GcHeader` is real, but total safe type-erased ownership metadata is at least 24 bytes per object on 64-bit: 8-byte header plus 16-byte external fat pointer.
- `GcBox<UpVal> == 40` would describe only the box allocation, not its total ownership footprint.

The RSS projection’s “32 bytes per object direct” therefore double-counts W2’s relocated pointer.

2. **Wave 3 does not reduce the header at all and repeats a measured regression.**

After deleting both links, the existing fields already occupy eight bytes:

```text
Color: 1
GcAge: 1
flags: 1
padding: 1
u32 size: 4
```

Packing them into `Cell<u16> + Cell<u32>` is also eight bytes. Worse, the source explicitly records that packing hot fields caused about +4% retired instructions ([heap.rs:729](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:729)). Wave 3 should be deleted. If layout stability is wanted, add `#[repr(u8)]` to `Color` and `GcAge` and assert the eight-byte header on applicable targets.

3. **“Only sweep removes during sweep” is false.**

The current implementation explicitly supports arbitrary owner-list removal while an incremental sweep is paused. `unlink_from_list` detects removal of the cell holding the active cursor and rewrites that cursor ([heap.rs:1940](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1940)).

All three ownership transitions remove from a source list, and two explicitly handle sweep-phase recoloring:

- allgc → finobj: [heap.rs:1994](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1994)
- finobj → tobefnz: [heap.rs:2009](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2009)
- tobefnz → allgc: [heap.rs:2017](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2017)

A segment move also necessarily removes from its source unless the design permits duplicate ownership or tombstones. “Moves append, never remove” is incompatible with unique membership.

With `swap_remove`, if position `j < cursor`, the old tail moves into an already-processed prefix and is skipped. Appending to a segment currently being swept can also cause a second visit; appending to an already-swept segment defers the object until the next cycle. The spec defines none of these cases.

4. **`retain` and an incremental cursor are not a complete algorithm.**

Full sweep is resumable across calls and owner classes ([heap.rs:2504](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2504), [heap.rs:2676](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2676)). A normal `Vec::retain` is an all-at-once operation. An incremental stable-compaction algorithm needs read/write cursors and creates duplicates or holes until compaction completes, complicating every concurrent move and membership search.

The spec needs one explicit strategy, such as:

- `Vec<Option<FatPtr>>` stable slots with tombstones, fixed phase-end watermarks, and compaction only after a segment finishes;
- stable ordered `remove` plus precise cursor adjustment, accepting potentially quadratic deletion;
- deferred move queues applied at a sweep-safe boundary.

It must also preserve the existing rule that allocations during an incremental sweep survive the current cycle, already covered by [heap.rs:3507](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:3507).

5. **The spec identifies the wrong mid-minor interleaving hazard.**

Young sweep is currently stop-the-world and completes in one call ([heap.rs:2340](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2340)). There is no mutator resumption halfway through `sweep_young`.

The relevant hazards are:

- full incremental sweep across calls;
- ownership moves performed while the heap reports a sweep phase;
- allocation between incremental steps;
- possible reentrancy from dropping a dead Rust payload during `release_box`.

A `RefCell<Vec<_>>` borrow must not remain held while `release_box` drops the box. A `retain` closure that frees objects under the vector’s mutable borrow risks a reentrant `RefCell` panic. Removal and destruction should be two-phase.

6. **Three age segments do not replace seven age states.**

`GcAge` has `New`, `Survival`, `Old0`, `Old1`, `Old`, `Touched1`, and `Touched2` ([heap.rs:253](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:253)). Barriers create `Old0` and `Touched1` directly ([heap.rs:2233](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2233)), while minor collection advances those transitional states differently.

Therefore either:

- age remains authoritative in the header and the three vectors are only coarse cohorts, contradicting “age promotion becomes segment membership”; or
- the design needs more segments or remembered sets for transitional ages.

This matters because the minor marker skips only exact `Old`, not every “oldish” age ([heap.rs:1234](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1234)).

Also, the finalizer registry already maintains independent cohort boundaries and advances them after minor collections ([heap.rs:506](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:506), [heap.rs:706](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:706)). Wave 2 does not delete cohort arithmetic system-wide; it creates another representation that must stay synchronized.

7. **Order cannot be dismissed and `swap_remove` is unsuitable for ordered lists.**

Allocation currently prepends, so traversal and sweeping are newest-first ([heap.rs:1668](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1668)). A push-based vector naturally stores oldest-to-newest. Matching the current order requires reverse traversal and carefully defined promotion ordering.

More importantly:

- allgc → finobj inserts at the head;
- finobj → tobefnz appends at the tail ([heap.rs:1977](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1977));
- finalizers are consumed FIFO through `remove(0)` ([heap.rs:714](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:714)).

`swap_remove` destroys relative order. The runtime’s typed `FinalizerRegistry` may remain the semantic FIFO authority, but the spec must say so and prove synchronization with the heap’s `tobefnz` membership vector. Otherwise use stable removal or `VecDeque`.

The existing sweep-phase recoloring in ownership moves is also load-bearing and absent from the proposed transition rules.

8. **W1 is viable, but it is not “four functions” or cleared every minor cycle.**

Grayagain entries can persist across multiple minors:

- `Old1` persists until `Old`: [heap.rs:3790](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:3790)
- `Touched2` persists until `Old`: [heap.rs:3810](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:3810)
- young sweep replaces grayagain with a next-cycle revisit set rather than simply clearing it: [heap.rs:2818](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2818), [heap.rs:2910](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2910)

Every freed gray-listed object must be removed before its box is released ([heap.rs:1909](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1909)). W1 therefore touches header construction, heap initialization, every grayagain operation, full-sweep deletion, minor replacement, teardown, and tests. It remains a reasonable independent packet, but “LOW risk/~4 functions” understates its surface.

9. **Pacer accounting becomes dishonest unless vector capacity is addressed.**

Allocation charges exactly `size_of::<GcBox<T>>()` ([heap.rs:1675](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1675)), per-object buffer growth updates that amount ([heap.rs:981](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:981)), and sweep refunds the stored size ([heap.rs:2702](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2702)). The post-cycle threshold derives from the remaining byte count ([heap.rs:2921](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2921)).

After W2, that counter drops by 16 bytes per object even though a 16-byte owner slot now exists elsewhere. Spare vector capacity persists after objects are swept. The spec must either:

- account vector-capacity deltas separately in `bytes`; or
- explicitly redefine pacer bytes to exclude ownership storage and measure the cadence consequences.

This is especially important for issue #113: the representation change itself alters collection thresholds.

10. **Quarantine and uncollected semantics require more than “push onto a Vec.”**

Uncollected allocations are heap-owned but deliberately excluded from sweeping, `bytes`, `objects`, and normal header enumeration ([heap.rs:1697](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1697), [heap.rs:1853](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1853)). Quarantined objects have already had byte, token, and object accounting removed before they are parked ([heap.rs:1491](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1491), [heap.rs:1863](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:1863)).

`HDR_COLLECTED` and `HDR_HEAP_OWNED` cannot identify a particular vector or age segment, and `HDR_COLLECTED` is not cleared when a quarantined box leaves the sweepable lists. They are insufficient as membership metadata.

The revised design needs an explicit unique-ownership invariant covering:

- all normal and finobj age vectors;
- tobefnz;
- quarantine;
- uncollected;
- grayagain as non-owning;
- teardown without duplicates or double frees.

It should also clear grayagain flags before boxes are destroyed, matching the present `drop_all` ordering ([heap.rs:2990](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-issues/crates/lua-gc/src/heap.rs:2990)).

11. **The 64-bit RSS claims do not generalize to wasm32.**

The 16-byte link sizes are 64-bit trait-object sizes. On wasm32 each fat link is normally eight bytes, making the current header approximately 24 rather than 40 bytes and the two-link saving 16 rather than 32. `GcBox<UpVal> 72→40` is likewise a native-layout claim.

Wasm linear-memory allocators also tend to retain grown pages, making owner-vector capacity and reallocations more important than native “box size” alone. The revised spec needs pointer-width-qualified layout assertions and a wasm memory/high-water check, not only `cargo check`.

### RSS projection

The proposed `2.96× → 2.2×` and `2.22× → 1.9×` figures are not supported by the stated arithmetic:

- the W2 pointer is relocated, not removed;
- vector excess capacity is omitted;
- allocator buckets are unknown;
- no absolute C and Rust peak-RSS values are shown for converting saved bytes into a ratio;
- prior evidence showed a genuine 3.2 MB payload reduction produced roughly a 3.4 MB process-RSS reduction, not the much larger ratio movement implied here.

Treat the projection as an unquantified hypothesis until `heap-diff` reports both box allocations and owner-vector backing allocations.

### Recommended revision

- Keep W1 as a separate measured experiment, but specify persistent grayagain ordering, deletion, buffer reuse, and teardown.
- Delete W3 entirely.
- Redesign W2 around stable slots/tombstones or deferred moves, with fixed sweep watermarks and an explicit phase-order table.
- Consider owner-class vectors that retain header age plus deferred cohort maintenance; this achieves the same 8-byte box header without barrier-driven physical moves among age vectors.
- Add targeted tests for every owner move before/at/after each active sweep cursor, allocation during every sweep phase, `StepBudget::from_work(1)` resumption, FIFO finalization, grayagain deletion, quarantine, uncollected teardown, and newest-first order.
- Do not claim W2’s 16-byte box reduction as direct RSS savings; require allocator-bucket or locality evidence before accepting the architectural complexity.

VERDICT: REVISE