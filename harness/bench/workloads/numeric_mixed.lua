--[[
numeric_mixed.lua — tight integer arithmetic loop from GitHub issue #134.

Measures: numeric for-loop dispatch, integer add/mul/sub throughput, and
compiler use of immediate/K arithmetic opcodes versus generic register ops.

Workload: 50M iterations of:
  x = i + 5
  y = x * 2
  z = y - 3

This is a matrix-sized variant of the 100M-iteration issue reproducer. The
final value is deterministic: z = iterations * 2 + 7.
]]

local iterations = 50000000
local x, y, z = 0, 0, 0

for i = 1, iterations do
    x = i + 5
    y = x * 2
    z = y - 3
end

local expected = iterations * 2 + 7
assert(z == expected, "numeric_mixed checksum mismatch: got " .. z)
io.write("numeric_mixed.lua OK: ", z, "\n")
