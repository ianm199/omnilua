--[[
table_field_index.lua - repeated table field and integer-index access.

Measures: GETFIELD/SETFIELD and GETI/SETI codegen/runtime behavior for stable
table shapes without allocation in the hot loop.

Workload: mutate named fields and array slots, then fold them into a checksum.
]]

local iterations = 12000000
local t = {
    x = 1,
    y = 2,
    z = 3,
    [1] = 4,
    [2] = 5,
    [3] = 6,
}

local checksum = 0
for i = 1, iterations do
    t.x = t.x + 1
    t.y = t.x + t.z
    t[1] = t[1] + 2
    t[2] = t[1] + t.y
    checksum = checksum + t.x + t.y + t[1] + t[2]
end

local expected = 504000246000000
assert(checksum == expected,
       "table_field_index checksum mismatch: got " .. checksum)
io.write("table_field_index.lua OK: ", checksum, "\n")
