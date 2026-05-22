-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- simple gotos
local x
do
  local y = 12
  goto l1
  ::l2:: x = x + 1; goto l3
  ::l1:: x = y; goto l2
end
::l3:: ::l3_1:: assert(x == 13)


-- long labels
do
  local prog = [[
  do
    local a = 1
    goto l%sa; a = a + 1
   ::l%sa:: a = a + 10
    goto l%sb; a = a + 2
   ::l%sb:: a = a + 20
    return a
  end
  ]]
  local label = string.rep("0123456789", 40)
  prog = string.format(prog, label, label, label, label)
  assert(assert(load(prog))() == 31)
end


-- ok to jump over local dec. to end of block
do
  goto l1
  local a = 23
  x = a
  ::l1::;
end

while true do
  goto l4
  goto l1
  goto l1
  local x = 45
  ::l1:: ;;;
end
::l4:: assert(x == 13)

if print then
  goto l1
  error("should not be here")
  goto l2
  local x
  ::l1:: ; ::l2:: ;;
else end
print("part2 ok")
