-- game.lua — the gameplay logic, written in Lua, driving a Bevy entity.
--
-- The host (the Bevy system in src/main.rs) calls update(dt) once per frame.
-- `player` is a live handle to a Bevy entity's Position component, lent to
-- Lua for the duration of this call via omnilua's scope API. `game` is a
-- live handle to a Bevy resource (the score / elapsed clock).
--
-- This whole file is the part that "follows your game to the browser": the
-- exact same bytes run under native Bevy and under wasm Bevy, because omniLua
-- is the interpreter in both.

local SPEED = 60.0      -- units per second
local BOUND = 200.0     -- bounce box half-width

function update(dt)
    game.elapsed = game.elapsed + dt

    -- Bounce the player horizontally inside [-BOUND, BOUND].
    local nx = player.x + game.vx * SPEED * dt
    if nx > BOUND then
        nx = BOUND
        game.vx = -game.vx
        game.bounces = game.bounces + 1
    elseif nx < -BOUND then
        nx = -BOUND
        game.vx = -game.vx
        game.bounces = game.bounces + 1
    end
    player.x = nx

    -- A little vertical bob, purely from Lua math.
    player.y = math.sin(game.elapsed * 3.0) * 40.0

    -- Score ticks up with distance travelled.
    game.score = game.score + math.abs(game.vx) * SPEED * dt
end
