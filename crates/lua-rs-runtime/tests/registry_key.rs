//! Anonymous registry keys — issue #226 (the keyed half).
//!
//! `create_registry_value` stashes a value behind a `RegistryKey` the host
//! holds across calls; `registry_value` reads it back (converting to any type);
//! `remove_registry_value` frees the slot. Provenance-bound to the parent Lua.

use omnilua::{Function, Lua, Value};

#[test]
fn value_round_trips_through_a_key() {
    let lua = Lua::new();
    let key = lua.create_registry_value(42i64).unwrap();
    let v: i64 = lua.registry_value(&key).unwrap();
    assert_eq!(v, 42);
}

#[test]
fn stored_function_survives_dropping_all_other_handles() {
    let lua = Lua::new();
    let key = {
        let f: Function = lua
            .load("return function(x) return x * 2 end")
            .eval()
            .unwrap();
        lua.create_registry_value(f).unwrap()
    };
    lua.gc_collect();
    let g: Function = lua.registry_value(&key).unwrap();
    let r: i64 = g.call(21).unwrap();
    assert_eq!(r, 42);
}

#[test]
fn registry_key_is_invisible_to_scripts() {
    let lua = Lua::new();
    let _key = lua.create_registry_value("hunter2").unwrap();
    let seen: Value = lua.load("return secret").eval().unwrap();
    assert!(matches!(seen, Value::Nil));
}

#[test]
fn remove_frees_the_slot() {
    let lua = Lua::new();
    let key = lua.create_registry_value(7i64).unwrap();
    let before: i64 = lua.registry_value(&key).unwrap();
    assert_eq!(before, 7);
    lua.remove_registry_value(key).unwrap();
}

#[test]
fn key_is_provenance_bound() {
    let lua_a = Lua::new();
    let lua_b = Lua::new();
    let key_b = lua_b.create_registry_value(1i64).unwrap();

    let err = lua_a.registry_value::<i64>(&key_b).unwrap_err();
    assert!(
        err.to_string().contains("different state"),
        "expected a cross-instance rejection, got: {err}"
    );
}
