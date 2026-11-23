# EVIDENCE — `GcRef::new_in` fast-path packet (issue #252 / PR #258 Rc-heap regression audit)

Measurement-only packet. Per `docs/MEASUREMENT_PROTOCOL.md`, the arbiter is
deterministic instruction count (Ir, cachegrind in the Linux container); wall
time is a secondary signal only on this rig (macOS/arm64, Apple M3 Max). The
rule is **drop-if-neutral**.

## The question

PR #258 moved the GC `Heap` behind `Rc` and made `with_current_heap` clone the
top-of-stack `Rc<Heap>` out of a TLS `RefCell` on **every** call. That sits on
the allocation path: every `GcRef::new` (gc.rs:37), `GcRef::downgrade`
(gc.rs:95), and `Gc::account_buffer` (gc.rs:128) now pays a `RefCell` borrow +
`Rc` strong-count inc/dec pair it did not pay before. A
`GcRef::new_in(&Heap, value)` fast path threading the heap explicitly through
hot VM allocation sites would skip it. This packet measures whether that Rc
overhead is real and attributable, to decide IMPLEMENT vs DROP.

## A/B pair (isolates exactly the Rc conversion)

- **A** = `56cfb01715dbe3c332b8c4712fd40789726ba902` (first parent of the #258 merge; pre-Rc, `NonNull<Heap>` on the TLS stack, no refcount)
- **B** = `45d6c3420cabb963079702fdab649d5c0eaf462b` (PR #258 merge, "Heap behind Rc")

`git diff A B` touches exactly three files — the Rc conversion, nothing else:
`crates/lua-gc/src/heap.rs` (+58/-55), `crates/lua-types/src/gc.rs`,
`crates/lua-vm/src/state.rs`. Confirmed the mechanism at B:

```
pub fn with_current_heap<R>(f: impl for<'a> FnOnce(Option<&'a std::rc::Rc<Heap>>) -> R) -> R {
    let top = CURRENT_HEAP_STACK.with(|stack| stack.borrow().last().cloned());  // Rc inc
    f(top.as_ref())
}   // top drops here -> Rc dec
```

At A the same TLS held `Vec<NonNull<Heap>>` and handed out a bare pointer — no
refcount traffic.

## Build

Both worktrees built `cargo build --release -p omnilua-cli` (binary
`target/release/omnilua`), exit 0. Cargo.lock present in both (required by the
read-only `/src` container mount).

## Commands

```
# Ir (arbiter) — run once per side; cachegrind is deterministic (~0.1%), no interleaving needed
cd <worktree-A>; bash harness/bench/instr-count.sh --workloads binarytrees,closure_ops,table_ops,table_ops_long,fibonacci,mandelbrot --label newin_A
cd <worktree-B>; find crates -name '*.rs' -exec touch {} +   # REQUIRED — see mtime trap below
cd <worktree-B>; bash harness/bench/instr-count.sh --workloads binarytrees,closure_ops,table_ops,table_ops_long,fibonacci,mandelbrot --label newin_B2

# Wall (secondary) — interleaved A,B best-of-5 via /usr/bin/time -p (scratchpad/wall.sh)
```

## Tooling finding: instr-count.sh mtime trap (silent stale-binary reuse)

The first B container run (label `newin_B`, TSV
`20260713T134219Z-45d6c342-newin_B.tsv`) was **invalid**: `instr-count.sh`
shares one cargo cache volume (`lua-rs-instr-cache`) across invocations, and
cargo's mtime-based fingerprinting saw B's source files (worktree checkout
~13:29Z) as *older* than A's cached build (binary linked 13:32:41Z), so it
silently skipped the rebuild and measured **A's binary again**. Proof: after
the B run, the cache binary's mtime was still 13:32:41Z (inside A's run
window; B started 13:42:19Z) and its sha256 (`596e5102...`) was unchanged.
Rule for any cross-commit A/B with instr-count.sh: `touch` the second tree's
sources (or clear the cache volume) before the second run, and verify the
cache binary's mtime/hash advanced afterward. B was re-measured after
`find crates -name '*.rs' -exec touch {} +` at 13:56:52Z (label `newin_B2`).

## Results

### Wall (secondary signal) — interleaved A,B per round, best-of-5 min, quiet host

Host binaries verified distinct (sha256 A `e00b1a23...`, B `a2041706...`).

| workload | min wall A (s) | min wall B (s) | wall B/A |
|---|---|---|---|
| binarytrees | 0.68 | 0.71 | 1.044 |
| closure_ops | 0.30 | 0.30 | 1.000 |
| table_ops_long | 1.11 | 1.10 | 0.991 |
| fibonacci (control) | 4.09 | 4.08 | 0.998 |
| mandelbrot (control) | 0.13 | 0.13 | 1.000 |

Controls flat. binarytrees +4.4% is inside this rig's code-layout noise band
(±2-3%, occasionally 12%) — attribution requires the Ir axis per protocol.

### Ir (arbiter) — deterministic cachegrind, Linux container, rs binary only

A = TSV `20260713T133201Z-56cfb017-newin_A.tsv` (worktree `lua-rs-port-newin-A`).
B = TSV `20260713T135708Z-45d6c342-newin_B2.tsv` (worktree `lua-rs-port-newin-B`,
sources touched, rebuild verified by cache-binary mtime 13:57:23Z + sha change).

The invalidated first B run (same binary as A, separate container run) doubles
as an exact **A-vs-A noise floor**: max |delta| 0.00025% (binarytrees). Every
delta below is therefore real signal, not measurement noise.

| workload | Ir_A | Ir_B | ΔIr | ΔIr% | wall B/A (best-of-5) | noise floor % |
|---|---|---|---|---|---|---|
| binarytrees (alloc) | 11,474,157,388 | 11,540,136,076 | +65,978,688 | **+0.575%** | 1.044 | 0.00025 |
| closure_ops (alloc) | 6,339,891,511 | 6,343,225,158 | +3,333,647 | +0.053% | 1.000 | 0.00000 |
| table_ops (alloc) | 701,184,351 | 701,216,423 | +32,072 | +0.005% | n/a (0.02s, startup-dominated) | 0.00002 |
| table_ops_long (alloc) | 35,214,297,711 | 35,214,537,298 | +239,587 | +0.001% | 0.991 | 0.00000 |
| fibonacci (control) | 83,417,548,290 | 83,417,557,487 | +9,197 | +0.000% | 0.998 | 0.00000 |
| mandelbrot (control) | 2,877,182,848 | 2,874,262,312 | −2,920,536 | −0.102% | 1.000 | 0.00001 |
| startup_empty (constant) | 1,148,402 | 1,157,029 | +8,627 | +0.751% | — | 0.00853 |

Reference-binary rows are omitted: the identical cached C binary jitters up to
1.67% Ir across runs (binarytrees ref 5.809B vs 5.906B vs 6.044B — C Lua's
time-seeded hash randomization), which is why only rs-vs-rs is compared and why
rs determinism was established separately via the A-vs-A pair.

## Verdict

**DROP** (measured sub-threshold; protocol rule is drop-if-neutral).

- Controls are flat: fibonacci +0.000%, mandelbrot −0.102% — both well under
  the 0.5% control bar. The rig is sound.
- The Rc tax is *real but small*: the worst allocation-heavy row, binarytrees,
  shows ΔIr **+0.575%** (+66.0M instructions, ~2,300× the noise floor, and in
  the right direction on every alloc row). At ~8 instructions per
  `with_current_heap` Rc clone+drop that is ~8M hot calls — the mechanism is
  confirmed, the magnitude is not actionable.
- The packet's bar was ΔIr ≥ ~1% on allocation-heavy rows. Max observed is
  0.575%, and that total includes *everything* in the #258 diff; the portion a
  `GcRef::new_in` fast path could recover is bounded above by it. closure_ops
  (+0.05%) and the table rows (≤+0.005%) show the tax is negligible outside
  binarytrees' extreme allocation profile.
- binarytrees' wall +4.4% with Ir only +0.575% is CPI/code-layout dominated
  (the rig's documented ±2-3%-and-worse layout band); per protocol a sub-5%
  wall delta cannot be attributed without Ir support, and Ir says 0.575%.

`GcRef::new_in` is not worth its churn on this evidence. If binarytrees-class
allocation wall time ever becomes the frontier again, re-measure with
`--branch-sim` (Bcm) before reopening — the wall gap there, if real, lives in
CPI, not instruction count.
