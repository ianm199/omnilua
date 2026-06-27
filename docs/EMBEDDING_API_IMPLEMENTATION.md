# Embedding API Implementation Status

`omnilua` (crate dir `crates/lua-rs-runtime`) exposes a Rust-native embedding API
shaped after `mlua`. Unlike `mlua` it is pure safe Rust: it builds for
`wasm32-unknown-unknown` and needs no C toolchain or `liblua`.

This document describes what exists now, verified against the code and its
integration tests rather than from prose. **Do not trust a hardcoded status here
over the source** — confirm any claim with the matching test file in
`crates/lua-rs-runtime/tests/` and the live backlog (`gh issue list`). The parity
push and the remaining-gap reconciliation live in
[EMBEDDING_PARITY_TIER_ATTACK_PLAN.md](EMBEDDING_PARITY_TIER_ATTACK_PLAN.md); the
design rationale lives in [docs/design/EMBEDDING_API.md](design/EMBEDDING_API.md).

## Public Surface

The primary API lives in `crates/lua-rs-runtime/src/lib.rs`.

- **Construct & run.** `Lua::new`; `Lua::new_versioned` / `Lua::version` for the
  5.1–5.5 selection; `lua.load(src).set_name(..).exec()` / `.eval()`;
  `Chunk::into_function()` to compile once and call many times.
- **Sandbox.** `Lua::sandboxed(SandboxConfig) -> (Lua, Sandbox)` and
  `Lua::install_sandbox` — uncatchable instruction budget + memory ceiling +
  capability stripping; `Sandbox::reset` refills the budget. (tests:
  `sandbox.rs`, `runtime_sandbox.rs`)
- **Values & tables.** `globals`, `create_table`, `create_string`; `Table::get`,
  `set`, `len`, `raw_pairs`, and the sequence helpers `push` / `insert` /
  `remove` / `pop` / `clear`. (tests: `table_helpers.rs`)
- **Functions & userdata.** `create_function`, `create_function_mut`,
  `create_userdata`, `create_userdata_with_uservalues`; `AnyUserData::borrow` /
  `borrow_mut` / `set_user_value` / `user_value`; the `UserData`,
  `UserDataMethods`, and `MetaMethod` traits. (tests: `uservalues.rs`)
- **Handles & identity.** Owned `Value`, `Table`, `Function`, `LuaString`,
  `AnyUserData`, `Thread`; `to_pointer` plus `PartialEq`/`Eq` (references by
  identity, strings by bytes). (tests: `handle_identity.rs`)
- **Registry.** Named: `set_named_registry_value` / `named_registry_value` /
  `unset_named_registry_value`. Keyed: `create_registry_value -> RegistryKey`,
  `registry_value`, `remove_registry_value`. (tests: `named_registry.rs`,
  `registry_key.rs`)
- **Host-driven coroutines.** `create_thread(Function) -> Thread`;
  `Thread::resume::<A, R>(args)`; `Thread::status() -> ThreadStatus`. (tests:
  `host_coroutine.rs`)
- **GC control.** `lua.gc() -> GcControl` with `collect` / `step` / `stop` /
  `restart` / `count` / `is_running` (version-divergent knobs error rather than
  lie); plus the shorthand `gc_collect()`. (tests: `gc_control.rs`)
- **Errors & tracebacks.** `Error` / `LuaError` with `Display`, `to_status`,
  `into_value`, `message_lossy`; `set_capture_tracebacks(true)` then
  `Error::traceback_bytes` / `traceback_lossy` (off by default → byte-identical
  errors). (tests: `traceback_capture.rs`, `error_display.rs`)
- **Bytecode.** `Function::dump(strip) -> Vec<u8>`; `load` auto-detects a binary
  chunk and enforces the version header. (tests: `bytecode.rs`, `dump_kit.rs`)
- **Multi-version bridging.** `set_lossy_int_policy` / `lossy_int_policy`
  (`LossyIntPolicy::{WidenLossy, ErrorOnInexact}`) for the host-int → float-only
  number-model seam; cross-instance `Lua::marshal_from(&src, &value)` copies a
  value between two engines (cycle-safe tables, number-model translation,
  function call-proxies). (tests: `number_seam.rs`, `cross_version_bridge.rs`)
- **Scope (non-`'static` borrow).** `Lua::scope`, `Scope::create_userdata_ref_mut`
  — lend Lua a `&mut T` for one call; a handle that escapes errors cleanly
  instead of dangling. (tests: `scope_delegate.rs`, `scope_error_rooting.rs`,
  `scope_world_smoke.rs`)
- **Conversions.** `IntoLua`, `FromLua`, `IntoLuaMulti`, `FromLuaMulti`;
  primitives, bytes/strings, `Option<T>`, `Vec<T>`, `HashMap<K, V>`, small
  tuples, and `Variadic<T>`.

## Example

```rust
use omnilua::{Lua, Result, UserData, UserDataMethods};

#[derive(Default)]
struct Counter {
    value: i64,
}

impl UserData for Counter {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("inc", |_, this, by: i64| {
            this.value += by;
            Ok(this.value)
        });
        methods.add_method("get", |_, this, ()| Ok(this.value));
    }
}

fn main() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    let shout = lua.create_function(|_, text: String| Ok(text.to_uppercase()))?;
    globals.set("shout", shout)?;
    globals.set("counter", Counter::default())?;

    let result: (String, i64) = lua
        .load(r#"
            counter:inc(2)
            counter:inc(3)
            return shout("lua-rs"), counter:get()
        "#)
        .eval()?;

    assert_eq!(result, ("LUA-RS".to_string(), 5));
    Ok(())
}
```

## Implementation Model

### Rooted Handles

External Rust handles are anchored by `ExternalRootSet` on `GlobalState`. The VM
traces that root set during collection, so Rust-owned `Table`, `Function`,
`LuaString`, `AnyUserData`, and collectable `Value` variants keep their Lua
referents alive.

Clone semantics use the simple model from the implementation spec: each clone
creates a fresh external root key. `Drop` unroots that key exactly once. Stale
keys include a generation, so a dropped handle cannot accidentally observe a
later value after slot reuse.

### Re-entrant `Lua`

`Lua` owns the VM state behind a boundary `RefCell`, but bytecode execution still
runs with direct `&mut LuaState`. Re-entry is handled at the Rust/Lua boundary:
while the VM is inside a callback, the active state pointer is available to the
callback-side `Lua` handle. This keeps borrow checks and dynamic dispatch out of
the opcode loop.

There is one audited unsafe bridge in `Lua::active_state_mut`. It is local,
documented with a `SAFETY` comment, and covered by callback/re-entry tests.

### Captured Rust Callbacks

Rust callbacks are not stored permanently in `GlobalState.c_functions`. Instead:

- one shared bare C trampoline is registered in the existing C-function table;
- each captured Rust callback is stored in a collectable userdata payload;
- a Lua C closure points at the shared trampoline and carries that userdata as
  an upvalue;
- the trampoline recovers the payload from the current call frame and invokes
  the Rust closure.

That shape lets dropped `Function` handles release captured Rust state after GC,
including captured Lua handles.

### Userdata and Metamethods

`Lua::create_userdata` stores Rust values inside `Rc<dyn Any>` payloads with
runtime borrow tracking. `AnyUserData::borrow` and `borrow_mut` expose typed
borrows and report Lua runtime errors for borrow conflicts.
`create_userdata_with_uservalues` pre-allocates uservalue slots; `set_user_value`
drives the barriered slot setter so the write is visible to the generational
collector.

`UserDataMethods` builds a metatable backed by the same closure machinery as
`create_function`. Supported metamethods are the common set: `__index`,
`__newindex`, the arithmetic and comparison operators, `__call`, `__tostring`,
`__len`, `__concat`, and `__pairs` (see Known Limits for the ones not yet
exposed).

### GC Allocation Boundaries

Embedding-side allocations that create GC-managed objects run under a
`lua_gc::HeapGuard`. This includes table/string creation, callback closure
payloads, userdata allocation, and parser-hook closure allocation.

If a Rust handle is dropped during GC sweep, mutating `GlobalState.external_roots`
would conflict with the collector's immutable borrow. Runtime handle drops use a
best-effort unroot path and queue pending external unroots until the next safe
embedding boundary.

## Verification

The embedding surface is covered by the integration tests in
`crates/lua-rs-runtime/tests/` (one file per capability, named above). Run them —
do not trust a count written here:

```bash
cargo test -p omnilua                  # all embedding integration tests
cargo test -p omnilua --test host_coroutine --test registry_key --test gc_control
./harness/canaries/gc/run_canaries.sh  # uservalue barriers + accounting, both GC modes
```

The GC-sensitive additions (uservalues, external roots) are gated on the GC
canaries in incremental and generational modes, not just unit tests.

## Performance

Embedding cost is kept at the Rust/Lua boundary, out of the opcode dispatch loop.
The tracked lua-rs / reference-C ratio (wall + RSS) lives in the benchmark
dashboard, not in this doc — see `harness/bench/history/index.html` and
`docs/MEASUREMENT_PROTOCOL.md`. Do not cite a frozen ratio from here.

## Known Limits

This is not a full `mlua` clone. The remaining gaps vs `mlua`, verified against
the current surface:

- **No async API.** No `create_async_function` / `Future` integration.
- **No serde integration.** No Lua ⇄ serde value bridge.
- **Table iteration materializes a `Vec`** (`raw_pairs`); there is no lazy /
  streaming `pairs()` iterator yet (issue #232). `__pairs` is honored by the
  value machinery, but the host helper does not stream.
- **Metamethod coverage is the common set.** Bitwise (`__band`/`__bor`/`__bxor`/
  `__bnot`/`__shl`/`__shr`), `__idiv`, `__close`, and `__name` are not yet
  exposed through `MetaMethod`.
- **Multi-version seam is partial.** Value marshaling between engines works
  (`marshal_from` + `LossyIntPolicy`), but the formal `enum Engine` / `Backend`
  trait / `Unsupported` divergence registry from
  `specs/WEBLUA_MULTIVERSION_API_SPEC.md` is deferred (issue #234).
- **Interpreter only.** No LuaJIT-class speed; not Luau.
- **Preview maturity.** The API is shaped after `mlua` but is not
  source-compatible, and has a far smaller user base / less battle-testing.

### Where it exceeds `mlua`

- Builds for `wasm32-unknown-unknown` with no C toolchain (mlua cannot).
- 5.1–5.5 from one core, runtime-selected, with cross-instance `marshal_from`
  (mlua is one version per build, no value transfer between states).
- Uncatchable sandbox (budget + memory ceiling + capability stripping) for
  standard Lua; mlua's hardened sandbox is Luau-only.
- `scope` borrow model for non-`'static` `&mut T`.
- Result-based error discipline — no user-facing panic in the surface.

## Future Opportunities

1. **Async + serde** — the two largest remaining `mlua`-parity features.
2. **Lazy table iteration** (#232) and **extended metamethods** (bitwise,
   `__idiv`, `__close`, `__name`).
3. **The full multi-version seam** (#234) — `enum Engine` / `Backend` /
   `Unsupported`. This is the differentiator, not just parity: it turns the
   working `marshal_from` bridge into a first-class, machine-checkable API.
4. **Stabilization** — rustdoc examples, a standalone embedding example crate,
   and migration notes for projects coming from `mlua` (e.g. a `bevy_scriptum`
   backend).
5. **Soundness hardening** — Miri coverage, randomized create/clone/drop/GC
   stress, callback-GC torture tests, external-root leak checks.
