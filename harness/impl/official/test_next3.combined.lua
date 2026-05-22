-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- erasing values - minimal repro
local t = {[{1}] = 1, [{2}] = 2, [string.rep("x ", 4)] = 3,
           [100.3] = 4, [4] = 5}

print('start')
local n = 0
local prev = nil
for k, v in pairs( t ) do
  n = n+1
  print('iter', n, 'k=', k, 'v=', v)
  assert(t[k] == v)
  t[k] = nil
  collectgarbage()
  assert(t[k] == nil)
end
print('F', n)
assert(n == 5)
print('G')
