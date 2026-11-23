# Issue #79(d) — missing trailing `[C]: in ?` traceback frame

Branch: `fix-79d-traceback` (off `main` @ v0.0.21).

## 1. Defect — confirmed against the oracle

Every uncaught-error traceback printed by the lua-rs CLI is missing the
trailing `[C]: in ?` frame that the reference emits. The frame is the base C
function that the standalone interpreter runs the whole program *beneath*
(`pmain`, pcall'd by `main`). lua-rs runs its `pmain` equivalent
(`interp::run`) as a plain Rust function with no CallInfo, so the call stack
has the main chunk sitting directly on `base_ci`, and `get_stack`/`traceback`
stop at `base_ci` (`is_base_ci`, `crates/lua-vm/src/debug.rs:477`) one frame
too early.

The builder is already correct: `auxlib::traceback`
(`crates/lua-stdlib/src/auxlib.rs:261`) renders a C frame (what==`C`,
empty namewhat) as `[C]: in ?` via `push_func_name`
(`auxlib.rs:201`, the final `else` arm). It would emit the frame if one
existed. The fix is purely to make a base C CallInfo exist.

### Captured reference stderr (uncaught `error`)

Script `/tmp/x.lua` = `inner()` calling `error("boom")` at line 2.

`(a) file` — lua5.4.7 / lua5.3.6:
```
lua: /tmp/x.lua:2: boom
stack traceback:
	[C]: in function 'error'
	/tmp/x.lua:2: in local 'inner'
	/tmp/x.lua:4: in main chunk
	[C]: in ?
```
lua5.5.0 differs only in the top frame: `[C]: in global 'error'`
(a separate pre-existing namewhat divergence, NOT part of #79d).

`(b) -e 'error("boom")'` — all versions:
```
lua: (command line):1: boom
stack traceback:
	[C]: in function 'error'      (5.5.0: in global 'error')
	(command line):1: in main chunk
	[C]: in ?
```

`(c) piped stdin` (`echo 'error("boom")' | lua`) — chunk name `stdin`:
```
lua: stdin:1: boom
stack traceback:
	[C]: in function 'error'      (5.5.0: in global 'error')
	stdin:1: in main chunk
	[C]: in ?
```

`(d) interactive REPL` (`printf 'error("boom")\n' | lua -i`) — identical to
stdin, including the trailing `[C]: in ?`, with a blank line on exit:
```
Lua 5.4.7  Copyright ...
stdin:1: boom
stack traceback:
	[C]: in function 'error'      (5.5.0: in global 'error')
	stdin:1: in main chunk
	[C]: in ?
```

In every entry point `[C]: in ?` is the LAST (deepest) frame; the frame above
it is `... in main chunk`. lua-rs today reproduces every line EXCEPT the
trailing `[C]: in ?`, for all four entry points and all of 5.3/5.4/5.5.

### Also affected: in-script `debug.traceback()`

`print(debug.traceback())` from a running script — reference shows
`... in main chunk` then `[C]: in ?`; lua-rs shows only `... in main chunk`.
Same for a traceback taken after a `pcall` boundary. This is the same root
cause and is fixed by the same change. Important: the missing frame is the
DEEPEST one, so adding it APPENDS at the bottom and does NOT shift the level
numbering of any existing frame — `debug.traceback`'s default `level=1` still
resolves to the same caller frame.

## 2. Chosen mechanism — faithful `pmain` C-closure pcall

lua.c (`reference/lua-5.4.7/src/lua.c:671`): `main` does
`lua_pushcfunction(L, &pmain); lua_pushinteger(argc); lua_pushlightuserdata(argv);
lua_pcall(L, 2, 1, 0)`. `pmain` (`lua.c:626`) does ALL orchestration —
`collectargs`, `openlibs`, `createargtable`, `handle_luainit`, `runargs`,
`handle_script`, `doREPL`/`dofile` — so the script/REPL chunk runs beneath
`pmain`'s C CallInfo, and an uncaught error walks `... main chunk -> pmain (C)
-> base_ci`, rendering pmain as `[C]: in ?`.

**Decision: mirror lua.c — push a real C closure and `pcall_k` it. Do NOT
hand-synthesize a fake base CallInfo.** Synthesizing a CallInfo by hand would
duplicate the C-frame setup that `do_::call` already does on the pcall path,
is fragile against the `callstatus`/`func`/`top` invariants that `get_stack`,
`get_info`, and `is_base_ci` depend on, and would be a second source of truth
for "what a C frame looks like." The pcall path already produces a correct,
walkable C CallInfo; reuse it.

### Threading argv / preload into the closure

A lua-rs C closure is `fn(&mut LuaState) -> Result<usize, LuaError>`
(`api::push_cclosure`, `crates/lua-vm/src/api.rs:1097`) — it cannot capture
the Rust `argv: &[Vec<u8>]` / `preload: fn(...)` arguments of `run`. lua.c
passes them as `lua_pushinteger`/`lua_pushlightuserdata`. lua-rs HAS
`api::push_light_userdata` (`api.rs:1164`) and can read it back
(`to_userdata`), but round-tripping a `&[Vec<u8>]` pointer is unsafe and the
lua-cli crate is currently `unsafe_blocks: 0`.

**Chosen threading: state-side scratch fields, matching the existing hook
pattern.** `main.rs` already installs ~20 embedder callbacks onto
`state.global_mut()` (`parser_hook`, `file_loader_hook`, … `main.rs:864-882`).
Add two transient fields to `GlobalState` (`crates/lua-vm/src/state.rs:989`):
- `cli_argv: Option<Vec<Vec<u8>>>`
- `cli_preload: Option<fn(&mut LuaState) -> Result<(), LuaError>>`

`run` moves `argv`/`preload` into these fields, pushes a zero-arg `pmain` C
closure, and `pcall_k`s it with `errfunc = 0` (the OUTER pcall installs NO
message handler — exactly like lua.c; the traceback is produced by `docall`'s
INNER msghandler). `pmain` takes them back out of the global at entry. This
keeps the CLI free of unsafe and reuses the established "embedder data lives
on GlobalState" convention. (Lightuserdata is the faithful-to-the-letter
alternative; rejected only to preserve `unsafe_blocks: 0`. If a reviewer
prefers byte-for-byte fidelity, lightuserdata is the fallback — behavior is
identical either way since the frame, not the argument-passing, is what #79d
is about.)

### Exact edit plan

1. `crates/lua-vm/src/state.rs` (`GlobalState`, ~line 989 + initializer
   ~line 4520): add `cli_argv: Option<Vec<Vec<u8>>>` and
   `cli_preload: Option<fn(&mut LuaState) -> Result<(), LuaError>>`, both
   initialized `None`. (Type of `cli_preload` matches `run`'s `preload`
   param.)

2. `crates/lua-cli/src/interp.rs`:
   - Rename the current `run` body into a free fn
     `fn pmain(state: &mut LuaState) -> Result<usize, LuaError>` that:
     - pulls `argv` and `preload` out of `state.global_mut().cli_argv.take()`
       / `cli_preload.take()`,
     - runs the EXACT existing orchestration body (collectargs → print_usage
       / -v / -E → script-dir LUA_PATH → open_libs → preload → sandbox →
       createargtable → handle_luainit → runargs → handle_script → REPL/stdin
       dofile → `run_close_finalizers`),
     - returns `Ok(1)` (mirrors lua.c `lua_pushboolean(L,1); return 1`); on a
       non-fatal orchestration error it should still return `Ok` after the
       existing `cli.report(...)` calls so the OUTER pcall is not the thing
       that prints. The process exit code is computed by the wrapper from the
       same `script>0 && !handle_script` / `runargs` booleans that today make
       `run` return 1 — capture that into a local and stash the intended exit
       code (e.g. push a boolean/integer result the wrapper reads, mirroring
       lua.c's `result = lua_toboolean(L,-1)`).
   - Rewrite `pub fn run(state, argv, preload) -> i32` to:
     - move `argv.to_vec()` and `preload` into the new GlobalState fields,
     - `api::push_cclosure(state, pmain, 0)`,
     - `api::pcall_k(state, 0, 1, 0, 0, None)`,
     - on `Err(e)` from the OUTER pcall (should be rare — only a non-Lua
       internal failure, since orchestration errors are already `report`ed
       inside `pmain`): `cli.report(Err(e))` and return 1, matching lua.c's
       `report(L, status)` after the outer pcall,
     - read the result the wrapper left and return the exit code.
   - `docall`, `dostring`, `dofile`, `handle_script`, `runargs`,
     `msghandler`, `traceback` — UNCHANGED. The whole point is that they now
     run with `pmain`'s CallInfo beneath them.

3. `crates/lua-cli/src/repl.rs`: UNCHANGED. `do_repl` is called from inside
   `pmain` now, so its `cli.docall` runs beneath the pmain C frame and the
   REPL traceback gains `[C]: in ?` for free (matches captured `(d)`).

4. Update the PORT STATUS trailer in `interp.rs` to note the pmain-pcall
   restructure and that the outer pcall uses no message handler.

No change to `auxlib.rs` (traceback/push_func_name), `debug.rs`
(get_stack/get_info), or `is_base_ci`. The stack walker is correct.

## 3. CLI-level oracle test design

The `[C]: in ?` frame appears only in the CLI traceback path, NOT in the
in-process `load`+`pcall` wrapper used by
`crates/lua-rs-runtime/tests/multiversion_oracle.rs` (no pmain there). So this
needs a SPAWN-THE-BINARY test.

Add `crates/lua-cli/tests/traceback_oracle.rs` (new `tests/` dir for lua-cli):
- For each version in `["5.3", "5.4", "5.5"]`:
  - Write a temp `.lua` (unique name: pid + counter) that raises an uncaught
    `error("boom")` from a nested local fn.
  - Spawn `env!("CARGO_BIN_EXE_lua-rs")` with `LUA_RS_VERSION=<ver>` on that
    file; capture stderr + exit code.
  - Normalize: replace the absolute script path and any `0x…` addresses the
    way `specs/oracle/diff_one.sh` does.
  - Assert: stderr CONTAINS `stack traceback:` and ENDS (last non-empty line)
    with `\t[C]: in ?`, and the line directly above it is `… in main chunk`.
  - Also assert exit code 1 for the `-e` and file cases.
- Cover all three entry points by spawning: (i) the file, (ii) `-e
  'error("boom")'`, (iii) piped stdin (`echo 'error("boom")' | lua-rs`). The
  REPL (`-i`) case is optional/lower-value to automate (rustyline + tty); the
  stdin case already exercises the same docall path.
- Optionally compare against `/tmp/lua-refs/bin/lua<ver>` when present (gate
  on the binary existing, like the existing oracle scripts) for a true diff;
  otherwise assert the literal expected tail, which is stable across versions
  for the `error`-less-frame question.

`multiversion_oracle.rs` MUST stay green and is expected to be unaffected (it
never sees the CLI pmain frame).

## 4. Regression-risk checklist (re-verify before claiming done)

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace --features lua-rs-runtime/derive`
- [ ] `specs/oracle/check.sh 5.4` (MUST NOT regress), then `5.3`, `5.5`
- [ ] New `crates/lua-cli/tests/traceback_oracle.rs` green for 5.3/5.4/5.5.
- [ ] `multiversion_oracle.rs` green (unchanged behavior).
- [ ] **Level-numbering**: confirm the new frame APPENDS at the bottom only.
      Re-check `debug.traceback()` default level (1) still names the caller,
      not a frame off-by-one — run `print(debug.traceback())` from a script
      and from inside a function and diff vs reference (already confirmed the
      target output: bottom gains `[C]: in ?`, nothing else moves).
- [ ] `error()` location prefix (`file:line:`) unchanged — the message-prefix
      logic counts up from the erroring frame, unaffected by a deeper frame.
- [ ] `xpcall` / `pcall` handlers: a handler invoked at level 1 must still see
      the same frame it did before (the new frame is below the pcall boundary,
      so handler-relative levels are unchanged). Spot-check an `xpcall(f,
      debug.traceback)` script vs reference.
- [ ] Official suites via the CLI on real files: `math.lua` must ADVANCE (its
      sole blocker was this missing frame); `errors.lua` and `calls.lua` must
      NOT regress — compare FIRST divergence line vs the matching
      `/tmp/lua-refs/bin/lua<ver>`.
- [ ] Exit codes preserved: file/`-e` uncaught error → exit 1; piped stdin
      error → exit 0 (matches captured reference). Verify the wrapper's
      result-reading reproduces today's `run` return values exactly.
- [ ] `os.exit` path still works (pre-existing TODO; ensure the pmain
      restructure doesn't change how `LuaExit`/panic unwinds through the new
      outer pcall — the panic-based exit must still escape `pcall_k`).
- [ ] Not in scope, do NOT "fix": 5.5.0 `in global 'error'` vs lua-rs `in
      function 'error'` top-frame namewhat — separate issue.
