//! Regression repro for #189: a Lua error raised uncaught through `lua.scope`
//! must keep its error value rooted until consumed. The raised value (e.g. the
//! `'boom'` short string) lives in the weak intern cache; once pcall pops it off
//! the stack it is held only by the Rust-side `LuaError`, which the collector
//! does not trace — so a collection in that window sweeps it (use-after-sweep,
//! caught by `LUA_RS_GC_QUARANTINE`).
//!
//! `LUA_RS_GC_STRESS=1` forces a collection at every allocation checkpoint,
//! making the otherwise GC-timing-dependent bug deterministic without the
//! heap-churning prelude the issue describes. Run with:
//!   LUA_RS_GC_STRESS=1 LUA_RS_GC_QUARANTINE=1 cargo test -p omnilua \
//!     --test scope_error_rooting

use omnilua::{Lua, LuaError, LuaVersion, Value, Variadic};

#[test]
fn error_raised_through_scope_keeps_its_value_rooted() {
    let lua = Lua::new_versioned(LuaVersion::V51);

    let result: omnilua::Result<Value> = lua.scope(|scope| {
        let host = scope
            .create_function_mut(&lua, |_, _: Variadic<Value>| Ok::<Value, _>(Value::Nil))?;
        lua.globals().set("host", &host)?;
        lua.load("error('boom')").set_name("user_script").eval::<Value>()
    });

    let err = result.expect_err("error('boom') must surface as Err, not Ok");

    // At this point `'boom'` is held ONLY by the Rust-side `err` (a raw,
    // un-traced GcRef) — its stack slot was popped by pcall's caller. Force a
    // full collection while nothing else references it. If the error value is
    // not rooted, this sweeps it; the deref below then hits the use-after-sweep
    // guard (under quarantine) or reads a freed string (without it).
    lua.load("collectgarbage('collect')")
        .exec()
        .expect("collectgarbage");

    let msg = err.message_lossy();
    assert!(
        msg.contains("boom"),
        "error message lost its value (swept?): {msg:?}"
    );
}

/// #189, non-string payload: `error({code=403})` must survive a collection while
/// held only by the Rust-side `Error`, and must remain a real Lua table — proof
/// that the fix *roots* the value rather than stringifying it. A stringified fix
/// would still pass the `'boom'` case above but would lose the table here.
#[test]
fn table_error_raised_through_scope_survives_collection_as_a_table() {
    let lua = Lua::new_versioned(LuaVersion::V54);

    let result: omnilua::Result<Value> = lua.scope(|scope| {
        let host = scope
            .create_function_mut(&lua, |_, _: Variadic<Value>| Ok::<Value, _>(Value::Nil))?;
        lua.globals().set("host", &host)?;
        lua.load("error({code = 403})")
            .set_name("user_script")
            .eval::<Value>()
    });

    let err = result.expect_err("error({code=403}) must surface as Err");

    lua.load("collectgarbage('collect')")
        .exec()
        .expect("collectgarbage");

    // The payload must still be a live, collectable Lua value (a table), not a
    // swept handle and not a stringified message. A swept value trips the
    // use-after-sweep guard under quarantine; a stringified fix would have made
    // this a `Runtime(Str)` instead of a table.
    match err.as_lua_error() {
        LuaError::Runtime(value) => assert!(
            value.is_collectable() && value.type_name() == "table",
            "error payload was not preserved as a table: {}",
            value.type_name()
        ),
        other => panic!("expected Runtime(table) payload, got {other:?}"),
    }

    // The same pattern caught *inside* Lua via pcall must still hand back the
    // real table — the VM re-pushes `LuaError::into_value()`, so a value that was
    // swept or stringified would break `pcall` semantics, not just the message.
    let (kind, code): (String, i64) = lua
        .load(
            "local ok, e = pcall(function() error({code = 403}) end)\n\
             assert(not ok)\n\
             return type(e), e.code",
        )
        .eval()
        .expect("pcall of a table error must return the table");
    assert_eq!(kind, "table");
    assert_eq!(code, 403);
}
