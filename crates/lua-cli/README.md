# omnilua-cli

The command-line interpreter for [omniLua](https://github.com/ianm199/omnilua),
a pure-Rust Lua (5.1–5.5) implementation in safe Rust. The crate is
`omnilua-cli`; the binary it installs is `omnilua`.

```bash
cargo install omnilua-cli
omnilua -e 'print(("safe rust"):upper())'   # SAFE RUST
```

## Usage

```bash
omnilua                          # REPL
omnilua script.lua               # run a file
omnilua -e 'print(1 + 2)'        # run a one-liner
echo 'print("hi")' | omnilua -   # read from stdin
omnilua -v                       # version
```

It mirrors the standard `lua` CLI: a bare argument is a script filename, `-e`
runs a chunk, `-` reads from stdin, and no arguments on a terminal starts the
REPL. The default version is 5.4; set `OMNILUA_VERSION=5.1` (through `5.5`) to
select another.

## What it is

A from-scratch reimplementation of PUC-Rio Lua (lexer, parser, bytecode
compiler, register VM, garbage collector, coroutines, standard library) with no
C dependency. It runs the unmodified upstream Lua 5.4.7 test suite against the
binary and passes 44/44, and it runs the stock LuaRocks 3.11.1 client for
pure-Lua rocks. It is competitive with reference C (about 1.45x geomean wall
time on a stock build, ~1.3x with PGO), not faster, and is not LuaJIT.

To embed Lua in a Rust program instead of running it from the shell, use
[`omnilua`](https://crates.io/crates/omnilua).

Full documentation, benchmarks, and source:
[github.com/ianm199/omnilua](https://github.com/ianm199/omnilua).

## License

A port of [Lua](https://www.lua.org/) (Roberto Ierusalimschy, Luiz Henrique de
Figueiredo, and Waldemar Celes, PUC-Rio). Lua and this port are both
MIT-licensed.
