//! GC control surface — issue #231.
//!
//! `Lua::gc()` returns a handle mirroring mlua's GC controls; each method is
//! the host-side equivalent of a `collectgarbage` option. Verified against the
//! `collectgarbage(...)` results on the same instance.

use omnilua::{Lua, LuaVersion};

#[test]
fn count_rises_after_alloc_and_falls_after_collect() {
    let lua = Lua::new();
    lua.gc().collect().unwrap();
    let base = lua.gc().count().unwrap();

    lua.load("retained = {}; for i = 1, 100000 do retained[i] = { i } end")
        .exec()
        .unwrap();
    let after_alloc = lua.gc().count().unwrap();
    assert!(
        after_alloc > base,
        "count should rise after allocation: base={base} after={after_alloc}"
    );

    lua.load("retained = nil").exec().unwrap();
    lua.gc().collect().unwrap();
    let after_collect = lua.gc().count().unwrap();
    assert!(
        after_collect < after_alloc,
        "count should fall after collect: after_alloc={after_alloc} after_collect={after_collect}"
    );
}

#[test]
fn stop_and_restart_gate_automatic_collection() {
    let lua = Lua::new();
    assert!(lua.gc().is_running().unwrap());

    lua.gc().stop().unwrap();
    assert!(!lua.gc().is_running().unwrap(), "stop() should halt the GC");

    lua.gc().restart().unwrap();
    assert!(
        lua.gc().is_running().unwrap(),
        "restart() should resume the GC"
    );
}

#[test]
fn step_returns_cycle_flag() {
    let lua = Lua::new();
    let _finished: bool = lua.gc().step(0).unwrap();
}

#[test]
fn count_matches_collectgarbage() {
    let lua = Lua::new();
    lua.gc().collect().unwrap();
    let host = lua.gc().count().unwrap();
    let script: f64 = lua.load("return (collectgarbage('count'))").eval().unwrap();
    assert!(
        (host - script).abs() < 16.0,
        "host count {host} should match collectgarbage('count') {script}"
    );
}

#[test]
fn is_running_is_unavailable_before_5_2() {
    let lua = Lua::new_versioned(LuaVersion::V51);
    assert!(
        lua.gc().is_running().is_err(),
        "isrunning has no 5.1 option and should error there"
    );
    lua.gc().collect().unwrap();
    lua.gc().stop().unwrap();
    lua.gc().restart().unwrap();
}
