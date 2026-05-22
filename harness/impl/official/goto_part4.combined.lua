-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- testing closing of upvalues

local debug = require 'debug'

local function foo ()
  local t = {}
  do
  local i = 1
  local a, b, c, d
  t[1] = function () return a, b, c, d end
  ::l1::
  local b
  do
    local c
    t[#t + 1] = function () return a, b, c, d end
    if i > 2 then goto l2 end
    do
      local d
      t[#t + 1] = function () return a, b, c, d end
      i = i + 1
      local a
      goto l1
    end
  end
  end
  ::l2:: return t
end

local a = foo()
assert(#a == 6)
print("part4-a ok #a=" .. #a)

for i = 2, 6 do
  assert(debug.upvalueid(a[1], 1) == debug.upvalueid(a[i], 1))
end
print("part4-b ok")

for i = 2, 6 do
  assert(debug.upvalueid(a[1], 2) ~= debug.upvalueid(a[i], 2))
  assert(debug.upvalueid(a[1], 3) ~= debug.upvalueid(a[i], 3))
end
print("part4-c ok")

for i = 3, 5, 2 do
  assert(debug.upvalueid(a[i], 2) == debug.upvalueid(a[i - 1], 2))
  assert(debug.upvalueid(a[i], 3) == debug.upvalueid(a[i - 1], 3))
  assert(debug.upvalueid(a[i], 2) ~= debug.upvalueid(a[i + 1], 2))
  assert(debug.upvalueid(a[i], 3) ~= debug.upvalueid(a[i + 1], 3))
end
print("part4-d ok")

for i = 2, 6, 2 do
  assert(debug.upvalueid(a[1], 4) == debug.upvalueid(a[i], 4))
end
print("part4-e ok")

for i = 3, 5, 2 do
  for j = 1, 6 do
    assert((debug.upvalueid(a[i], 4) == debug.upvalueid(a[j], 4))
      == (i == j))
  end
end
print("part4-f ok")
