-- harness preamble: emulate the globals lua-c testes/all.lua sets
_soft = true
_port = true
_nomsg = true
_U = false
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end

-- to repeat a label in a different function is OK
local function foo ()
  local a = {}
  goto l3

  ::l1:: a[#a + 1] = 1; goto l2;
  ::l2:: a[#a + 1] = 2; goto l5;
  ::l3::
  ::l3a:: a[#a + 1] = 3; goto l1;
  ::l4:: a[#a + 1] = 4; goto l6;
  ::l5:: a[#a + 1] = 5; goto l4;
  ::l6:: assert(a[1] == 3 and a[2] == 1 and a[3] == 2 and
              a[4] == 5 and a[5] == 4)
  if not a[6] then a[6] = true; goto l3a end
end

::l6:: foo()


do
  local x
  ::L1::
  local y
  assert(y == nil)
  y = true
  if x == nil then
    x = 1
    goto L1
  else
    x = x + 1
  end
  assert(x == 2 and y == true)
end

do
  local first = true
  local a = false
  if true then
    goto LBL
    ::loop::
    a = true
    ::LBL::
    if first then
      first = false
      goto loop
    end
  end
  assert(a)
end

do
  goto escape
  ::a:: goto a
  ::b:: goto c
  ::c:: goto b
end
::escape::
print("part3 ok")
