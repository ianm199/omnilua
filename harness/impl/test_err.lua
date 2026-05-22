local function doit (s)
  local f, msg = load(s)
  if not f then return msg end
  local cond, msg = pcall(f)
  return (not cond) and msg
end
print("got:", doit("local a,b,c; (function () a = b+1.1 end)()"))
