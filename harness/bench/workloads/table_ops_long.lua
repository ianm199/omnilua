--[[
table_ops_long.lua — same shape as table_ops.lua but scaled so reference
C Lua runs ~3-5s of wall and /usr/bin/sample can capture meaningful stacks.

Workload: build a 100,000-element array, do 5,000 random-position removes
and inserts, ipairs traverse, then pairs over a 10,000-key hash table.

Deterministic. Checksums multiply linearly from the smaller workload's
verified values.
]]

local arr = {}
for i = 1, 100000 do arr[i] = i end

local removed_sum = 0
math.randomseed(42)
for _ = 1, 5000 do
    local pos = math.random(1, #arr)
    removed_sum = removed_sum + table.remove(arr, pos)
end

for i = 1, 5000 do
    table.insert(arr, math.random(1, 10), i * 2)
end

local ipairs_sum = 0
for _, v in ipairs(arr) do ipairs_sum = ipairs_sum + v end

local hash = {}
for i = 1, 10000 do hash["key_" .. i] = i * 3 end
local pairs_sum = 0
for _, v in pairs(hash) do pairs_sum = pairs_sum + v end

assert(pairs_sum == 150015000,
       "table_ops_long pairs_sum mismatch: got " .. pairs_sum)
io.write("table_ops_long.lua OK: ipairs_sum=", ipairs_sum,
         " pairs_sum=", pairs_sum,
         " removed_sum=", removed_sum, "\n")
