--[[
concat_chain.lua — OP_CONCAT chains: short multi-operand `..` expressions
(the common string-building shape), bounded pieces so total allocation
stays linear.

Deterministic: checksums total built length.
]]

local iterations = 1500000
local len_acc = 0
local sep = "-"
for i = 1, iterations do
    local k = i % 1000
    local s = "id" .. sep .. k .. sep .. (k + 1) .. sep .. "end"
    len_acc = len_acc + #s
end

io.write("concat_chain.lua OK: len=", len_acc, "\n")
