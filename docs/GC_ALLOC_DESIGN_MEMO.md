# GC/allocator architecture memo (sprint-2 T3a)

Supervisor-authored 2026-06-11 per `docs/archive/PERF_SPRINT_2_GOAL.md` §T3.
Quantifies `docs/archive/GC_ALLOC_PLAN.md` causes 2/3 with fresh dhat absolutes and
ranks the options. One bounded step (T3b) is approved out of this memo;
everything deeper needs explicit human sign-off.

## Fresh dhat absolutes (sha 5727ee4, `--features dhat-heap` release)

| workload | total blocks | total bytes | avg B/block | peak live | profile |
|---|---:|---:|---:|---:|---|
| gc_pressure | 601,293 | 54.5 MB | 90.7 | 85 KB / 734 blk | pure churn |
| concat_chain | **13,901,829** | 527.5 MB | **37.9** | 105 KB / 899 blk | extreme small-block churn |
| binarytrees | 6,315,592 | 748.9 MB | 118.6 | 36.0 MB / 263,915 blk | churn + large live set |
| table_hash_pressure | 1,425,446 | 91.3 MB | 64.0 | 44.5 MB / 455,302 blk | large live set |

Context: wall ratios gc_pressure 1.98, concat_chain 2.02, binarytrees 1.77;
RSS ratios binarytrees 2.54, table_hash_pressure 2.14 (matrix
20260611T164856Z-b0e68f8).

## The cost structure (verified in source, 2026-06-11)

1. **Every allocation does a side-table HashMap insert.** `Heap::allocate`
   (heap.rs:1496-1516) ends with `allocation_tokens.borrow_mut()
   .insert(identity, token)` — the weak-handle validation table
   (PERFORMANCE_MODEL.md candidate 10). Sweep removes the entry
   (heap.rs:2447, 2526). So every object pays insert + remove + its share of
   map capacity (~50 B/live object at the high-water mark), even though the
   only consumer is weak-handle creation/validation
   (lua-types/src/gc.rs:82,128) and **almost no object ever gets a weak
   handle**. For the table above that is 601 k / 13.9 M / 6.3 M / 1.4 M
   insert+remove pairs per run.
2. **Every object is an independent `Box::new` / `Box::from_raw`**
   (heap.rs:1496, release_box heap.rs:1663). No pooling, no size classes;
   quarantine mode parks instead of freeing (HDR_FREED) — any pooling design
   must coexist with that tripwire.
3. **Three mallocs per non-empty table** (GC_ALLOC_PLAN cause 2): box + node
   Vec + array Vec.
4. **GcHeader is 40 B** and that is close to its floor: the two intrusive
   links are `NonNull<GcBox<dyn Trace>>` — FAT pointers (16 B each, 32 B of
   the 40). The header doc-comment records that packing the hot fields was
   already tried and REGRESSED (+4% Ir on gc_pressure, recount 2026-06-10).
   Going below 40 means thin-pointer + vtable-recovery unsafe surgery.
5. **concat_chain allocates ~14 M blocks at 38 B average** — multiple
   allocations per string temporary (GcBox<LuaString> 64 B + the separate
   `Rc<[u8]>` payload + token insert + …) where C pays one TString block per
   result. This is its own follow-up packet (below), not part of T3b.

## Options, ranked

### R1 — APPROVED as T3b: lazy weak-token registration (candidate 10)

Move token registration from `allocate` to weak-handle creation:
`gc.rs:82` already queries `allocation_token(identity)` while holding a
strong ref — replace with a `register_allocation_token(identity)`
(get-or-insert, monotonic `next_token`). Delete the insert from `allocate`;
sweep's removal becomes remove-if-present (already is). Correctness argument:
every valid weak handle registered its identity at creation, so
`contains_allocation` returning false for an absent identity means swept —
exactly today's semantics; monotonic tokens keep address-reuse safe; an
object never weak-referenced never enters the map. Expected, measurable:
Ir DOWN on all four rows above (per-iteration budgets), heap-diff total
blocks/bytes down (the map's rehash allocations disappear), RSS down on the
live-set rows. Bounded to heap.rs + lua-types/gc.rs — no overlap with T2's
table/vm files. Gates: full T2-style battery PLUS the weak-table canaries and
the heap.rs token unit tests (heap.rs:3187-3248) updated to the lazy
contract, plus quarantine on gc.lua (weak tables stress validation).

### R2 — concat string-churn packet (discovered; next sprint or T3c with sign-off)

14 M blocks for one workload is the single largest allocation anomaly in the
tree. Needs its own recon: allocations per concat op (expect GcBox + Rc<[u8]>
+ intern probe), whether the concat fast path can build into a reusable
buffer and intern once. Not bounded enough for this sprint's T3b.

### R3 — size-class free lists for GcBoxes (deferred, needs human sign-off)

Converts Box::new/from_raw into free-list pop/push via raw `Layout` alloc.
Addresses every churn row at once, but: unsafe-budgeted surgery in the
allocator, must coexist with quarantine (pooled-and-reused memory defeats the
HDR_FREED tripwire unless pooling is disabled under quarantine), and R1
should land first so the win is measured on top of it.

### R4 — candidate 9, Vec→Box<[T]> table parts (stays on the #113 ladder)

−16 B/table box + removes the cap fields; touches table.rs (T2 conflict —
sequence after T2 merges). Modest, safe, still worth doing; not the top lever.

### R5 — REJECTED for now: GcHeader sub-40 diet

Hot-field packing measured +4% Ir (2026-06-10, header doc-comment); the
remaining 32 B is two fat pointers, and thin-pointer redesign is deep unsafe
work with quarantine/Trace-object interplay. Reopen only with an R1+R3-level
RSS shortfall and a Linux branch-miss measurement.

### R6 — pacer cadence tuning (deferred)

Wall-only lever; tuning cadence while concat does 14 M mallocs measures the
allocator, not the pacer. Revisit after R1/R2.

## SmallVec stays rejected

GC_ALLOC_PLAN's inline-storage lesson stands (20-35% slower at every size);
nothing here relitigates it.
