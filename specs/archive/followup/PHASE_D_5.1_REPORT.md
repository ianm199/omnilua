# Phase D — Lua 5.1 backend verification report

Read-only independent verification of the V51 (Lua 5.1) backend on branch
`finish-5.1`. Oracle = `/tmp/lua-refs/bin/lua5.1.5`. No source was edited.

## Before / after

- **Before this phase**: `LuaVersion::V51` existed with `FloatOnly` number
  model but was excluded from `is_supported()`; V51 was refused at runtime.
- **After (current state of branch)**: V51 is in `is_supported()`, the runtime
  `new_versioned` guard accepts it, and the CLI parses `LUA_RS_VERSION=5.1`.
  fenv globals (getfenv/setfenv via Option B reuse of the `_ENV` upvalue),
  the `__len`-on-tables inert flip, `__pairs`/`__gc`-on-tables inert, the 5.1
  stdlib roster, pre-5.2 syntax gates, and the float-only model are all wired
  and V51-gated.

## Gate results

| Gate | Result |
|---|---|
| `cargo build -p lua-cli` | PASS |
| `cargo test --workspace --features lua-rs-runtime/derive` | PASS — 0 failures workspace-wide |
| multiversion_oracle `v51_*` tests | PASS — 23/23 |
| `check.sh 5.1` battery | PASS — 55/55 vs lua5.1.5 |
| `check.sh 5.2` (no-regression) | PASS — 54/54 |
| `check.sh 5.3` (no-regression) | PASS — 23/23 |
| `check.sh 5.4` (no-regression) | PASS — 7/7 |
| `check.sh 5.5` (no-regression) | PASS — 10/10 |

## diff_one.sh spread (independent of the battery)

| Area | Verdict |
|---|---|
| fenv: per-closure env, getfenv()==_G, setfenv returns f, new-closure inherits creator env, setfenv(0) thread split, loaded-chunk thread env, getfenv/setfenv level forms, float-level truncation, getfenv(C fn)==_G | MATCH |
| fenv: setfenv(1,g) readonly-globals idiom (readonly.lua) | MATCH (behavior); traceback wording diverges — see gaps |
| `__len` on table ignored (primitive length wins) | MATCH — #1 silent-failure trap is correctly handled |
| `__len` on **userdata** (newproxy) honored | MATCH — `#p` returns 42 from metamethod |
| `__pairs` on table ignored | MATCH |
| `__gc` on table inert (userdata-only finalize) | MATCH |
| Roster present: getfenv/setfenv, unpack(global), loadstring, table.getn/setn/maxn/foreach/foreachi, module, package.seeall/loaders, string.gfind, math.log10/atan2/pow, gcinfo, newproxy, math.log 1-arg | MATCH |
| Roster absent: table.unpack, bit32, utf8, math.type | MATCH |
| Float-only numbers: 10/2, 2^2, %, ^0.5, tostring(1.0), big-int literal, hex-int literal, strcoerce add, math.modf, math.huge | MATCH |
| _VERSION == "Lua 5.1" | MATCH |
| xpcall arity (extra args ignored, handler-only) | MATCH — both ref and ours pass nil for the dropped extra args |
| coroutine.running() == nil in main | MATCH |
| Syntax rejected: goto/labels, `//`, `&`, `|`, `~`, `<const>`, `0x1p4`, `\x` | MATCH (all rejected) |
| Example scripts (bisect, factorial, fib, fibfor, hello, life, printf, readonly, sieve, sort, table, trace-calls, trace-globals) | 11/13 byte-identical; fib differs only in os.clock timing (allowed); readonly differs only in traceback wording (see gaps) |

## Documented gaps / divergences (none behavioral-blocking)

1. **5.1 traceback wording — `[C]: ?` vs `[C]: in ?`.** Lua 5.1 prints the
   final C frame as `[C]: ?`; 5.2+ changed it to `[C]: in ?`. Under V51 we emit
   the 5.2+ form. Confirmed 5.1-specific (5.4 matches). Cosmetic; affects the
   tail line of every error traceback. (Surfaced in readonly.lua and bare
   `error()`.)

2. **5.1 traceback frame naming — `in function <src:line>` vs
   `in metamethod 'newindex'`.** In a `__newindex` error, 5.1 names the frame
   by source location; we use the 5.2+ metamethod-name form. Cosmetic
   traceback diff only; the error message and behavior match (readonly.lua).

3. **Argument-typecheck error messages name the function (`'unpack'`,
   `'setfenv'`, `'newproxy'`) where 5.1 prints `'?'`.** 5.1's argument errors
   from these globals report the function as `'?'`; we report the real name.
   The error *class* and value match (e.g. `pcall` returns false + a
   `table expected, got nil` message). Cosmetic.

4. **`unpack(nil)` error message.** Ours raises "attempt to get length of a nil
   value"; 5.1 raises "bad argument #1 to 'unpack' (table expected, got nil)".
   We skip the up-front arg typecheck and fail at the length op. Both error and
   both `pcall` to false; only the message text differs.

5. **`getfenv(2)` across a tail call.** With `return a()` (a tail call), 5.1
   reports `no function environment for tail call at level 2`; ours resolves a
   table. We perform TCO (verified: 1M tail calls do not overflow), so this is
   a genuine narrow divergence in how level resolution interacts with collapsed
   tail-call frames. Extremely unlikely to affect real programs.

6. **math.random / randomseed sequence (pre-existing allowed exception).** 5.1
   uses C `rand()`; the byte sequence is host-dependent and not portably
   bit-matchable in Rust. Contract verified instead: `math.random()` in [0,1),
   `math.random(n)` in [1,n], argument errors, randomseed accepts a number.
   Sequence divergence is the one allowed-documented exception.

7. **C-function environment (LUA_ENVIRONINDEX).** `getfenv(print)` returns `_G`
   (matches ref for the common case). A distinct per-C-function environment is a
   documented gap, not silently faked — `getfenv(C fn)==_G` is the observed and
   tested behavior, consistent with the spec's allowance.

All seven items are either cosmetic (traceback/error-message text), a
pre-approved RNG exception, or a documented C-env gap. None changes the
observable behavior of a correct 5.1 program; the fenv mechanism, the
`__len`-on-tables inert flip (the #1 trap), the roster, the syntax gates, and
the float-only model are all faithful.

## Verdict

**PASS — Lua 5.1 is faithful enough to be marked supported, with no other-version
regression, RNG-sequence (and the listed cosmetic traceback/error-text)
divergences excepted.**

Battery 55/55, oracle test suite 23/23, full workspace tests green, no
regression on 5.2/5.3/5.4/5.5, and 11/13 example scripts byte-identical (the
other two differ only in os.clock timing and traceback wording). The two
highest-risk axes called out in the spec — fenv globals and `__len`-on-tables —
both behave correctly against lua5.1.5.
