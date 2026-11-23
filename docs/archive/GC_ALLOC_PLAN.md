# lua-rs GC + Allocator Deep Dive

## 1. The gap, measured

Ground truth this session (Apple M3 Max, best-of-5):

| Metric | lua-rs | luars | reference C |
|---|---|---|---|
| Geomean wall | 1.69x | 0.88x | 1.00x |
| binary_trees wall | 2.75x | - | 1.00x |
| binary_trees RSS | 357 MB | 67 MB | 51 MB |
| table_hash_pressure wall | 4.17x | - | 1.00x |
| table_hash_pressure RSS | 191 MB | 45 MB | - |
| gc_pressure wall | 2.0x | - | 1.00x |

lua-rs is worst exactly where allocation and GC dominate. The RSS gap is 5-7x C and is the headline target. luars (C-in-unsafe-Rust: paged pool allocator, union TValue, incremental GC ported straight from C) sits within noise of C, so the gap is not "Rust tax" — it is four specific architectural choices in our from-scratch GC:

1. **Uncharged bytes -> systematic under-collection (the dominant RSS cause).** `Heap::allocate` charges only `size_of::<GcBox<T>>()` to the pacer (heap.rs:555,566) and freezes that into `header.size`; sweep refunds exactly that (heap.rs:869,886). The table array `Vec` backing (table.rs:606), the node `Vec` backing (table.rs:561-563), and the string `Rc<[u8]>` payload (string.rs:8,17) are **never charged**. The threshold is `bytes * pause/100` floored at 256KB (heap.rs:494,894-899). So `bytes` under-reports true footprint roughly 3-5x, the pacer thinks the heap is tiny, collections fire far too rarely, and the live+dead set balloons. On binary_trees this is on the order of 200MB+ of the 357MB that the pacer simply cannot see.

2. **Three system mallocs per non-empty table.** dhat (`lua-cli --features dhat-heap`) shows every non-empty table literal = `GcBox<LuaTable>` (heap.rs:556) + node `Vec` (table.rs:561) + array `Vec` (table.rs:606). Empty `{}` is 1. This is per-object allocator metadata and fragmentation pressure; mimalloc as global allocator bought +9% binary_trees / +20% gc_pressure, proving the per-alloc cost is real.

3. **No pool / recycling.** Every `GcBox` is an independent `Box::new` and every sweep is a `Box::from_raw` free. luars holds 67MB substantially because its paged pool keeps objects dense (locality, amortized metadata, low fragmentation). We thrash the system allocator on a churn workload.

4. **Per-cycle mark overhead.** `Marker.visited: HashSet<usize>` (heap.rs:343) is rebuilt empty every cycle and takes O(live) sip-hashed pointer inserts plus its own malloc/rehash churn, on top of an O(heap) reset-to-white walk (heap.rs:800-805). This is mark-phase CPU, not RSS.

Cause (1) is the RSS lever. Causes (2)/(3) are alloc-throughput levers. Cause (4) is mark-CPU only and does not move RSS.

## 2. The inline-storage lesson

A prior attempt cut 3 mallocs -> 1 by inlining tiny-table array/node storage (SmallVec) into `LuaTable`. It measured **20-35% SLOWER at every inline size** and was reverted. The cause is structural: a table is **accessed far more than it is allocated**, so the inline approach paid a per-access inline/spill discriminant branch on every read, enlarged the `LuaTable`/`GcBox` struct (hurting cache density for all tables including big ones), and added a `resize()` swap-memcpy on the spill transition — and those recurring costs dwarfed the one-time mallocs saved. The lesson governs every intervention here: **allocation-COUNT reductions are not the goal; measured RSS and wall are.** Any intervention that adds cost to the hot table-access path or bloats the `LuaTable`/`GcBox` struct is suspect by default and must clear a wall-time-neutrality gate, not just a dhat count drop.

## 3. Ranked interventions

| # | Title | Target | Expected effect | Hot-path risk | Unsafe? (lua-gc ok) | Conformance risk | Verdict |
|---|---|---|---|---|---|---|---|
| A | Constructor arg-swap fix (NewTable array/hash swapped) | alloc | record 3->2 mallocs; RSS down on record-heavy benches | none | none | low (nextvar canary) | **build-first** |
| B | Charge Vec/Rc buffer bytes to the pacer (per-object `Cell` header.size) | RSS | binary_trees 357->~67-100MB; table_hash_pressure 191->~50-70MB | none (cold paths only) | none new | low-med (count() correctness) | **build-first** |
| C | Re-tune `GC_MIN_THRESHOLD` + sweep `pause_multiplier` | both | the knob that makes B "RSS down, wall flat" vs "wall regressed" | none | none | low | **plausible (B's companion)** |
| D | Bounded recycling free-list for `GcBox<LuaTable>` (shape d) | alloc | ~+3-6% wall binary_trees; recycles 1 of 3 mallocs | none | ~5 lines | medium | **plausible** |
| E | One-malloc joint array+node `TableBuf` (shape #1) | alloc | 3->2 / 2->1 mallocs; makes B's byte count exact | **yes if {ptr,acap,ncap}** | contained | medium | **risky** |
| F | Two-white-color flip (delete `visited` HashSet) | gc-cpu | removes O(live) inserts + reset walk; does NOT move RSS | none | none new | high | **risky** |
| G | Generalize recycling to strings/closures (shape a) | alloc | string/closure churn; marginal on table benches | low | same as D | medium | **risky (after D)** |
| H | luars-parity paged size-classed pool (shape b) | RSS | largest RSS lever after B; luars-parity endgame | none | substantial | high | **risky (defer)** |
| I | Fused GcBox+buffer DST (shape #3) | alloc | 2->1 on top of E, never-resizing tables only | yes (re-treads inline trap) | invasive | high | **reject** |

### A. Constructor arg-swap fix (build-first)

The `NewTable` VM handler passes `new_table_with_sizes(b, c)` while the signature is `(array_size, hash_size)`. Reference C (`lvm.c` OP_NEWTABLE) decodes `b=hash, c=array` and calls `luaH_resize(L, t, c, b)` i.e. `(array=c, hash=b)`. The Rust port passes `(array=b, hash=c)` — **the two arguments are swapped.** Empirically confirmed with dhat: empty `{}` = 1 malloc, but pure record `{a,b,c}` = 3 mallocs (not the 2 the codegen analysis predicted). The three sites for 100k records are `GcBox<LuaTable>`, a dead array `Vec` sized from the swapped hash count, and a lazily-promoted `DUMMY_TABLE_INIT_HASH_NODES=4` node vector that only fires because the constructor allocated zero real hash part.

- Patch: swap the two args at the NewTable site (crates/lua-vm/src/vm.rs:1741), so record literals build a real pre-sized hash part and skip both the dead array Vec and the lazy DUMMY promotion. Touch points flow unchanged through `new_table_with_sizes` (crates/lua-vm/src/state.rs:1718) and `table.rs:730` (the lazy DUMMY path).
- Files: crates/lua-vm/src/vm.rs:1741 (the swap), crates/lua-vm/src/state.rs:1718, crates/lua-types/src/table.rs:730.
- Why safe: changes constructor sizing only, no hot read/access path, no struct growth, less allocation not more (so no #44 over-collection risk).
- Gate: `make test` 33/33 with **nextvar.lua green** (it asserts the array/hash boundary and the swap shifts which slots land where — the one real correctness exposure). dhat on a pure-record script must drop 3->2 mallocs. `compare_luars.sh`: binary_trees RSS down, wall within best-of-5 noise on binary_trees/gc_pressure/table_hash_pressure.

### B. Charge Vec/Rc buffer bytes to the pacer (build-first)

Make the pacer see the memory it currently ignores. This is the single highest-leverage RSS fix and the direct cause of the 5-7x gap.

- Patch: change `GcHeader.size: usize` (heap.rs:133) to `Cell<usize>` (zero-overhead repr, no struct growth). Add `Heap::charge_extra(n)` / `uncharge_extra(n)` (safe `Cell` ops). Add `Gc<T>::resize_accounting(heap, old_extra, new_extra)` that sets `header.size = base + new_extra - old_extra` and adjusts the heap total in lock-step — single source of truth, so sweep's one `header.size` refund (heap.rs:869) stays correct with no second counter to drift. Charge sites, all cold:
  - table array grow/shrink at table.rs:606: `(new_asize - old_asize) * size_of::<LuaValue>()`.
  - table node build at set_node_vector table.rs:561-563: `actual_size * size_of::<TableNode>()`, and uncharge the swapped-out old node cap in resize.
  - string at from_bytes string.rs:17: one-shot `b.len() + 2*size_of::<usize>()` (immutable, refund is automatic on sweep).
  - reach the live heap from these cold type-methods via `lua_gc::with_current_heap` (gc.rs:31-37) — off the hot access path.
- **Mandatory companions, not optional:** raise `GC_MIN_THRESHOLD` 256KB -> ~1MB (heap.rs:494) so the floor does not become a trip-wire once crossing it is easier; make `pause_multiplier` (heap.rs:534) a swept knob (this is intervention C).
- **Correctness hazard to handle explicitly:** boxes allocated UNCOLLECTED when no `HeapGuard` is active (gc.rs:34, heap.rs:205) never join the allgc chain and are never swept. If such a table is later grown under a live guard, `with_current_heap` charges its growth but no sweep ever refunds it -> `bytes` over-reports permanently. The charge path must skip charging on uncollected boxes (or charge nothing when no guard is active), matching the no-refund reality.
- Files: heap.rs:133,494,534,555,566,869,886,894-899; table.rs:561-563,606; string.rs:17; gc.rs:31-37.
- Why not the inline trap: zero hot-table-access-path cost (all charge calls on grow/create), `Cell<usize>` is same size as `usize` so no struct bloat.
- Gate (KEEP only if ALL hold): new `lua-gc` test asserting `bytes` returns to baseline after alloc+grow+drop+full-GC; `cargo test -p lua-gc`; `make test` 33/33 with gc/gengc (count() correctness) + tracegc + nextvar (resize double-count canary) green; `compare_luars.sh` binary_trees RSS <=120MB (target 67-100), table_hash_pressure <=80MB, binary_trees wall <=2.85x C, gc_pressure wall <=2.1x C after sweeping pause; dhat alloc COUNT unchanged; gc_pressure at 10x iters RSS must plateau, not creep (catches over-charge drift). Any >5% gc_pressure wall regression = revert (#44 trap).

### C. Re-tune GC_MIN_THRESHOLD + pause_multiplier (B's companion)

Both constants were implicitly tuned against under-counted bytes. Once B charges real buffer bytes, the 256KB floor becomes a trip-wire and `pause%` applies to a 5-7x larger base. These are the lever that decides whether B lands as "RSS down, wall flat" (accept) or "RSS down, wall regressed" (the #44 failure).

- Patch: raise `GC_MIN_THRESHOLD` -> ~1MB (heap.rs:494); make `pause_multiplier` a configurable `Heap` field, default 200 (heap.rs:534); threshold form `max(bytes*pause/100, floor)` unchanged (heap.rs:894-899).
- Files: heap.rs:494,534,894-899.
- Measured JOINTLY with B (meaningless alone). Sweep {floor: 256K, 1M} x {pause: 150, 200, 300}; expect the winner to be higher-floor + higher-pause. Accept the pair that is strictly "RSS down, wall within noise"; `make test` 33/33 at the chosen values.

### D. Bounded recycling free-list for GcBox<LuaTable> (plausible)

binary_trees allocates/frees tables in a tight churn wave — the pattern free-lists win. Recovers the per-alloc cost mimalloc proved real, stacks with it. Recycles only the outer `GcBox` malloc (1 of 3), so the win is bounded (~+3-6% wall) and it does NOT close RSS.

- Patch: `Heap.table_freelist: RefCell<Vec<NonNull<GcBox<LuaTable>>>>` capped ~512. `allocate::<T>` branches on `TypeId::of::<T>()==TypeId::of::<LuaTable>()` (compile-time-resolved via the monomorphized gc.rs:33 chokepoint, so no runtime type switch on the hot alloc path): pop a recycled box, `ptr::write` the value, reset header (color=White, next relinked, finalized=false), relink into allgc; miss -> `Box::new`. Sweep White arm (heap.rs:866-873): if `LuaTable` and cache not full, `ptr::drop_in_place(&mut (*p).value)` (still frees the table's own backing buffers, no leak) then push; else `Box::from_raw`. ~5 new unsafe lines.
- Files: heap.rs:554,866-873; gc.rs:31-37.
- Gate: write the `lua-gc` unit test FIRST (rung 2) — allocate, collect, re-allocate, assert pointer reused AND header re-init byte-identical to `new_white` (color White, next relinked, **finalized reset**) AND trace correct. Then `make test` 33/33 (gc/gengc break first on header re-init); `compare_luars.sh` binary_trees wall must IMPROVE >=3% (else not worth the complexity, revert), gc_pressure no regression, binary_trees RSS flat-to-better (RSS regression from cache holding too much = revert).

### E. One-malloc joint array+node TableBuf (risky)

Merge the two table Vecs into one raw buffer `[LuaValue;acap] ++ [TableNode;ncap]`. Structurally the correct version of the inline-storage idea — same malloc-count attack, but on the allocation path only — **provided the layout is amended.** The sketched `{ptr, acap, ncap}` forces a node-base recompute (load acap, multiply, align_up, add) on every node access (get_str_value/get_int_value_cold loops), which is exactly the per-access tax that killed SmallVec. The "struct shrinks AND zero per-access cost" claim is internally contradictory.

- Amendment: cache `node_ptr` (and lens) eagerly — a 4-word `TableBuf` (still a shrink vs two 3-word Vecs = 6 words) with branch-free node access. Provide a helper returning `(&[LuaValue], &[TableNode])` computed once per method, not per index (94 indexing sites in table.rs).
- Files: table.rs (TableInner:231, set_node_vector:550, resize:578-622) + a `Sized` unsafe `TableBuf` helper in heap.rs; lua-types stays `unsafe=forbid` by consuming the helper.
- Hazards: alignment when acap=0 or ncap=0 (node-only/array-only buffers); hand-rolled `Drop` must drop exactly the live `LuaValue`/`TableNode` (they hold GcRefs) and init every node slot to empty before any trace/read or `for_each_entry` walks uninit memory (UB -> tracegc/gengc).
- Gate: own commit, separate from B (so an over-collection regression is attributable). `make test` 33/33 with nextvar + tracegc as canaries; dhat confirms 3->2 / 2->1; binary_trees + table_hash_pressure wall NEUTRAL-or-better best-of-5 (>2-3% regression = inline trap again, revert); RSS down-not-up.

### F. Two-white-color flip (risky)

Replace single `White` with `White0|White1` + a `current_white` bit so color is the cross-cycle source of truth, deleting the `visited` HashSet and the reset-to-white walk. Pure mark-phase CPU; **does NOT move RSS** (off the headline target), and the collector is incremental so the reset-walk deletion is a partial CPU win, not full.

- **Blocked dependency (not optional):** the codebase comment at heap.rs:357-365 states `visited` exists *because* off-chain `new_uncollected` boxes carry stale color. Long strings are reclaimed purely via `!marker.is_visited(id)` (state.rs:3225-3238) and never enter sweep; `visited_count()` is a monotonic fixed-point counter in ~5 ephemeron/thread convergence loops (state.rs:3011/3020, 3170/3184, 3196/3210, 3365/3379, 3502/3516). Deleting `visited` is INCORRECT until `new_uncollected` is retired (all boxes on-chain) or `visited` is kept exclusively for off-chain objects while color dedups on-chain ones.
- Gate (cheapest first): `cargo test -p lua-gc` with a NEW two-consecutive-full_collect test over BOTH an on-chain survivor chain AND an off-chain `new_uncollected` object + orphans (single-cycle tests miss the stale-color survivor UAF); then `./harness/canaries/gc/run_gc_canaries.sh`; then `make test` 33/33 with **tracegc the canary** (weak/finalizer reachability via the is_visited->color predicate), then gc/gengc/nextvar. Wall on binary_trees/table_hash_pressure must show >=-5% best-of-5 (sub-3% noise = revert; not worth the correctness surface). Failure mode is silent UAF, so revert on any conformance drop, do not re-run. Do NOT bundle with B.

### G / H — see §5 for sequencing; build only after their prerequisites plateau.

## 4. Recommended implementation sequence

Build and measure in this exact order. Each step is its own commit; never bundle B with E or F (an over-collection or mark-CPU regression must be attributable).

**Step 1 — A (constructor arg-swap).** Highest value/lowest risk: a real bug, no hot-path, no struct change, less allocation.
- Change: swap the two args at crates/lua-vm/src/vm.rs:1741.
- Measure: `make test`; `lua-cli --features dhat-heap` on a pure-record script; `harness/bench/compare_luars.sh`.
- KEEP iff: 33/33 with nextvar green; dhat pure-record drops 3->2 mallocs; binary_trees RSS down (even 10-20% is real on record-heavy benches), binary_trees/gc_pressure/table_hash_pressure wall within +/-2-3% noise. REVERT iff: any conformance drop, or wall regresses past noise.

**Step 2 — B (charge buffer bytes), with C as its tuning surface.** The headline RSS lever. Land B's symmetric charge/refund + the uncollected-box guard, then sweep C's two constants in the same measurement session.
- Change: `Cell<usize>` header.size + `charge_extra`/`uncharge_extra` + `resize_accounting`; charge at table.rs:606, table.rs:561-563, string.rs:17; raise `GC_MIN_THRESHOLD` -> ~1MB; `pause_multiplier` knob.
- Measure: new `lua-gc` baseline-return test; `cargo test -p lua-gc`; `make test`; `compare_luars.sh` sweeping {floor 256K,1M} x {pause 150,200,300}; gc_pressure at 10x iters.
- KEEP iff: 33/33 (gc/gengc count() correctness, tracegc, nextvar green); binary_trees RSS <=120MB (target 67-100), table_hash_pressure <=80MB; binary_trees wall <=2.85x C, gc_pressure wall <=2.1x C; dhat count unchanged; 10x-iter RSS plateaus. REVERT iff: RSS moves <2x (accounting did not reach the buffers), or any >5% gc_pressure wall regression (#44 trap), or count() conformance breaks.

**Step 3 — D (table free-list).** Pure alloc-throughput, stacks with mimalloc, zero hot-path/struct cost. Lands after B so the RSS picture is already correct and D's effect is isolated to wall.
- Change: bounded `table_freelist` + recycle on sweep / pop on allocate; header re-init.
- Measure: rung-2 pointer-reuse + header-re-init unit test FIRST; `make test`; `compare_luars.sh`.
- KEEP iff: 33/33 (gc/gengc green); unit test shows pointer reuse + byte-identical re-init; binary_trees wall improves >=3%; gc_pressure no regression; binary_trees RSS flat-or-better. REVERT iff: <3% wall win, or any RSS regression, or conformance drop.

**Step 4 — E (joint TableBuf), amended layout only.** Take only if A+B+D leave a wall gap on table-heavy benches. Build with cached `node_ptr` (4-word struct), never the 2-word `{ptr,acap,ncap}`. Own commit.
- Measure: focused table get/set/resize unit test + `cargo test -p lua-gc`/lua-types FIRST; dhat (3->2 / 2->1); `make test` (nextvar + tracegc canaries); `compare_luars.sh`.
- KEEP iff: 33/33; dhat count drops; binary_trees + table_hash_pressure wall NEUTRAL-or-better best-of-5; RSS down-not-up. REVERT iff: >2-3% wall regression on either bench (the inline trap recurring), or any conformance drop.

**Step 5 — F (two-white flip), only after retiring new_uncollected.** Mark-CPU only, does not touch the RSS target, highest correctness surface. Do the `new_uncollected` retirement (all boxes on-chain) or the off-chain/on-chain hybrid FIRST, then the flip behind the two-cycle on-chain+off-chain unit test.
- KEEP iff: 33/33 (tracegc the canary), gc canaries green, binary_trees/table_hash_pressure wall >=-5% best-of-5. REVERT iff: sub-3% wall win (not worth the surface) or ANY conformance drop (silent-UAF failure mode — revert, do not re-run).

## 5. What to NOT do

- **Do NOT inline tiny-table array/node storage (SmallVec).** Already measured 20-35% slower at every size. A table is accessed >> allocated; a per-access inline/spill branch + bigger `LuaTable`/`GcBox` + spill-transition swap-memcpy cost more than the mallocs saved. Any new proposal with this shape is rejected on sight.

- **Do NOT ship the joint TableBuf with the `{ptr, acap, ncap}` layout (E unamended).** It recomputes the node base (load+multiply+align_up+add) on every node access — the same per-access tax that killed SmallVec. Cache `node_ptr` or do not build it.

- **Do NOT build the fused GcBox+buffer DST (I).** `Gc<LuaTable>` is a raw `NonNull` into the box (heap.rs:164-165); growing the trailing buffer reallocs the whole box and invalidates every live handle (UAF, not a clean assert). It only works for never-resizing tables, needs a full split-buffer fallback maintained alongside, re-treads the inline-storage access-path tax, and saves only 1 marginal malloc on top of E. Reject; prefer B+E.

- **Do NOT delete the `visited` HashSet (F) before retiring `new_uncollected`.** Long-string reclaim (state.rs:3225-3238) and ~5 ephemeron/thread fixed-point loops depend on `is_visited`/`visited_count` for off-chain objects whose per-object color is never reset. A color-only predicate mis-handles them -> silent leak or live-object over-collection. The enabling refactor is mandatory, not "best paired."

- **Do NOT charge buffer bytes (B) without raising `GC_MIN_THRESHOLD` and sweeping `pause_multiplier` (C).** Against honest bytes the 256KB floor becomes a frequent trip-wire and `pause%` applies to a 5-7x larger base — that is the #44 over-collection wall regression. C is a mandatory companion, not optional.

- **Do NOT bundle B with E or F in one commit.** B has independent throughput risk (#44, RSS signal), E has hot-path risk (wall signal), F has mark-CPU risk (wall signal) and a different oracle. Bundling makes a regression unattributable and confounds the measurement that settles each.

- **Do NOT build the paged size-classed pool (H) first.** It is the luars-parity RSS endgame but does NOT charge buffer bytes by itself — building it before B optimizes density before the accounting bug that actually balloons RSS is fixed, and RSS would still be dominated by uncharged/un-pooled backings. High unsafe surface (type-erased fat-pointer -> size-class recovery, Drop-before-reuse, finalizer ordering) caught only probabilistically by the suite. Defer until A/B/D/E plateau; when built, use address-masking for page lookup (not a stored back-pointer, which is a per-GcBox struct tax) and keep accounting decoupled.

- **Do NOT generalize recycling to strings/closures (G) against the current target.** binary_trees/table_hash_pressure are table-dominated, so G is near-orthogonal to the measured gap; free-lists also RETAIN memory and can push RSS UP. Build only after D proves out, behind a hard RSS-non-regression gate, and only when a string/closure-heavy workload is the actual target.

- **Do NOT treat the constructor sizing as a no-op (the original brief's conclusion).** It was empirically falsified — a pure record is 3 mallocs, not 2, because of the arg swap. That is intervention A, build-first, not a close-as-no-op.