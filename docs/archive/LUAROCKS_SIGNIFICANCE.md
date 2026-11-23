# What The LuaRocks Result Means

LuaRocks is the standard package manager for Lua. It plays roughly the same
role for Lua that Cargo plays for Rust, npm plays for JavaScript, and pip plays
for Python.

Getting LuaRocks running under `lua-rs` means this runtime is no longer only
passing isolated language tests. It can run a real Lua ecosystem tool, ask the
public LuaRocks server for a package, install that package into a LuaRocks tree,
rebuild the tree manifest, and then load the installed module with `require`.

The verified end-to-end example is:

```bash
luarocks install inspect
```

running through:

```bash
lua-rs /tmp/luarocks-3.11.1/src/bin/luarocks --tree /tmp/lua-rs-remote-tree install inspect
```

Then `lua-rs` can load the result:

```lua
local inspect = require("inspect")
print(inspect({ ok = true }))
```

## Why This Is Cool

The official Lua test suite proves that the language semantics are close to
upstream Lua. LuaRocks proves something different: the runtime can survive real
Lua software.

LuaRocks stresses a broad set of behavior at once:

- command-line script arguments and global `arg`;
- `os.exit` process behavior;
- `os.execute` subprocess behavior;
- `io.open`, `io.popen`, and file read error handling;
- `package.path`, `require`, and installed module lookup;
- LuaFileSystem-style directory traversal and install-tree locking;
- HTTP-backed package metadata through LuaRocks' own fetch path;
- manifest generation and installed-package discovery.

That is valuable because real software finds integration bugs that small
language tests often miss. In this case, LuaRocks exposed several concrete
compatibility gaps: script varargs, clean `os.exit`, missing `os.execute`,
`lfs.lock_dir`, Unix shebang handling, and macOS directory-probe errno behavior.

Fixing those made the runtime more Lua-like in ways users will actually notice.

## The Honest Claim

Good public wording:

> `lua-rs` can run LuaRocks 3.11.1 well enough to install and use pure-Lua rocks
> such as `inspect`.

That is a meaningful ecosystem milestone. It means existing pure-Lua packages
can plausibly work without being rewritten for this project.

## Examples Of Useful Pure-Lua Rocks

Pure-Lua rocks are packages that install `.lua` files and run on the Lua VM
without compiling or loading a native C extension. These are the packages that
fit `lua-rs` best today.

Useful categories:

- **Inspection/debugging**: `inspect`, `serpent`
- **JSON**: `dkjson`, `lunajson`
- **CLI argument parsing**: `argparse`
- **Testing/assertions**: `luassert`, `say`
- **Object helpers**: `middleclass`
- **Templating/text utilities**: packages such as `etlua` or small string/table
  helper libraries, when they have no native dependencies

Verified so far under `lua-rs`:

| Rock | Category | Result |
|---|---|---|
| `inspect` | table inspection/debugging | installs, loads, runs |
| `dkjson` | JSON encode/decode | installs, loads, encode/decode smoke passes |
| `argparse` | command-line parser | installs, loads, parse smoke passes |
| `middleclass` | object/class helper | installs, loads, constructor smoke passes |
| `say` | testing message helper | installs as a pure-Lua dependency |
| `luassert` | assertions/testing | installs with `say`, loads, assertion smoke passes |

This is a useful starter matrix because it covers real package installation,
dependency resolution, module lookup, and a few common library shapes.

## What It Does Not Mean

This does not mean every LuaRocks package works.

LuaRocks packages fall into two broad groups:

- **Pure-Lua rocks**: packages implemented in Lua source files. These are the
  current sweet spot.
- **Native C rocks**: packages that compile or load C modules against the
  PUC-Rio Lua C API/ABI. These are not supported as stock binary modules today.

The `luafilesystem` probe demonstrated the boundary clearly: LuaRocks reached
the native build step and failed looking for `lua.h`. Even if headers were
provided, stock C rocks still need either a real Lua C API/ABI compatibility
layer or targeted Rust-native replacements.

So the correct claim is not "LuaRocks is fully supported." The correct claim is
"LuaRocks can install and use real pure-Lua packages, and native C rocks are the
next compatibility frontier."

## Why This Matters For The Project

Before this, the strongest evidence was:

- the upstream Lua 5.4.7 official suite passes 44/44;
- benchmarks show the runtime is in the performance conversation;
- crates are published.

With LuaRocks, there is now a fourth proof point:

- the runtime can participate in the Lua package ecosystem for pure-Lua code.

That makes the project easier to explain. It is not just a port that passes
tests. It is a Lua runtime in Rust that can run a real Lua package manager and
consume real Lua packages.

## Good README-Sized Version

> LuaRocks self-hosting is in progress. `lua-rs` can run LuaRocks 3.11.1 well
> enough to search, install, list, show, and use pure-Lua rocks such as
> `inspect`. Native C rocks remain out of scope until the project has either
> targeted Rust-native module replacements or a PUC-Rio Lua C API/ABI
> compatibility layer.
