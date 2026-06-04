--[[
compare_immediates.lua - comparisons against integer and string constants.

Measures: immediate/K comparison codegen and VM execution for <, <=, ==, ~=,
and string equality in a branch-heavy loop.

Workload: classify loop counters with constant comparisons and fold the branch
results into a deterministic score.
]]

local iterations = 20000000
local labels = { "alpha", "beta", "gamma", "delta" }
local score = 0

for i = 1, iterations do
    local r = i % 64

    if r < 17 then score = score + 1 end
    if r <= 31 then score = score + 3 end
    if r == 42 then score = score + 5 end
    if r ~= 9 then score = score + 7 end
    if r > 50 then score = score + 11 end
    if r >= 60 then score = score + 13 end

    local label = labels[(r & 3) + 1]
    if label == "gamma" then score = score + 17 end
    if label ~= "alpha" then score = score + 19 end
end

assert(score == 605625000,
       "compare_immediates checksum mismatch: got " .. score)
io.write("compare_immediates.lua OK: ", score, "\n")
