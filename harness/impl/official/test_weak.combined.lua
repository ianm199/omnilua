-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

a = {}
local t = {x = 10}
local C = setmetatable({key = t}, {__mode = 'v'})
local C1 = setmetatable({[t] = 1}, {__mode = 'k'})
a.x = t

setmetatable(a, {__gc = function (u)
  print("[gc] C.key:", C.key)
  print("[gc] next(C1):", next(C1))
  print("[gc] type(next(C1)):", type(next(C1)))
end})

print("before nil")
a, t = nil
print("after nil, before gc1")
collectgarbage()
print("after gc1, before gc2")
collectgarbage()
print("after gc2")
print("next(C):", next(C))
print("next(C1):", next(C1))
assert(next(C) == nil and next(C1) == nil)
print("OK")
