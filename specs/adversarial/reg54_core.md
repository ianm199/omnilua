# reg54_core — 5.4 regression guard vs lua5.4.7

Adversarial differential set probing 5.4 arithmetic, number formatting/coercion,
string library, `string.format`, bitwise, integer/float, `math.*`, comparisons.
Every case below was run through `specs/oracle/diff_one.sh 5.4 '<code>'`. In this
category every case is EXPECTED to MATCH; any DIFF is a REGRESSION.

## Summary

- Cases run: 268 (5 batches; 2 apparent DIFFs were `sed` UTF-8 normalization
  noise re-verified byte-identical, counted as MATCH).
- MATCH: 241
- DIFF (confirmed REGRESSION): 27

The DIFFs collapse into a small number of root causes, all in the
**error-message / value-representation** layer, not in arithmetic/number math
itself. Core arithmetic, number formatting (`%g`/`%a`/`%e`/`%f`/`%q`),
coercion, bitwise, comparisons, `math.*` numeric results are all clean.

## Root-cause clusters (most → least serious)

### 1. `bad argument` messages drop the `to '<fn>'` function qualifier
Ours: `bad argument #1 (value out of range)`
Ref : `bad argument #1 to 'string.char' (value out of range)`
Affects every C-stdlib argument check probed. Central — touches a huge swath of
error output that real Lua programs and test suites assert on.

### 2. Runtime arithmetic/length/concat errors on string-coercion path drop the `(command line):N:` location prefix
Ours: `attempt to mod a 'string' with a 'number'`
Ref : `(command line):1: attempt to mod a 'string' with a 'number'`
Selective: `nil+1`, `{}+1`, `t.x` on nil, calling nil all KEEP the prefix and
MATCH. Only the string-arithmetic-coercion failure path (`"x"%2`, `-"x"`,
`"5"+nil`, `#true`, `nil..nil`) drops it. Central within that path.

### 3. Garbled second-operand type in unary/binary metamethod-failure messages
Ours: `attempt to pow a 'string' with a 'function'` (and `-"x"` → `with a 'function'`)
Ref : `attempt to pow a 'string' with a 'string'`
The reported type of the *second* operand is wrong (shows `'function'` where the
real operands are both strings; unary `unm` should report `'string'`). This is a
genuine logic bug, not just a missing prefix.

### 4. `math.tointeger` / `math.type` return `false` instead of `nil` on failure
Ours: `false`  Ref: `nil`
`math.type(non-number)`, `math.tointeger(non-integral)`, `math.tointeger(huge)`.
The manual specifies these return `nil` (a `fail`). `false` ~= `nil` breaks
`if not math.type(x)` style guards differently and prints differently.

### 5. `got nil` should be `got no value` for an entirely-absent argument
Ours: `... got nil`  Ref: `... got no value`
Lua distinguishes a passed `nil` from a missing argument.

### 6. `table.concat` invalid-value error prints byte array instead of `table`
Ours: `invalid value ([116, 97, 98, 108, 101]) at index 2 ...`
Ref : `invalid value (table) at index 2 ...`
Leaks our internal byte representation of the type name `"table"`.

### 7. Stack traceback omits trailing `[C]: in ?` frame
On every uncaught top-level error the ref traceback ends with `	[C]: in ?`;
ours stops one frame short. Cosmetic-ish but a uniform traceback-format
regression.

## Table

| snippet | ours | ref | class | note |
|---|---|---|---|---|
| `print(math.type("3"))` | `false` | `nil` | REGRESSION | cluster 4 |
| `print(math.tointeger(3.5))` | `false` | `nil` | REGRESSION | cluster 4 |
| `print(math.tointeger(math.huge))` | `false` | `nil` | REGRESSION | cluster 4 |
| `print(math.tointeger(2.0^63))` | `false` | `nil` | REGRESSION | cluster 4 |
| `print(pcall(string.format,"%d",1.5))` | `bad argument #2 (number has no integer representation)` | `bad argument #2 to 'string.format' (...)` | REGRESSION | cluster 1 |
| `print(pcall(string.format,"%d",2^63))` | `bad argument #2 (...)` | `bad argument #2 to 'string.format' (...)` | REGRESSION | cluster 1 |
| `print(pcall(string.sub))` | `... got nil` | `... got no value` | REGRESSION | clusters 1+5 (`to 'string.sub'` + `no value`) |
| `print(pcall(math.max))` | `bad argument #1 (value expected)` | `bad argument #1 to 'math.max' (...)` | REGRESSION | cluster 1 |
| `print(pcall(string.char,256))` | `bad argument #1 (value out of range)` | `bad argument #1 to 'string.char' (...)` | REGRESSION | cluster 1 |
| `print(pcall(string.char,-1))` | `bad argument #1 (value out of range)` | `bad argument #1 to 'string.char' (...)` | REGRESSION | cluster 1 |
| `print(pcall(tonumber,"x",1))` | `bad argument #2 (base out of range)` | `bad argument #2 to 'tonumber' (...)` | REGRESSION | cluster 1 |
| `print(pcall(string.byte,"x","y"))` | `bad argument #2 (number expected, got string)` | `... to 'string.byte' (...)` | REGRESSION | cluster 1 |
| `print(pcall(select,0,"a"))` | `bad argument #1 (index out of range)` | `... to 'select' (...)` | REGRESSION | cluster 1 |
| `print(pcall(ipairs))` | `bad argument #1 (value expected)` | `... to 'ipairs' (...)` | REGRESSION | cluster 1 |
| `print(pcall(rawlen,5))` | `bad argument #1 (table or string expected, got number)` | `... to 'rawlen' (...)` | REGRESSION | cluster 1 |
| `print(pcall(math.tointeger))` | `bad argument #1 (value expected)` | `... to 'math.tointeger' (...)` | REGRESSION | cluster 1 |
| `string.char(256)` (top-level) | `bad argument #1 (value out of range)` + no loc prefix + short traceback | `(command line):1: bad argument #1 to 'char' (...)` + `[C]: in ?` | REGRESSION | clusters 1+2+7; note ref uses short name `'char'` |
| `print(pcall(function() return #nil end))` | `attempt to get length of a nil value` | `(command line):1: attempt ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return #true end))` | `attempt to get length of a boolean value` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return nil .. nil end))` | `attempt to concatenate a nil value` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return 1 .. {} end))` | `attempt to concatenate a table value` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return "x"+1 end))` | `attempt to add a 'string' with a 'number'` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return ("x") % 2 end))` | `attempt to mod a 'string' with a 'number'` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return "5" + nil end))` | `attempt to add a 'string' with a 'nil'` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return "5" - {} end))` | `attempt to sub a 'string' with a 'table'` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return "a" * 2 end))` | `attempt to mul a 'string' with a 'number'` | `(command line):1: ...` | REGRESSION | cluster 2 |
| `print(pcall(function() return "3"^"x" end))` | `attempt to pow a 'string' with a 'function'` | `(command line):1: attempt to pow a 'string' with a 'string'` | REGRESSION | clusters 2+3 (wrong 2nd-operand type) |
| `print(pcall(function() return -"x" end))` | `attempt to unm a 'string' with a 'function'` | `(command line):1: attempt to unm a 'string' with a 'string'` | REGRESSION | clusters 2+3 |
| `print(pcall(table.concat,{1,{},3}))` | `invalid value ([116, 97, 98, 108, 101]) at index 2 in table for 'concat'` | `invalid value (table) at index 2 ...` | REGRESSION | cluster 6 |
| `print(1//0)` (top-level) | traceback missing `[C]: in ?` | `... [C]: in ?` | REGRESSION | cluster 7 |
| `print(1%0)` (top-level) | traceback missing `[C]: in ?` | `... [C]: in ?` | REGRESSION | cluster 7 |

## Clean (representative MATCHes — confirms numeric core is intact)

Integer/float arithmetic, `//`, `%` (incl. negative operands), `^`,
overflow wrap (`maxinteger+1`, `mininteger//-1`, `mininteger%-1`), all bitwise
ops incl. `1<<63`, `1<<64`, `-1>>1`; `0/0`, `±1/0`, `10//0.0`; float printing
(`0.1`, `1/3`, `1e16`, `2^53`, `-0.0`, `math.pi`, `math.huge`); hex/binary float
literals (`0x1p4`, `0x.1p4`); string→number coercion in `+ - * .. `;
`string.format` `%d %i %x %X %#x %o %c %e %E %g %G %f %a %A %s %q %u %% % d` incl.
width/precision/flags; `%q` of control chars, `huge`, `maxinteger`, `mininteger`,
`-huge`, `nan`; all `math.*` numeric results (`abs`,`ceil`,`floor`,`sqrt`,
trig, `log` w/ base, `fmod`, `modf`, `max`/`min` mixed, `ult`, `atan(y,x)`);
comparisons incl. `maxinteger == 2.0^63`, `0.0==-0.0`, `1==1.0`, string ordering;
`tonumber` with bases / hex / scientific / whitespace / malformed; `string.pack`
/`unpack`/`packsize`.
