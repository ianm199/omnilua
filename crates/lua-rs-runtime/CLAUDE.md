# lua-rs-runtime — the embedding API

The public, `mlua`-shaped embedding crate. This is the **stable surface** users
depend on (published to crates.io, docs on docs.rs) and the `wasm32` target.
Changes here are semver-visible — be conservative. Read the root `../../CLAUDE.md`
first.

## Surface

- `Lua::new()` (5.4) / `Lua::new_versioned(LuaVersion::V51..V55)` — pick the
  version per instance. `LuaVersion`/`is_supported` live in `lua-types::version`.
- `Lua::scope` — lends Lua a non-`'static` borrow for one call; a handle that
  escapes the scope errors cleanly instead of dangling.
- `Lua::sandboxed(SandboxConfig)` — uncatchable CPU/memory caps + host stripping
  (see `docs/SANDBOXING_EXPLORATION.md`).
- `create_function`, `globals`, `load(...).exec()/.eval()`, `AnyUserData`, etc.

## The differential oracle lives here

`tests/multiversion_oracle.rs` is the **tier-2 inner loop** for the whole port —
in-process `Lua::new_versioned` + a `load`+`pcall` wrapper, asserting against
constants captured from the reference binaries. Every version/behavior fix lands
an assertion here. (Things that only appear in the CLI — tracebacks, `warn`/`__gc`
stderr, the `[C]: in ?` frame — go in `crates/lua-cli/tests/traceback_oracle.rs`,
which spawns the binary.)

## Stability rules

- Don't change public type/method signatures without a deliberate semver bump.
- Keep it building for `wasm32-unknown-unknown` (no C toolchain, no `std::fs`
  assumptions on the wasm path).
- `lua-hlua-shim` provides an alternate (hlua-shaped) facade — keep it in sync
  when the core API moves.

## Test
`cargo test -p lua-rs-runtime` (includes `multiversion_oracle` + embedding
doctests). `make rust` runs the doctests CI runs.
