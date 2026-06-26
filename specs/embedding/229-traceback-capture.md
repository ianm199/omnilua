# Spec #229 — Stack-traceback capture in the embedding `Error`

Status: **IMPLEMENTED** (commit 9f11f008). The pcall_k error-contract open
question was resolved by reading `protected_call_raw` (state.rs:3589): on error it
cleans the stack to `func` and the handler sits below `func`, so it survives the
unwind and the `rotate`+`set_top` removal is valid on both paths. 7 tests green;
capture-off byte-identical (oracle 184 + CLI traceback oracle 16). Originally
design, **revised after Codex adversarial review (VERDICT: REVISE — all
findings adopted)**. The readable-message half shipped in 0.3.5. This is the
traceback **capture** half. Reviewer focus held: **capture must never change the
error message** for code that opts in OR out.

## Problem

A host catching an `Error` from `Chunk::exec/eval` or `Function::call` gets only the
immediate error value — no `debug.traceback()` stack.

## Substrate (verified)

- `pcall_k(state, nargs, nresults, errfunc, ctx, k)` (`api.rs:2016`): `errfunc` =
  stack index of a message handler run **before unwind** (`do_.rs:133`
  `run_message_handler`).
- `auxlib::traceback(state, other, msg, level)` (`auxlib.rs:298`) builds the
  traceback string on the stack — **returns `Result`** (it allocates).
- CLI pattern (`interp.rs:91` handler, `:391` insert-below-func + remove-after).
- `Chunk::eval` (lib.rs:1659/1674) and `Function::call` (lib.rs:2003/2023) compute
  result counts from `saved_top` and use `errfunc=0` today.
- The runtime passes Rust state into C closures via upvalues
  (`create_function`, lib.rs:1153/3401) — re-entrancy-safe per call.

## Design — per-call upvalue slot, best-effort, message preserved (opt-in)

Capture must run pre-unwind (handler) and must not alter the error value. Codex
showed a single `GlobalState` slot cross-wires under nesting and `<close>`. So the
capture slot is **per protected call**, carried in the handler closure's upvalue.

### API (bytes, per the repo rule)

```rust
impl Lua {
    pub fn set_capture_tracebacks(&self, on: bool);  // default OFF (zero cost)
    pub fn captures_tracebacks(&self) -> bool;
}
impl Error {
    /// Captured traceback bytes (Lua bytes / source names are not guaranteed
    /// UTF-8). The error *message* is identical whether capture is on or off.
    pub fn traceback_bytes(&self) -> Option<&[u8]>;
    /// Lossy-UTF8 convenience.
    pub fn traceback_lossy(&self) -> Option<String>;
}
```
`Error` gains `traceback: Option<Vec<u8>>` (not `String`).

### Mechanism (addresses every Codex finding)

1. **Per-call slot.** Each protected call that opts in creates a fresh
   `Rc<RefCell<Option<Vec<u8>>>>`, builds a one-shot C closure handler capturing it
   (same upvalue mechanism as `create_function`), and installs it as `errfunc`.
   Nested calls each own their slot — no shared global state, no generation IDs.
2. **Best-effort, message-preserving handler.** The handler:
   - reads the error value at arg 1 as **raw bytes only** — if it's a `Str`, take
     its bytes; otherwise use a fixed type-name placeholder. **No `__tostring`, no
     metamethods, no user code** (Codex finding 6);
   - calls `auxlib::traceback`; on **any** error from it, the handler **restores the
     stack to arg 1 and returns it unchanged** — never `?`, never `ErrErr`
     (finding 1). On success, it moves the traceback bytes into the per-call slot,
     **pops the traceback string**, and returns arg 1 **unchanged** (message stays
     pristine).
3. **Exact stack choreography** (finding 7), mirroring the CLI: push the handler,
   `insert` it just below the function+args (so it sits at a fixed `errfunc` index),
   `pcall_k(..., errfunc=that_index, ...)`, then `remove` it on **both** success and
   error paths **before** the existing `saved_top`-based result counting runs — so
   `MULTRET` never counts/returns the handler.
4. **Attach at the specific call site, not `capture_error_in_state`** (finding 3).
   Only `Chunk::eval/exec` and `Function::call` (the paths that installed a handler)
   read their per-call slot in the `Err` arm and set `Error.traceback`.
   `capture_error_in_state` is unchanged and never touches tracebacks, so syntax/load
   errors (handler-skipped, do_.rs:1551) can't pick up a stale trace.
5. **Off by default**: when capture is off, `errfunc=0` exactly as today — byte-for-byte
   no behavior change.

### Known limitation (finding 2 — `<close>` replacement)

The handler runs before `close_protected`, which can replace the final error (a
`<close>` metamethod erroring after the original `error(...)`). In that case the
captured traceback reflects the **originally raised** error, not the replacement.
v1 documents this: `traceback_bytes()` is best-effort and corresponds to the error
at the point the message handler ran. (Detecting replacement reliably needs a VM
hook we don't add in v1.) A test pins this characterized behavior.

## Test plan (oracle-backed where possible)

`crates/lua-rs-runtime/tests/traceback_capture.rs`:
- **capture off (default)**: `traceback_bytes()` is `None`; message bytes
  byte-identical to today (the anti-pollution guard).
- **capture on**: Rust→Lua→Rust nested error → traceback names frames; message bytes
  still clean (no `stack traceback` substring in `message_lossy`).
- **nested Rust callback re-entry**: inner and outer errors get their **own**
  tracebacks (per-call slot isolation) — the regression the global slot would cause.
- **syntax error after a runtime error**: the parse error has `traceback() == None`
  (no stale leak) — finding 3.
- **non-UTF8 error bytes / non-string error object with no `__tostring`**: handler
  produces a placeholder, never runs user code, message preserved — finding 6.
- **capture-failure best-effort**: simulate `auxlib::traceback` failure path → error
  message unchanged, `traceback()` is `None` — finding 1.
- **`<close>` replaces the error**: characterized per the documented limitation.

Oracle gate: `multiversion_oracle` byte-identical (pcalls inside Lua, untouched),
CLI `traceback_oracle` (16) green, full `cargo test -p omnilua` green.

## Open questions resolved by the review

- Home for the slot → **per-call upvalue**, not `GlobalState` (finding 4).
- String vs bytes → **bytes** (finding 5).
- Single consumer → **the call site**, not `capture_error_in_state` (finding 3).

## Implementation recipe (mapped; ready to execute)

Substrate confirmed. Concrete steps:

1. `Error` (lib.rs:156) gains `traceback: Option<Vec<u8>>`; add `traceback_bytes()`
   / `traceback_lossy()` and a private `with_traceback(self, Option<Vec<u8>>)`.
   `Error::from` sets `None`; `capture_error_in_state` does NOT touch it.
2. `LuaInner` gains `capture_tracebacks: Cell<bool>` (init false); `Lua::
   set_capture_tracebacks` / `captures_tracebacks`.
3. Handler via `create_registered_function` (it gives a raw `&mut LuaState`
   closure — no re-entrancy through `with_state`), capturing
   `Rc<RefCell<Option<Vec<u8>>>>`:
   ```
   let saved = api::get_top(state);
   if auxlib::traceback(state, None, None, 1).is_ok() {        // msg=None → NO metamethods
       if let Ok(Some(s)) = api::to_lua_string(state, -1) {    // the tb string itself, no mm
           *slot.borrow_mut() = Some(s.as_bytes().to_vec());
       }
   }
   let _ = api::set_top(state, saved);                          // best-effort, restore to arg 1
   Ok(1)                                                        // return arg 1 unchanged
   ```
4. Helper mirroring the CLI `docall` (interp.rs:385) exactly:
   ```
   let base = api::get_top(state) - nargs;
   state.push(handler_raw); state.insert(base)?;
   let r = api::pcall_k(state, nargs, nresults, base, 0, None);
   api::rotate(state, base, -1); let _ = api::set_top(state, -2);   // lua_remove(base)
   r
   ```
   After this the stack is identical to the no-handler case (results/error start at
   the function's original slot), so each site's existing `set_top_idx(saved_top)`
   cleanup and result counting work unchanged.
5. Sites: `eval` (lib.rs:1719), `Function::call` (lib.rs:2064), and `exec_state`
   (lib.rs:4123 — thread an `Option<RawLuaValue>` handler through it). When capture
   is on, create the handler+slot **before** the site's `with_state`, get
   `handler_raw = handler.root.raw()?`, pass it into the helper, and after the call
   attach `slot.take()` to the `Error` via `with_traceback`. When off, pass `None`
   → `errfunc=0`, byte-identical to today.

### OPEN QUESTION to resolve before/while implementing (the one real risk)

`pcall_k`'s **error-path stack contract**: when the message handler runs and the
call errors, exactly what does `pcall_k` leave on the stack (error object present
at `base`? already popped into the returned `Err`?). The `rotate(base,-1)+set_top(-2)`
handler-removal must be valid in that state without underflow. Verify against
`do_.rs` `run_message_handler` / the pcall error unwind, OR rely on the contained
blast radius (capture-off is byte-identical; capture-on is opt-in + the tests
below catch stack mismanagement). Tests MUST include nested Rust-callback errors
and the `<close>`-replacement case to exercise the unwind paths.
