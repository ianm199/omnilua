-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

print("a")
local A,B = 0,{g=10}
print("b")
local function f(x)
  local a = {}
  for i=1,3 do
    local y = 0
    do
      a[i] = function () B.g = B.g+1; y = y+x; return y+A end
    end
  end
  print("created")
  local dummy = function () return a[A] end
  collectgarbage()
  A = 1
  print("dummy()=", dummy(), "a[1]=", a[1])
  assert(dummy() == a[1])
  A = 0
  print("d ok")
  assert(a[1]() == x)
  print("1 ok")
  assert(a[3]() == x)
  print("3 ok")
  collectgarbage()
  assert(B.g == 12)
  return a
end
local r = f(10)
print("end:", r)
