-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

print('testing incremental garbage collection')

local debug = require"debug"

assert(collectgarbage("isrunning"))

collectgarbage()

local oldmode = collectgarbage("incremental")

assert(collectgarbage("generational") == "incremental")
assert(collectgarbage("generational") == "generational")
assert(collectgarbage("incremental") == "generational")
assert(collectgarbage("incremental") == "incremental")


local function nop () end

local function gcinfo ()
  return collectgarbage"count" * 1024
end


do
  local a = collectgarbage("setpause", 200)
  local b = collectgarbage("setstepmul", 200)
  local t = {0, 2, 10, 90, 500, 5000, 30000, 0x7ffffffe}
  for i = 1, #t do
    local p = t[i]
    for j = 1, #t do
      local m = t[j]
      collectgarbage("setpause", p)
      collectgarbage("setstepmul", m)
      collectgarbage("step", 0)
      collectgarbage("step", 10000)
    end
  end
  collectgarbage("setpause", a)
  collectgarbage("setstepmul", b)
  collectgarbage()
end


_G["while"] = 234


local function GC1 ()
  local u
  local b
  local finish = false
  u = setmetatable({}, {__gc = function () finish = true end})
  b = {34}
  print("GC1.1 loop start")
  repeat u = {} until finish
  print("GC1.1 done")
  assert(b[1] == 34)

  finish = false; local i = 1
  u = setmetatable({}, {__gc = function () finish = true end})
  print("GC1.2 loop start")
  repeat i = i + 1; u = tostring(i) .. tostring(i) until finish
  print("GC1.2 done")
  assert(b[1] == 34)

  finish = false
  u = setmetatable({}, {__gc = function () finish = true end})
  print("GC1.3 loop start")
  repeat local i; u = function () return i end until finish
  print("GC1.3 done")
  assert(b[1] == 34)
end

local function GC2 ()
  local u
  local finish = false
  u = {setmetatable({}, {__gc = function () finish = true end})}
  local b = {34}
  print("GC2.1 loop start")
  repeat u = {{}} until finish
  print("GC2.1 done")
  assert(b[1] == 34)

  finish = false; local i = 1
  u = {setmetatable({}, {__gc = function () finish = true end})}
  print("GC2.2 loop start")
  repeat i = i + 1; u = {tostring(i) .. tostring(i)} until finish
  print("GC2.2 done")
  assert(b[1] == 34)

  finish = false
  u = {setmetatable({}, {__gc = function () finish = true end})}
  print("GC2.3 loop start")
  repeat local i; u = {function () return i end} until finish
  print("GC2.3 done")
  assert(b[1] == 34)
end

local function GC()  GC1(); GC2() end


do
  print("creating many objects")
  print("before loop1")
  local limit = 5000

  for i = 1, limit do
    local a = {}; a = nil
  end
  print("after loop1")

  local a = "a"

  for i = 1, limit do
    a = i .. "b";
    a = string.gsub(a, '(%d%d*)', "%1 %1")
    a = "a"
  end
  print("after loop2")

  a = {}

  function a:test ()
    for i = 1, limit do
      load(string.format("function temp(a) return 'a%d' end", i), "")()
      assert(temp() == string.format('a%d', i))
    end
  end

  a:test()
  print("after test")
  _G.temp = nil
end
print("end")
