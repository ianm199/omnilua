-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- testing if x goto optimizations

local function testG (a)
  if a == 1 then
    goto l1
    error("should never be here!")
  elseif a == 2 then goto l2
  elseif a == 3 then goto l3
  elseif a == 4 then
    goto l1
    error("should never be here!")
    ::l1:: a = a + 1
  else
    goto l4
    ::l4a:: a = a * 2; goto l4b
    error("should never be here!")
    ::l4:: goto l4a
    error("should never be here!")
    ::l4b::
  end
  do return a end
  ::l2:: do return "2" end
  ::l3:: do return "3" end
  ::l1:: return "1"
end

print("testG(1) =", testG(1))
print("testG(2) =", testG(2))
print("testG(3) =", testG(3))
print("testG(4) =", testG(4))
print("testG(5) =", testG(5))
assert(testG(1) == "1")
assert(testG(2) == "2")
assert(testG(3) == "3")
assert(testG(4) == 5)
assert(testG(5) == 10)
print("part5-a ok")

do
  local X
  goto L1

  ::L2:: goto L3

  ::L1:: do
    local a <close> = setmetatable({}, {__close = function () X = true end})
    assert(X == nil)
    if a then goto L2 end
  end

  ::L3:: assert(X == true)
end
print("part5-b ok")
