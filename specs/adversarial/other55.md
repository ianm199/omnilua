# Adversarial: other55 — 5.5 OTHER DELTAS vs lua5.5.0

Category: table.create, for-loop control-var read-only, version string, 5.4->5.5
syntax acceptance (`global` keyword/statement, named vararg tables), utf8.offset
second return, collectgarbage("param",...), float round-trip tostring, error(nil).

Ground truth = `/tmp/lua-refs/bin/lua5.5.0`. All DIFFs reproduced with
`specs/oracle/diff_one.sh 5.5 '<code>'`.

Cases run: 64. MATCH: 18. DIFF: 46. All DIFFs are 5.5-version-fidelity gaps
(FIDELITY) except where noted; one is a CRASH (panic). None classified NOISE.

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `print(type(table.create))` | function | function | MATCH | table.create exists |
| `local t=table.create(5); print(#t, type(t))` | 0 table | 0 table | MATCH | |
| `local t=table.create(3,2); print(#t)` | 0 | 0 | MATCH | |
| `local t=table.create(0); print(#t)` | 0 | 0 | MATCH | |
| `local t=table.create(3.0); print(#t)` | 0 | 0 | MATCH | float-with-int-rep ok |
| `table.create(-1)` | bad argument #1 to 'create' (size out of range) | bad argument #1 to 'create' (out of range) | FIDELITY | msg wording + traceback `field 'create'` vs `function 'table.create'`, missing `[C]: in ?` frame |
| `table.create()` | (number expected, got nil) | (number expected, got no value) | FIDELITY | nil vs "no value" |
| `table.create("x")` | (number expected, got string) | same msg | FIDELITY | only traceback frame label differs (`function 'table.create'` vs `field 'create'`) |
| `print(pcall(table.create, 3.5))` | bad argument #1 (number has no integer representation) | bad argument #1 to 'table.create' (...) | FIDELITY | missing `to 'table.create'` |
| `print(pcall(table.create, 1e9))` | true table | true table | MATCH | |
| `local t=table.create(10); t[1]=5; t[2]=6; print(#t,t[1],t[2])` | (match) | | MATCH | |
| `local t=table.create(0,5); t.a=1; t.b=2; print(t.a,t.b)` | (match) | | MATCH | |
| `print(pcall(table.create))` | ...(number expected, got nil) | ...got no value | FIDELITY | |
| `print(pcall(table.create,"x"))` | (match) | | MATCH | |
| `print(pcall(table.create,-5))` | bad argument #1 to 'create' (size out of range) | ...to 'table.create' (out of range) | FIDELITY | |
| `print(pcall(table.create, math.maxinteger))` | ...'create' (size out of range) | ...'table.create' (out of range) | FIDELITY | |
| `for i=1,3 do i=i+1 end` | (no error, exit 0) | attempt to assign to const variable 'i' | FIDELITY | **for control var not read-only — central 5.5 change** |
| `for k,v in pairs({1,2}) do k=0 end` | (no error) | const 'k' error | FIDELITY | generic-for var not read-only |
| `for k,v in pairs({1,2}) do v=0 end` | (no error) | (no error) | MATCH | second generic-for var IS writable in both |
| `for i=1,3 do local i=99; i=i+1; print(i) end` | (match) | | MATCH | shadowing local is writable |
| `local function f() for i=1,3 do i=2 end end` | (no error) | const 'i' error | FIDELITY | compile-time error inside function |
| `for i=1.0,3.0 do i=0 end` | (no error) | const 'i' error | FIDELITY | float-for var read-only too |
| `for i=1,3 do for j=1,3 do j=0 end end` | (no error) | const 'j' error | FIDELITY | nested |
| `print(pcall(load,"for i=1,3 do i=5 end"))` | true function:ADDR | true nil ...const 'i' | FIDELITY | should be a compile error |
| `for i=10,1,-1 do i=i end` | (no error) | const 'i' error | FIDELITY | even self-assign errors |
| `for _, v in ipairs({1,2,3}) do _=nil end` | (no error) | const '_' error | FIDELITY | `_` first var still const |
| `print(_VERSION)` | Lua 5.5 | Lua 5.5 | MATCH | version string correct |
| `print(_VERSION, type(_VERSION))` | (match) | | MATCH | |
| `global X; X=1; print(X)` | (match) | | MATCH | global stmt basic works |
| `do global Y; Y=1; print(Y) end` | (match) | | MATCH | |
| `global a,b,c; a=1;b=2;c=3; print(a,b,c)` | (match) | | MATCH | multi-name global |
| `global x <const> = 5; print(x)` | (match) | | MATCH | global const init |
| `do global * end; print(1)` | (match) | | MATCH | global * form |
| `X=5; global X=10` | (no error, exit 0) | global 'X' already defined (runtime err) | FIDELITY | redeclare-with-init runtime error missing |
| `global X <close>` | `<close> is not allowed for a global declaration near <eof>` | global variables cannot be to-be-closed | FIDELITY | wrong/diff error message |
| `global function foo() end` | `<name> expected near 'function'` (parse error) | (accepted, exit 0) | FIDELITY | `global function` form not accepted |
| `local global = 1` | **PANIC** lua-lex/src/lib.rs:662 index OOB | (accepted, exit 0) | FIDELITY/CRASH | **`global` as identifier panics the lexer** |
| `global=1` | `<name> expected near '='` | (accepted) | FIDELITY | `global` as assignable name |
| `print(global)` | **PANIC** lib.rs:662 | nil | FIDELITY/CRASH | reading global identifier panics |
| `local t={global=5}; print(t.global)` | **PANIC** lib.rs:662 | 5 | FIDELITY/CRASH | `global` as table key panics |
| `local x = global` | **PANIC** lib.rs:662 | (accepted) | FIDELITY/CRASH | |
| `return global` | **PANIC** lib.rs:662 | (accepted) | FIDELITY/CRASH | |
| `print(type(global))` | **PANIC** lib.rs:662 | nil | FIDELITY/CRASH | |
| `local function f(a, ...t) return a,t[1],#t end print(f(10,20,30))` | `')' expected near 't'` | 10 20 2 | FIDELITY | **named vararg tables not parsed** |
| `local function f(...t) return #t end print(f())` | `')' expected near 't'` | 0 | FIDELITY | |
| `local function f(...t) return #t,t.n end print(f(1,nil,3))` | `')' expected near 't'` | 1 3 | FIDELITY | |
| `local function f(a,...t) return #t end print(f(1))` | `')' expected near 't'` | 0 | FIDELITY | |
| `print(pcall(function() error(nil) end))` | false nil | false `<no error object>` | FIDELITY | nil error not replaced by string |
| `local ok,e=pcall(error,nil); print(ok,e,type(e))` | false nil nil | false `<no error object>` string | FIDELITY | type(e) should be string |
| `print(pcall(error))` | false nil | false `<no error object>` | FIDELITY | |
| `print(pcall(function() error() end))` | false nil | false `<no error object>` | FIDELITY | |
| `local ok,e=pcall(error,nil); print(tostring(e))` | nil | `<no error object>` | FIDELITY | |
| `error(nil)` | `(error object is a nil value)` + traceback | same msg, traceback `global 'error'` vs `function 'error'`, missing `[C]: in ?` | FIDELITY | direct error(nil) top-level traceback frame labels differ |
| `print(utf8.offset("héllo", 2))` | 2 | 2 3 | FIDELITY | **utf8.offset missing 2nd return (end pos)** |
| `local a,b=utf8.offset("abc",2); print(a,b)` | 2 nil | 2 2 | FIDELITY | |
| `print(utf8.offset("abc",1))` | 1 | 1 1 | FIDELITY | |
| `print(utf8.offset("abcd",-1))` | 4 | 4 4 | FIDELITY | |
| `print(utf8.offset("abc",5))` | nil | nil | MATCH | out-of-range single nil ok |
| `print(utf8.offset("héllo",3))` | 4 | 4 4 | FIDELITY | |
| `print(utf8.offset("αβγ",2))` | 3 | 3 4 | FIDELITY | multibyte 2nd return |
| `print(utf8.offset("abc",1,2))` | 2 | 2 2 | FIDELITY | with i arg |
| `print(utf8.offset("héllo",-1,7))` | 6 | 6 6 | FIDELITY | |
| `print(pcall(collectgarbage,"param","pause",200))` | false bad argument #1 (invalid option) | true 250 | FIDELITY | **collectgarbage("param",...) not supported** |
| `print(pcall(collectgarbage,"incremental",100,100,10))` | true incremental | true generational | FIDELITY | mode-name return wrong; 5.5 always reports prior mode "generational" default |
| `print(pcall(collectgarbage,"setpause",200))` | true 50 | false bad argument...(invalid option 'setpause') | FIDELITY | obsolete option should be rejected in 5.5 |
| `print(pcall(collectgarbage,"generational"))` | true incremental | true generational | FIDELITY | prior-mode return string differs |
| `print(type(collectgarbage("count")))` | number | number | MATCH | |
| `print(collectgarbage("isrunning"))` | (match) | | MATCH | |
| `print(0.1, 1/3, 3.14159265358979)` | 0.1 0.33333333333333 3.1415926535898 | 0.1 0.33333333333333331 3.14159265358979 | FIDELITY | **float tostring uses %.14g not 5.5 round-trip %.17g** |
| `print(1/3)` | 0.33333333333333 | 0.33333333333333331 | FIDELITY | |
| `print(math.pi)` | 3.1415926535898 | 3.1415926535897931 | FIDELITY | |
| `print(0.30000000000000004)` | 0.3 | 0.30000000000000004 | FIDELITY | |
| `print(1e100, 2^53, 0.1+0.2, 1e-10)` | 1e+100 9.007199254741e+15 0.3 1e-10 | 1e+100 9007199254740992.0 0.30000000000000004 1e-10 | FIDELITY | int-valued floats print as `N.0` in 5.5 |
| `print(tostring(123456789012345.0))` | 1.2345678901234e+14 | 123456789012345.0 | FIDELITY | |
| `print(2^63, 2^64)` | 9.2233720368548e+18 1.844674407371e+19 | 9.2233720368547758e+18 1.8446744073709552e+19 | FIDELITY | |
| `print(-0.1, 1e-5, 123.0)` | (match) | | MATCH | short values still match |
| `print(string.format("%.17g", 0.1))` | (match) | | MATCH | explicit format ok |
| 5.4 control: `print(0.1,1/3,math.pi)` | (match under 5.4) | | MATCH | confirms float round-trip is a genuine 5.5-only delta |

## Classification summary

All confirmed divergences are 5.5 version-fidelity gaps (FIDELITY). No 5.4
REGRESSIONs found in this category (5.4 float control matched). No NOISE.

## Top 5 confirmed divergences

1. **CRASH: `global` as an identifier panics the lexer.** Any use of the bare
   word `global` outside the new statement form (`local global=1`,
   `print(global)`, `{global=5}`, `local x=global`) aborts with a Rust panic at
   `crates/lua-lex/src/lib.rs:662` ("index out of bounds: the len is 37 but the
   index is 37", exit 101). Reference (LUA_COMPAT_GLOBAL on) treats `global` as a
   normal identifier. Repro: `diff_one.sh 5.5 'print(global)'`. Highest priority:
   a panic, and `global` is a common variable name.

2. **For-loop control variable is not read-only.** The headline 5.5 semantic
   change. `for i=1,3 do i=i+1 end` runs cleanly for us; the reference rejects it
   at compile time with `attempt to assign to const variable 'i'`. Applies to
   numeric (int/float), the first generic-for var, and inside functions / `load`.
   Repro: `diff_one.sh 5.5 'for i=1,3 do i=i+1 end'`.

3. **Named vararg tables (`function f(a, ...t)`) not parsed.** New 5.5 syntax;
   we emit `')' expected near 't'`. Reference binds varargs to table `t`. Repro:
   `diff_one.sh 5.5 'local function f(a, ...t) return a,t[1],#t end print(f(10,20,30))'`.

4. **Float `tostring` uses `%.14g`, not 5.5's round-trip `%.17g`.** Pervasive:
   `1/3`, `math.pi`, `0.1+0.2`, `2^53`, large int-valued floats (`123456789012345.0`
   prints as `1.2345678901234e+14` instead of `123456789012345.0`). Confirmed
   5.5-specific (our 5.4 matches 5.4's `%.14g`). Repro:
   `diff_one.sh 5.5 'print(1/3)'`.

5. **`utf8.offset` missing its new second return value** and **`collectgarbage`
   GC-tuning API not updated.** utf8.offset returns only the start position, not
   the new final-byte position (`utf8.offset("héllo",2)` → `2` vs `2  3`).
   collectgarbage rejects the new `("param",...)` option, still accepts the
   obsolete `"setpause"`, and returns wrong prior-mode strings (`incremental` vs
   `generational`). Repros: `diff_one.sh 5.5 'print(utf8.offset("héllo",2))'` and
   `diff_one.sh 5.5 'print(pcall(collectgarbage,"param","pause",200))'`.
