//! End-to-end smoke for the headline scope use case: handing Lua a
//! short-lived `&mut World` plus a few closures borrowing from the same
//! stack frame.
//!
//! `World` here is a deliberately tiny stand-in for `bevy::ecs::World` —
//! enough surface (a HashMap of named entities, position/velocity records)
//! to exercise the parts that matter: `&mut World` flowing through a Lua
//! method, mutations being visible from Rust after the scope returns, and
//! invalidation kicking in if the script squirrels the userdata away on a
//! global.
//!
//! Why this exists: the inline unit tests pin the *mechanism* (cells,
//! borrow counters, invalidation). This test pins the *integration shape*:
//! a script that looks like a real Bevy system body, working against an
//! API that looks like real Bevy. If the API ever drifts, this test
//! flags it before users do.

use omnilua::{Lua, LuaError, MetaMethod, UserData, UserDataMethods, Value};
use std::collections::HashMap;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Pos {
    x: f64,
    y: f64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Vel {
    x: f64,
    y: f64,
}

#[derive(Debug, Default)]
struct Entity {
    pos: Pos,
    vel: Vel,
}

/// A `World`-shape: a name-keyed entity store. In real Bevy this is the
/// `bevy::ecs::World`; the shape that matters for this test is just
/// "something held by `&mut` and used to spawn / mutate entities."
#[derive(Default)]
struct World {
    entities: HashMap<String, Entity>,
    log: Vec<String>,
}

impl UserData for World {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("spawn", |_lua, this, name: String| {
            this.entities.entry(name.clone()).or_default();
            this.log.push(format!("spawn {}", name));
            Ok(())
        });

        methods.add_method_mut("set_pos", |_lua, this, (name, x, y): (String, f64, f64)| {
            let e = this.entities.entry(name).or_default();
            e.pos = Pos { x, y };
            Ok(())
        });

        methods.add_method_mut("set_vel", |_lua, this, (name, x, y): (String, f64, f64)| {
            let e = this.entities.entry(name).or_default();
            e.vel = Vel { x, y };
            Ok(())
        });

        methods.add_method_mut("step", |_lua, this, dt: f64| {
            for e in this.entities.values_mut() {
                e.pos.x += e.vel.x * dt;
                e.pos.y += e.vel.y * dt;
            }
            Ok(())
        });

        methods.add_method("pos", |_lua, this, name: String| {
            let e = this
                .entities
                .get(&name)
                .ok_or_else(|| LuaError::runtime(format_args!("no entity named {name}")))?;
            Ok((e.pos.x, e.pos.y))
        });

        methods.add_method("count", |_lua, this, ()| Ok(this.entities.len() as i64));
    }

    fn add_meta_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Index, |_lua, this, key: String| {
            if key == "len" {
                Ok(Value::Integer(this.entities.len() as i64))
            } else {
                Ok(Value::Nil)
            }
        });
    }
}

/// The script we're going to run — looks plausibly like a Bevy system body.
const TICK_SCRIPT: &str = r#"
    -- "spawn three entities, set their velocities, advance the world by dt."
    world:spawn("a")
    world:spawn("b")
    world:spawn("c")
    world:set_pos("a", 0, 0)
    world:set_vel("a", 1, 0)
    world:set_pos("b", 10, 10)
    world:set_vel("b", 0, 2)
    world:set_pos("c", 100, 100)
    world:set_vel("c", -1, -1)

    world:step(0.5)
    world:step(0.5)

    -- Read back through __index for variety.
    return world.len
"#;

/// The headline test: a `&mut World` flows into Lua, the script mutates it,
/// Rust observes the mutations after the scope ends.
#[test]
fn world_round_trips_through_scope() {
    let lua = Lua::new();
    let mut world = World::default();

    let count: i64 = lua
        .scope(|scope| {
            let w = scope.create_userdata_ref_mut(&lua, &mut world)?;
            lua.globals().set("world", &w)?;
            lua.load(TICK_SCRIPT).eval::<i64>()
        })
        .expect("scope body should succeed");

    assert_eq!(count, 3);
    assert_eq!(world.entities.len(), 3);
    let a = world.entities.get("a").expect("a was spawned");
    let b = world.entities.get("b").expect("b was spawned");
    let c = world.entities.get("c").expect("c was spawned");
    assert_eq!(a.pos, Pos { x: 1.0, y: 0.0 });
    assert_eq!(b.pos, Pos { x: 10.0, y: 12.0 });
    assert_eq!(c.pos, Pos { x: 99.0, y: 99.0 });
    assert_eq!(world.log.len(), 3);
}

/// The defensive test: a script that squirrels the `world` userdata on a
/// global and tries to use it after the scope ends gets a clean Lua error,
/// not a stale `&mut World`.
#[test]
fn world_stashed_on_global_is_unusable_after_scope() {
    let lua = Lua::new();
    let mut world = World::default();

    lua.scope(|scope| {
        let w = scope.create_userdata_ref_mut(&lua, &mut world)?;
        lua.globals().set("escaped", &w)?;
        lua.load("escaped:spawn(\"in_scope\")").exec()
    })
    .expect("scope body should succeed");

    assert_eq!(world.entities.len(), 1, "in-scope spawn happened");

    let err = lua
        .load("escaped:spawn(\"after_scope\")")
        .exec()
        .expect_err("post-scope call must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Str(") || msg.contains("Runtime"),
        "expected a Lua runtime error, got: {msg}"
    );
    assert_eq!(
        world.entities.len(),
        1,
        "post-scope spawn must not have touched the world"
    );
}

/// A scoped function paired with a scoped `&mut World`. The closure reads
/// borrowed Rust-side state (a `done` flag) while the userdata mutates the
/// world. Models a real system body that wants both shapes in one scope.
#[test]
fn world_and_scoped_callback_together() {
    let lua = Lua::new();
    let mut world = World::default();
    let mut tick_log: Vec<String> = Vec::new();

    lua.scope(|scope| {
        let w = scope.create_userdata_ref_mut(&lua, &mut world)?;
        let log_tick = scope.create_function_mut(&lua, |_lua, name: String| {
            tick_log.push(name);
            Ok(())
        })?;
        lua.globals().set("world", &w)?;
        lua.globals().set("log_tick", &log_tick)?;
        lua.load(
            r#"
            for _, name in ipairs({"alpha", "beta", "gamma"}) do
                world:spawn(name)
                log_tick(name)
            end
        "#,
        )
        .exec()
    })
    .expect("mixed scope body should succeed");

    assert_eq!(world.entities.len(), 3);
    assert_eq!(tick_log, vec!["alpha", "beta", "gamma"]);
}
