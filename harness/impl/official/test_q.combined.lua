-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

local x = "\0\1\0023\5\0009"
local q = string.format('%q', x)
print('q=', q)
print('xlen=', #x)
local fmt = string.format('return %q', x)
print('fmt=', fmt)
local r = load(fmt)()
print('rlen=', #r)
print('eq=', r == x)
for i=1,#x do io.write(string.byte(x,i),' ') end print('x bytes')
for i=1,#r do io.write(string.byte(r,i),' ') end print('r bytes')
assert(r == x)
print('passed')
