--[[
sort_seeded.lua — table.sort throughput: numeric default comparator and a
custom comparator over deterministic pseudo-random input (inline LCG so
the sequence is identical on every Lua implementation).

Deterministic: checksums sampled elements after each sort.
]]

local function lcg(seed)
    local state = seed
    return function()
        state = (state * 1103515245 + 12345) % 2147483648
        return state
    end
end

local n = 100000
local rounds = 12
local acc = 0
for r = 1, rounds do
    local rnd = lcg(r)
    local arr = {}
    for i = 1, n do arr[i] = rnd() end
    table.sort(arr)
    acc = acc + arr[1] % 1000 + arr[n] % 1000 + arr[n // 2] % 1000
    if r % 3 == 0 then
        table.sort(arr, function(a, b) return a > b end)
        acc = acc + arr[1] % 1000
    end
end

io.write("sort_seeded.lua OK: acc=", acc, "\n")
