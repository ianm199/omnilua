--[[
loop_variants.lua - numeric, while, repeat, and generic loop variants.

Measures: FORPREP/FORLOOP throughput, while/repeat branch dispatch, and generic
for iterator overhead over a fixed array.

Workload: run equivalent counting work through several loop forms and combine
their totals into a checksum.
]]

local iterations = 12000000
local sum = 0

for i = 1, iterations do
    sum = sum + (i & 7)
end

local j = 1
while j <= iterations do
    sum = sum + (j & 3)
    j = j + 1
end

local k = 1
repeat
    sum = sum + (k & 1)
    k = k + 1
until k > iterations

local values = {}
for i = 1, 256 do
    values[i] = i & 15
end

for _ = 1, 120000 do
    for _, v in ipairs(values) do
        sum = sum + v
    end
end

assert(sum == 296400000,
       "loop_variants checksum mismatch: got " .. sum)
io.write("loop_variants.lua OK: ", sum, "\n")
