--[[
metatable_index_chain.lua — metamethod-PRESENT reads: field lookups that
miss the object and resolve through a 2-level __index chain (classic
inheritance). Every other workload measures the metamethod-absent fast
path; real programs live here.

Deterministic: sums resolved fields.
]]

local Base = { kind = "base", factor = 3 }
local Mid = setmetatable({ label = "mid" }, { __index = Base })
local Leaf = { __index = Mid }

local function make(i)
    return setmetatable({ x = i }, Leaf)
end

local objs = {}
for i = 1, 64 do objs[i] = make(i) end

local iterations = 3000000
local sum = 0
for i = 1, iterations do
    local o = objs[(i % 64) + 1]
    sum = sum + o.x + o.factor
    if o.kind == "base" then sum = sum + 1 end
end

local expected = (iterations / 64) * (64 * 65 / 2) + iterations * 4
assert(sum == expected, "metatable_index_chain sum mismatch: " .. sum)
io.write("metatable_index_chain.lua OK: sum=", sum, "\n")
