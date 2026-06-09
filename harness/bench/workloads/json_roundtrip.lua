--[[
json_roundtrip.lua — macro workload: a self-contained pure-Lua JSON
encoder/decoder round-tripping a nested document. Exercises the mixed
opcode profile of real programs: string scanning, table construction,
method-free dispatch, concat, pcall-free recursion.

Deterministic: asserts round-trip fidelity and checksums encoded length.
]]

local function encode(v, out)
    local t = type(v)
    if t == "number" then
        out[#out + 1] = string.format("%.17g", v)
    elseif t == "string" then
        out[#out + 1] = '"' .. v:gsub('[%c"\\]', function(c)
            return string.format("\\u%04x", c:byte())
        end) .. '"'
    elseif t == "boolean" then
        out[#out + 1] = v and "true" or "false"
    elseif t == "table" then
        if #v > 0 then
            out[#out + 1] = "["
            for i = 1, #v do
                if i > 1 then out[#out + 1] = "," end
                encode(v[i], out)
            end
            out[#out + 1] = "]"
        else
            out[#out + 1] = "{"
            local keys = {}
            for k in pairs(v) do keys[#keys + 1] = k end
            table.sort(keys)
            for i, k in ipairs(keys) do
                if i > 1 then out[#out + 1] = "," end
                out[#out + 1] = '"' .. k .. '":'
                encode(v[k], out)
            end
            out[#out + 1] = "}"
        end
    else
        out[#out + 1] = "null"
    end
end

local decode
local function skip_ws(s, i)
    return s:find("[^ \t\r\n]", i) or #s + 1
end

local function decode_value(s, i)
    i = skip_ws(s, i)
    local c = s:sub(i, i)
    if c == "{" then
        local obj = {}
        i = i + 1
        i = skip_ws(s, i)
        if s:sub(i, i) == "}" then return obj, i + 1 end
        while true do
            local _, e, k = s:find('^"([^"]*)"', skip_ws(s, i))
            i = skip_ws(s, e + 1) + 1
            local v
            v, i = decode_value(s, i)
            obj[k] = v
            i = skip_ws(s, i)
            local sep = s:sub(i, i)
            i = i + 1
            if sep == "}" then return obj, i end
        end
    elseif c == "[" then
        local arr = {}
        i = i + 1
        i = skip_ws(s, i)
        if s:sub(i, i) == "]" then return arr, i + 1 end
        while true do
            local v
            v, i = decode_value(s, i)
            arr[#arr + 1] = v
            i = skip_ws(s, i)
            local sep = s:sub(i, i)
            i = i + 1
            if sep == "]" then return arr, i end
        end
    elseif c == '"' then
        local _, e, str = s:find('^"([^"]*)"', i)
        return str, e + 1
    elseif c == "t" then
        return true, i + 4
    elseif c == "f" then
        return false, i + 5
    elseif c == "n" then
        return nil, i + 4
    else
        local _, e, num = s:find("^(-?[%d.eE+-]+)", i)
        return tonumber(num), e + 1
    end
end

decode = function(s)
    local v = decode_value(s, 1)
    return v
end

local doc = {
    name = "lua-rs bench",
    version = 54,
    tags = { "vm", "gc", "stdlib", "parse" },
    nested = {
        depth = { a = 1, b = 2.5, c = { 10, 20, 30 } },
        flags = { true, false, true },
    },
    items = {},
}
for i = 1, 40 do
    doc.items[i] = { id = i, score = i * 1.5, label = "item_" .. i }
end

local rounds = 1500
local len_acc = 0
for _ = 1, rounds do
    local out = {}
    encode(doc, out)
    local s = table.concat(out)
    len_acc = len_acc + #s
    local back = decode(s)
    assert(back.name == doc.name, "name round trip")
    assert(#back.items == 40, "items round trip")
    assert(back.items[7].label == "item_7", "label round trip")
    assert(back.nested.depth.c[3] == 30, "nested round trip")
end

io.write("json_roundtrip.lua OK: len=", len_acc, "\n")
