-- testC finalizer cohort telemetry. This pins the current finalizer registry's
-- age split so the next collector-owned finobj/tobefnz work has a fast signal.

assert(T and T.gcstats and T.gcage and T.checkmemory,
       "FAIL: testC finalizer telemetry helpers missing")

local function statnum(stats, name)
  local value = stats:match(name .. "=(%-?%d+)")
  assert(value, "FAIL: missing gcstats field " .. name .. " in: " .. stats)
  return tonumber(value)
end

collectgarbage("generational")

local baseline = T.gcstats()
local old_base = statnum(baseline, "pendingfinold")
local young_base = statnum(baseline, "pendingfinyoung")
local ran = 0
local mt = { __gc = function() ran = ran + 1 end }

local old = setmetatable({}, mt)
collectgarbage("collect")
assert(T.gcage(old) == "old", "FAIL: rooted finalizer object did not age old")

local after_old = T.gcstats()
assert(statnum(after_old, "pendingfinold") > old_base,
       "FAIL: old finalizer object not counted in pending old cohort")

local young = setmetatable({}, mt)
assert(T.gcage(young) == "new", "FAIL: new finalizer object did not start new")

local after_young = T.gcstats()
assert(statnum(after_young, "pendingfinyoung") > young_base,
       "FAIL: young finalizer object not counted in pending young cohort")

young = nil
collectgarbage("step", 0)

local after_minor = T.gcstats()
assert(statnum(after_minor, "pendingfinold") >= statnum(after_old, "pendingfinold"),
       "FAIL: minor step moved rooted old finalizer out of pending cohort")

old.keepalive = true
T.checkmemory()
print("METRIC finalizercohorts " .. after_minor)
print("PASS canary_m")
