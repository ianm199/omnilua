# Issue #300 — `<const>` compile-time-constant folding (RDKCTC): design + execution spec

**Status:** supervisor spec, **v2 (post-codex-review)**. Correctness-sensitive
value-model / codegen work → **deep-spec → adversarial codex-review → execute**
(the recorded workflow for value-model changes). This document is the execution
contract. v1 claimed "all plumbing exists, wire two functions"; the adversarial
codex round (gpt-5.x/xhigh) proved that WRONG in two load-bearing ways — v2 folds
in all six findings. **The two CRITICAL prerequisites (§4.0) are the real work;
the recognition/discharge wiring is trivial on top of them.**

## 1. The bug (oracle-divergent, 5.4 + 5.5)

A `local x <const> = <constant-expr>` in reference Lua is **folded**: the value
is stored in the variable descriptor, the variable is marked `RDKCTC`
(compile-time constant), and **no register/local and no upvalue are emitted** for
it. Every later reference to `x` materializes the stored constant inline (as a
`K`/`KINT`/`KFLT`/`KSTR`/`nil`/`true`/`false` operand). The omniLua port never
folds — it keeps `x` as an ordinary local — so a phantom local/upvalue appears.

Repros (reference vs port), both on 5.4 and 5.5:

```lua
-- phantom LOCAL
local x <const> = 42
local a = 10
local i = 1
while true do local n = debug.getlocal(1, i); if not n then break end; print(n); i = i + 1 end
-- reference locals: {a, i}   port: {x, a, i}   (x should be folded away)
```

```lua
-- phantom UPVALUE
local x <const> = 7
local function g() return x end
print(debug.getupvalue(g, 1))
-- reference: nil (x folded into a constant inside g)   port: "x  7" (phantom upvalue)
```

Both diverge because the port emits `x` as a real local (repro 1) that then
becomes a captured upvalue of `g` (repro 2). Correct behavior emits neither.

## 2. Reference algorithm (authoritative — do not reverse-engineer)

All cites are `reference/lua-5.4.7/src/`. 5.5.0 is identical in structure
(`reference/lua-5.5.0/src/` — verify the two functions match before relying on
5.4 line numbers; attributes and RDKCTC exist unchanged in 5.5).

**`luaK_exp2const` (lcode.c:84)** — "if expression is a constant, fill `v` and
return 1, else return 0":
```c
int luaK_exp2const (FuncState *fs, const expdesc *e, TValue *v) {
  if (hasjumps(e)) return 0;
  switch (e->k) {
    case VFALSE: setbfvalue(v); return 1;
    case VTRUE:  setbtvalue(v); return 1;
    case VNIL:   setnilvalue(v); return 1;
    case VKSTR:  setsvalue(fs->ls->L, v, e->u.strval); return 1;
    case VCONST: setobj(fs->ls->L, v, const2val(fs, e)); return 1;   /* const-of-const */
    default:     return tonumeral(e, v);                              /* VKINT / VKFLT */
  }
}
```

**`tonumeral` (lcode.c:56)** — VKINT→int, VKFLT→float, else 0 (also `hasjumps`
guarded).

**`const2val` (lcode.c:75)** — `return &fs->ls->dyd->actvar.arr[e->u.info].k;`
i.e. the stored `TValue` on the active-variable descriptor at absolute index
`e->u.info`.

**`const2exp` (lcode.c:692)** — the reverse; used by `luaK_dischargevars` when
`e->k == VCONST` (lcode.c:775 `const2exp(const2val(fs,e), e)`):
```c
static void const2exp (TValue *v, expdesc *e) {
  switch (ttypetag(v)) {
    case LUA_VNUMINT: e->k = VKINT; e->u.ival = ivalue(v); break;
    case LUA_VNUMFLT: e->k = VKFLT; e->u.nval = fltvalue(v); break;
    case LUA_VFALSE:  e->k = VFALSE; break;
    case LUA_VTRUE:   e->k = VTRUE; break;
    case LUA_VNIL:    e->k = VNIL; break;
    case LUA_VSHRSTR: case LUA_VLNGSTR: e->k = VKSTR; e->u.strval = tsvalue(v); break;
    default: lua_assert(0);
  }
}
```

**`localstat` wiring (lparser.c:1751-1763)** — the only call site:
```c
var = getlocalvardesc(fs, vidx);            /* last variable */
if (nvars == nexps && var->vd.kind == RDKCONST &&
    luaK_exp2const(fs, &e, &var->k)) {      /* compile-time constant? */
  var->vd.kind = RDKCTC;                     /* mark it */
  adjustlocalvars(ls, nvars - 1);           /* EXCLUDE the last var */
  fs->nactvar++;                             /* but count it */
} else { adjust_assign(ls, nvars, nexps, &e); adjustlocalvars(ls, nvars); }
```

**`searchvar` (lparser.c:395-396)** — a reference to an RDKCTC var yields
`init_exp(var, VCONST, fs->firstlocal + i)` (absolute index), NOT a VLOCAL.

**`localdebuginfo` (lparser.c:253)** and **`reglevel` (lparser.c:232)** — both
skip RDKCTC vars (no debug entry, no register slot consumed).

## 3. Current port state (what exists vs what is missing)

`crates/lua-parse/src/lib.rs`. **The plumbing is already present** — this is why
the fix is bounded, not a redesign:

| Piece | C name | Port status |
|---|---|---|
| `VarKind::CompileTimeConst = 3` | RDKCTC | EXISTS (line 96) |
| `ExprKind::Const` | VCONST | EXISTS (line 146) |
| value slot on the descriptor | `Vardesc.k` (TValue) | EXISTS as `VarDesc.const_val: LuaValue` (line 270) |
| searchvar → VCONST materialization | lparser.c:395 | EXISTS (line 3179-3180: `init_exp(var, ExprKind::Const, fs.firstlocal + i)`) |
| debug-info suppression for CTC | localdebuginfo | EXISTS (lines 2951, 3072 gate on `!= CompileTimeConst`) |
| reglevel skips CTC | reglevel | EXISTS (line 3276 region) |
| check_readonly handles VCONST name | lparser.c:281 | EXISTS (line 2987) |
| literal-exprdesc → LuaValue | (part of exp2const) | PARTIAL helper `cg_exp_to_k` (line 1073) covers KInt/KFlt only |

**Missing / broken (the codex round corrected v1 here):**

1. **Recognition.** `localstat` line **6044** is a hardcoded stub (`let is_const
   = false; // placeholder`) — `exp2const` is never called, `const_val` never
   written.
2. **Discharge.** `cg_discharge_vars` (line 2142) has **no `ExprKind::Const`
   arm**; `const_val` is read **nowhere** (grep-confirmed). No `const2exp`.
3. **`const_val` is UNREADABLE from the discharge context** (codex CRITICAL-1).
   `DynData::actvar` lives on `LexState` (line 490); `FuncState` (line 345) holds
   NO reference to `LexState`/`DynData`, and `cg_discharge_vars` takes only
   `&mut FuncState`. So the v1 `cg_const2val(fs,e)` reading `dyd.actvar[...]`
   **cannot compile.** → see §4.0.A.
4. **`nactvar` is used as the register watermark** (codex CRITICAL-2). A folded
   const bumps `fs.nactvar` but consumes no register. But `cg_free_reg` (732),
   `cg_exp_to_any_reg` (2124), and `cg_free_reg_if_temp` (2615) all treat
   `fs.nactvar` as the count of occupied local registers. C instead uses
   `luaY_nvarstack(fs)` (= reglevel, skips CTCs) as the watermark — `freereg`
   at lcode.c:491. Without this, one CTC makes register 0 look like a local when
   it is a temp → register leaks, max-stack divergence, spurious "too many
   registers". → see §4.0.B.

## 4. The fix — `crates/lua-parse/src/lib.rs`

### 4.0 PREREQUISITES (the real work — do these first, each with its own test)

**A. Give `ExprPayload` a const-value snapshot (fixes CRITICAL-1).** Add
`const_snapshot: Option<LuaValue>` to `ExprPayload` (line 220). In `searchvar`
(3175), when a CTC var is found, set BOTH `u.info = fs.firstlocal + i` (absolute
index, keep — `check_readonly` at 2987 needs it) AND
`u.const_snapshot = Some(vd.const_val.clone())`. Then `exp2const`'s const-of-const
arm and the discharge arm read the SNAPSHOT (`e.u.const_snapshot`), never
`dyd.actvar` — so no `LexState` access is needed in `FuncState`-only code. This
is smaller and cleaner than threading a constant arena through the whole codegen
call graph (the alternative codex named); prefer the snapshot.

**B. Decouple the real-local-register watermark from `nactvar` (fixes
CRITICAL-2).** Add a cached watermark to `FuncState` (call it `nreg` /
`freereg_base`) = number of active vars that are in registers (i.e. reglevel of
`nactvar`), maintained by `adjust_local_vars` and `remove_vars` as they add/drop
vars. Audit EVERY codegen use of `fs.nactvar` as a register bound — at minimum
`cg_free_reg` (732), `cg_exp_to_any_reg` (2124), `cg_free_reg_if_temp` (2615) —
and switch those to the new watermark (the port's equivalent of swapping
`fs->nactvar` → `luaY_nvarstack(fs)`, exactly as C's `freereg`/`exp2anyreg` do).
Uses of `nactvar` for *variable-count / scope* semantics (not register bounds)
stay. **This audit is the blast-radius risk: if it reaches beyond codegen-local
register math into scope/goto/upvalue logic, STOP and escalate before proceeding.**
Verify with a bytecode-parity check that with ZERO CTCs present, the new
watermark equals `nactvar` and emitted bytecode is byte-identical to today.

### 4.1 `exp2const` (recognition) + wire `localstat`

Define ONE jump-guard helper `cg_has_jumps(e) -> e.t != e.f` (codex MEDIUM-5: the
reference `hasjumps` is `t != f`, NOT `t==NO_JUMP && f==NO_JUMP`; the port is
currently inconsistent — `cg_is_numeral`:1026 uses the strong form, `exp_to_val`:
4397 uses `t != f`). Use `cg_has_jumps` in `exp2const`.

```
fn cg_exp2const(e) -> Option<LuaValue>:          // note: no fs needed (snapshot)
    if cg_has_jumps(e) { return None }
    match e.k {
        False => Some(Bool(false)),  True => Some(Bool(true)),  Nil => Some(Nil),
        KStr  => Some(Str(e.u.strval.clone())),
        KInt  => Some(Int(e.u.ival)),  KFlt => Some(Float(e.u.nval)),
        Const => e.u.const_snapshot.clone(),      // const-of-const, from §4.0.A snapshot
        _     => None,
    }
```

Wire `localstat` (replace 6043-6052) to the C shape (lparser.c:1751):
```
if nvars == nexps && last_vd_kind == VarKind::Const {
    if let Some(v) = cg_exp2const(&e) {
        actvar[first_local + vidx].const_val = v;              // store on the descriptor
        actvar[first_local + vidx].kind = VarKind::CompileTimeConst;
        adjust_local_vars(ls, state, nvars - 1)?;              // EXCLUDE last
        fs.nactvar += 1;                                       // but count it
    } else { adjust_assign(...); adjust_local_vars(..., nvars)?; }
} else { adjust_assign(...); adjust_local_vars(..., nvars)?; }
```
Ordering is load-bearing: `const_val` set BEFORE `kind`; the `nvars-1` /
`nactvar += 1` accounting counts the const without a register. NOTE the
initializer expr `e` here is still an ExprDesc from `explist` — it is NOT yet a
`Const` (it's the literal), so `cg_exp2const` reads its literal fields directly;
the snapshot only matters for LATER *references* to the folded var.

### 4.2 `const2exp` (discharge) — new `ExprKind::Const` arm in `cg_discharge_vars`

At the top of the `match e.k` (line 2143), mirror `const2exp`, reading the
SNAPSHOT (§4.0.A), never `dyd`:
```
ExprKind::Const => {
    match e.u.const_snapshot.clone().expect("Const expr carries a snapshot") {
        Int(i)   => { e.k = KInt; e.u.ival = i; }
        Float(f) => { e.k = KFlt; e.u.nval = f; }
        Bool(false) => e.k = False,   Bool(true) => e.k = True,   Nil => e.k = Nil,
        Str(s)   => { e.k = KStr; e.u.strval = Some(s); }
    }
}
```
The existing literal paths (`cg_exp_to_k`, `cg_discharge_to_reg`) then lower it to
`LOADI`/`LOADK`/`LOADFALSE`/`LOADTRUE`/`LOADNIL`/K exactly as a bare literal.

### 4.3 `cg_infix` must discharge the const operand FIRST (codex HIGH-3)

`cg_infix` (1996) tests `KInt`/`KFlt` BEFORE discharge, so a `Const` left operand
falls through to `cg_exp_to_any_reg`, emits a load, and stops folding. Reference
`luaK_infix` (lcode.c:1636) discharges first. Fix: call `cg_discharge_vars`
unconditionally at `cg_infix` entry (turning a `Const` into its literal), THEN do
the numeral/immediate tests. Regression MUST include the LEFT-operand case:
`local a<const>=1; local b<const>=a+2` (both fold on 5.4/5.5; `1+a` alone won't
catch it).

### 4.4 5.5 barrier must read the right payload for `ExprKind::Const` (codex HIGH-4)

The 5.5 global/local-shadowing barrier (3274-3281) groups `Local | Const` and
reads `u.var_vidx`, which `searchvar` leaves UNSET for a CTC. Fix: in `searchvar`,
also populate `u.var_vidx` for a CTC found in the current function, and revise the
recursion so a matching current-function `global` declaration wins before an outer
local/CTC resolves. Add BOTH oracle cases from the review:
```lua
global print; local pad=0; global x; local x <const> = 1; print(x)          -- ref: 1
global print; local x <const> = 1; local function g() global x; return x end; print(g())  -- ref: nil
```

### 4.5 Verify (do NOT re-implement) the already-correct downstream

- searchvar → `ExprKind::Const` for CTC (3179): correct; assert, don't rewrite.
- check_readonly (2987): assign-to-`<const>` still errors; oracle-match the
  wording.
- debug-info / reglevel suppression: `adjust_local_vars` always registers debug
  info, so the fold path's `nvars-1` (skipping the last var) is what suppresses
  the CTC's locvar; `remove_vars` (3072) skips the nonexistent CTC entry;
  `reg_level` (2946-2955) skips consecutive CTC descriptors. Confirmed correct by
  codex under the §4.1 accounting — assert a folded var yields NO locvar.

## 5. Version gating (codex-confirmed sound)

RDKCTC folding is **5.4 + 5.5 only**; attributes are accepted only for those
versions (line 5719-5758), so pre-5.4 production parsing CANNOT create
`CompileTimeConst` and the new path is structurally dead there. **No version
branch.** PROVE byte-identical pre-5.4 output: multiversion oracle + bytecode
parity on a 5.3 chunk with ordinary `local x = 42` (and, per §4.0.B, confirm the
new register watermark == `nactvar` when no CTC exists).

## 6. GC / value-model (codex-resolved — record the verdict, don't re-litigate)

- **No UAF via the supported loader.** The port stops GC over the WHOLE
  production parse window (`do_.rs:1652-1690`); explicit GC requests early-return
  while internally stopped (`api.rs:2246`). That protects all untraced parser
  state — half-built `Box<LuaProto>`, token/name handles, `long_str_anchor`, and
  both `const_val` and the new `const_snapshot`. So folding a const string is
  safe under `load`/`loadstring`/`dofile`. (v1's claim that C's collector "sees
  actvar" was wrong — C roots scanned strings via `LexState.h` anchored on the
  stack, `llex.c:134` / `lparser.c:1946`. Same outcome, different mechanism.)
- **Caveat to DOCUMENT, not fix here:** a direct caller of public
  `lua_parse::parse` (bypassing the loader) does not get the stop-GC window. This
  is a PRE-EXISTING whole-parser contract issue (equally true for the constant
  table today), NOT introduced by folding. Add a doc-comment precondition on
  `parse`; do not expand scope to move the guard.
- **String identity:** `GcRef::clone` (gc.rs:233) copies the handle, not the
  bytes — the folded `KStr` is the SAME interned object, so K-table dedup and `==`
  hold. Confirmed.
- **No stray-instruction leak:** an accepted last-expression literal/fold emits no
  instruction; earlier `explist` elements are intentionally emitted (4785). If any
  code WAS emitted, the expr wouldn't pass `exp2const`. Confirmed (the only real
  fold-blocker is the §4.3 infix-discharge bug, which fails to fold rather than
  leaking).

## 7. Kit + oracle plan

**Inner-loop kit (deterministic, no reference binary):** parse-level test in
`crates/lua-parse/tests/` (or extend `multiversion_oracle.rs`) compiling with
`Lua::new_versioned(5.4)` / `5.5` and inspecting the `Proto`. Assert:
- (a) no `LocVar` named `x` for a folded `<const>`;
- (b) no upvalue named `x` in a nested function that reads it;
- (c) the folded value is materialized correctly — but ASSERT THE RIGHT SHAPE
  (codex MEDIUM-6): a small int folds to **`LOADI` with that operand**, NOT a
  constant-table entry; bool/nil use `LOADFALSE`/`LOADTRUE`/`LOADNIL`. Use a
  **string const** or an **out-of-sBx integer** for any "constant table contains
  the value" assertion;
- (d) a NON-constant `<const>` (`local x <const> = f()`) is NOT folded (stays a
  real local — `exp2const` returns None);
- (e) LEFT-operand fold (§4.3): `local a<const>=1; local b<const>=a+2` → `b` folded;
- (f) two-level nested capture reads the const without an upvalue at either level.
Negative-verify by reverting the localstat wiring in-place; the phantom local must
reappear. **Never `git stash` in a worktree.**

**Oracle (truth-teller):** the two §1 repros + the two §4.4 barrier cases via
`specs/oracle/diff_one.sh 5.4` and `5.5`, plus:
- `local k <const> = "s"; local function g() return k end; print(debug.getupvalue(g,1))`
- const-of-const: `local a <const> = 5; local b <const> = a; print(b)`
- float/bool/nil consts; assignment-to-const error wording; `local a<const>=1; local b<const>=a+2; print(b)`
- `harness/run_official_test.sh reference/lua-c/testes/{constructs,errors,locals}.lua`
- full `harness/run_official_all.sh` green; bytecode parity on a pre-5.4 chunk (§5).

## 8. Secondary items from the issue (scope decision)

The issue grouped two minor codegen items. **Recommendation: keep #300 focused on
`<const>` folding.** Fold these in ONLY if trivial and oracle-clean; otherwise
they ride a follow-up — do not let them dilute the value-model review.

- **localfunc unset startpc** (`lib.rs:5707` discards `_fvar`/`_pc`) — C sets the
  local-function debug var's `startpc` after the CLOSURE is emitted so the name
  is visible only from its definition point. Marginally observable via locvar
  dumps / a hook landing on the CLOSURE pc. Low risk, small; OK to include.
- **missing `luaK_checkstack` before for-in control** (`~lib.rs:5612`) — a
  robustness/`maxstacksize` under-count; not easily oracle-shown. Include only
  with a concrete stack-overflow repro that changes observable behavior; else
  defer — an un-observable "fix" has no oracle and violates the campaign's
  keep-vs-nuke rule.

Note: the "missing `luaK_fixline` equivalent" originally listed under #278 was
found ALREADY implemented (`lib.rs:4877-4885`, oracle-verified) — no work.

## 9. Keep-vs-nuke

Ships iff: both repros match reference on 5.4 AND 5.5, pre-5.4 bytecode parity is
byte-identical, the GC-rooting question (§6.1) is resolved soundly (codex-
confirmed), full official stays green, and codex finds no unaddressed value-model
defect. If §6.1 shows `const_val` is NOT safely rooted and rooting it is
non-trivial, STOP and escalate — a folding that can UAF a collected const string
is worse than the phantom-local divergence it fixes.
