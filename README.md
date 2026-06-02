# lua-rs

A pure-Rust Lua interpreter — one implementation that runs **Lua 5.1, 5.2, 5.3,
5.4, and 5.5**, selected per instance. Runs as a standalone binary with no C
dependency, embeds in Rust programs and in the browser. The stable 5.4 backend
passes the full upstream PUC-Rio test suite.

[![CI](https://github.com/ianm199/lua-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/ianm199/lua-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/lua-cli.svg?label=crates.io%2Flua-cli)](https://crates.io/crates/lua-cli)
[![docs.rs](https://img.shields.io/docsrs/lua-rs-runtime?label=docs.rs%2Flua-rs-runtime)](https://docs.rs/lua-rs-runtime)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

```bash
cargo install lua-cli            # crate `lua-cli`; the binary it installs is `lua-rs`
lua-rs -e 'print("hello")'
lua-rs script.lua                # run a file; `lua-rs` for the REPL
```

It is competitive with reference C (~1.3× geomean wall time, benchmarked per
[commit](https://ianm199.github.io/lua-rs/harness/bench/history/)), not faster,
and is not LuaJIT. Most crates build under `#![forbid(unsafe_code)]`; the
trusted unsafe is budgeted in the GC, the dynamic-library loader, and the WASM
pointer ABI.

## Embed it in Rust

[`lua-rs-runtime`](https://crates.io/crates/lua-rs-runtime) is an embedding API
shaped after `mlua`. Being pure Rust, it builds for `wasm32-unknown-unknown`
and needs no C toolchain or `liblua`. It's young and it isn't LuaJIT, so if you
need either, use `mlua`.

```rust
let lua = Lua::new();
let f = lua.create_function(|_, name: String| Ok(format!("hello, {name}")))?;
lua.globals().set("greet", f)?;
lua.load(r#"print(greet("lua-rs"))"#).exec()?;
```

`Lua::scope` lends Lua a non-`'static` borrow (e.g. a game engine's `&mut
World`) for one call; a handle that escapes the scope errors cleanly instead of
dangling. `AnyUserData::delegate` returns live sub-references into it.

```rust
lua.scope(|s| {
    let world = s.create_userdata_ref_mut(&lua, &mut my_world)?;
    lua.globals().set("world", &world)?;
    lua.load("world:spawn('player')").exec()
})?;
```

Full API on [docs.rs](https://docs.rs/lua-rs-runtime). Worked Bevy 0.18
integration:
[bevy-lua-rs-starter](https://github.com/ianm199/bevy-lua-rs-starter)
([live demo](https://ianm199.github.io/bevy-lua-rs-starter/)).

## Multiple Lua versions

The same API and binary run **Lua 5.1 through 5.5**, selected per instance:

```rust
let lua = Lua::new_versioned(LuaVersion::V51); // V51..V55; Lua::new() is 5.4
```

The CLI selects with `LUA_RS_VERSION=5.1|5.2|5.3|5.4|5.5`. All five share one
core — the bytecode dispatch loop carries no per-version cost, so compute-bound
code runs identically across versions; version differences (number model,
globals model, stdlib roster, error wording) live in cold-path seams. Every
version is verified against its unmodified upstream reference binary.

Maturity, by version:

- **5.4** — stable; passes the full upstream PUC-Rio suite.
- **5.3 / 5.5** — beta; long tails closed (compat-math, error wording, `global`
  declarations, named varargs, traceback fidelity), verified against the
  reference. A few documented divergences remain (see the open issues).
- **5.1 / 5.2** — supported, float-only number family (5.2 on the modern `_ENV`
  globals model; 5.1 adds fenv globals). Verified against the reference binary
  via a behavioral battery and the upstream example programs rather than a
  bundled conformance suite, so treat them as the newest backends. `math.random`
  sequences differ (host PRNG); see the [changelog](CHANGELOG.md).

Use 5.4 for production; the others are good for running version-specific scripts
and for the embedding use cases the [playbook](specs/MULTIVERSION_PLAYBOOK.md)
describes.

## Running untrusted Lua

Bound CPU and memory and strip host access, so a buggy or hostile script can't
hang the process, exhaust memory, or reach the filesystem. Limits are enforced
on every thread (coroutines included) and are **uncatchable** — a script can't
escape them with `pcall`. A non-sandboxed runtime pays zero overhead.

```rust
let (lua, sandbox) = Lua::sandboxed(SandboxConfig::strict())?;
match lua.load(untrusted_source).exec() {
    Ok(()) => { /* finished within limits */ }
    Err(_) => match sandbox.tripped() {
        Some(TripReason::Instructions) => { /* CPU budget hit */ }
        Some(TripReason::Memory)       => { /* memory ceiling hit */ }
        None                           => { /* ordinary Lua error */ }
    },
}
sandbox.reset(); // refill the budget before re-running
```

From the CLI:

```
lua-rs --sandbox script.lua              # strip host globals + default caps
lua-rs --max-instructions=5000000 s.lua  # CPU budget
lua-rs --max-memory=64M s.lua            # memory ceiling (K/M/G suffixes)
```

Design and threat model:
[docs/SANDBOXING_EXPLORATION.md](docs/SANDBOXING_EXPLORATION.md).

## Browser / WebAssembly

Ships as [`lua-rs-wasm`](https://www.npmjs.com/package/lua-rs-wasm) (npm) for
running Lua in the browser or Node without bundling the C interpreter. Try it in
the [playground](https://ianm199.github.io/lua-rs/examples/wasm-browser/); see
[`harness/wasm/README.md`](harness/wasm/README.md) for the host-hook API.

Sandboxing is exposed over the WASM ABI for running untrusted user scripts:
`lua_rs_wasm_set_limits(max_instructions, max_memory, strict)` before
`lua_rs_wasm_run`, then `lua_rs_wasm_last_trip()` to learn which limit (if any)
stopped a run, and `lua_rs_wasm_sandbox_reset()` to refill the budget.

## LuaRocks

Runs the stock LuaRocks 3.11.1 client and installs pure-Lua rocks (`inspect`,
`dkjson`, `argparse`, `middleclass`, `say`, `luassert`). Native C rocks are not
supported yet.

## More

- Conformance: `TEST_TIMEOUT_S=90 ./harness/run_official_all.sh` runs the
  unmodified upstream suite against `lua-rs` and reports the live pass count.
  This is Lua source/runtime compatibility, not C API/ABI compatibility.
- Building, testing, and contributing: [CONTRIBUTING.md](CONTRIBUTING.md).
- Embedding internals and roadmap:
  [docs/EMBEDDING_API_IMPLEMENTATION.md](docs/EMBEDDING_API_IMPLEMENTATION.md),
  [docs/FUTURE_GOALS.md](docs/FUTURE_GOALS.md).

## License

A port of [Lua](https://www.lua.org/) (Roberto Ierusalimschy, Luiz Henrique de
Figueiredo, and Waldemar Celes, PUC-Rio). Lua and this port are both
MIT-licensed. See [LICENSE](LICENSE).
