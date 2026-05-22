# {{PORT_NAME}} Test-Fixer Packet

You are running as the `test-fixer` role from `{{AGENTS_DIR}}/test-fixer.md`.
Fix the Rust implementation so the failing official Lua test passes. The PUC-Rio
Lua 5.4.7 test suite is the oracle — never edit a test file in
`reference/lua-c/testes/`, never edit the reference C sources in
`reference/lua-c/` or `reference/lua-5.4.7/`, and never edit harness scripts
under `harness/` unless this packet explicitly lists one of those paths as a
target.

## Failing Evidence

Latest oracle evidence blob:

`{{LATEST_ORACLE_BLOB}}`

Current failing fixtures or runner rows:

{{FAILING_FIXTURES_SUMMARY}}

Packet-specific instruction:

{{PACKET_NOTE}}

Declared target files:

{{PACKET_TARGETS}}

Reference/source ranges:

{{SOURCE_RANGES}}

Affected capabilities and owners:

{{AFFECTED_CAPABILITIES}}

## Required Process

1. Read the latest evidence blob (`{{LATEST_ORACLE_BLOB}}`) — it contains the
   stdout/stderr from the failing run.
2. Reproduce the failure with the smallest oracle command available — usually
   `./harness/run_official_test.sh reference/lua-c/testes/<name>.lua` for a
   single test. Inspect `harness/impl/official/<name>.out` and the line cited
   by the assertion.
3. Read the implicated Rust `.rs` files in `crates/` and the corresponding
   reference C in `reference/lua-c/` (e.g. `ldebug.c`, `lgc.c`, `ldo.c`).
   Trace from the failing test line back to the impl path that diverges.
4. Make the smallest implementation change that flips this one assertion.
   Do NOT refactor a whole subsystem to satisfy one assertion. Do NOT generalize
   beyond what the packet note instructs.
5. Re-run the focused oracle. If green, re-run the smoke set to confirm no
   regression: `for t in strings closure tracegc big sort math; do
   ./harness/run_one_test.sh reference/lua-c/testes/$t.lua; done`. Run the GC
   canaries (`./harness/canaries/gc/run_gc_canaries.sh`) if you touched any
   GC code.
6. Stop the moment the focused oracle passes and the smoke set is still green.
   Do not chase other failures — they are their own packets.

## Hard Rules (project: lua-rs-port)

- **Never edit a test.** Tests in `reference/lua-c/testes/` are the oracle.
  If you believe a test is wrong, leave `TODO(port): test <name> appears to
  test impl-defined behavior` in the impl and stop.
- **Fix the impl, never the symptom.** If a test asserts `42` and our impl
  returns `41`, do NOT patch the output formatter — find why the arithmetic
  is off.
- **No inline `//` comments.** Doc strings only. (Global CLAUDE.md.)
- **No fallback patterns** (`x || y || z`). Single source of truth — if data
  may be missing, fix the data path.
- **No new `unsafe`** outside `lua-gc`, `lua-coro` (and `lua-cli` with a
  4-block budget for FFI). The workspace default is `unsafe_code = "forbid"`.
- **No `String` / `&str` for Lua data.** Use `&[u8]` / `Vec<u8>` / `LuaString`.
- **No `tokio` / `rayon` / `std::process` / `std::fs` / `std::net`** outside
  `lua-cli`. The hook pattern (`PopenHook`, `FileOpenHook`, `OsExecuteHook`
  in `state.rs`) is how stdlib reaches the OS.
- **Logic changes update the `PORT STATUS` trailer** of the file you change.
- **No `--no-verify` on commits.** The Stop hook auto-commits and gates on
  the smoke set.
- **If the fix requires changing a cross-crate API or dependency edge**, leave
  a `TODO(architect): ...` marker and stop.

## Output Contract

When done, leave the workspace in a state where:

- `cargo build -p lua-cli -q` is clean;
- the focused oracle passes (`./harness/run_official_test.sh
  reference/lua-c/testes/<name>.lua` exits 0);
- the smoke set is still green;
- if you touched GC code, the GC canaries are still green;
- any touched `.rs` file has a refreshed `PORT STATUS` trailer.

Project root: `{{PROJECT_ROOT}}`
Prompt hash: `{{PROMPT_HASH}}`
Evidence path: `{{EVIDENCE_PATH}}`
