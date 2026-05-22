-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- repro of nextvar.lua lines 308-419
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
print('A')

assert(#{} == 0)
assert(#{[-1] = 2} == 0)
for i=0,40 do
  local a = {}
  for j=1,i do a[j]=j end
  assert(#a == i)
end
print('B')

function table.maxn (t)
  local max = 0
  for k in pairs(t) do
    max = (type(k) == 'number') and math.max(max, k) or max
  end
  return max
end

assert(table.maxn{} == 0)
assert(table.maxn{["1000"] = true} == 0)
assert(table.maxn{["1000"] = true, [24.5] = 3} == 24.5)
assert(table.maxn{[1000] = true} == 1000)
table.maxn = nil
print('C')

local a = {}
for i=0,50 do a[2^i] = true end
assert(a[#a])
print('D')

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
    if b[v] then b[v] = nil end
  end
end
print('E')

-- erasing values
local t = {[{1}] = 1, [{2}] = 2, [string.rep("x ", 4)] = 3,
           [100.3] = 4, [4] = 5}

local n = 0
for k, v in pairs( t ) do
  n = n+1
  assert(t[k] == v)
  t[k] = nil
  collectgarbage()
  assert(t[k] == nil)
end
print('F', n)
assert(n == 5)
print('G')
