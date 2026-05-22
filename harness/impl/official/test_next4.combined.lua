-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

local t = {[{1}] = 1, [{2}] = 2}
print('start')
local n = 0
for k, v in pairs(t) do
  n = n+1
  print('iter', n, k, v)
  assert(t[k] == v)
  t[k] = nil
  collectgarbage()
end
print('done', n)
