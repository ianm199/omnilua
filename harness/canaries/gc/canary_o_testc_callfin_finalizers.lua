-- To-be-finalized objects should be dispatched from the observable callfin
-- phase, not left for an unrelated later VM check.

assert(T and T.gcstate and T.gcstats and T.checkmemory,
       "FAIL: testC GC state helpers missing")

local function statnum(stats, name)
  return assert(tonumber(stats:match(name .. "=(%-?%d+)")),
                "missing stat " .. name .. " in " .. tostring(stats))
end

collectgarbage("incremental")
collectgarbage("stop")

local ran = 0
do
  local obj = setmetatable({}, { __gc = function() ran = ran + 1 end })
  obj = nil
end

T.gcstate("callfin")
assert(T.gcstate() == "callfin", "FAIL: did not stop in callfin")
assert(statnum(T.gcstats(), "tobefin") > 0,
       "FAIL: unreachable finalizer did not enter tobefnz")
assert(ran == 0, "FAIL: finalizer ran before callfin step")

collectgarbage("restart")
collectgarbage("step", 0)
assert(ran == 1, "FAIL: callfin step did not run finalizer")
assert(T.gcstate() == "pause",
       "FAIL: empty callfin should finish after draining finalizers")

T.checkmemory()
print("PASS canary_o")
