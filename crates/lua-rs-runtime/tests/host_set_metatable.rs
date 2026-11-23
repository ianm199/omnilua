//! `Table::set_metatable` from the host must behave like Lua's
//! `setmetatable`: `__mode` makes the table weak, `__gc` registers a
//! finalizer, and a metatable from another `Lua` is rejected.

use omnilua::{Lua, Table};

#[test]
fn weak_keys_from_host_are_cleared() {
    let lua = Lua::new();
    let weak = lua.create_table().unwrap();
    let mt = lua.create_table().unwrap();
    mt.set("__mode", "k").unwrap();
    weak.set_metatable(Some(&mt)).unwrap();
    lua.globals().set("weak_host", weak).unwrap();
    let left: i64 = lua
        .load(
            "for i = 1, 100 do weak_host[{}] = i end
             collectgarbage(); collectgarbage()
             local n = 0
             for _ in pairs(weak_host) do n = n + 1 end
             return n",
        )
        .eval()
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn weak_values_from_host_are_cleared() {
    let lua = Lua::new();
    let weak = lua.create_table().unwrap();
    let mt = lua.create_table().unwrap();
    mt.set("__mode", "v").unwrap();
    weak.set_metatable(Some(&mt)).unwrap();
    for i in 1..=100i64 {
        weak.set(i, lua.create_table().unwrap()).unwrap();
    }
    lua.gc_collect();
    lua.gc_collect();
    let mut left = 0;
    for pair in weak.raw_pairs().unwrap() {
        let _ = pair;
        left += 1;
    }
    assert_eq!(left, 0);
}

#[test]
fn gc_metamethod_from_host_runs() {
    let lua = Lua::new();
    lua.load("finalized = 0").exec().unwrap();
    let mt: Table = lua
        .load("return {__gc = function() finalized = finalized + 1 end}")
        .eval()
        .unwrap();
    for _ in 0..10 {
        let t = lua.create_table().unwrap();
        t.set_metatable(Some(&mt)).unwrap();
    }
    lua.gc().collect().unwrap();
    lua.gc().collect().unwrap();
    let n: i64 = lua.globals().get("finalized").unwrap();
    assert_eq!(n, 10);
}

#[test]
fn clearing_metatable_drops_weakness() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    let mt = lua.create_table().unwrap();
    mt.set("__mode", "k").unwrap();
    t.set_metatable(Some(&mt)).unwrap();
    t.set_metatable(None).unwrap();
    assert!(t.get_metatable().unwrap().is_none());
    lua.globals().set("t", t).unwrap();
    let left: i64 = lua
        .load(
            "for i = 1, 10 do t[{}] = i end
             collectgarbage(); collectgarbage()
             local n = 0
             for _ in pairs(t) do n = n + 1 end
             return n",
        )
        .eval()
        .unwrap();
    assert_eq!(left, 10);
}

#[test]
fn metatable_from_another_lua_is_rejected() {
    let a = Lua::new();
    let b = Lua::new();
    let t = a.create_table().unwrap();
    let foreign = b.create_table().unwrap();
    let err = t.set_metatable(Some(&foreign)).unwrap_err();
    assert!(err.to_string().contains("different state"), "{err}");
    assert!(t.get_metatable().unwrap().is_none());
}
