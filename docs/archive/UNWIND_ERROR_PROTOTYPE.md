# Unwind-error prototype (T5b) — sizing the `Result`-tax before a VM-wide arc

**Verdict: NO-GO on a full VM-wide `Result`→panic/`catch_unwind` conversion.**
The measured per-operation `Result`-tax on the hottest table get/set path —
the exact subsystem T2 gave per-write budgets for, and the one
`PERFORMANCE_MODEL.md §Safety-tax ablation` named as the residual idiom-tax — is
**~2–3 retired instructions per write and 0 conditional branches per write**.
That is **below the candidate's own code-layout floor**: the `fibonacci`
control, which shares no table code, swung **−2.60% Ir** on pure layout in the
same build — *larger* than the −1.65% attributable to the hottest converted row.
And the GET-side conversion returned **exactly 0** because the compiler already
elides an always-`Ok` `Result`. The upside is sub-floor; the blast radius is
enormous (≈735 signatures, ≈474 raise sites, ≈1821 `?` propagation sites, ≈246
of them public-API breaking) and forces a global `panic=unwind` requirement
that conflicts with the embeddable-everywhere thesis. The table path proves the
mechanism works and is correct, but the delta does not justify the arc.
Reasoning and receipts below.

This branch (`proto/unwind-errors-tableset`) is a **measurement instrument like
`ablation/unchecked-stack`** — it is pushed unmerged for reproducibility and
**never merges**. Only this memo reaches `main`.

## What was prototyped

The table set AND get hot paths were converted from `Result`-threading to
panic/`catch_unwind`, caught at the single protected-call boundary:

- **SET** — `OP_SETI` / `OP_SETFIELD` / `OP_SETTABLE` / `OP_SETTABUP`
  no-metatable (and metatable-hit) fast paths now call `()`-returning
  `raw_set_*_unwind` setters. On the rare real error (allocation-cap, nil/NaN
  key) the setter `panic_any(LuaSetError)` instead of returning `Err`. The
  success path constructs and checks no `Result`.
- **GET** — `OP_GETTABLE` / `OP_GETI` / `OP_GETFIELD` / `OP_GETTABUP` /
  `OP_SELF` fast paths call `fast_get*_unwind` returning `Option<LuaValue>`
  directly. The fast-get layer is **provably infallible** (every arm of the old
  `Result<Option<LuaValue>, LuaError>` returned `Ok`), so this drops a pure-tax
  wrapper with zero error-path risk; the only real GET error lives in the cold
  `finish_get` metamethod path, reached on a `None` and left untouched.
- **CATCH** — `lua_vm::do_::raw_run_protected` (the one protected frame every
  `pcall` / coroutine `resume` / `close` already routes through) wraps its body
  in `catch_unwind`, downcasts the `LuaSetError` marker, and reconstructs the
  identical `Err(LuaError)` the rest of the pipeline already expects. Nothing
  downstream changed.

### The first hard finding: the error value cannot ride the panic payload

`std::panic::panic_any` requires the payload to be `Any + Send`. A
`LuaError::Runtime` carries a `LuaValue` of **non-`Send`** GC pointers
(`GcRef`, `Rc<[u8]>`) — deliberately so, this is a single-threaded VM. So the
panic payload is a **zero-size `LuaSetError` marker** and the actual `LuaError`
rides a thread-local stash (`raise_set_error` / `take_pending_set_error`). This
is the exact Rust analog of how C's `longjmp` works: `longjmp` transfers only
control; the error object is left on `L`'s stack for the handler to read. The
existing unwind payloads in this codebase (`LuaThreadClose(LuaStatus)`,
`LuaExit(i32)`) are `Send`-trivial precisely because they carry no Lua value.
**A full conversion inherits this constraint everywhere**: every raise site
must stash-then-unwind, not unwind-with-value. It is not a blocker (the TLS
works), but it removes the naive "just `panic_any(err)`" simplicity the arc was
imagined to have.

## Correctness — NOT-WRONG-CODE (so the Ir numbers are valid)

The measurement is only meaningful if the prototype does the same work as
`main`; broken code that skips work would report fake Ir wins. Gates, all green
on the SET+GET prototype (`2ddaaec`):

- `cargo test -p omnilua --test multiversion_oracle` → **165/165**.
- Official suite: `errors.lua`, `nextvar.lua`, `events.lua`, `coroutine.lua`,
  `closure.lua`, `calls.lua` → **PASS**.
- `harness/canaries/gc/run_canaries.sh` → **36/36 PASS, 0 FAIL** (the set path
  touches GC write barriers).
- `cargo test -p omnilua --test panic_hook_chaining` → **PASS** — the new
  install-once `LuaSetError` suppression hook coexists correctly with
  `coro_lib`'s `LuaThreadClose` hook (each chains to the previously-installed
  hook; genuine Rust panics still print).
- Behavioral probes, byte-identical to `main`: type errors on indexing nil
  (with the `(local 'x')` variable hint and location prefix), `table index is
  nil` / `is NaN`, `__index` / `__newindex` user errors, all caught correctly
  by `pcall`; uncaught errors reach the top level with exit code 1 and a clean
  traceback; **an error raised inside a coroutine surfaces via `resume` as
  `(false, msg)`, not a process panic** (the coroutine's own inner
  `raw_run_protected` catches it); **zero stderr panic leak** on the caught
  path.
- **Zero new `unsafe`** — `catch_unwind` / `AssertUnwindSafe` / `panic_any` are
  all safe; the unsafe budget is untouched.

## Measurement

Bench host: this rig (macOS/arm64, M3 Max), deterministic cachegrind in the
Linux container, `--branch-sim`. Protocol: `docs/MEASUREMENT_PROTOCOL.md`
(Ir = instruction-removal arbiter; Bc = layout-immune branch cross-check;
Bcm ≈ 0 throughout, so this is wasted *work*, not stalls).

Baseline frozen at `origin/main` `6669499` (post-T5a; the `is_collectable`
reorder is already in, so this measures `Result`-tax on top of T5a's win).
Evidence TSVs in `harness/bench/results/`:
`20260613T051310Z-6669499-unwind-baseline-6669499.tsv` (baseline),
`20260613T052235Z-cbe7385-unwind-cand-cbe7385.tsv` (SET-only `cbe7385`),
`20260613T053127Z-2ddaaec-unwind-cand2-getset-2ddaaec.tsv` (SET+GET `2ddaaec`).

### Per-iteration budgets (rs, Ir minus startup / iterations)

| row | what/iter | base Ir | SET-only Ir | SET+GET Ir | base Bc | SET+GET Bc |
|---|---|---|---|---|---|---|
| table_setfield_same | 1 SETFIELD | 182 | 179 (−3) | **179 (−3, −1.65%)** | 30 | 30 (0) |
| table_seti_same | 1 SETI | 147 | 145 (−2) | **145 (−2, −1.36%)** | 22 | 22 (0) |
| table_field_index | 4 SET + ~10 GET | 1885 | 1841 (−44) | **1841 (−44, −2.33%)\*** | 270 | 264 (−6)\* |
| method_calls | 1 SELF (a GET) | 1358.83 | 1356.82 (−2) | **1354.82 (−4, −0.29%)** | 201.13 | 201.13 (0) |
| **fibonacci (CONTROL)** | no table code | — | −2.60% Ir | **−2.60% Ir** | — | −489 / 11.7e9 (0.00000%) |

\* The `table_field_index` −44 Ir / −2.33% is **smaller than** the fibonacci
control's **−2.60%** — and fibonacci shares no table code, so its −2.60% is
*entirely* code-layout shift (its Bc moved −489 on 11.7 *billion*, i.e.
0.00000% — layout-immune proof nothing real changed on the control). The
candidate's layout floor is therefore **larger than the entire attributable
table-row win.** `table_field_index` is also **byte-identical between SET-only
and SET+GET** — adding the GET conversion (≈10 GETs/iter) moved it by **0**.
Of the −44, ~12 is the 4 SETs (~−3 Ir each) and the rest is layout; the
layout-immune −6 Bc is the only honest residual signal there.

### Reading the numbers — two findings

1. **The attributable, layout-immune per-write `Result`-tax is ~2–3 Ir and 0
   conditional branches.** On a monomorphic, fully-inlined, always-`Ok` hot
   path, LLVM already elides nearly all of the `Result` machinery — the residue
   is a couple of `Ok`-wrap/discriminant `mov`s, and the `?` on an always-`Ok`
   value is a perfectly-predicted (Bcm ≈ 0) branch the optimizer folds. Bc is
   flat on every clean SET/GET row.
2. **The GET-path `Result` removal yielded essentially zero.** `fast_get*`
   always returned `Ok`, so the optimizer had *already* eliminated that wrapper
   before this prototype touched it: SET-only and SET+GET are byte-identical on
   `table_field_index` and the `method_calls` SELF-GET moved only −2 Ir.
   "Always-`Ok` `Result`" is free on this compiler. The tax only exists where a
   path *can* return `Err` and the optimizer must keep the discriminant live —
   and even there it is 2–3 Ir.

The ~100-Ir gap to C that T2 measured on these rows is therefore **not** `Result`
plumbing; it is representation/safety tax (T4's bounds checks + `RefCell`
guards, the 16-byte tagged enum) — exactly as `PERFORMANCE_MODEL.md`
conclusion 2 split it. This prototype quantifies the `Result`-plumbing slice of
that conclusion as the **smallest** of the three named idiom-taxes.

## Full-conversion blast radius (the go/no-go denominator)

Counted across `lua-vm` / `lua-types` / `lua-stdlib` / `lua-coro` /
`lua-rs-runtime` (excluding tests/examples), via ripgrep:

| dimension | count | note |
|---|---|---|
| fns returning `Result<_, LuaError>` (signature changes) | **≈735** | lua-stdlib 531, lua-vm 187, lua-types 17 |
| `?` propagation sites | **≈1821** | lua-stdlib 1133, lua-vm 441, lua-rs-runtime 238 |
| raise sites (`return Err` / `Err(LuaError…`) | **≈474** | lua-stdlib 266, lua-vm 134, lua-rs-runtime 57 |
| **public-API** `Result<_, LuaError>` fns (breaking) | **≈246** | incl. 54 in `lua-vm/src/api.rs` C-API shim + 36 in `lua-rs-runtime` embedding API |
| registered stdlib C-functions (every one a raise site) | **156** | + ~65 internal C-fn-shaped helpers |
| **catch boundaries that would need wiring** | **~6** | `raw_run_protected`, `protected_call*`, the embedding-callback `catch_unwind`s — the *cheap* side |

The asymmetry is the whole story: the **catch** side is trivial (~6 sites; this
prototype added exactly one and it covered every `pcall`/`resume`/`close`), but
the **raise + propagate + signature** side is ≈3000 sites, ≈246 of them
public-API breaking changes for embedders (the crates.io / C-API surface).

## Risks specific to the unwind approach (independent of the small delta)

- **`panic = "abort"` kills it.** Both profiles currently default to unwind
  (no `panic=` set), so `catch_unwind` works — but any consumer building with
  `panic=abort` (common for size-optimized / embedded / some wasm configs)
  would turn every caught Lua error into a process abort. A `Result` path has no
  such global-config dependency. This alone is close to disqualifying for an
  *embeddable* library.
- **FFI/`wasm` unwinding.** Unwinding across an FFI boundary (the C-API shim,
  `extern "C"` callbacks) is UB unless explicitly `extern "C-unwind"`; the C-API
  surface (54 pub fns) would need an audit. On `wasm32` exception-handling
  support varies by toolchain/runtime — the project explicitly ships
  `wasm32-unknown-unknown`, where panics historically lower to `abort`.
- **Coexistence is fine (verified).** The coroutine `LuaThreadClose` unwind and
  this `LuaSetError` unwind already share the process via chained, install-once
  panic hooks; `panic_hook_chaining` passes and errors inside coroutines stay
  inside the coroutine's protected frame. So adding a third unwind class is not
  the risk — the global-config and FFI exposure are.
- **The non-`Send` value constraint** (above) means a full conversion is
  stash-then-unwind everywhere, not the simpler `panic_any(err)` — more code,
  not less, at each of the ≈474 raise sites.

## Recommendation: NO-GO (with reasoning)

A full `Result`→unwind conversion is **not worth weeks of work**:

1. **The upside is below the noise floor — measured, not asserted.** The
   attributable per-write tax is ~2–3 Ir and 0 branches; the candidate's own
   `fibonacci` layout swing (−2.60% Ir) is *larger* than the best table-row win
   (−1.65%). The optimizer already removes most `Result` overhead on the hot
   monomorphic path, and removes an always-`Ok` `Result` (the GET path)
   entirely. The residual ratio-to-C on these rows is representation/safety tax,
   which this lever does not touch — consistent with the T4 ablation already
   showing ≥1.9× C *after* deleting all bounds checks and `RefCell` guards.
2. **The cost is ≈3000 edit sites and ≈246 public-API breaks**, plus a
   global `panic=unwind` requirement and an FFI/wasm unwinding audit that
   directly conflicts with the project's embeddable-everywhere thesis (LuaRocks
   client, Bevy, `wasm32-unknown-unknown`).
3. **The asymmetry favors leaving it alone.** The mechanism is real and correct
   (this prototype proves it), but you'd take on the entire raise/propagate
   surface to harvest a sub-layout-floor instruction win.

**Where the remaining tail-row instructions actually are** is representation
(NaN-boxing / a narrower value, a `RefCell`-free table fast path) — the T3/T5a
lever — not error-propagation idiom. That is the lever to keep sizing.

**NEEDS-MORE-DATA caveat:** if a future representation redesign (NaN-boxing)
collapses the safety/representation tax and `Result` plumbing becomes a
*relatively* larger share, re-run this instrument — the branch is preserved for
exactly that. But on today's representation, it is a NO-GO.

## Commits / repro

- Prototype code: `2ddaaec` (branch `proto/unwind-errors-tableset`).
- This memo: see the following commit (docs-only, cleanly separable for
  extraction onto a clean branch to `main`).
- Re-measure: `bash harness/bench/instr-count.sh --branch-sim --workloads
  table_setfield_same,table_seti_same,table_field_index,method_calls,fibonacci`
  at `6669499` (baseline) then at `2ddaaec` (candidate). Note: a fresh worktree
  has no `Cargo.lock` (gitignored); run `cargo generate-lockfile` once so the
  read-only container mount can build.
