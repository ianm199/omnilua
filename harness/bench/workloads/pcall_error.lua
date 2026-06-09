--[[
pcall_error.lua — protected-call setup/teardown plus the error throw/catch
path. ok_fn always succeeds (pure pcall overhead); err_fn raises and
unwinds for two thirds of its inputs.

Deterministic: counts successes and failures.
]]

local function ok_fn(x)
    return x + 1
end

local function err_fn(x)
    if x ~= 0 then
        error("boom " .. x)
    end
    return x
end

local iterations = 1200000
local succ, fail, acc = 0, 0, 0
for i = 1, iterations do
    local good, v = pcall(ok_fn, i)
    if good then
        succ = succ + 1
        acc = acc + (v % 7)
    end
    local good2, msg = pcall(err_fn, i % 3)
    if good2 then
        succ = succ + 1
    else
        fail = fail + 1
        if i == 1 then
            assert(msg:find("boom"), "error message shape")
        end
    end
end

assert(succ + fail == 2 * iterations, "pcall_error count mismatch")
io.write("pcall_error.lua OK: succ=", succ, " fail=", fail, " acc=", acc, "\n")
