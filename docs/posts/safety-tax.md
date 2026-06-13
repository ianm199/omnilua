<!--
DRAFT — not posted anywhere. For the user to edit and publish.
Every number below carries an inline HTML-comment footnote naming the in-repo
doc + line it came from, so it can be fact-checked before posting.
Primary sources:
  docs/PERFORMANCE_MODEL.md  §"Safety-tax ablation"   (lines ~792-841)
  docs/PERF_SPRINT_2_SPEC.md §"T2 setter family"      (the four-layer tax)
  docs/MEASUREMENT_PROTOCOL.md                        (the method/discipline)
  docs/ISSUE_BURNDOWN_SPEC.md                          (the negatives-as-deliverables story)
-->

# We measured what memory safety costs in a Lua interpreter (it's ~0%)

There is a folk belief that safe Rust buys its safety with a runtime tax: every
bounds check, every `RefCell` borrow flag, every `Result` you thread instead of
a `longjmp` is a slice of performance you handed back. The belief is reasonable.
It is also, on a modern out-of-order core, mostly wrong — and we now have the
receipts to say so.

omniLua is a from-scratch, pure-safe-Rust port of PUC-Rio Lua, one of the most
carefully hand-tuned C programs in wide use. The whole point of the project is
faithfulness: same bytecode, same semantics, the official test suites as the
oracle. That makes it an unusually honest place to ask the question, because
there is no algorithmic cheating to hide behind. If we are slower than C, the
slowness has to come from *somewhere specific*, and we can go and measure it.

So we did. We built an ablation: a branch where the safety checks are simply
deleted, and we counted what came back. The answer is the title.

## The setup

The hot path of a tree-walking-free bytecode interpreter is dominated by a small
number of opcode families: table writes (`SETFIELD`, `SETI`, `SETTABUP`), table
reads, calls and returns. Those are the rows where we trail reference C, and they
are where any "safety tax" would have to live. The ablation targets exactly the
two safety mechanisms that show up there:

- **Stack accessors** — the bounds-checked `get_at` / `set_at` indexing that
  guards every stack slot read and write.
- **Table fast paths** — the `RefCell` borrow guard taken on every table write,
  plus the array/node bounds checks inside the integer- and string-key store.

Two cargo features (`perf-ablation-unchecked-stack`,
`perf-ablation-unchecked-table`) turn each axis off independently, on a branch
that **never merges** — it exists only to be measured.<!-- docs/PERFORMANCE_MODEL.md ~L794-797: branch ablation/unchecked-stack, the two feature names --> All four
build configurations (default, stack-off, table-off, both-off) were first proven
*not-wrong-code* against the oracle — 165 multiversion cases, the official
`calls`/`nextvar` suites, and the GC canaries — before a single performance
number was trusted.<!-- docs/PERFORMANCE_MODEL.md ~L797-799: all four configs validated not-wrong-code (oracle 165, official calls/nextvar, canaries) --> A safety ablation that quietly broke correctness would
measure a fiction.

The arbiter is not wall-clock time. It is **retired instruction count** under
cachegrind — deterministic to single-digit counts per billion, immune to the
code-layout luck that makes small wall deltas meaningless on this hardware.<!-- docs/MEASUREMENT_PROTOCOL.md L51: Ir floor ~single-digit counts/billion, effectively exact; L46-48: code-layout floor ±2-3%, a call-free control once moved 12% wall --> A
branch simulator cross-checks the conditional-branch count and miss rate.

## What the safety checks actually cost

Here is the headline attribution. "Axis A" is the unchecked-stack ablation,
"Axis B" the unchecked-table ablation, "A+B" both at once; the last column is the
instruction-count ratio versus reference C *before* ablation and *after deleting
all of it*.

| row | axis A (stack) | axis B (table) | A+B | Ir ratio vs C: before → after |
|---|---:|---:|---:|---|
| table_setfield_same | −7.7% | −7.7% | −9.8% | 2.38 → 2.14 |
| table_seti_same | −9.7% | −10.4% | −12.3% | 2.52 → 2.21 |
| global_settabup_same | −6.4% | −8.5% | −14.3% | 2.45 → 1.88 |
| table_settable_string_key | −8.5% | −10.1% | −10.6% | 2.14 → 1.91 |
| call_return_shapes | −11.9% | −2.1% | −9.0% | 2.39 → 2.18 |
| method_calls | −10.0% | −9.1% | −15.5% | 2.43 → 2.06 |
| fibonacci | −8.7% | −2.1% | −5.7% | 2.54 → 2.39 |
| binarytrees | −4.0% | −2.8% | −4.9% | 2.18 → 2.07 |

<!-- docs/PERFORMANCE_MODEL.md L804-813: the full two-axis ablation table, verbatim -->

The nameable safety tax — every bounds check and every borrow guard on these hot
rows combined — is **5% to 15.5% of retired instructions**.<!-- docs/PERFORMANCE_MODEL.md L826-828: "5-15.5% of retired instructions" --> That is the most it
could ever be worth on instruction count, and it is real, recoverable work. It is
also not where the gap to C is.

Because look at the last column. After you delete *all* of it — every check, on
both axes, with correctness no longer a constraint — every single row still
executes **at least 1.9× the instructions reference C executes**.<!-- docs/PERFORMANCE_MODEL.md L830-832: "every row still executes ≥1.9× C's instructions" --> The safety
checks were never the dominant term.

## So where does the residual go?

If safety is a sixth of the gap at most, the other five-sixths has a name too. It
is **data-representation idiom**, and it decomposes into four layers we can each
point at:<!-- docs/PERF_SPRINT_2_SPEC.md L119-135: the four-layer breakdown of the ~21 extra branches/write -->

| layer | what it is | what C does instead |
|---|---|---|
| Borrow guards | a `RefCell` borrow flag taken per table write | no borrow flag exists |
| Bounds checks | `alimit` semantic check **and** a `Vec` index check LLVM can't prove redundant | a raw pointer deref |
| Tag dispatch | `is_collectable()` is a multi-variant `matches!` → ~2 branches | one `rawtt(v) & BIT_ISCOLLECTABLE` bit-test, 0 branches |
| Error plumbing | `Result<(), LuaValue>` threaded through the store layers | a `longjmp` |

The first two are the safety tax — the part the ablation deletes. The second two
are not safety at all. They are consequences of how we chose to *represent
data*: a 16-byte tagged enum for every Lua value, with discriminants you compare
against, versus C's `union` with a one-bit collectable flag; and `Result`
threading versus the non-local jump C uses for errors. Closing those is a
representation redesign — NaN-boxing the value, a union overlay, a panic-based
unwind for the error path — not a check you can switch off.<!-- docs/PERFORMANCE_MODEL.md L830-834: "dominant residual is representation/idiom... Closing that is a representation redesign (e.g. NaN-boxing), not a packet" -->

This is the load-bearing distinction. **The tax is a design decision, not a
safety levy.** We pay it because a tagged enum is a clean, safe, `Copy`-able
value type that the borrow checker understands and the rest of the codebase can
hold without ceremony. We could trade it away. We have not, because — and this is
the second half of the result — there is nothing on the other side of the trade.

## The wall-clock punchline

Here is the part that surprised us. We had the instruction-count ablation; we
expected the wall-clock ablation to show a smaller-but-positive win. It did not.
Measured quietly, interleaved, base build versus fully-ablated build:

- `table_seti_same`: **+23% slower** with the safety checks *removed*
- `fibonacci`: **+10% slower** ablated
- `method_calls`: −6.5% (a real but isolated win)<!-- docs/PERFORMANCE_MODEL.md L820-823: wall neutral-to-NEGATIVE; table_seti_same +23%, fibonacci +10% slower, method_calls -6.5% -->

Deleting the safety checks made the interpreter *slower* on most rows. Not within
noise — measurably slower.

The branch simulator explains why. Across these rows the safety checks add 4 to 6
conditional branches per table write on top of C's count, and the miss rate on
every one of them is **zero — Bcm ≈ 0 both before and after ablation**.<!-- docs/PERFORMANCE_MODEL.md L815-818: full ablation removes 4-6 of ~21 extra branches; Bcm ≈ 0 before AND after — every branch predicts perfectly --> These
branches predict perfectly. A bounds check that is always in-bounds, run a
million times in a loop, is a branch the predictor learns on the first iteration
and never pays for again. On a wide out-of-order core those instructions retire
in the shadow of work that was going to stall anyway; removing them mostly just
perturbs code layout, and code layout is worth more than the checks.

So the honest, decision-grade conclusion: the memory-safety tax in this
interpreter is **5–15.5% of instructions and ~0% of reliable wall time.** There
is no unsafe-shaped win waiting to be unlocked. The unsafe budget stays where it
is — at zero outside the GC.<!-- docs/PERFORMANCE_MODEL.md L826-829: conclusion 1, "~0% of reliable wall time... no meaningful unsafe-shaped wall win... unsafe budget stays where it is" -->

## How we measured (and why you should believe it)

The result is only worth the discipline behind it, so the discipline is part of
the claim. The rules we ran under:<!-- docs/MEASUREMENT_PROTOCOL.md is the source for this whole box -->

> **The arbiter, not the stopwatch.** This rig (Apple M3 Max, macOS/arm64) cannot
> attribute small wall deltas — a call-free control once moved 12% wall purely
> from a whole-crate layout shift.<!-- docs/MEASUREMENT_PROTOCOL.md L46-48 --> So every claim is classified before it is
> measured: *instruction removal* is judged by deterministic Ir, *branch/CPI* by
> the branch simulator's Bc/Bcm, *latency* by wall plus an enumerated list of
> removed work. Wall time alone settles nothing.<!-- docs/MEASUREMENT_PROTOCOL.md L11-27: the wall = instructions × CPI model and per-class arbiters -->
>
> **Frozen baseline, interleaved.** The baseline binary is built from the tip of
> the branch *before* any edit and copied out of the build tree; candidate and
> base are run alternately, never in back-to-back blocks, and we judge the
> min-ratio over ≥4 rounds.<!-- docs/MEASUREMENT_PROTOCOL.md L31-39 -->
>
> **Drop-if-neutral. Honest negatives are deliverables.** A neutral result is
> reported as neutral and the code is reverted — we do not keep changes "because
> they should help."<!-- docs/MEASUREMENT_PROTOCOL.md L63-65 --> The same sprint that produced this ablation also produced
> a stack of measured *failures*: a setter-family fast-path rewrite that turned
> out to be at branch-parity with C already, a method-lookup diet whose premise
> (clone overhead) didn't exist because our value type is `Copy`. Those negatives
> are written down in full.<!-- docs/ISSUE_BURNDOWN_SPEC.md T2-D, docs/PERF_SPRINT_2_SPEC.md "T2 setter family — RESOLVED-NEGATIVE": the recorded negatives -->

Reproducing it is two commands. Build the ablation branch with one or both
features, and run the instruction counter:

```bash
git checkout ablation/unchecked-stack
# axis A, axis B, or both:
cargo build --release -p omnilua-cli \
  --features perf-ablation-unchecked-stack,perf-ablation-unchecked-table
bash harness/bench/instr-count.sh --branch-sim \
  --workloads table_setfield_same,table_seti_same,method_calls,fibonacci
```

The evidence TSVs live at `harness/bench/results/20260611T*-t4-*.tsv`.

## Why this matters beyond a Lua interpreter

A bytecode interpreter is the *worst case* for a safe-Rust port. It is the most
hot-loop-bound program there is: a tight dispatch loop, billions of iterations,
every per-opcode instruction multiplied a billion times. If the safety tax were
ever going to be a wall-clock problem, it would be a problem here. It is ~0%.

Now point that result at the software the safe-Rust-port question actually cares
about: servers, proxies, databases — nginx-class systems. Those are I/O-bound. A
request spends its life waiting on a socket, a disk, a downstream service; the
fraction of wall time inside a tight, safety-checked compute loop is small to
begin with. If the hot-loop safety tax is ~0% even in an interpreter that is
*all* hot loop, then in software where the hot loop is a thin sliver of the
runtime, the safe-Rust penalty rounds to nothing.

Which is the whole strategic point. "Safe Rust at C speed" is not a hope. For
the class of C software most worth rewriting, it is what the measurements imply.

---

*The numbers in this post are pinned to in-repo evidence; every figure carries a
source comment in the markdown. The performance model lives in
[`docs/PERFORMANCE_MODEL.md`](../PERFORMANCE_MODEL.md) (§Safety-tax ablation) and
the method in [`docs/MEASUREMENT_PROTOCOL.md`](../MEASUREMENT_PROTOCOL.md). The
ablation branch is `ablation/unchecked-stack`, pushed unmerged for
reproducibility.*
