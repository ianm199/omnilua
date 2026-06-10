--[[
loop_only.lua — differential probe (NOT a matrix workload): the bare numeric
for-loop matching the setter rows' shape. Ir(setter_row) - Ir(loop_only)
isolates the per-iteration opcode cost on both interpreters.
]]
local iterations = 20000000
for i = iterations, 1, -1 do end
io.write("loop_only.lua OK\n")
