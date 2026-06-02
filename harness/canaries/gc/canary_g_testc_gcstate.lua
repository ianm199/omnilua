-- Internal T/testC-style GC state and telemetry checks. This keeps the
-- feedback loop small before running the full official gc.lua file.

assert(T and T.gcstate and T.checkmemory and T.totalmem and T.gcstats,
       "FAIL: internal GC telemetry functions missing")

collectgarbage("incremental")
collectgarbage("stop")

T.gcstate("atomic")
assert(T.gcstate() == "atomic", "FAIL: could not stop at atomic state")
local stats = T.gcstats()
assert(type(stats) == "string" and stats:match("state=atomic"),
       "FAIL: gcstats did not report atomic state")

T.gcstate("sweepallgc")
assert(T.gcstate() == "sweepallgc", "FAIL: could not stop at sweep state")
T.gcstate("sweepfinobj")
assert(T.gcstate() == "sweepfinobj", "FAIL: could not stop at finobj sweep state")
T.gcstate("sweeptobefnz")
assert(T.gcstate() == "sweeptobefnz", "FAIL: could not stop at tobefnz sweep state")
T.gcstate("sweepend")
assert(T.gcstate() == "sweepend", "FAIL: could not stop at sweepend state")
T.gcstate("callfin")
assert(T.gcstate() == "callfin", "FAIL: could not stop at callfin state")
T.gcstate("pause")
assert(T.gcstate() == "pause", "FAIL: could not return to pause state")

collectgarbage("restart")

local tables = T.totalmem("table")
local t = {{}, {}, {}}
assert(T.totalmem("table") == tables + 4, "FAIL: table count telemetry")
assert(t[1] and t[2] and t[3], "FAIL: table payload corrupted")

local functions = T.totalmem("function")
local f = function () return 1 end
assert(T.totalmem("function") == functions + 1, "FAIL: function count telemetry")
assert(f() == 1, "FAIL: function payload corrupted")

local threads = T.totalmem("thread")
local co = coroutine.create(function () return 2 end)
assert(T.totalmem("thread") == threads + 1, "FAIL: thread count telemetry")
local ok, value = coroutine.resume(co)
assert(ok and value == 2, "FAIL: thread payload corrupted")

T.checkmemory()
print("PASS canary_g")
