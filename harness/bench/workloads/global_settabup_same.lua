-- Repeated global writes through OP_SETTABUP to an existing _ENV slot.

local iterations = 20000000
bench_global_settabup_same = 0

for i = iterations, 1, -1 do
    bench_global_settabup_same = i
end

assert(bench_global_settabup_same == 1, "global_settabup_same checksum mismatch")
bench_global_settabup_same = nil
io.write("global_settabup_same.lua OK: 1\n")
