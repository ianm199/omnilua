//! Host bytecode dump/load — issue #228.
//!
//! `Function::dump` serializes a Lua function to a binary chunk; `Lua::load`
//! auto-detects binary input, so dump -> load round-trips.

use omnilua::{Function, Lua, LuaVersion};

#[test]
fn dump_then_load_round_trips() {
    let lua = Lua::new();
    let f: Function = lua
        .load("local x = ...; return x * 3")
        .into_function()
        .unwrap();

    let bytes = f.dump(false).unwrap();
    assert!(bytes.starts_with(b"\x1bLua"), "should be a binary chunk");

    let g: Function = lua.load(&bytes[..]).into_function().unwrap();
    let r: i64 = g.call(14).unwrap();
    assert_eq!(r, 42);
}

#[test]
fn stripped_dump_is_no_larger_and_still_loads() {
    let lua = Lua::new();
    let f: Function = lua
        .load("local a = 1; local b = 2; return a + b")
        .into_function()
        .unwrap();

    let full = f.dump(false).unwrap();
    let stripped = f.dump(true).unwrap();
    assert!(stripped.len() <= full.len());

    let g: Function = lua.load(&stripped[..]).into_function().unwrap();
    let n: i64 = g.call(()).unwrap();
    assert_eq!(n, 3);
}

#[test]
fn rust_function_cannot_be_dumped() {
    let lua = Lua::new();
    let f = lua.create_function(|_, ()| Ok(7i64)).unwrap();
    assert!(f.dump(false).is_err());
}

#[test]
fn cross_version_binary_chunk_is_rejected() {
    let v54 = Lua::new();
    let bytes = v54
        .load("return 1")
        .into_function()
        .unwrap()
        .dump(false)
        .unwrap();

    let v53 = Lua::new_versioned(LuaVersion::V53);
    assert!(
        v53.load(&bytes[..]).into_function().is_err(),
        "a 5.4 binary chunk must not load into a 5.3 instance"
    );
}
