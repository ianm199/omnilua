-- Repeated short-string field writes to an existing hash slot.

local iterations = 20000000
local t = { n = 0 }

for i = iterations, 1, -1 do
    t.n = i
end

assert(t.n == 1, "table_setfield_same checksum mismatch")
io.write("table_setfield_same.lua OK: ", t.n, "\n")
