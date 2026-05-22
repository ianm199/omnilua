-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

do
  local X
  goto L1

  ::L2:: goto L3

  ::L1:: do
    print("entered L1 block")
    local a <close> = setmetatable({}, {__close = function () print("close fired"); X = true end})
    print("a created, X =", X)
    assert(X == nil)
    print("first assert passed")
    if a then
      print("about to goto L2")
      goto L2
    end
  end

  ::L3::
  print("at L3, X =", X)
  assert(X == true)
end
print("part5-b ok")
