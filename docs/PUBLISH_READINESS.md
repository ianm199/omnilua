# Publish Readiness

Status date: 2026-05-24.

This project is close to a credible public preview, but not yet ready for a
blind crates.io release. Treat "publish" as three separate lanes:

1. public repository / technical preview;
2. binary release of `lua-rs`;
3. crates.io release of the crate graph.

The first lane is the nearest target. The third lane needs explicit naming,
dependency, and API decisions.

## Current Claim

The honest public claim is:

- Lua 5.4.7-compatible runtime in Rust;
- official Lua test harness currently passes 44/44 in the port-compatible
  runner;
- no C runtime dependency for normal script execution;
- safe public surface over a small audited unsafe core;
- not a drop-in replacement for C-Lua's C ABI;
- not LuaJIT and not intended to match LuaJIT performance.

## Unsafe Positioning

Do not describe the project as "completely safe Rust."

Describe it as:

> The runtime keeps unsafe code behind explicit budgets. Most crates forbid
> unsafe code. The trusted unsafe core is currently `lua-gc` plus the
> `lua-cli` dynamic-library backend.

Current budget summary:

- `lua-gc`: 13 counted sites, all in `heap.rs`;
- `lua-cli`: 5 counted sites for `libloading` / dynamic module loading;
- `lua-coro`: 0, with `unsafe_code = "forbid"` until a concrete stackful
  backend lands;
- all other runtime crates: expected 0.

The remaining `lua-gc` unsafe is the collector kernel: `Gc<T>` dereference,
intrusive allgc walking, gray-queue dereference, sweep cursor management, and
`Box::from_raw` reclamation. The key safety invariant is that collection only
runs at safepoints where live `Gc<T>` handles are reachable from the traced root
graph.

## Package Audit

Immediate findings:

- Root `LICENSE` is required because manifests declare `license = "MIT"`.
- Root README must stay current with the actual 44/44 status and the unsafe
  story.
- Accidental runtime artifacts do not belong in the repo root. Known examples
  removed during this pass: `appendonly.aof`, `dump.rdb`, `luac.out`.
- Internal workspace dependencies now specify both `path` and `version`, which
  is required for crates.io packaging.
- Manifests now have shared description, repository, and homepage metadata.
- Full verified packaging of dependent crates requires publishing internal
  crates in dependency order first, because Cargo verifies packaged crates
  against registry dependencies, not unpublished local paths.
- `lua-cli-test-rust-module` is a test fixture and should not be published.
- The crate names are working names. Before crates.io, decide whether to use
  these names or a namespaced family such as `lua-rs-*`.
- Crates.io API returned 404 for the current working crate names on
  2026-05-24, so the names appeared available at the time of this check.

## Public Preview Checklist

Required before calling the repo public-preview ready:

- README states the current compatibility, safety, and non-goal story.
- `LICENSE` exists and matches manifest metadata.
- Root-level generated or accidental artifacts are removed or ignored.
- Official suite is green: `RUSTFLAGS='-Awarnings' TEST_TIMEOUT_S=90 ./harness/run_official_all.sh`.
- Workspace check is green: `RUSTFLAGS='-Awarnings' cargo check -q`.
- Unsafe budget hook is green: `.claude/hooks/unsafe-budget.sh`.
- Package readiness smoke is green: `./harness/check_publish_readiness.sh`.
- Performance dashboard has a recent datapoint and does not claim C parity
  without context.
- A known-gaps section links to deeper docs instead of hiding limitations.

## Binary Release Checklist

Required before releasing `lua-rs` binaries:

- Decide target triples and build mode.
- Add a smoke test for `lua-rs 'print("hello")'`.
- Track `Cargo.lock` or document why binary release builds are intentionally
  unlocked.
- Build release binary with `cargo build --release --bin lua-rs`.
- Run the official suite against the release binary using `LUA_RS_BIN`.
- Document unsupported C ABI behavior and dynamic-loading caveats.

## Crates.io Checklist

Required before publishing crates:

- Decide crate names and ownership.
- Add package metadata: description, repository, readme, keywords/categories
  where appropriate.
- Add version requirements to all internal dependencies while keeping local
  `path` entries for workspace development.
- Mark fixtures/internal-only crates with `publish = false`.
- Decide whether low-level crates are public API or implementation details.
- Publish/verify in dependency order:
  `lua-gc`, `lua-types`, `lua-vm`, then leaves such as `lua-code`,
  `lua-lex`, `lua-stdlib`, `lua-coro`, `lua-rs-lfs`, `lua-parse`,
  and finally `lua-cli`.
- Run `cargo package` without `--no-verify` for each publishable crate.
- Review package contents with `cargo package --list`.

## Recommended Next Order

1. Make the repo public-preview honest: README, LICENSE, artifact cleanup,
   publish-readiness doc.
2. Commit the unsafe cleanup and publish-readiness cleanup as one coherent
   checkpoint.
3. Expand `harness/check_publish_readiness.sh` into a full release gate that
   also runs the official suite, unsafe budget, and a CLI smoke test.
4. Decide product naming before touching crates.io metadata broadly.
