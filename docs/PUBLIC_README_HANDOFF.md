# Public README Handoff

This is a content handoff for rewriting the root README into a public-facing
project page. The current README still reads like an agent-porting work log.
The public README should lead with the runtime and move the harness material
lower.

## Public Positioning

Lead with:

> `lua-rs` is a Lua 5.4.7 runtime implemented in Rust.

The project currently has two stories:

1. the runtime: `lua-rs`, published as the `lua-cli` crate with a `lua-rs`
   binary;
2. the porting harness: official-test runners, benchmark history, unsafe-budget
   checks, and debugging notes used to build the port.

The runtime story should come first. The harness story is valuable, but it is
secondary for new users evaluating a Rust Lua runtime.

## Current Facts To State

- Crates.io preview release exists at `0.0.1`.
- Install command:

  ```bash
  cargo install lua-cli
  ```

- Installed binary:

  ```bash
  lua-rs -e 'print("hello from lua-rs")'
  lua-rs script.lua
  ```

- The package name is `lua-cli`; the binary name is `lua-rs`.
- The upstream Lua 5.4.7 official suite passes 44/44 in the repo harness.
- Normal script execution has no C runtime dependency.
- Most crates forbid `unsafe`.
- Remaining unsafe is budgeted in `lua-gc` and the `lua-cli` dynamic-loading
  backend.
- The project is not LuaJIT.

## Do Not Overclaim

Do not claim:

- a REPL exists;
- stdin execution works;
- complete PUC-Rio Lua C API compatibility;
- ABI drop-in compatibility with `liblua`;
- arbitrary existing Lua C modules load unchanged;
- completely safe Rust;
- LuaJIT-level performance.

Current CLI gaps:

- `lua-rs -e '...'` works.
- `lua-rs script.lua` works.
- a bare non-file argument is treated as source and works.
- `echo 'print("hi")' | lua-rs -` does not work today.
- `lua-rs --help` is not a polished help path today.
- There is no REPL today.

Good phrasing:

> The preview release targets Lua source/runtime compatibility first. Rust-native
> embedding and C API compatibility are future goals.

See `docs/FUTURE_GOALS.md` for the API/ABI distinction.

## Suggested README Shape

1. `# lua-rs`
2. One-paragraph description.
3. Status bullets.
4. Install.
5. Usage examples.
6. Compatibility / conformance.
7. Performance.
8. Safety model.
9. Limitations and non-goals.
10. Crate layout.
11. Development / verification commands.
12. Porting harness docs.

## Conformance Section

Suggested wording:

> The repository's Lua 5.4.7 official-test harness currently passes 44/44 tests.
> This is strong evidence for Lua source/runtime compatibility. It does not imply
> C API or ABI compatibility.

Useful command:

```bash
RUSTFLAGS='-Awarnings' cargo build -q --bin lua-rs
RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=90 ./harness/run_official_all.sh
```

Useful link:

- `docs/OFFICIAL_TEST_INVESTIGATIONS.md`

## Performance Section

The dashboard is the strongest public performance artifact:

- local file: `harness/bench/history/index.html`
- source data: `harness/evidence/ledger.jsonl`
- builder: `python3 harness/bench/history.py`

Latest tracked dashboard artifact reports:

- 1046 measurements over 36 commits;
- wall-time geomean about `1.27x` versus upstream PUC-Rio Lua 5.4.7 across the
  benchmark set;
- RSS geomean about `1.96x`;
- workload-level variance, including some workloads below `1.0x` and others
  above it.

Lower ratio is better. `1.00x` is parity with upstream Lua on the same workload.

Do not summarize this as simply "faster than C." Link the dashboard and let the
workload-level data carry the nuance.

Current public-link caveat:

- local `main` was ahead of `origin/main` when this note was written, so any
  RawGithack/GitHub Pages link based on `main` may show stale dashboard data
  until commits are pushed.

## Safety Section

Suggested wording:

> `lua-rs` exposes a safe public surface over a small audited unsafe core. Most
> crates forbid unsafe code. The remaining unsafe surface is budgeted in `lua-gc`
> and the `lua-cli` dynamic-library backend.

Do not call the project "completely safe Rust."

Useful links:

- `docs/PUBLISH_READINESS.md`
- `docs/LUA_SYSTEM_DEEP_DIVE.md`

## LuaRocks Status

LuaRocks is not ready for a public headline yet.

Current verified state:

- LuaRocks 3.11.1 boots far enough under `lua-rs` to print the CLI
  help/version/config output.
- The smoke still exits nonzero with:

  ```text
  lua: pcall_k failed: Runtime: error in error handling
  ```

- `luarocks install <rock>` has not been made to work yet.

Smoke command used:

```bash
curl -sSL https://luarocks.org/releases/luarocks-3.11.1.tar.gz | tar xz -C /tmp
tail -n +2 /tmp/luarocks-3.11.1/src/bin/luarocks > /tmp/luarocks_noshebang.lua
LUA_PATH="/tmp/luarocks-3.11.1/src/?.lua;/tmp/luarocks-3.11.1/src/?/init.lua" \
  ./target/debug/lua-rs -e 'arg={[0]="luarocks","--version"}; dofile("/tmp/luarocks_noshebang.lua")'
```

Good roadmap phrasing:

> LuaRocks self-hosting is in progress: the LuaRocks 3.11.1 CLI boots and prints
> version/config output under `lua-rs`; package installation still needs
> fetch/digest/install-path work.

Expected LuaRocks work:

- fix the clean-exit/error-handler path;
- clean up the `=[C]` program/chunk name;
- first target a curated local `file://` rock repo;
- add digest support if needed;
- add HTTP/HTTPS later;
- avoid arbitrary native rocks until a C API/ABI strategy exists.

See `docs/PHASE_G_LUAROCKS_PLAN.md`.

## Existing Docs Worth Linking

- `docs/FUTURE_GOALS.md`: compatibility targets and C API/ABI distinction.
- `docs/OFFICIAL_TEST_INVESTIGATIONS.md`: how the official suite reached 44/44.
- `docs/PERFORMANCE_PRINCIPLES.md`: benchmark method and performance policy.
- `docs/MATCHING_C_PERFORMANCE.md`: deeper performance analysis.
- `harness/bench/README.md`: benchmark runner and dashboard usage.
- `docs/PHASE_G_LUAROCKS_PLAN.md`: LuaRocks plan and current status.
- `docs/LUA_SYSTEM_DEEP_DIVE.md`: architecture, GC, unsafe model, gaps.
