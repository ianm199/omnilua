# #113 size-class candidate ranking (measure-first, read-only) — 2026-07-15

Analysis-only packet for issue #113 (RSS object diet). **No collector or
runtime source was modified and no object layout was changed.** The
deliverables are a new measurement tool and this ranked go/no-go verdict.
Rev-2: revised after codex R1 (verbatim in
`PERF_113_SIZE_CLASS_ANALYSIS_20260715_REVIEW.md`; responses at the end here).

## Why this packet exists

The #113 arc's two banked lessons (`docs/PERF_EVIDENCE_113_W2_OWNERVEC_20260713.md`,
`docs/PERFORMANCE_PRINCIPLES.md` "Patterns from the owner-vector negative"):

1. **Relocation is not removal.** Wave 2 moved the 16 B intrusive link into an
   owner vector; on 64-bit that slot is a 16 B fat pointer — the same 16 B —
   plus slack, so RSS went *up* on binarytrees. Closed unmerged.
2. **Allocator size-class crossings are the real RSS mechanism.** Every RSS win
   in the arc came from a malloc bucket boundary (UpVal 104→72, GcBox<UpVal>
   56→40 in the W2 branch), never from raw byte counts. A shrink that stays
   inside one malloc bucket buys nothing; a shrink that crosses a boundary
   avoids the whole class step × the live population.

So before any more layout surgery: dump the live-object size histogram against
the platform allocator's real class table and rank shrink candidates by
**(crosses-a-bucket) × (bytes-saved-per-crossing) × (peak population)**.

## The tool

`crates/lua-rs-runtime/examples/size_class_histogram.rs`, driven by
`harness/size_class/run.sh`. Per bench workload, in its own process:

- A `#[global_allocator]` maintains a live **exact-byte** size-histogram (one
  bucket per request size, not an 8-byte range) and snapshots the whole
  histogram at the instant of peak live *requested* bytes. The hook allocates
  nothing, so it is reentrancy-safe.
- The workload runs through the real omniLua VM (`Lua::load().exec()`).
- It prints: the **authoritative** macOS libmalloc class table via
  `malloc_good_size` (probed live, not hardcoded); `size_of::<GcBox<T>>()` for
  every core GC-boxed payload type with the class it lands in, the next-smaller
  class cap, the bytes to remove to cross, and the allocator-slot bytes
  reclaimed by crossing; and the peak-moment histogram annotated with
  populations.

Raw output for every workload is under `harness/size_class/out/`.

### What the tool does and does NOT establish (measurement honesty)

- **It measures request sizes, not types.** Attribution to a `GcBox<T>` is by
  size: a bucket = all allocations of exactly that many bytes. For the two
  ranking candidates the bucket is *exclusive to one GcBox type* (see below),
  so the count is a tight upper bound; workload structure supplies the type.
  For the 48-byte bucket, `GcBox<LuaLClosure>` and `GcBox<LuaString>` genuinely
  share it — that count is a sum. Exact per-type population would need typed
  heap counters (out of scope for a read-only tool that adds no collector code).
- **The peak snapshot is peak requested `Layout::size()` bytes**, not rounded
  allocator bytes, not per-type population, and **not process max-RSS**. It is a
  peak-*heap-demand* proxy, useful for ranking populations; it is not an RSS
  measurement. Any RSS claim below is a *mechanism projection* to be confirmed
  by interleaved max-RSS A/B per `docs/MEASUREMENT_PROTOCOL.md`.
- **A single run is not the benchmark run.** `harness/bench/compare.sh` repeats
  short workloads inside one VM (closure_ops ×3, binarytrees ×2,
  table_hash_pressure ×9); the tool runs each once. Repetition shifts GC timing,
  allocator retention, and max-RSS, so the tool's peak is a per-invocation
  population estimate, not the benchmark's peak.

## macOS libmalloc class table (this machine, `malloc_good_size`)

Probed every 8 bytes on this machine (M-series, macOS 24.3). For the range that
matters here — allocations ≤ 240 B — libmalloc rounds to **16-byte quanta**:
`good(40)=48, good(48)=48, good(56)=64, good(64)=64, good(72)=80, good(80)=80,
good(88)=96, good(96)=96, good(104)=112, good(232)=240`. Independently
cross-checked with a standalone C program calling `malloc_good_size` — identical.

This is scoped to the ≤240 B candidates below; it is **not** presented as a
general platform model (libmalloc has nano/tiny/small/medium zones with
different quanta and region/page pooling above this range). `malloc_good_size`
reports the block size a request is served from — i.e. **allocator-slot bytes**,
not guaranteed RSS: reduced slot demand lowers RSS only insofar as it changes
region/page occupancy or the peak. The tool re-derives this table on whatever
machine it runs, so the boundary math is never a stale hardcode.

## GcBox<T> at the current tree (GcHeader = 24 B, Wave 1 landed)

`GcHeader` is 24 B: `color`(1) + `age`(1) + `flags`(1) + pad(1) + `size:u32`(4)
+ `next`(16, a `dyn Trace` fat pointer). `GcBox<T> = GcHeader + T`, 8-aligned.
Measured `size_of` and class placement (layout is workload-independent; only
population varies):

| GcBox\<T\>          | payload | GcBox | class | next-smaller cap | slack | to cross | peak pop (workload) | bucket exclusive? |
|---|---:|---:|---:|---:|---:|---:|---|---|
| **UpVal**           | 32 | **56** | **64** | 48 | **8** | **8** | **99,985 (closure_ops)** | **yes (only 56 B GcBox)** |
| LuaLClosure         | 24 | 48 | 48 | 32 | 0 | 16 | ~100,000 (closure_ops) | no (shares 48 with LuaString) |
| LuaString           | 24 | 48 | 48 | 32 | 0 | 16 | 165,753 (table_hash_pressure) | no (shares 48) |
| **LuaTable**        | 72 | **96** | **96** | 80 | 0 | 16 | **131,329 (binarytrees)** | **yes (only 96 B GcBox)** |
| LuaCClosure         | 40 | 64 | 64 | 48 | 0 | 16 | ~19 | no |
| LuaUserData         | 88 | 112 | 112 | 96 | 0 | 16 | 3 | — |
| LuaProto            | 208 | 232 | 240 | 224 | 8 | 8 | 3 | — |
| LuaThread           | 8 | 32 | 32 | 16 | 0 | 16 | 1 (real) | no |
| LuaState            | 176 | 200 | 208 | 192 | 8 | 8 | not established | — |

`slack` = class cap − GcBox size (bytes already wasted to rounding). `to cross`
= GcBox size − next-smaller cap (payload bytes to remove to drop into the
cheaper class). Crossing reclaims 16 allocator-slot bytes/object.

**Bucket-exclusivity check (from the exact-byte histogram, all 7 workloads):**
size 56 has a population ≥ 8 only in closure_ops (99,985; ≤ 7 everywhere else)
and `GcBox<UpVal>` is the only 56 B GcBox — so that count is UpVal. Size 96 has
scale only in binarytrees (131,329) and `GcBox<LuaTable>` is the only 96 B
GcBox — that count is tables (± a handful of background tables, ~20). Size 48 is
shared: closure_ops 100,194 (closure-dominated), table_hash_pressure 165,753
(string-dominated) — a sum of two GcBox types plus any 48 B buffer.
`LuaThread` (size 32) and `LuaState` (size 200) rows are `size_of` facts, **not
live counts**: the 32 B and 200 B buckets hold unrelated allocations, real
`LuaThread`/`LuaState` GcBox populations here are ~0–1 and unestablished.

## Per-candidate verdicts

### 1. GcBox\<UpVal\>  →  **WORTH-IT** (the one qualifying candidate — but needs a representation redesign, not a field cast)

- **State:** 56 B, sits in the 64 B class wasting **8 B of slack** — the *only*
  high-population GcBox with allocator slack. Population **99,985** on
  closure_ops (a named #113 tall pole still above the ≤2.0× done-condition), and
  the 56 B bucket is exclusive to UpVal, so the count is tight.
- **The crossing:** shrink UpVal 32 → 24 B ⇒ GcBox 56 → 48 B ⇒ crosses the 64 B
  class into the 48 B class (`good(56)=64`, `good(48)=48`, verified). This is
  the *same* 48 B class the killed Wave-2 header diet would have reached (its
  "56→40", since `good(40)=48`) — but via a payload shrink, with no header
  surgery and no owner-vector relocation cost.
- **The proposed field cast is UNSOUND — corrected.** UpVal is
  `open_thread_id: Cell<i64>`(8) + `open_idx: Cell<u32>`(4) + pad(4) +
  `closed_value: Cell<LuaValue>`(16) = 32 B, and `open_thread_id` doubles as the
  open/closed discriminant (`-1` = closed). My rev-1 proposal — retag it
  `Cell<u32>` with a `u32::MAX` sentinel — is **not safe**: thread ids come from
  `GlobalState.next_thread_id`, a **monotonic u64 counter that is never reused**
  (`crates/lua-vm/src/state.rs`, `g.next_thread_id += 1`). u32::MAX is reachable
  after ~4 billion *lifetime* thread creations (not 4 billion live coroutines),
  after which a cast would alias ids or collide with the sentinel. Retracted.
- **The sound way to reach 24 B (for the implement lane):** collapse the three
  cells into one tagged cell that **preserves the full u64 id domain** by
  overlapping the open and closed storage:
  ```rust
  enum UpValInner { Open { thread_id: u64, idx: u32 }, Closed(LuaValue) }
  struct UpVal { state: Cell<UpValInner> }
  ```
  Open (`u64`+`u32` = 12 B) fits inside the 16 B `Closed(LuaValue)` slot, so the
  union is 16 B; with the discriminant `UpValInner` is ≤ 24 B (16 if a `LuaValue`
  niche is found, 24 otherwise) — either lands GcBox in the 48 B class
  (`good(40)=good(48)=48`). No id-domain narrowing. A fallback is an explicit
  **checked** global `u32` id domain with defined exhaustion behavior, but the
  enum is cleaner and keeps semantics identical.
- **Load-bearing caveat (hot path):** the current all-`Cell` layout exists so
  `upvalue_get`/`upvalue_set` read `open_thread_id` with a single load and zero
  borrow-guard cost — the dominant cost in fibonacci-class recursion. A
  `Cell<enum>` read copies/matches the whole 16–24 B value. So this candidate is
  **WORTH-IT on RSS but gated on the perf-arbiter**: it must clear a hot-path Ir
  check (fibonacci/closure_ops, per MEASUREMENT_PROTOCOL) and a wasm32 layout
  check before landing. If it regresses hot-path Ir, it does not ship.
- **Payoff (mechanism projection, upper bound):** 16 allocator-slot bytes ×
  99,985 = **1,599,760 B ≈ 1.53 MiB** of avoided slot demand on closure_ops.
  Against the historical 25–37 MB process max-RSS that is ≈ **4–6 %**. Whether
  that slot saving materializes as RSS, and whether the pacer cadence shifts
  (the payload delta is −8 B × 100k ≈ −0.8 MB of *charged* bytes, ≈ 5 % of the
  requested-byte peak — enough to potentially cross a nonlinear pacing
  threshold, so cadence effects must be measured, not assumed), is settled only
  by interleaved max-RSS + heap-diff A/B. The June UpVal diet (104→72) is the
  precedent that a same-magnitude change translated to real RSS (−8.3 %).

### 2. GcBox\<LuaLClosure\> and GcBox\<LuaString\>  →  **NOT-WORTH-IT via simple field removal** (structural change is a separate track)

Both 48 B, both **fill the 48 B class exactly (0 slack)**; crossing to the 32 B
class needs payload 24 → 8 B. LuaLClosure = `proto: GcRef`(8) + `upvals:
Box<[…]>`(16); LuaString = `bytes: Box<[u8]>`(16) + `is_short` + `hash`. In each
the 16 B owning fat pointer alone blocks an 8 B payload. **No simple field
removal crosses.** This is *not* "impossible" in principle — inline/trailing
storage, thin pointers, or co-allocation are structural representation projects
— but those are a different, larger track, not a header/field diet, and are not
justified by these two (LuaString's tall workload, table_hash_pressure, already
meets ≤2.0×).

### 3. GcBox\<LuaTable\>  →  **NO SIMPLE SAFE CROSSING; a bounded structural spike is defensible** (biggest population)

96 B, **fills the 96 B class exactly (0 slack)**, population **131,329** on
binarytrees — the largest GcBox population in the suite. Crossing to the 80 B
class needs payload 72 → 56 B (−16 B). LuaTable = `inner: RefCell<TableInner>`
(56 = 48 + 8 B borrow flag) + `metatable: Cell<Option<GcRef>>`(8) + `weak_mode:
Cell<u8>`(1, +7 pad). Findings:
- The only 16 B available is the RefCell borrow flag (8) + the weak_mode slot
  (8). Removing **only one** gets to 64 B, whose GcBox (88 B) still rounds to the
  96 B class (`good(88)=96`) — **partial removal buys nothing; it is
  all-or-nothing.**
- Reclaiming the borrow flag means replacing `RefCell` with `UnsafeCell`, which
  **conflicts with the crate's zero-`unsafe` budget** and deletes the runtime
  borrow guard on every table mutation — against omniLua's safe-Rust position.
- `weak_mode` **cannot** simply fold into `TableFlags`: that `u8` is full — lower
  7 bits are the fast-access metamethod cache, bit 7 is `BIT_RAS`
  (`crates/lua-types/src/table.rs`), and weak mode needs 2 bits (WEAK_KEYS,
  WEAK_VALUES). So the rev-1 "fold weak_mode into spare bits" was wrong.

Verdict: no *simple, safe* crossing exists. But given the 131k population, a
**bounded design spike** for a more structural table representation (a
safe ownership-swap that removes the borrow flag without `unsafe`, or relocating
`weak_mode` into `TableInner`'s existing padding + a safe cell discipline) is
defensible as a *separate* investigation — not a park-and-forget, and not this
packet's quick win.

### 4. Low-population group (LuaCClosure / LuaUserData / LuaProto / LuaState / LuaThread)  →  **FEASIBLE-BUT-IMMATERIAL**

Populations of ~0–19 at peak across every workload — even a free crossing saves
< 320 B total. Not all are infeasible (e.g. `LuaProto.cache`
`RefCell<Option<GcRef<_>>>` → `Cell<Option<GcRef<_>>>` would save 8 B and cross
232→224 into the 224 B class; `LuaCClosure`/`LuaUserData` fixed-length
`RefCell<Vec<_>>` could become boxed cells) — but at population 1–3 the payoff
is below the noise floor. Label: feasible, immaterial in the sampled workloads.

## Secondary findings (NOT GcBoxes — where the remaining RSS actually lives)

The tool surfaced that on the tall poles, **non-GcBox buffer allocations rival
or exceed the GcBox bytes**. These are outside this packet's GcBox<T> scope and
each is a larger representation change, but they are the real remaining levers:

- **binarytrees is node-buffer-bound, not table-box-bound.** Table hash-part
  buffers total **13,656,256 B ≈ 13.02 MiB** (65,652 leaf 1-node buffers at
  good-size 48 B = 3,151,296 B + 65,656 internal 4-node buffers at 160 B =
  10,504,960 B) vs **12,607,584 B ≈ 12.02 MiB** for the table boxes. `TableNode`
  is **40 B** (full `LuaValue` key(16) + value(16) + `next:i32`(4) + `dead:bool`)
  vs C's 32 B packed `key_tt`/`key_val` node. Packing TableNode 40 → 32 B would
  cross two buffer classes: the 1-node buffer 48 → 32 (65k × 16 = ~1 MiB) and
  the 4-node buffer 160 → 128 (65k × 32 = ~2 MiB) ≈ **~3 MiB on binarytrees** —
  the single biggest remaining binarytrees lever. Known candidate-9 follow-on; a
  value-representation change, not a header/box shrink.
- **closure_ops pays ~100k × 16 B (≈ 1.6 MiB) for len-1 upval-slot boxes.** Each
  `LuaLClosure` allocates a separate `Box<[Cell<GcRef<UpVal>>]>` of length 1
  (8 B request → good-size 16 B; 99,999 live at peak). An inline-small-upvals
  optimization would delete these but enlarges every closure box — a bigger
  redesign; separate issue.
- **concat_chain, string_format_mixed, gc_pressure, fibonacci are not RSS tall
  poles** (peak live heap 0.02–0.06 MiB). Their >1.9× RSS ratios come from
  process/interner baseline, not a populous shrinkable object — no object-diet
  lever applies. This covers all five #113 done-condition rows: only closure_ops
  carries a feasible GcBox crossing.

## Verdict / recommendation to the supervisor

- **Implement candidate 1 (UpVal → single tagged `Cell` preserving the u64
  id domain), gated on the perf-arbiter.** It is the only GcBox<T> shrink that
  crosses a real libmalloc class boundary at a large, cleanly-attributed
  population (≈ 100k on a live tall pole). Do **not** ship the u32-cast form
  (unsound). Ship the enum/checked-domain form only if it (a) reaches the 48 B
  class, (b) holds hot-path Ir on fibonacci/closure_ops, (c) is confirmed by
  interleaved max-RSS + heap-diff A/B (projected ≈ 1.5 MiB / ~4–6 % on
  closure_ops), and (d) passes oracle + GC canaries + wasm layout. If any gate
  fails, it does not ship — the W2 precedent (measured-negative → close) applies.
- **Triage the rest, do not park collectively.** After candidate 1:
  - *Structurally infeasible by simple diet:* LuaLClosure, LuaString (fill their
    class; only a structural representation project could move them — separate,
    larger track, not justified now).
  - *Defensible bounded spike:* LuaTable — largest population, no simple safe
    crossing, but a structural safe representation deserves its own scoped
    investigation given 131k live.
  - *Feasible-but-immaterial:* the low-population group (payoff below noise).
  - *The real remaining RSS lever is buffer representation* (TableNode 40→32,
    inline upvals) — a different track from header/box diets; file it as such.

## Adversarial review (codex R1) — responses

Full R1 verbatim in `PERF_113_SIZE_CLASS_ANALYSIS_20260715_REVIEW.md`
(VERDICT: REVISE). Disposition:

1. **UpVal u32 cast unsound (blocking)** — **FIX.** Confirmed at source
   (`next_thread_id` monotonic u64, never reused). Retracted the cast; candidate
   1 now specifies a tagged-`Cell` representation preserving the u64 domain (or a
   checked u32 domain), gated on hot-path Ir + wasm. WORTH-IT stands on the
   crossing opportunity, not the specific broken change.
2. **Histogram not typed/exact (blocking)** — **FIX.** Tool switched to
   exact-byte buckets (was 8-byte ranges); added an explicit measurement-honesty
   section and per-candidate bucket-exclusivity check. "Exact populations"
   softened to "strongly corroborated by workload structure + exclusive bucket";
   "peak-RSS proxy" softened to "peak heap-demand proxy, not RSS."
3. **Sampled run ≠ benchmark run** — **FIX/ACKNOWLEDGE.** Documented the
   compare.sh repetition mismatch; added concat_chain + string_format_mixed to
   the default set so all five done-condition rows are covered (both confirmed
   to hold nothing at scale). All RSS numbers labeled projection-pending-A/B.
4. **Allocator model overreach** — **FIX.** "RSS bytes reclaimed" → "allocator-
   slot bytes avoided"; 1.53 MiB framed as upper-bound mechanism projection;
   removed the "cadence barely shifts" claim (now: must be measured); arithmetic
   fixed to 4–6 %; the class-table claim scoped to ≤240 B on this machine, not a
   general platform model.
5. **Candidate labels too categorical** — **FIX.** LuaLClosure/LuaString
   relabeled "no simple field removal; structural track separate"; LuaTable
   "no simple safe crossing; bounded spike defensible" with the TableFlags
   spare-bits error corrected; low-pop group "feasible-but-immaterial" with the
   LuaProto.cache 8 B crossing noted.
6. **LuaState population "1" unestablished** — **FIX.** Removed; LuaState/
   LuaThread rows now flagged as `size_of` facts, not live counts.
7. **13.6 vs 13.02 MiB decimal error** — **FIX.** Recomputed from exact bytes:
   node buffers 13,656,256 B = 13.02 MiB; table boxes 12,607,584 B = 12.02 MiB.

## Reproduce

```bash
harness/size_class/run.sh                       # default #113 five-row + controls → harness/size_class/out/
harness/size_class/run.sh closure_ops binarytrees
cargo run -q --release -p omnilua --example size_class_histogram -- \
    harness/bench/workloads/closure_ops.lua
```
