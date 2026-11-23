# Adversarial: globals55_const (5.5 `global` declarations, `<const>`/`<close>`, init, redeclaration)

Category: 5.5 `global` statement semantics vs reference `lua5.5.0`.

NOTE on the reference binary: `/tmp/lua-refs/bin/lua5.5.0` was built with
`LUA_COMPAT_GLOBAL` **OFF**. `global` is a reserved keyword; once an explicit
`global` decl appears in scope the implicit `global *` is voided, so free names
like `print` must themselves be declared (`global print`). All cases below were
shaped accordingly so the reference produces meaningful output, then run through
`specs/oracle/diff_one.sh 5.5 '<code>'`.

Cases run: 53. MATCH: 24. DIFF: 29.

## Headline finding

lua-rs parses the `global` statement and enforces `<const>` write-protection at
compile time, but **discards the initializer expression entirely** — every
initialized global reads back `nil`. This is the dominant defect and accounts
for ~17 of the 29 DIFFs. Separately, lua-rs does **not implement the strict
declared-global scope model** (undeclared free names inside/after a `global`
decl do not error; redeclaration does not raise "already defined"; `<const>`
upgrade on redeclare is not tracked), and several **error-message wordings**
diverge.

## Findings table

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `global x <const> = 7; global print; print(x)` | `nil` | `7` | FIDELITY | initializer dropped — core |
| `global print; global x <const> = 7; print(x)` | `nil` | `7` | FIDELITY | initializer dropped — core |
| `global x <const> = 7; x = 8` | err assign const | err assign const | MATCH | const write-protection works |
| `global x <close> = nil` | `<close> is not allowed for a global declaration near '='` | `global variables cannot be to-be-closed` | FIDELITY | wrong wording |
| `global x <close>` | `<close> is not allowed ... near <eof>` | `global variables cannot be to-be-closed` | FIDELITY | wrong wording |
| `global a, b <close>` | `<close> is not allowed ... near <eof>` | `global variables cannot be to-be-closed` | FIDELITY | wrong wording |
| `global x <foo> = 1` | `unknown attribute 'foo' near '='` | `unknown attribute 'foo'` | FIDELITY | trailing `near` clause should be absent |
| `global x <foobar>` | `unknown attribute 'foobar' near <eof>` | `unknown attribute 'foobar'` | FIDELITY | trailing `near` clause should be absent |
| `global print; global a = 5; print(a)` | `nil` | `5` | FIDELITY | initializer dropped — core |
| `global print; global a; a = 5; print(a)` | `5` | `5` | MATCH | post-decl assignment works |
| `global print; global a <const> = 1; global b = 2; print(a, b)` | `nil nil` | `1 2` | FIDELITY | initializers dropped |
| `global print; global a, b = 1, 2; print(a, b)` | `nil nil` | `1 2` | FIDELITY | initializers dropped |
| `global print; global a <const>, b <const> = 1, 2; print(a, b)` | `nil nil` | `1 2` | FIDELITY | initializers dropped |
| `global print; global a <const>, b = 1, 2; print(a, b)` | `nil nil` | `1 2` | FIDELITY | initializers dropped |
| `global x; global x` | (ok) | (ok) | MATCH | plain redeclare of nil global ok in both |
| `global x = 1; global x = 2` | (ok, exit 0) | `global 'x' already defined` (exit 1) | FIDELITY | missing already-defined runtime error |
| `global x <const> = 1; global x <const> = 2` | (ok) | `global 'x' already defined` | FIDELITY | missing already-defined runtime error |
| `global x; global x <const>` | (ok) | (ok) | MATCH | const-upgrade redeclare accepted |
| `global x <const>; global x` | (ok) | (ok) | MATCH | |
| `global print; global x; global x <const>; x = 1` | (ok, exit 0) | `attempt to assign to const variable 'x'` | FIDELITY | const upgrade on redeclare not tracked |
| `local x; global x` | (ok) | (ok) | MATCH | |
| `local x = 1; global x` | (ok) | (ok) | MATCH | |
| `global x; local x = 1` | (ok) | (ok) | MATCH | |
| `global print; global x; x = 9; global x = 10; print(x)` | `9` | `global 'x' already defined` | FIDELITY | missing already-defined (already non-nil) |
| `global *; global *` | (ok) | (ok) | MATCH | |
| `global <const> *` | (ok) | (ok) | MATCH | |
| `global x <>` | `<name> expected near '>'` | `<name> expected near '>'` | MATCH | |
| `global x <const> <const>` | `unexpected symbol near '<'` | `unexpected symbol near '<'` | MATCH | |
| `global print; global x <const>; x = 1` | err assign const | err assign const | MATCH | const w/o init still protected |
| `global print; global x <const>; print(x)` | `nil` | `nil` | MATCH | const w/o init reads nil in both |
| `print(1)` | `1` | `1` | MATCH | implicit global-by-default preamble |
| `x = 5; print(x)` | `5` | `5` | MATCH | |
| `global x <const> = 7` | (ok) | (ok) | MATCH | bare decl, no read |
| `global a <const> = 1, b = 2` | `unexpected symbol near '='` | `unexpected symbol near '='` | MATCH | grammar: `=1,b` is explist, then stray `=2` |
| `global print; global y <const> = 10; global z <const> = 20; print(y + z)` | arith on nil `y` | `30` | FIDELITY | initializer dropped → runtime error |
| `global print; global f <const> = function() return 42 end; print(f())` | call nil `f` | `42` | FIDELITY | initializer dropped → runtime error |
| `global print; global t <const> = {1,2,3}; print(t[2])` | index nil `t` | `2` | FIDELITY | initializer dropped → runtime error |
| `global print; global s <const> = "hi"; print(s .. "!")` | concat nil | `hi!` | FIDELITY | initializer dropped → runtime error |
| `global print; global n <const> = 3.14; print(n)` | `nil` | `3.14` | FIDELITY | initializer dropped |
| `global x <const> = 1; x = x + 1` | err assign const | err assign const | MATCH | |
| `global print; global x <const> = 5; local y = x; print(y)` | `nil` | `5` | FIDELITY | initializer dropped |
| `global; print(1)` | `<name> expected near ';'` | `syntax error near ';'` | FIDELITY | wrong wording |
| `global 5` | `<name> expected near '5'` | `syntax error near '5'` | FIDELITY | wrong wording |
| `global *` | (ok) | (ok) | MATCH | |
| `global <const>` | `'*' expected near <eof>` | `<name> expected near <eof>` | FIDELITY | wrong wording |
| `global print; do global a = 1 end; print(a)` | `nil` | `variable 'a' not declared` | FIDELITY | block-scoped global decl + strict-scope model missing |
| `global print; global a; do a = 2 end; print(a)` | `2` | `2` | MATCH | |
| `global print, x; x = 1; global x` | (ok) | (ok) | MATCH | |
| `global print; global x; global x = 5; print(x)` | `nil` | `5` | FIDELITY | initializer dropped (redeclare w/ init) |
| `global print; global x <const> = nil; print(x)` | `nil` | `nil` | MATCH | nil initializer happens to match |
| `global print; global x <close> = setmetatable(...)` | `<close> is not allowed ...` | `global variables cannot be to-be-closed` | FIDELITY | wrong wording |
| `global x = nil; global x = nil` | (ok) | (ok) | MATCH | nil re-decl ok (no already-defined since nil) |
| `global print; if true then global g = 1 end; print(g)` | `nil` | `variable 'g' not declared` | FIDELITY | strict-scope model missing + init dropped |

All DIFFs are classified FIDELITY (5.5 version-fidelity gaps). None are
REGRESSION (no 5.4 cases) and none are NOISE (no nondeterminism — outputs are
deterministic literals/errors).

## Most important confirmed divergences (repro)

1. **Initializer is silently discarded (CORE).** Every `global x = expr`
   reads back `nil`. This breaks the primary documented use of the feature.
   ```
   diff_one.sh 5.5 'global x <const> = 7; global print; print(x)'
   #  OURS: nil   REF: 7
   ```

2. **Dropped initializer cascades into runtime type errors.** When the value
   is then used, lua-rs raises spurious nil-operand errors where the reference
   computes the result.
   ```
   diff_one.sh 5.5 'global print; global f <const> = function() return 42 end; print(f())'
   #  OURS: attempt to call a nil value (global 'f')   REF: 42
   ```

3. **No "already defined" runtime error on redeclaration.** Reference raises
   `global 'x' already defined` (runtime, with traceback) when a `global` decl
   with init re-declares an existing global; lua-rs accepts it silently.
   ```
   diff_one.sh 5.5 'global x = 1; global x = 2'
   #  OURS: (exit 0, no output)   REF: global 'x' already defined  (exit 1)
   ```

4. **Strict declared-global scope model not enforced.** Reference makes
   undeclared free names a compile error once an explicit `global` decl is in
   scope, and scopes block-local `global` decls; lua-rs lets them leak.
   ```
   diff_one.sh 5.5 'global print; if true then global g = 1 end; print(g)'
   #  OURS: nil   REF: variable 'g' not declared
   ```

5. **`<close>` / attribute / syntax error wording all diverge.** lua-rs uses
   its own messages and appends spurious `near '...'` clauses.
   ```
   diff_one.sh 5.5 'global x <close>'
   #  OURS: <close> is not allowed for a global declaration near <eof>
   #  REF : global variables cannot be to-be-closed
   diff_one.sh 5.5 'global x <foo> = 1'
   #  OURS: unknown attribute 'foo' near '='   REF: unknown attribute 'foo'
   ```
