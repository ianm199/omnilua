# omnilua

Run Lua in the browser or Node, with no C interpreter to bundle and no native
build step. `omnilua` is a pure-Rust Lua runtime compiled to WebAssembly — you
ship one `.wasm` file and a small JS wrapper, and it runs the same Lua your
backend does.

If your app — or your game — ships a wasm build, a C-backed Lua binding can't
follow it. omniLua is pure Rust: the same scripting runtime compiles natively
and to `wasm32-unknown-unknown`, with no Emscripten and no toolchain gymnastics.

## Install

```bash
npm install omnilua
```

The package ships the `.wasm` artifact plus an ES-module wrapper. There is no
postinstall build and no native dependency.

## Use it

You give the runtime a host environment (virtual files, env vars, a stdout
sink), then run Lua source through it. The runtime keeps one Lua state alive
across `exec` calls until you `reset()`.

```js
import { loadLuaRs, luaRsWasmUrl } from "omnilua";

const { lua } = await loadLuaRs(luaRsWasmUrl, {
  files: {
    "./greeter.lua":
      "return { message = function(name) return 'hello ' .. name end }",
  },
  onStdout: (chunk) => console.log(chunk),
});

lua.exec(`
  local greeter = require("greeter")
  print(greeter.message("wasm"))
`);
```

`exec` throws on a Lua error. When you want to inspect the failure instead of
catching an exception, use `tryExec`:

```js
const result = lua.tryExec('error("boom")');
console.log(result.ok);    // false
console.log(result.error); // the Lua error text
```

In Node without a bundler, read the packaged `.wasm` with the `/node` entry
point, which otherwise behaves identically:

```js
import { loadLuaRsNode } from "omnilua/node";

const { lua } = await loadLuaRsNode({
  onStdout: (chunk) => process.stdout.write(chunk),
});
lua.exec('print("hello from node")');
```

## Running untrusted scripts

Bound CPU and memory and strip host access before running scripts you don't
trust. Limits are enforced on every thread (coroutines included) and **cannot be
caught** with `pcall`. Call `setLimits` once, then run as usual; `lastTrip`
reports which limit (if any) stopped a run, and `sandboxReset` refills the
budget.

```js
lua.setLimits({
  maxInstructions: 5_000_000,
  maxMemory: 64 * 1024 * 1024,
  strict: true, // also remove os.execute, io, load, require, debug, …
});

const result = lua.tryExec("while true do end"); // runaway user script
console.log(result.ok);       // false
console.log(lua.lastTrip());  // "instructions"  ("memory" | null)

lua.sandboxReset(); // refill the budget for the next run
```

Omit a limit (or pass `0`) to leave that dimension unbounded.

## Which Lua version

The published npm artifact runs **Lua 5.4**. omniLua's defining feature —
running 5.1, 5.2, 5.3, 5.4, and 5.5 from one core, selected per instance — is
exposed today through the native crate (`Lua::new_versioned(...)`) and is
demonstrated live in the
[playground](https://ianm199.github.io/omnilua/), which runs the same snippet
across all five versions side by side. Per-version selection over the wasm ABI
is on the roadmap; until then, treat this package as the 5.4 runtime.

## Size expectations

You ship one WebAssembly module (the Lua runtime — lexer, parser, VM, GC, and
standard library) plus a few kilobytes of JS wrapper. There is no Emscripten
glue and no separate `liblua`. Serve the `.wasm` with `Content-Type:
application/wasm` and gzip/brotli compression and let the browser stream-compile
it; `loadLuaRs(luaRsWasmUrl)` fetches it for you.

## Links

- Source, issues, full docs:
  [github.com/ianm199/omnilua](https://github.com/ianm199/omnilua)
- Live playground (all five Lua versions):
  [ianm199.github.io/omnilua](https://ianm199.github.io/omnilua/)
- Embedding in Rust (the native crate): [`omnilua` on
  crates.io](https://crates.io/crates/omnilua)

## License

A port of [Lua](https://www.lua.org/) (PUC-Rio). Lua and this port are both
MIT-licensed.
</content>
