# Examples

Runnable Lua programs that exercise `omniLua`. Each runs on the reference
PUC-Rio Lua 5.4.7 interpreter and on `omniLua` with identical output.

Run one with the installed binary:

```bash
omnilua examples/fibonacci.lua
```

or from a source build:

```bash
cargo build --bin omnilua
./target/debug/omnilua examples/fibonacci.lua
```

| File | Shows |
|---|---|
| [`fibonacci.lua`](fibonacci.lua) | recursion, table memoization, integer `//` vs float `/` |
| [`coroutines.lua`](coroutines.lua) | coroutines as lazy generators and producer/consumer |
| [`oop.lua`](oop.lua) | object-orientation and inheritance via metatables |
| [`patterns.lua`](patterns.lua) | Lua string patterns: `match`, `gmatch`, `gsub` |
| [`errors.lua`](errors.lua) | `pcall`, table error objects, `<close>` to-be-closed variables |
| [`wasm-browser/`](wasm-browser/) | static browser playground for the published `omnilua` package |

Run them all:

```bash
for f in examples/*.lua; do echo "== $f =="; omnilua "$f"; done
```
