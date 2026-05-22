# Stuck: `reference/lua-c/testes/files.lua`

**Status:** highest-leverage next agent target. The architectural escape hatch already exists in tree; the remaining work is mechanically finishing five stubs.

## Current failure

```
testing i/o
lua: pcall_k failed: Runtime: TODO(port): borrow split needed for f_seek
```

## What's actually in tree (correcting the earlier draft)

The earlier version of this doc said *"wrap the file handle in `RefCell`"*. That's **already done**:

- `crates/lua-stdlib/src/io_lib.rs:31` — `LSTREAM_REGISTRY: HashMap<usize, Rc<RefCell<LStream>>>` (side-table keyed by `GcRef::identity()`)
- `crates/lua-stdlib/src/io_lib.rs:404` — `get_lstream(state) -> Rc<RefCell<LStream>>`
- `crates/lua-stdlib/src/io_lib.rs:1096` — `io_write` already demonstrates the working pattern: **collect all stack arguments into owned `Vec<u8>` first, then borrow the file**

So the borrow split has been solved at the type level. What's left is replacing five TODO bodies with the same "collect-then-borrow" pattern that `io_write` already uses.

## The five remaining stubs

| Site | Function | Lua-facing |
|---|---|---|
| `io_lib.rs:1009` | `io_read` | `io.read(...)` |
| `io_lib.rs:1019` | `f_read` | `file:read(...)` |
| `io_lib.rs:1155` | `f_write` | `file:write(...)` (mirror of `io_write`) |
| `io_lib.rs:1166` | `f_seek` | `file:seek(...)` ← current blocker |
| `io_lib.rs:1223` | `f_flush` / `io_flush` | `file:flush()`, `io.flush()` |

All five fail with `TODO(port): borrow split needed for <name>`.

## Suggested prompt for the next Opus run

> Finish the remaining io_lib.rs borrow-split stubs using the existing
> `LSTREAM_REGISTRY` / `Rc<RefCell<LStream>>` pattern. Do not redesign
> userdata, do not touch loadfile/dofile, do not change the cherry-picked
> file_open_hook plumbing. Implement in this order — stop and ship whatever
> compiles even if you don't finish all five:
>
>   1. `f_seek` (io_lib.rs:1166) — current blocker for files.lua
>   2. `f_flush` and `io_flush` (io_lib.rs:1223 area)
>   3. `f_write` (io_lib.rs:1155) — mirror `io_write` at io_lib.rs:1096
>   4. `f_read` (io_lib.rs:1019)
>   5. `io_read` (io_lib.rs:1009)
>
> Pattern, copied from working `io_write`:
>   a. Parse and convert all stack args to owned data BEFORE borrowing the file.
>   b. `let p_rc = tofile(state)?;` then `p_rc.borrow_mut()` for the I/O.
>   c. Release the borrow (`drop(file)` or scope end) before pushing return values.
>
> Test as you go: `target/debug/lua-rs reference/lua-c/testes/files.lua` and
> watch the failure point move down the file.

This is the "real engineering but bounded" run. ~200 lines net, mechanical, ≤2 hours of Opus.

## What past agents have tried

Only one commit ever landed: `229c5b2 agent debug: reference/lua-c/testes/files.lua`. The most recent O2 opus run (transcript `mega-O2-D4-...`, ~550 KB) **did not touch f_seek** — it spent its budget implementing `load_filex` in `auxlib.rs` (loadfile / dofile) using `std::fs::read`. That fix is real and correct, but loadfile isn't on the path to files.lua's current failure. The agent solved the wrong problem because the test output didn't make the dependency chain obvious.

The prompt above avoids that miss by naming exactly which functions to fix.

## Files most touched

`crates/lua-stdlib/src/io_lib.rs` (the only file the prompt should need).
