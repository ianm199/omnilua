# Adversarial: 5.5 `global`-declaration scoping vs lua5.5.0

All cases run via `specs/oracle/diff_one.sh 5.5 '<code>'`. Reference = `/tmp/lua-refs/bin/lua5.5.0` (compat-on build: `global` usable as identifier).

Cases run: 60. MATCH: 41. DIFF: 19.

## Headline finding

The suspected **chunk-wide-vs-block-scoped** bug is CONFIRMED and is the dominant
defect. In real 5.5, an explicit `global` declaration voids the implicit
`global *` only for the **remainder of its enclosing block** (do/while/for/repeat
body, if-branch, or function body); when that block ends, implicit `global *` is
restored. Our implementation makes the `global` decl leak to the rest of the
**whole chunk** (and out of nested functions to the enclosing block), so every
subsequent free name (even `print`) wrongly becomes a compile error
"variable 'X' not declared". This breaks otherwise-valid programs.

A second cluster of defects: `global <const> *` (and `global X <const>` write
enforcement at the collective level), the init-on-non-nil runtime error
`global 'X' already defined`, the `<close>`-global error message, and the
`global`-statement syntax-error message.

## Results table

| # | snippet | ours | ref | class | note |
|---|---|---|---|---|---|
| 1 | `global Y; Y=5; print(Y)` | 5 | 5 | MATCH | |
| 2 | `global Y; Y=1; Z=2` | err Z not declared | err Z not declared | MATCH | top-level void works |
| 3 | `global Y; print(undeclared)` | err print | err print | MATCH | |
| 4 | `X=1; do global Y; Y=1; X=1 end; print(X)` (manual ex) | err X | err X | MATCH | |
| 5 | `global Y; Y=1; global *; Z=2; print(Z)` | 2 | 2 | MATCH | `global *` re-enable works |
| 6 | `do global Y; Y=1 end; print("after")` | err print | after | **FIDELITY** | scope-leak: do-block not restored |
| 7 | `do global Y; Y=1 end; do print("x") end` | err print | x | **FIDELITY** | scope-leak to sibling block |
| 8 | `local function f() global Y; Y=1 end; local function g() Z=2 end; f(); g(); print(Y,Z)` | err Z | `1	2` | **FIDELITY** | global decl in fn f leaks to fn g |
| 9 | `Y=1; global Y; print(Y)` | err print | err print | MATCH | decl-after-use still voids rest |
| 10 | `global Y; local Y=5; print(Y)` | err print | err print | MATCH | |
| 11 | `do do global Y; Y=1 end; X=2 end; print(X)` | err X | 2 | **FIDELITY** | inner-do leak to outer-do |
| 12 | `global *; global Y; Y=1; Z=3` | err Z | (ok) | **FIDELITY** | scope-leak |
| 13 | `local function f() global Y; Y=1; return undeclared end` | err undeclared | err undeclared | MATCH | void within fn works |
| 14 | `local function f() global Y; Y=1; print("hi") end; f()` | err print | err print | MATCH | |
| 15 | `global Y; do global *; Z=1 end` | (ok) | (ok) | MATCH | |
| 16 | `while false do global Y; Y=1 end; print("ok")` | err print | ok | **FIDELITY** | scope-leak (while) |
| 17 | `for i=1,1 do global Y; Y=1 end; print("ok")` | err print | ok | **FIDELITY** | scope-leak (for) |
| 18 | `repeat global Y; Y=1 until true; print("ok")` | err print | ok | **FIDELITY** | scope-leak (repeat) |
| 19 | `if true then global Y; Y=1 end; print("ok")` | err print | ok | **FIDELITY** | scope-leak (if) |
| 20 | `if false then global Y else global Z end; print("ok")` | err print | ok | **FIDELITY** | scope-leak (if/else) |
| 21 | `do global Y; Y=1; W=2 end` | err W | err W | MATCH | void within block works |
| 22 | `for i=1,1 do global Y; Y=1; W=2 end` | err W | err W | MATCH | |
| 23 | `while true do global Y; Y=1; W=2 end` | err W | err W | MATCH | |
| 24 | `repeat global Y; Y=1; W=2 until true` | err W | err W | MATCH | |
| 25 | `if true then global Y; Y=1; W=2 end` | err W | err W | MATCH | |
| 26 | `function f() global Y; Y=1 end; X=2; print("ok")` | err X | ok | **FIDELITY** | scope-leak out of function |
| 27 | `local f = function() global Y end; Z=2; print("ok")` | err Z | ok | **FIDELITY** | scope-leak out of anon fn |
| 28 | `global Y; local function f() Z=2 end` | err Z | err Z | MATCH | nested fn inherits void (correct) |
| 29 | `do global Y end; Z=2` | err Z | (ok) | **FIDELITY** | scope-leak |
| 30 | `do global * end; print("ok")` | ok | ok | MATCH | |
| 31 | `global Y <const>; print(type(Y))` | err print | err print | MATCH | |
| 32 | `global Y <const>; Y=5` | const err | const err | MATCH | |
| 33 | `global Y <close>` | `<close> is not allowed for a global declaration near <eof>` | `global variables cannot be to-be-closed` | **FIDELITY** | wrong error message |
| 34 | `X=5; global X = 10` | (no error) | `global 'X' already defined` + traceback | **FIDELITY** | missing runtime "already defined" check on init-over-non-nil |
| 35 | `global X = 10; print(X)` | err print | err print | MATCH | |
| 36 | `global A,B,C; A=1;B=2;C=3; print(A,B,C)` | err print | err print | MATCH | |
| 37 | `global A=1, B=2; print(A,B)` | err print | err print | MATCH | |
| 38 | `global A <const>, B; A=1` | const err | const err | MATCH | |
| 39 | `global` | `<name> expected near <eof>` | `syntax error near <eof>` | **FIDELITY** | wrong syntax-error message |
| 40 | `global 5` | `<name> expected near '5'` | `syntax error near '5'` | **FIDELITY** | wrong syntax-error message |
| 41 | `global *; print("ok")` | ok | ok | MATCH | |
| 42 | `global <const> *; X=1` | (no error) | `attempt to assign to const variable 'X'` | **FIDELITY** | `global<const>*` const not enforced |
| 43 | `global <const> *; print("ok")` | ok | ok | MATCH | |
| 44 | `global Y; global Y; Y=1; print(Y)` | err print | err print | MATCH | |
| 45 | `global Y, Y; Y=1` | err | err | MATCH | dup name in one decl |
| 46 | `global Y; do Z=2 end` | err Z | err Z | MATCH | void inherited into child block (correct) |
| 47 | `global Y; do global *; print("inner ok") end; W=2` | inner ok (no W err) | `variable 'W' not declared` | **FIDELITY** | inner `global*` should not restore implicit for outer; outer still in must-declare |
| 48 | `do global * end; X=1; print("after")` | after | after | MATCH | |
| 49 | `global Y; if true then print("x") end` | err print | err print | MATCH | |
| 50 | `global Y; for i=1,1 do print(i) end` | err print | err print | MATCH | |
| 51 | `global Y; while false do print("x") end` | err print | err print | MATCH | |
| 52 | `local function outer() global Y; local function inner() return W end; return inner end` | err W | err W | MATCH | |
| 53 | `do local function f() global Y end; X=2 end` | err X | (ok) | **FIDELITY** | global decl inside nested fn leaks to enclosing do-block |
| 54 | `global a <const> = 5; print(a)` | err print | err print | MATCH | |
| 55 | `global a = 1; a = 2; print(a)` | err print | err print | MATCH | |
| 56 | `global <const> *; foo = 1` | (no error) | `attempt to assign to const variable 'foo'` | **FIDELITY** | `global<const>*` const not enforced |
| 57 | `do global <const> *; X=1 end` | (no error) | `attempt to assign to const variable 'X'` | **FIDELITY** | same, in block |
| 58 | `global x <const>; global x; print("ok")` | err print | err print | MATCH | |
| 59 | `function f() return undeclared end; f()` | err undeclared | err undeclared | MATCH | |
| 60 | `global G; G=1; (function() return undeclared end)()` | err undeclared | err undeclared | MATCH | |

## Classification

All confirmed DIFFs are **FIDELITY** (5.5-specific version-fidelity gaps). None
are REGRESSIONs (this category is 5.5-only). No NOISE — all snippets are
deterministic.

The scope-leak DIFFs (6,7,8,11,12,16-20,26,27,29,53) are a single CENTRAL bug.
The const-`*`/`<close>`/already-defined/syntax-message DIFFs (33,34,39,40,42,
47,56,57) are distinct, narrower defects.
