# Codex adversarial review — issue #267 owner-identity design spec

Cross-model (OpenAI Codex, `codex exec -s read-only`) review of the FIRST draft of
`ISSUE_267_OWNER_IDENTITY_SPEC.md`. Verdict was **REVISE**; the findings held and the
spec was revised once to incorporate them (same-owner swept F2c, the safety-boundary
reframing for safe public APIs, W2-vs-slot-handle evidence separation, corrected
wasm32/W2 byte ledgers, `HDR_FREED` quarantine tripwire, realigned tests). Recorded
verbatim below.

---

The core observation is sound, but the recommendation does not follow from the analysis. The spec misses a major stale-handle state, misattributes W2 performance evidence, understates several costs, and proposes tests that option D cannot pass.

## Critical findings

1. The failure model omits same-owner staleness after ordinary sweep.

A `GcRef` local is not a GC root. This safe sequence is possible while heap A remains open and its guard remains active:

1. Allocate `g` on A.
2. Collect A with roots that do not trace `g`.
3. Call `g.downgrade()` under A’s guard.

`downgrade` reads only the pointer identity and `register_allocation_token` inserts it into A’s map, thereby validating an already-freed address. The repository’s own token test explicitly registers an identity after sweep and asserts that it becomes valid again at [heap.rs:3721](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-gc/src/heap.rs:3721). That test is evidence of the missing failure, not merely an address-reuse test.

This adds at least:

- F2c: same open owner heap, target already swept, `downgrade` resurrects it.
- F3c: same open owner heap, target already swept, `account_buffer` dereferences freed memory.
- Same-owner address-reuse/type-confusion: a stale typed pointer may now address a different allocation.

Option D does not detect F2c: the owner tag matches A. This materially weakens the recommendation.

2. “Documented hard invariant” is not a valid resolution for a safe Rust API.

`Gc`, `GcRef`, `Heap::full_collect`, `Heap::drop_all`, `Gc::account_buffer`, and `Deref` are all public safe APIs. See [Gc and its safe dereference](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-gc/src/heap.rs:840), [full_collect](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-gc/src/heap.rs:2422), and [drop_all](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-gc/src/heap.rs:3161). Safe callers can retain a handle or even an issued `&T`, call `drop_all(&self)`, and then use it.

Documentation cannot make UB from exclusively safe operations acceptable. The maintainer is not simply choosing “contract versus bytes” unless the raw GC surface becomes private or explicitly unsafe.

The high-level embedding API already demonstrates the correct boundary: `RootedValue` owns a `Lua` clone and a root-slab key, and checks cross-state use at [lib.rs:2032](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-rs-runtime/src/lib.rs:2032). The spec should decide whether raw `Gc` is an internal unsafe capability or a safe public handle. It currently treats it as both.

3. Option B does not yet fully close F4.

A slot lookup can validate before dereference, but `Deref::deref` returns `&T`. A later safe `drop_all(&self)` or collection can free the box while that reference remains live. Preventing that requires one of:

- tying collection/teardown to an exclusive lifetime;
- returning an access guard rather than `&T`;
- pinning/rooting while the borrow exists;
- removing safe `Deref`.

There is also no complete generation scheme. `{ slot: u32, gen: u32 }` needs to distinguish both heap reuse and slot reuse. If `gen` is only the heap generation, reusing a slot aliases a new object. If slots are never reused, the table grows with historical allocation churn. Compaction cannot move indexed slots without forwarding or rewriting handles.

4. The failure surface is broader than the three named operations.

Safe owner-sensitive paths also include:

- `Marker::mark`, which dereferences any supplied `Gc` at [heap.rs:1252](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-gc/src/heap.rs:1252);
- incremental and generational barriers, which mutate headers and can insert foreign pointers into heap-side state at [heap.rs:2319](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-gc/src/heap.rs:2319);
- public color, age, finalized, and tracking accessors;
- finalizer-list move methods accepting raw type-erased pointers.

F4 partially gestures at transitive tracing, but the claim that only `downgrade` and `account_buffer` are unsound is too narrow for an owner-identity design.

Also, `identity` and `ptr_eq` are memory-safe on stale handles, but not allocation-identity-safe: allocator reuse creates ABA equality. F5’s “none” residual gap is therefore too strong.

## Byte and performance accounting

The current native layout arithmetic is mostly right, but the cross-target and allocator conclusions are not.

| Layout | u8 owner | u16/u32 owner |
|---|---:|---:|
| Current 64-bit header | 24→24 | 24→32, +8 |
| Current wasm32 header | 16→16 | 16→20, +4 |
| W2 8-byte target | 8→8 | 8→12, +4 |

The appendix incorrectly says wasm32 u32 is 16→24. With a 4-byte-aligned 8-byte fat pointer, it is 16→20. And because the recommendation explicitly leaves W2 open, dismissing the 8-byte layout is incomplete: against that target, u16/u32 costs four header bytes, not eight.

Further:

- A header growing by eight bytes does not guarantee every `GcBox<T>` grows by eight; an over-aligned `T` may already consume that padding.
- `size_of` proves requested layout, not allocator buckets or RSS. `GcBox<[u8;16]>` going 40→48 does not by itself prove a bucket crossing. The spec’s “empirically verified” label overclaims what the standalone probe measured.
- Actual consequences require the live type-size histogram and heap/RSS evidence. Representative byte arrays are not evidence that common omniLua allocations cross classes.
- The u8 tag is allocation-size-neutral, but not “zero release cost”: it must be initialized on every allocation. That may compile into a free combined store, but the Ir result must establish it.
- `WeakState` is not necessarily “one bool.” Its enum layout and niche effects need a `size_of::<GcWeak<T>>()` assertion.

The performance-triage evidence also invalidates the description of option B. Historical W2 replaced owner lists with fat-pointer vectors while retaining raw-pointer `Gc` handles; it did not add a slot lookup to every dereference. Its measured negative came from relocating a 16-byte fat owner pointer plus vector slack, not handle indirection. See [the retained W2 evidence](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/docs/PERF_EVIDENCE_113_W2_OWNERVEC_20260713.md:10) and [the W2 owner-vector ledger](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/specs/ISSUE_113_GCHEADER_DIET_SPEC.md:73). Calling the proposed slot handle “W2 measured-negative” transfers evidence between different architectures.

## Option honesty

- A is incomplete but not dominated by D. A performs live foreign-owner detection in release; D does it only in debug and aliases after 256 heap constructions. Those are different benefits. A still needs a non-wrapping/exhaustion rule if its ID is presented as a guarantee.

- B is underdesigned and unmeasured. It omits slot storage, per-slot generation, registry memory, `Option<Gc<T>>` niche/layout effects, type-erased trace/drop metadata, and borrow-lifetime enforcement. It is not the historical W2 architecture.

- C contains a valid narrow fix: a no-guard path can panic or fail without dereferencing. But the debug exception is invalid. `debug_assertions` does not imply quarantine, and teardown frees quarantined boxes. There is never a safe general reason to dereference in the no-guard branch. A deref-free unconditional panic is simpler and preserves the repository’s always-on guard policy.

- D is size-neutral but not mechanically safe. Before reading `owner_gen`, code does not know whether the box is live. On F2b, same-owner post-sweep, or post-drop foreign use, the debug check itself dereferences freed memory. Its “deterministic” claim also conflicts with deliberate u8 wrap. A live-ID leasing scheme could improve the tripwire, but it still would not establish release soundness.

- E has a real, measured cost: the earlier eager map was approximately 50 bytes per live object and removing it improved Ir by roughly 2.6–3.7% on allocation-heavy workloads. That supports rejecting it on performance grounds. But it is not “strictly dominated”: it detects foreign-live and absent-after-sweep addresses without dereferencing, a benefit C/D lack. Nor does it fully close F2b—if B has reused A’s freed address, lookup finds B’s current token because the stale handle carries no original token.

The existing token counter also wraps to 1 at [heap.rs:1898](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-267spec/crates/lua-gc/src/heap.rs:1898), contradicting claims that it “never reissues” a token, especially relevant on wasm32.

## The proposed tests contradict option D

Several tests cannot pass under the recommendation:

- F2a expects the weak never to upgrade, but D only rejects it in debug; release still registers it in B.
- F2b expects no resurrection even though D explicitly leaves F2b contract-governed. Debug may UAF while reading the tag; release resurrects it.
- Foreign F3 expects B’s pacer unchanged, but release D still charges B.
- The F1 sequence says dropping the owning `Rc` frees the heap before dropping the guard, but the guard itself holds an `Rc`.
- The quarantine variant expects a post-drop tripwire even though `drop_all` drains and frees quarantined boxes—the spec correctly states the opposite earlier.
- `WeakState::Detached` is unreachable in the illustrative `downgrade`: active heap always constructs `Tracked`; no heap constructs `Dead`. No path constructs `Detached`.
- The plan omits same-owner post-sweep, same/foreign address reuse, generation wrap/collision, and validation of already-issued `&T`.
- The oracle cannot settle memory safety. Quarantine, ASan/Miri where available, and negatively verified state-machine tests do; the oracle only checks Lua-visible behavior.
- This GC guard/lifecycle change should also run the repository-mandated `harness/strict_guard_check.sh`.

## Required revision

The next version should:

1. Define the safety boundary first. Either make raw `Gc`/`GcRef` operations private or unsafe and rely on rooted high-level embedding handles, or require safe arbitrary-lifetime behavior and accept a larger redesign.
2. Expand the failure matrix across live/swept/quarantined/closed/dropped × no/owner/foreign guard × collected/bootstrap/detached allocation.
3. Add same-owner stale mint/account cases and allocator-reuse ABA.
4. Separate historical W2 owner vectors from a new generational slot-handle design, including its complete memory and lifetime model.
5. Correct the native/wasm/W2 byte ledgers and label allocator effects as hypotheses until heap/RSS measurements exist.
6. Align tests with the selected guarantee. If D remains merely a debug tripwire, release tests cannot demand F2/F3-foreign safety.
7. Consider the smaller immediate patch independently: no-guard `downgrade` and `account_buffer` should fail without any box dereference. That is worthwhile, but it should not be presented as owner-identity soundness.

VERDICT: REVISE