--[[
bitwise_mixed.lua - tight integer bitwise loop with constant operands.

Measures: bitwise arithmetic dispatch and compiler use of constant/immediate
bitwise opcode forms for &, |, ~, <<, and >>.

Workload: update an integer accumulator with a fixed mix of bitwise operations.
The accumulator is masked back to a small range each iteration to avoid relying
on wide integer wraparound behavior.
]]

local iterations = 20000000
local x = 0x12345678
local checksum = 0

for i = 1, iterations do
    x = ((x ~ i) & 0x00ffffff) | 0x10000000
    x = ((x << 3) ~ (x >> 2)) & 0x1fffffff
    checksum = checksum ~ x
end

assert(checksum == 1279193,
       "bitwise_mixed checksum mismatch: got " .. checksum)
io.write("bitwise_mixed.lua OK: ", checksum, "\n")
