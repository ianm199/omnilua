-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- minimal repro
local function checknext (a)
  local b = {}
  do local k,v = next(a); while k do b[k] = v; k,v = next(a,k) end end
  for k,v in pairs(b) do assert(a[k] == v) end
  for k,v in pairs(a) do assert(b[k] == v) end
end

checknext{1,x=1,y=2,z=3}
checknext{1,2,x=1,y=2,z=3}
checknext{1,2,3,x=1,y=2,z=3}
checknext{1,2,3,4,x=1,y=2,z=3}
checknext{1,2,3,4,5,x=1,y=2,z=3}
print('checknext OK')

-- test 354+: testing next with all kinds of keys
do
  local a = {
    [1] = 1,
    [1.1] = 2,
    ['x'] = 3,
    [string.rep('x', 1000)] = 4,
    [print] = 5,
    [checknext] = 6,
    [true] = 8,
    [{}] = 10,
  }
  local b = {}; for i = 1, 10 do b[i] = true end
  for k, v in pairs(a) do
    print("k=", k, "v=", v)
    if b[v] then b[v] = nil end
  end
end
print('all kinds OK')
