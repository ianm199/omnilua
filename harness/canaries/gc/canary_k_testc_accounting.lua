-- Lua-visible memory accounting must be driven by the collector-owned heap
-- byte model. This catches regressions back to simulated collectgarbage("count")
-- observables or payload bytes that never refund after sweep.

assert(T and T.totalmem and T.checkmemory, "FAIL: testC accounting helpers missing")

local function count_bytes()
  return math.floor(collectgarbage("count") * 1024 + 0.5)
end

local function total_bytes()
  local bytes = T.totalmem()
  return bytes
end

local function check_count(label)
  local tracked = total_bytes()
  local counted = count_bytes()
  assert(math.abs(tracked - counted) <= 1,
         "FAIL: count mismatch at " .. label ..
         ": tracked=" .. tracked .. " counted=" .. counted)
end

collectgarbage("collect")
local baseline = total_bytes()
check_count("baseline")

local s = string.rep("a", 512 * 1024)
collectgarbage("collect")
local after_string = total_bytes()
check_count("rooted string")
assert(after_string >= baseline + 256 * 1024,
       "FAIL: long string payload was not charged")

local t = {}
for i = 1, 4096 do
  t[i] = i
end
collectgarbage("collect")
local after_table = total_bytes()
check_count("rooted table")
assert(after_table >= after_string + 16 * 1024,
       "FAIL: table buffer payload was not charged")

local u = T.newuserdata(512 * 1024, 3)
collectgarbage("collect")
local after_userdata = total_bytes()
check_count("rooted userdata")
assert(after_userdata >= after_table + 256 * 1024,
       "FAIL: userdata payload was not charged")

s, t, u = nil, nil, nil
collectgarbage("collect")
collectgarbage("collect")
local final = total_bytes()
check_count("after sweep")
assert(final <= baseline + 64 * 1024,
       "FAIL: payload bytes did not refund after sweep: baseline=" ..
       baseline .. " final=" .. final)

T.checkmemory()
print("PASS canary_k")
