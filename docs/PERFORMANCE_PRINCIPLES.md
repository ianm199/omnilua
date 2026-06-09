# Performance principles for lua-rs

For current agent handoff, active bottleneck model, tool map, and packet
candidates, start with `docs/PERFORMANCE_MODEL.md`.

Adapted from the redis-rs-port hotpath methodology
(`redis-rs-port/docs/RUNTIME_OWNER_HOTPATH_PUSH.md`,
`redis-rs-port/docs/BENCHMARKS.md`,
`redis-rs-port/docs/history/HARNESS_LEARNINGS.md`). Same discipline, adapted
for an interpreter rather than a network server.

The headline rule: **perf work is evidence-driven, not intuition-driven.**
Every commit that claims to improve performance has to cite an artifact
that proves it. The dashboard at `harness/bench/history/index.html` is the
audit trail.

## The reference is the target

Upstream Lua 5.4.7 is a 30-year-old, heavily micro-optimized C
interpreter. **The goal is parity-quality evidence**: ratios around 1.0× where
the faithful safe-Rust port has no structural reason to be slower, and clear
profile-backed explanations where it still is. Some workloads may beat C in a
microbench shape; that is not the goal. Regressions and unexplained >2× gaps
are backlog until measured otherwise.

The current scorecard, tall-pole ranking, and packet candidates live in
**`docs/PERFORMANCE_MODEL.md`** — the single source of truth for state.
Hand-copied scorecards in this file rotted (a v0.0.31-era matrix sat here
while the model doc carried v0.0.32) and were removed on 2026-06-09; this
file keeps only the rules and the named patterns. Experiment-by-experiment
history lives in `docs/MATCHING_C_PERFORMANCE.md`, and the active work plan
is `docs/PERF_PUSH_SPEC.md`.

## The gate

Every performance change goes through the same 4-step verification.
Refusal to honor this gate is the single biggest source of perf-work
self-deception:

```text
implementation
    ↓
44/44 official suite green (correctness wall)
    ↓
harness/bench/compare.sh shows the targeted workload improves
    ↓
profile artifact under harness/bench/profiles/ shows the previous top
frame is no longer top, or has shrunk by the predicted amount
```

When the previous top frame is `lua_vm::vm::execute`, read the adjacent
`vm-execute.txt` from `profile-hotspots.sh` before forming the next packet.
The summary frame is too coarse; the source-region report can usually separate
dispatch fetch, `OP_CALL`, upvalue traffic, arithmetic, and return re-entry.

If any step fails, the commit doesn't land. Profiling data that
contradicts the hypothesis is the most valuable data you have — it
means the hypothesis was wrong, and the fix would have been a coincidence.

## The packet shape (when filing perf work)

Borrowed unmodified from the redis hotpath methodology. Every perf
investigation should be written up in this shape, even when you're doing
it solo in one session:

1. **Hypothesis.** One sentence: what we think the bug is. Link the
   profile artifact that suggests it.
2. **Source anchors.** Reference C lines + current Rust lines, by path
   and line number.
3. **Allowed ideas.** A bounded list of legal interventions.
4. **Forbidden.** What you must *not* do — almost always: no benchmark
   special-cases, no removed safety, no skipped semantic correctness,
   no new `unsafe`.
5. **Gate.** The 4-step verification chain above.
6. **Evidence after.** Link the artifact that shows the fix worked.

A commit that lands a hotpath fix should mention the before/after ratio
in its message and link the artifacts.

## Hard rules (binding non-goals)

These apply to *every* perf-shaped commit:

- **44/44 must stay green.** If the official suite regresses, the
  commit doesn't land.
- **No benchmark-only fast paths.** If the workload triggers a code
  path no real Lua program would, the optimization is fraudulent. The
  workload exists to measure that path; bypassing it is lying.
- **No skipped semantic correctness.** No skipped metamethod dispatch,
  no skipped GC barriers, no skipped error checks. The fix is always
  "do less work when the work is provably unnecessary," never "do the
  same work less correctly." Typed fast paths are fine when they preserve the
  same semantics; PR #120's table-barrier fast path kept the barrier and
  removed only `dyn Any` recovery. A primitive-barrier skip experiment failed a
  warning/finalizer path and was not kept.
- **No new `unsafe` in core runtime crates.** The current budgeted unsafe
  surface is `lua-gc`, the existing `lua-cli` FFI backend, and the dedicated
  WASM pointer ABI crates. `lua-coro` currently has a zero budget; a stackful
  backend must raise it in the same patch. A hotpath fix that needs raw pointer
  arithmetic is a different architectural conversation.
- **No `String` for Lua data.** Use `&[u8]` / `Vec<u8>` / `LuaString`.
  The string library is byte-oriented and our impl must be too.
- **GC canaries must stay green.** `./harness/canaries/gc/run_canaries.sh`
  before and after any change to GC-touching code.

## What good fixes look like

These are the shapes that have worked, in this port and in the redis
sibling:

### 1. Don't disable a feature; cache its precondition

When a feature has overhead even in the inactive case (slowlog, WATCH,
line hooks, GC barriers, metatable lookup), the fix is a cheap
precondition check, not a feature flag.

Example (redis): `slowlog-log-slower-than` and `slowlog-max-len` get
cached into a `LiveConfig` predicate. Dispatch still routes every
command through the normal handler but skips the duration timer when
the gate says nothing will be recorded.

Example (lua, applied 7682720): `#t` was calling `get_int(i)` in a
loop. The canonical `LuaTable::getn()` already implements C-Lua's
`luaH_getn` (alimit + binary search). The "feature" — supporting
arbitrary integer keys past `alimit` — wasn't disabled. It's still
correct. The hot path just stopped paying for it when alimit alone
sufficed.

**Sub-pattern: cache the invariant, not the mutable value.** Example
(lua, applied 658c7ec): `upvalue_get` was paying a `RefCell::borrow`
on the shared `GlobalState` to read `current_thread_id` per call. The
naïve cache shape is "snapshot the global on read, refresh on every
write site" — that requires hunting down every assignment to
`current_thread_id` (in lua-coro, ...) and inserting refresh calls.

The better shape: cache **each thread's own immutable id** on the
per-thread `LuaState`. The invariant `global.current_thread_id ==
self.cached_thread_id` is preserved structurally by the existing
resume protocol — there's no write path that breaks it. No refresh
logic, no cross-crate coordination, just a regular `u64` field set
once at thread construction. Reduced `upvalue_get`'s profile share
from 9.2% to 5.7% with zero invalidation logic.

The general lesson: when "cache the precondition" tempts you toward a
refresh-everywhere shape, look for an immutable invariant equivalent
to the mutable value. If it exists, cache that instead and the
invalidation problem evaporates.

### 2. Match upstream's structure, not just upstream's behavior

C-Lua's `lua_geti` doesn't go through the same code path as
`lua_gettable`. It has a direct integer-key fast path. If our
`table_get_i` always routes through the generic `table_get_with_tm`
that handles arbitrary keys + metamethods, we pay the generic-path
overhead on every integer lookup, even when the table has no
metatable. **Solution:** add an integer-key fast path that
short-circuits when the table is metamethod-free for `__index`.

A second flavor of this pattern: C does work IMPLICITLY that we do
explicitly. C-Lua's `lua_geti(L, 1, n)` resolves stack index 1 with a
single pointer arithmetic op (`ci->func.p + idx`) folded into the
load — never materializes a "resolution function call." Our
`index_to_value(state, idx)` is a real function with branches for
positive, negative, pseudo, and upvalue indices. **In a shift loop
over an N-element array we re-resolved the same stack slot N times.**
The fix is to resolve ONCE at the top of the calling function (e.g.
`table.remove`) and pass the resolved `&LuaValue` through a sibling
method that skips the resolution.

Example (lua, applied f179afb): `table.remove(arr, pos)` now calls
`state.value_at(1)` once, then uses `table_get_i_value` /
`table_set_i_value` in the shift loop body. The new methods take the
resolved table directly. `table_ops_long` ratio dropped from 4.76x
to 4.02x — a 15% workload-level speedup from skipping the per-iteration
resolution.

The general lesson: **if the C version's runtime cost is "essentially
free" relative to ours, the question to ask is "what does C do
implicitly that we do explicitly?"** The fix is to hoist or fold the
implicit work, not to remove it.

### 3. Avoid allocation on the hot path

Every `Vec::new()`, `to_vec()`, `clone()`, `Box::new()` in an inner
loop is a potential pole. Look for them in the profile. Common
offenders in interpreters:

- Building error message vectors that are never read on the happy path
- Boxing values that don't need indirection
- Cloning `Rc`/`Arc` when the borrow would suffice
- `Vec::push` into a growing collection — use `Vec::with_capacity` if
  the size is known
- Constructing `String` for type-name lookups (PERF(port) callout in
  `tagmethods.rs:328`)

**The hidden-allocation-in-helper pattern.** Watch out for innocuous-
looking helpers that secretly allocate. Example (lua, fixed 3190288):
`check_arg_string(idx)` returned `Vec<u8>` — calling code treated it
as "argument coercion + tiny validation," but every call did
`as_bytes().to_vec()` of the full source string. For `string.byte(s, i)`
in a tight loop (e.g. iterating every byte of a 14 KB string), that's
14000 calls × 14000-byte copies = ~200 MB of allocator churn per
iteration — and the workload runs the loop 50 times. The fix was to
hold a `GcRef<LuaString>` (one-pointer Copy) and borrow `.as_bytes()`,
so the bytes stay heap-resident on the GC and the helper just adds a
type check. **`string_ops_long` dropped from 2.25× to 1.58×** — first
workload to fall below the 1.5× parity threshold. Max RSS for the
workload also dropped from 116 MB to 7 MB.

The general lesson: when a function is called N times in an inner loop,
*every* allocation it does internally is multiplied by N. Cheap-looking
one-liners (`x.to_vec()`, `format!(...)`, `Vec::new()`) deserve as much
profile scrutiny as explicit `Box::new` constructions.

### 4. Inline-friendly fast paths

If the inner loop is a method call that ends in a `match` on a tag,
profile to see whether LLVM can inline through it. If the function is
too large or crosses crate boundaries, `#[inline]` may help; if the
match is too wide, splitting the cold cases into a separate function
(`#[cold]` + `#[inline(never)]`) often does.

The 2026-06-02 focused pass applied this conservatively:

- VM table/metamethod helpers (`fast_get*`, `fast_tm_table`) and string pattern
  predicates (`classend`, `handle_class_with_suffix`, `singlematch`,
  `match_class`) were promoted to inline-only hot helpers.
- Profiles confirmed the helper frames disappeared, and the focused matrix
  moved from 2.67× to 2.45× before deeper GC/string changes.

### 5. Replace compatibility state with upstream-shaped state

Some earlier port phases used "works for now" data structures that are
semantically valid but structurally unlike C-Lua. Those are fair perf targets
when a profile names them.

Example: `string.gmatch` previously stored iterator state in a four-slot Lua
table held as one closure upvalue. That kept the mutable state visible to GC,
but every iteration paid table reads/writes and barriers. C-Lua uses three
C-closure upvalues: source string, pattern string, and a userdata carrying
mutable byte positions. The Rust port now mirrors that shape safely:

- upvalues 1 and 2 are ordinary traced Lua strings;
- upvalue 3 is userdata;
- userdata `host_value` stores only `pos` and `last_match`, not GC references.

This reduced `gmatch_aux` in `string_ops_long` from ~6.9% to ~2.9% and moved
the workload from 2.51× to 2.42× before the GC bookkeeping pass.

### 6. Shrink collector bookkeeping; don't weaken collector semantics

GC profiles must be read with extra skepticism. A hot `HashMap` frame might be
allocation tokens, marker visited state, weak-table retention, interned-string
retention, or sweep bookkeeping. Use call stacks, not just leaf summaries.

Two safe-Rust collector fixes survived the 2026-06-02 pass:

- Young sweep no longer inserts every swept object into a `processed` set just
  because the grayagain old-revisit list is non-empty. It tracks only swept
  objects that are actually in the old-revisit list.
- `Heap` now maintains a GC-box count and reserves `Marker.visited` at cycle
  start. That removed `reserve_rehash` from the scaled `gc_pressure_x100`
  top-25 profile.

The boundary after those fixes is clearer: `gc_pressure` is still dominated by
marker visited insertion/lookup, table allocation/free, and table construction.
That is real collector/data-structure work, not a generic "safe Rust is
cooked" result.

C-Lua's `vmcase`/`vmbreak` macros plus its `OP_GETI` / `OP_SETI`
opcodes are the bytecode-level expression of this discipline.

**`#[inline]` vs `#[inline(always)]` — when LLVM bails.** Examples
(lua, applied 686a8bb + 4682b22): `LuaTable::get` and `do_::precall`
were both already marked `#[inline]` with LTO + codegen-units=1
enabled. Profile still showed them as their own frames at 8–20% of
wall. Upgrading to `#[inline(always)]` made the function bodies vanish
from the profile and the workloads moved 5–10% each.

The threshold: when a function body is large enough (say >50 lines
after monomorphization), LLVM's inline-cost heuristic blocks the
inline even with the `#[inline]` hint. `#[inline(always)]` overrides
the heuristic. **Rule of thumb: if profile shows a `#[inline]`'d
function as its own frame share >5%, escalate to
`#[inline(always)]`.** Doesn't hurt cold callers (each `[always]`-
inlined copy bloats the binary slightly but in our case the binary is
tiny relative to that).

The negative-result corollary: not every `#[inline(always)]` move
pays. Agent on commit `(opcode-dispatch)` upgraded
`Instruction::opcode` similarly and the bench didn't move — turned
out the 7.6% profile share was attribution noise on an
already-inlined function. **Profile share ≠ recoverable wall.**
The way to distinguish: re-sample after the change. If the frame
moves AND the workload wall drops, the inline did real work. If the
frame only moves, it was attribution cleanup.

**Macro-boundary rule.** When upstream C uses a macro, `l_sinline`, or
`static inline` helper in a hot path, treat that as a code-generation
requirement: the translated Rust equivalent should compile away too.
A behaviorally faithful Rust method is still too expensive if it shows
up as a leaf frame in the interpreter profile.

Concrete lua-rs example (applied `d8c1423`, evidenced by `a313817`):
after the table fast path, `fibonacci` still showed separate frames for
`precall`, `prep_call_info`, `set_ci_previous`, and `set_top`. Those map
to C-Lua macro/static-inline call-frame mechanics, not algorithmic work.
Inlining stack index arithmetic, stack accessors, CallInfo accessors, and
`precall`/`poscall` wrappers moved `fibonacci` from 3.04x to 2.41x and
overall from 1.71x to 1.45x, while keeping 44/44 official tests green.

The same rule applies to assertion macros with side effects. C-Lua spells
the VM stack-top invalidation as
`lua_assert(isIT(i) || (cast_void(L->top.p = base), 1))`; in normal builds
`lua_assert` is `((void)0)`, so the opcode-mode lookup and `top` write do
not exist. The Rust port initially ran that expression unconditionally on
every dispatch tick. Commit `7e32098` made it debug-only, keeping dev/release
official tests at 44/44 and moving the matrix overall from 1.39x to 1.33x
(`fibonacci` 2.26x -> 2.10x). Audit rule: when a C macro contains side
effects, check whether the macro is compiled out before preserving those side
effects in release Rust.

Also audit "hygiene" work added by the port. Rust ports often clear or nil-fill
reserved slots eagerly to make ownership and tracing feel tidy, but upstream C
may deliberately defer that cleanup. C-Lua's `prepCallInfo` links the next
`CallInfo` frame without clearing the reserved register tail; dead stack slots
are cleaned during thread traversal / stack shrinking. The Rust port initially
cleared that tail on every Lua call. Commit `97b3c4c` removed the per-call clear
after dev/release official suites stayed 44/44, moving the matrix overall from
1.31x to 1.25x and `fibonacci` from 2.07x to 1.93x. Rule: before adding
hot-path cleanup for GC neatness, find where upstream actually pays that cost.

**The negative-result variant: clones aren't always the cost.** Example
(lua, da9401e): we suspected `LuaValue::Clone` in the arith opcodes
was a real cost — every `OP_ADD` cloned two operands to satisfy the
borrow checker. We refactored to use primitive-tag accessors
(`get_int_at` returns `Option<i64>`, no enum clone). **Result:
mandelbrot improved (2.12× → 2.00×) but fibonacci was essentially
unchanged** — the LLVM-inlined value copy was not material.
The real fibonacci bottleneck is the dispatch machinery (precall,
upvalue_get, instruction decode). Profile evidence beats hypothesis;
sometimes the hypothesis was wrong about magnitude even when the
direction is right.

The fix landed anyway because (a) it's a structural improvement
matching C-Lua's `op_arith_aux` shape, (b) float-heavy workloads
benefit, (c) the new primitive accessors enable follow-up fixes on
the ORDER/BITWISE opcodes. But the bench-driven discipline matters:
we'd otherwise have claimed "fixed the fibonacci arith clones" when
the truth is "fixed mandelbrot incidentally, fibonacci was elsewhere."

### 5. Value copies are free; borrows are the tax

`LuaValue` is a 16-byte `Copy` enum and `GcRef<T>` is a `Copy` pointer
wrapper (`crates/lua-types/src/gc.rs`) — there is no reference count.
Copying values in tight loops is a plain memcpy and almost never the
bottleneck. (An earlier version of this section described a refcount bump
that does not exist in the current representation; corrected 2026-06-09.)
The real representation tax is interior mutability and helper boundaries:
`RefCell`/`Cell` borrow-flag traffic on shared GC structures (`LuaTable`'s
metatable and data cells), and `Result`-shaped fast-path helpers that move
keys/values back out by value on the miss path. Profile for borrow flags
and helper frames, not for `Clone`/`Drop`.

### 6. Avoid type erasure on hot paths

Generic helpers are useful at public or dynamic boundaries, but they are
usually the wrong shape inside a benchmark's inner loop. If the profile shows
`dyn Any`, `Any::type_id`, trait-object dispatch, or downcast helpers, first
ask whether the caller already knows the concrete type.

Example (lua, applied PR #120 / `2d5cffe`): table mutation already holds
`GcRef<LuaTable>`, but the GC write-barrier path erased that to `dyn Any` and
recovered it at runtime. `table_ops_long` sampled at 31.0% in `barrier_any`,
22.9% in `barrier_child_any`, plus visible `Any::type_id` frames. Adding typed
table-barrier helpers removed those frames, moved `table_ops` from 2.50× to
1.25× on the same Apple M3 Max runner, and left the barrier semantics intact.

Packet rule: add typed sibling helpers for known-type hot paths; keep the
generic helper for cold/dynamic sites. Do not turn this into skipped GC
barriers, skipped metamethod checks, or benchmark-only dispatch.

## Profile discipline

- **Wall-clock sampling ≠ CPU profiling.** `/usr/bin/sample` and the
  Activity Monitor's "Sample Process" both record stacks of *running*
  threads at intervals, including time spent in syscalls and I/O. For
  pure CPU attribution, use `xctrace` Time Profiler. We use `sample`
  by default because it's universally available and good enough for
  finding the first 80% of hotspots.

- **Build with frame pointers + debug symbols** before profiling:

  ```bash
  CARGO_PROFILE_RELEASE_DEBUG=true \
  RUSTFLAGS="-C force-frame-pointers=yes" \
    cargo build --release -p lua-cli
  ```

  Without these, symbols are missing or wrong and the sample output is
  useless.

- **Sample long-running workloads.** A 0.18-second workload (mandelbrot)
  doesn't sample well. Loop the workload N times or use a workload that
  runs at least 5 seconds. fibonacci@13s is ideal.

- **Probes vs ledgered profiles.** Quick exploration runs go to
  `harness/bench/profiles/<UTC>-<sha>-<workload>/` and are gitignored —
  they're telemetry, not evidence. Profile artifacts that motivate a
  commit get linked from the commit message.

- **`vm::execute` needs source-region attribution.** If
  `profile-hotspots.sh` sees `lua_vm::vm::execute` in the raw `sample.txt`, it
  writes `vm-execute.txt` beside the hotspot summary. Treat that as the first
  pass at per-region timing inside the interpreter loop. Read
  `opaque_self_samples` and the opaque-source table before treating
  `UNKNOWN_INLINED` as one bucket; it often distinguishes `vm.rs:0` from
  standard-library inlining such as `result.rs:0` or `value.rs:0`. It is
  sampled line/offset evidence, not exact opcode timing. When the report shows
  visible opaque offset neighbors, treat them as hints attached to an aggregate
  line-zero row, not per-offset timing. Pair the report with
  `opcode-profile.sh` when you need executed-op counts.

## What about JIT?

Out of scope. The lua-rs goal is *safe Rust*, no JIT, no tracing
compiler, no `unsafe` shortcuts. If/when we want LuaJIT-grade speed,
that's a separate project with a separate name. The dashboard's
purpose is to show how close we can get *without* a JIT, and to catch
regressions when normal porting work makes a hot path slower by accident.

## Workflow checklist

When starting a perf push:

1. Run `bash harness/bench/compare.sh` to capture the current state.
   This appends a ledger row at the current commit, which becomes the
   "before" datapoint on the dashboard.
2. Identify the workload to attack (typically the tallest pole in the
   current dashboard).
3. Build the profile-friendly binary (`force-frame-pointers=yes`).
4. Run `bash harness/bench/profile-hotspots.sh <workload>` to capture
   a wall-clock sample. Read the top frames and, when present, the adjacent
   `vm-execute.txt` source-region and opaque-source report.
5. Form a hypothesis. Write it down (commit message body is fine).
6. Apply *one* change. Smaller is better.
7. `cargo build --release -p lua-cli`. Run the 44/44 suite.
8. Run `bash harness/bench/compare.sh` again. Confirm the workload
   moved, others didn't regress.
9. Re-sample the workload. Confirm the previous top frame shrank.
10. Commit, referencing the before/after artifacts. Push.
11. Rebuild dashboard (`python3 harness/bench/history.py`). The new
    datapoint should appear.

If any step fails: that's the data. Move on or rethink. Don't paper over
a contradiction by tweaking the workload until it agrees with the fix.
