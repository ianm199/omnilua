-- Repeated integer-index writes to an existing array slot.

local iterations = 20000000
local t = { 0 }

for i = iterations, 1, -1 do
    t[1] = i
end

assert(t[1] == 1, "table_seti_same checksum mismatch")
io.write("table_seti_same.lua OK: ", t[1], "\n")
