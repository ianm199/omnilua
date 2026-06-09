--[[
string_format_mixed.lua — string.format with mixed specifiers plus
number-to-string conversion, the hot path of logging/serialization code.

Deterministic: checksums total formatted length.
]]

local iterations = 400000
local len_acc = 0
for i = 1, iterations do
    local s1 = string.format("%d:%s:%x", i, "k", i % 4096)
    local s2 = string.format("%.3f|%5d|%-4s", i / 7, i % 100000, "ab")
    local s3 = tostring(i * 3)
    len_acc = len_acc + #s1 + #s2 + #s3
end

io.write("string_format_mixed.lua OK: len=", len_acc, "\n")
