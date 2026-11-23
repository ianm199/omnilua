# omniLua

omniLua is a Lua interpreter written entirely in Rust, with no C dependency or
unsafe FFI. It runs Lua 5.1 through 5.5, works as a command-line tool or an
embedded library, and compiles to WebAssembly.

[![CI](https://github.com/ianm199/omnilua/actions/workflows/ci.yml/badge.svg)](https://github.com/ianm199/omnilua/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/omnilua-cli.svg?label=crates.io%2Fomnilua-cli)](https://crates.io/crates/omnilua-cli)
[![docs.rs](https://img.shields.io/docsrs/omnilua?label=docs.rs%2Fomnilua)](https://docs.rs/omnilua)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Five versions, one binary

Choose the Lua version at runtime. All five run from the same build:

```rust
let lua = Lua::new_versioned(LuaVersion::V51);   // V51 through V55
```
```bash
OMNILUA_VERSION=5.3 omnilua script.lua
```

Because omniLua is written in Rust, the same build compiles to
`wasm32-unknown-unknown` and runs in the browser. C-based Lua libraries can't
target that platform.

## Try it

Run all five versions in your browser without installing anything:
[omniLua playground](https://ianm199.github.io/omnilua/).

Or locally:

```bash
cargo install omnilua-cli        # installs the `omnilua` binary
omnilua -e 'print("hello")'
omnilua script.lua               # run a file; no args opens a REPL
```

In Node or the browser:

```bash
npm install omnilua
```

## Embed it in Rust

The `omnilua` crate is an embedding library. Its API is close to mlua's:

```rust
use omnilua::Lua;

let lua = Lua::new();
let greet = lua.create_function(|_, name: String| Ok(format!("hello, {name}")))?;
lua.globals().set("greet", greet)?;
lua.load(r#"print(greet("omniLua"))"#).exec()?;
```

The `scope` API lets you pass a non-`'static` borrow into Lua for one call, such
as a game's `&mut World`. If a handle outlives the scope, using it returns an
error instead of dangling:

```rust
lua.scope(|s| {
    let world = s.create_userdata_ref_mut(&lua, &mut my_world)?;
    lua.globals().set("world", &world)?;
    lua.load("world:spawn('player')").exec()
})?;
```

For untrusted scripts, the sandbox limits instructions and memory and removes
code loading, `io`, `debug`, `package` and the host-access parts of `os`:

```rust
let (lua, sandbox) = Lua::sandboxed(SandboxConfig::strict())?;
lua.load(untrusted_source).exec().ok();
sandbox.reset();
```

The full API is on [docs.rs](https://docs.rs/omnilua). The repository includes a
Bevy example in which a Lua script drives an entity each frame; it compiles to
the browser. See [`examples/bevy/`](examples/bevy/).

## Versions

All five versions run from the same core and are tested against the reference
Lua implementation. Version 5.4 is the most thoroughly tested and passes the
official PUC-Rio test suite, run with its portable `_soft`/`_port` settings
(which skip the heaviest and platform-specific checks). Version 5.5 passes its
official suite the same way.

Two behavioral notes: 5.1 and 5.2 have no integer type, so all numbers are
floats, and `math.random` uses Rust's PRNG and does not reproduce C's sequence.

## Browser and WebAssembly

The `omnilua` npm package runs Lua in the browser or Node without bundling a C
interpreter. The sandbox is also available over wasm for untrusted user scripts.
See [`packages/omnilua/README.md`](packages/omnilua/README.md).

## LuaRocks

omniLua runs the stock LuaRocks 3.11.1 client and installs pure-Lua rocks such
as `inspect`, `dkjson`, and `argparse`. C rocks are not supported yet.

## Speed

omniLua is an interpreter and runs at roughly 1.4× the time of reference C, or
1.3× with PGO. It does not JIT-compile; if you need LuaJIT speed, use mlua.
[Benchmarks are tracked per commit.](https://ianm199.github.io/omnilua/harness/bench/history/)

## License

omniLua is an AI-assisted port (a derivative work — a translation from C to
Rust) of the PUC-Rio reference implementation of [Lua](https://www.lua.org/)
5.4.7, extended from that base to run 5.1–5.5. Both Lua and this port are MIT
licensed. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
