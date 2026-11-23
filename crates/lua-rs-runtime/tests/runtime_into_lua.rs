//! `LuaRuntime::into_lua` keeps the runtime's Lua version, so host values
//! convert under that version's number model.

use omnilua::{HostHooks, LuaRuntime, LuaVersion, Value};

const ALL: [LuaVersion; 5] = [
    LuaVersion::V51,
    LuaVersion::V52,
    LuaVersion::V53,
    LuaVersion::V54,
    LuaVersion::V55,
];

#[test]
fn into_lua_keeps_version() {
    for version in ALL {
        let rt = LuaRuntime::with_hooks_versioned(HostHooks::default(), version).unwrap();
        assert_eq!(rt.into_lua().version(), version);
    }
}

#[test]
fn into_lua_uses_the_versions_number_model() {
    for version in ALL {
        let lua = LuaRuntime::with_hooks_versioned(HostHooks::default(), version)
            .unwrap()
            .into_lua();
        lua.globals().set("x", 7i64).unwrap();
        let x: Value = lua.globals().get("x").unwrap();
        match version {
            LuaVersion::V51 | LuaVersion::V52 => {
                assert!(
                    matches!(x, Value::Number(n) if n == 7.0),
                    "{version:?}: {x:?}"
                )
            }
            _ => assert!(matches!(x, Value::Integer(7)), "{version:?}: {x:?}"),
        }
    }
}
