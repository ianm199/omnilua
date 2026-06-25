//! Reusable compiled chunks — issue #227.
//!
//! `Chunk::into_function` compiles source once into a callable `Function` that
//! can be invoked many times, instead of re-parsing on every `exec`/`eval`.

use omnilua::{Function, Lua};

#[test]
fn compile_once_call_many() {
    let lua = Lua::new();
    let doubler: Function = lua.load("local x = ...; return x * 2").into_function().unwrap();

    let a: i64 = doubler.call(3).unwrap();
    let b: i64 = doubler.call(10).unwrap();
    let c: i64 = doubler.call(-5).unwrap();

    assert_eq!((a, b, c), (6, 20, -10));
}

#[test]
fn syntax_error_surfaces_at_compile_time() {
    let lua = Lua::new();
    assert!(lua.load("return 1 +").into_function().is_err());
}

#[test]
fn no_arg_chunk_matches_eval() {
    let lua = Lua::new();
    let f: Function = lua.load("return 6 * 7").into_function().unwrap();
    let n: i64 = f.call(()).unwrap();
    assert_eq!(n, 42);
}

#[test]
fn compiled_function_is_an_ordinary_handle() {
    let lua = Lua::new();
    let f: Function = lua.load("return 42").into_function().unwrap();
    lua.globals().set("f", f).unwrap();

    let v: i64 = lua.load("return f()").eval().unwrap();
    assert_eq!(v, 42);
}
