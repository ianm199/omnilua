# Multi-version compatibility audit — 5.1–5.5 (2026-06-21)

A measured, oracle-grounded picture of where omniLua actually stands on each
language version, the root causes of every gap, and a prioritized fix queue.
This supersedes prose claims (e.g. the `version.rs` "V51–V55 complete"
doc-comment, which is aspirational).

## TL;DR scorecard

Compatibility = fraction of the official self-checking suite that omniLua passes,
**relative to what the real PUC-Rio reference binary passes under the identical
stock harness** (so `ltests`-dependent files — which even real Lua can't run
without the C debug lib — are excluded from the denominator, not charged to us).

| Version | Suite files | Real Lua passes | **omniLua passes** | Our-bug files | Verdict |
|---|---|---|---|---|---|
| **5.4** | 44 | 43 | **44 (100%)** | 0 | ✅ Reference-grade (port baseline) |
| **5.5** | 34 | 31 | **34 (100%)** | 0 | ✅ Reference-grade (shares the 5.4 core) |
| **5.3** | 28 | 27 | **20 (74%)** | 7 | 🟡 Close — mostly localized gates |
| **5.2** | 27 | 24 | **13 (54%)** | 11 | 🟠 Float-only bridge, many gaps |
| **5.1** | 24 | 20 | **8 (40%)** | 12 | 🔴 The legacy tail (hardest by design) |

Plus **~58 confirmed latent divergences** the suites do not exercise — including
**four real bugs in the "100%" 5.4/5.5 baseline** (see "Cross-version" below).

## Why this gradient — the architecture predicted it

The design is two cores behind one runtime `LuaVersion` flag
(`MULTIVERSION_ARCHITECTURE_DECISION.md`):

- **Modern core {5.3, 5.4, 5.5}** — shared dual `Int|Float` value, arithmetic,
  metamethod dispatch, GC, `_ENV`, embedding API; differences gated at cold-path
  sites. 5.5 = 5.4 + small deltas, so both are at 100%.
- **Legacy core {5.1, 5.2}** — float-only number model (enforced as *behavior*,
  not a `LuaValue` fork). 5.2 is the bridge (float-only + modern `_ENV`); 5.1
  adds the `fenv` globals subsystem and the inert-metamethod flips.

The doc explicitly ranks 5.1 as "the largest single effort — float-only **plus** a
new globals subsystem **plus** the biggest legacy-stdlib surface **plus** the
metamethod flip **plus** the weakest oracle." The measured 5.1<5.2<5.3<5.4≈5.5
ordering is exactly that prediction. **The gaps are coverage holes in the
per-version behavior table, not design flaws.** Almost every fix is a localized
version-gate or a wording delta — not a rewrite.

## The harness lesson (the real root cause of the gap)

5.4 was developed against its full official suite. 5.1/5.2/5.3 only ever had the
hand-curated `specs/oracle/check.sh` battery — which is **circular**: it re-tests
cases someone already fixed (it passes 100% on every version yet misses the
`string.pack`-in-5.1 bug). This audit gave 5.1/5.2/5.3 a real heavyweight oracle
for the first time: all five reference binaries built in `/tmp/lua-refs/bin/`, and
the official **5.1** (`lua5.1-tests`) and **5.2.2** (`lua-5.2.2-tests`) suites
pulled into `reference/extra-tests/` (they were never vendored).

**Action:** wire these suites into `harness/run_official_all.sh` (it already
takes `--version`; add 5.1/5.2 test-dir cases) and make the per-version
differential the CI gate. A version is not "done" until its full official suite
is the gate, not a battery.

## How to reproduce any number here

```bash
# build all five reference binaries (5.4.7/5.5.0 from repo; 5.3.6 built; 5.1.5/5.2.4 downloaded)
bash /tmp/setup_refs.sh                       # -> /tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7,5.5.0}
# per-version official suite, ours vs reference, under identical harness
bash /tmp/diff_suite.sh                        # -> /tmp/compat-report/diff/<ver>.tsv
# one snippet, any version
bash specs/oracle/diff_one.sh 5.1 'print(string.format("%q", 42))'
```

---

## Prioritized fix queue

Grouped by theme and tiered by risk/supervision. Each fix is oracle-gated
(reproduce with `diff_one.sh`, fix impl, confirm MATCH, re-run the affected
official-suite file + the other four versions for no regression). The dominant
file clusters — `string_lib.rs`, `base.rs`, the VM error path, `debug_lib.rs`,
the lexer/parser — are the parallelization axis (one owner per file/subsystem).

### Tier 1 — mechanical version-gates ("modern behavior applied to all versions")
Highest ROI; mostly one-liners. Low risk, autonomous-safe.

- **`string.pack`/`unpack`/`packsize` gated out of 5.1/5.2** — `STRING_LIB` array
  registers them unconditionally (`string_lib.rs:3169-3171`); gate like `gfind`.
- **`table.maxn` missing in 5.1/5.2** (and `table.getn`/`foreach`/`foreachi`).
- **`debug.getfenv`/`setfenv` missing in 5.1; `debug.upvalueid`/`upvaluejoin`/
  `getuservalue`/`setcstacklimit` wrongly present in 5.1** — fix the 5.1 debug roster.
- **`debug.setcstacklimit` present in 5.1/5.2/5.3/5.5** (5.4-only).
- **`_ENV` present in 5.1** (should be nil; 5.1 has no `_ENV`).
- **`string.rep` separator accepted in 5.1** (2-arg only pre-5.2).
- **`debug.upvalueid` out-of-range suppressed for all** → raise on 5.1/5.2/5.3
  (`debug_lib.rs` `check_upval` require_valid gate). [closure.lua 5.2/5.3]
- **`__tostring` must-return-string enforced for all** → 5.1/5.2 have no such check
  (`api.rs to_display_string`).
- **`math.max`/`min` accept strings in 5.1/5.2** → number-only.
- **`coroutine.create`/`wrap` accept C functions in 5.1** → must reject.
- **`collectgarbage("count")` single return in 5.2** → return two values (5.2+).
- **`collectgarbage` option validity per version** — `incremental`/`generational`
  valid only 5.2 (5.3+ must error); 5.2 returns previous-mode integer.
- **`getfenv` default level** (locals.lua 5.1 — `base.rs getfenv_fn`).

### Tier 2 — version-aware error-message + traceback layer
The single biggest cluster (the `errors.lua`/`calls.lua` family + ~14 sweep
divergences). Needs a version parameter on the message/traceback formatters.
Well-bounded by the oracle (capture expected strings from the reference, never
hand-write — feed into `multiversion_oracle.rs`). Autonomous with strict gating.

- **Type-error attribution order**: 5.1/5.2 = `attempt to call local 'x' (a nil
  value)`; 5.3+ = `attempt to call a nil value (local 'x')`. Affects call/index/
  arith/concat/compare wording.
- **`error(number)` + location prefix** converts to string in 5.1/5.2.
- **`stack overflow` missing `file:line:` prefix — ALL versions** (incl. 5.4).
- **Traceback C-function naming**: 5.1 = `?`; 5.2 = short name / `_G.<name>`;
  metamethod naming `__add`/`add`/`metamethod 'add'` differs every version.
- **`'for' initial value must be a number`** checked before limit (5.1/5.2).
- **arithmetic-on-string wording** (`perform arithmetic on a string value`, 5.1/5.2).
- **coroutine.* argerror names — ALL versions** (`to 'coroutine.resume'`).
- **Lexer error 'near' tokens** + `unfinished long string (starting at line N)`
  (5.3+ only) + `cannot use '...' outside a vararg function` location/token.

### Tier 3 — number-model & lexer gates (legacy core)
- **`%x`/`%u`/`%o` negatives**: 5.1 truncates to 0 (C cast), 5.2 errors, 5.3+ differ.
- **`%q`**: 5.1/5.2 string-only (error on nil/bool; number→quoted string); 5.1 uses
  `\000`/`\r`/named escapes; 5.3 `%q` of inf→`inf`.
- **`%s` strict in 5.1** (no `tostring`, no `__tostring`).
- **`\u{}` rejected in 5.2**; **hex-float fractional rejected in 5.1**; **hex
  literal overflow → float in 5.1/5.2** (`0xFFFFFFFFFFFFFFFF`); **str2num int
  detection version-blind** (math.lua 5.2 — `object.rs str2num` must honor FloatOnly).

### Tier 4 — semantics & structural (SUPERVISED — architectural-ish)
- **`gsub` rejects numeric replacement values — ALL versions incl. 5.4** (HIGH,
  `string_lib.rs add_value`). A real correctness bug in the shipped baseline.
- **`%g`/`%e` lose `-0.0` sign — ALL versions.**
- **5.1 metamethod same-reference rule**: `__eq`/`__lt`/`__le` fire only when both
  operands carry the *same* handler function; asymmetric → error.
- **`__index`/`__newindex` loop limit** (`loop in gettable/settable`, 5.1/5.2).
- **`goto` to enclosing-block label doesn't close upvalues** (5.2/5.3, likely all).
- **5.1 implicit `arg` local** for vararg functions (parser; `LUA_COMPAT_VARARG`).
- **Bytecode `string.dump` headers for 5.1/5.2/5.3** (`dump.rs` only emits 5.4/5.5)
  — structural oracle; full per-version format needed (5.1 has no int subtype).
- **5.1 `load` reader pre-drains to EOF** instead of streaming (`state_stub.rs
  load_with_reader`) — affects all versions, masked behind earlier failures.
- **5.2 `_ENV` injection** uses `nupvalues>=1` (5.3+ rule); 5.2 needs `==1`
  (`api.rs:1936`).
- **5.1 `io` `file_result` / `os.execute` raw-status** shape (`io_lib.rs`).

### Known noise (do NOT chase)
Finalizer collection-order within a cycle is nondeterministic — the two
"finalizer order" sweep hits (5.4/5.5) reproduce flakily and MATCH on rerun.
`math.random` C-`rand()` sequence and `os.execute` raw status bytes are
documented host-dependent divergences.

---
## Root-cause inventory (15 failing official-suite files)

### `calls.lua` — versions 5.1, 5.2, 5.3  ·  medium/medium  ·  shared=False
- **First divergence:** Per-version, three independent first-divergences: 5.1 -> calls.lua:250 'assert(not a and type(b) == "string" and i == 2)'; 5.2 -> calls.lua:293 'assert(x() == nil)'; 5.3 -> calls.lua:381 'assert(string.sub(c, 1, #header) == header)'
- **What it tests:** 5.1: load() with a function reader must read lazily and stop pulling from the reader the instant the parser hits a syntax error (reader called exactly twice for source '*a = 123'). 5.2: load(string.dump(f)) of a non-main function with 2 real upvalues must leave its upvalues nil (the global-table/_EN
- **Repro:** `5.1: local i=0; local function read1(x) return function() i=i+1; return string.sub(x,i,i) end end; local a,b=load(read1('*a = 123')); print(not a, i)  -- OURS true 9 / REF true 2 5.2: local a,b=20,30; local f=function(x) if x=='set' then a=10+b;b=b+1 else return a end end; local x=load(string.dump(f`
- **Root cause:** Three independent bugs, one per version. (1) 5.1 reader-drain: crates/lua-stdlib/src/state_stub.rs load_with_reader (~line 1777) eagerly loops the reader to EOF into one Vec<u8> buffer BEFORE calling lua_vm::api::load, instead of streaming reader chunks into the lexer on demand; the reference lexer pulls lazily and stops at the first syntax error, so it reads 2 chunks where we read all 9. (2) 5.2 _ENV injection: crates/lua-vm/src/api.rs load() (line 1936) uses `if !lcl.upvals.is_empty()` (nupvalues >= 1) for ALL versions and overwrites upvalue[0] with the global table. That is the 5.3+ rule. Lua 5.2's lua_load injects the global table only when nupvalues == 1 (exactly one); with 2 upvalues i
- **Location:** 5.1: crates/lua-stdlib/src/state_stub.rs:1777 load_with_reader; 5.2: crates/lua-vm/src/api.rs:1936 load (the `if !lcl.upvals.is_empty()` _ENV injection); 5.3: crates/lua-vm/src/dump.rs:402 DumpState::dump_header (and the version field threading in dump())

### `closure.lua` — versions 5.2, 5.3  ·  quick/medium  ·  shared=True
- **First divergence:** closure.lua:219 (5.2) / closure.lua:222 (5.3): assert(not pcall(debug.upvalueid, foo1, 3))
- **What it tests:** debug.upvalueid with an out-of-range upvalue index (foo1 has only 2 upvalues a,b; index 3 is invalid) must RAISE an arg error so pcall returns false. assert(not pcall(...)) requires the call to fail.
- **Repro:** `local a,b=3,5; local f=function() return a+b end; print(pcall(debug.upvalueid, f, 3))`
- **Root cause:** crates/lua-stdlib/src/debug_lib.rs upvalue_id calls check_upval(state, 1, 2, require_valid=false), unconditionally suppressing the out-of-range error for ALL versions; it hardcodes the Lua 5.4/5.5 semantics. In the C reference, checkupval only skips the 'invalid upvalue index' luaL_argcheck when pnup==NULL: in 5.1/5.2/5.3 (lua-5.3.6/src/ldblib.c checkupval) the check ALWAYS runs for both db_upvalueid and db_upvaluejoin, so out-of-range always raises; in 5.4/5.5 (lua-5.4.7/src/ldblib.c checkupval) db_upvalueid passes pnup=NULL and does NOT raise (pushes fail), while db_upvaluejoin passes &n and still raises. Our single un-gated upvalue_id matches 5.4/5.5 but is wrong for 5.2/5.3. Fix: version
- **Location:** crates/lua-stdlib/src/debug_lib.rs:upvalue_id (helper check_upval; DBLIB registration entry b"upvalueid")

### `errors.lua` — versions 5.1, 5.2, 5.3  ·  medium/medium  ·  shared=False
- **First divergence:** errors.lua:39 (5.1): assert(doit";")  — also 5.2 errors.lua:83 via checkmessage at :20, and 5.3 errors.lua:298 via lineerror at :284
- **What it tests:** 5.1: a bare ';' must be a syntax error (the empty statement does not exist in the 5.1 grammar). 5.2: arithmetic on a string operand must report "perform arithmetic on local 'aaa'" with operand varinfo. 5.3: a multi-line function call a\n(\n23) must attribute its call-time error to the line of the ca
- **Repro:** `5.1: bash specs/oracle/diff_one.sh 5.1 'local f,e=loadstring(";"); print(f and "OK-compiled" or ("ERR:"..tostring(e)))' 5.2: bash specs/oracle/diff_one.sh 5.2 "local ok,m=pcall(function() local aaa='a'; return aaa+1 end); print(m)" 5.3: bash specs/oracle/diff_one.sh 5.3 'local _,m=pcall(load("\na\n(`
- **Root cause:** THREE INDEPENDENT BUGS, one per version (not a shared cause). (1) 5.1 — crates/lua-parse/src/lib.rs:statement() lines 5908-5910 unconditionally accept a leading ';' as an empty statement for ALL versions. In PUC-Rio 5.1 lparser.c statement() has NO case ';' (the empty statement was added in 5.2); a bare ';' falls through to exprstat and raises \"unexpected symbol near ';'\". The ';' arm needs a version gate (>=5.2). (2) 5.2 — crates/lua-vm/src/tagmethods.rs:try_bin_tm() the arith-string-coercion error-wording intercept at line 506 is gated `matches!(lua_version, V53)` only. 5.2 has identical semantics to 5.3 (core-owned arith string coercion, error \"attempt to perform arithmetic on a <type>
- **Location:** 5.1: crates/lua-parse/src/lib.rs:statement (lines 5908-5910); 5.2: crates/lua-vm/src/tagmethods.rs:try_bin_tm (line 506, V53-only gate); 5.3: crates/lua-parse/src/lib.rs:funcargs (line 4568) + suffixedexp (line 4648, must capture+thread start line)

### `gc.lua` — versions 5.1, 5.2, 5.3  ·  medium/high  ·  shared=False
- **First divergence:** 5.1: gc.lua:80 and 5.2: gc.lua:131 -> string.gsub(s, '(%d%d%d%d)', math.sin) then assert(i==20000/4). 5.3: gc.lua:298 inside the __gc finalizer for 'a': assert(C.key == nil) (collectgarbage at gc.lua:303).
- **What it tests:** 5.1/5.2: a gsub replacement FUNCTION whose return value is a NUMBER must be accepted and coerced to a string (math.sin returns a float). 5.3: weak-value table clearing must happen BEFORE finalizable objects are resurrected -- t is reachable only through the about-to-be-finalized table a (a.x = t), s
- **Repro:** `5.1/5.2 (gsub): print(string.gsub("1234", "(%d%d%d%d)", function() return 5 end))  -- REF: 5  1 ; OURS: invalid replacement value (a number). 5.3 (weak/finalizer order): local t={x=10}; local C=setmetatable({key=t},{__mode='v'}); local a=setmetatable({x=t},{__gc=function() print(C.key==nil) end}); a`
- **Root cause:** Two INDEPENDENT bugs. (A) gsub number-replacement [first divergence for 5.1 & 5.2, and shared across ALL versions]: in crates/lua-stdlib/src/string_lib.rs add_value (~line 1530), after calling the replacement function the code does `if state.type_at(-1) != LuaType::String { error 'invalid replacement value' }`. Reference lstrlib.c add_value uses lua_isstring(L,-1), which is true for numbers as well as strings, then coerces via luaL_addvalue/lua_tolstring. Our check is too strict: it rejects LuaType::Number instead of accepting and coercing it. (B) weak-clear-before-finalizer ordering [first divergence for 5.3, also latently present in 5.4/5.5]: in crates/lua-vm/src/state.rs collect_via_heap_
- **Location:** Bug A: crates/lua-stdlib/src/string_lib.rs:add_value (line ~1530). Bug B: crates/lua-vm/src/state.rs:collect_via_heap_mode post-mark hook (lines ~4827-4872); driver/comment in crates/lua-vm/src/api.rs:run_pending_finalizers_limited (lines 1541-1664).

### `db.lua` — versions 5.1, 5.2  ·  quick/high  ·  shared=False
- **First divergence:** db.lua:17 (in test()) assert(l == line, "wrong trace!!"), driven by the first test([[if/math.sin(1)/then...]]) call at db.lua:98-105
- **What it tests:** 5.1: that debug.sethook(f,'l') produces the correct sequence of line events. The first test() call expects the trace {2,4,7} for a multi-line if/then/else. 5.2: separately, db.lua:189 asserts debug.getlocal(1,-i) returns name '(*vararg)' for a vararg slot.
- **Repro:** `5.1: local seen={}; debug.sethook(function(e,l) seen[#seen+1]=l end,'l'); loadstring([[if\nmath.sin(1)\nthen\n  a=1\nelse\n  a=2\nend\n]])(); debug.sethook(); print(table.concat(seen,','))   -- REF chunk lines 2,4,7 ; OURS 2,3,4,7.   5.2: local function foo(a,...) print(debug.getlocal(1,-1)) end foo`
- **Root cause:** Two independent, version-specific bugs (not a shared cause):

(1) 5.1 line-trace bug [the db.lua failure for 5.1]. In crates/lua-parse/src/lib.rs:test_then_block (line ~5384), the gate that decides whether the conditional TEST/JMP is attributed to the condition-expression line vs the `then`-keyword line is `fold_onto_cond = (lua_version == V55)`. The doc-comment claims '5.1-5.4 attribute them to the then-keyword line; 5.5 to the condition line', but that is factually wrong for 5.1: PUC-Rio Lua 5.1 attributes the conditional jump to the CONDITION line (no separate line event for `then`), exactly like 5.5. Only 5.2/5.3/5.4 emit the `then` line. Because we treat 5.1 like 5.2-5.4, a multi-line `
- **Location:** crates/lua-parse/src/lib.rs:test_then_block (5.1 line-trace gate, ~line 5384); crates/lua-vm/src/debug.rs:find_vararg (5.2 vararg name, line 543); compounding panic: crates/lua-vm/src/debug.rs:trace_exec (line 1670) -> crates/lua-vm/src/state.rs:set_saved_pc (line 735)

### `api.rs` — versions 5.1, 5.2  ·  medium/medium  ·  shared=True
- **First divergence:** events.lua:22 — assert(tostring(a) == nil)
- **What it tests:** In 5.1/5.2, when an object's __tostring metamethod returns a non-string value, tostring() returns that value verbatim (no type-check). Here a.name is still nil, so __tostring returns nil and tostring(a) must equal nil. In 5.3+ this same construct is an error, which is why the test only fails on 5.1/
- **Repro:** `local a=setmetatable({},{__tostring=function(x) return x.name end}); print(tostring(a))`
- **Root cause:** to_display_string in crates/lua-vm/src/api.rs unconditionally enforces the 5.3+ rule that __tostring must return a string. In 5.1/5.2 there is no such check; tostring() returns the metamethod's result verbatim (nil/number/table all pass through). The fix is to gate the type-check on lua_version != V51/V52.
- **Location:** crates/lua-vm/src/api.rs:to_display_string (also crates/lua-stdlib/src/auxlib.rs:to_lua_string)

### `nextvar.lua` — versions 5.1, 5.2  ·  medium/medium  ·  shared=True
- **First divergence:** nextvar.lua:349 (5.1) / nextvar.lua:370 (5.2): assert(next(a,nil) == 1000 and next(a,1000) == nil)
- **What it tests:** next() given an existing integer key as the second argument must return the following pair (or nil at end of traversal). Here a={} filled then nil-ed so only key 1000 remains; next(a,1000) must return nil.
- **Repro:** `local a = {}; a[5]=true; print(next(a,5))`
- **Root cause:** find_index (the linear-position lookup behind next) is a faithful port of C 5.4's findindex: `i = if let LuaValue::Int(k) = key { array_index(k) } else { 0 }`, then a hash lookup via get_generic_slot_deadok with NO key normalization. It therefore only accepts an integer-subtype key. In the legacy float-only number model (5.1/5.2, NumberModel::FloatOnly), every integer literal in source -- including the `1000`/`5`/`1` that nextvar.lua passes as next's second argument -- is a LuaValue::Float, never an Int. So: (a) the array-part fast path is skipped (the `else { 0 }` arm makes i=0, and 0u32.wrapping_sub(1)=u32::MAX is not < asize), and (b) get_generic_slot_deadok looks up Float(5.0) directly i
- **Location:** crates/lua-types/src/table.rs:TableInner::find_index (called via next_pair/try_next_pair)

### `pm.lua` — versions 5.1, 5.2  ·  quick/medium  ·  shared=True
- **First divergence:** pm.lua:153 (5.1) / pm.lua:167 (5.2): assert(string.gsub("alo $a=1$ novamente $return a$", "$([^$]*)%$", dostring) == "alo  novamente 1")
- **What it tests:** string.gsub with a function replacement whose return value is a NUMBER (the chunk `return a` returns the number 1). Lua coerces a number replacement value to its string form and substitutes it. The 5.1/5.2 pm.lua use `$a=1$` (number); 5.3/5.4/5.5 pm.lua use `$a='x'$` (string), which is why only 5.1/
- **Repro:** `print(string.gsub("x", ".", function() return 5 end))`
- **Root cause:** In crates/lua-stdlib/src/string_lib.rs, fn add_value (line ~1530), the guard on the function/table replacement result is `if state.type_at(-1) != LuaType::String { error \"invalid replacement value\" }`. This is a strict type check that rejects numbers. The reference C (lstrlib.c add_value) instead uses `lua_isstring(L, -1)`, which returns true for BOTH strings AND numbers (numbers are string-coercible), then calls luaL_addvalue which stringifies the number. Our check should accept numbers too; the existing `state.to_bytes(-1)` call at line 1537 already coerces numbers correctly (via lua_vm::api::to_lua_string == lua_tolstring), so only the type gate at line 1530 is wrong.
- **Location:** crates/lua-stdlib/src/string_lib.rs:add_value (line ~1530, the LuaType::String gate)

### `goto.lua (5.2)` — versions 5.2, 5.3  ·  medium/medium  ·  shared=True
- **First divergence:** 5.2: goto.lua:134  assert(a[1]() == 10 and a[2]() == 20 and a[3] == nil)  |  5.3: goto.lua:180  assert(debug.upvalueid(a[i], 2) ~= debug.upvalueid(a[i + 1], 2))
- **What it tests:** Closing of upvalues across a backward goto loop: a local declared AFTER the target label must get a FRESH upvalue cell on each iteration of the goto loop, so closures captured in different iterations are independent. Both versions exercise the same VM mechanic (5.2 behaviorally, 5.3 via debug.upvalu
- **Repro:** `local t = {} local i = 1\n::l1:: do\n  local x = i\n  t[i] = function() return x end\n  i = i + 1\n  if i <= 2 then goto l1 end\nend\nprint(t[1](), t[2]())   -- REF: 1  2   OURS: 2  2`
- **Root cause:** A backward `goto` whose target label lives in an ENCLOSING block fails to close the upvalues of locals declared after that label, so all goto-loop iterations alias one upvalue cell. In our codegen the close is implemented via a standalone OP_CLOSE opcode (5.4 style) for all versions. For 5.2/5.3, goto resolution is block-scoped (`findlabel_for_goto` only scans the current block), so at `gotostat` time the enclosing-block label is NOT found: the goto takes the pending branch (lib.rs:5012-5014) which emits a bare `cg_jump` with NO close. The goto is later resolved in `leave_block`->`movegotosout` (lib.rs:4021, 3823-3864): that code sets `ls.dyd.gt[i].close = true` (lib.rs:3839/3845) and then c
- **Location:** crates/lua-parse/src/lib.rs : movegotosout / solvegoto (the pending-backward-goto resolution path used by 5.2/5.3); the dead `gt[i].close` flag set at lib.rs:3839/3845 is never turned into an OP_CLOSE. Contrast with gotostat's eager-close branch at lib.rs:5025-5034 used by 5.4/5.5.

### `files.lua (5.3)` — versions 5.1, 5.3  ·  medium/medium  ·  shared=False
- **First divergence:** 5.1: files.lua:28  assert(io.open(file) == nil)  |  5.3: files.lua:415  load(io.lines(file, "L"))() then assert(_G.X == 2) at :416
- **What it tests:** 5.1: io.open of a just-removed (nonexistent) file must return the fail value nil (luaL_fileresult). 5.3: load() must accept the io.lines iterator as its single reader argument; io.lines must return exactly one value so it does not spill extra returns into load's chunkname/mode/env parameters.
- **Repro:** `5.1: print(io.open('/no/such/path/xyz'))  -> OURS: false ... | REF: nil ...   ||   5.3 (arity is the cause): local f=os.tmpname(); local h=io.open(f,'w'); h:write('a\n'); h:close(); print(select('#', io.lines(f)))  -> OURS 4, REF 1`
- **Root cause:** Two INDEPENDENT bugs, one per version. (5.1) crates/lua-stdlib/src/io_lib.rs:file_result (the luaL_fileresult analogue, lines 413-440) pushes LuaValue::Bool(false) on failure at lines 420 and 423; reference C uses luaL_pushfail == lua_pushnil, so the fail value must be nil. The 5.1 test uses the strict `io.open(file) == nil`, which false fails. (5.3) crates/lua-stdlib/src/io_lib.rs:io_lines (lines 1551-1577) unconditionally returns 4 values (iterfn, nil, nil, file) — the Lua 5.4+ to-be-closed generic-for protocol — with no version gate. In 5.1/5.2/5.3 io_lines must return only 1 value. The surplus 4th return (the file handle) spills into load()'s 4th argument (env) when io.lines is passed in
- **Location:** 5.1: crates/lua-stdlib/src/io_lib.rs:file_result (lines 420, 423) — push false instead of nil. 5.3: crates/lua-stdlib/src/io_lib.rs:io_lines (lines 1569-1576) — returns 4 values for all versions instead of 1 for <=5.3.

### `literals.lua` — versions 5.2  ·  medium/medium  ·  shared=True
- **First divergence:** literals.lua:54 inside lexerror(); first triggered by literals.lua:56  lexerror([["abc\x"]], [[\x"]]) -- assert(not st and string.find(msg, "near "..err, 1, true))
- **What it tests:** When loading a string with a malformed escape (e.g. return "abc\x"), compilation must fail AND the syntax-error message must contain the substring  near '\x"'  -- i.e. the `near` token in a string-escape lexer error must be just the offending escape fragment, not the whole string token.
- **Repro:** `local st, msg = load([[return "abc\x"]]); print(st, msg)`
- **Root cause:** In Lua 5.2 specifically, an in-string escape error is raised via escerror() (llex.c), which FIRST calls luaZ_resetbuffer(ls->buff), then saves '\\' followed by only the offending escape chars, so txtToken prints the near-token as just the escape fragment (e.g. '\\x\"', '\\300', '\\g'). Lua 5.3+ changed this mechanism to esccheck()+luaZ_buffremove and reports the full accumulated string buffer instead. Our lexer implements ONLY the 5.3+ model uniformly: esc_check() (lib.rs:917) appends the offending char onto the already-accumulated buffer (delimiter + prior chars + \\ + bad char) without resetting, and txt_token() (lib.rs:557) returns that whole buffer. Result: for 5.2 we emit near '\"abc\\x
- **Location:** crates/lua-lex/src/lib.rs:esc_check (and txt_token); escape-error paths read_hex_esc/read_dec_esc/read_string

### `locals.lua` — versions 5.1  ·  quick/medium  ·  shared=True
- **First divergence:** locals.lua:58-59 — `for i=1,10 do f[i] = function (x) A=A+1; return A, _G.getfenv(x) end end; A=10; assert(f[1]() == 11)`. f[1]() is called with NO argument, so inside the closure x is nil, making the call `_G.getfenv(nil)`.
- **What it tests:** Lua 5.1 fenv semantics: getfenv called with a nil argument must be treated as the default stack level 1 (return the running function's environment, i.e. _G here), exactly like getfenv() with no argument.
- **Repro:** `print(type(getfenv(nil)))`
- **Root cause:** In crates/lua-stdlib/src/base.rs, getfenv_fn (around line 743) matches the first argument and only treats an ABSENT argument as default level 1: the arm `LuaValue::Nil if state.type_at(1) == LuaType::None => fenv_getfunc(state, 1)`. An EXPLICIT nil (where type_at(1) == LuaType::Nil, not None) falls through to the catch-all `other =>` arm, which raises `number expected, got nil`. This is wrong for getfenv: PUC-Rio lbaselib.c's luaB_getfenv calls getfunc(L, 1), and getfunc with opt=1 reads the level via `luaL_optint(L, 1, 1)`, so nil (absent OR explicit) defaults to level 1. The `state.type_at(1) == LuaType::None` guard incorrectly narrows the default-handling to only the absent case. (Note: s
- **Location:** crates/lua-stdlib/src/base.rs:getfenv_fn (the `LuaValue::Nil if state.type_at(1) == LuaType::None` match arm, ~line 747)

### `math.lua` — versions 5.2  ·  medium/medium  ·  shared=True
- **First divergence:** math.lua:60 — assert(tonumber("0x"..string.rep("f", 150)) == 2^(4*150) - 1)
- **What it tests:** tonumber() parsing a hex string whose integer value overflows i64. In the float-only number model (Lua 5.1/5.2) the result must be a double (no integer subtype), and an overflowing hex literal must be parsed as a hex float, yielding 4.149515568881e+180.
- **Repro:** `print(tonumber("0x"..string.rep("f", 150)))`
- **Root cause:** crates/lua-vm/src/object.rs:str2num (and its helper str2int) is version-blind: it always tries str2int first and, on success, returns LuaValue::Int regardless of the number model. In the float-only versions (5.1/5.2) Lua has NO integer subtype — every parsed number must be a double. Two manifestations of this one root cause both hit math.lua:60: (1) str2int succeeds for any integer-shaped string and str2num returns Int, so tonumber/string-coercion of a large integer string prints in integer format instead of %.14g float format (e.g. tonumber('4503599627370495') gives our 4503599627370495 vs ref 4.5035996273705e+15); (2) for the 150-hex-digit string, str2int's hex loop uses wrapping_mul(16) w
- **Location:** crates/lua-vm/src/object.rs:str2num (and str2int hex-overflow branch); missing number-model gate on the stdlib path lua-vm/src/state.rs:str_to_num / lua-vm/src/api.rs:string_to_number, contrast with lua-lex/src/lib.rs:read_numeral which gates via is_float_only

### `string_lib.rs` — versions 5.1  ·  medium/medium  ·  shared=True
- **First divergence:** strings.lua:105 -- assert(string.format('%q', "\0") == [["\000"]])
- **What it tests:** Lua 5.1's string.format('%q', ...) escaping of a NUL byte. In 5.1, %q escapes \0 as the literal 4-char sequence \000 (zero-padded 3-digit decimal); the test asserts the result equals the long-bracket literal [["\000"]].
- **Repro:** `print(string.format("%q", "\0"))`
- **Root cause:** addquoted() in crates/lua-stdlib/src/string_lib.rs (line 1807) implements only the Lua 5.2+ %q escaping rules and is NOT version-gated. Our code: for every is_ascii_control() byte it emits backslash+decimal, zero-padded to 3 digits only when the next byte is a digit (the modern rule). Lua 5.1's addquoted (lstrlib.c:696) uses a different, smaller rule set: double-quote, backslash, newline -> backslash+char; carriage-return -> backslash-r; NUL -> the fixed 4-char string backslash-000 (ALWAYS 3-digit zero-padded, unconditionally); ALL other bytes (including other control bytes like 0x01, tab, 0x7f) are emitted RAW/unescaped. So under 5.1 our function diverges in several ways; the first the test
- **Location:** crates/lua-stdlib/src/string_lib.rs:addquoted (called from addliteral; %q path of string.format)

### `vararg.lua` — versions 5.1  ·  medium/medium  ·  shared=False
- **First divergence:** vararg.lua:6  assert(type(arg) == 'table')  (in function f, first called at vararg.lua:24)
- **What it tests:** Lua 5.1 implicit arg table: a function declared with ... gets a local arg = {[1..n]=extras, n=count}. Test does _G.arg=nil at line 3 first, so it only passes if arg is a true implicit local, not the global.
- **Repro:** `_G.arg = nil function f(a, ...)   return type(arg) end print(f())   --> REF: table   OURS: nil`
- **Root cause:** omniLua does not implement Lua 5.1's LUA_COMPAT_VARARG implicit arg local. In stock 5.1, parlist (lparser.c) creates a local named arg for ... functions (new_localvarliteral arg, flags VARARG_HASARG|VARARG_NEEDSARG) and the runtime adjust_varargs (ldo.c) builds the arg table {[1..nvar]=extras, n=nvar} at call entry when NEEDSARG is set. Our parser parlist TK_DOTS arm (crates/lua-parse/src/lib.rs ~4397-4415) only handles the 5.5 named-vararg / (vararg table) local gated on is_v55; there is no V51 branch creating the arg local, and precall/adjust_varargs has no code to populate it. So inside a ... function, the name arg falls through to a global lookup. Confirmed: with arg=SENTINEL global, REF
- **Location:** crates/lua-parse/src/lib.rs:parlist (TK_DOTS arm) and crates/lua-vm/src/do_.rs:precall + crates/lua-vm/src/vm.rs:adjust_varargs

---

## Confirmed latent divergences (adversarial sweep — bugs the suites miss)

Re-verified through `specs/oracle/diff_one.sh`. **CONFIRMED** = reproduced this run; ✗ = did not reproduce / flaky (treat as noise). Severity from the sweep agent.

| # | Sev | Category | Versions(confirmed) | Snippet | Ours → Ref | Correct behavior |
|---|---|---|---|---|---|---|
| 1 | high | number-model | 5.1,5.2 | `print(string.format("%q", 42))` | `0x1.5p+5` → `"42"` | In 5.1/5.2, string.format("%q") only accepts strings. When given a number, the reference c |
| 2 | high | number-model | 5.1,5.2 | `print(pcall(string.format, "%q", nil))` | `true	nil` → `false	bad argument #2 to '?' (string exp` | In 5.1/5.2, string.format("%q") requires a string argument and must raise an error for nil |
| 3 | high | number-model | 5.1 | `print(string.format("%x", -1))` | `ffffffffffffffff` → `0` | In Lua 5.1 all numbers are doubles. When a negative float is passed to `%x`/`%u`/`%o`, the |
| 4 | high | number-model | 5.2 | `print(string.format("%x", -1))` | `ffffffffffffffff` → `error: bad argument #2 to 'format' (not ` | In Lua 5.2, the integer format specifiers (%x, %u, %o) explicitly reject negative numbers  |
| 5 | medium | number-model | 5.1,5.2 | `print(pcall(math.max, "hello"))` | `true	hello` → `false	bad argument #1 to '?' (number exp` | In 5.1/5.2, math.max and math.min are number-only functions and must reject non-numeric st |
| 6 | high | stdlib-roster | 5.1,5.2 | `print(type(string.pack))` | `function` → `nil` | string.pack/string.unpack/string.packsize were added in 5.3. They must not exist in 5.1 or |
| 7 | high | stdlib-roster | 5.2 | `print(type(table.maxn))` | `nil` → `function` | table.maxn exists in Lua 5.1 and 5.2. It was removed in 5.3. Our 5.2 implementation is mis |
| 8 | high | stdlib-roster | 5.1 | `print(type(debug.getfenv))` | `nil` → `function` | debug.getfenv and debug.setfenv are 5.1-only debug functions (Lua 5.1 has per-function env |
| 9 | medium | stdlib-roster | 5.2,5.3,5.5 | `print(type(debug.setcstacklimit))` | `function` → `nil` | debug.setcstacklimit was added in Lua 5.4 only and should not exist in 5.1, 5.2, 5.3, or 5 |
| 10 | medium | stdlib-roster | 5.1 | `print(type(_ENV))` | `table` → `nil` | _ENV is a 5.2 concept (replacing the per-function environment model of 5.1). It must not b |
| 11 | medium | stdlib-roster | 5.1 | `print(string.rep("ab", 3, ","))` | `ab,ab,ab` → `ababab` | In Lua 5.1, string.rep takes exactly two arguments (string, n). The optional separator arg |
| 12 | medium | stdlib-roster | 5.1,5.2 | `local x = nil x()` | `attempt to call a nil value (local 'x')` → `attempt to call local 'x' (a nil value)` | In Lua 5.1 and 5.2 the error message format for type errors is 'attempt to <op> local/glob |
| 13 | high | stdlib-roster | 5.1 | `local t={} for k in pairs(debug) do t[#t+1]=k end table.sort(t) print(` | `debug,gethook,getinfo,getlocal,getmetata` → `debug,getfenv,gethook,getinfo,getlocal,g` | The 5.1 debug library should contain getfenv and setfenv (5.1-specific) and should NOT con |
| 14 | high | error-messages-traceback | 5.1,5.2 | `local x = nil; x()` | `attempt to call a nil value (local 'x')` → `attempt to call local 'x' (a nil value)` | In Lua 5.1 and 5.2 the attribution clause comes FIRST, in the form 'attempt to call <kind> |
| 15 | high | error-messages-traceback | 5.2 | `local u = nil; local function f() return u() end; f()` | `stack traceback:\n\t(command line):1: in` → `stack traceback:\n\t(command line):1: in` | In Lua 5.2 local functions are shown in tracebacks as 'in function \'name\'' not 'in local |
| 16 | high | error-messages-traceback | 5.2 | `string.rep("x", "notanumber")  -- and all stdlib calls in 5.2` | `[C]: in function 'string.rep'` → `[C]: in function 'rep'` | In Lua 5.2, C functions registered in tables (string.*, table.*, math.*, io.*, debug.*) ar |
| 17 | medium | error-messages-traceback | 5.1 | `local function a() return debug.traceback() end; local function b() re` | `stack traceback:\n\t(command line):1: in` → `stack traceback:\n\t(command line):1: in` | In Lua 5.1, tail-called functions insert a '(tail call): ?' entry in debug.traceback outpu |
| 18 | high | error-messages-traceback | 5.1,5.2 | `local ok, err = pcall(function() error(42) end); print(ok, type(err), ` | `false\tnumber\t42` → `false\tstring\t(command line):1: 42` | In Lua 5.1 and 5.2, error() with a NUMBER argument and a level > 0 converts the number to  |
| 19 | medium | error-messages-traceback | ✗ | `for i = 'a', 'b' do end  (both initial and limit are non-numeric)` | `'for' limit must be a number` → `'for' initial value must be a number` | In Lua 5.1 and 5.2, the numeric for loop checks the INITIAL value first. When the initial  |
| 20 | high | error-messages-traceback | 5.3,5.4,5.5 | `local ok, err = pcall(function()\n  local function f() f() end\n  f()\` | `false\tstack overflow` → `false\t(command line):3: stack overflow` | The 'stack overflow' runtime error should include the file:line location prefix like all o |
| 21 | medium | error-messages-traceback | ✗ | `print("abc" + 1)  (non-numeric string literal in arithmetic)` | `attempt to add a 'string' with a 'number` → `attempt to perform arithmetic on a strin` | In Lua 5.1 and 5.2, when a non-numeric string is used in arithmetic, the error is 'attempt |
| 22 | low | error-messages-traceback | 5.3 | `print("abc" + 1)  (non-numeric string LITERAL in arithmetic, 5.3)` | `attempt to perform arithmetic on a strin` → `attempt to perform arithmetic on a strin` | In Lua 5.3, when a non-numeric string LITERAL (constant) is used in arithmetic, the error  |
| 23 | medium | error-messages-traceback | 5.1 | `local mt = {__add = function(a,b) error('e') end}; local t = setmetata` | `stack traceback:\n\t(command line):1: in` → `stack traceback:\n\t(command line):1: in` | In Lua 5.1, an anonymous function used as a metamethod is shown in the traceback as 'in fu |
| 24 | medium | error-messages-traceback | 5.2 | `local mt = {__add = function(a,b) error('e') end}; local t = setmetata` | `stack traceback:\n\t(command line):1: in` → `stack traceback:\n\t(command line):1: in` | In Lua 5.2, metamethods are shown in tracebacks as 'in function \'__add\'' (with the __ pr |
| 25 | medium | error-messages-traceback | 5.3 | `local mt = {__add = function(a,b) error('e') end}; local t = setmetata` | `stack traceback:\n\t(command line):1: in` → `stack traceback:\n\t(command line):1: in` | In Lua 5.3, metamethods are shown in tracebacks as 'in metamethod \'__add\'' (with the __  |
| 26 | high | metamethods | 5.1,5.2 | `local mt1={__eq=function(a,b) return true end}; local mt2={__eq=functi` | `true` → `false` | In 5.1 and 5.2, __eq only fires if both operands carry the SAME function reference as thei |
| 27 | high | metamethods | 5.1 | `local mt1={__lt=function(a,b) return true end}; local mt2={__lt=functi` | `true` → `PROG: (command line):1: attempt to compa` | In 5.1, __lt (and __le) only fires if both operands carry the SAME function reference as t |
| 28 | high | metamethods | 5.1 | `local mt={__lt=function(a,b) return true end}; local t1=setmetatable({` | `true` → `false` | In 5.1, if only one operand has __lt (regardless of left/right), the comparison must raise |
| 29 | high | metamethods | 5.1 | `local mt={__le=function(a,b) return true end}; local t1=setmetatable({` | `true` → `false` | In 5.1, __le (like __eq and __lt) requires the same function reference on both operands. A |
| 30 | medium | metamethods | 5.1 | `local t=setmetatable({},{__tostring=function() return 'myobj' end}); l` | `true	myobj` → `false	(command line):1: bad argument #2 ` | In Lua 5.1, string.format('%s', ...) requires a string argument; it does NOT call __tostri |
| 31 | medium | metamethods | 5.1,5.2 | `local t = {base=1}; for i=1,100 do t=setmetatable({},{__index=t}) end;` | `1` → `PROG: (command line):1: loop in gettable` | In Lua 5.1 and 5.2, the __index traversal depth is limited (100 hops triggers 'loop in get |
| 32 | medium | metamethods | 5.1,5.2 | `local t = {}; for i=1,100 do t=setmetatable({},{__newindex=t}) end; t.` | `nil` → `PROG: (command line):1: loop in settable` | In Lua 5.1 and 5.2, the __newindex forwarding depth is limited (100 hops triggers 'loop in |
| 33 | low | metamethods | 5.1,5.2 | `local ok,err=pcall(function() local t=nil; return t.x end); print(err)` | `(command line):1: attempt to index a nil` → `(command line):1: attempt to index local` | In Lua 5.1 and 5.2, metamethod-related error messages use the format 'attempt to <op> <loc |
| 34 | high | string-and-patterns | 5.1,5.2,5.3,5.4,5.5 | `print(string.gsub("hello", "l+", function(s) return #s end))` | `PROG: invalid replacement value (a numbe` → `he2o	1` | gsub must accept numbers (integer or float) as replacement values from both function and t |
| 35 | medium | string-and-patterns | 5.1,5.2,5.3,5.4,5.5 | `print(string.format("%g", -0.0))` | `0` → `-0` | Negative zero should preserve its sign when formatted with %g, %G, %e, and %E. Note: %f co |
| 36 | medium | string-and-patterns | 5.3 | `print(string.format("%q", 1/0))` | `1e9999` → `inf` | In Lua 5.3, %q applied to infinity should output 'inf' (and '-inf' for negative infinity,  |
| 37 | medium | string-and-patterns | 5.1 | `print(string.format("%q", "hello\0world"))` | `"hello\0world"` → `"hello\000world"` | In Lua 5.1, %q must escape null bytes as \000 (three-digit decimal escape), not \0. This m |
| 38 | medium | string-and-patterns | 5.1 | `print(string.format("%q", "\r"))` | `"\13"` → `"\r"` | In Lua 5.1, %q must escape carriage return as \r (the named escape), not as \13 (decimal). |
| 39 | medium | string-and-patterns | 5.1 | `print(string.format("%q", "\7\8\14"))` | `"\7\8\14"` → `"\7\8\14" (with literal bytes 0x07 0x08 ` | In Lua 5.1, %q passes through control characters other than NUL, newline, quote, and backs |
| 40 | high | string-and-patterns | 5.1 | `print(string.format("%s", true))` | `true [exit 0]` → `PROG: bad argument #2 to 'format' (strin` | In Lua 5.1, %s strictly requires a string argument and errors on boolean, nil, table, or a |
| 41 | medium | string-and-patterns | 5.1,5.2 | `print(string.format("%u", -1))` | `18446744073709551615 [exit 0]` → `5.1: 0 [exit 0] \| 5.2: error 'not a non-` | 5.1: %u with negative float -1 yields 0 (C library truncation behavior on the platform). 5 |
| 42 | medium | string-and-patterns | 5.1,5.2 | `print(string.format("%x", -1))` | `5.1: ffffffffffffffff [exit 0] \| 5.2: ff` → `5.1: 0 [exit 0] \| 5.2: error 'not a non-` | Same root as %u/%o bug. In 5.1/5.2, hex/octal/unsigned formats treat the input as a floati |
| 43 | high | syntax-lexer-gates | 5.2 | `print("\u{0041}")` | `A↵(exit 0)` → `lua5.2.4: (command line):1: invalid esca` | Lua 5.2 does not support \u{} Unicode escapes; they were introduced in 5.3. The lexer must |
| 44 | high | syntax-lexer-gates | 5.1 | `print(0x1.8p0)` | `1.5↵(exit 0)` → `lua5.1.5: (command line):1: malformed nu` | Lua 5.1 does not support hexadecimal floating-point literals with a fractional part (0x<in |
| 45 | high | syntax-lexer-gates | 5.1,5.2 | `print(0xFFFFFFFFFFFFFFFF)` | `-1↵(exit 0)` → `1.844674407371e+19↵(exit 0)` | In Lua 5.1 and 5.2, all numbers are floats (doubles). A hex literal that exceeds 2^63-1 mu |
| 46 | high | syntax-lexer-gates | 5.5 | `global function f(a) b = a end` | `(empty, exit 0)` → `lua5.5.0: (command line):1: variable 'b'` | In Lua 5.5, using the 'global' declaration keyword activates a strict-variable-declaration |
| 47 | low | syntax-lexer-gates | 5.1,5.2 | `print([[hello)` | `PROG: (command line):1: unfinished long ` → `lua5.1.5: (command line):1: unfinished l` | In Lua 5.1 and 5.2, the error message for an unclosed long string omits the '(starting at  |
| 48 | low | syntax-lexer-gates | 5.2 | `print("\j")` | `PROG: (command line):1: invalid escape s` → `lua5.2.4: (command line):1: invalid esca` | In Lua 5.2, the 'near' token in invalid-escape-sequence errors should show just the escape |
| 49 | low | syntax-lexer-gates | 5.1 | `print("\256")` | `PROG: (command line):1: escape sequence ` → `lua5.1.5: (command line):1: escape seque` | In Lua 5.1, the 'near' token for 'escape sequence too large' should be just the opening qu |
| 50 | high | closure-vararg-coroutine | 5.1 | `local function f(...) return arg end; local t = f(10,20); print(t[1], ` | `-e	nil` → `10	20	2` | In Lua 5.1, every vararg function body gets an implicit LOCAL named `arg` containing a tab |
| 51 | medium | closure-vararg-coroutine | 5.1,5.2 | `print(type(arg))` | `table` → `nil` | When Lua is invoked with the -e flag the standalone interpreter does NOT set the global `a |
| 52 | low | closure-vararg-coroutine | 5.1,5.2,5.3,5.4,5.5 | `local function f() return ... end` | `PROG: cannot use '...' outside a vararg ` → `PROG: (command line):1: cannot use '...'` | The compile-time error for using `...` in a non-vararg function must include the source lo |
| 53 | high | closure-vararg-coroutine | 5.1 | `local ok,e = pcall(coroutine.create, print); print(ok, e)` | `true	thread: ADDR` → `false	bad argument #1 to '?' (Lua functi` | In Lua 5.1, `coroutine.create` and `coroutine.wrap` only accept Lua (non-C) functions. Pas |
| 54 | medium | closure-vararg-coroutine | 5.1,5.2,5.3,5.4,5.5 | `local ok,e = pcall(coroutine.resume, 'not_a_coroutine'); print(ok,e)` | `false	bad argument #1 (thread expected, ` → `false	bad argument #1 to 'coroutine.resu` | Error messages from coroutine library functions must include the function name via `luaL_a |
| 55 | high | gc-collectgarbage | 5.2 | `local a,b = collectgarbage("count"); print(type(a), type(b))` | `number	nil` → `number	number` | Since Lua 5.2 (not just 5.3+), collectgarbage("count") returns TWO values: total memory in |
| 56 | high | gc-collectgarbage | 5.2 | `print(type(collectgarbage("incremental")), collectgarbage("incremental` | `string	incremental` → `number	0` | In Lua 5.2, collectgarbage("incremental") is a valid option that switches the GC to increm |
| 57 | high | gc-collectgarbage | 5.3 | `local ok, err = pcall(collectgarbage, "incremental"); print(ok, err an` | `true	nil` → `false	invalid` | Lua 5.3 does NOT support collectgarbage("incremental") — it is an invalid option that must |
| 58 | high | gc-collectgarbage | 5.3 | `local ok, err = pcall(collectgarbage, "generational"); print(ok, err a` | `true	nil` → `false	invalid` | Lua 5.3 does NOT support collectgarbage("generational") — same as "incremental", it was ex |
| 59 | medium | gc-collectgarbage | 5.1 | `local ok, err = pcall(collectgarbage, "badopt"); print(err)` | `bad argument #1 to 'collectgarbage' (inv` → `bad argument #1 to '?' (invalid option '` | In Lua 5.1, C functions appear as '?' in error messages because 5.1 did not record C funct |
| 60 | medium | gc-collectgarbage | ✗ | `local ok, err = pcall(collectgarbage, "badopt"); print(err)` | `bad argument #1 to 'collectgarbage' (inv` → `bad argument #1 to '_G.collectgarbage' (` | In Lua 5.2, global functions appear qualified as '_G.<name>' in error messages. omniLua us |
| 61 | high | gc-collectgarbage | 5.4 | `local order = {}; local mt = {__gc = function(t) table.insert(order, t` | `4,3,2,1,5` → `5,4,3,2,1` | Finalizers must be called in reverse allocation order (last allocated = first finalized).  |
| 62 | high | gc-collectgarbage | 5.5 | `local order = {}; local mt = {__gc = function(t) table.insert(order, t` | `1,3,2` → `3,2,1` | In Lua 5.5 (generational GC), finalizers must also be called in reverse allocation order w |
| 63 | medium | gc-collectgarbage | 5.2,5.3 | `local mt = {__gc = function(t) error("gc error") end}; local obj = set` | `PROG: error in __gc metamethod ((command` → `PROG: error in __gc metamethod ((command` | In Lua 5.2 and 5.3, when a __gc finalizer raises an error, the runtime prints just the err |
---

## Results — autonomous fix campaign (2026-06-21)

Four file-disjoint worktree agents (string / stdlib / frontend / vm) fixed the
Tier 1–3 + safe Tier-4 queue, each oracle-gated, merged into
`fix/multiversion-compat`. **~31 distinct bugs fixed, all confirmed MATCH vs the
reference; zero 5.4/5.5 suite regression.**

| Version | our-bug files before → after | file pass-rate before → after |
|---|---|---|
| 5.1 | 12 → 10 | 8/20 (40%) → 11/21 (52%) |
| 5.2 | 11 → 7  | 13/24 (54%) → 17/24 (71%) |
| 5.3 | 7 → 6   | 20/27 (74%) → 21/27 (78%) |
| 5.4 | 0 → 0   | 100% (no regression) |
| 5.5 | 0 → 0   | 100% (no regression) |

**Baseline hardening:** four latent bugs that also affected the shipped 5.4/5.5
are now fixed without regressing either suite — `gsub` numeric replacement,
`%g`/`%e` negative-zero sign, `stack overflow` location prefix, and `__tostring`
accepting a number return.

**Why whole-file pass-rates moved less than per-bug count:** official test files
abort on their *first* divergence and carry deep failure chains; a file flips to
PASS only when every bug in it is drained. Many remaining file-failures are gated
behind the **deferred architectural items** (task: supervised follow-up):
- `calls.lua` needs bytecode dump headers (5.1/5.2/5.3) **+** `load` reader
  streaming **+** the 5.2 `_ENV`-injection `nupvalues==1` rule;
- `errors.lua`/`gc.lua`/`goto.lua` need the funcname-per-version resolver (F1),
  goto-to-enclosing-label upvalue closing, and the gsub chain's later links.
- Residual cosmetic dep flagged by 3 agents: 5.1 pcall arg-errors should name a
  C function `'?'` (5.1 recorded no C names); gate `find_func_name_in_loaded` off
  for V51 in `crates/lua-vm/src/debug.rs::arg_error_impl`.

### Autonomous loop progress (2026-06-22)

Kit-first architectural waves, file-disjoint worktrees, oracle-gated, no 5.4/5.5 regression:
- **dump headers + `_ENV` injection** (wave pre-loop): flipped `calls.lua` on 5.2 + 5.3.
- **F1 funcname resolver + `error_wording_kit`**: fixed the baseline value-expected name-drop (5.3/4/5), 5.2 `_G.` prefix, 5.1 `?` C-fn naming.
- **5.1 implicit `arg`** (parser V51-only) + **CLI `arg`-on-`-e` gating**: `arg` MATCHes all 5 versions.
- **goto upvalue-closing** (block-scoped goto, 5.2/5.3): flipped `goto.lua` on **both 5.2 and 5.3** via a `CLOSE;JMP` trampoline in `movegotosout`.
- **db.lua 5.1 trace_exec C-frame panic**: fixed (hook now saves/restores `ci`); db.lua advanced 5.1→184, 5.2→481 (not flipped; next links need lua-parse nups + auxlib traceback name).

Running tally: 5.1 40%→52%, 5.2 54%→**79%**, 5.3 74%→**85%**, 5.4/5.5 100% (intact).
Precise remaining diagnoses captured for: gc weak-value clear ordering (state.rs post-mark hook), 5.1 `_ENV`-upvalue→nups, 5.2 unqualified traceback name, 5.1 load reader streaming (cross-crate).

### Loop wave 3 (2026-06-22)
- **gc weak-clear ordering** (state.rs): two-phase atomic (weak-value clear before finalizer resurrection) + 5.1 non-ephemeron weak-key. Advanced gc.lua on 5.1/5.2/5.3 (not flipped; next links = LuaProto::trace cache over-rooting in lua-types/trace_impls.rs + collect-time userdata-finalizer registration in api.rs).
- **db/errors wording** (debug.rs+auxlib.rs): flipped **db.lua@5.2**; 4 version-gated wording fixes (5.2 unqualified traceback name, metamethod `__`-prefix per version, 5.1 nups excludes synthetic _ENV, 5.4+-only call-error attribution). db.lua@5.1 advanced 184→279. errors.lua chains blocked on lua-parse (5.1 empty-statement `;`, 5.2/5.3 CALL line attribution).

Tally: 5.1 52%, 5.2 **83%**, 5.3 85%, 5.4/5.5 100%. Next: lua-parse (errors.lua on 3 versions) + finalizer (gc.lua trace_impls/api.rs).

### Iteration-speed audit (2026-06-22)

Measured the agent inner loop to kill slow loops:
- **Warm edit→build→snippet is already ~1.1s** — `cargo build -p omnilua-cli` after editing *any* crate (even root `lua-types`) is ~0.8–0.9s; `diff_one.sh` is 0.2s. Not the bottleneck. (A per-crate `cargo test -p lua-vm --no-run` is *slower* at 3.5s — building the inline-test harness exceeds the thin CLI binary — so do NOT switch the inner loop to per-crate tests.)
- **The killer was the 60s hang timeout.** Exactly one file hangs: **db.lua@5.3** (infinite loop — the `repeat until name` / CIST_FIN finalizer-frame-level bug). Every progress re-run ate the full 60–65s; everything else fails fast (0.0s). 10 reruns = 10+ wasted minutes on one file.
- **Fix:** `harness/quick_file.sh <ver> <base>` — whole-file check with an 8s cap, classifying PASS / FAIL\<msg\> / HANG. A HANG is a "still failing" signal; never wait 60s for it. Develop on `diff_one` snippets (0.2s); use quick_file for occasional advance checks; only the final gate uses the long timeout. Each worktree has its own `target/` (cold build once per agent) and `[profile.dev] opt-level=1` is deliberate (keeps the debug binary fast enough to run the oracle) — left as-is.
- **Known inner-loop quarantine:** db.lua@5.3 hangs until the CIST_FIN finalizer-frame fix lands; treat its timeout as FAIL, don't grind against it.

### Loop wave 4 (2026-06-22)
- **lua-parse errors chain**: flipped **errors.lua@5.3**; advanced 5.1 (39→181) & 5.2 (211→352). 5 version-gated fixes (5.1 empty-stmt reject, 5.1 ambiguous-syntax, 5.2/5.3 CALL-line attribution, 5.2/5.3 'too many C levels', 5.1 implicit-arg startpc=0).
- **proto-cache trace**: flipped **gc.lua@5.2** (drop white proto.cache, C traverseproto fidelity); gc.lua@5.3 advanced 428→502.
Tally: 5.1 52%, 5.2 **88%**, 5.3 **89%**, 5.4/5.5 100%.
New known hang: errors.lua@5.1 (xpcall+C-stack-overflow+traceback loop on legacy path). Remaining VM-side links: db.lua@5.1 getlocal base-shift (debug.rs/tagmethods adjust_varargs), gc.lua@5.1:228 collect-time userdata finalizability + gc.lua@5.3:502 coroutine-cycle finalization (state.rs), errors.lua@5.2:352 lexer invalid-byte token name (lua-lex).

### Loop wave 5 (2026-06-22) — 5.2 reaches 100%
- **errors.lua@5.2** flipped (lexer: 5.2 invalid-byte near-token `char(N)` vs 5.3+ `<\N>`).
- **nextvar.lua@5.1+5.2** flipped (float-only next-key normalization, version-gated in `api::next`; reverted an unconditional table.rs normalization that regressed 5.4/5.5 `next(t,2.0)`).
- **math.lua@5.2** flipped (oversized-hex `tonumber` via float-only `str2num_float_only`, strtod-only like 5.1/5.2; 5.3+ keep wrapped Int).
- Wave ran in ~8 min (fast-loop rule working).

**MILESTONE: 5.2, 5.4, 5.5 all at 100% (0 our-bugs).** Tally: 5.1 **57%** (12/21), 5.2 **100%**, 5.3 **89%** (24/27), 5.4 100%, 5.5 100%.
Remaining: 5.3 (3) = files@415, gc@502 (coroutine-cycle finalization, state.rs), all@131 (harness cwd); 5.1 (9) = calls/closure/db/errors(hang)/events/files/gc/locals/vararg + the documented VM-side links (getlocal base-shift, collect-time userdata finalizability, metamethod same-ref, reader streaming).

### Loop wave 6 (2026-06-22)
- **gc.lua@5.3** flipped: version-gated legacy `remark_legacy_open_upvalues` pass (5.1/5.2/5.3 re-mark a suspended thread's touched open upvalues once/cycle, keeping a co↔closure↔table cycle alive one extra collect; 5.4+ already matched). state.rs.
- **events.lua@5.1** flipped: 5.1 metamethod same-reference rule for __eq/__lt/__le (TM fires only when both operands carry the same handler; else 'attempt to compare'). tagmethods.rs + vm.rs, V51-gated.
- **locals.lua@5.1** advanced (58→67): getfenv explicit-nil → level 1. Next link is the fenv env-storage gap (closures with no free globals have no _ENV upvalue — needs closure.rs or parse+api).
Tally: 5.1 **62%** (13/21), 5.2 100%, 5.3 **93%** (25/27), 5.4 100%, 5.5 100%.
5.3 at real-bug floor: remaining = files@415 (load-with-reader, cross-crate) + all@131 (harness cwd, not a bug).

### Loop wave 7 (2026-06-22)
- **db.lua@5.1** advanced 279→336: 5.1 implicit `arg` table now built in `adjust_varargs` at call entry (before hooks), V51+K-bit gated. Next link = tail-call frame synthesis (vm.rs/state.rs/lua-stdlib).
- **reader streaming**: documented-no-edit (finding **F6** in docs/IDIOMATIZATION_VM_FINDINGS.md). Confirmed it needs a 10-file coordinated change (api::load signature → reentrant `FnMut(&mut LuaState)`, zio getc/fill take `&mut LuaState`, ParserHook signature + 3 installers). Blocks calls@5.1, files-later@5.1, files@415@5.3. SUPERVISED.
0 file flips. 5.1's remaining tail is cross-crate architectural: reader streaming (F6), fenv env-storage for closures w/o free globals, tail-call frame synthesis, collect-time userdata finalizability.

### Loop wave 8 (2026-06-22) — 5.1 to 71%
- **vararg.lua@5.1** flipped: `clear_arg_table_needed` rewrites the entry VARARGPACK to LOADNIL so a direct-`...` body leaves implicit `arg` nil (C `VARARG_NEEDSARG` cleared). lua-parse.
- **files.lua@5.1** flipped: `file_result` pushes `nil` not `Bool(false)` on failure (luaL_pushfail). io_lib. Also advanced files@5.2/5.3 past line 28.
- **closure.lua@5.1** documented-no-edit: true first divergence is `setfenv(0,a)` in a coroutine clobbering shared `GlobalState.globals`; needs per-thread `l_gt` (state.rs + base.rs + api.rs + load path).
Tally: 5.1 **71%** (15/21), 5.2 100%, 5.3 93%, 5.4 100%, 5.5 100%.
Remaining 5.1 (6, all cross-crate, all V51-gated-safe): calls (reader F6), closure + locals (5.1 fenv/per-thread-env model), db@336 (tail-call frame synthesis), errors (xpcall+stackoverflow hang), gc@228 (collect-time userdata finalizability).
