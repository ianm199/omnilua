# Matching C performance in a safe-Rust port

A working journal of what we've learned (and what we expect) about closing the
gap between a faithful, line-by-line C → Rust port and the original C
implementation's speed. Written against `lua-rs-port` but the lessons are
intended to be portable to any "translate a mature C systems program into safe
Rust" project (Valkey, nginx, …).

Read alongside `PERFORMANCE_PRINCIPLES.md`. That doc is the *playbook* for
ongoing perf work in this repo (evidence-driven, gate-driven, dashboard-driven).
This doc is the *research journal* — what we learned about why the gap exists
in the first place, what shape the fixes take, and what's on the other side of
the interpreter (a future JIT).

## The framing

lua-rs is, at the most honest level, an **experiment in safe-Rust C porting at
scale**. We picked Lua 5.4 because:

- The source is small enough to port end to end (~28k LoC of C).
- The test suite is a hard, externally-verifiable oracle (the PUC-Rio official
  tests).
- It runs real software (LuaRocks 3.11.1) so we know "done" when we see it.
- Reference performance is a single, faithful interpreter binary. No CDN, no
  network, no syscall noise. Just `target/release/lua-rs my_workload.lua`
  vs `reference/lua-5.4.7/src/lua my_workload.lua`. Numbers are unambiguous.

The harness work (typed work-packets, ledgered evidence, dashboards,
chassis-driven dispatch) is the *real* product. The port is how we earn the
right to claim "this methodology produces working safe-Rust ports of C
programs at performance parity." Each commit is a data point in that claim.

If you're picking up this doc later: the answers below were earned at the
cost of profiling sessions and bench commits. Use them. Don't re-derive.

## The headline finding

**Most of the performance gap between a faithful safe-Rust port and the
original C is not a safety tax. It's a Rust-idiom tax.**

When we first benchmarked lua-rs against reference Lua 5.4.7, the overall
wall-clock ratio was something like 5× slower on a microbench mix, with some
individual workloads much worse due to missed fast paths. After two surgical
fixes (`#t` boundary search + GC snapshot fast-path) the overall ratio dropped
to 2.61× without changing the algorithms or touching `unsafe_code = "forbid"`.
Later table/string/dispatch/GC work pushed individual workloads much closer:
the PR #120 selected matrix (`20260602T122050Z-26c831f`, merged as `2d5cffe`)
has `table_ops` at 1.25× and `table_ops_long` at 1.00× on Apple M3 Max.
The follow-up focused pass on `binarytrees,gc_pressure,string_ops_long`
(`20260602T132632Z-2d5cffe`) moved that focused subset from 2.67× to 2.24×.
That is not "all workloads are done"; it is evidence that the remaining
gap is made of concrete hot paths, not a blanket safe-Rust ceiling.

Splitting the remaining non-parity gap into honest components:

| Source of gap | Estimated contribution | Recoverable without `unsafe`? |
|---|---|---|
| Genuine safety cost (bounds checks Rust can't elide, `RefCell` borrow checks on hot fields, `GcRef` indirection) | ~1.10–1.20× | mostly no |
| Stack/frame/object representation gaps (`StackValue` 24B vs C 16B, `CallInfo` 72B vs C 64B, larger table/upvalue objects) | unknown until isolated | partial; stack-slot parity likely needs a design packet |
| Match-dispatch loop vs computed-goto threaded dispatch | ~1.05–1.10× | partial (cold-arm split, `#[inline(never)]` on rare opcodes) |
| **Clones where C uses pointers** — `state.get_at(i).clone()` to drop a borrow before passing `&mut state` to a helper | **~1.20–1.40×** | **yes, with careful refactoring** |
| **`RefCell` on the hot path** — `state.global.borrow()` per opcode, `RefCell<UpValState>` per upvalue read | **~1.10–1.20×** | **yes, restructure access patterns** |
| **Allocations in cold-but-frequent code** — e.g. the GC snapshot loop we just fixed | **~1.10–1.30×** | **yes** |
| **Accessor methods that should be inlined field reads** — `state.get_at`, `state.ci_base`, `Instruction::opcode` | **~1.05×** | **yes** |
| **Type erasure in known-type hot paths** — `dyn Any` barrier dispatch when the caller already has `GcRef<LuaTable>` | workload-local 1.1–2.0× | **yes, preserve semantics with typed helpers** |
| **Compatibility data structures** — table-backed closure state where C uses userdata/upvalues | workload-local 1.05–1.20× | **yes, if GC roots stay traced** |
| **Collector scratch churn** — HashSet growth, over-broad processed sets, intern-retention scans | workload-local 1.05–1.25× | **yes, if reachability semantics stay intact** |

The bolded rows are the bulk of the remaining gap, and **none of them are
safety taxes**. They are choices we made to satisfy the borrow checker or to
follow idiomatic Rust without thinking carefully about hot paths.

**The goal is parity (~1.0×), not "close enough."**

The sibling project `redis-rs-port` reached full performance parity with
upstream Valkey through exactly the kind of disciplined hot-path work
described in this doc. There is no fundamental reason — no law of physics,
no inherent Rust limitation — why a faithful safe-Rust port of an
interpreter cannot reach the same place. Every unit of the remaining gap
has a recoverable cause; the work is to find and fix them one at a time.

What's different for an interpreter vs. a server like Valkey:

- **Valkey** spends most of its wall-clock in I/O and syscalls (epoll,
  socket reads/writes, packet parsing). The command-handling logic is a
  small fraction of total time. Hot-path optimization there means
  making the small CPU portion fast enough that I/O dominates again, at
  which point parity is the natural state.
- **Lua** is pure CPU. Every nanosecond of interpreter overhead is
  naked CPU time — there's no I/O layer to hide behind. So the
  remaining gap *is* the bottleneck, by definition.

That makes the work harder per unit of improvement, but it doesn't move
the floor. The same disciplined "find a hot-path missed fast path, fix
it surgically" pattern that got redis-rs-port to parity will get us there
too. Earlier versions of this doc claimed "1.3–1.5× is the realistic
floor" — that was a guess, not a measured limit. Discard it.

**The last 1.0–1.2× of the gap** may require carefully-scoped `unsafe`
or a deliberately C-shaped safe layout redesign: raw/unchecked stack slot
access under frame invariants, a C-like stack-slot representation, cached
frame pointers, and possibly `become` for tail-call dispatch when stabilized.
Each of those would be a deliberate, well-contained boundary with its own
correctness argument, not a wholesale departure from the `unsafe_code =
"forbid"` default. `redis-rs-port` accepts scoped unsafe trade-offs in its
network buffer and FFI code without compromising the overall safety story.

## What we've actually shipped (evidence)

Representative commits, each with a `compare.sh` ledger row that shows up on
the dashboard:

### 1. `7682720` — `#t` boundary search via canonical `LuaTable::getn()`

**Pattern:** *Use the fast path that already exists.*

`state.table_length` was hand-rolling an O(n) probe loop that called
`tbl.get_int(i)` until nil. But `LuaTable` (in `lua-types`) already had a
proper C-Lua-style `getn()` using `alimit` + binary search, O(log n). The
slow caller bypassed it. A 4-line fix moved the table workload back into a
normal interpreter-ratio range, and three other workloads (`closure_ops`,
`string_ops`) also moved because they exercised the same path indirectly.

The forensics that found it: a `/tmp/probe_table.lua` micro-isolation
(table.remove only, no math.random) showed pure-table-remove was 6,348×
slower than C-Lua. That ratio implied "single hot function is doing way too
much work," not "wrong algorithm."

### 2. `36a3bbe` — GC fast-path: skip snapshot allocations when no step would fire

**Pattern:** *Cache the precondition, don't disable the feature.*

`state::GcHandle::collect_via_heap` was building 5+ `Vec`/`HashSet`/`RefCell`
allocations on every call — *even when the heap's `step()` would short-circuit
because `bytes_used < threshold`*. The threshold check happened *inside* the
heap, after we had already paid the snapshot allocation cost. Since
`gc_check_step` runs from `precall` on every Lua function call, fibonacci's
~10M calls created a continuous storm of `malloc`/`free` that the OS allocator
dominated profiles with.

A `/usr/bin/sample` profile (`harness/bench/profiles/...615e197-fibonacci/`)
showed the smoking gun directly: 30% of wall-clock in `_nanov2_free` and
`nanov2_malloc_type`, all reached through `collect_via_heap` (NOT user-code
allocation). The call graph made the path unambiguous.

Fix: add `Heap::would_collect()` predicate; bail out of `collect_via_heap`
before the snapshot block when `force=false && !would_collect()`.

Bench impact (best-of-5):

| workload | before | after | speedup |
|---|---|---|---|
| binarytrees | 3.23× | 1.80× | 1.8× |
| closure_ops | 4.06× | 2.28× | 1.8× |
| fibonacci | 5.25× | 2.71× | 1.9× |
| string_ops | 10.00× | 8.00× | 1.25× |
| overall | 4.85× | 2.61× | **1.86× geomean** |

One commit. One ledger row. Three workloads ~halved their gap to C.

### 3. `98f5e50` — `#[inline]` on bytecode-dispatch accessors

**Pattern:** *Inline single-instruction-equivalent functions on the hot path.*

`Instruction::opcode()` is a 83-arm match on the low 7 bits of the
instruction word. Without `#[inline]`, it was showing 8.4% of fibonacci's
post-fix profile as its own frame — i.e., LLVM was emitting an actual function
call per dispatched instruction. Same for `arg_a()`/`arg_b()`/`arg_c()` etc.,
each of which is a single shift+mask.

Adding `#[inline]` to all thirteen accessors collapsed them into the
dispatch loop where LLVM can fold them into the surrounding bytecode-decode
code. Bench impact marginal (2.61× → 2.60×) because LLVM was already inlining
some via LTO, but it's the right hygiene and shows up in the profile
re-sample: `Instruction::opcode` dropped from a top-5 frame to invisible.

### 4. `d8c1423` — inline VM stack and call accessors

**Pattern:** *C macros are a code-generation contract, not just syntax.*

After the table fast path (`92c905e`) removed the obvious stdlib overhead, the
remaining `fibonacci` profile was mostly VM/call mechanics:

- before profile (`dc0a693`, `fibonacci`): `execute` 72.2%, `precall` 8.5%,
  `prep_call_info` 7.6%, `upvalue_get` 4.4%, `collect_via_heap` 3.6%,
  `set_ci_previous` 1.8%, `set_top` 1.8%;
- after profile (`a313817`, `fibonacci`): `execute` 89.3%,
  `upvalue_get` 6.4%, `collect_via_heap` 4.3%.

The important reading is not "execute got worse." It means the C-macro-shaped
helpers disappeared as separate call frames and got attributed back into the
dispatch loop, where C-Lua would have had them all along.

The source diagnosis: C-Lua expresses a lot of VM mechanics as macros or
`l_sinline` helpers (`s2v`, `ci_func`, `RA`, `updatebase`, `prepCallInfo`,
`moveresults`). A literal Rust port tends to translate those into ordinary
methods split across crates (`state.get_at`, `state.set_top`, `state.ci_base`,
`state.precall`, `do_::prep_call_info`). That preserves behavior but loses the
code-generation shape: one C pointer arithmetic expression becomes an actual
Rust function call at the hottest point in the interpreter.

`d8c1423` added `#[inline(always)]` to the stack-index arithmetic, stack
accessors, CallInfo accessors, `precall` / `poscall` wrappers, and the
`l_sinline`-equivalent call helpers. It did not change semantics.

Bench impact (best-of-3, `92c905e`/`dc0a693` evidence baseline to
`d8c1423`/`a313817`):

| workload | before | after | note |
|---|---:|---:|---|
| fibonacci | 3.04x | 2.41x | call/return-heavy recursion |
| closure_ops | 2.61x | 2.41x | closure calls + upvalues |
| binarytrees | 2.41x | 2.22x | mixed VM + allocation/GC |
| string_ops_long | 1.38x | 1.35x | small incidental win |
| table_ops_long | 0.39x | 0.39x | unchanged; already direct-table bound |
| overall | 1.71x | 1.45x | broad VM win |

Lesson: when a profile shows helpers whose C origin is a macro or static
inline, do a "macro boundary audit" before inventing a new algorithm. The
right Rust fix is often to restore the same code-generation boundary C had:
inline the helper, split cold paths away, or move the tiny operation back into
the dispatch arm.

### 7. `20260602T144939Z-ea6d8d4` — pattern-start and telemetry pass after PR #122

**Pattern:** *Skip impossible pattern starts, keep scalar barriers scalar, and
add telemetry where stack samples are too coarse.*

Starting matrix after PR #122
(`harness/bench/results/20260602T140413Z-858cc5e-compare.json`) to final
matrix (`harness/bench/results/20260602T144939Z-ea6d8d4-compare.json`):

| workload | before | after |
|---|---:|---:|
| binarytrees | 2.11x | 2.09x |
| closure_ops | 1.94x | 2.06x |
| fibonacci | 1.88x | 1.84x |
| gc_pressure | 2.50x | 2.50x |
| mandelbrot | 1.88x | 1.88x |
| mandelbrot_long | 1.82x | 1.82x |
| string_ops | 2.00x | 1.00x |
| string_ops_long | 1.86x | 1.51x |
| table_hash_pressure | 1.88x | 1.75x |
| table_ops | 1.00x | 1.00x |
| table_ops_long | 1.06x | 1.04x |
| overall | 1.60x | 1.54x |

What changed:

- Unanchored string matching now skips directly to source offsets that can
  match when the first pattern element is a required literal byte. Patterns
  whose first element can match zero bytes, is a class, is a capture/frontier,
  or is otherwise non-literal stay on the old byte-by-byte path.
- `string.gsub` now borrows its source and pattern strings through
  `GcRef<LuaString>` instead of copying both into `Vec`s, and pre-sizes the
  replacement buffer to the source length.
- Table string equality now checks full Lua string hash equality before the
  byte-compare fallback. Pointer equality still wins first; content equality
  for distinct string objects is preserved.
- The no-metatable `OP_SET*` path writes through the already-proven table
  reference directly instead of re-matching the `LuaValue` in
  `table_raw_set`.
- `SETUPVAL` skips the upvalue GC barrier call for scalar values. The shared
  `upvalue_set` fallback has the same guard. Collectable values still take
  the normal barrier.
- Added `LUA_RS_GC_PROFILE=<path|->` and `harness/bench/gc-profile.sh` for
  end-of-run GC counters: collection counts, heap cohorts, last mark/sweep
  stats, grayagain count, and intern-table size.

Profile evidence:

- Before the pattern skip, `string_ops_long_x5`
  (`harness/bench/profiles/20260602T142436Z-be7347f-string_ops_long_x5/summary.txt`)
  had `match_pat` at 29.2%.
- After the pattern skip, `string_ops_long_x5`
  (`harness/bench/profiles/20260602T144840Z-ea6d8d4-string_ops_long_x5/summary.txt`)
  had `match_pat` at 16.6%. The workload ratio moved from 1.86x to about
  1.5x in broad/focused runs.
- `closure_ops_x30` before the scalar barrier guard
  (`harness/bench/profiles/20260602T142517Z-be7347f-closure_ops_x30/summary.txt`)
  showed `gc_barrier_upval` at 4.6%. After
  (`harness/bench/profiles/20260602T144856Z-ea6d8d4-closure_ops_x30/summary.txt`)
  the barrier leaf disappeared, but wall ratio stayed around 2x because the
  remaining work is opaque `vm::execute`.
- `table_hash_pressure_x80`
  (`harness/bench/profiles/20260602T144847Z-ea6d8d4-table_hash_pressure_x80/summary.txt`)
  still has `concat` and `get_short_str_slot` as the top frames. The hash
  guard reduced `memcmp` modestly, but did not remove the table/string-key
  pole.
- GC profile telemetry for `gc_pressure`
  (`harness/bench/profiles/gc-profile/20260602T144913Z-ea6d8d4-gc_pressure/gc.tsv`)
  shows thousands of minor collections and only one full collection. The
  matching sampler
  (`harness/bench/profiles/20260602T144904Z-ea6d8d4-gc_pressure_x200/summary.txt`)
  still spreads time across VM dispatch, young sweep, allocator/free,
  table resize/new-key, barriers, and intern retention.

What did *not* change:

- No new unsafe.
- No skipped metamethod checks, finalizers, weak-table behavior, or collectable
  GC barriers.
- `gc_pressure` did not move. The new GC telemetry is useful, but this pass did
  not find a safe free lunch in pacer/phase mechanics.
- `closure_ops` remains a VM dispatch problem. Opcode counts can identify
  MOVE/GETUPVAL/SETUPVAL/CALL frequency, but the pre-spike tooling still
  lacked time attribution inside `lua_vm::vm::execute`.

### 8. `perf/vm-execute-attribution` — source-region VM attribution and safe frame/upvalue locality

**Pattern:** *When the profiler says "the VM", make the VM profileable before
guessing.*

The previous `closure_ops_x30` hotspot summary
(`harness/bench/profiles/20260602T144856Z-ea6d8d4-closure_ops_x30/summary.txt`)
showed `lua_vm::vm::execute` at about 89.8% of samples. That was true but not
actionable. The raw `/usr/bin/sample` call graph already carried source
lines/offsets for many `execute` frames; we just were not using them.

This pass adds `harness/bench/vm-execute-attribution.py` and wires it into
`profile-hotspots.sh`. When a sample contains `lua_vm::vm::execute`, the runner
now writes `vm-execute.txt` beside `summary.txt`. The report maps source lines
inside `crates/lua-vm/src/vm.rs` to buckets: frame setup, dispatch fetch,
opcode arms, return re-entry, inlined/unknown lines, and inclusive call context.
The main metric is self-samples, so an outer `OP_CALL` frame is not charged for
work done by the nested callee frame.

What the new attribution showed on the baseline `closure_ops_x30` sample:

| region | self samples | VM share |
|---|---:|---:|
| dispatch fetch | 1,829 | 30.3% |
| unknown/inlined | 723 | 12.0% |
| `OP_CALL` | 697 | 11.5% |
| `OP_SETUPVAL` | 598 | 9.9% |
| `OP_GETUPVAL` | 539 | 8.9% |
| frame setup | 495 | 8.2% |
| `OP_ADD` | 285 | 4.7% |
| `OP_FORLOOP` | 279 | 4.6% |

Measured safe-Rust packet:

- `execute` now caches the active Lua closure's code and constants at frame
  entry and fetches same-frame instructions directly from that slice.
- Hot scalar/load arms write through the proven stack slot directly instead of
  routing through tiny state helpers.
- Constant-bearing arms read from the cached constants slice.
- Closed upvalues store their canonical payload in `Cell<LuaValue>` instead of
  `RefCell<LuaValue>`, and scalar closed writes no longer refresh the legacy
  `UpValState` mirror unless a collectable value is involved.
- `GETUPVAL` and `SETUPVAL` expand the common open-current and closed paths in
  the opcode arm, preserving the shared fallback for cross-thread/open cases.

After the kept packet, the focused telemetry was noisy but directionally useful:

| artifact | binarytrees | closure_ops | fibonacci | mandelbrot | mandelbrot_long |
|---|---:|---:|---:|---:|---:|
| baseline `20260602T144939Z-ea6d8d4` | 2.09x | 2.06x | 1.84x | 1.88x | 1.82x |
| frame/code/constants packet `20260602T165133Z-876f9a0` | 1.95x | 1.94x | 1.82x | 1.75x | 1.84x |
| closed-upvalue packet `20260602T165534Z-876f9a0` | 2.05x | 1.88x | 1.87x | 1.88x | 1.85x |
| final kept closure-only probe `20260602T165805Z-876f9a0` | - | 1.94x | - | - | - |

The best attribution re-sample after the kept changes
(`harness/bench/profiles/20260602T165817Z-876f9a0-closure_ops_x30/vm-execute.txt`)
showed:

| region | self samples | VM share |
|---|---:|---:|
| dispatch fetch | 1,690 | 27.9% |
| `OP_CALL` | 788 | 13.0% |
| unknown/inlined | 694 | 11.5% |
| frame setup | 578 | 9.6% |
| `OP_SETUPVAL` | 470 | 7.8% |
| `OP_GETUPVAL` | 469 | 7.8% |
| `OP_FORLOOP` | 351 | 5.8% |
| `OP_ADD` | 314 | 5.2% |

Measured facts:

- The new tool resolves the former `execute` monolith into actionable buckets.
- The kept upvalue packet reduced sampled `GETUPVAL` / `SETUPVAL` self share
  versus the baseline sample.
- Wall-clock movement is real but modest and noisy: `closure_ops` probes ranged
  from 1.88x to 1.94x after the kept changes, versus 2.06x in the baseline
  broad pass.
- No new unsafe was added.

Negative result:

- A naive safe exact-Lua-call fast path for simple fixed-arity Lua calls was
  worse and was reverted. The focused matrix
  (`harness/bench/results/20260602T165955Z-876f9a0-compare.tsv`) had
  `closure_ops` at 2.00x and `fibonacci` at 1.95x, both worse than the kept
  packet. The signal is that duplicating frame setup inside `OP_CALL` is not a
  free lunch; the next call packet needs a deeper `CallInfo`/frame architecture
  change or should wait for a deliberate value/stack representation discussion.

Architecture comparison:

- C Lua keeps `cl`, `k`, `pc`, `base`, and `CallInfo *` as local/raw-pointer
  state in `luaV_execute`; `RA`, `RB`, `KC`, `vmfetch`, `savepc`, and upvalue
  reads are macro-shaped pointer operations.
- CppCXY/lua-rs pushes further toward that C shape: a 16-byte `LuaValue`
  union, cached raw `chunk_ptr` / `upvalue_ptrs` in `CallInfo`, direct stack
  pointer helpers, and an explicitly documented unsafe surface for hot
  stack/upvalue/value paths.
- Piccolo is useful as the opposite design point: mostly safe, sandbox-first,
  stackless/trampoline VM, safe enum `Value`, and cached current
  prototype/upvalues/registers in the loop. It is not trying to be a faithful
  C-Lua performance baseline.

Hypothesis after this pass: safe Rust still has local wins left in cold-path
splitting and call-frame layout, but the broad "obvious helper traffic" is
mostly gone. If `dispatch fetch`, `OP_CALL`, and value-stack copies remain the
top buckets after one more careful call-frame packet, the next meaningful
design discussion is likely unsafe/value layout: 16-byte `LuaValue`, raw stack
slot access with proven frame invariants, and cached upvalue pointer arrays.

### 9. `20260602T183215Z-98bd6bd` — concat pair fast path and repeatable short-workload telemetry

**Pattern:** *Do not intern scratch values that C only formats into a temporary
buffer.*

`table_hash_pressure` builds 266k string keys per run (`"k" .. i`, `"x" .. i`)
and then inserts/lookups those keys. The pre-packet profile made the bottleneck
obvious:

- Before: `table_hash_pressure_x100`
  (`harness/bench/profiles/20260602T182714Z-98bd6bd-table_hash_pressure_x100/summary.txt`)
  had `lua_vm::vm::concat` at 1,297 samples / 19.4% of wall time, with
  `get_short_str_slot` close behind at 18.9%.
- Opcode counts
  (`harness/bench/profiles/opcode-profile/20260602T182741Z-98bd6bd-table_hash_pressure/opcodes.tsv`)
  showed `CONCAT`, `SETTABLE`, `ADD`, and `FORLOOP` each at roughly 11-16% of
  executed opcodes, confirming this was workload structure rather than a
  sampler artifact.

The old two-operand concat path coerced numeric operands by interning the
number as a temporary Lua string, then allocated/copied again for the final
concatenated key. The kept packet adds a `total == 2` string/number fast path:
format numeric operands into byte buffers, append both operands directly into
the final buffer, intern only the final concatenated string, and leave
metamethod behavior unchanged for non-string/non-number operands.

Evidence:

| workload | recent baseline | after |
|---|---:|---:|
| table_hash_pressure, broad best-of-5 | 1.75x-2.00x band | 1.14x |
| table_hash_pressure, focused best-of-10 | 1.75x | 1.17x |
| overall broad matrix | ~1.54x | 1.54x |

After-profile:

- `table_hash_pressure_x100`
  (`harness/bench/profiles/20260602T183359Z-98bd6bd-table_hash_pressure_x100/summary.txt`)
  has `lua_vm::vm::concat` down to 116 samples / 3.1%.
- The new top cost is now split across `execute`, table short-string lookup,
  `new_key`, intern allocation, and allocator/free. That means the old concat
  scratch-intern pole is gone.

Telemetry improvements kept with this packet:

- `profile-hotspots.sh` gained `PROFILE_REPEAT=N`, so short workloads can be
  sampled without hand-writing a custom eval payload.
- `opcode-profile.sh` and `gc-profile.sh` now support the same repeat mechanism
  and label artifacts as `<workload>_x<N>`.
- `history.py` now includes `gc_pressure`, `mandelbrot_long`, and
  `table_hash_pressure` in the dashboard workload registry instead of silently
  filtering their ledger rows out.

Negative results from the same spike:

- Guarding unconditional `ci_trap` refreshes behind `hookmask != 0` was not a
  free lunch. It looked plausible from VM attribution, but a best-of-10
  isolation moved `binarytrees` from 1.93x on clean baseline
  (`20260602T182315Z-98bd6bd`) to 2.12x with the guard
  (`20260602T181943Z-98bd6bd`). The branch/read tradeoff and code layout were
  worse than the unconditional `ci_trap` read in allocation-heavy recursion.
- Rewriting `OP_MUL` to manually load operands once also did not hold up.
  `mandelbrot_long` got worse in focused runs, so the change was dropped.
  The lesson is that opcode arm shape needs benchmark confirmation; fewer
  source-level probes are not automatically better machine code.

### 10. `perf/vm-value-telemetry` — post-v0.0.27 VM opacity and layout spike

**Pattern:** *When the remaining gap looks architectural, measure the
architecture before rewriting it.*

After the concat packet, the broad best-of-5 matrix
(`harness/bench/results/20260602T183215Z-98bd6bd-compare.tsv`) had the current
shape:

| workload | ratio | read |
|---|---:|---|
| table_ops | 1.00x | at parity |
| table_ops_long | 1.05x | near parity |
| table_hash_pressure | 1.14x | concat scratch-intern pole removed |
| string_ops_long | 1.59x | still around the gate |
| mandelbrot_long | 1.82x | arithmetic + dispatch |
| fibonacci | 1.87x | call/return recursion |
| binarytrees | 1.93x | allocation + traversal + GC |
| closure_ops | 1.94x | closure calls + upvalues |
| gc_pressure | 2.50x | allocation/collection throughput |

The new VM attribution samples make the next call/frame problem concrete:

- `closure_ops_x40`
  (`harness/bench/profiles/20260602T184054Z-18c5f24-closure_ops_x40/vm-execute.txt`)
  has `lua_vm::vm::execute` at 90.3% of sampled wall time. Inside execute:
  dispatch fetch is 26.8% of VM self-samples, `OP_CALL` 13.3%,
  frame setup 9.7%, `OP_SETUPVAL` 7.6%, `OP_GETUPVAL` 7.0%, and
  return re-entry 2.4%.
- `fibonacci_x2`
  (`harness/bench/profiles/20260602T184108Z-18c5f24-fibonacci_x2/vm-execute.txt`)
  has `execute` at 100% of samples. Inside execute: dispatch fetch is 22.6%,
  unknown/inlined samples 15.5%, `OP_CALL` 14.1%, `OP_ADD` 11.9%, `OP_SUB`
  7.7%, frame setup 7.5%, `OP_GETUPVAL` 6.4%, and `OP_RETURN1` 5.5%.

The measured layout probe is the important correction to earlier guesses. Run:

```bash
bash harness/bench/value-layout.sh
```

Current Apple M3 Max output:

| type | Rust size/align | C Lua 5.4.7 size/align | note |
|---|---:|---:|---|
| `LuaValue` / `TValue` | 16 / 8 | 16 / 8 | value size is not the current gap |
| `StackValue` | 24 / 8 | 16 / 8 | Rust stores `tbc_delta` beside the value; C overlays it in a union |
| `CallInfo` | 72 / 8 | 64 / 8 | Rust uses indices/options and enum payloads |
| `LuaState` / `lua_State` | 176 / 8 | 200 / 8 | Rust thread state is not larger overall |
| `LuaTable` / `Table` | 104 / 8 | 56 / 8 | table object layout is a real representation target |
| `UpVal` / `UpVal` | 64 / 8 | 40 / 8 | upvalue representation remains costly |
| `LuaProto` | 208 / 8 | - | no direct C row in this probe yet |

Tooling inventory is now explicit:

```bash
bash harness/bench/profile-inventory.sh
```

Available on the current host: macOS `sample`, `xctrace`, `leaks`, DTrace,
Cargo/rustc, and `inferno-flamegraph`. Missing on this host: `samply` and
Linux `perf`. Existing repo probes are `compare.sh`, `profile-hotspots.sh`,
`vm-execute-attribution.py`, `opcode-profile.sh`, `gc-profile.sh`,
`value-layout.sh`, and `history.py`.

Architecture comparison:

- C Lua keeps active frame state in local/raw-pointer form: instruction
  pointer, base, constants, function closure, and `CallInfo *`.
- CppCXY/lua-rs takes the same direction more aggressively: `LuaValue` is a
  C-shaped 16-byte union/tag pair, `CallInfo` caches raw `chunk_ptr` and
  `upvalue_ptrs`, the VM loop carries `FrameCtx`/`ActiveFrame` local state,
  and the exact Lua-call path tries to avoid a flush -> `precall` -> reload
  round trip.
- That implementation is not a safe-Rust counterexample. A local clone showed
  hundreds of `unsafe` hits across raw GC pointers, stack access, value unions,
  and compiler/runtime pointer plumbing. Treat it as evidence for what a
  future reviewed boundary might look like, not as a drop-in pattern.

What this says about the roadmap:

- Safe Rust is not cooked. The next safe packet should target call/frame
  shape: fewer active-frame reloads, a cleaner hot Lua-call entry path, and
  possibly a narrower `CallInfo`/active-frame cache. The previous naive exact
  call fast path regressed, so this should be designed around frame state, not
  duplicated `precall` logic.
- Dispatch cold-path splitting remains plausible but subtle. A no-hook
  `ci_trap` guard regressed `binarytrees`, so the next split needs source
  attribution and matrix confirmation, not a branch-count argument.
- The unsafe/design ceiling is now narrower and more concrete: not
  "`LuaValue` is too large", but stack-slot shape, unchecked stack access under
  frame invariants, cached frame/upvalue pointers, and larger table/upvalue
  heap objects.

Tool gaps:

- `sample` plus `vm-execute.txt` still leaves `UNKNOWN_INLINED` samples and
  cannot provide exact per-op timing.
- `opcode-profile.sh` gives execution counts but not time per opcode.
- `gc-profile.sh` gives end-of-run counters but not allocation stack
  attribution or cumulative per-phase timing.
- `harness/runners.toml` still has correctness runners only. Benchmark,
  profile, GC, opcode, layout, and inventory probes are reusable scripts but
  not typed runner entries with resource locks.
- `xctrace`, DTrace, and `leaks` are available on this host, but the repo does
  not yet have wrappers that turn them into stable artifacts.

### 6. `20260602T140413Z-858cc5e` — broad safe-Rust pass after PR #121

**Pattern:** *Use specialized internal data structures for internal identities,
and skip generic table-set scaffolding only when the metatable precondition
proves it unnecessary.*

Starting matrix after PR #121
(`harness/bench/results/20260602T134659Z-858cc5e-compare.json`) to final
matrix (`harness/bench/results/20260602T140413Z-858cc5e-compare.json`):

| workload | before | after |
|---|---:|---:|
| binarytrees | 2.49x | 2.11x |
| closure_ops | 2.12x | 1.94x |
| fibonacci | 1.84x | 1.88x |
| gc_pressure | 3.00x | 2.50x |
| mandelbrot | 1.75x | 1.88x |
| mandelbrot_long | 1.79x | 1.82x |
| string_ops | 2.00x | 2.00x |
| string_ops_long | 2.14x | 1.86x |
| table_hash_pressure | 2.00x | 1.88x |
| table_ops | 1.00x | 1.00x |
| table_ops_long | 1.04x | 1.06x |
| overall | 1.64x | 1.60x |

What changed:

- The global short-string intern map stopped using Rust's default hasher for
  byte keys. The replacement is an internal fast byte hasher plus `entry` for
  new short strings, preserving intern semantics while removing the
  `DefaultHasher::write` pole from string-key workloads.
- `OP_SETTABLE`, `OP_SETI`, and `OP_SETFIELD` now set directly when the target
  is definitely a table with no metatable. That is the same result
  `finish_set` would reach after checking `__newindex`; the GC table barrier
  still runs.
- GC pointer-identity maps/sets (`Marker.visited`, allocation tokens) use an
  internal identity hasher. These keys are heap addresses, not user table keys,
  so the change removes SipHash-style overhead without changing Lua-visible
  hashing or equality.
- Added feature-gated opcode count telemetry:
  `cargo build --release -p lua-cli --features opcode-profile` plus
  `harness/bench/opcode-profile.sh`. Normal builds have no dispatch-loop
  counter branch.

Profile evidence:

- Pre-pass `table_hash_pressure_x80`
  (`harness/bench/profiles/20260602T134940Z-858cc5e-table_hash_pressure_x80/summary.txt`)
  had `DefaultHasher::write` at 19.9%.
- Post-pass `table_hash_pressure_x80`
  (`harness/bench/profiles/20260602T140609Z-858cc5e-table_hash_pressure_x80/summary.txt`)
  has no default-hasher pole; remaining top frames are real key construction
  and table lookup (`get_short_str_slot`, `concat`, raw set/new key).
- `fibonacci` opcode telemetry
  (`harness/bench/profiles/opcode-profile/20260602T135755Z-858cc5e-fibonacci/opcodes.tsv`)
  shows the opaque `vm::execute` time is spread across `LOADI`, `CALL`, `LTI`,
  `RETURN1`, `GETUPVAL`, `SUB`, and `ADD`, not one obvious arithmetic helper.
- `closure_ops` opcode telemetry
  (`harness/bench/profiles/opcode-profile/20260602T135818Z-858cc5e-closure_ops_x10/opcodes.tsv`)
  shows `MOVE` + `GETUPVAL` at about 39.6% of executed opcodes, with
  `SETUPVAL`, `CALL`, `ADD`, and `RETURN1` behind them.

What did *not* change:

- No skipped metatable checks except in the proven no-metatable table case.
- No skipped GC barriers, weak/finalizer behavior, or error checks.
- No new unsafe.
- `string_ops_long` still has `match_pat` as the top frame
  (`harness/bench/profiles/20260602T140609Z-858cc5e-string_ops_long_x5/summary.txt`).
  A meaningful next packet is likely pattern precompile/cache or a deeper
  matcher rewrite, not another local allocation tweak.

### 5. `20260602T132632Z-2d5cffe` — focused safe-Rust pass on string/GC poles

**Pattern:** *Keep semantics, restore the C shape, and remove scratch work that
proved unnecessary.*

Starting point after PR #120 on the current HEAD (`2d5cffe`):

| workload | before |
|---|---:|
| binarytrees | 2.54x |
| gc_pressure | 3.50x |
| string_ops_long | 2.72x |
| focused overall | 2.67x |

Final focused matrix (`harness/bench/results/20260602T132632Z-2d5cffe-compare.json`):

| workload | after |
|---|---:|
| binarytrees | 2.23x |
| gc_pressure | 3.00x |
| string_ops_long | 2.23x |
| focused overall | 2.24x |

What changed:

- Tiny VM/table/string helpers that profile showed as leaf cost were made
  inline-only. The helper frames disappeared on re-sample.
- `string.gmatch` stopped using a four-slot Lua table as iterator state and
  now mirrors C-Lua's closure shape: source string, pattern string, userdata
  state. The userdata host payload stores only byte positions; strings remain
  traced closure upvalues.
- Young sweep now records only swept objects that are in the old-revisit list,
  instead of hashing every swept object into a processed set.
- `Heap` maintains a heap-owned object count and uses it to reserve
  `Marker.visited` at cycle start. The scaled `gc_pressure_x100` profile no
  longer shows `reserve_rehash` in the top 25.
- `profile-hotspots.sh` now accepts `PROFILE_LUA_EVAL` so short workloads can
  be sampled as scaled probes without creating throwaway workload files.

What did *not* change:

- No skipped GC barriers, weak-table rules, finalizer behavior, or metamethod
  semantics.
- No benchmark-only fast paths.
- No new unsafe.
- The remaining `gc_pressure` gap is real marker/table allocation work:
  marker visited insertion/lookup, table construction, sweep/free, and
  interned-string retention. That is not a conclusion that safe Rust is
  exhausted; it is the next evidence-backed packet boundary.

### 5. `2d5cffe` — typed table write barriers instead of `dyn Any`

**Pattern:** *Don't erase a type the hot path already knows.*

PR #120 started as traceback/GC scratch work, but the profiling pass exposed a
separate table hot path: `table_ops_long` was spending most of its sampled time
inside dynamic GC-barrier dispatch. The pre-fix sample
(`harness/bench/profiles/20260602T121328Z-26c831f-table_ops_long/summary.txt`)
showed:

- `barrier_any`: 31.0%;
- `barrier_child_any`: 22.9%;
- `Any::type_id`: 7.2% across two monomorphized frames.

That shape is not a Lua algorithm. It is a Rust abstraction leak: table
mutation already has a `GcRef<LuaTable>`, but the barrier path erased the type
to `dyn Any` and recovered it at runtime. C-Lua's barrier macros never pay a
runtime type-id lookup for this case; the object type is known by the call
site.

Fix: add typed table-barrier paths for table mutation and raw-set call sites
that already hold `GcRef<LuaTable>`. The barrier still runs. The child value is
still traced. The only removed work is dynamic type recovery.

After the fix, the profile
(`harness/bench/profiles/20260602T122145Z-26c831f-table_ops_long/summary.txt`)
had no `barrier_any`, `barrier_child_any`, or `Any::type_id` leaf frames. The
remaining samples were real table work:

- `LuaTableRefExt::raw_set_int`: 44.0%;
- `barrier_lua_value`: 28.2%;
- `table_lib::insert`: 21.0%;
- `table_lib::remove`: 6.8%.

Bench impact on the same Apple M3 Max runner:

| workload | before | after | evidence |
|---|---:|---:|---|
| table_ops | 2.50× | 1.25× | `20260602T120610Z` -> `20260602T122050Z` |
| table_ops_long | previous Linux ledger 1.78×; no same-host pre-row | 1.00× | `20260602T122050Z` |

Guardrail: a tempting follow-up was to skip barriers for primitive child
values. That was not kept; the experiment broke the warning/finalizer CLI path.
The lesson is narrow: **avoid type erasure when the type is already statically
known**. Do not weaken GC-barrier semantics to get the same shape.

## The patterns by name

Distilled from the representative commits above, plus the redis-rs-port hotpath
methodology we adapted:

### Pattern 1: Use the fast path that already exists

C codebases routinely have two implementations of the same operation: a
generic one for the rare case (e.g. `lua_gettable` for any key type with
metamethod support) and a specialized one for the common case (e.g.
`luaH_getint` for raw integer-key array access). A naive Rust port often
routes everything through the generic path, paying generic-path overhead
on every call.

**The fix is to find the spot in the C source where the optimization happens
and replicate that structure, not the surface API.**

### Pattern 2: Cache the precondition, don't disable the feature

When a feature has overhead even in the inactive case (GC step, slowlog,
WATCH, line hooks, metatable lookups), the fix is a cheap precondition check
that bails *before* the expensive prep work. NOT a feature flag.

The trap shape: the inactive case eventually does early-return inside the
feature's main function, but only AFTER snapshot/prep allocations have
already fired. Move the check up.

### Pattern 3: Match upstream's structure, not just upstream's behavior

This is the meta-pattern. If C-Lua's `lua_geti` has an integer-key fast path
that skips metamethod handling, our `table_get_i` must have one too. If
C-Lua's `vmdispatch` macro inlines the opcode fetch into the loop body, our
match arm must not call into a separate function for it.

A faithful port matches structure. The benchmark gap *is* the surface area
where the port chose a different structure than C-Lua. Profiles reveal
exactly which differences are paying off and which are pure waste.

### Pattern 4: Don't clone where C uses pointers

C-Lua's hot opcode handlers operate on `TValue *` — pointers into the stack.
Our naive port writes:

```rust
let v1 = state.get_at(rb).clone();   // drops the borrow on `state`
let v2 = state.get_at(rc).clone();
helper(state, &v1, &v2, ...);        // pass &mut state + immutable refs
```

We clone because the borrow checker will not let us hold `&LuaValue` into
`state` while also passing `&mut state` to a helper. **But the clone is not
required by safety — it's required by the *shape* we chose for the helper.**

The fix is to either:

- inline the operation into the match arm so no helper is called;
- pass indices instead of values, and have the helper re-read;
- extract just the needed primitive (`i64`, `f64`) without cloning the
  enum;
- split-borrow via dedicated reader methods that return owned primitives.

None require `unsafe`. All require thinking carefully about hot-path
shapes.

### Pattern 5: Inline-friendly accessor discipline

Single-instruction operations (bit-shifts, masks, enum tag reads) should
NOT be function calls on the hot path. `#[inline]` is the cheap fix; sometimes
it's not enough and you have to flatten the abstraction.

The stronger version: **C macros and `static inline` helpers are performance
architecture.** Treat them as "must compile away" contracts. A translated
Rust method may be behaviorally faithful and still be structurally wrong if it
survives as a call frame in profiles.

Macro-boundary audit procedure:

1. When a hot Rust frame appears, find its C origin. If the C source spells it
   as `#define`, `l_sinline`, `static inline`, or a one-expression helper, it
   was probably intended to disappear into the caller.
2. Check whether the Rust equivalent crosses a crate/module boundary, is
   generic, returns an owned value, or calls through another wrapper. Those are
   common reasons LLVM stops inlining.
3. First try `#[inline(always)]` on the tiny helper and its immediate wrapper.
   If the body is large, split the cold/error/metamethod path into
   `#[cold] #[inline(never)]` so the hot path is inlineable.
4. Re-sample. A successful fix makes the helper frame disappear and the parent
   hot frame grow; only keep the packet if the workload wall ratio also moves.

Evidence from this port: `d8c1423` inlined VM stack and call accessors that
map to C-Lua macros / `l_sinline` helpers. `fibonacci` moved 3.04x -> 2.41x
and the separate `precall`, `prep_call_info`, `set_ci_previous`, and `set_top`
frames vanished from the profile.

Assertion macros need the same audit. C-Lua's dispatch loop contains
`lua_assert(isIT(i) || (cast_void(L->top.p = base), 1))`; because normal
upstream builds compile `lua_assert` to `((void)0)`, that side-effectful
expression is debug-only. Porting it as unconditional Rust release work added
an opcode-mode lookup and stack-top write to every dispatch tick. `7e32098`
moved that invalidation behind `#[cfg(debug_assertions)]`; dev and release
official suites stayed 44/44, and the matrix moved overall 1.39x -> 1.33x
(`fibonacci` 2.26x -> 2.10x). If a hot C macro is an assertion/check macro,
first determine whether the side effect exists in the upstream release build.

The same principle applies to eager cleanup added by the port. Upstream
`prepCallInfo` does not clear a new Lua frame's reserved register tail; stack
cleanup is paid later by GC/thread traversal machinery. The Rust port was
clearing that range on every Lua call. `97b3c4c` removed the per-call clear
after dev and release official suites stayed 44/44, moving overall 1.31x ->
1.25x and `fibonacci` 2.07x -> 1.93x. Treat "make stale slots tidy now" as a
performance-sensitive semantic claim, not a free safety improvement.

### Pattern 6: Don't erase types the hot path already knows

Generic helpers are fine at API boundaries. They are expensive inside a loop
when the caller already knows the concrete object type.

The GC write barrier is the concrete example. A broad
`barrier_any(parent: &dyn Any, child: &dyn Any)` shape let many object kinds
share one path, but table mutation already holds `GcRef<LuaTable>`. Passing
that through `dyn Any` forced runtime type-id checks at exactly the spot C-Lua
spells as a tiny barrier macro. PR #120 replaced that path with typed table
barriers and removed `barrier_any`, `barrier_child_any`, and `Any::type_id`
from the `table_ops_long` profile.

Rule: if a profiler shows `Any`, trait-object dispatch, vtable calls, erased
enum wrappers, or dynamic downcasts in a hot path, ask whether the caller
already has a concrete type. If it does, add a typed sibling helper and leave
the generic helper for cold/dynamic call sites.

This is not permission to skip correctness work. In the table-barrier case the
barrier still ran; only dynamic type recovery disappeared. An attempted
"primitive child values do not need barriers" shortcut failed a finalizer /
warning path and was dropped.

### Pattern 7: Wall-clock sampling beats hypothesizing

Every meaningful win in this session came from a `/usr/bin/sample` profile
artifact. The hypothesis "table_ops is slow because it's quadratic" was
wrong (it was a missed fast path). The hypothesis "fibonacci is slow because
of call dispatch" was wrong (it was malloc churn inside the GC). Without
profile data, both fixes would have aimed at the wrong target.

The redis-rs-port discipline: every perf-shaped commit links the profile
artifact that motivated it. We follow the same rule.

## What may require representation work (the last increment to parity)

The list below is what we may not recover with local safe-Rust hot-path
patches alone. Some items could still be addressed with safe layout redesign;
others are candidates for carefully-scoped `unsafe` once safe-Rust work
plateaus. None of these are blockers. They become relevant for the final
1.0–1.2× push toward parity.

- **C-shaped stack slots.** The current measured layout is `LuaValue` 16 bytes,
  matching C Lua 5.4.7's `TValue`, but Rust `StackValue` is 24 bytes while C
  `StackValue` is 16 bytes because C overlays the to-be-closed delta in a
  union. Recovering stack-slot parity may require a side structure for
  to-be-closed metadata or an unsafe C-shaped union. Do not claim value-size
  pressure until `harness/bench/value-layout.sh` says so.

- **Raw pointer stack access.** C-Lua reads `s2v(L->top.p - 1)` as a single
  pointer load. Our `state.get_at(idx)` indexes a `Vec<StackValue>` with
  whatever bounds-check LLVM can or can't elide. Profile evidence so far
  suggests this is ~5% overhead, not the dominant cost.

- **Cached frame and upvalue pointers.** C-Lua's `luaV_execute` keeps the
  active closure/prototype/constants/pc/base in local pointer state. CppCXY's
  Rust implementation copies that shape with raw `chunk_ptr` and
  `upvalue_ptrs` in `CallInfo` and unchecked stack helpers. Our safe port has
  moved partway there with cached code/constants, but the current profiles
  still show frame setup, `OP_CALL`, and return re-entry as real buckets.

- **Computed gotos / true threaded dispatch.** C-Lua's `vmdispatch` macro
  emits `goto *jump_table[opcode]` after each opcode. There is no Rust
  equivalent in safe code. LLVM compiles our `match` to a similar jump
  table, but the loop header overhead is real — ~5–10%.

- **Tail-call optimization for the dispatcher.** A common pattern in
  hand-written interpreters (LuaJIT, V8 baseline) is to make each opcode a
  function that tail-calls the next. Rust does not guarantee TCO; the
  pattern is unreliable. Some projects use `become` (experimental) or
  `musttail` (LLVM attribute, not exposed) but those require nightly.

Combined, the unsafe-only opportunities account for maybe 1.15–1.25× of
additional speedup. **That's the difference between "well within striking
distance of parity" and "actually at parity."** Worth doing once we've
exhausted the safe-Rust hot-path work — same trade-off redis-rs-port
made for its network buffer and FFI code paths.

## Future: where a JIT would fit

The interpreter we're porting *isn't a stopping point*. It's the substrate
for a future JIT.

### The pattern

Every mature dynamic-language JIT keeps the interpreter alive and adds a
JIT alongside it. The interpreter remains the canonical execution path
and the deopt target; the JIT is an *optimization that can fail* (guard
violations) and fall back to the interpreter.

```
                  ┌──────────────────────────────┐
parse → bytecode  │                              │
       │          │  Interpreter (lua-rs today)  │
       ├──────────►  - canonical execution       │
       │          │  - deopt target              │
       │          │  - cold code stays here      │
       │          └──────────────────────────────┘
       │                       ▲
       │                       │ deopt on guard failure
       │                       │
       │          ┌────────────┴─────────────────┐
       │          │  Tracer / profiler           │
       └──────────►  - watch interpreter         │
                  │  - identify hot loops/funcs  │
                  └──────────┬───────────────────┘
                             ▼
                  ┌──────────────────────────────┐
                  │  JIT compiler                │
                  │  - bytecode/trace → IR       │
                  │  - IR → native code          │
                  │  - emit type guards          │
                  └──────────┬───────────────────┘
                             ▼
                  ┌──────────────────────────────┐
                  │  Native code cache           │
                  │  - executable pages          │
                  │  - per-trace/method          │
                  └──────────────────────────────┘
```

### Prior art (read these before designing)

- **YJIT** — Ruby's JIT, written in Rust, in production at Shopify and
  GitHub since 2021. Closest analog to "Rust JIT bolted onto an existing
  C-ported interpreter." ~30–50% speedup over MRI's interpreter.
  Architecturally a basic block / method JIT with custom assembler.
- **LuaJIT** (Mike Pall) — tracing JIT for Lua 5.1 in hand-written x86/ARM64
  assembly. The reference for "fast Lua." Architectural template; we
  wouldn't recreate the hand-assembly approach.
- **Wasmtime + Cranelift** — production codegen library used by Wasmtime,
  Wasmer, others. The realistic Rust-native option for emitting native
  code from typed IR. Would be the codegen backend for a lua-jit project.
- **PyPy** — meta-tracing JIT for Python. Different philosophy (generates
  the JIT from the interpreter spec) but proves the substrate pattern.
- **MoarVM** — VM for Raku with JIT. Less directly relevant but the design
  notes are educational.

### Three tiers of ambition

**Tier 1: method JIT via Cranelift (realistic 6–12 month project).**
Compile whole Lua functions to native code via Cranelift's SSA IR. Emit
type guards (e.g. "this register is `Int`") that deopt to the interpreter
on failure. Expected 2–5× over interpreter on numeric/call-heavy code.
Cranelift is real production codegen — you don't write your own
assembler.

**Tier 2: tracing JIT (LuaJIT-grade, multi-year).** Record traces through
hot loops, compile linear bytecode sequences with type specialization,
support side exits. Where the 10–50× numbers come from. Substantially
harder — trace assembly, deopt at arbitrary points, type-narrowing
inference.

**Tier 3: template JIT / baseline compiler (3–6 months as a first step).**
Compile each bytecode opcode to a small native template, stitch them
together, no type specialization. Maybe 1.5–2× over interpreter. What YJIT
started as. A good warm-up before Tier 1.

### What our current choices buy us toward this (good)

- **Faithful Lua 5.4 bytecode preserved.** Register-based bytecode maps
  cleanly to SSA IR (Cranelift, LLVM). Stack-based bytecodes are harder.
  We're already in the easy regime.
- **Deterministic interpreter, no hidden state.** Deopt requires
  reconstructing interpreter state from JIT state. Clean state model →
  tractable deopt.
- **Mark-and-sweep GC with explicit roots via `Trace`.** A JIT will need
  to emit stack maps (metadata telling the GC where references live in
  JIT'd frames at each safepoint). Our `Trace` discipline is a clean
  abstraction to extend. Harder GCs (concurrent, compacting) would be
  much harder.
- **`unsafe_code = "forbid"` in interpreter doesn't constrain the JIT.**
  The JIT itself will use `unsafe` (executable pages, calling convention
  juggling) but that's contained in a future `lua-jit/` crate.

### What might cause friction later (worth designing for now)

- **Stack and frame layout.** `LuaValue` itself currently measures 16 bytes,
  but `StackValue`, `CallInfo`, table objects, and upvalues are larger than
  their C counterparts. A JIT wants to specialize register representation —
  keep values in CPU registers as raw `i64` / `f64`, only "boxing" back into
  `LuaValue` at safepoints. Not blocking, but worth knowing.
- **`RefCell`-wrapped state fields.** A JIT can't `borrow_mut` a RefCell
  from native code easily; it needs raw pointer access. Refactoring some
  state for JIT-friendliness is a real cost.
- **`GcRef` indirection layers.** A JIT wants to load a table's field
  with one `mov rax, [rdi+offset]`. If our `GcRef` has multiple layers of
  indirection, the inlined access path gets messy. Keep the `GcRef`
  abstraction thin.

### Three things to bake in NOW (cheap to do, expensive to undo)

1. **Safepoints are explicit.** GC only runs at known points. Our
   `gc_check_step` from `precall` is essentially the safepoint
   mechanism — keep that pattern. A future JIT will emit explicit
   safepoint checks at loop back-edges and call sites; the interpreter's
   safepoint discipline is the model.

2. **Value type tests are local, not metatable-traversing.** If
   `is_integer(v)` reads a single tag, the JIT can emit a single
   compare-and-branch. If it walks metatables, the JIT has to bail. Keep
   type tests cheap.

3. **No interpreter logic the JIT can't replicate.** If the interpreter's
   behavior depends on "what data structure is currently checked out via
   RefCell," the JIT will struggle. Stick to "input state → output state"
   semantics in the dispatch loop.

We are already doing all three by accident, because faithful C-Lua porting
demands it. Worth noticing so we don't lose it.

## Distilled rules for the next C → Rust port (the takeaway)

If you are starting another systems-level C → Rust port and want to hit
performance parity (or close), the lessons from this one in priority order:

1. **Build the bench loop before you need it.** A working `compare.sh`
   that puts a ratio number on the dashboard for every commit is the
   single most valuable piece of infrastructure. Without it, you'll
   optimize the wrong thing.

2. **Build the profile harness before you need it.** A `/usr/bin/sample`
   wrapper that captures the top frames in 10 seconds is the difference
   between "informed surgery" and "lucky guess." See
   `harness/bench/profile-hotspots.sh`.

3. **Port faithfully first, then optimize.** A working faithful port is
   the substrate. Out-of-the-box ratios of 5–10× are NORMAL. They are not
   the verdict. They are the starting point.

4. **Most gaps are non-faithful Rust idioms, not Rust safety.** Clones
   instead of pointers, `RefCell` instead of direct field access,
   allocations in cold-frequent code, accessor methods that should be
   inlined. The taxonomy table at the top of this doc applies.

5. **Profile. Don't guess.** The big wins in this port came
   from profile evidence pointing at things we wouldn't have suspected
   (a missed fast path on `getn`, GC bookkeeping allocations, dynamic
   table-barrier dispatch). Hand inspection of the code suggested
   different targets.

6. **"Use the fast path that already exists" is the highest-leverage
   pattern.** C codebases have two impls of everything: generic for the
   rare case, specialized for the common one. Routing everything through
   the generic path is the default Rust port mistake.

7. **Treat C macros as code-generation evidence.** If upstream wrote a hot
   operation as a macro, `l_sinline`, or `static inline`, the Rust version
   should compile away too. If profiling shows it as a frame, audit the
   abstraction boundary before looking for a bigger algorithmic rewrite.

8. **Do not preserve debug-only macro side effects in release.** C assertion
   macros can hide assignments used only for invariant checking. Before
   translating that expression literally, verify whether upstream release
   builds compile it out.

9. **"Cache the precondition" is the second.** Features that short-circuit
   on inactive cases need the short-circuit at the outer layer, not
   inside.

10. **Do not type-erase known hot-path objects.** Generic `dyn Any` or
   downcast-based helpers belong at dynamic boundaries. If the caller
   already has `GcRef<LuaTable>` or another concrete type, add a typed
   sibling helper and preserve the same semantics.

11. **Every commit cites its evidence.** Profile artifact path in the
   commit body. Dashboard row showing before/after. 44/44 oracle test
   gate. No exceptions, even on "obvious" fixes — the GC fix looked
   obvious AFTER the profile said so.

12. **The interpreter is the substrate, not the endpoint.** Plan for a
   future JIT — keep safepoints explicit, type tests cheap, semantics
   clean. None of this costs anything if the C source you're porting
   already does it.

13. **`unsafe_code = "forbid"` is not the bottleneck. Sloppy idioms are.**
    The gap you should worry about is the one between "Rust that
    type-checks" and "Rust that compiles to the same machine code C
    does." That gap is most of the work. Safety is rarely where the
    time goes.

## What goes here next

This doc is a living journal. When a new perf commit lands, the lessons
that generalize should make their way back into "The patterns by name" or
"Distilled rules" above. Things to add as we encounter them:

- Lua pattern matcher precompile/cache investigation if future samples keep
  `match_pat` near the top after the literal-start skip and allocation fixes.
- Follow-ups enabled by primitive accessors on ORDER/BITWISE opcodes.
- The `RefCell`-on-hot-path audit (deeper refactor when ready).
- Remaining GC pressure allocation/accounting work after the PR #120 scratch
  pre-sizing pass.
- The first time we add a Cranelift dependency and emit one native opcode
  handler. (No timeline. Tier 3 baseline JIT as a thought experiment.)
- Whatever pattern emerges from running the same playbook on
  `redis-rs-port` and `nginx-rs-port`. If the lessons transfer, the
  methodology is the real product.
