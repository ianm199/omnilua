//! Smoke test for the sandbox / embedded feature profile (issue #223).
//!
//! Runs a representative sandbox script — `string` / `table` / `math` only —
//! and asserts that each feature-gated standard library is present *iff* its
//! Cargo feature is enabled. The always-on core (`base`, `string`, `table`,
//! `math`) must work in every profile; `io` / `os` / `package` / `debug` are
//! present only when their feature is compiled in.
//!
//! Run the lean (sandboxed) profile and the full profile:
//!
//! ```sh
//! cargo run -p omnilua --no-default-features --example sandbox_smoke
//! cargo run -p omnilua --example sandbox_smoke
//! ```

use omnilua::Lua;

/// Lua-side `type(<name>)`, the version-agnostic way to ask whether a global
/// library table exists (`"table"`) or was never registered (`"nil"`).
fn type_of(lua: &Lua, name: &str) -> String {
    lua.load(format!("return type({name})")).eval().unwrap()
}

fn main() {
    let lua = Lua::new();

    let sum: i64 = lua
        .load("local t = {} for i = 1, 10 do t[i] = i end local s = 0 for _, v in ipairs(t) do s = s + v end return s")
        .eval()
        .unwrap();
    assert_eq!(sum, 55, "table/loop core must work in every profile");

    let formatted: String = lua
        .load(r#"return string.format("%d-%s", math.floor(3.7), "ok")"#)
        .eval()
        .unwrap();
    assert_eq!(
        formatted, "3-ok",
        "string/math core must work in every profile"
    );

    let gated = [
        ("io", cfg!(feature = "io")),
        ("os", cfg!(feature = "os")),
        ("package", cfg!(feature = "package")),
        ("debug", cfg!(feature = "debug")),
    ];
    for (name, enabled) in gated {
        let ty = type_of(&lua, name);
        let want = if enabled { "table" } else { "nil" };
        assert_eq!(
            ty, want,
            "library `{name}` should be `{want}` when its feature is {}enabled",
            if enabled { "" } else { "not " }
        );
        println!("  {name:<8} = {ty:<5} (feature enabled = {enabled})");
    }

    println!("sandbox_smoke OK: core libs work; gated libs match their compiled features");
}
