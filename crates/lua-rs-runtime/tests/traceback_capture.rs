//! Stack-traceback capture — issue #229 (codex-reviewed design).
//!
//! Opt-in (`set_capture_tracebacks`). When off (default), error handling is
//! byte-identical and `traceback_bytes()` is `None`. When on, a per-call message
//! handler stashes the `debug.traceback()` stack into the `Error` WITHOUT altering
//! the error message.

use omnilua::{Function, Lua, LuaError};

#[test]
fn capture_off_by_default_clean_message_no_traceback() {
    let lua = Lua::new();
    assert!(!lua.captures_tracebacks());

    let err = lua.load("error('boom')").exec().unwrap_err();
    assert!(err.traceback_bytes().is_none());

    let msg = format!("{err}");
    assert!(msg.contains("boom"), "msg: {msg}");
    assert!(!msg.contains("stack traceback"), "message must stay clean: {msg}");
}

#[test]
fn capture_on_produces_traceback_without_polluting_message() {
    let lua = Lua::new();
    lua.set_capture_tracebacks(true);

    let err = lua
        .load("local function f() error('boom') end f()")
        .exec()
        .unwrap_err();

    let tb = err.traceback_lossy().expect("traceback should be captured");
    assert!(tb.contains("stack traceback"), "tb: {tb}");

    let msg = format!("{err}");
    assert!(msg.contains("boom"), "msg: {msg}");
    assert!(
        !msg.contains("stack traceback"),
        "the message must NOT contain the traceback: {msg}"
    );
}

#[test]
fn eval_and_function_call_both_capture() {
    let lua = Lua::new();
    lua.set_capture_tracebacks(true);

    let e1 = lua.load("error('via eval')").eval::<i64>().unwrap_err();
    assert!(e1.traceback_bytes().is_some());

    let f: Function = lua.load("return function() error('via call') end").eval().unwrap();
    let e2 = f.call::<(), ()>(()).unwrap_err();
    assert!(e2.traceback_lossy().unwrap().contains("stack traceback"));
}

#[test]
fn toggling_off_disables_capture() {
    let lua = Lua::new();
    lua.set_capture_tracebacks(true);
    lua.set_capture_tracebacks(false);
    assert!(!lua.captures_tracebacks());

    let err = lua.load("error('z')").exec().unwrap_err();
    assert!(err.traceback_bytes().is_none());
}

#[test]
fn success_path_undisturbed_with_capture_on() {
    let lua = Lua::new();
    lua.set_capture_tracebacks(true);

    let n: i64 = lua.load("return 6 * 7").eval().unwrap();
    assert_eq!(n, 42);

    let f: Function = lua.load("return function(a, b) return a + b end").eval().unwrap();
    let s: i64 = f.call((20, 22)).unwrap();
    assert_eq!(s, 42);

    lua.load("local x = 1").exec().unwrap();
}

#[test]
fn fresh_traceback_per_call_no_stale_leak() {
    let lua = Lua::new();
    lua.set_capture_tracebacks(true);

    let e1 = lua.load("local function a() error('one') end a()").exec().unwrap_err();
    assert!(e1.traceback_bytes().is_some());

    let ok: i64 = lua.load("return 5").eval().unwrap();
    assert_eq!(ok, 5);

    let e3 = lua.load("error('three')").exec().unwrap_err();
    assert!(e3.traceback_bytes().is_some());
    assert!(format!("{e3}").contains("three"));
}

#[test]
fn non_string_error_object_captures_without_running_metamethods() {
    let lua = Lua::new();
    lua.set_capture_tracebacks(true);

    let err = lua.load("error(setmetatable({}, {__tostring = function() error('mm') end}))")
        .exec()
        .unwrap_err();

    // The handler uses msg=None, so it never invokes __tostring; capture still
    // succeeds and the error object is preserved as a table.
    assert!(err.traceback_bytes().is_some());
    assert!(matches!(err.kind(), LuaError::Runtime(_)));
}
