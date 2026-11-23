//! Host-side table writes must run the GC write barrier.
//!
//! A key or value stored from Rust while a collection cycle is in progress
//! has to be marked like a store from Lua code would be; otherwise the sweep
//! frees it while the table still points at it.
//!
//! These tests fail fastest under
//!   LUA_RS_GC_STRESS=1 LUA_RS_GC_QUARANTINE=1 cargo test -p omnilua \
//!     --test host_raw_set_barrier

use omnilua::{Lua, Table, Value};

fn make_garbage(lua: &Lua) {
    lua.load("local t = {} for i = 1, 20000 do t[i] = 'garbage_' .. i end")
        .exec()
        .unwrap();
}

#[test]
fn raw_set_string_keys_survive_collection() {
    let lua = Lua::new();
    make_garbage(&lua);
    let table = lua.create_table().unwrap();
    for i in 0..512i64 {
        table.raw_set(format!("key_{i}"), i).unwrap();
    }
    lua.gc_collect();
    for i in 512..1024i64 {
        table.raw_set(format!("key_{i}"), i).unwrap();
    }
    lua.gc_collect();
    for i in 0..1024i64 {
        let v: i64 = table.raw_get(format!("key_{i}")).unwrap();
        assert_eq!(v, i);
    }
}

#[test]
fn raw_set_table_keys_survive_collection() {
    let lua = Lua::new();
    make_garbage(&lua);
    let map = lua.create_table().unwrap();
    for i in 0..256i64 {
        map.raw_set(lua.create_table().unwrap(), i).unwrap();
    }
    lua.gc_collect();
    for i in 256..512i64 {
        map.raw_set(lua.create_table().unwrap(), i).unwrap();
    }
    lua.gc_collect();
    let mut seen = 0;
    for (k, _) in map.raw_pairs().unwrap() {
        assert!(matches!(k, Value::Table(_)));
        seen += 1;
    }
    assert_eq!(seen, 512);
}

#[test]
fn raw_set_values_survive_collection() {
    let lua = Lua::new();
    make_garbage(&lua);
    let table = lua.create_table().unwrap();
    lua.gc_collect();
    for i in 1..=512i64 {
        let inner = lua.create_table().unwrap();
        inner.raw_set("n", i).unwrap();
        table.raw_set(i, inner).unwrap();
        table
            .raw_set(format!("s{i}"), format!("value_{i}"))
            .unwrap();
    }
    lua.gc_collect();
    for i in 1..=512i64 {
        let inner: Table = table.raw_get(i).unwrap();
        assert_eq!(inner.raw_get::<_, i64>("n").unwrap(), i);
        let s: String = table.raw_get(format!("s{i}")).unwrap();
        assert_eq!(s, format!("value_{i}"));
    }
}

#[test]
fn raw_set_sandbox_env_then_load() {
    let lua = Lua::new();
    make_garbage(&lua);
    let globals = lua.globals();
    let string_lib: Table = globals.get("string").unwrap();
    for name in ["dump", "rep"] {
        string_lib.raw_set(name, Value::Nil).unwrap();
    }
    lua.gc_collect();
    let require = lua
        .create_function(|_, name: String| Ok(format!("module:{name}")))
        .unwrap();
    for i in 0..256 {
        globals.raw_set(format!("host_global_{i}"), i).unwrap();
    }
    globals.raw_set("host_require", require).unwrap();
    lua.gc_collect();
    let out: String = lua
        .load("return host_require('a') .. tostring(host_global_255)")
        .eval()
        .unwrap();
    assert_eq!(out, "module:a255");
}
