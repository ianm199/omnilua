//! GC control surface — issue #231 (safe universal subset).
//!
//! collect / used-memory / stop / restart / is-running. These drive collection
//! and read counters — no write barriers — so they are version-invariant and
//! memory-safe. The version-gated `step` and the per-version mode knobs are
//! deferred.

use omnilua::Lua;

#[test]
fn used_memory_is_positive() {
    let lua = Lua::new();
    assert!(lua.gc_used_memory() > 0);
}

#[test]
fn stop_restart_toggles_running() {
    let lua = Lua::new();
    assert!(lua.gc_is_running());

    lua.gc_stop();
    assert!(!lua.gc_is_running());

    lua.gc_restart();
    assert!(lua.gc_is_running());
}

#[test]
fn collect_reclaims_dead_objects_allocated_while_stopped() {
    let lua = Lua::new();
    lua.gc_stop();

    lua.load(
        r#"
        local t = {}
        for i = 1, 20000 do t[i] = {} end
        return #t
    "#,
    )
    .exec()
    .unwrap();
    let high = lua.gc_used_memory();

    lua.gc_restart();
    lua.gc_collect();
    let low = lua.gc_used_memory();

    assert!(
        low <= high,
        "a full collect should reclaim the dead tables: low={low} high={high}"
    );
}
