-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

print("before random")
local x = math.random()
print("random done", x)
local y = math.random(10)
print("random(10) done", y)
local z = math.random(5, 15)
print("random(5,15) done", z)
math.randomseed(42)
print("randomseed done")
local w = math.random()
print("after seed", w)
