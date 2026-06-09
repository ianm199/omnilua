--[[
method_calls.lua — OP_SELF dispatch: obj:method() through a plain method
table, the hottest call shape in OO-style Lua code.

Deterministic: checksum over accumulated counter.
]]

local Counter = {}
Counter.__index = Counter

function Counter.new()
    return setmetatable({ n = 0 }, Counter)
end

function Counter:bump(k)
    self.n = self.n + k
    return self.n
end

function Counter:get()
    return self.n
end

local iterations = 6000000
local c = Counter.new()
local acc = 0
for i = 1, iterations do
    c:bump(1)
    if i % 1000 == 0 then
        acc = acc + c:get()
    end
end

assert(c:get() == iterations, "method_calls count mismatch")
io.write("method_calls.lua OK: n=", c:get(), " acc=", acc, "\n")
