# Changelog

All notable changes to `lua-rs` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
