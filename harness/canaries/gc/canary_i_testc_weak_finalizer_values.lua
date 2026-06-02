-- Weak values that are pending finalization must be cleared before __gc runs,
-- while the same objects remain visible as weak keys to their finalizer.

assert(T and T.newuserdata, "FAIL: testC userdata helper missing")

collectgarbage("incremental")
collectgarbage("stop")

local undef
local function newproxy(u)
  return debug.setmetatable(T.newuserdata(0), debug.getmetatable(u))
end

local u = newproxy(nil)
debug.setmetatable(u, {__gc = true})
local s = 0
local a = {[u] = 0}
setmetatable(a, {__mode = "vk"})

for i = 1, 10 do
  a[newproxy(u)] = i
end

local a1 = {}
for k, v in pairs(a) do
  a1[k] = v
end
for k, v in pairs(a1) do
  a[v] = k
end

getmetatable(u).a = a1
getmetatable(u).u = u
do
  local u = u
  getmetatable(u).__gc = function(o)
    assert(a[o] == 10 - s, "FAIL: finalizable weak key was cleared too early")
    assert(a[10 - s] == undef, "FAIL: finalizable weak value survived into __gc")
    assert(getmetatable(o) == getmetatable(u), "FAIL: userdata metatable changed")
    assert(getmetatable(o).a[o] == 10 - s, "FAIL: finalizer metatable state lost")
    s = s + 1
  end
end

a1, u = nil
assert(next(a) ~= nil, "FAIL: weak table emptied before collection")
collectgarbage()
assert(s == 11, "FAIL: expected all userdata finalizers")
collectgarbage()
assert(next(a) == nil, "FAIL: finalized weak keys were not cleared in second cycle")

collectgarbage("restart")
print("PASS canary_i")
