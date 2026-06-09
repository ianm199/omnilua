--[[
coroutine_pingpong.lua — resume/yield switch cost: a producer coroutine
yielding values to a consumer loop, plus periodic coroutine creation.

Deterministic: sums yielded values.
]]

local function producer(n)
    return coroutine.create(function()
        for i = 1, n do
            coroutine.yield(i)
        end
        return -1
    end)
end

local batches = 400
local per_batch = 2000
local sum = 0
for _ = 1, batches do
    local co = producer(per_batch)
    while true do
        local ok, v = coroutine.resume(co)
        assert(ok, v)
        if v == -1 then break end
        sum = sum + v
    end
end

local expected = batches * (per_batch * (per_batch + 1) / 2)
assert(sum == expected, "coroutine_pingpong sum mismatch: " .. sum)
io.write("coroutine_pingpong.lua OK: sum=", sum, "\n")
