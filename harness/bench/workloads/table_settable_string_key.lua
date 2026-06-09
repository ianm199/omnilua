-- Repeated variable short-string key writes to an existing hash slot.

local iterations = 20000000
local t = { n = 0 }
local k = "n"

for i = iterations, 1, -1 do
    t[k] = i
end

assert(t.n == 1, "table_settable_string_key checksum mismatch")
io.write("table_settable_string_key.lua OK: ", t.n, "\n")
