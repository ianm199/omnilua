# omniLua + Bevy — "Lua scripting that follows your game to the browser"

A minimal, self-contained Bevy app whose gameplay logic lives entirely in a
Lua script (`assets/game.lua`), interpreted by [omniLua](../../crates/lua-rs-runtime)
— the pure-Rust Lua 5.1–5.5 runtime. The point is the **wedge**: the same Lua
bytes drive the game natively *and* in the browser, because omniLua is pure
safe Rust with no C dependency and compiles to `wasm32-unknown-unknown`. That
is the seam mlua (which binds the C Lua library) cannot cross.

## What it shows

Every frame, a Bevy system lends the player entity's `Position` component and a
`GameState` resource to a fresh omniLua *scope* and calls the script's
`update(dt)`:

```rust
lua.scope(|s| {
    let player_ud = s.create_userdata_ref_mut(lua, position.as_mut())?;
    let game_ud   = s.create_userdata_ref_mut(lua, game.as_mut())?;
    lua.globals().set("player", &player_ud)?;
    lua.globals().set("game", &game_ud)?;
    let update: omnilua::Function = lua.globals().get("update")?;
    update.call::<f64, ()>(dt)
})?;
```

The Lua code mutates the **real** Bevy component and resource in place — no
copy-out, no per-field glue — and the borrow is invalidated the instant the
scope closes, so a handle a script squirrels away fails cleanly instead of
dangling. `game.lua` bounces the player horizontally, bobs it vertically with
`math.sin`, and accumulates a score:

```lua
function update(dt)
    game.elapsed = game.elapsed + dt
    local nx = player.x + game.vx * SPEED * dt
    if nx > BOUND or nx < -BOUND then game.vx = -game.vx; game.bounces = game.bounces + 1 end
    player.x = math.max(-BOUND, math.min(BOUND, nx))
    player.y = math.sin(game.elapsed * 3.0) * 40.0
    game.score = game.score + math.abs(game.vx) * SPEED * dt
end
```

This is the exact shape a real engine uses: the scheduler owns the `World` and
lends it to a scripting system for one tick. omniLua's `Lua::scope` +
`AnyUserData::delegate` make the `App → World → Component` chain a chain of
short borrows rather than one long-held `&mut`. See
[`crates/lua-rs-runtime/examples/scope_world.rs`](../../crates/lua-rs-runtime/examples/scope_world.rs)
for the pure-API version of the same pattern.

## Bevy version

Pinned to **Bevy `0.16`** (resolves to `0.16.1`). 0.16 is a settled stable
line with a mature `wasm-bindgen` / `trunk` browser path. The native smoke test
uses `default-features = false` → the headless `MinimalPlugins` graph (ECS +
time + transform + math + app, ~125 crates, **no GPU, no winit**), which keeps
the build fast and deterministic. The windowed/wasm build turns on the render
features documented below.

## Build & run — native (headless)

Verified working. From the repository root:

```bash
cargo run --manifest-path examples/bevy/Cargo.toml
```

It runs 180 frames and prints a heartbeat, e.g.:

```
frame   1  player=(   0.00,   0.00)  score=    0.00  bounces=0
frame  30  player=(  28.88,  39.68)  score=   28.88  bounces=0
...
frame 180  player=( 179.70,  17.02)  score=  179.70  bounces=0
```

The advancing `x`, the `sin`-bobbing `y`, and the climbing `score` are all
computed in Lua and written back into live Bevy state — proof the bridge works.

The demo is its **own Cargo workspace** (note the empty `[workspace]` table in
its `Cargo.toml`) and is listed under `exclude` in the root `Cargo.toml`, so
the heavy Bevy graph never enters core omniLua CI or `cargo metadata`.

## Build — wasm (`wasm32-unknown-unknown`)

**Status: the omniLua + Bevy-ECS core compiles cleanly to wasm today.** This is
the load-bearing result for the wedge claim — the scripting bridge survives the
wasm boundary with zero code changes:

```bash
rustup target add wasm32-unknown-unknown          # one-time
cargo build --manifest-path examples/bevy/Cargo.toml --target wasm32-unknown-unknown
# -> Finished; produces target/wasm32-unknown-unknown/debug/omnilua-bevy-demo.wasm
```

This was run and **succeeds**. The omniLua interpreter, its scope/userdata
machinery, and Bevy's ECS all target wasm unchanged. (The debug artifact is
large; a `--release` build plus `wasm-opt -Oz` shrinks it by ~10x.)

### To a *running* in-browser render (the remaining, optional layer)

The headless `main()` above uses `ScheduleRunnerPlugin::run_loop`, which
busy-loops — fine for a CLI smoke test, wrong for a browser (it would block the
JS event loop). A real in-canvas render needs three additions, all documented
here so the path is one build away:

1. **Render features.** Swap the dependency to include the windowed +
   WebGL2 stack:
   ```toml
   bevy = { version = "0.16", default-features = false, features = [
       "bevy_winit", "bevy_core_pipeline", "bevy_sprite", "bevy_render",
       "webgl2", "x11",
   ] }
   ```
   and replace `MinimalPlugins`/`ScheduleRunnerPlugin` with `DefaultPlugins`
   (which drives frames off the winit event loop / `requestAnimationFrame` on
   wasm) plus a `Camera2d` and a `Sprite` rendered at the player `Position`.

2. **wasm-bindgen packaging.** A bare `cargo build --target wasm32-...` emits a
   `.wasm` with no JS glue. Use [`trunk`](https://trunkrs.dev) (already present
   in this environment) — the `index.html` in this directory is a ready trunk
   entrypoint:
   ```bash
   cargo install trunk            # if not present; this env has it
   cd examples/bevy
   trunk serve --release          # builds wasm, runs wasm-bindgen, serves on :8080
   ```
   Trunk invokes `wasm-bindgen` and `wasm-opt` internally, so you do **not**
   need the standalone `wasm-bindgen-cli` on PATH.

3. **Canvas.** Bevy's winit backend creates/attaches the `<canvas>`
   automatically on wasm; no manual canvas wiring is needed for 0.16.

**How far this got tonight:** the native headless demo and the
`wasm32-unknown-unknown` *compile* of the full scripting core are both verified
green. The windowed-render + trunk-bundle step was scaffolded (this README's
feature list + `index.html`) but **not built end-to-end** — Bevy's render
feature graph is the churn-prone layer the session was explicitly told not to
burn the night on, and the compile-to-wasm result already proves the wedge. The
omniLua side of the browser story is independently proven by the existing
[`examples/wasm-browser/`](../wasm-browser/) playground, which runs omniLua in
the browser today via the published npm package.

## Why this matters (the wedge)

mlua and rlua bind the **C** Lua library: they cannot run on
`wasm32-unknown-unknown` without an Emscripten/C toolchain, which Bevy's
browser target does not use. omniLua is pure safe Rust, so a Bevy game's
scripting layer compiles to the *same* wasm target as the rest of the engine —
**the Lua scripting follows your game to the browser** with no second
toolchain, no C, and no separate build for the web. Ship one `game.lua`; it
runs on the desktop binary and in the tab.
