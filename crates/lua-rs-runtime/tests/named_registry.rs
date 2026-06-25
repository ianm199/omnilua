//! Named registry — issue #226 (named half; the anonymous RegistryKey token is
//! deferred since omniLua handles already root themselves across calls).
//!
//! The registry is a value store scripts cannot reach, for host-owned values
//! that must outlive a single call.

use omnilua::{Function, Lua, Value};

#[test]
fn named_value_round_trips() {
    let lua = Lua::new();
    lua.set_named_registry_value("answer", 42i64).unwrap();
    let v: i64 = lua.named_registry_value("answer").unwrap();
    assert_eq!(v, 42);
}

#[test]
fn stored_function_is_callable_after_local_handle_drops() {
    let lua = Lua::new();
    {
        let f: Function = lua
            .load("return function(x) return x + 1 end")
            .eval()
            .unwrap();
        lua.set_named_registry_value("cb", f).unwrap();
    }
    let g: Function = lua.named_registry_value("cb").unwrap();
    let r: i64 = g.call(41).unwrap();
    assert_eq!(r, 42);
}

#[test]
fn registry_is_invisible_to_scripts() {
    let lua = Lua::new();
    lua.set_named_registry_value("secret", "hunter2").unwrap();
    let seen: Value = lua.load("return secret").eval().unwrap();
    assert!(matches!(seen, Value::Nil));
}

#[test]
fn unset_removes_value() {
    let lua = Lua::new();
    lua.set_named_registry_value("k", 1i64).unwrap();
    lua.unset_named_registry_value("k").unwrap();
    let v: Option<i64> = lua.named_registry_value("k").unwrap();
    assert_eq!(v, None);
}
