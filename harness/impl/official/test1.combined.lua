-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

local x = "x \v\f = \t\r 'a\0a' \v\f\f"
print(#x)
local f, err = load(x)
print(f, err)
