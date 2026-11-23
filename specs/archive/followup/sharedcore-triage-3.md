# Shared-Core Triage 3 — Item G + Item-H architectural candidates

Read-only triage. All claims reproduced against the unmodified reference
binaries in `/tmp/lua-refs/bin` (lua5.3.6 / lua5.4.7 / lua5.5.0) and
lua-rs at `target/debug/lua-rs` (version via `LUA_RS_VERSION`). Each item
below has: exact divergence, affected versions, clear-cut-vs-architectural
verdict, and the edit seam.

---

## G. `__le`-from-`__lt` derivation across a coroutine yield

**Affected versions: 5.3, 5.4 (NOT 5.5).**

### Confirmed reference behavior
The `a <= b` → `not (b < a)` fallback via `__lt` (used when `__le` is
absent) exists ONLY in 5.1–5.4; it was **removed in 5.5**. Verified with
no yield:

```lua
local mt = { __lt = function(a,b) return a.x < b.x end }
local a = setmetatable({x=10}, mt); local b = setmetatable({x=12}, mt)
print(pcall(function() return a <= b end))
```
- 5.1/5.2/5.3/5.4: `true  true`
- 5.5: `false  attempt to compare two table values`

lua-rs MATCHES all versions on this no-yield case. The version gating in
`call_order_tm` (`crates/lua-vm/src/tagmethods.rs:748`) is already correct.

### The actual bug — derivation does not survive a yield
When the derived `__lt` call YIELDS, the reference suspends, and on resume
reads the result back and applies the `not(b<a)` inversion. lua-rs loses
both the yield and the inversion.

Repro (`/tmp/le_yield.lua`, mirrors coroutine.lua 5.3-tests:598-600):
```lua
local mt = { __lt = function(a,b) coroutine.yield(nil,"lt"); return a.x < b.x end }
local function new(x) return setmetatable({x=x}, mt) end
local a, b = new(10), new(12)
local function run(f, t)
  local i = 1; local c = coroutine.wrap(f)
  while true do
    local res, stat = c()
    if res then assert(t[i]==nil); return res end
    assert(stat == t[i]); i = i + 1
  end
end
print(run(function() if (a<=b) then return '<=' else return '>' end end, {"lt"}))
```
- ref 5.3.6 / 5.4.7: yields `"lt"` once, then returns `<=`.
- lua-rs 5.3 / 5.4: returns `>` immediately, NO yield observed (wrong
  boolean AND lost yield).
- ref 5.5.0 / lua-rs 5.5: both error (no derivation) — MATCH.

A DIRECT `__lt` metamethod that yields works correctly in lua-rs
(`/tmp/lt_yield.lua` → `nil lt` then `yes`, matching ref). The bug is
specific to the DERIVED second `__lt` call.

### Verdict: ARCHITECTURAL (yield-continuation gap)
The derivation is the second `call_bin_tm` at `tagmethods.rs:758`, whose
result is inverted at line 761 (`matches!(result, Nil | Bool(false))`).
That synchronous read assumes the metamethod returned without yielding. In
C, `luaT_callorderTM` → `callbinTM` uses a continuation
(`lua_callk`/`luaD_call` with a `k` function) so resume re-enters and
re-applies the inversion; lua-rs's `call_bin_tm` has no continuation here,
so a yield unwinds past the inversion logic and the eventual resume returns
the raw (un-inverted, possibly default-false) value.

**Re-entry note.** Fixing requires the derived-`__lt` call to carry a
resume continuation that, on coroutine resume, reads the scratch slot and
applies `!l_isfalse` (the `not(b<a)` inversion) — the same machinery the
non-derived order metamethods already use for direct yields. Check how the
existing direct `__lt`/`__le` yield path threads its result back through
`coroutine.resume` (it works — see `/tmp/lt_yield.lua`) and extend that
path to remember "this was the swapped-operand derivation, invert on
resume." This is NOT a one-line change; it touches the coroutine
resume/continuation plumbing in `crates/lua-vm` (state.rs resume path +
tagmethods.rs `call_order_tm`). 5.5 is unaffected (no derivation).
**Seam:** `crates/lua-vm/src/tagmethods.rs:746-763` + the coroutine
resume continuation machinery in `crates/lua-vm/src/state.rs`.

---

## H1. goto label scoping in disjoint/nested blocks

**Affected versions: 5.3 ONLY (lua-rs over-rejects on 5.3).**

### Confirmed reference behavior — the rule CHANGED at 5.3→5.4
Repro (`/tmp/goto_min2.lua`):
```lua
::l3:: print("a")
do goto l3; ::l3:: end
print("ok")
```
- ref 5.3.6: `a` then `ok` — ACCEPTED. A label inside an inner `do` block
  does not collide with a same-named label in the enclosing block.
- ref 5.4.7 / 5.5.0: `label 'l3' already defined on line 1` — REJECTED.
- lua-rs ALL versions: REJECTED.

So lua-rs is correct for 5.4/5.5 and WRONG for 5.3. This is exactly the
goto.lua (5.3-tests) line-71 case `do goto l3; ::l3:: end` after a
top-level `::l3::` — sole structural blocker on 5.3 goto.lua.

### Root cause (from reference C source)
- `lua-5.3.6/src/lparser.c` `checkrepeated`:
  `for (i = fs->bl->firstlabel; i < ll->n; i++)` — scans only labels in the
  **current block** (`fs->bl->firstlabel`).
- `lua-5.4.7/src/lparser.c` `checkrepeated`: calls `findlabel`, whose loop
  is `for (i = ls->fs->firstlabel; ...)` — scans the **whole function**.

lua-rs ported the 5.4 semantics only: `checkrepeated`
(`crates/lua-parse/src/lib.rs:3780`) calls `findlabel`
(`lib.rs:2654`), which scans from `fs.firstlabel` for ALL versions.

### Verdict: CLEAR-CUT and localized
`BlockCnt.firstlabel` already exists and is populated
(`crates/lua-parse/src/lib.rs:261`, set at `lib.rs:2764-2777`). The fix is
a version-gated scan-start: for `V51|V52|V53`, scan repeated-label
detection from the current block's `bl.firstlabel` instead of
`fs.firstlabel`. Add a block-scoped variant of the `findlabel` loop (or a
parameterized `first`) used by `checkrepeated` when version ≤ 5.3.
**Seam:** `crates/lua-parse/src/lib.rs:3780` (`checkrepeated`) +
`lib.rs:2654` (`findlabel`), gated on `global().lua_version`.
Gate: 5.3 goto.lua; re-run 5.4/5.5 goto.lua to confirm no regression.

---

## H2. loop-built-closure equality caching (closure.lua:48 / :59-66)

**Affected versions: NONE — already correct.**

Repro covering closure.lua:55-66 (`for i=1,5 ... a[i]=function...end`,
`a[3]~=a[4]`, and `f()==f()` identity caching):
```lua
local a = {}
for i = 1, 5 do  a[i] = function (x) return i + a + _ENV end  end
print(a[3] ~= a[4] and a[4] ~= a[5])
do local a = function(x) return math.sin(_ENV[x]) end
   local function f() return a end
   print(f() == f()) end
```
ref and lua-rs both print `true` / `true` on 5.3, 5.4, 5.5.

**Verdict: NO ACTION.** Already fixed. closure.lua's remaining blocker is
item A (`_ENV[1<2]` index codegen), not equality caching.

---

## H3. `__gc` finalizer error propagation (gc.lua:360)

**Affected versions: 5.3 (value divergence) + 5.4/5.5 (missing warning).**

gc.lua:360 itself is gated behind `if T then` (needs the test-C library,
absent in normal builds), so it is not a live oracle assertion. But the
underlying behavior genuinely diverges. Repro (`/tmp/gcerr.lua`):
```lua
local u = setmetatable({}, {__gc = function() error("@boom@") end})
u = nil
print("ok=", pcall(collectgarbage))
```
- ref 5.3.6: `ok= false` — finalizer error PROPAGATES out of
  `collectgarbage`. lua-rs 5.3: `ok= true` — DIVERGES.
- ref 5.4.7 / 5.5.0: `ok= true` (error caught), but emits to stderr:
  `Lua warning: error in __gc (...:2: @boom@)`. lua-rs 5.4/5.5: `ok= true`
  but emits NOTHING — missing warning.

### Verdict: ARCHITECTURAL (collector error/warning plumbing)
Two distinct gaps: (a) 5.3 must let a finalizer error escape the GC step
into the caller's pcall; 5.4+ wraps each finalizer call and converts a
raised error into a `luaE_warning` ("error in __gc (...)"). lua-rs's
finalizer-run path neither propagates (5.3) nor warns (5.4/5.5). This
requires running each `__gc` under a protected call with version-specific
disposition (rethrow on 5.3, warn-and-continue on 5.4+), wired into the
GC's finalizer loop and the warning subsystem.
**Re-entry note.** Find lua-rs's finalizer execution loop in
`crates/lua-gc` / the `to_be_finalized` drain in
`crates/lua-vm/src/state.rs` (`gc_step_flags`/`should_check_gc` reference
`to_be_finalized`). Wrap each finalizer call in a protected frame; on 5.3
re-raise into the triggering call, on 5.4+ route the message through the
same path `warn(...)` uses. Confirm against `/tmp/gcerr.lua` and
`/tmp/gcwarn.lua`. Not localized — touches GC + warning + error-unwind.

---

## H4. debug line-hook fidelity (db.lua)

**Affected versions: 5.3, 5.4, 5.5 (all diverge).**

Basic single-statement line hooks MATCH. The divergence is in the line
event sequence for **multi-line control structures**. Running db.lua from
its real file path (`cd lua-5.5.0-tests && lua db.lua` with the standard
`_soft/_port/_nomsg/_U/arg` preamble via `-e`): ref prints `OK`; lua-rs
fails at `db.lua:28 assert(#l == 0)` reached from `db.lua:124` — the
multi-line `if / math.sin(1) / then / a=1 / else` test left expected lines
unconsumed because the traced line events differ.

Minimal repro (`/tmp/mlif.lua`) — emitted line numbers for a traced
multi-line `if`:
```
if            -- line 1 of chunk
math.sin(1)
then
  a=1
else
  a=2
end
```
| version | reference trace        | lua-rs trace       |
|---------|------------------------|--------------------|
| 5.3     | `3,9,3,10,2,3,4,7,11`  | `3,10,2,3,4,7,11`  |
| 5.4     | `3,9,3,10,2,3,4,7,11`  | `3,10,2,3,4,7,11`  |
| 5.5     | `3,9,3,10,2,4,7,11`    | `3,10,2,3,4,7,11`  |

lua-rs is missing the leading `3,9,3` re-fire pattern (chunk entry / the
calling line) and, on 5.5, emits an extra `3` after `2` that the reference
omits.

### Verdict: ARCHITECTURAL (instruction→line attribution + event timing)
This is about which line each VM instruction is attributed to and exactly
when the `line` hook fires (on back-jumps, on entry, on the
`load`-wrapper's calling line). Matching it byte-for-byte requires
reworking the line-info table consumption and the hook-fire decision in the
VM dispatch loop — deep and cross-cutting, and the correct trace differs
per version (5.5 vs 5.3/5.4).
**Re-entry note.** Seam is the line-hook fire logic in
`crates/lua-vm/src/debug.rs` (`trace_exec`) and the proto line-info
mapping. Drive iteration with `/tmp/mlif.lua` per version (it pins the
exact expected sequence with zero harness noise). Do NOT attempt without a
per-version expected-trace table; this is a fidelity grind, not a
localized fix. Note db.lua also has line-number-sensitive asserts that
ONLY pass from a real file path with no preamble offset — run it via
`cd <tests-dir> && lua-rs -e '<preamble>' db.lua`, never as a combined
source string.

---

## H5. named-vararg `...v` aliasing (vararg.lua:235)

**Affected versions: 5.5 only — already implemented.**

Named varargs (`function (t, ...v)` binding `v` as a no-table vararg
accessor) is a 5.5-only feature. Repro (`/tmp/namedva.lua`):
```lua
local function notab(t, ...v) return v[1], v[2], v.n end
print(notab("x", 10, 20, 30))
```
ref 5.5.0 and lua-rs 5.5 both print `10  20  3`.

**Verdict: NO ACTION.** Already implemented in lua-rs 5.5. The vararg.lua
suite may still fail for other reasons (e.g. line 226 `... _ENV` /
`global` 5.5 syntax), but the named-vararg aliasing mechanism itself is
present and correct.

---

## Summary table

| Item | Affected versions | Verdict | Edit seam |
|------|-------------------|---------|-----------|
| G  `__le`-from-`__lt` across yield | 5.3, 5.4 | ARCHITECTURAL (yield continuation) | tagmethods.rs:746-763 + state.rs resume |
| H1 goto disjoint-block labels | 5.3 only | CLEAR-CUT, localized | lib.rs:3780 checkrepeated + 2654 findlabel, version-gated |
| H2 loop-closure equality | none | NO ACTION (already correct) | — |
| H3 `__gc` finalizer error/warning | 5.3 (propagate) + 5.4/5.5 (warn) | ARCHITECTURAL (GC+warning) | lua-gc finalizer loop + state.rs to_be_finalized + warn path |
| H4 debug line-hook multi-line | 5.3, 5.4, 5.5 | ARCHITECTURAL (line attribution) | debug.rs trace_exec + proto lineinfo |
| H5 named-vararg `...v` aliasing | 5.5 only | NO ACTION (already implemented) | — |

Only H1 is a clear-cut localized fix (5.3 over-rejection). G, H3, H4 are
genuinely architectural. H2 and H5 are already done.
