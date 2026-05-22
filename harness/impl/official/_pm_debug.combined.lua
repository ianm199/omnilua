-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- Minimal repro: simulating pm.lua up to the failing point
print('SETUP')

local function checkerror (msg, f, ...)
  local s, err = pcall(f, ...)
  assert(not s and string.find(err, msg))
end

local function f (s, p)
  local i,e = string.find(s, p)
  if i then return string.sub(s, i, e) end
end

local function PU (p)
  p = string.gsub(p, "(" .. utf8.charpattern .. ")%?", function (c)
    return string.gsub(c, ".", "%0?")
  end)
  p = string.gsub(p, "%.", utf8.charpattern)
  return p
end

local function f1 () end
local function range () end
local abc = "abc"
local function strset () end

-- match pm.lua line 167's local t:
local t = "abç d"

-- pm.lua line 211:
local function dostring (s) return load(s, "")() or "" end

-- pm.lua line 216:
local x = string.gsub("$x=42$", "$([^$]*)%$", function () return "" end)
local _x2 = " assim vai para ALO"

-- pm.lua line 221:
local t = {}
-- pm.lua line 222:
local s = 'a alo jose  joao'
-- pm.lua line 223:
local r = s

-- pm.lua line 230:
local function isbalanced (s)
  return not string.find(string.gsub(s, "%b()", ""), "[()]")
end

-- pm.lua line 239:
local t = {"apple", "orange", "lime"; n=0}

-- pm.lua line 273:
local function rev (s)
  return string.gsub(s, "(.)(.+)", function (c,s1) return rev(s1)..c end)
end

-- pm.lua line 277:
local x = "abcdef"

print('BEFORE rev test')
-- pm.lua line 278:
assert(rev(rev(x)) == x)
print('A278 done')

-- pm.lua line 282:
assert(string.gsub("alo alo", ".", {}) == "alo alo")
print('A282 done')
-- pm.lua line 283:
assert(string.gsub("alo alo", "(.)", {a="AA", l=""}) == "AAo AAo")
print('A283 done')
-- pm.lua line 284:
assert(string.gsub("alo alo", "(.).", {a="AA", l="K"}) == "AAo AAo")
print('A284 done')
-- pm.lua line 285:
assert(string.gsub("alo alo", "((.)(.?))", {al="AA", o=false}) == "AAo AAo")
print('A285 done')
-- pm.lua line 287:
assert(string.gsub("alo alo", "().", {'x','yy','zzz'}) == "xyyzzz alo")
print('A287 done')

t = {}; setmetatable(t, {__index = function (t,s) return string.upper(s) end})
assert(string.gsub("a alo b hi", "%w%w+", t) == "a ALO b HI")
print('A290 done')

print('END')
