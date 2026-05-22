-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

local function errmsg (code, m)
  local st, msg = load(code)
  assert(not st and string.find(msg, m))
end

-- cannot see label inside block
errmsg([[ goto l1; do ::l1:: end ]], "label 'l1'")
errmsg([[ do ::l1:: end goto l1; ]], "label 'l1'")

-- repeated label
errmsg([[ ::l1:: ::l1:: ]], "label 'l1'")
errmsg([[ ::l1:: do ::l1:: end]], "label 'l1'")


-- undefined label
errmsg([[ goto l1; local aa ::l1:: ::l2:: print(3) ]], "local 'aa'")

-- jumping over variable definition
errmsg([[
do local bb, cc; goto l1; end
local aa
::l1:: print(3)
]], "local 'aa'")

-- jumping into a block
errmsg([[ do ::l1:: end goto l1 ]], "label 'l1'")
errmsg([[ goto l1 do ::l1:: end ]], "label 'l1'")

-- cannot continue a repeat-until with variables
errmsg([[
  repeat
    if x then goto cont end
    local xuxu = 10
    ::cont::
  until xuxu < x
]], "local 'xuxu'")
print("part1 ok")
