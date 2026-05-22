-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

local function doit (s)
  local f, msg = load(s)
  if not f then return msg end
  local cond, msg = pcall(f)
  return (not cond) and msg
end
print("got:", doit("local a,b,c; (function () a = b+1.1 end)()"))
