# omniLua

**Every Lua, everywhere.** One pure-Rust runtime for Lua 5.1, 5.2, 5.3, 5.4, and
5.5 — as a standalone interpreter, embedded in Rust, or in the browser. No C
dependency, no unsafe FFI. Passes the official PUC-Rio test suites. Runs the
stock LuaRocks client.

[![CI](https://github.com/ianm199/omnilua/actions/workflows/ci.yml/badge.svg)](https://github.com/ianm199/omnilua/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/omnilua-cli.svg?label=crates.io%2Fomnilua-cli)](https://crates.io/crates/omnilua-cli)
[![docs.rs](https://img.shields.io/docsrs/omnilua?label=docs.rs%2Fomnilua)](https://docs.rs/omnilua)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

If your Rust program — or your game — ships a wasm build, a C-backed Lua binding
can't follow it. omniLua is pure Rust: the same scripting runtime compiles
natively and to `wasm32-unknown-unknown`, with no Emscripten and no toolchain
gymnastics.

## Try it in 30 seconds

Run five Luas side by side in the browser, no install:
[**omniLua playground**](https://ianm199.github.io/omnilua/).

Or locally:

```bash
cargo install omnilua-cli         # crate `omnilua-cli`; the binary it installs is `omnilua`
omnilua -e 'print("hello")'
omnilua script.lua                # run a file; `omnilua` with no args for the REPL
```

In the browser or Node via npm:

```bash
npm install omnilua
```

## What's checkable

Conformance you can check: the official Lua test suites pass on every supported
version, benchmarks vs reference C are published per-commit, and we measured
what memory safety costs — see
[`docs/PERFORMANCE_MODEL.md`](docs/PERFORMANCE_MODEL.md) (§Safety-tax ablation):
~0% of wall time.

- **Suites.** `TEST_TIMEOUT_S=90 ./harness/run_official_all.sh` runs the
  unmodified upstream suite against `omnilua` and reports the live pass count.
  5.4 passes the full PUC-Rio suite; 5.3 and 5.5 run their own upstream test
  trees in beta; 5.1 and 5.2 are verified against the reference binary via a
  behavioral battery and the upstream example programs. This is Lua
  source/runtime compatibility, not C API/ABI compatibility.
- **Benchmarks.** The omniLua/reference-C wall and RSS ratios are tracked
  per-commit on the
  [bench dashboard](https://ianm199.github.io/omnilua/harness/bench/history/).
  ~1.45× geomean of reference-C wall time on the stock release build (~1.3×
  with PGO).
- **Safety tax.** The bounds-checks-and-`RefCell`-guards ablation costs ~0% of
  reliable wall time — the residual gap to C is representation, not safety. The
  unsafe budget stays at zero outside the GC, the dynamic-library loader, and
  the wasm pointer ABI.

## Embed it in Rust

[`omnilua`](https://crates.io/crates/omnilua) is an embedding API shaped after
`mlua`. Being pure Rust, it builds for `wasm32-unknown-unknown` and needs no C
toolchain or `liblua`.

```rust
use omnilua::Lua;

let lua = Lua::new();
let f = lua.create_function(|_, name: String| Ok(format!("hello, {name}")))?;
lua.globals().set("greet", f)?;
lua.load(r#"print(greet("omniLua"))"#).exec()?;
```

`Lua::scope` lends Lua a non-`'static` borrow (e.g. a game engine's `&mut
World`) for one call; a handle that escapes the scope errors cleanly instead of
dangling.

```rust
lua.scope(|s| {
    let world = s.create_userdata_ref_mut(&lua, &mut my_world)?;
    lua.globals().set("world", &world)?;
    lua.load("world:spawn('player')").exec()
})?;
```

Untrusted scripts get uncatchable CPU and memory caps with host access stripped:

```rust
use omnilua::{Lua, SandboxConfig};

let (lua, sandbox) = Lua::sandboxed(SandboxConfig::strict())?;
lua.load(untrusted_source).exec().ok();
sandbox.reset(); // refill the budget before re-running
```

Full API on [docs.rs](https://docs.rs/omnilua). In-repo Bevy demo — a Lua
script drives a Bevy entity each frame, and the whole thing compiles to
`wasm32-unknown-unknown` (the thing a C-backed binding can't do):
[`examples/bevy/`](examples/bevy/).

## Honesty

~1.45× geomean of reference-C wall time on a stock release build (~1.3× with
PGO) — competitive, not faster, and not LuaJIT. If you need LuaJIT speed or a decades-mature binding, use `mlua`. Native
C rocks are not supported in LuaRocks yet (pure-Lua rocks are). The 5.4 backend
is production-ready; the other versions are the newer surfaces (see the table).

## Supported versions

The same API and binary run all five, selected per instance
(`Lua::new_versioned(LuaVersion::V51)`; the CLI reads
`OMNILUA_VERSION=5.1|5.2|5.3|5.4|5.5`, with `LUA_RS_VERSION` honoured as a
fallback). All five share one core — the bytecode dispatch loop carries no
per-version cost — so compute-bound code runs identically across versions, and
version differences live in cold-path seams.

| Version | Status | Verified against |
|---|---|---|
| **5.4** | Stable; production | Full upstream PUC-Rio suite |
| **5.3 / 5.5** | Beta; long tails closed | Their own upstream test trees + reference binary |
| **5.1 / 5.2** | Supported; newest backends | Behavioral battery + upstream example programs |

5.1/5.2 are float-only number families (5.2 on the modern `_ENV` globals model;
5.1 adds fenv globals); `math.random` sequences differ from C (host PRNG). The
per-version methodology lives in
[`specs/MULTIVERSION_PLAYBOOK.md`](specs/MULTIVERSION_PLAYBOOK.md).

## Browser / WebAssembly

The npm package [`omnilua`](https://www.npmjs.com/package/omnilua) runs Lua in
the browser or Node without bundling the C interpreter. Sandboxing is exposed
over the wasm ABI for untrusted user scripts. Wrapper API and a runnable example:
[`packages/omnilua/README.md`](packages/omnilua/README.md).

## LuaRocks

Runs the stock LuaRocks 3.11.1 client and installs pure-Lua rocks (`inspect`,
`dkjson`, `argparse`, `middleclass`, `say`, `luassert`). Native C rocks are not
supported yet.

## More

- Building, testing, and contributing: [CONTRIBUTING.md](CONTRIBUTING.md).
- Sandboxing design and threat model:
  [docs/SANDBOXING_EXPLORATION.md](docs/SANDBOXING_EXPLORATION.md).
- Embedding internals and roadmap:
  [docs/EMBEDDING_API_IMPLEMENTATION.md](docs/EMBEDDING_API_IMPLEMENTATION.md).

## License

A port of [Lua](https://www.lua.org/) (Roberto Ierusalimschy, Luiz Henrique de
Figueiredo, and Waldemar Celes, PUC-Rio). Lua and this port are both
MIT-licensed. See [LICENSE](LICENSE).
</content>
</invoke>
