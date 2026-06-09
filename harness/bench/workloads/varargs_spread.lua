--[[
varargs_spread.lua — vararg pack/spread: `...` forwarding, select('#'),
select(n), and table.pack/unpack round trips.

Deterministic: checksum over spread sums.
]]

local function sum3(a, b, c)
    return a + (b or 0) + (c or 0)
end

local function forward(...)
    return sum3(...)
end

local function count(...)
    return select("#", ...)
end

local function tail(...)
    return select(2, ...)
end

local iterations = 2000000
local acc = 0
for i = 1, iterations do
    acc = acc + forward(i, 2, 3)
    acc = acc + count(1, 2, 3, 4)
    local b, c = tail(i, i + 1, i + 2)
    acc = acc + b + c
    if i % 1000 == 0 then
        local t = table.pack(i, i + 1, i + 2)
        acc = acc + sum3(table.unpack(t, 1, t.n))
    end
end

io.write("varargs_spread.lua OK: acc=", acc, "\n")
