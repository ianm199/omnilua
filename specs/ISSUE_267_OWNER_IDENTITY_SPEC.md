# Issue #267 — GC owner identity for stale-handle safety: design + options

> **SUPERSEDED IN PART (2026-07-15).** The cheap parts of the recommendation
> shipped in PR #294 as a *partial* mitigation: the **no-guard deref-free
> `downgrade`/`account_buffer` guards** (which close F1/F3/F4 in release).
> **The `HDR_FREED`/`owner_gen` tripwire this spec recommends below (see the
> "recommended option" and the C/D sections) was IMPLEMENTED, REVIEWED, and
> WITHDRAWN as UNSOUND** — it reads a foreign/freed box header *inside* the
> safety check, which is itself a use-after-free (worse than the bug it aimed
> to catch), and its u8 owner-generation wraps/collides. **Do not
> re-implement it.** The foreign-heap, stale-after-sweep, and
> issued-`&T`-across-`drop_all` cases genuinely require **option B**
> (slot-indexed handles / an exclusive-teardown API) because reading the box
> IS the UAF for them — tracked as **#295**. Treat every `HDR_FREED`/
> `owner_gen`/per-box-owner-tag recommendation below as REJECTED history.

**Status:** design spec, no production code. Recommends a direction; the load-bearing
choice is the maintainer's because it is a soundness-vs-per-object-cost /
contract-vs-mechanism trade-off. Revised once after an adversarial codex review
(verdict recorded verbatim in `ISSUE_267_OWNER_IDENTITY_SPEC_REVIEW.md`); this revision
incorporates its findings (byte ledger corrected for wasm32 and the W2 target; option B
separated from historical W2; the debug-deref exception removed; the failure surface
widened; the test plan realigned to what each option actually guarantees).

**Scope of writes for the implementing packet:** `crates/lua-types/src/gc.rs`
(`GcRef::downgrade`, `GcRef::account_buffer`, `GcWeak`) and `crates/lua-gc/src/heap.rs`
(`GcHeader`, `Heap` construction, the token machinery). No behavior change to the
bytecode dispatch loop or any hot deref path is contemplated by the recommended
direction.

---

## 0. TL;DR

`Gc<T>`/`GcRef<T>` is a bare `Copy` pointer that carries no owner-heap identity. Operations
on a **stale** handle (one whose box has been freed — because its owning `Heap` was
dropped/closed, because it was swept while unrooted, or because it is operated under a
*different* live heap) are currently unsound: `downgrade` (three distinct bugs, one of them
**same-heap with no close involved**) and `account_buffer`. Plain deref of a stale handle —
and every collector method that reads the box — is the older baseline UB. #260's closed-heap
gating already covers every **in-tree** path *it targets*; the residual gaps are (i)
host/embedding code that holds a `GcRef` across `close`/`drop` or runs two heaps on one
thread, **and (ii) a `GcRef` that is swept while unrooted on its own still-open heap and then
re-`downgrade`d** — the lazy token map re-validates the freed address (F2c below; the repo's
own `registering_after_sweep_yields_a_distinct_token` test, `heap.rs:3721`, is the mechanism).

Two premises in the issue framing need correction against the code before the trade-off
is legible:

1. **The shipped `GcHeader` is 24 B on 64-bit / 16 B on wasm32, not 8 B.** The 8 B figure
   is the #113 W2 target, which was **measured-negative and closed unmerged** (only W1
   landed — see `docs/archive/PERF_EVIDENCE_113_W2_OWNERVEC_20260713.md`). The current 24 B layout
   has **one spare padding byte**, so a **u8 owner tag is free** and a **u32 owner-id costs
   +8 B/box** on 64-bit (24→32; +4 B on wasm32, 16→20). `size_of` verified in §7.
2. **"A slot-index handle resurrects W2, therefore it is measured-negative"** is not a
   valid transfer of evidence. Historical W2 kept **raw-pointer** `Gc` handles and
   relocated the *owner-list* pointer into a heap-side vector; its RSS loss came from that
   16 B/slot relocation plus vector slack, **not** from a handle indirection. A
   slot-*indexed* `Gc` (option B) is a different, unmeasured architecture.

**The structural fact that governs everything:** a freed box's fields are unreadable, and
a bare `Copy` `Gc` carries no identity of its own. So the *freed-box* cases (a stale
handle whose heap already tore the box down) cannot be validated in release by any
per-*box* field — reading that field is the very deref that faults — and can only be
closed by putting identity *in the handle* (fat handle, or slot-indexed handle: option B).

**A "documented contract" is not, by itself, a valid resolution** (codex finding 2). `Gc`,
`GcRef`, `Deref → &T`, `Gc::account_buffer`, and `Heap::drop_all(&self)` are all **safe**
functions, and `drop_all` takes `&self` (`heap.rs:3174`) — so a purely-safe caller can hold a
handle (or an already-issued `&T`), call `drop_all(&self)`, and then use it. Documentation
cannot make UB reachable from exclusively-safe code acceptable. So the maintainer's decision
is not "contract vs bytes"; it is a **safety-boundary** decision (§5). The embedding facade
already shows the correct shape: embedders go through `RootedValue` (owns a `Lua`, roots via a
slab key, checks staleness → `stale_handle_error`, `lib.rs:2033`), and raw `Gc`/`GcRef`/`Heap`
are **not re-exported through it** — they are `pub` only at the `lua-gc`/`lua-types` crate
level.

**Recommended direction, in independently-landable pieces:**

- **Now, unconditionally:** make the *no-guard* `downgrade`/`account_buffer` paths
  **deref-free** (return a permanently-dead weak / no-op, or panic — but never dereference
  the box to decide). Removes the two "UAF-in-the-check" bugs (F1, F3-no-guard) mechanically,
  in every build, at **zero per-object cost**. This is bug removal, *not* "owner-identity
  soundness."
- **Now, free, and higher-value than the owner tag:** add a **quarantine tripwire in
  `downgrade`** — before minting a token, in quarantine mode read the (parked, therefore
  safe) box header and refuse/panic if `HDR_FREED` is set. This catches **F1, F2b, and F2c**
  (every *freed/swept* box), including the same-heap F2c the owner tag cannot see.
- **Optional, free:** the u8 heap-generation tag (option D) — a *complementary* quarantine
  tripwire that additionally catches the *live-box foreign* case (F2a), which `HDR_FREED`
  cannot. Explicitly a tripwire, not a release guarantee; it does **not** catch F2c
  (owner matches).
- **Deferred, the maintainer's real decision:** whether stale-handle use must be
  mechanically safe **in release**. If yes, the honest options are to **seal the raw surface**
  (make raw `Gc`/`GcRef` ops `pub(crate)`/`unsafe` and route all embedders through the rooted
  `RootedValue`-style handle) and/or an **API-shape redesign** (exclusive/`&mut` teardown, or
  access guards instead of `&T`) — and, if arbitrary handle lifetimes must be safe,
  slot-indexed handles (option B), co-designed with the #113 owner-vector work and measured.
  Owner identity alone is **necessary but not sufficient**: even option B leaves an
  already-issued `&T` danglable across a later safe `drop_all(&self)` (codex finding 3).

Do **not** ship a u32 per-box owner-id (option A): it costs +8 B/box (64-bit) and still
faults on the freed box, so it buys only *live-box foreign* detection — which the free u8 tag
gives in the mode (quarantine) where it is used — and it catches neither F2c nor any freed-box
case.

---

## 1. The machinery as it stands

File references are to this worktree (`origin/main`).

- **`Gc<T>`** (`heap.rs:841`) is `{ ptr: NonNull<GcBox<T>>, _marker }` — one machine word,
  `Copy + Clone`, carrying no heap identity. `GcRef<T>` (`gc.rs:23`) is a newtype over it.
- **`GcBox<T>`** (`heap.rs:826`, `#[repr(C)]`) is `{ header: GcHeader, value: T }`.
- **`GcHeader`** (`heap.rs:727`, `#[repr(C)]`) after the #113 W1 diet is
  `color(1) + age(1) + flags(1) + pad(1) + size:u32(4) + next: fat ptr(16)` = **24 B,
  align 8** on 64-bit (`gcheader_is_24_bytes_after_grayagain_diet`, `heap.rs:4272`); 16 B on
  wasm32. Flags: `HDR_FINALIZED/COLLECTED/GRAY_LISTED/FREED/HEAP_OWNED` (`heap.rs:759-773`).
- **Weak-handle validation** is the per-heap `allocation_tokens: IdentityHashMap<usize>`
  (`heap.rs:1480`), address → monotonic token. Tokens are minted **lazily** at `downgrade`
  time via `register_allocation_token` (`heap.rs:2284`), removed at sweep (`heap.rs:2844`),
  validated by `contains_allocation` (`heap.rs:2303`). The hot allocation path deliberately
  does **not** touch this map (the lazy-token win over the earlier eager scheme). The
  monotonic counter **wraps to 1 on overflow** (`next_token`, `heap.rs:1898-1902`) — so
  "never reissues a token" is true only until `usize` exhaustion (2⁶⁴ on 64-bit,
  practically unreachable; **2³² on wasm32**, reachable under pathological churn).
- **`HeapRef`** (`heap.rs:203`) is a `Weak<Heap>` (issue #252). A `GcWeak` stores one, so
  after its heap dies `contains_allocation` answers `false` via `Weak::upgrade() == None` —
  no deref of freed memory. **A correctly-minted `GcWeak` is already sound across heap
  death.** The hazard is concentrated at *mint* time (`downgrade`) and at deref of a stale
  strong handle.
- **`with_current_heap`** (`heap.rs:192`) returns the top of a thread-local stack of
  `Rc<Heap>` (`HeapGuard`, `heap.rs:120`). Multiple heaps on one thread is a supported
  embedding shape — this is what makes a *foreign* active heap reachable.
- **#260 closed-heap gating**: `drop_all` sets `closed` and clears the token map
  (`heap.rs:3191`, `3200`); `register_allocation_token` returns `0` (never-valid) when
  closed (`heap.rs:2285`); `contains_allocation` is unconditionally `false` when closed
  (`heap.rs:2304`); `assert_open` panics on allocation into a closed heap (`heap.rs:1720`).

---

## 2. Precise failure model

A **stale handle** is a `GcRef<T>`/`Gc<T>` whose box has been (or is about to be) freed by
its owning heap, held by code that outlives the heap or runs under a different heap. Line
references are to the current source. The full hazard axis is
`{live, swept, quarantined, closed, dropped} × {no-guard, owner-guard, foreign-guard} ×
{collected, bootstrap-uncollected, detached}`; the entries below are the reachable
unsound corners.

### F1 — no-guard `downgrade`: UAF *in the safety check itself*

`GcRef::downgrade` (`gc.rs:105`). With **no active `HeapGuard`**, `with_current_heap`
yields `None`, so `tracked = None`, and the guard clause runs
`tracked.is_none() && self.0.is_heap_owned()` (`gc.rs:113`). `is_heap_owned` (`heap.rs:952`)
→ `self.header()` → `as_box()` → `unsafe { self.ptr.as_ref() }` (`heap.rs:922`): a
**dereference of the box to read its flags byte**. If the owning heap has been dropped, the
box is freed — the check that exists to *prevent* misuse is itself a use-after-free.

- **Trigger:** host holds a `GcRef`; the guard is dropped (pop), *then* the last
  `Rc<Heap>` is dropped (`Drop for Heap` → `drop_all` frees the box, `heap.rs:3261`) — this
  ordering matters, because a live `HeapGuard` holds an `Rc<Heap>` and would keep the heap
  alive; no-guard is a precondition, not simultaneous. Then host calls `downgrade`.
- **Detection today:** under `LUA_RS_GC_QUARANTINE=1` *while the heap is still alive* the
  box is parked with `HDR_FREED` and the `debug_assert` in `as_box` fires. **Teardown
  frees quarantined boxes for real** (`drop_all` drains `quarantined`, `heap.rs:3208`), so
  **post-drop there is no tripwire** — plain UB.
- **#260 coverage:** none (no active guard).
- **Note:** F1 is the freed ∩ no-guard corner. A no-guard downgrade of a still-live box
  (guard merely popped) derefs safely and panics correctly (`gc.rs:274`); of a *detached*
  (never-freed) box, derefs safely and returns an always-upgrades weak.

### F2 — foreign-heap `downgrade`: token mint → resurrection

`downgrade` with a **different** live heap B active than the box's owner A.
`register_allocation_token(identity)` (`heap.rs:2284`) registers A's box address into **B's**
token map and mints a valid token; the `GcWeak` holds `heap = HeapRef(B)` and a valid
token. Later `upgrade` (`gc.rs:174`) → `B.contains_allocation(...)` → `true` → returns
`GcRef(self.target)`, a pointer into A's memory.

- The mint itself does **not** deref the box (address only; the `is_heap_owned` clause is
  short-circuited by `tracked.is_some()`). The unsoundness is the *resurrection*, deferred
  to the deref after `upgrade`.
- **F2a (box still live in A):** `upgrade` returns a currently-valid pointer, but B never
  sweeps A's box and A's later sweep removes only from A's map — B keeps the stale entry,
  so the handle resurrects the box after A frees it.
- **F2b (A already closed/dropped):** B's map holds a token for a freed address from the
  moment of `downgrade`; `upgrade` resurrects freed memory immediately.
- **#260 coverage:** none — the closed-check runs on B, which is **open**.

### F2c — same-heap, swept-then-re-`downgrade`: resurrection with no close, no foreign heap

The most in-tree-shaped variant, and the one no owner tag can catch. A `GcRef` local is
**not** a GC root. On a single still-open heap A with A's guard active:

1. allocate `g` on A;
2. `A.full_collect(roots)` where `roots` does not trace `g` → sweep frees `g`'s box and
   removes its token (`heap.rs:2844`);
3. `g.downgrade()` under A's guard → `register_allocation_token(identity)` (`heap.rs:2284`)
   finds the identity absent (sweep removed it) and **re-inserts it with a fresh valid
   token**; the weak then validates and `upgrade` hands back the freed box.

This is exactly the behaviour the repo's `registering_after_sweep_yields_a_distinct_token`
test pins (`heap.rs:3721`): after sweep, `register_allocation_token(id)` yields a token for
which `contains_allocation(id, token) == true`. The monotonic-token defense only protects a
weak handle minted **before** the sweep (it still holds the *old* token); it does nothing for
a strong handle **re-downgraded after** the sweep, which simply mints the new token. `F3c` is
the `account_buffer` analog (same-heap, swept target → `header()` derefs freed memory,
`heap.rs:993`).

- **Trigger:** an unrooted `GcRef` held across a collection — i.e. a rooting-discipline bug,
  the same class quarantine's `as_box` assert catches on *deref*. But `downgrade`'s guarded
  path does **not** deref, so quarantine misses it today and the box is silently resurrected.
- **#260 coverage:** none (A is open, not closed).
- **Option D coverage:** none — `box.owner_gen == A.gen` matches. Only an `HDR_FREED` read at
  downgrade (quarantine) or a per-slot generation (option B) detects it.

### F3 — `account_buffer` on a stale handle

`GcRef::account_buffer` (`gc.rs:142`).
- **No guard:** falls to the `None` arm and evaluates `self.0.is_heap_owned()`
  (`gc.rs:149`) — the **same UAF-in-the-check deref as F1**.
- **Foreign guard B:** runs `self.0.account_buffer(B, delta)` (`gc.rs:147`) → `self.header()`
  (`heap.rs:993`, deref → UAF if freed) and, if it reads `collected() == true`, charges the
  **wrong heap's** pacer (`heap.rs:1002`), drifting B's cadence.
- `account_buffer` *intrinsically* reads the box (it mutates `header.size`), so the
  foreign path cannot be made deref-free the way F1 can.
- **#260 coverage:** none.

### F4 — deref of a stale strong handle, generalized (baseline, not new)

The oldest hazard: **any** method that reads the box is UB on a stale handle. `as_box`
(`heap.rs:922`) is the choke point, reached by:
- `*gcref` / `as_ref` directly;
- `PartialEq for GcRef` value-eq arm (`gc.rs:322`, `Gc::ptr_eq(..) || **self == **other`);
- `Marker::mark` / `mark_box` on a supplied `Gc` (`heap.rs:1252`);
- the write barriers, which read *and mutate* the header and can splice a foreign pointer
  into heap-side state (`heap.rs:2319`);
- the color/age/finalized/tracking accessors (`heap.rs:956-978`);
- finalizer/registry move methods that take raw type-erased `NonNull<GcBox<dyn Trace>>`
  (`heap.rs:2092-2136`).
All are reachable in-tree **only** during collection with the owning heap active, so they
are not in-tree stale-handle hazards; they are listed because an owner-identity design that
claims to make stale handles *generally* safe must account for every one of them, not just
`downgrade`/`account_buffer`. Detection/UB status is as F1 (quarantine while alive; UB
post-drop). This predates every token/heap-ref mechanism ("unchanged from the pre-#260
world").

### F5 — `identity`/`ptr_eq`: memory-safe, not allocation-identity-safe

`identity` (`heap.rs:909`) and `ptr_eq` (`heap.rs:903`) read only pointer bits — never a
deref, so **memory-safe** on a stale handle. But after free + allocator reuse a *new*
object at the same address makes `ptr_eq == true` / `identity` collide (ABA). Any consumer
using `identity` as a durable object key across a possible free must not assume it names the
same allocation. This is not a UAF; it is a correctness caveat the owner-identity work
should note.

### Coverage summary

| Failure | Mechanism | Deref of freed box? | #260 covers in-tree? | Residual gap |
|---|---|---|---|---|
| F1 no-guard downgrade | `is_heap_owned()` in the check | **yes, in the check** | no (no guard) | host holds handle across drop |
| F2 foreign downgrade | token minted on wrong heap → `upgrade` | no at mint; UAF at later deref | no (foreign heap open) | two heaps on one thread |
| **F2c same-heap swept re-downgrade** | token re-minted for a freed address → `upgrade` | no at mint; UAF at later deref | **no (open heap)** | **unrooted handle across a collection** |
| F3 account_buffer | `is_heap_owned()` (no-guard) / `header()` (foreign/swept) | yes | no | host misuse / unrooted handle |
| F4 any stale deref (mark, barriers, accessors, …) | `as_box()` | yes | no (contract only) | fundamental to a bare handle |
| F5 identity/ptr_eq | pointer bits only | no (memory-safe) | n/a | ABA on allocator reuse |

**Load-bearing facts:** (1) F1–F4 all reduce to "you cannot safely read anything out of a
freed box, and a bare `Copy` `Gc` carries no identity." A fix must therefore either (i) put
identity *in the handle* (fat / slot-indexed), (ii) keep the box readable (quarantine) or
alive (contract/rooting), or (iii) stop the operations that read a *freed* box from doing so
(F1/F3-no-guard, cheaply). A per-*box* owner field does **not** fix F1/F2b/F2c/F4 — reading it
is the deref that faults. (2) The lazy token scheme has an intrinsic resurrection hole
independent of owner identity: `register_allocation_token` re-mints a valid token for any
identity it is handed, so **downgrading a strong handle after its box is gone always
validates** (F2c). Owner identity narrows *which heap* a handle belongs to; it does not fix
"this address was already freed on the right heap." (3) The safe API shape is itself part of
the bug: `Deref → &T` plus `drop_all(&self)` means even perfect owner identity cannot stop an
already-issued `&T` from dangling across a teardown — see §5.

---

## 3. Design options

Scored against the **actual** 24 B / 16 B header (§7 has compiled `size_of`). Allocator
size-class and RSS consequences are labelled **hypotheses** — `size_of` proves requested
layout only; whether a growth crosses a live-workload malloc bucket needs the heap-diff /
peak-RSS measurement in §6, not a probe.

### (A) Per-box owner-id in the header

Add `owner: Cell<u32>` (a per-heap generation/id); at `downgrade`/`account_buffer` compare
the active heap's id with `box.owner`; mismatch → fail safe.

- **Byte cost (measured, §7):** u32 → header **24→32** on 64-bit (+8), **16→20** on wasm32
  (+4); the added word cannot use the offset-3 pad because it needs 4-byte alignment. Only
  a **u8** id is free (24→24 / 16→16). *Hypothesis to measure:* +8 B pushes common boxes
  (`GcBox<[u8;16]>` 40→48, `GcBox<[u8;40]>` 64→72 by `size_of`) into larger malloc buckets
  — the size-class harm W1 fought; requires heap/RSS evidence to assert. Note the +8 is not
  universal: a header growth does not force a box growth for a `T` whose alignment already
  rounds the value offset past the added word (e.g. a 16-aligned `T` sits at offset 32 either
  way), so the real cost is the live type-size histogram, not a per-type constant.
- **Benefit:** O(1) foreign detection for a **live** box, *in release* (a real benefit D
  lacks — D detects it only in debug). Does not catch F2c (owner matches).
- **Limitation:** reading `box.owner` is a **deref**, so it does **not** fix F1, F2b, F2c, or
  F4 (freed/swept box). It catches only F2a. If the id is presented as a *guarantee*, it also
  needs a non-wrapping / exhaustion rule.
- **Verdict:** not recommended. Spending +8 B/box (64-bit) to catch the single sub-case
  (F2a) that the *free* u8 tag catches in quarantine is a poor trade; it leaves the
  worst-case (freed box) exactly as unsound.

### (B) Slot-indexed handle (a new design — NOT historical W2)

Make `Gc<T>` carry `{ slot: u32, gen: u32 }` (still 8 B) instead of a raw pointer; the heap
owns a slot table `slots: Vec<Option<NonNull<GcBox>>>`; a global/generational registry maps
`gen` → heap. `downgrade`/`upgrade`/`account_buffer` — and crucially `deref` — resolve
`slots[slot]` after validating `gen`, **without ever touching a freed box**.

- **This is the only option that closes F1, F2, F2c, F3 at the *operation entry* in
  release**, because owner identity lives in the handle and validation never dereferences the
  box (per-slot generation also detects the F2c swept-then-reused slot). **But it still does
  not fully close F4** (codex finding 3): `Deref::deref` returns `&T`, and a later safe
  `drop_all(&self)` or collection can free the box while that borrow is live. Closing that
  needs an API-shape change orthogonal to owner identity — tying teardown/collection to an
  exclusive/`&mut` lifetime, returning an access guard instead of `&T`, pinning/rooting for
  the borrow's extent, or removing safe `Deref`.
- **This is not the W2 architecture and W2's RSS number does not apply.** W2
  (`docs/archive/PERF_EVIDENCE_113_W2_OWNERVEC_20260713.md`) kept raw-pointer `Gc` handles and
  relocated the *owner-list* pointer into a 16 B/slot vector; its measured-negative RSS came
  from that relocation + vector slack, and its Ir was actually −2.7% on binarytrees. A
  slot-*indexed* handle instead adds an indirection to `deref` (the hot path) — a cost W2
  never incurred and **never measured**. That cost is a hypothesis, plausibly real, and must
  be measured before this can land.
- **Underdesigned here on purpose:** a complete B design must specify slot storage and
  reuse, a *per-slot* generation (not just per-heap — otherwise slot reuse aliases a new
  object), registry memory, `Option<Gc<T>>` niche/layout, type-erased trace/drop metadata,
  compaction with handle forwarding, and borrow-lifetime enforcement. That is a multi-day
  redesign, and it should be co-designed with the #113 owner-vector spine (they share the
  slot table), so the deref cost is paid once, deliberately, and measured.
- **Verdict:** the principled answer **iff** release-mode safety against arbitrary host
  handle lifetimes is required. Disproportionate to #267 alone.

### (C) Thin boxes + explicit contract + deref-free fail-safe guards

Keep `Gc` a bare pointer and the header at 24 B. Close what is cheaply closable; document
the rest.

1. **F1 fix (mechanical, free, every build):** the *no-guard* `downgrade` path must decide
   **without dereferencing the box**. Since a bare handle cannot distinguish
   heap-owned from detached without a deref, and `debug_assertions` does **not** imply
   quarantine (and teardown frees quarantined boxes anyway), there is *no* safe conditional
   deref. Two deref-free choices, both sound:
   - **(C-panic, recommended, matches repo policy):** no-guard `downgrade` **panics**
     unconditionally, reading nothing — consistent with `GcRef::new`'s guard-less panic
     (`gc.rs:39`) and the always-on guard policy. Removes the UAF *and* stays loud.
   - **(C-dead):** returns a permanently-dead `GcWeak` (a new `Dead` state) that never
     upgrades. Softer for embedding, but silences a real missing-guard bug.
   Either drops the legacy detached-box "always upgrades on no-guard downgrade" path; §6
   must confirm nothing in-tree relies on it (`GcRef::new` panics guard-less, so heap-owned
   boxes always have a guard at creation, and no in-tree path downgrades a detached box with
   no guard — but the oracle, not this doc, settles that).
2. **F3 fix (no-guard, mechanical, free):** the no-guard `account_buffer` arm becomes a
   deref-free no-op or panic (same choice as above), never `is_heap_owned()`.
3. **Quarantine tripwire in `downgrade`/`account_buffer` (free, no header change):** before
   minting a token, in quarantine mode read the parked box header and refuse/panic if
   `HDR_FREED` is set. Catches **F1, F2b, and F2c** (every *freed/swept* box) — notably the
   same-heap F2c that no owner tag sees. Safe because quarantine parks (does not free) the
   box; still nothing post-`drop_all` (teardown frees the parked list).
4. **F2 / F2c / F3-foreign / F4 in release:** documented as UB-with-quarantine-detection. But
   see §5 — a *document* is not a valid resolution while the raw ops are safe public
   functions; the honest cheap resolution is to **seal the raw surface** (`pub(crate)`/`unsafe`)
   and route embedders through the rooted `RootedValue` handle, which the facade already does.

- **Byte cost:** zero per object (C-dead adds one discriminant to `GcWeak`, per handle, not
  per object).
- **Benefit:** removes both "UAF-in-the-check" bugs (F1, F3-no-guard) outright in release, and
  makes F1/F2b/F2c deterministically detectable under quarantine.
- **Limitation:** F2/F2c/F4 stay UB in release; the resolution is sealing the surface or a
  redesign, not documentation.

### (D) Hybrid — (C) plus a free u8 heap-generation debug tag

Everything in (C), and spend the **free padding byte** on `owner_gen: Cell<u8>` in
`GcHeader` (24→24 / 16→16, §7), stamped from a process-global wrapping heap-generation
counter at allocation. Under quarantine/`debug_assertions`, `downgrade`/`account_buffer`
assert `box.owner_gen == active_heap.gen`.

- **Byte cost:** zero (occupies existing pad; size asserts unchanged). *Not literally
  "zero release cost":* the tag is initialized on every allocation — likely a free combined
  store, but §6's Ir arbiter must establish it.
- **What it buys over (C):** a deterministic **quarantine tripwire** for *live-box* foreign
  misuse (F2a) — the box is still allocated/parked, so the tag read is safe.
- **What it explicitly does NOT buy — and must not be sold as:** (i) it cannot be read on a
  **freed** box, so F2b/F2c/F4 are unchanged and the debug check must **not** run on a
  possibly-freed handle (that read is itself the UAF — the `HDR_FREED` tripwire in (C) must
  gate it); (ii) it **cannot see F2c at all** (same-heap swept box: `owner_gen` still matches
  A); (iii) u8 **wraps at 256** heap constructions, so even live-box detection aliases — a
  *tripwire, not a guarantee*. A live-ID leasing scheme could sharpen it but still gives no
  release soundness. **Its only unique value over the free `HDR_FREED` tripwire is catching
  F2a (foreign, box still live).**

### (E) Considered — eager token registration

Revert lazy tokens to **eager** registration at allocation, so the ambient heap knows "I
own this address" and `downgrade` under a foreign B (which never registered A's address)
refuses. Unlike A/D, this detects foreign-live **and** absent-after-sweep addresses
**without dereferencing the box** — a genuine architectural advantage. But:
- **Cost (measured historical):** the earlier eager map was ≈ 50 B per **live** object;
  removing it improved Ir ≈ 2.6–3.7 % on allocation-heavy workloads. It re-taxes the hot
  allocation path (`heap.rs:2264-2277`) and grows memory per-live-object — worse than A's
  +8 B.
- **Incomplete:** if B has reused A's freed address for a live B-object, the stale handle
  (which carries no original token) validates against B's *current* token → ABA
  resurrection of the wrong object. So it does not fully close F2b either.
- **Verdict:** rejected on measured cost, not domination; its deref-free detection is noted
  as the one thing C/D lack cheaply.

### Options at a glance

| Option | Per-box bytes 64-bit / wasm32 | F1 | F2 (foreign) | F2c (same-heap swept) | F3 | F4 | Hot-path cost |
|---|---|---|---|---|---|---|---|
| (A) u32 owner-id | **+8 / +4** | no (derefs freed) | F2a, **release** | no | no | no | none |
| (A′) u8 owner-id | 0 / 0 | no | F2a, debug only | no | no | no | none |
| (B) slot-indexed handle | 0 header; slot table + registry | **yes** | **yes** | **yes** | **yes** | **entry only; &T still danglable** | **deref indirection (unmeasured; NOT W2's number)** |
| (C) deref-free + `HDR_FREED` tripwire | 0 / 0 | **yes (release)** | F2b debug; F2a no | **debug tripwire** | F3-no-guard: yes | seal/redesign | none |
| (D) (C) + free u8 tag | **0 / 0** | **yes (release)** | + F2a debug tripwire | debug tripwire (via C) | yes / debug | seal/redesign | ~0 (Ir to confirm) |
| (E) eager tokens | +≈50 B / live object | — | deref-free, F2b partial | deref-free detect | — | — | alloc hashmap insert |

---

## 4. Concrete shape of the recommended change

Illustrative, not prescriptive. Recommended = C-panic (deref-free) + the free u8 tag (D).

No-guard `downgrade` becomes deref-free — it never calls `is_heap_owned()`:

```rust
pub fn downgrade(&self) -> GcWeak<T> {
    let identity = self.identity();
    match lua_gc::with_current_heap(|heap| {
        heap.map(|h| (HeapRef::from_heap(h), h.register_allocation_token(identity)))
    }) {
        Some((heap, allocation_token)) => GcWeak { target: self.0, identity, heap: Some(heap), allocation_token },
        // No active guard: cannot validate against any heap and must not read the
        // (possibly freed) box. Panic loudly — consistent with GcRef::new's
        // guard-less panic — OR return a permanently-dead GcWeak (the C-dead variant).
        None => panic!("GcRef::downgrade with no active HeapGuard — operating a GcRef \
                        outside its owning heap's guard is a bug; a stale handle here \
                        would be use-after-free"),
    }
}
```

`GcHeader` gains the free tag (field order matters — `owner_gen` must sit in the offset-3
pad slot to stay free; the `#[cfg(target_pointer_width = "64")]` 24 B assert must stay green
and a wasm32 16 B assert added):

```rust
#[repr(C)]
pub struct GcHeader {
    color: Cell<Color>,
    age: Cell<GcAge>,
    flags: Cell<u8>,
    owner_gen: Cell<u8>,   // was padding — 0 bytes added
    size: Cell<u32>,
    next: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
}
```

If instead the C-dead variant is chosen, `GcWeak` gains a two-state discriminant (`Tracked`
vs `Dead`); note the illustrative `downgrade` above has **no** reachable "detached" arm — the
legacy detached-always-upgrade behavior is dropped, so do not model a `Detached` state that
no code constructs.

---

## 5. Recommendation (the maintainer's call)

**Frame the decision as a safety boundary first (codex finding 2).** The current state
treats raw `Gc`/`GcRef` as *both* an internal capability *and* a safe public surface: `Gc`,
`GcRef`, `Deref → &T`, `account_buffer`, and `Heap::drop_all(&self)` are all safe fns, so the
"a `GcRef` must not outlive its heap" rule is a naked comment, not a type-enforced invariant —
purely-safe code can violate it. A comment cannot license UB from safe code. So before any
byte question, decide **what the raw surface *is*.** Two coherent postures:

- **Sealed capability (recommended, cheap):** make raw `Gc`/`GcRef` stale-sensitive ops
  `pub(crate)`/`unsafe`, and keep the *public* embedding surface the rooted `RootedValue`
  handle the facade already uses (`lib.rs:2033`) — it owns a `Lua`, roots via a slab key, and
  already returns `stale_handle_error` on stale use. Then the "contract" is real (enforced by
  visibility), and C/D's cheap in-crate hardening is the right amount of GC-internal work.
- **Safe arbitrary-lifetime surface (expensive):** if raw `Gc` must stay safely usable with
  any host lifetime, owner identity is necessary but *not sufficient* (finding 3): you also
  need an API-shape change so `drop_all`/collection cannot free a box while a borrow lives —
  `&mut`/exclusive teardown, access guards instead of `&T`, or pinning/rooting for the borrow.
  Only then does a slot-indexed handle (B) buy release soundness.

**Recommended, separable lands (all assume the sealed-capability posture):**

1. **Land the deref-free no-guard fix now, independent of everything else.** No-guard
   `downgrade`/`account_buffer` deciding *without* dereferencing the box removes F1 and
   F3-no-guard — where the *safety machinery itself* faults — in release, at zero per-object
   cost. Unambiguous bug removal; do not conflate it with "owner-identity soundness." Prefer
   the **panic** variant (matches the always-on guard policy); dead-weak is the softer choice.
2. **Land the free `HDR_FREED` quarantine tripwire in `downgrade`** — it turns F1, F2b, and
   the same-heap **F2c** into deterministic test-time failures, at zero bytes, and catches the
   case (F2c) the owner tag cannot.
3. **Optionally add the free u8 tag (D)** — a complementary quarantine tripwire whose only
   unique catch is F2a (foreign, box still live). Zero bytes; honest that it is a tripwire,
   misses F2c, and wraps at 256 heaps.

**Deferred — the maintainer's real decision:** whether stale use must be safe **in release**
(not just detected under quarantine). Sealed-capability says no (visibility makes the contract
real; C/D suffice). Safe-arbitrary-lifetime says yes → budget the API-shape redesign **and**
slot-indexed handles (B), co-designed with the #113 owner-vector work and gated on a
deref-path Ir/RSS measurement (multi-day, correctness-sensitive).

**Do not choose (A):** +8 B/box on 64-bit *and* still faults on every freed box (F1/F2b/F2c/
F4); its only unique gain over the free u8 tag is release-mode F2a detection, which does not
justify re-crossing the size classes the diet cleared.

**Why this is the maintainer's call:** lands 1–2 are a clear yes (pure bug removal). The
genuine judgment is the boundary — *seal the raw surface and treat C/D as sufficient*
(cheap, byte-neutral, makes the contract type-real) vs *promise safe arbitrary-lifetime raw
handles* (API-shape redesign + B, real perf cost, multi-day). That hinges on what the omniLua
crate-level API promises to code depending on `lua-gc`/`lua-types` directly — a product/API
decision, not a GC-internal one.

---

## 6. Test / verification plan

These are pure in-memory GC state-machine bugs — the "custom subsystem tester" tier
(rung 2/3): milliseconds, 100 % deterministic, no oracle needed to *develop* against.
**The oracle does not settle memory safety** — it only checks Lua-visible behavior;
memory-safety is settled by quarantine + ASan/Miri (where available) + negatively-verified
state-machine tests. Tests must match the guarantee the chosen option actually gives (a
release test may not demand a property only B provides).

### 6.1 Regression tests — aligned to the recommended C-panic + D

`cargo test -p lua-types` / `-p lua-gc`, each pinning one failure and its *actual* fix:

- **F1 (release-tested):** `no_guard_downgrade_is_deref_free`. Allocate under a guard; drop
  the guard (pop); **then** drop the last `Rc<Heap>` (frees the box); then `downgrade` the
  surviving handle. Under C-panic: assert it **panics without dereferencing** (the panic
  must not depend on box contents — verify by also running the guard-popped-but-box-**alive**
  case and confirming identical behavior). Under C-dead: assert a dead weak that never
  upgrades. The old `guardless_downgrade_of_heap_owned_box_panics` (`gc.rs:274`) is replaced,
  because its panic decision was the UAF vector.
- **F3-no-guard (release-tested):** `no_guard_account_buffer_is_deref_free` — deref-free
  no-op/panic; no UAF.
- **F2a (debug/quarantine tripwire only):** `foreign_live_downgrade_tripwire` under
  `LUA_RS_GC_QUARANTINE=1` — heaps A,B live; allocate on A; `downgrade` under B; assert the
  u8-tag mismatch panics. **Do not** assert release-mode "never upgrades" — D does not
  provide it; that assertion belongs to a B implementation.
- **F2c (the important same-heap case; quarantine tripwire under C):**
  `same_heap_swept_redowngrade_tripwire` — allocate `g` on A under A's guard,
  `A.full_collect(&roots_without_g)` (so `g` is swept), then `g.downgrade()` still under A's
  guard. Under quarantine, the `HDR_FREED` check in `downgrade` must panic (box is parked).
  Without the fix, assert (on a reverted build) that `upgrade()` **resurrects** — i.e. the
  weak validates against the re-minted token — to prove the bug is real. A release C/D build
  cannot make this safe; only B does.
- **F2b (documented; quarantine tripwire under C):** `foreign_downgrade_after_close` — under
  quarantine the `HDR_FREED`/closed-token path refuses; a release test asserts only that the
  deref-free no-guard change did not *worsen* it, and (if B is chosen) that there is no
  resurrection.
- **Issued-`&T` outliving teardown (finding 3, documents the B residual):**
  `deref_borrow_across_drop_all_is_ub` — hold `let r: &T = &*g;` then call `drop_all(&self)`;
  document that this dangles under *every* option including B unless the API shape changes.
  A compile-fail/`trybuild` test is the real fix once teardown takes `&mut`/returns a guard.
- **F4 (contract boundary):** `stale_deref_trips_quarantine` — under quarantine while the
  heap is alive, the `HDR_FREED` assert fires; asserts detection persists, not that it is
  fixed. Explicitly **not** a post-drop test (drop_all frees quarantined boxes; no tripwire
  survives teardown).
- **F5 (correctness caveat):** `identity_aba_after_reuse` — document that `identity`/`ptr_eq`
  can alias across free+reuse; not a UAF.
- **Additional matrix cases the packet must cover:** same-owner post-sweep downgrade/account
  (token-mismatch path), same/foreign address reuse (ABA), and — for any B implementation —
  slot/generation wrap and collision.

**Reproduce-the-bug discipline:** each UAF test must be shown to FAIL on pre-fix code under
`LUA_RS_GC_QUARANTINE=1 LUA_RS_GC_STRESS=1` (a green run without the sanitizer is not
evidence). Per this repo's #249 memory, verify a fix by git-reverting it, not by trusting a
pass. Add the tests to `harness/canaries/gc/`.

### 6.2 Proving zero per-object cost (required for the "free" claim)

1. **Size asserts:** `gcheader_is_24_bytes_after_grayagain_diet` (`heap.rs:4272`) must stay
   green with `owner_gen` present (proves the pad was reused, not grown); add a wasm32 16 B
   header assert, a `size_of::<GcBox<T>>()` assert on a representative payload, **and** a
   `size_of::<GcWeak<T>>()` assert (the C-dead discriminant / niche must not silently grow
   the weak handle).
2. **Ir arbiter (`docs/MEASUREMENT_PROTOCOL.md`):** frozen-baseline interleaved A/B on
   `binarytrees` and `gc_pressure` — must be Ir-neutral (drop-if-neutral). The tag is one
   byte written at alloc and read only on cold paths; a measurable delta means the layout or
   the alloc-store regressed.
3. If **(A)** or **(B)** is chosen instead, report the *opposite* honestly: (A) flips the
   24 B assert to 32 B and must show the size-class hypothesis on peak-RSS triples; (B)
   changes `Gc`'s shape and must carry the full deref-indirection Ir/RSS measurement — and
   must **not** borrow W2's RSS number — before it can land.

### 6.3 Full gate (rung 6)

`harness/strict_guard_check.sh` (repository-mandated for guard/lifecycle changes) +
`harness/run_official_all.sh` + `cargo test --workspace` + the GC canary battery
(incremental + generational × quarantine) + the two multiversion oracle passes, to confirm
the no-guard behavior change (panic/dead-weak vs former deref-and-panic) regresses no real
program. Where available, a Miri run over the new lua-gc tests to catch residual UB the
`debug_assert` model misses.

---

## 7. Appendix — compiled layout evidence

`rustc -O` on a standalone probe replicating `GcHeader` with a **fat** `next` pointer
(`NonNull<dyn Trace>`, matching `NonNull<GcBox<dyn Trace>>`); `Color`/`GcAge` are 1 B
fieldless enums as in the source. **This proves requested `size_of` only — not allocator
buckets or RSS.**

```
HdrCurrent = 24 (align 8)          # matches heap.rs:4272 assert
HdrU8      = 24   (u8 owner in the offset-3 pad byte)   -> +0 B
HdrU16     = 32                                          -> +8 B (64-bit)
HdrU32     = 32                                          -> +8 B (64-bit)
GcBox<()>      : cur 24  u8 24  u32 32
GcBox<[u8;16]> : cur 40  u8 40  u32 48    # size grows to next 8-mult (bucket = hypothesis)
GcBox<[u8;40]> : cur 64  u8 64  u32 72    # size grows to next 8-mult (bucket = hypothesis)
```

Byte ledger, corrected for all three target layouts:

| owner width | current 64-bit (24 B) | current wasm32 (16 B) | hypothetical W2 target (8 B) |
|---|---|---|---|
| u8 | 24 (+0) | 16 (+0) | 8 (+0) |
| u16 / u32 | 32 (+8) | 20 (+4) | 12 (+4) |

The 64-bit +8 comes from the 8-byte alignment of the fat `next` pointer; on wasm32 the fat
pointer is 4-byte-aligned (8 B: 4 data + 4 vtable), so the added word costs only +4. Against
the W2 8 B target (color+age+flags+pad+size, align 4) a u32 owner costs +4, not +8 — so
"owner-id is expensive" is layout-dependent, and the recommendation's dismissal of A is
specifically about the **shipped** 24 B header. wasm32 figures are reasoned from `#[repr(C)]`
field order (not separately compiled here); the implementing packet must pin them with
wasm32 `const` size asserts. **Whether any of these `size_of` growths crosses a live-workload
malloc bucket, or moves RSS, is a hypothesis to be settled by §6.2, not by this probe.**
