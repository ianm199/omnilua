-- During sweep, assigning a white table into a black userdata uservalue must
-- trigger the backward barrier and turn the userdata gray for rescanning.

assert(T and T.gcstate and T.gccolor and T.newuserdata,
       "FAIL: testC GC helpers missing")

collectgarbage("incremental")
local u = T.newuserdata(0, 1)
collectgarbage()
collectgarbage("stop")

local anchor = {}
T.gcstate("atomic")
T.gcstate("sweepallgc")
local x = {}

assert(T.gccolor(u) == "black", "FAIL: userdata was not black in sweep")
assert(T.gccolor(x) == "white", "FAIL: new table was not white in sweep")
debug.setuservalue(u, x)
assert(T.gccolor(u) == "gray", "FAIL: userdata backward barrier did not gray parent")

collectgarbage("restart")
assert(anchor ~= nil and x ~= nil, "FAIL: barrier canary payload corrupted")
print("PASS canary_j")
