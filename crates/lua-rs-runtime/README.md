# omnilua

Embed Lua in a Rust program. `omnilua` is the embedding API for
[omniLua](https://github.com/ianm199/omnilua), a pure-Rust Lua implementation
that runs **Lua 5.1–5.5** from one API (`Lua::new()` is 5.4; `Lua::new_versioned`
selects another). Being pure Rust, it builds for `wasm32-unknown-unknown` and
needs no C toolchain or `liblua`. It's young and it isn't LuaJIT, so if you need
either, use `mlua`.

```toml
[dependencies]
omnilua = "0.1.0"
```

## Calling Rust from Lua

```rust
use omnilua::{Lua, Result};

fn main() -> Result<()> {
    let lua = Lua::new();
    let f = lua.create_function(|_, name: String| Ok(format!("hello, {name}")))?;
    lua.globals().set("greet", f)?;

    let out: String = lua.load(r#"return greet("omniLua")"#).eval()?;
    assert_eq!(out, "hello, omniLua");
    Ok(())
}
```

The API is shaped after `mlua`: owned GC-rooted handles (`Value`, `Table`,
`Function`, `AnyUserData`), closure callbacks, conversion traits, and
`UserData` for binding Rust types with methods, fields, and metamethods. A
`#[derive(LuaUserData)]` / `#[lua_methods]` macro pair generates the
boilerplate (enable the `derive` feature).

## Scope: lending non-`'static` borrows

`Lua::scope` lends Lua a value that lives on the Rust stack for one call
(typically a game engine's `&mut World`). The borrow is invalidated when the
scope returns, so a script that stashes a handle and uses it later gets a clean
Lua error instead of touching freed memory.

```rust
lua.scope(|s| {
    let world = s.create_userdata_ref_mut(&lua, &mut my_world)?;
    lua.globals().set("world", &world)?;
    lua.load("world:spawn('player')").exec()
})?;
```

`Scope::create_function` does the same for closures that capture non-`'static`
borrows, and `AnyUserData::delegate` returns a sub-userdata that re-borrows a
field of its parent per call, so an `App -> World -> Component` chain stays a
chain of short borrows. Runnable example:
`cargo run -p omnilua --example scope_world`.

## Links

- Worked Bevy integration (a Lua script drives a Bevy entity, compiles to wasm):
  [examples/bevy](https://github.com/ianm199/omnilua/tree/main/examples/bevy).
- WebAssembly distribution: [`omnilua`](https://www.npmjs.com/package/omnilua) on npm.
- Project, benchmarks, and conformance: [github.com/ianm199/omnilua](https://github.com/ianm199/omnilua).

## License

A port of [Lua](https://www.lua.org/) (PUC-Rio). MIT-licensed.
