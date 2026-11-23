# Shared-core triage 1 (codegen / lexer) — items A, D, F

Read-only triage. All facts reproduced against the reference binaries in
`/tmp/lua-refs/bin` via `specs/oracle/diff_one.sh` and the reference C sources in
`/tmp/lua-refs/lua-5.{3.6,4.7,5.0}/src`. No code was edited.

---

## Item A — upvalue indexed by a relational/jump key — **CLEAR-CUT**

### Affected versions
5.3 and 5.5 diverge; **5.4 already matches** (incidental register-layout luck,
not a real fix — root cause is shared).

### Exact divergence (reproduced)
The trigger is **a genuine upvalue table indexed by a relational expression**
(a value that the parser codes as a jump list). Both `_ENV` (upvalue 0 in any
chunk) and a captured local qualify.

| snippet | 5.3 | 5.4 | 5.5 |
|---|---|---|---|
| `print(_ENV[1<2])` | DIFF (errors) | MATCH | DIFF (errors) |
| `local up={}; (function() print(up[1<2]) end)()` | DIFF | MATCH | DIFF |
| `local x,y=1,1 ... up[x==y]` (non-folded) | DIFF | MATCH | DIFF |
| `_ENV[true]` / `up[true]` (literal bool) | MATCH | MATCH | MATCH |
| `t[1<2]` where `t` is a **register** table | MATCH | MATCH | MATCH |
| `g[x<y]` where `g` is a global (→ register first) | MATCH | MATCH | MATCH |

Our error: `attempt to index a number value`; reference returns `nil`.
The discriminator is sharp: a **literal boolean constant** key works; a key that
is a **comparison/jump expression** breaks — and only when the indexed table is
an **upvalue** (GETTABUP path), never when it is a register (GETTABLE).

### Root cause / edit seam
`crates/lua-parse/src/lib.rs`, `yindex` (line ~3028). The call that materializes
the index key is **stubbed out**:
```rust
// TODO(port): lua_code::exp_to_val(ls.fs.as_mut().unwrap(), v)?;   // line 3040
```
All three reference versions call `luaK_exp2val(ls->fs, v)` in `yindex`
(`lparser.c`: 5.4 L826, 5.3 L626, 5.5 L902). `luaK_exp2val` (`lcode.c` L987) is:
```c
if (hasjumps(e)) luaK_exp2anyreg(fs, e); else luaK_dischargevars(fs, e);
```
i.e. it forces a jump/relational key into a real register **before**
`luaK_indexed` runs. In `luaK_indexed` (`lcode.c` L1279) the upvalue branch then
relies on the invariant that the key is already a clean value
(`lua_assert(isKstr(fs,k))` after the `exp2anyreg(t)` that discharges the table).

In our port the order is inverted: `cg_indexed` (lib.rs L2500) discharges the
**table upvalue** to a register at L2512 (`cg_exp_to_any_reg(fs,line,t)`) **while
the relational key's jump list is still pending**, then discharges the key only
at L2536. Emitting the table's `GETUPVAL` between the comparison and its
boolean-materialization corrupts the register/jump bookkeeping, so the
LoadTrue/LFalseSkip lands wrong and GETTABUP indexes a number.

**Fix:** implement `exp_to_val` (port of `luaK_exp2val`: if `e` has jumps →
`cg_exp_to_any_reg`, else `cg_discharge_vars`) and call it in `yindex` at the
TODO (L3040), matching all three reference versions. Localized; no version gate.
Also fix `codename`'s string-key path if it shares `yindex`'s caller — string
keys already work, so the change must be a no-op for `VKStr` keys (exp2val on a
no-jump VKStr just dischargevars, which is benign). Sole blocker on official
`closure.lua`.

---

## Item D — `\u{...}` upper bound in the lexer — **CLEAR-CUT (5.3-only)**

### Affected versions
**5.3 only.** 5.4 and 5.5 already match.

### Exact divergence (reproduced)
| snippet | 5.3 | 5.4 | 5.5 |
|---|---|---|---|
| `"\u{110000}"` | ref errors "UTF-8 value too large"; ours accepts (len 4) → DIFF | MATCH | MATCH |
| `"\u{7FFFFFFF}"` | ref errors; ours accepts (len 6) → DIFF | MATCH | MATCH |

5.3 caps the codepoint at **`0x10FFFF`**; 5.4/5.5 cap at **`0x7FFFFFFF`**.

Reference (`llex.c readutf8esc`):
- 5.3 L339: `esccheck(ls, r <= 0x10FFFF, "UTF-8 value too large");` — checked
  **once on the final value**, after the digit loop.
- 5.4 L351 / 5.5 L373: `esccheck(ls, r <= (0x7FFFFFFFu >> 4), ...)` — a
  **per-digit** overflow guard (`r <= 0x7FFFFFF` before each `r = r*16 + d`),
  allowing a final value up to `0x7FFFFFFF`.

### Edit seam
`crates/lua-lex/src/lib.rs`, `read_utf8_esc` (L1302). Line 1321 hard-codes the
5.4/5.5 per-digit guard `r <= (0x7FFF_FFFF >> 4)` with no version branch:
```rust
esc_check(state, ls, r <= (0x7FFF_FFFFu32 >> 4), b"UTF-8 value too large")?;
```
**Fix:** read the version from `state` (`state.global().lua_version`). For
`V53`, replace the per-digit guard with a **single final-value check**
`r <= 0x10FFFF` after the loop (matching 5.3's structure at its L339); keep the
existing per-digit guard for V54/V55. (`V51`/`V52` have no `\u{}` escape at all,
so they are out of scope here.) Localized; clean version gate. Blocks
`literals.lua` on 5.3.

---

## Item F — `string.unpack("c0", x, 0)` lower-bound position — **CLEAR-CUT (5.3-only)**

### Affected versions
The **pos=0 rejection** is **5.3-only** (5.4/5.5 deliberately accept pos=0).
A *separate* defect — the missing `to '<fn>'` in the error message — is shared
across all versions but belongs to **item B** (`arg_error` funcname omission),
not item F.

### Exact divergence (reproduced)
| snippet | 5.3 | 5.4 | 5.5 |
|---|---|---|---|
| `string.unpack("c0", "abc", 0)` | ref ERRORS "initial position out of string"; ours returns `1` → DIFF | ref returns `1` (ours matches) | ref returns `1` (ours matches) |
| `string.unpack("c0", "abc", 5)` (upper bound) | errors on all; message differs by item B only | same | same |

tpack.lua confirms the version split: 5.3 tests `checkerror("out of string",
unpack, "c0", x, 0)` (line 315); 5.5's tpack.lua checks only `#x + 2`
(line 318) — pos=0 is intentionally valid on 5.4/5.5.

### Root cause / edit seam
`crates/lua-stdlib/src/string_lib.rs`, `str_unpack` (L2509). Line 2514:
```rust
let mut pos = pos_relat_i(pos_raw, ld).saturating_sub(1);
```
This uses 5.4+'s `posrelatI` semantics for **all** versions. 5.3 uses the older
`posrelat` (`lstrlib.c` 5.3 L1481): `pos = (size_t)posrelat(opt,ld) - 1`, where
`posrelat(0,ld)` returns `0`, so `pos` underflows to `(size_t)-1` and the
`pos <= ld` check (L2516 here) fails → the error. `posrelatI(0,ld)` returns `1`,
and `saturating_sub(1)` additionally masks the underflow 5.3 depends on.

**Fix:** version-gate L2514 — for `V53`, use the old `posrelat` (0 stays 0) and
do **not** saturate the `-1` (let it produce a large `pos` so the existing
`pos > ld` guard at L2516 fires). Keep `pos_relat_i` for V54/V55. Localized;
clean version gate. The `to 'string.unpack'` message wording is item B; do not
conflate. Blocks `tpack.lua` on 5.3.

---

## Note on items D and F vs the task statement
The task framed D and F as multi-version. **Confirmed against the references:
both are 5.3-only** — our port already matches 5.4 and 5.5. They are still
"shared-core" in that the seam is a single un-versioned code path that must grow
a `V53` branch, but the wording in the task ("blocks literals.lua / tpack.lua",
"confirm per-version") is satisfied by a 5.3 gate, not a cross-version rewrite.
