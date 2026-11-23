//! `Chunk::set_environment` gives a chunk its own global table, like the
//! `env` argument of Lua's `load`.

use omnilua::{Function, Lua, LuaVersion, Table};

const ALL: [LuaVersion; 5] = [
    LuaVersion::V51,
    LuaVersion::V52,
    LuaVersion::V53,
    LuaVersion::V54,
    LuaVersion::V55,
];

fn sandbox_env(lua: &Lua) -> Table {
    let env = lua.create_table().unwrap();
    env.set(
        "tostring",
        lua.globals().get::<_, Function>("tostring").unwrap(),
    )
    .unwrap();
    env.set("x", 41).unwrap();
    env
}

#[test]
fn exec_reads_and_writes_the_environment() {
    for version in ALL {
        let lua = Lua::new_versioned(version);
        let env = sandbox_env(&lua);
        lua.load("y = x + 1; print = nil")
            .set_environment(env.clone())
            .exec()
            .unwrap();
        assert_eq!(env.get::<_, i64>("y").unwrap(), 42, "{version:?}");
        assert!(lua.globals().get::<_, Option<i64>>("y").unwrap().is_none());
        assert!(lua
            .globals()
            .get::<_, Option<Function>>("print")
            .unwrap()
            .is_some());
    }
}

#[test]
fn eval_sees_only_the_environment() {
    for version in ALL {
        let lua = Lua::new_versioned(version);
        lua.globals().set("x", 1000).unwrap();
        let env = sandbox_env(&lua);
        let out: String = lua
            .load("return tostring(x) .. tostring(print)")
            .set_environment(env)
            .eval()
            .unwrap();
        assert_eq!(out, "41nil", "{version:?}");
    }
}

#[test]
fn functions_defined_in_the_chunk_keep_the_environment() {
    for version in ALL {
        let lua = Lua::new_versioned(version);
        let env = sandbox_env(&lua);
        let f: Function = lua
            .load("function bump() x = x + 1; return x end")
            .set_environment(env.clone())
            .into_function()
            .unwrap();
        f.call::<_, ()>(()).unwrap();
        let bump: Function = env.get("bump").unwrap();
        assert_eq!(bump.call::<_, i64>(()).unwrap(), 42, "{version:?}");
        assert_eq!(env.get::<_, i64>("x").unwrap(), 42);
    }
}

#[test]
fn environment_from_another_lua_is_rejected() {
    let a = Lua::new();
    let b = Lua::new();
    let foreign = b.create_table().unwrap();
    let err = a
        .load("return 1")
        .set_environment(foreign)
        .eval::<i64>()
        .unwrap_err();
    assert!(err.to_string().contains("different state"), "{err}");
}
