//! Regression test for the T2-B2 install-once chaining panic hook.
//!
//! `aux_resume` (lua-stdlib `coro_lib.rs`) installs a single process-global
//! chaining panic hook on first resume and scopes its `LuaThreadClose`
//! suppression with a thread-local counter, replacing the old per-resume
//! `take_hook` / `set_hook` / restore dance. The behavioral contract that must
//! survive that change: a **non-`LuaThreadClose`** panic raised inside a
//! resumed coroutine still reaches the hook that was current before lua-rs
//! installed its chaining hook — it is delegated to, not swallowed.
//!
//! This file is its own integration-test binary, so it runs in a dedicated
//! process with a fresh `OnceLock` and a clean panic-hook stack. That isolates
//! it from the rest of `cargo test`'s multi-threaded process: no other test's
//! panic, and no other test's resume, can race the once-only install here.
//!
//! The test installs a flag-setting hook **before** the first resume so the
//! chaining hook captures it as its delegate, drives one resume to perform the
//! install, then resumes a coroutine that calls a panicking Rust host
//! function. It asserts the flag was set (the previous hook ran) and that the
//! panic still propagated to the embedder as a Lua error (the callback
//! boundary's own `catch_unwind` converts it, exactly as before this change).

use std::sync::atomic::{AtomicBool, Ordering};

use omnilua::Lua;

/// Unique payload marker so the flag-setting hook only reacts to this test's
/// panic and tolerates any unrelated panic that might occur in the process.
const MARKER: &str = "t2b2-panic-hook-chaining-marker-9f3c";

static PREVIOUS_HOOK_RAN: AtomicBool = AtomicBool::new(false);

#[test]
fn non_threadclose_panic_in_resumed_coroutine_reaches_previous_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let is_marker = info
            .payload()
            .downcast_ref::<&str>()
            .is_some_and(|s| *s == MARKER)
            || info
                .payload()
                .downcast_ref::<String>()
                .is_some_and(|s| s == MARKER);
        if is_marker {
            PREVIOUS_HOOK_RAN.store(true, Ordering::SeqCst);
        } else {
            prev(info);
        }
    }));

    let lua = Lua::new();

    let warm_up = lua
        .create_function(|_lua, ()| Ok(7_i64))
        .expect("warm-up callback should create");
    lua.globals()
        .set("warm_up", warm_up)
        .expect("warm-up callback should register");

    let warmed: i64 = lua
        .load(
            r#"
            local co = coroutine.create(function() return warm_up() end)
            local ok, v = coroutine.resume(co)
            assert(ok, v)
            return v
        "#,
        )
        .eval()
        .expect("warm-up resume should install the chaining hook and return");
    assert_eq!(warmed, 7, "warm-up resume must run normally");

    assert!(
        !PREVIOUS_HOOK_RAN.load(Ordering::SeqCst),
        "the previous hook must not have fired during the non-panicking warm-up"
    );

    let boom = lua
        .create_function(|_lua, ()| -> omnilua::Result<i64> {
            std::panic::panic_any(MARKER)
        })
        .expect("panicking callback should create");
    lua.globals()
        .set("boom", boom)
        .expect("panicking callback should register");

    let result = lua
        .load(
            r#"
            local co = coroutine.create(function() return boom() end)
            local ok, err = coroutine.resume(co)
            assert(ok, err)
        "#,
        )
        .exec();

    assert!(
        result.is_err(),
        "the panic must still propagate to the embedder as a Lua error, \
         not be silently swallowed"
    );
    assert!(
        PREVIOUS_HOOK_RAN.load(Ordering::SeqCst),
        "the chaining hook must delegate a non-LuaThreadClose payload to the \
         previously installed hook"
    );
}
