# lua-rs-port

Rust port of Lua 5.4.7, built alongside an AI-agent porting harness.

There are two artifacts here:

1. **`lua-rs`**: a Lua 5.4 runtime implemented in Rust.
2. **The porting harness**: scripts, docs, oracle tests, hooks, and debugging
   patterns for driving a large C-to-Rust port with agents.

## Current Status

As of 2026-05-24:

- the harnessed official Lua test suite passes **44/44**;
- normal script execution has no C runtime dependency;
- most crates forbid `unsafe`;
- the remaining unsafe surface is explicitly budgeted in `lua-gc` and the
  `lua-cli` dynamic-library backend;
- this is not a drop-in replacement for C-Lua's C ABI;
- this is not LuaJIT.

The best current safety phrasing is: safe public surface over a small audited
unsafe core. Do not call the project "completely safe Rust."

## Quick Start

Run a Lua snippet:

```bash
RUSTFLAGS='-Awarnings' cargo run -q --bin lua-rs -- 'print("hello from lua-rs")'
```

Run the official suite:

```bash
RUSTFLAGS='-Awarnings' cargo build -q --bin lua-rs
RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=90 ./harness/run_official_all.sh
```

Run the unsafe budget gate:

```bash
.claude/hooks/unsafe-budget.sh
```

## Read Next

- [docs/PUBLISH_READINESS.md](docs/PUBLISH_READINESS.md): what "ready to
  publish" means for this repo.
- [docs/LUA_SYSTEM_DEEP_DIVE.md](docs/LUA_SYSTEM_DEEP_DIVE.md): architecture,
  GC, unsafe model, and remaining runtime gaps.
- [docs/PERFORMANCE_PRINCIPLES.md](docs/PERFORMANCE_PRINCIPLES.md):
  performance philosophy and benchmark process.
- [docs/OFFICIAL_TEST_INVESTIGATIONS.md](docs/OFFICIAL_TEST_INVESTIGATIONS.md):
  hard official-test debugging notes.
- [PORTING.md](PORTING.md): translation rules used by the agent harness.
- [HARNESS_DESIGN.md](HARNESS_DESIGN.md): harness structure and enforcement
  model.

## Non-Goals

- LuaJIT-level performance.
- Compatibility with Lua 5.1-specific systems such as OpenResty, Neovim's
  LuaJIT embedding, or World of Warcraft addons.
- Transparent C-Lua ABI compatibility. Dynamic loading exists at the CLI
  backend boundary, but a stock Lua C module expects the C API/ABI, which this
  runtime does not currently expose.
