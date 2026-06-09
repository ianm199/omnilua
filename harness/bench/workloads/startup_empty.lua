--[[
startup_empty.lua — process + runtime initialization only. The measured
wall of this row is the startup constant to mentally subtract from short
workloads; it also feeds the per-iteration instruction budgets
(PERF_PUSH_SPEC P2.2).
]]

io.write("startup_empty.lua OK\n")
