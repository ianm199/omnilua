# Codex R1 adversarial review (verbatim) — PERF_113_SIZE_CLASS_ANALYSIS

`codex exec -s read-only` output, recorded verbatim per the campaign method.
Verdict: **REVISE**. The rev-2 analysis doc addresses every finding below; see
its "Adversarial review (codex R1) — responses" section for the point-by-point
disposition (FIX / ACKNOWLEDGE / REBUT).

---

The packet needs revision. Its relevant size-class boundaries are sound, and UpVal remains the best high-population target, but the evidence overstates exact populations and the proposed UpVal field change is not safe as written.

### Blocking findings

1. **The proposed UpVal `u32` conversion is semantically unsound.**

`open_thread_id` ultimately receives a globally monotonic `u64` thread ID, not a count of currently live coroutines. The counter advances on every thread creation and IDs are not reused ([upval.rs](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/crates/lua-types/src/upval.rs:27), [state.rs](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/crates/lua-vm/src/state.rs:1760), [allocation site](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/crates/lua-vm/src/state.rs:6044)).

Therefore, `u32::MAX` can be reached after roughly four billion lifetime creations, without four billion live coroutines. A cast would eventually alias IDs or collide with the sentinel. Calling this “range safe” and “sentinel-preserving” is incorrect.

The 56→48 opportunity remains real, but implementation needs either:

- a representation preserving the existing ID domain;
- or an explicit checked global `u32` limit with defined exhaustion behavior.

Any replacement must also be benchmarked for hot-path instruction cost and checked on wasm.

2. **The histogram does not establish exact typed populations.**

`bucket_of(size) = size / 8`, so “bucket 56” actually aggregates requested sizes 56–63, and bucket 96 aggregates 96–103 ([size_class_histogram.rs](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/crates/lua-rs-runtime/examples/size_class_histogram.rs:101)). Showing that no other listed `GcBox<T>` has size 56 does not exclude unrelated allocations within that range.

Thus 100,001 UpVals and 131,331 LuaTables are strongly corroborated by workload structure, but not “exact” from this instrument. Exact attribution requires typed heap counters or at least exact `(size, alignment)` request counters rather than 8-byte ranges.

The snapshot also occurs at peak requested `Layout::size()` bytes, not peak rounded allocator bytes, per-type population, or process RSS. “Peak-RSS proxy” is too strong.

3. **The sampled execution is not the benchmark execution.**

The benchmark runner repeats short workloads inside one VM—closure_ops three times, binarytrees twice, and table_hash_pressure nine times in the current ledger—while the histogram tool executes each workload once ([compare.sh](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/harness/bench/compare.sh:112), [ledger.jsonl](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/harness/evidence/ledger.jsonl:3806)). Repetition can change GC timing, allocator retention, and max RSS.

The default set also omits `concat_chain` and `string_format_mixed`, despite both being part of #113’s five-row done condition ([ISSUE_BURNDOWN_SPEC.md](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/docs/ISSUE_BURNDOWN_SPEC.md:361)). That makes the recommendation to park the rest of the issue too broad.

### Allocator-model corrections

The locally relevant observations are correct: live probing confirmed that requests 40, 56, 72, 88, 104, and 232 receive 48, 64, 80, 96, 112, and 240-byte blocks respectively. So a real 56→48 UpVal allocation removes 16 allocator-slot bytes.

But `malloc_good_size` reports an efficient allocation size, not guaranteed RSS reclaimed. Apple’s documentation describes it as a size that can be allocated without padding; `malloc_size` reports the backing block’s size ([Apple manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/malloc_good_size.3.html)). Libmalloc serves small objects from pooled regions, so reduced slot demand becomes lower RSS only when it changes region/page occupancy or peak allocation behavior ([Apple memory-allocation guidance](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/ManagingMemory/Articles/MemoryAlloc.html)).

Consequently:

- Rename “RSS bytes reclaimed” to “allocator slot bytes avoided.”
- Treat 1.53 MiB as an upper-bound mechanism projection pending interleaved max-RSS A/B.
- The claim that cadence “barely shifts” is unsupported: reducing charged bytes by 0.8 MB is about 5% of the reported requested-byte peak and could cross nonlinear pacing thresholds.
- The quoted 25–37 MB range makes 1.53 MiB roughly 4–6%, not 4–8%.
- “Tiny continuously through 1024, then 512-byte classes, then pages” is broader than the probe establishes and ignores libmalloc’s zone/optional-medium behavior. It does not affect these ≤240-byte candidates, but should not be presented as a general platform model.

### Candidate verdict corrections

- **LuaLClosure and LuaString:** Correct within the narrow “simple field removal” scope. Neither can reach an 8-byte payload while retaining the current owning slice. Inline storage, thin/trailing allocations, or coallocation remain structural representation projects, not impossibilities.

- **LuaTable:** “No demonstrated simple safe crossing” is justified, but “infeasible” is too categorical. `UnsafeCell` would conflict with the crate’s zero-unsafe budget, yet a safe ownership-swapping or more structural representation deserves a bounded design spike because the population is large. Also, `TableFlags` does not plainly have enough spare bits: the documented metamethod cache consumes the lower bits and weak mode needs two bits ([table.rs](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/crates/lua-types/src/table.rs:84)).

- **Low-population group:** The payoff verdict is reasonable, but the types are not uniformly infeasible. For example, `LuaProto.cache` can plausibly change from `RefCell<Option<GcRef<_>>>` to `Cell<Option<GcRef<_>>>`, saving eight bytes and crossing 240→224 ([proto.rs](/Users/ianmclaughlin/PycharmProjects/rustExperiments/lua-rs-port-113analysis/crates/lua-types/src/proto.rs:12)). LuaCClosure and LuaUserData also have fixed-length `RefCell<Vec<_>>` storage that could potentially become boxed cells. These should be labeled “feasible but immaterial in sampled workloads.”

- **LuaState:** Its reported population of one is not established and appears to be another allocation colliding with the 200-byte request bucket; the current runtime does not normally allocate a `GcBox<LuaState>`. Remove that population claim.

There are also decimal-byte totals labeled as MiB in the buffer section: 13,656,624 bytes is about 13.02 MiB, not 13.6 MiB.

The revised conclusion should be: UpVal is the best high-population size-class opportunity, but its representation must be redesigned without narrowing the thread-ID domain; population and RSS savings then require benchmark-faithful typed measurement and interleaved A/B validation. Other candidates should be separated into “structurally infeasible,” “feasible but low population,” and “larger representation track,” rather than parked collectively.

VERDICT: REVISE