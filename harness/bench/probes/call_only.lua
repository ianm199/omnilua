--[[
call_only.lua — differential probe (NOT a matrix workload): bare Lua
call/return + one ADD per iteration over the same loop shape.
]]
local function f(x)
    return x + 1
end
local iterations = 8000000
local acc = 0
for i = iterations, 1, -1 do
    acc = f(acc)
end
assert(acc == iterations, "call_only acc")
io.write("call_only.lua OK: ", acc, "\n")
