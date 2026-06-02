-- testC weak-table registry telemetry under generational collection.

assert(T and T.gcstats and T.checkmemory,
       "FAIL: testC weak registry helpers missing")

local function statnum(stats, name)
  local value = stats:match(name .. "=(%-?%d+)")
  assert(value, "FAIL: missing gcstats field " .. name .. " in: " .. stats)
  return tonumber(value)
end

local function count(t)
  local n = 0
  for _ in pairs(t) do n = n + 1 end
  return n
end

collectgarbage("generational")

local wk = setmetatable({}, {__mode = "k"})
local wv = setmetatable({}, {__mode = "v"})
local wa = setmetatable({}, {__mode = "kv"})

do
  local k1, v1 = {}, {}
  local k2, v2 = {}, {}
  wk[k1] = "weak-key"
  wv["weak-value"] = v1
  wa[k2] = v2
end

collectgarbage("step", 0)

local stats = T.gcstats()
assert(statnum(stats, "weak") >= 3,
       "FAIL: weak registry lost rooted weak tables")
assert(statnum(stats, "weaklive") >= 3,
       "FAIL: weak registry did not snapshot rooted weak tables")
assert(statnum(stats, "weakretained") >= 3,
       "FAIL: weak registry did not retain rooted weak tables")
assert(count(wk) == 0, "FAIL: weak-key entry survived")
assert(count(wv) == 0, "FAIL: weak-value entry survived")
assert(count(wa) == 0, "FAIL: all-weak entry survived")

T.checkmemory()
print("METRIC weakregistry " .. stats)
print("PASS canary_n")
