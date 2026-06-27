//! Host-driven coroutines — issue #230.
//!
//! A host can create a coroutine from a Lua function, resume it across several
//! steps passing/receiving values, and observe its status transitions, without
//! dropping into Lua-level `coroutine.*`. Behavior must match running the same
//! coroutine purely in Lua, because the API drives the same builtins.

use omnilua::{Function, Lua, ThreadStatus};

const YIELDING_BODY: &str = "return function(a)
    local b = coroutine.yield(a + 1)
    local c = coroutine.yield(b + 1)
    return c + 1
end";

#[test]
fn resume_across_steps_passing_and_receiving_values() {
    let lua = Lua::new();
    let body: Function = lua.load(YIELDING_BODY).eval().unwrap();
    let co = lua.create_thread(body).unwrap();

    assert_eq!(co.status().unwrap(), ThreadStatus::Suspended);

    let first: i64 = co.resume(10).unwrap();
    assert_eq!(first, 11);
    assert_eq!(co.status().unwrap(), ThreadStatus::Suspended);

    let second: i64 = co.resume(20).unwrap();
    assert_eq!(second, 21);
    assert_eq!(co.status().unwrap(), ThreadStatus::Suspended);

    let third: i64 = co.resume(30).unwrap();
    assert_eq!(third, 31);
    assert_eq!(co.status().unwrap(), ThreadStatus::Dead);
}

#[test]
fn host_resume_matches_pure_lua() {
    let lua = Lua::new();
    let body: Function = lua.load(YIELDING_BODY).eval().unwrap();
    let co = lua.create_thread(body).unwrap();
    let host: (i64, i64, i64) = (
        co.resume(10).unwrap(),
        co.resume(20).unwrap(),
        co.resume(30).unwrap(),
    );

    let pure: (i64, i64, i64) = lua
        .load(
            "local body = ".to_string()
                + YIELDING_BODY.trim_start_matches("return ")
                + "
            local co = coroutine.create(body)
            local _, a = coroutine.resume(co, 10)
            local _, b = coroutine.resume(co, 20)
            local _, c = coroutine.resume(co, 30)
            return a, b, c",
        )
        .eval()
        .unwrap();

    assert_eq!(host, pure);
}

#[test]
fn resuming_a_dead_coroutine_errors() {
    let lua = Lua::new();
    let body: Function = lua.load("return function() return 1 end").eval().unwrap();
    let co = lua.create_thread(body).unwrap();

    let _done: i64 = co.resume(()).unwrap();
    assert_eq!(co.status().unwrap(), ThreadStatus::Dead);

    let err = co.resume::<(), i64>(()).unwrap_err();
    assert!(
        err.to_string().contains("dead"),
        "expected a dead-coroutine error, got: {err}"
    );
}

#[test]
fn coroutine_error_surfaces_as_err() {
    let lua = Lua::new();
    let body: Function = lua
        .load("return function() error('boom from coroutine') end")
        .eval()
        .unwrap();
    let co = lua.create_thread(body).unwrap();

    let err = co.resume::<(), ()>(()).unwrap_err();
    assert!(
        err.to_string().contains("boom from coroutine"),
        "expected the raised error to surface, got: {err}"
    );
    assert_eq!(co.status().unwrap(), ThreadStatus::Dead);
}

#[test]
fn thread_is_provenance_bound_to_its_instance() {
    let lua_a = Lua::new();
    let lua_b = Lua::new();
    let body_b: Function = lua_b.load("return function() end").eval().unwrap();

    let err = lua_a.create_thread(body_b).unwrap_err();
    assert!(
        err.to_string().contains("different state"),
        "expected a cross-instance rejection, got: {err}"
    );
}

#[test]
fn normal_status_when_resuming_a_child() {
    let lua = Lua::new();
    let parent: Function = lua
        .load(
            "return function()
                local child = coroutine.create(function(outer)
                    coroutine.yield(coroutine.status(outer))
                end)
                local _, child_view = coroutine.resume(child, coroutine.running())
                coroutine.yield(child_view)
            end",
        )
        .eval()
        .unwrap();
    let co = lua.create_thread(parent).unwrap();

    let child_view_of_parent: String = co.resume(()).unwrap();
    assert_eq!(child_view_of_parent, "normal");
}
