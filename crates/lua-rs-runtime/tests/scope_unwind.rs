//! A panic inside a `Lua::scope` body still invalidates what the scope
//! created, and the panic reaches the caller.

use std::panic::{catch_unwind, AssertUnwindSafe};

use omnilua::Lua;

#[test]
fn panic_in_scope_body_invalidates_scoped_function() {
    let lua = Lua::new();
    let captured = String::from("captured");
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let _ = lua.scope(|scope| {
            let f = scope.create_function(&lua, |_, ()| Ok(captured.clone()))?;
            lua.globals().set("leaked", f)?;
            panic!("boom");
            #[allow(unreachable_code)]
            Ok(())
        });
    }));
    assert!(caught.is_err());
    let err = lua.load("return leaked()").eval::<String>().unwrap_err();
    assert!(err.to_string().contains("no longer valid"), "{err}");
}
