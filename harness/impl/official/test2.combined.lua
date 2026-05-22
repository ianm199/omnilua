-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

local function dostring (x) return assert(load(x), "")() end

dostring("x \v\f = \t\r 'a\0a' \v\f\f")
assert(x == 'a\0a' and string.len(x) == 3)
print("ok")
