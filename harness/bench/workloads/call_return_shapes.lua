--[[
call_return_shapes.lua - small function calls with fixed return shapes.

Measures: Lua call frame setup, one-result returns, multi-result adjustment,
and tail-call dispatch in a tight deterministic loop.

Workload: exercise empty, one-return, two-return, and tail-call paths in a
single loop while folding all results into a checksum.
]]

local iterations = 8000000

local function no_result(x)
    if x == -1 then return x end
end

local function one_result(x)
    return x + 1
end

local function two_results(x)
    return x, x + 2
end

local function tail_target(x)
    return x + 3
end

local function tail_call(x)
    return tail_target(x)
end

local sum = 0
for i = 1, iterations do
    no_result(i)
    local a = one_result(i)
    local b, c = two_results(a)
    local d = tail_call(c)
    sum = sum + d - b
end

assert(sum == 40000000,
       "call_return_shapes checksum mismatch: got " .. sum)
io.write("call_return_shapes.lua OK: ", sum, "\n")
