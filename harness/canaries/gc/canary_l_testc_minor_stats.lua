-- testC minor-collection telemetry. This keeps each canary run honest about
-- whether generational steps are doing real mark work and how much of that
-- work scans old vs young objects.

assert(T and T.gcstats and T.gcage and T.checkmemory,
       "FAIL: testC GC stats helpers missing")

local function statnum(stats, name)
  local value = stats:match(name .. "=(%-?%d+)")
  assert(value, "FAIL: missing gcstats field " .. name .. " in: " .. stats)
  return tonumber(value)
end

collectgarbage("generational")

local root = {}
collectgarbage("collect")
assert(T.gcage(root) == "old", "FAIL: telemetry root did not become old")

local before = statnum(T.gcstats(), "collections")
root.child = { payload = string.rep("x", 1024) }
assert(T.gcage(root) == "touched1", "FAIL: telemetry root was not touched")

collectgarbage("step", 0)

local stats = T.gcstats()
assert(statnum(stats, "collections") > before, "FAIL: minor step did not run")
assert(statnum(stats, "marked") > 0, "FAIL: minor step marked no objects")
assert(statnum(stats, "traced") > 0, "FAIL: minor step traced no objects")
assert(statnum(stats, "tracedold") > 0, "FAIL: minor step recorded no old scan work")
assert(statnum(stats, "tracedyoung") > 0, "FAIL: minor step recorded no young scan work")
assert(statnum(stats, "sweepvisited") > 0, "FAIL: minor step swept no objects")
assert(statnum(stats, "sweepvisitedold") == 0, "FAIL: minor sweep walked old tail")
assert(statnum(stats, "sweeprevisit") > 0, "FAIL: minor step skipped touched revisit work")
assert(root.child.payload:len() == 1024, "FAIL: telemetry payload corrupted")

T.checkmemory()
print("METRIC minorstats " .. stats)
print("PASS canary_l")
