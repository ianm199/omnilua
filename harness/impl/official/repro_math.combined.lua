-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

local a = "2"
print("step1: a=", a, type(a))
local b = a + 1
print("step2: b=", b, type(b))
