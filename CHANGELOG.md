# Changelog

All notable changes to `lua-rs` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.32] - 2026-06-09

### Added

- **benchmarks/perf** (#134): added focused coverage for isolated table and
  global setter hot paths: `table_setfield_same`,
  `table_settable_string_key`, `table_seti_same`, and
  `global_settabup_same`.

### Changed

- **vm/table** (#134): accelerated stable table/global setter paths by adding
  existing-slot updates for short-string and integer keys, routing `SETFIELD`,
  string-key `SETTABLE`, `SETI`, and no-metatable `SETTABUP` through those
  paths, caching table metatable presence, and avoiding write-barrier work for
  non-collectable values.

## [0.0.31] - 2026-06-04

### Added

- **benchmarks/perf** (#134): expanded the benchmark matrix with bytecode and
  dispatch telemetry workloads: `numeric_mixed`, `bitwise_mixed`,
  `compare_immediates`, `loop_variants`, `call_return_shapes`, and
  `table_field_index`.

### Changed

- **parser/vm**: emit Lua 5.4 immediate and constant-pool opcode shapes for
  arithmetic, bitwise, shift, and equality expressions where the reference
  compiler uses them, while preserving the correct metamethod fallback opcodes.
- **vm/stdlib**: tighten the generic-for C-iterator path used by `ipairs`,
  avoiding the full generic call slow path when the iterator is already a C
  function and removing repeated positive stack-index resolution in
  `ipairs_aux`.

### Fixed

- **conformance**: the expanded perf packet exposed three version-sensitive
  official-suite edges, now fixed for Lua 5.4 while preserving Lua 5.5 oracle
  behavior: long `__call` chains no longer hit the 5.5-only cap on 5.4,
  stripped bytecode errors report `?:-1:` on 5.4, and high-index method calls
  keep their `method 'name'` attribution on 5.4.

### Docs

- Documented the final performance matrix, profile artifacts, and next packets
  for call frames, iterator dispatch, allocation/GC cadence, and upvalue-heavy
  closure workloads.

## [0.0.25] - 2026-06-01

### Fixed

- **debug** (#92): version-gated line-hook (`debug.sethook(f,"l")`) fidelity.
  Lua 5.5 folds the conditional `TEST`/`JMP` of an `if`/`elseif` onto the
  condition-expression line, so a multi-line `if/<cond>/then` no longer fires a
  separate `then`-line event (5.1–5.4 keep it). On 5.1/5.2/5.3 a numeric `for`
  now fires a line event on every iteration's back-edge — the legacy
  FORPREP-jumps-to-the-bottom-test loop shape — where 5.4/5.5 fire once per
  iteration. Verified byte-for-byte against the Lua 5.3.6 and 5.4.7 references.
- **lexer** (#105): Lua 5.1 quotes the special multi-char tokens (`<eof>`,
  `<name>`, `<number>`, `<string>`) in syntax-error messages
  (`'<name>' expected near '<eof>'`), matching 5.1's unconditional `LUA_QS`
  wrapping; 5.2+ leave them bare.

### Added

- **reference**: pinned upstream Lua 5.3.6 (with the 5.3.4 test bundle) as a
  secondary behavioral oracle for version-gated 5.1/5.2/5.3 work
  (`reference/lua-5.3.6/`, source committed, binaries built locally).

## [0.0.24] - 2026-06-01

### Fixed

- **vm/coroutine** (#97): a `__le` derived from `__lt` (the `LUA_COMPAT_LT_LE`
  fallback on 5.1–5.4) now negates correctly when the `__lt` metamethod yields,
  via the `CIST_LEQ` mark. Previously the comparison returned the inverted
  result across a yield.
- **vm** (#96): closures built in a loop over identical upvalues compare equal
  (`==`) on 5.2/5.3, matching reference closure caching; distinct on 5.1/5.4/5.5.
- **vm** (#94): Lua 5.5 named varargs `function f(...t)` share one storage object
  between `t` and `...`, so mutating `t` is observable through a later `...`
  (count follows `t.n`); preserved across `string.dump`/`load`.
- **parser** (#95): the `break`-outside-loop error message is now version-correct
  (5.1 `no loop to break`, 5.2/5.3 `<break> ... not inside a loop`, 5.4
  `break outside loop at line N`, 5.5 `break outside loop near 'break'`).

## [0.0.23] - 2026-06-01

### Fixed

- **Windows build** (#90): `lua-cli`'s `os.date`/`os.time` local-timezone hook
  used `libc::localtime_r` and read `tm_gmtoff`, neither of which exists in
  Windows' MSVCRT, so `lua-cli` failed to compile on Windows. The
  `local_offset_hook` is now `cfg`-split — the Unix path is unchanged; the
  Windows path derives the same offset (DST included) by decomposing the instant
  with `localtime_s` and `gmtime_s` and differencing the two wall clocks.

### CI

- Releases now gate on a `windows-build` job (`windows-latest`, MSVC) that
  `publish-crates` depends on, so a Windows compile break can no longer ship.

## [0.0.22] - 2026-06-01

### Added — Lua 5.1 and 5.2; one API now spans Lua 5.1–5.5

`lua-rs` now runs **Lua 5.1, 5.2, 5.3, 5.4, and 5.5** from a single embedding
API, selected per instance (`Lua::new_versioned(LuaVersion::V51 …)` /
`LUA_RS_VERSION=5.1…5.5`). Every version is verified against its unmodified
upstream reference binary; all five share one core, and the bytecode dispatch
loop carries no per-version cost (3/4/5 are byte-identical in benchmarks; the
only measurable delta is 5.1/5.2 integer-arithmetic, an inherent consequence of
their float-only number model).

- **5.2 (the bridge)**: float-only numbers on the modern `_ENV` globals model;
  `bit32`, compat-math, `module`; `//`/bitwise/`<const>` rejected; `math.type`/
  `utf8`/`string.pack` absent.
- **5.1 (the legacy family)**: float-only **plus fenv globals** —
  `getfenv`/`setfenv` over a per-closure environment; `__len`-on-tables inert;
  `loadstring`, global `unpack`, `table.getn`/`foreach`, `newproxy`, `gcinfo`,
  1-arg `math.log`; `goto`/`bit32`/`string.pack`/`utf8` absent. C-`rand()` PRNG
  sequence is a documented divergence (the contract — ranges/arg-errors — matches).

5.3 and 5.5 graduate from **alpha to beta**: their long tails are closed
(compat-math, bitwise string coercion, error wording, `global` declarations,
named varargs, `utf8.offset`, `collectgarbage` params, traceback fidelity).

### Fixed

- **Cross-version fidelity** (improves 2–3 versions at once): `_ENV[<relational>]`
  index codegen, arg-error `to '<fn>'` qualifier + offending value + location
  prefix, `print`→global `tostring` (5.1–5.3), `\u{}`/`utf8.char` ceilings,
  `string.unpack`/`format`/`pack` boundaries, `math.random` interval guards.
- **The trailing `[C]: in ?` traceback frame** on uncaught errors, cross-version
  (#79) — official `math.lua` now runs clean.
- **goto label scoping** (block-scoped on 5.2/5.3 vs function-wide on 5.4/5.5).
- **`__gc` finalizer error disposition** (propagate on 5.2/5.3, warn on 5.4/5.5)
  — and the previously-unwired `warn`/`@on`/`@off` subsystem now emits.
- A **panic** in the table downward-resize path (`index out of bounds`) — now
  clamps to the physical array length per upstream `luaH_resize`.

### Changed

- CI: the release perf-dashboard step's `/usr/bin/time` parsing is now portable
  to the Linux runner (#82).

### Docs

- `specs/MULTIVERSION_PLAYBOOK.md` — the reusable "how to add a Lua version"
  methodology (oracle/contract, adversarial-first, the iteration ladder, the
  version seam, the per-phase workflow).

## [0.0.21] - 2026-05-31

### Added — Lua 5.3 fidelity (toward #19)

The clear-cut Lua 5.3 long tail surfaced by the multi-version oracle sweep,
each fix verified against the upstream `lua5.3.6` reference binary and guarded
in `tests/multiversion_oracle.rs` (now 29 cases):

- **`LUA_COMPAT_MATHLIB` roster.** `math.atan2/cosh/sinh/tanh/pow/log10` are now
  present on 5.3 and 5.4, and `math.frexp/ldexp` on 5.3/5.4/5.5 — matching the
  default reference builds. (This also closed a latent 5.4 gap: those six
  functions were previously absent on 5.4, where the reference exposes them.)
- **String→integer coercion in core bitwise ops.** On 5.3, numeric strings
  coerce in `& | ~ << >>` (e.g. `"0xff" & 0xf0` → `240`); 5.4/5.5 keep raising.
  Non-integral numeric strings still report "number has no integer
  representation". This made the official `bitwise.lua` and `constructs.lua`
  byte-identical to the reference.
- **5.3-specific error wording.** Arithmetic on a non-coercible string now
  reports `attempt to perform arithmetic on a <type> value (<varinfo>)` with the
  correct `local`/`global`/`constant` qualifier; a non-number `for` bound reports
  `'for' <what> must be a number`. 5.4/5.5 wording is unchanged.

5.4 and 5.5 behavior is unaffected (`check.sh` 5.4=7/0, 5.5=10/0); the
compat-math roster, bitwise coercion, and error wording are all version-gated.

## [0.0.20] - 2026-05-31

### Fixed — reference-fidelity bugs surfaced by the multi-version oracle

Cross-version bugs found by diffing against the upstream reference binaries
(present before the multi-version work; they affected 5.3/5.4/5.5):

- `math.type` / `math.tointeger` now return `nil` (a `fail`), not `false`, so
  `== nil` guards and truthiness behave as the manual specifies (#76).
- `string.find` with a magic-character pattern and no explicit captures no
  longer returns a spurious trailing empty value — arity matches the reference
  (#77).
- `__le` is derived from `__lt` (`a <= b` ⇒ `not (b < a)`) on 5.1–5.4 (matching
  the default `LUA_COMPAT_LT_LE` reference builds) and raises on 5.5 (#78).
- Error-message fidelity: `bad argument` errors carry the `to '<fn>'` qualifier
  and `got no value` for absent args; length/concat/string-arithmetic errors
  carry the `(command line):N:` location prefix; arithmetic metamethod-failure
  messages report the correct operand types; `table.concat` reports `(table)`
  instead of leaking an internal byte-array (#79).

### Changed

- CI: the release workflow's npm `verify registry install` step now retries with
  backoff so registry propagation lag no longer fails an otherwise-successful
  publish (#80).

## [0.0.19] - 2026-05-31

### Added — multi-version support: Lua 5.3 and 5.5 (alpha)

A single embedding API can now run Lua **5.3** and **5.5** alongside the stable
**5.4**, selected per instance:

```rust
let lua = Lua::new_versioned(LuaVersion::V53); // or V55, or V54 (default)
```

Both share 5.4's mature core (VM, GC, number model, metatables, most of stdlib)
with version-specific behavior confined to a few cold-path seams; the bytecode
dispatch loop carries no per-version cost, so compute-bound code runs
identically across versions. The CLI selects a version with
`LUA_RS_VERSION=5.3|5.4|5.5`.

- **5.5**: contextual `global` (`LUA_COMPAT_GLOBAL`), block-scoped `global`
  declarations with strict undeclared-name checking, `<const>` globals,
  read-only numeric/generic for control variables, stored `global` initializers,
  round-trip float `tostring`, `table.create`.
- **5.3**: `bit32` library, string-in-arithmetic coercion to float, `warn` and
  `coroutine.close` absent, `<const>`/`<close>` attribute syntax rejected.

**Alpha caveat.** 5.3 and 5.5 are preliminary. Their headline features are
verified against the upstream reference binaries (`tests/multiversion_oracle.rs`),
but each has a documented long tail (e.g. 5.3 compat-math and error wording; 5.5
named varargs and the "global already defined" guard) — see `specs/`. Use 5.4
for production and treat 5.3/5.5 as experimental. Lua 5.1/5.2 are not yet
supported and refuse construction rather than masquerade as 5.4.

## [0.0.18] - 2026-05-30

### Added — sandboxing for untrusted Lua

Run untrusted scripts with bounded CPU and memory and no host access. Limits are
enforced on every thread (coroutines included) and are **uncatchable** — a script
cannot escape them with `pcall`/`xpcall`/`coroutine.resume`. A non-sandboxed
runtime pays zero overhead.

- **Rust:** `Lua::sandboxed(SandboxConfig)` returns the runtime plus a `Sandbox`
  handle (`tripped()` / `reset()`). `Lua::install_sandbox` and
  `LuaRuntime::install_sandbox` apply limits to an existing runtime.
- **CLI:** `--sandbox`, `--max-instructions=N`, `--max-memory=N[K|M|G]`.
- **WASM / JS:** `lua_rs_wasm_set_limits` / `lua_rs_wasm_last_trip` /
  `lua_rs_wasm_sandbox_reset`; the `lua-rs-wasm` JS wrapper adds `setLimits`,
  `lastTrip`, and `sandboxReset`.

Three controls: an instruction budget (aborts infinite loops and runaway
recursion), a memory ceiling (refuses oversize allocations before they happen,
plus per-interval sampling), and capability stripping (removes `os.execute`,
`io`, `load`, `require`, `debug`, … from `_G`). Design and threat model:
[docs/SANDBOXING_EXPLORATION.md](docs/SANDBOXING_EXPLORATION.md).

## [0.0.17] - 2026-05-30

### Changed — `#[derive(LuaUserData)]` field exposure (BREAKING)

**Private fields are no longer auto-exposed to Lua. Mark fields `pub` or use
`#[lua(field)]`.**

Rust visibility is now the scriptability boundary for the derive:

- **Public named fields** are auto-exposed to Lua, exactly as before
  (`obj.field` read/write, requiring `Clone + IntoLua + FromLua` on the field
  type).
- **Private named fields** are now opaque — invisible to Lua. Previously
  *every* named field was exposed regardless of visibility, which forced
  `Clone` (and Lua-marshaling) onto fields that were only ever meant to be
  internal. To keep a private field scriptable, either make it `pub` or
  annotate it `#[lua(field)]`.
- **Tuple/newtype and unit structs** (e.g. `struct Handle(App);`) now derive
  successfully and become **opaque userdata handles** — no field access, but
  full support for `#[lua(methods)]` and metamethods. Previously the derive
  rejected them with a compile error.

This makes both data-record structs and opaque engine/resource-handle structs
work without extra boilerplate, and lets a struct hold a non-`Clone` private
field (e.g. `app: bevy::App`) and still derive cleanly.

Closes [#56](https://github.com/ianm199/lua-rs/issues/56) and
[#57](https://github.com/ianm199/lua-rs/issues/57).

#### Migration

```rust
// Before: `x` and `y` were exposed to Lua because every field was.
#[derive(LuaUserData)]
struct Point { x: f64, y: f64 }

// After: mark the fields you want scriptable `pub`.
#[derive(LuaUserData)]
struct Point { pub x: f64, pub y: f64 }

// ...or force-expose a specific private field with `#[lua(field)]`:
#[derive(LuaUserData)]
struct Point {
    #[lua(field)]
    x: f64,
    pub y: f64,
}
```

If a previously-exposed field silently becomes `nil` in your Lua code after
upgrading, this is the cause: add `pub` or `#[lua(field)]` to that field.

### Added

- `#[lua(field)]` field attribute on `#[derive(LuaUserData)]` to force-expose a
  private field (escape hatch for the visibility change above).
- `#[derive(LuaUserData)]` support for tuple/newtype and unit structs as opaque
  userdata handles.
- Behavioral-parity oracle (`make parity` / `harness/parity_check.sh`): a golden
  diff of normalized stdout + exit code against reference C Lua 5.4.7, distinct
  from the existing no-crash gate. ([#60](https://github.com/ianm199/lua-rs/pull/60))

### Fixed

- `os.date` / `os.time` local-time handling and close-time (`<close>`
  to-be-closed variable) finalizers — two behavioral divergences from C Lua 5.4
  surfaced by the new parity oracle. Official-test conformance 24 → 27/33.
  ([#60](https://github.com/ianm199/lua-rs/pull/60))

### Performance

- GC pacer now charges table array/hash backing buffers, not just the `GcBox`
  header, so the collector's byte budget reflects real allocation.
  ([#58](https://github.com/ianm199/lua-rs/pull/58))
- Removed a redundant duplicate short-string intern table; short strings were
  interned twice and one copy was never read (−56% RSS, −54% wall on the
  `table_hash_pressure` benchmark). ([#62](https://github.com/ianm199/lua-rs/pull/62))
