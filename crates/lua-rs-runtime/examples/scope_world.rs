//! Lending a `&mut World` to Lua with `Lua::scope`, and exposing live
//! sub-references to its entities with `AnyUserData::delegate`.
//!
//! Run with:
//!
//! ```text
//! cargo run -p lua-rs-runtime --example scope_world
//! ```
//!
//! This is the shape a game engine (e.g. Bevy) uses to drive scripts: the
//! scheduler owns the `World` and lends it to a system for one tick. The
//! script gets to mutate it through Lua for the duration of the call, then
//! the borrow goes back to Rust. If a script squirrels a handle away and
//! tries to use it after the tick, the call fails cleanly instead of
//! touching a dangling pointer.

use lua_rs_runtime::{AnyUserData, Lua, Result, UserData, UserDataMethods};

/// A component on an entity.
#[derive(Default)]
struct Position {
    x: f64,
    y: f64,
}

impl UserData for Position {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_field_method_get("x", |_, this| Ok(this.x));
        m.add_field_method_get("y", |_, this| Ok(this.y));
        m.add_field_method_set("x", |_, this, v: f64| {
            this.x = v;
            Ok(())
        });
        m.add_field_method_set("y", |_, this, v: f64| {
            this.y = v;
            Ok(())
        });
    }
}

/// The thing the host owns and lends to scripts for one tick.
#[derive(Default)]
struct World {
    entities: Vec<Position>,
}

impl UserData for World {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method_mut("spawn", |_, this, ()| {
            this.entities.push(Position::default());
            Ok((this.entities.len() - 1) as i64)
        });
        m.add_method("count", |_, this, ()| Ok(this.entities.len() as i64));

        // `world:position(i)` returns a sub-userdata holding a live `&mut
        // Position`, re-borrowed from the World on every field access. No
        // clone, no write-back: the script mutates the real component.
        m.add_function("position", |lua, (this, idx): (AnyUserData, i64)| {
            let i = idx as usize;
            this.delegate::<World, Position, _>(lua, move |w| &mut w.entities[i])
        });
    }
}

fn main() -> Result<()> {
    let lua = Lua::new();
    let mut world = World::default();

    let count: i64 = lua.scope(|s| {
        let world_ud = s.create_userdata_ref_mut(&lua, &mut world)?;
        lua.globals().set("world", &world_ud)?;

        lua.load(
            r#"
            local a = world:spawn()
            local b = world:spawn()

            -- Direct mutation through a live sub-reference.
            local pa = world:position(a)
            pa.x, pa.y = 10, 20

            local pb = world:position(b)
            pb.x = pa.x + 5      -- reading pa here re-borrows; both are short-lived

            -- Stash one to prove it dies with the scope.
            escaped = world:position(a)

            return world:count()
        "#,
        )
        .eval()
    })?;

    println!("spawned {count} entities");
    // The mutations are visible to Rust after the scope returns.
    for (i, e) in world.entities.iter().enumerate() {
        println!("entity {i} = ({}, {})", e.x, e.y);
    }

    // The handle the script stashed on `escaped` is now invalid. Reading a
    // field raises a Lua error rather than touching the released `&mut World`.
    // We surface the message through Lua's own `pcall` + `tostring`, since the
    // error payload is a Lua value.
    let (ok, msg): (bool, String) = lua
        .load("local ok, e = pcall(function() return escaped.x end); return ok, tostring(e)")
        .eval()?;
    assert!(!ok, "stashed handle should be unusable after the scope");
    println!("post-scope use of a stashed handle -> {msg}");

    Ok(())
}
