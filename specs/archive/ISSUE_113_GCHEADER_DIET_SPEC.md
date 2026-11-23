# Issue #113 — GcHeader diet without unsafe: kill both intrusive fat links (rev 3)

Status: SPEC rev 3 (not started). Rev-2 answered the round-1 adversarial
review (`ISSUE_113_GCHEADER_DIET_SPEC_REVIEW_R1.md`); the round-2 review
(`ISSUE_113_GCHEADER_DIET_SPEC_REVIEW_R2.md`, VERDICT: REVISE) judged W1,
the W3 deletion, the pointer-width accounting, and the measurement plan
materially improved and confined its findings to W2. Rev-3 is that W2 pass:
(1) destruction ownership — dead boxes are owned by a heap-side
`pending_release` structure with per-object accounting at release, a
`releasing` window that makes collection entry points inert, and a
drain-until-stable `drop_all`; (2) a bounded tombstone policy (25% density,
move-time trigger outside sweep, one compaction algorithm, physical
boundary indices, capacity shrink); (3) the `FinalizerRegistry` precedent
claim withdrawn; (4) the `firstold1` deletion restated on
unread-by-decisions grounds with a graph-shaped G3 test; (5) per-transition
move/recolor semantics (`move_finobj_to_tobefnz` does not recolor).

Rev-1's core claim stands: both fat links can be removed in safe Rust.
Rev-1's W2 design (per-age segment vectors) did not survive R1 and was
replaced in rev-2 by owner-class vectors plus deferred cohort maintenance;
Wave 3 is dropped; RSS projections are replaced by a measurement plan.

## Review R1 disposition table

| # | Finding (one line) | Disposition |
|---|---|---|
| 1 | W2 relocates the fat pointer (owner-vector slot), does not eliminate it | INCORPORATED — "Where the bytes go today": relocation arithmetic stated (≥ 24 B / ≥ 16 B total ownership metadata per object), direct W2 saving called ≈ 0 ± slack, W2's case re-founded on size-class crossings + sweep locality + cursor-machinery deletion, all measurement-gated |
| 2 | W3 saves zero bytes and repeats a measured +4% Ir packing regression | INCORPORATED — "Wave 3 — deleted": deleted outright (not deferred); replaced by `#[repr(u8)]` on `Color`/`GcAge` + an 8-byte `GcHeader` size assert |
| 3 | "Only sweep removes during sweep" is false; `unlink_from_list` rewrites the live cursor | INCORPORATED — W2 "Design decision" pt 3 + "The one mutation rule": tombstones make indices stable so the cursor-rewrite apparatus is deleted rather than translated; the sweep-phase `current_white()` recolor in `move_allgc_to_finobj`/`move_tobefnz_to_allgc` is carried over verbatim; full before/at/ahead/beyond-watermark case table given |
| 4 | `retain` + incremental cursor is not a complete algorithm | INCORPORATED — "Full incremental sweep": one explicit strategy chosen from R1's menu (stable slots + tombstones + fixed watermark, compaction only at named sweep-complete points); `Vec::retain` confined to compaction points; mid-sweep-allocation survival rule preserved by watermark + white color |
| 5 | Wrong mid-minor hazard named; young sweep is STW, real hazards are full-sweep pauses + `release_box` reentrancy under RefCell | INCORPORATED — W2 "Minor collection" confirms `sweep_young` stays STW single-call; hazards restated as budgeted-sweep interleavings; two-phase scan/release rule with the stated invariant that no owner-vec borrow is ever held across `release_box` |
| 6 | Three segments do not replace seven `GcAge` states | INCORPORATED — W2 "Design decision" pt 2: header age stays authoritative, vectors are coarse position cohorts bounding the young-sweep scan; counter updates at compaction points model `finish_minor_collection`'s arithmetic (the broader registry-precedent claim is withdrawn per R2 finding 3); `Marker::should_trace_age`'s exact-`Old` skip untouched |
| 7 | Order is load-bearing (newest-first sweep, tobefnz FIFO, head-vs-tail inserts); `swap_remove` unsuitable | INCORPORATED — "tobefnz FIFO": `FinalizerRegistry` named as the semantic FIFO authority with the entry/exit synchronization argument; head-prepend maps to tail-append = nursery cohort; `swap_remove` absent from the design; intra-cycle free order documented as non-contractual with the battery as falsifier (test row O1) |
| 8 | W1 is bigger than "4 functions"; grayagain persists across minors and has deletion paths | INCORPORATED — W1 "What exists today": both rev-1 claims corrected against code ("cleared each cycle" is false — `replace_grayagain` persists `Old1`/`Touched2` until `Old`; full touchpoint list given), risk re-rated MEDIUM-LOW, all deletion cases enumerated incl. the `correct_generation_pointers` → `unlink_grayagain` hook and cross-list moves (new test G3) |
| 9 | Pacer accounting must address owner-vector capacity | INCORPORATED — W2 "Pacer accounting": R1's option (b) chosen — pacer bytes explicitly redefined to exclude ownership storage, with rationale; cadence deltas measured (`collections()`/`minor_collections()`), `owner_capacity_bytes()` diagnostic feeds heap-diff, slack-charge fallback pre-declared |
| 10 | Quarantine/uncollected need explicit unique-ownership invariants; flags can't identify a vector | INCORPORATED — W2 "Unique-ownership invariant": exactly-one-slot invariant across the five structures, membership never inferred from flags, `HDR_COLLECTED`-not-cleared-when-quarantined documented, grayagain declared non-owning ⊆ live sweepable set, `drop_all` ordering (grayagain cleared first, `mem::take` before frees) |
| 11 | 64-bit layout claims do not generalize to wasm32 | INCORPORATED — "Where the bytes go today" dual-width table (40/24 B header, 16/8 B links); measurement plan item 6 adds a wasm linear-memory high-water measurement beyond `cargo check`; post-diet 8-B assert valid ungated on both widths |
| R | RSS projection (2.96→2.2 etc.) unsupported by the stated arithmetic | INCORPORATED — projections withdrawn; "Measurement plan" states done-conditions as measurements to take (Ir arbiter, heap-diff incl. ownership storage, absolute peak-RSS triples, cadence, wasm high-water) with the drop-if-neutral decision rule, no target numbers |

No finding is rebutted. Finding 1 carries one nuance rather than a
disagreement: "direct saving ≈ 0" is accepted as stated, and the spec adds
that allocator size-class effects can still make the relocation a real RSS
win — which is R1's own closing recommendation ("require allocator-bucket
or locality evidence"), here promoted to the W2 keep/revert criterion.

## Where the bytes go today (pointer-width qualified)

`crates/lua-gc/src/heap.rs`, `GcHeader` (`#[repr(C)]`). A
`NonNull<GcBox<dyn Trace>>` is a fat pointer: 16 B on 64-bit targets, 8 B on
wasm32 (4 B data + 4 B vtable). `Option<NonNull<...>>` is the same size via
the null-pointer niche. Every layout figure in this spec is therefore stated
for both widths.

| field | 64-bit | wasm32 | role |
|---|---|---|---|
| `color: Cell<Color>` | 1 | 1 | tri-color mark state |
| `age: Cell<GcAge>` | 1 | 1 | generational age (seven states, authoritative — see W2) |
| `flags: Cell<u8>` | 1 | 1 | finalized / collected / gray_listed / freed / heap_owned |
| padding | 1 | 1 | |
| `size: Cell<u32>` | 4 | 4 | pacer bytes sweep will refund |
| `next: Cell<Option<NonNull<GcBox<dyn Trace>>>>` | 16 | 8 | intrusive owner-list link (head/finobj/tobefnz/quarantined/uncollected) |
| `gray_next: Cell<Option<NonNull<GcBox<dyn Trace>>>>` | 16 | 8 | grayagain revisit link |
| **total** | **40** | **24** | |

Removing both links leaves `color + age + flags + pad + size` = **8 B on both
widths** (alignment 4 from the `u32`). Rev-1's "40 → 8" and "`GcBox<UpVal>`
72 → 40" were 64-bit-only claims; on wasm32 the header goes 24 → 8 and each
removed link saves 8 B, not 16.

The honest accounting (R1 finding 1): removing a link from the header does
not always remove its bytes from the process. The owner-list link is
*relocated* — every live heap-owned object still needs exactly one
`NonNull<GcBox<dyn Trace>>` somewhere so the heap can trace, sweep, and
destroy it (safety-model invariant 2 in the heap.rs module docs). A W2 owner
vector holds that pointer at 16 B/slot (64-bit) or 8 B/slot (wasm32), plus
`Vec` excess capacity and tombstone slack. So:

- **W1 removes bytes outright**: `gray_next` dies in every object; the
  replacement Vec slot exists only while an object is on the revisit list
  (transient, bounded by the barrier-touched set).
- **W2 relocates bytes**: box shrinks by one link, owner vector gains one
  slot. Direct saving ≈ 0 ± capacity slack *before* allocator size-class
  effects. What W2 actually buys: (a) size-class crossings — e.g. on 64-bit
  `GcBox<UpVal>` 72 → 40 B moves macOS malloc buckets 80 → 48; (b) linear
  sweep over a dense pointer array instead of a pointer chase; (c) deletion
  of the seven generational cursor cells and the `unlink_from_list` /
  `correct_generation_pointers` cursor-patch machinery (the code region where
  #113's original pacing bug lived). All three are hypotheses to measure, not
  savings to assert — see the measurement plan.
- Total type-erased ownership metadata per live object after W1+W2 is 8 B
  header + one external fat slot: ≥ 24 B on 64-bit, ≥ 16 B on wasm32.
  "`GcBox<UpVal>` = 40 B" describes the box allocation, not the ownership
  footprint.

Vector slots hold pointers to `Box` allocations; a `Vec` reallocation moves
slots, never boxes, so no `Gc<T>` or `Marker::gray_queue` entry is ever
invalidated by owner-vector growth.

## Wave 1 — grayagain becomes a heap-owned Vec (lands alone, measured)

### What exists today (ground truth)

`gray_next` + `HDR_GRAY_LISTED` implement one list, `Heap::grayagain`, with
these operations:

- `remember_minor_revisit` — prepend with flag-based dedup. Callers:
  `generational_forward_barrier` (child aged `Old0`),
  `generational_backward_barrier` (parent aged `Touched1`), and
  `replace_grayagain`.
- `mark_minor_revisit_objects` — walked at the start of every minor mark
  (`minor_collect_with_post_mark`, before roots are traced).
- `take_grayagain` — drains to a `Vec`, clearing links and flags; used by
  `sweep_young` (the drained set feeds `OldRevisitTracker`) and by
  `unlink_grayagain`.
- `replace_grayagain` — `clear_grayagain` then re-prepend a new set in
  reverse, preserving the vec's order; called at the end of `sweep_young`
  with the `next_revisit` set.
- `clear_grayagain` — clears links + flags; called from
  `clear_generation_cursors` (so from `reset_all_ages` and `drop_all`) and
  `set_all_cursors_to_head` (from `promote_all_to_old`).
- `unlink_grayagain` — **the deletion path R1 flagged**: called from
  `correct_generation_pointers` whenever an object with `HDR_GRAY_LISTED`
  set is unlinked from an owner list for *any* reason — a full sweep freeing
  it (`sweep_budgeted` → `correct_generation_pointers`, pinned by the
  `full_sweep_unlinks_freed_grayagain_entries` test), a young sweep freeing
  it, or a cross-list move (`move_allgc_to_finobj` etc. via
  `unlink_from_list`). Implemented as take-filter-replace.
- `grayagain_count` — diagnostic (read by lua-cli telemetry).

Two rev-1 claims were wrong against this code (R1 finding 8, confirmed):
the list is **not** cleared each minor cycle — `sweep_young` *replaces* it
with the next cycle's revisit set via `push_next_revisit`/`replace_grayagain`,
and entries persist across minors until their object reaches `Old`
(`grayagain_list_carries_old1_until_old` and
`grayagain_list_carries_touched2_until_old` pin `Old1` and `Touched2`
persistence). And the surface is larger than "~4 functions": header
construction (`GcHeader::new_white`), `Heap::new`, all seven operations
above, `correct_generation_pointers`, `sweep_young`, `drop_all` ordering,
and the tests. Risk is re-rated **MEDIUM-LOW**: still a self-contained
packet, but it touches every sweep-free path via the deletion hook.

### The replacement

```rust
grayagain: RefCell<Vec<NonNull<GcBox<dyn Trace>>>>,
```

`gray_next` is deleted from `GcHeader`. `HDR_GRAY_LISTED` stays and keeps
exactly its current meaning: "this object has an entry in grayagain" — it is
the dedup check in `remember_minor_revisit` and the cheap guard that lets
`correct_generation_pointers` skip the deletion scan for the common
(non-listed) case. The Vec is **non-owning**: membership invariant is
`grayagain ⊆ live sweepable boxes` (every entry's object is currently in
allgc/finobj/tobefnz and not `HDR_FREED`), maintained by the deletion hook
below.

Operation mapping:

- `remember_minor_revisit`: flag check, then `borrow_mut().push(ptr)` + set
  flag. Push order replaces prepend order — see ordering note below.
- `mark_minor_revisit_objects`: iterate a short borrow, calling
  `Marker::mark_box` per entry. Marking is idempotent and deduped by
  `Marker::visited`, so iteration order affects only the order gray-queue
  entries are pushed, not the reachability fixed point.
- `take_grayagain`: `std::mem::take(&mut *borrow_mut())`, then clear each
  entry's flag *after* the borrow is released. Returns the Vec;
  `sweep_young`'s `OldRevisitTracker` consumes it unchanged.
- `replace_grayagain(objects)`: clear flags of any current entries, then
  install `objects` as the new buffer, setting flags. To reuse capacity
  across cycles, `sweep_young` builds `next_revisit` in the buffer returned
  by `take_grayagain` after draining it — one persistent buffer pair, the
  same recycling pattern as `marker_pool`.
- `clear_grayagain`: clear flags, `vec.clear()` (capacity retained).
- `unlink_grayagain(removed)`: `retain(|p| !addr_eq(p, removed))` + clear
  the removed object's flag. Same take-filter semantics as today, no longer
  needs the full relink.
- `grayagain_count`: `borrow().len()` — flag-dedup guarantees no duplicates,
  so len equals today's walk count (`grayagain_links_object_once` pins it).

**Ordering.** Today's intrusive list iterates newest-remembered-first
(prepend order); the Vec iterates oldest-first. The two consumers are
`mark_minor_revisit_objects` (order-insensitive per above) and
`sweep_young`'s revisit loop over the drained set, which processes each
entry independently (`was_processed` filtering by identity, age advance,
`push_next_revisit`) — no entry's handling reads another's. No behavior
depends on intra-list order; the canary battery ×2 modes plus
gc.lua/gengc.lua are the falsifiers, per the test matrix.

**Deletion cases** (each must clear the flag and remove the entry before the
box can be freed):

1. Full sweep frees a gray-listed object: `sweep_budgeted` →
   `correct_generation_pointers` → `unlink_grayagain`, *before*
   `release_box`. Unchanged call order; the Vec op must not hold its borrow
   into `release_box` (it doesn't — the retain completes first).
2. Young sweep frees a gray-listed object: entries were already drained by
   `take_grayagain` at `sweep_young` entry, flags cleared, so
   `correct_generation_pointers` sees `gray_listed() == false`. Unchanged.
3. Cross-list move of a gray-listed object (e.g. `move_allgc_to_finobj` on a
   `Touched1` table given a `__gc` metatable): under the vector design the
   move no longer unlinks `next`, but it must still *keep* the grayagain
   entry — the object remains live and still needs its revisit. Today's code
   **deletes** the entry on move (via `unlink_from_list` →
   `correct_generation_pointers`) and the object re-enters grayagain only if
   a barrier fires again. W1 must preserve today's observable behavior
   exactly: the move deletes the grayagain entry. A new kit test pins this
   (test matrix, row G3).
4. Teardown: `drop_all` → `clear_generation_cursors` → `clear_grayagain`
   runs before any `drop_list` — flags and entries are gone before boxes are
   freed. The Vec version keeps this order (R1 finding 10's last point).

**Quarantine mode**: `release_box` under `LUA_RS_GC_QUARANTINE=1` parks the
box on the quarantined list with `HDR_FREED`; case 1/2 above ran first, so no
grayagain entry can point at a parked box. The quarantine debug asserts in
`Gc::as_box`/`Marker::mark_box` would catch a violation deterministically —
run the canary battery under quarantine as part of the gate.

Gates: full battery (36 GC canaries × incremental+generational, quarantine
run, strict-guard run, officials ×5, workspace tests, wasm `cargo check`) +
Ir arbiter on binarytrees/table_ops + heap-diff, per the measurement plan.
W1 ships as its own PR and its measured result gates W2.

## Wave 2 — owner-class vectors with deferred cohort maintenance

### Design decision

Rev-2 **adopts the R1 alternative** (owner-class vectors retaining header
age, deferred cohort maintenance) and **abandons rev-1's per-age segment
vectors**. Rev-1's design was wrong against the code in three ways R1
correctly identified:

1. Barriers never physically move objects today.
   `generational_forward_barrier` sets `age = Old0` and
   `generational_backward_barrier` sets `age = Touched1` *in place*, plus a
   `remember_minor_revisit` — the allgc chain order is pure allocation
   order. A design that moves objects between age vectors at barrier time
   invents a mutation the collector has never had, precisely in the
   hazard-heavy window (mid-sweep). Deferred cohort maintenance means: no
   physical move ever happens at barrier time; position changes only at
   sweep-completion points where the cursor state is known.
2. Three segments cannot represent seven `GcAge` states (`New`, `Survival`,
   `Old0`, `Old1`, `Old`, `Touched1`, `Touched2`). The minor marker's skip
   test is exact-`Old` (`Marker::should_trace_age`), barriers create `Old0`
   and `Touched1` at arbitrary positions, and `GcAge::next_after_minor`
   advances each state differently. **Header age stays authoritative.**
   Vectors are coarse *position cohorts* that bound which slice the young
   sweep scans — which is already how the cursor scheme behaves today
   (an `Old0`-aged object sits in the nursery region; `sweep_young_range`
   frees only `is_white() && !age.is_old()`, so age, not position, decides
   life or death).
3. "Only sweep removes during sweep" is false. `unlink_from_list` explicitly
   rewrites `sweep_prev_next` when the removed cell is the live cursor, and
   `move_allgc_to_finobj` / `move_tobefnz_to_allgc` recolor to
   `current_white()` when `GcState::is_sweep()` — cross-list moves during a
   paused incremental sweep are supported, load-bearing behavior.

Rev-2 claimed `FinalizerRegistry` as a precedent making this design
"low-novelty." **That claim is withdrawn** (R2 finding 3): the registry has
a dense `Vec` it rebuilds immediately on removal
(`retain_pending_not_in`), no tombstones, no incremental cursor, no
watermark, no scratch dead ownership, no destructor reentrancy, and it
cohort-tracks only `pending`, not `to_be_finalized`. None of the hard parts
of `OwnerVec` have an in-tree precedent; they are new machinery and carry
their risk undiscounted. What the registry honestly supplies is the
*post-compaction cohort arithmetic*: `finish_minor_collection`'s rotation
(`reallyold += old1; old1 = survival; survival = new`) and
`promote_all_pending_to_old`'s whole-list promotion are the model for how
W2 updates its counters at compaction points — where the vector is dense,
which is the only regime the registry ever operates in.

### Data structures

```rust
struct OwnerVec {
    slots: Vec<Option<NonNull<GcBox<dyn Trace>>>>,
    tombstones: usize,
    reallyold: usize,
    old1: usize,
    survival: usize,
}

allgc:       RefCell<OwnerVec>,
finobj:      RefCell<OwnerVec>,
tobefnz:     RefCell<OwnerVec>,
quarantined: RefCell<Vec<NonNull<GcBox<dyn Trace>>>>,
uncollected: RefCell<Vec<NonNull<GcBox<dyn Trace>>>>,
sweep_index:     Cell<usize>,
sweep_watermark: Cell<usize>,
```

All three sweepable lists are the same `OwnerVec` type so the tombstone
counter and density policy apply uniformly (R2 flagged rev-2's `tobefnz` as
a plain `Vec<Option<_>>` with no counter — an inconsistency with the
mutation rule). `tobefnz`'s cohort counters are permanently zero: the young
sweep scans all of it (today's `sweep_young_range(tobefnz, None)`), so it
needs membership and tombstones, never cohorts.

Replaced outright: `head`, `finobj`, `tobefnz`, `quarantined`, `uncollected`
head cells; the seven cursor cells (`survival`, `old1`, `reallyold`,
`firstold1`, `finobjsur`, `finobjold1`, `finobjrold`); `sweep_prev_next`;
`GcHeader::next`. (`firstold1` is written and cursor-corrected today but
never read by any sweep decision — it exists for upstream parity and one
test assert. The cohort counters make it unnecessary; its deletion is part
of this wave and gets a changelog note.)

Slot orientation: **oldest at index 0, `push` appends newest at the tail.**
This maps today's prepend-at-`head` (newest first in traversal order) to
tail-append; cohort prefix ranges from index 0 are
`[0, reallyold)` = old/reallyold, `[reallyold, reallyold+old1)` = old1,
`[.., ..+survival)` = survival, remainder = nursery. `allocate` pushes
`Some(ptr)` onto `allgc` — one amortized push replacing two `Cell` writes.
`quarantined` and `uncollected` need no tombstones or cohorts: they are
append-only during life (`release_box` under quarantine;
`allocate_uncollected`) and drained only in `drop_all`. (Rev-2 justified
`tobefnz`'s missing cohorts by a registry mirror; the correct reason —
above — is that the young sweep scans all of `tobefnz`, and
`FinalizerRegistry` cohort-tracks only `pending`, not `to_be_finalized`.)

### The one mutation rule: tombstones, never shifts — with a bound

Any removal from `allgc`/`finobj`/`tobefnz` outside a compaction point
writes `None` into the slot (a tombstone) and increments that vector's
`tombstones`. **While tombstones exist, `reallyold`/`old1`/`survival` are
physical slot-index boundaries — they count slots, tombstoned or not.** A
tombstone under a boundary does not move it; only compaction recounts
boundaries to dense indices. No slot index ever changes except at a
**compaction point**, defined as any moment with no live sweep cursor for
that vector.

There is exactly one compaction algorithm: a whole-vector, order-preserving
pass that drops `None` slots, sets `tombstones = 0`, and recounts each
cohort boundary as the number of surviving slots below its old physical
index. (Rev-2 described a second, slice-only variant for the minor sweep;
that is withdrawn — moves can tombstone old-prefix slots too, so slice-only
compaction leaves the prefix dirty and the two algorithms diverge. R2
finding 2.)

**Density policy — the bound R2 finding 2 demanded.** Per vector, the
threshold is `tombstones * 4 > slots.len()` (25% density; a named constant,
initial value tunable only by measurement). Compaction runs:

- *mandatorily* at `finish_cycle`, the end of `sweep_young`, `abort_cycle`,
  and `drop_all`, whatever the density;
- *by threshold* at `start_cycle` (no cursor exists at `Pause →
  Propagate`), and — this is the piece rev-2 lacked — **immediately after
  any tombstoning move, if the source vector crosses the threshold and
  `GcState` is not a sweep phase**. Moves are the only tombstone source
  outside a sweep, so checking there closes the churn hole.

R2's pathological case — cycling one object `allgc → finobj → tobefnz →
allgc` during `Pause`, three tombstones and three appends per round,
forever, with owner capacity invisible to the pacer — is bounded by the
move-time trigger: each vector compacts every ≥ `len/4` tombstoning ops at
a cost of O(len), i.e. amortized O(1) slots and O(4) work per move. During
a sweep phase compaction is prohibited (cursor live), so density can exceed
25% transiently; a cycle completes in bounded steps and `finish_cycle`
compacts unconditionally. Worst-case slot count is therefore
`(4/3) × live-at-cycle-start + appends-during-that-cycle`, and at every
no-cursor moment slot count ≤ `(4/3) × live`.

The rev-2 claim that a move's membership scan is "the same O(n) as today"
is corrected (R2 called it misleading): today's `n` is live chain length;
W2's `n` is slot count including tombstones — bounded to `4/3 ×` live at
no-cursor times plus intra-cycle churn by the policy above, which is the
honest statement.

**Capacity high-water**: `Vec::retain` never shrinks capacity, so
compaction alone does not reduce retained memory. Policy: at the mandatory
compaction points, if `capacity > 2 × len` after compacting, `shrink_to(len
* 3 / 2)` — bounding the retained high-water near 2× live while leaving
headroom against shrink-grow thrash. `owner_capacity_bytes()` and the wasm
linear-memory high-water measurement (plan item 6) judge whether this needs
tightening; on wasm the pages are unreturnable, so the shrink only caps
future growth there — the measured number is what matters.

Because indices are stable between compactions, the entire cursor-patch
apparatus — `unlink_from_list`'s `sweep_prev_next` rewrite and
`correct_generation_pointers`' seven cursor fixups — is deleted rather than
translated. The grayagain deletion hook (W1) survives as the only
`correct_generation_pointers` duty and is called directly by the sweep and
move paths.

This answers R1 finding 4 by choosing one explicit strategy from its menu:
stable slots + tombstones + fixed watermark + compaction only at
sweep-complete points. `Vec::retain` is used *only* at compaction points
where it is a complete (non-incremental) operation; the incremental sweep
never compacts.

### Full incremental sweep

At the `Atomic → SweepAllGc` transition (`run_atomic`), set
`sweep_index = 0` and `sweep_watermark = allgc.slots.len()`. Entering
`SweepFinObj` / `SweepToBeFnz` re-arms both for that vector. A budgeted
sweep step (`sweep_budgeted`'s replacement) is two-phase:

1. **Scan phase** (short `RefCell` borrow): examine up to `budget` slots in
   `slots[sweep_index .. sweep_watermark]`. Tombstone → skip (counts as one
   work unit, so `StepBudget::from_work(1)` still advances and terminates).
   Dead white (`color == other_white()`) → tombstone the slot, run the
   grayagain deletion hook if `gray_listed()`, and transfer the ptr into
   `pending_release` (below). **No accounting happens in the scan.** Live →
   recolor Black/Gray to `current_white()`, exactly today's
   `sweep_budgeted` logic.
2. **Release phase** (all owner-structure borrows released): drain
   `pending_release` one object at a time — pop, settle *that object's*
   accounting (byte refund via `header.size()`, `allocation_tokens`
   removal, `objects` decrement), `release_box` it, then move to the next.

### Destruction ownership and reentrancy (R2 findings 1 and 6)

Dead-in-transit boxes are owned by a sixth heap structure, not a stack
scratch list:

```rust
pending_release: RefCell<Vec<NonNull<GcBox<dyn Trace>>>>,
releasing:       Cell<bool>,
```

The exactly-one-owner invariant now holds at every instant: the scan moves
a dead box's single owning reference from its owner slot into
`pending_release` (a different `RefCell`; both borrows are transient), and
the drain moves it from `pending_release` into the local frame that
immediately frees it. Rev-2's dead pointers "in none of the five
structures" no longer exist.

**Accounting travels with release, per object, in pop order.** This is
today's ordering guarantee restated: current `sweep_budgeted` unlinks,
accounts, and calls `release_box` for each object before inspecting the
next. Rev-2's batch accounting broke it — R2's counterexample: dead peers A
and B both refunded up front, then A's `Drop` calls
`B.account_buffer(heap, n)` (legal: B's `HDR_COLLECTED` is still set, per
`Gc::account_buffer`), inflating `bytes` with no later refund — the pacer
drifts permanently. Under the per-object rule a peer still in
`pending_release` has *not* been refunded yet, so a reentrant charge lands
in `B.header.size` and heap `bytes` together and is refunded in full when B
is popped. The pacer cannot drift.

**Collector reentrancy is prohibited during the drain.** `releasing` is set
for the duration of every release drain (incremental step, minor sweep,
full collect) and cleared by a drop guard so a caught panic cannot wedge
the collector shut. While set, every collection entry point — `step`,
`step_with_post_mark`, `full_collect_with_post_mark`,
`minor_collect_with_post_mark`, `incremental_step_with_post_mark`,
`mark_only_with_post_mark` — returns immediately, the same early-return
shape `paused` already has. This closes R2's nested-collection variant: a
destructor cannot start a collection that traces a `pending_release` box as
a root and then have the outer drain free it.

**Allocation during release is permitted** under one precise rule: it takes
the normal `allocate` path (or `allocate_uncollected` under bootstrap),
appends at the `allgc` tail beyond any live watermark, is charged to
`bytes` normally, and survives the current cycle. Moves are also legal
inside a destructor (they touch owner structures only through their own
transient borrows). Collections are inert per the flag; nothing else needs
a rule.

**Panic safety** falls out of heap ownership: a payload `Drop` that panics
mid-drain leaves the not-yet-released pointers owned by `pending_release` —
nothing leaks with the unwinding stack frame (R2's leak variant). The box
in flight is freed by the unwinding `Box::from_raw` drop; the remainder is
freed by the next drain or by `Heap::drop` → `drop_all`, which drains
`pending_release` first.

**Teardown drains until stable** (R2 finding 6 — `drop_all` is public, so
"weak upgrade fails during `Heap::drop`" is not a sufficient guard).
`drop_all` becomes:

1. Drain `pending_release` (per-object accounting, then free).
2. Clear grayagain (flags, then entries — existing order).
3. Loop until every owner structure is empty: `std::mem::take` each vector
   out of its `RefCell` in today's `drop_list` order (allgc, finobj,
   tobefnz, quarantined, uncollected) and free the boxes from the local. A
   destructor that allocates mid-teardown lands in a fresh (empty-again)
   vector and is collected by the next pass of the loop; a `debug_assert`
   bounds the pass count against a degenerate allocate-in-`Drop` ping-pong.
4. Zero `bytes`/`objects` only after a full pass finds every structure
   empty.

A reentrant `drop_all` from inside a payload `Drop` is safe by the same
ownership argument: the box currently being freed is owned by the outer
frame (in no structure), so the inner call cannot double-free it, and the
outer loops find their queues empty afterwards and terminate. Step 3 also
fixes a latent hole in *today's* `drop_all`: the current single-pass
`drop_list` walk empties each head cell once and then zeroes accounting, so
a destructor allocating during teardown strands its box on the just-emptied
`head` cell — leaked outright if the walk was `Heap::drop`'s own.

Invariant, stated once and enforced everywhere: **`release_box` is never
called while any owner-structure borrow is held, and only after that
object's own bookkeeping is complete.** A `Drop`-triggered allocation
during the release phase appends beyond the watermark and survives the
cycle (below).

Slots appended at or beyond `sweep_watermark` are never visited by the
in-progress sweep. This is the structural twin of today's behavior —
`allocate` prepends at `head`, behind a cursor that only moves away from
`head`, so mid-sweep allocations are never visited today either — and the
color rule (`allocate` paints `current_white()`, sweep frees only
`other_white()`) independently protects them, pinned by
`allocation_during_incremental_sweep_survives_current_cycle`. Both guards
stay.

### Every owner-list move vs. an in-progress sweep cursor

The three transitions keep their exact current semantics, including the
sweep-phase recolor that R1 finding 3/7 called load-bearing:

- `move_allgc_to_finobj` (from `luaC_checkfinalizer` path): requires
  `HDR_COLLECTED`; tombstone its `allgc` slot (linear membership scan over
  slots — cold path; the cost is bounded by the density policy, see "The
  one mutation rule"); **if `GcState::is_sweep()`, recolor to
  `current_white()`** (unchanged); push onto `finobj` tail. Tail = nursery
  cohort region, matching today's `link_to_head(finobj)` which also lands in
  the newest region; header age is untouched either way.
- `move_finobj_to_tobefnz`: tombstone in `finobj`, push onto `tobefnz` tail
  (today: `link_to_tail` — order-preserving append, identical). No recolor,
  matching today.
- `move_tobefnz_to_allgc`: tombstone in `tobefnz`; **if sweeping, recolor
  `current_white()`** (unchanged); push onto `allgc` tail. Today's
  `firstold1` special-case for `Old1`-aged objects is dropped with
  `firstold1` itself. Rev-2 defended this by claiming the moved object
  retains a grayagain entry; **that claim was false and is withdrawn** (R2
  finding 4): `unlink_from_list` always runs
  `correct_generation_pointers`, whose final duty is `unlink_grayagain`, so
  today every cross-list move *deletes* the object's grayagain entry — the
  behavior W1's deletion case 3 specifies and preserves. The deletion of
  `firstold1` therefore rests on one ground only: **it is written and
  cursor-corrected but read by no collector decision** (its only reader is
  a test assert), so removing it cannot change behavior. The moved object's
  *own* survival in minors needs no entry — the young sweep frees only
  `is_white() && !age.is_old()`, and `Old1` is old. Whether its *young
  descendants* can be missed after the entry deletion when the only path to
  them runs through an exact-`Old` parent (which `Marker::should_trace_age`
  skips) is a property of *today's* code that W2 inherits unchanged; test
  G3 pins it against current behavior as the oracle baseline, and if that
  baseline itself shows a reachable young child dying, that is a
  pre-existing bug to file separately — not a W2 regression and not
  something W2 silently fixes.

Cursor interaction table, for a move that tombstones slot `j` in the vector
the sweep is currently walking (`i = sweep_index`, `w = sweep_watermark`):

Positionally, the source-slot cases are transition-independent:

| case | what the sweep sees | outcome |
|---|---|---|
| `j < i` (before cursor) | nothing — slot already visited | already recolored live by the scan; now leaves the list; no double visit |
| `j == i` (at cursor) | next scan reads `None`, skips | one wasted work unit; no cursor rewrite needed |
| `i < j < w` (ahead of cursor) | scan reads `None` when it arrives, skips | object is not swept from the *source* list; its fate is transition-specific — table below |
| `j >= w` (beyond watermark) | never visited | object was appended mid-sweep and moved again; both slots beyond watermark or tombstoned |

What protects the moved object afterwards differs per transition — rev-2's
table wrongly generalized "the `is_sweep()` recolor" to all three (R2
finding 5: `move_finobj_to_tobefnz` deliberately does not recolor, today or
here):

| transition | recolor? | what keeps the object correct through the rest of the cycle |
|---|---|---|
| `allgc → finobj` | yes, `current_white()` when `is_sweep()` | the recolor: `SweepFinObj` runs after `SweepAllGc`, its phase-entry watermark includes the new tail slot, and the scan sees a this-cycle-live white and skips it; if the finobj phase is somehow already past, not being visited is equally safe — the recolor already made it live-white |
| `finobj → tobefnz` | **no** (deliberate, matching today's `move_finobj_to_tobefnz`) | being **marked before promotion**: the runtime resurrects dead finalizable objects in the atomic post-mark hook before promoting them, so the object is not `other_white()` when a later sweep reaches it — and when promotion happens with `SweepToBeFnz` not yet started, the phase-entry watermark includes the tail slot and the object *is* swept there, exactly as today's `link_to_tail` chain walk reaches it. One documented divergence: a still-dead-white object promoted *during* `SweepToBeFnz` lands beyond the watermark and is not visited this cycle (today's chain walk does reach a growing tail), so its box free defers one cycle — the next `start_cycle` repaint + sweep collects it; a liveness-timing difference, not a safety one, and `__gc` order is unaffected (registry-owned) |
| `tobefnz → allgc` | yes, `current_white()` when `is_sweep()` | the recolor: the usual call window is finalizer dispatch (`CallFin` or between cycles, no cursor live); if invoked mid-sweep, `SweepAllGc` has already passed, the object is not visited in `allgc` this cycle, and the recolor makes that safe |

The destination push always lands at the tail; position never does the
semantic work. For the two recoloring transitions the recolor rule is the C
design's reasoning (`lgc.c` makes moved objects white during sweep so a
sweep that already passed them cannot free them); for `finobj → tobefnz`
the invariant is mark-before-promote plus later tobefnz sweep, and the spec
depends on the runtime maintaining it — test F1/M3 assert it.

`barrier` / `barrier_back` / the generational barriers touch only colors,
ages, the marker's gray queue, and grayagain — no owner-vec access at all.
Barrier-during-sweep therefore cannot invalidate anything (R1 finding 5's
reframing of rev-1's wrongly-named hazard).

### Minor collection (`sweep_young`) in vector terms

`minor_collect_with_post_mark` remains stop-the-world, single-call (R1
finding 5: there is no mutator resumption inside `sweep_young` today, and
this design does not introduce one). Mapping:

- Scan range: today's `sweep_young_range(head → survival)` +
  `(survival → old1)` walks nursery + survival cohorts. Vector version scans
  `allgc.slots[reallyold + old1 ..]` — the same two cohorts. Traversal order
  flips from newest-first to oldest-first within the slice; per-element
  handling (free white non-old, `next_after_minor` aging, recolor `New` →
  `current_white` / `Touched*` → Black, `push_next_revisit`) reads no other
  element, so no order dependence exists in the logic. The battery is the
  falsifier (test matrix).
- `finobj` mirror: scan `finobj.slots[reallyold + old1 ..]`; `tobefnz`: scan
  all slots (today `sweep_young_range(tobefnz, None)`).
- Grayagain revisit pass: unchanged from today (`take_grayagain`,
  `OldRevisitTracker` positional filtering, age advance for unprocessed
  entries, `replace_grayagain(next_revisit)`) — W1 already converted the
  container.
- Frees are two-phase exactly like the incremental sweep: the scan
  tombstones and transfers dead boxes into `pending_release` under the
  borrow, compaction and cohort rotation complete all bookkeeping, and the
  `pending_release` drain (per-object accounting + `release_box`, with
  `releasing` set) runs last, after every borrow is released.
- Cohort rotation at the end (replacing today's cursor rotation in
  `sweep_young`): run the single whole-vector compaction (the slice-only
  variant is withdrawn — see "The one mutation rule"), then
  `reallyold += old1; old1 = survivors_of_survival_cohort;
  survival = survivors_of_nursery_cohort`, with survivor counts taken from
  the compaction's boundary recount. Today's `new_old1` boundary object and
  `survival = head.get()` cursor writes fall out as index arithmetic.
- `promote_all_to_old` → **compact first** (its call sites are VM mode
  transitions at `Pause`; no cursor is live), then
  `reallyold = slots.len(), old1 = survival = 0`. Rev-2's
  `reallyold = live_len` without compaction was wrong whenever holes
  preceded a live tail (R2 finding 2). `reset_all_ages` /
  `clear_generation_cursors` → all three counters to 0 + clear grayagain.

### Unique-ownership invariant (R1 finding 10)

Every heap-owned box is referenced by **exactly one** owning entry across
the six structures:
`allgc.slots ∪ finobj.slots ∪ tobefnz.slots ∪ quarantined ∪ uncollected ∪
pending_release`. `pending_release` is the transitional owner between "slot
tombstoned by a sweep scan" and "freed by the release drain" — it is what
makes the invariant hold *during* destruction, which rev-2's stack scratch
list did not (R2 finding 1). Structure membership is explicit — never
inferred from flags. Flag semantics
are untouched and remain coarse invariant guards, not membership metadata:
`HDR_COLLECTED` = "in one of the three sweepable vectors" (checked by
`move_allgc_to_finobj`, `Gc::account_buffer`), `HDR_HEAP_OWNED` = "in any of
the five" (strict-guard checks), `HDR_FREED` = "parked in quarantined".
As today, `HDR_COLLECTED` is not cleared when a box moves to quarantined;
the invariant text for the sweepable set is therefore "`HDR_COLLECTED` set
and `HDR_FREED` clear." `grayagain` is **non-owning** and must be a subset
of the live sweepable set (W1 deletion cases keep it so). Accounting
asymmetries carry over verbatim: `allocate_uncollected` charges neither
`bytes` nor `objects`; quarantined boxes had bytes/token/object accounting
settled before `release_box` parked them.

Teardown: the drain-until-stable `drop_all` specified under "Destruction
ownership and reentrancy" — `pending_release` first, grayagain second,
then repeated `std::mem::take`-and-free passes over the owner structures
until a full pass finds them all empty, and only then the accounting zero.
No borrow is live while payload `Drop` runs, and no box can be reached
twice because each is in exactly one structure at every instant.

### Phase-order table

| Phase (`GcState`) | Collector work | Mutator windows between steps | Owner-vec mutations permitted | Cursor validity |
|---|---|---|---|---|
| `Pause` | none | yes | append (`allocate`), tombstone+append (moves) | no cursor; compaction allowed |
| `Propagate` | `drain_gray_budgeted` | yes | append, moves (no recolor — not sweep) | no cursor |
| `EnterAtomic` | state hop | yes (budget can end here) | append, moves | no cursor |
| `Atomic` | `run_atomic`: final drain, post-mark hook, arm `sweep_index`/`sweep_watermark` | no (single step) | none (STW step) | cursor armed at end |
| `SweepAllGc` | budgeted two-phase scan of `allgc` | yes | append ≥ watermark; moves tombstone per the case table; sweep-phase recolor applies | `sweep_index` valid — tombstones never shift indices |
| `SweepFinObj` | same for `finobj` | yes | same | re-armed at phase entry |
| `SweepToBeFnz` | same for `tobefnz` | yes | same (incl. `move_tobefnz_to_allgc` from finalizer dispatch) | re-armed at phase entry |
| `SweepEnd` | state hop | yes | append, moves | cursor dead |
| `CallFin` | `finish_cycle`: **compaction of all three + boundary recount**, threshold calc | — | — | no cursor; compaction point |
| minor (STW inside `minor_collect_with_post_mark`) | mark revisits + roots → atomic hook → `sweep_young` scan/compact/rotate → release | none | payload-`Drop` allocations during release append at tail | no persistent cursor; compaction inside |
| `abort_cycle` | repaint colors, reset state | — | — | compaction point |

### Pacer accounting (R1 finding 9)

Definition change, stated explicitly: **pacer `bytes` counts box + charged
buffer bytes only, excluding ownership-metadata storage** (owner-vector
backing). Rationale: owner-slot bytes are proportional to *live* objects and
are reclaimed only at compaction, so charging them would feed capacity —
which does not shrink when objects die — into a threshold formula
(`finish_cycle`: `bytes * pause_multiplier / 100`) that exists to measure
collectable pressure. The mechanism is untouched: `allocate` charges
`size_of::<GcBox<T>>()`, `Gc::account_buffer` adjusts, sweep refunds
`header.size()`.

Consequences to measure, not hand-wave: after the diet every header charge
shrinks (−32 B/object 64-bit, −16 B wasm32), so thresholds derived from
`bytes` drop and collection cadence shifts even with zero behavior change —
R1's point that the representation change alters collection thresholds is
correct and applies to W1 too (−16/−8 B per object). The measurement plan
therefore records `collections()` / `minor_collections()` per workload
before/after, and a new diagnostic `Heap::owner_capacity_bytes()` (sum of
`capacity × slot size` across owner vectors) feeds heap-diff so total memory
claims include the relocated pointer and its slack. If cadence shift alone
regresses a GC-heavy row, the fallback is an explicit slack charge into
`bytes` at compaction points — a measured decision, not a default.

### Diagnostics and remaining surfaces

`for_each_header` / `start_cycle`'s repaint / `promote_all_to_old` /
`reset_all_ages` / `abort_cycle`: iterate vectors, skip `None`.
`type_name_count` and `allgc_cohort_stats` (lua-cli telemetry): trivial slot
walks; cohort stats become index arithmetic instead of a cursor-compare
walk. `Marker` internals: untouched (its `gray_queue` is already a
`Vec<NonNull<GcBox<dyn Trace>>>`). No code outside `heap.rs` touches
`header.next`/`gray_next` (verified by grep across `lua-gc`, `lua-types`,
`lua-vm`); `lua-vm` consumes only the three `move_*` functions, whose
signatures are unchanged.

### tobefnz FIFO (R1 finding 7)

Stated explicitly: **`FinalizerRegistry` is the semantic FIFO authority**
for `__gc` order — `push_to_be_finalized` appends, `pop_to_be_finalized`
consumes via `remove(0)`. The heap-side `tobefnz` vector is a *membership*
structure only: entries enter both structures in the same act (the runtime
registers the entry and calls `move_finobj_to_tobefnz`) and leave in the
same act (`pop_to_be_finalized` then `move_tobefnz_to_allgc`), so the sets
stay equal; the heap-side *order* is read by nothing except sweep visitation
(order-insensitive) and `drop_all`. Tail-append (today's `link_to_tail`)
preserves front-to-back FIFO order in the heap vector anyway, but
correctness does not lean on it. `swap_remove` appears nowhere in this
design.

Risk: MEDIUM-HIGH stands — this rewires the collector's spine, and the
tombstone/watermark/`pending_release` machinery is new, with no in-tree
precedent for its hard parts (the withdrawn registry claim). Mitigations:
the tombstone rule is a single mechanism replacing seven cursor patches;
the destruction window is one flag with the same shape as `paused`; W2
starts only after W1's measured landing; supervised branch, full battery
per commit, no mixing with other GC work.

## Wave 3 — deleted (R1 finding 2 accepted)

After both links are removed the header is already 8 B on both widths —
`color(1) + age(1) + flags(1) + pad(1) + size(4)` — so `Cell<u16>` packing
saves nothing. Worse, it is a measured anti-pattern in this exact spot: the
`GcHeader` doc-comment records that packing the hot fields cost ~+4% Ir on
gc_pressure (recount 2026-06-10), which is why color/age/flags each own a
byte today. `Cell`-based bit-packing turns every hot color/age read-write in
`mark_box`, `drain_gray_queue`, and the sweep loops into read-modify-write.
Wave 3 is deleted, not deferred.

What replaces it (mechanical, rides with W2's PR): `#[repr(u8)]` on `Color`
and `GcAge` to lock the one-byte layout, and a compile-time size assertion
on `GcHeader` — valid ungated on both widths since the post-diet header
contains no pointers (`const _: () = assert!(size_of::<GcHeader>() == 8)`),
plus updated `value_layout` example output.

## Measurement plan (replaces rev-1's RSS projections)

Rev-1's projected ratios (closure_ops 2.96× → ~2.2×, binarytrees 2.22× →
~1.9×) are withdrawn — R1's arithmetic critique is accepted in full: the W2
pointer is relocated not removed, vector capacity was omitted, allocator
buckets are unknown, no absolute C/Rust peak-RSS pairs were shown, and prior
evidence (the R2-diet packet) showed payload-byte savings translating
roughly 1:1 into process RSS, not into outsized ratio movement. No number
appears below as a target; the done-condition is that the following
measurements exist and the drop-if-neutral rule is applied to them.

All measurements follow `docs/MEASUREMENT_PROTOCOL.md`: frozen baseline
binary built from origin/main before edits, interleaved A/B within rounds,
≥4 rounds judged on min-ratio, quiet machine for any number that enters the
PR body, revert-validation for any surprise, and the bench host used
exclusively (coordination board).

Per wave (W1 first, W2 separately):

1. **Compute-neutrality (Ir arbiter)** — `harness/bench/instr-count.sh` on
   fibonacci and mandelbrot. These rows allocate little and must be flat;
   the abort criterion is Ir regression > 2% on either. This is the
   instruction-removal class check: the diet must be free for non-GC paths.
2. **GC-path Ir** — instr-count on binarytrees, table_ops, closure_ops.
   W1 expectation: flat to slightly down (barrier fast path loses a Cell
   write). W2 expectation: sweep-loop instruction mix changes shape
   (pointer-chase → linear scan + tombstone skips); an Ir increase here with
   a wall/Bcm improvement is a legitimate CPI-class outcome — classify
   before judging, per the protocol's win-class table. Any change touching
   slot layout is codegen-layout-adjacent: pair Ir with a cold-machine wall
   A/B (the T5a lesson).
3. **Heap-diff including ownership storage** — `harness/bench/heap-diff.sh`
   on closure_ops and binarytrees for alloc-count / bytes-per-block deltas,
   which surfaces both the shrunken `GcBox` allocations and the owner-vector
   backing blocks (few, large). Supplement with the new
   `Heap::owner_capacity_bytes()` diagnostic sampled at workload peak so
   excess capacity and tombstone slack are numbers, not assumptions.
4. **Peak RSS with absolutes** — `harness/bench/compare.sh` on the `_long`
   GC-heavy rows, reporting C peak, Rust-before peak, Rust-after peak in
   bytes, and only then the ratios. Sub-100ms rows are excluded
   (startup-dominated, per the protocol).
5. **Cadence** — `collections()` / `minor_collections()` totals per workload
   before/after. A shift is *expected* (header bytes shrink, so
   pacer-derived thresholds shift; `GC_MIN_THRESHOLD` floors small heaps).
   Record it; if a GC-heavy row's wall regression tracks the cadence change,
   test the fallback (slack charge at compaction) before concluding.
6. **wasm32** — `cargo check -p lua-vm --target wasm32-unknown-unknown` is
   the compile gate (layout asserts must not be 64-bit-only), but per R1
   finding 11 that is not a memory measurement: additionally run a fixed
   GC-heavy workload under a wasm runtime and record linear-memory
   high-water (`memory.size` after run, before/after the change — the
   playground harness or wasmtime both expose it). Wasm linear memory
   retains grown pages, so owner-vector growth spikes cost real,
   unreturnable pages there; this is the environment where capacity slack
   matters most.

Decision rule: W1 keeps if Ir is flat on compute rows and heap-diff/RSS do
not regress. W2 keeps only if the measurements show a net win at peak
(heap-diff + owner_capacity_bytes accounting) or a wall/Ir win from sweep
locality — otherwise it is reverted and the honest negative recorded, per
the drop-if-neutral rule. "The 8-byte header is aesthetically right" is not
a keep reason.

## Test matrix

R1's enumerated list, mapped to existing gates or new kit tests. "Kit"
means deterministic in-memory tests in `heap.rs`'s test module — the
fast-inner-loop tier, milliseconds per run.

| # | Scenario | Existing coverage | New work |
|---|---|---|---|
| M1 | Each of the 3 owner moves with target **before** the active sweep cursor | none mid-sweep (`finalizer_intrusive_lists_sweep_and_drop` is at-Pause only) | NEW kit ×3 transitions: drive `incremental_run_until_state_with_post_mark` to `SweepAllGc`, advance partially with `StepBudget::from_work(n)`, move an already-visited object, finish; **assert the move returned `true`** (R1 item 7 sync note), then membership/liveness/`allgc_count` |
| M2 | Moves with target **at** the cursor slot | none | NEW kit ×3 transitions (same harness, position at `sweep_index`); assert move returned `true` |
| M3 | Moves with target **ahead of** the cursor (and beyond watermark) — per-transition expectations | none | NEW kit ×3: `allgc→finobj` and `tobefnz→allgc` assert the `current_white()` recolor carried the object through both lists' sweeps; `finobj→tobefnz` (no recolor) asserts the marked-before-promotion invariant instead, plus the documented one-cycle free deferral for a dead-white promotion during `SweepToBeFnz`; every case asserts the move returned `true` |
| A1 | Allocation during `SweepAllGc` | `allocation_during_incremental_sweep_survives_current_cycle` | extend to `SweepFinObj`, `SweepToBeFnz`, and the `EnterAtomic`→`Atomic` gap |
| B1 | `StepBudget::from_work(1)` resumption to completion | `full_collect_equivalent_to_incremental_to_pause`, `budget_zero_does_some_work`, `sweep_can_pause_and_resume` | NEW kit variant: interleave a move + an allocation at every `InProgress` step under budget 1 (churn test; also proves tombstone-skip termination) |
| B2 | Adversarial repeated-move churn (R2 finding 2) | none | NEW kit: cycle one object `allgc → finobj → tobefnz → allgc` for N ≫ threshold rounds, at `Pause` and again with an incremental sweep paused mid-`allgc`; assert every move returns `true`, slot counts stay within the `(4/3)·live + cycle-churn` bound, boundaries stay physical-index-consistent, and post-compaction order/cohorts are intact |
| F1 | FIFO finalization order | `finalizer_registry_minor_snapshot_uses_cohort_boundaries`, `finalizer_registry_marks_and_clears_finalized_bit`; gc.lua/gengc.lua `__gc`-order asserts | NEW kit: registry↔heap sync — register N finalizable, kill all, pop FIFO, assert each `move_tobefnz_to_allgc` succeeds and set-equality holds throughout |
| G1 | grayagain deletion via full-sweep free | `full_sweep_unlinks_freed_grayagain_entries` | ports as-is |
| G2 | grayagain dedup + persistence | `grayagain_links_object_once`, `grayagain_list_carries_old1_until_old`, `grayagain_list_carries_touched2_until_old` | port as-is (these pin the persistence facts rev-1 got wrong) |
| G3 | Cross-list move of a gray-listed transitional object, graph-shaped (R2 finding 4) | none (the deletion happens today via `unlink_from_list` → `correct_generation_pointers` → `unlink_grayagain`, untested) | NEW kit, run against **current** code first to fix the oracle baseline: root → exact-`Old` parent (skipped by `Marker::should_trace_age`) → `Old1`/`Touched2` object → young child; move the transitional object (`move_allgc_to_finobj`, and the `move_tobefnz_to_allgc` variant), assert the move returned `true` and `grayagain_count` dropped, then run a minor and assert the young child's fate matches the pre-W2 baseline; a baseline-level child loss is filed as a separate pre-existing issue |
| Q1 | Quarantine parking + single free at teardown | canary battery under `LUA_RS_GC_QUARANTINE=1`, `harness/asan-stress.sh` | NEW kit: quarantined box's slot is tombstoned, box appears once in `quarantined`, `drop_all` frees exactly once |
| U1 | Uncollected teardown | `allocate_uncollected_survives_collection_but_is_freed_on_heap_drop`, `bootstrapping_routes_allocate_to_the_uncollected_list`, #249 leak canaries (with the git-stash-revert verification caveat) | port as-is |
| O1 | Newest-first order dependence (absence thereof) | not directly unit-testable — it is a claim of *no* dependence | falsifiers: 36 GC canaries × incremental+generational (`harness/canaries/gc/run_canaries.sh`), gc.lua + gengc.lua officials ×5 versions, the quarantine canary run |
| C1 | Cohort rotation / scan bounding | `minor_collect_frees_young_and_keeps_old`, `minor_sweep_uses_generation_cursors_to_skip_old_tail` (sweep-visited counts pin the scan range), `minor_collect_skips_untouched_old_root_scan_work`, `promote_and_reset_all_ages`, `full_sweep_corrects_generation_cursors_when_cursor_object_is_freed` (assertion rewrites to counter state) | port with cursor asserts translated to counter/index asserts |

PR gate on top of the kit tier: full battery — canaries ×2 modes,
quarantine run, strict-guard run, `harness/run_official_all.sh`,
`specs/oracle/check.sh` ×5, workspace tests, wasm check — per the repo's
rung-6 definition.

## Sequencing & gates

1. **W1 lands alone**: own PR, kit rows G1–G3 + A1 + battery + measurement
   plan items 1/3/5. Small, reversible, and it produces the first real
   number for "what does removing one link actually buy" — which is the
   evidence W2's approval depends on.
2. **W2 only after** W1's measured result and this spec's round-2 review
   verdict. Supervised branch (per the deep-spec → codex-review → execute
   workflow), full battery per commit, no mixing with other GC work. The
   `repr(u8)`/size-assert cleanup (ex-W3) rides with W2.
3. Abort criteria (unchanged from rev-1, plus one): any canary/quarantine
   flip that needs more than a localized fix; Ir regression > 2% on
   fibonacci/mandelbrot; new: a GC-heavy-row regression attributable to
   cadence or tombstone-scan overhead that survives the slack-charge
   fallback test — then W2 reverts and the negative is recorded.

## Relation to other open work

Independent at the code level of #252 (Rc<Heap> ownership — already landed;
`Heap::new` returns `Rc<Self>` in current main, so rev-1's "sequence after"
note is satisfied) and #253 (LuaError bytes). If #253 is still open when W2
starts, land it first for the same diff-purity reason as before. The
`grayagain`/owner-vector work overlaps textually with any generational-GC
follow-up in `specs/followup/issue-93-generational-gc-plan.md`; coordinate
on the board before parallel GC branches exist.
