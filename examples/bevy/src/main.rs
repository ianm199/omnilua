//! omniLua + Bevy wedge demo: "Lua scripting that follows your game to the
//! browser."
//!
//! A tiny Bevy app whose gameplay logic lives entirely in `assets/game.lua`.
//! Every frame, a Bevy system lends the player entity's `Position` component
//! and a `GameState` resource to a Lua scope (via omniLua's `Lua::scope` +
//! `create_userdata_ref_mut`), then calls the script's `update(dt)`. The Lua
//! code mutates the real Bevy component and resource in place — no copy-out,
//! no glue per field — and Rust reads the results back after the borrow
//! returns.
//!
//! The point of the demo is the *seam*: the exact same `game.lua` bytes drive
//! the game natively and (with the wasm build, see README) in the browser,
//! because omniLua is a pure-Rust interpreter that compiles to
//! `wasm32-unknown-unknown` with no C toolchain. That is the wedge mlua cannot
//! reach.
//!
//! Run natively (headless, prints frames):
//!
//! ```text
//! cargo run --manifest-path examples/bevy/Cargo.toml
//! ```

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

use omnilua::{Lua, UserData, UserDataMethods};

/// The player entity's world position. A normal Bevy component; the script
/// reads and writes `player.x` / `player.y` against the live value.
#[derive(Component, Default)]
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

/// Shared gameplay state the host owns and the script mutates: score, elapsed
/// clock, horizontal velocity sign, and a bounce counter. A Bevy resource on
/// the Rust side; a plain table-like handle (`game.score`, `game.vx`, ...) on
/// the Lua side.
#[derive(Resource)]
struct GameState {
    score: f64,
    elapsed: f64,
    vx: f64,
    bounces: i64,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            score: 0.0,
            elapsed: 0.0,
            vx: 1.0,
            bounces: 0,
        }
    }
}

impl UserData for GameState {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_field_method_get("score", |_, this| Ok(this.score));
        m.add_field_method_get("elapsed", |_, this| Ok(this.elapsed));
        m.add_field_method_get("vx", |_, this| Ok(this.vx));
        m.add_field_method_get("bounces", |_, this| Ok(this.bounces));
        m.add_field_method_set("score", |_, this, v: f64| {
            this.score = v;
            Ok(())
        });
        m.add_field_method_set("elapsed", |_, this, v: f64| {
            this.elapsed = v;
            Ok(())
        });
        m.add_field_method_set("vx", |_, this, v: f64| {
            this.vx = v;
            Ok(())
        });
        m.add_field_method_set("bounces", |_, this, v: i64| {
            this.bounces = v;
            Ok(())
        });
    }
}

/// Holds the live omniLua interpreter and the compiled script for the run.
///
/// `omnilua::Lua` is `!Send` (it roots handles through `Rc`), so this is a
/// Bevy **non-send** resource: Bevy runs systems that touch it on the main
/// thread, which is exactly the constraint a single-threaded scripting VM
/// wants. The interpreter is created once at startup and reused every frame;
/// only the per-frame borrows are scoped.
struct ScriptEngine {
    lua: Lua,
}

/// How many frames the headless run executes before exiting cleanly. Keeps
/// `cargo run` a finite, CI-friendly smoke test rather than an infinite loop.
const FRAMES_TO_RUN: u64 = 180;

#[derive(Resource, Default)]
struct FrameCount(u64);

fn setup(world: &mut World) {
    let lua = Lua::new();
    lua.load(include_str!("../assets/game.lua"))
        .set_name("game.lua")
        .exec()
        .expect("game.lua failed to load");
    world.insert_non_send_resource(ScriptEngine { lua });

    world.spawn(Position::default());
}

/// The bridge system: lend the player `Position` and the `GameState` resource
/// to Lua for one `update(dt)` call, then let the borrow return.
fn run_script(
    engine: NonSend<ScriptEngine>,
    time: Res<Time>,
    mut players: Query<&mut Position>,
    mut game: ResMut<GameState>,
) {
    let dt = time.delta_secs_f64();
    let lua = &engine.lua;

    for mut position in &mut players {
        lua.scope(|s| {
            let player_ud = s.create_userdata_ref_mut(lua, position.as_mut())?;
            let game_ud = s.create_userdata_ref_mut(lua, game.as_mut())?;
            let globals = lua.globals();
            globals.set("player", &player_ud)?;
            globals.set("game", &game_ud)?;

            let update: omnilua::Function = globals.get("update")?;
            update.call::<f64, ()>(dt)
        })
        .expect("update(dt) raised a Lua error");
    }
}

/// Print a heartbeat and exit after `FRAMES_TO_RUN` frames so the headless run
/// terminates. A windowed/wasm build (see README) would drop this and run the
/// event loop forever.
fn report_and_maybe_exit(
    mut frames: ResMut<FrameCount>,
    game: Res<GameState>,
    players: Query<&Position>,
    mut exits: EventWriter<AppExit>,
) {
    frames.0 += 1;
    if frames.0 % 30 == 0 || frames.0 == 1 {
        if let Ok(p) = players.single() {
            println!(
                "frame {:>3}  player=({:7.2}, {:6.2})  score={:8.2}  bounces={}",
                frames.0, p.x, p.y, game.score, game.bounces
            );
        }
    }
    if frames.0 >= FRAMES_TO_RUN {
        exits.write(AppExit::Success);
    }
}

fn main() {
    App::new()
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f64(1.0 / 60.0),
        )))
        .init_resource::<GameState>()
        .init_resource::<FrameCount>()
        .add_systems(Startup, setup)
        .add_systems(Update, (run_script, report_and_maybe_exit).chain())
        .run();
}
