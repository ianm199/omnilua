# Issue #304 — 5.5 per-frame global/local name resolution (the `global x` barrier done right)

**Status:** supervisor spec, **v2 (post-codex-review)**. Value-model / name
resolution → deep-spec → adversarial-codex → execute. v1 proposed a post-hoc
check reusing #300's `ctc_shadowed_by_global`; the adversarial codex round proved
that approach is **fundamentally broken** (it leaves phantom upvalues and cannot
undo capture side effects) and specified the correct design. v2 IS that design.
It is larger than "fix one bug": it corrects the 5.5 name-resolution structure to
match reference, fixing **four** real divergences and REMOVING the post-hoc
barrier machinery (including #300's `ctc_shadowed_by_global` and the
`global_function_names` stack), which become redundant.

## 1. The divergences (all 5.5-only) this fixes

Reference 5.5 `searchvar` scans each function's declarations — locals AND globals
together — newest-first, at EVERY recursive function level, and stops at the
first match (`lparser.c:414`). Globals live in `actvar` as `GDKREG` entries
(`globalfunc`/`global` add them: `lparser.c:1947`, `:502`). The port instead
resolves locals via recursion and applies `global` barriers POST-HOC in
`singlevar`, which misses several cases:

1. **Captured local, owner global-after** (the #304 headline):
   `local x=1; global x; local function g() return x end` → ref `g()==nil`, port `1`.
2. **Global-function name shadowed by an inner local** (finding 3):
   `global function x() local x=1; local function g() return x end; return g end`
   → ref `g()==1` with only upvalue `x`; port returns the global function + a
   phantom `x` upvalue.
3. **Direct named-vararg shadowed by `global`** (finding 4):
   `local function f(...x) global x; return x end` → ref `nil`; port returns the
   vararg table.
4. **`global function x` with an enclosing local `x`** (finding 1's proof):
   `local x=99; global function x() return x end` → ref: the function gets only
   `_ENV`; port gives it `x` + `_ENV` (phantom capture).

Plus the `_ENV` neighbors in §7 that reference rejects and the port accepts.

## 2. Why post-hoc CANNOT work (codex BLOCKER — do not attempt v1)

`singlevaraux` performs IRREVERSIBLE side effects during the recursive unwind,
BEFORE `singlevar`'s post-hoc check could run:

- `new_upvalue` increments `fs.nups` (lib.rs:3244) at each captured level
  (lib.rs:3373); `close_func` preserves every slot through `fs.nups`
  (lib.rs:4520). Setting `var.k = Void` afterward cannot un-mint the upvalue →
  **phantom upvalue** (divergences 2/4).
- `markupval` sets block/`needclose` flags (lib.rs:3358).
- A captured named vararg sets `vararg_table_needed` and rewrites bytecode
  (lib.rs:3354, :2153).
- A phantom capture can hit the upvalue limit and ERROR (lib.rs:3228) before the
  post-hoc check is ever reached.

A transactional unwind of all of that is a larger, worse design. **The barrier
decision MUST be made inside `singlevaraux`, at each frame, before `markupval` /
`mark_vararg_table_needed` / `search_upvalue` / `new_upvalue`** — i.e. where
reference makes it.

## 3. The design (A′ — in-`singlevaraux` per-frame resolution)

At each recursion frame in `singlevaraux`, BEFORE any capture side effect, decide
the winner between the frame's matching local/const/vararg declaration and its
matching `global` declaration, by scope-level (declaration) order — exactly
reference's per-level `searchvar`.

Thread a **barrier-slice upper bound** down the recursion (NOT an origin index —
codex HIGH-2/HIGH-5: an origin out-param is insufficient because `search_upvalue`
can short-circuit before reaching the origin, and `ls.fs` is `None` inside
`singlevaraux` so the existing `ctc_shadowed_by_global` walk cannot run there):

```
fn singlevaraux(ls, fs, n, var, base, upper):   // upper: barrier-slice bound
  match fs {
    None => init_exp(var, Void, 0),
    Some(fs) => {
      let lower = fs.first_scope_barrier;
      let slice = &ls.scope_barriers[lower.min(upper)..upper];
      let v = searchvar(ls, fs, n, var);          // Local | Const | VarArgVar | -1
      let local_level = (v >= 0).then(|| scope level of the found decl in this frame);
      let global_level = latest matching `global` barrier level in `slice` (Option);
      if v >= 0 && (global_level.is_none() || local_level > global_level) {
        // LOCAL/CONST/VARARG WINS in this frame — existing base/markupval/
        // new_upvalue handling, UNCHANGED (this is #300's + the current path)
        ...current found-branch logic (VarArgVar->Local, markupval, etc.)...
      } else if global_level.is_some() {
        // GLOBAL WINS in this frame — resolve as a global, NO capture, NO
        // markupval, NO search_upvalue, NO new_upvalue.
        init_exp(var, Void, 0);
      } else {
        // no match in this frame: existing-upvalue reuse THEN recurse
        let idx = search_upvalue(fs, n);
        ... if idx<0 { singlevaraux(ls, fs.prev, n, var, false, lower)?; ... new_upvalue } ...
      }
    }
  }
```

Key points the executor MUST get right:

- **`searchvar` must expose the declaration scope level** of the found local/
  const/vararg so it can be compared to `global_level`. `scope_level_of_local_in`
  (lib.rs:3815) already computes this given the frame + slice; reuse it. The
  comparison is strict `>` (later declaration wins), identical to
  `ctc_shadowed_by_global`'s owner rule.
- **`VarArgVar` is a declaration** for this comparison (divergences 3) — handle
  it in the same found-branch, for BOTH `base` (direct) and non-base (captured)
  references, BEFORE `mark_vararg_table_needed`.
- **Recurse with `upper = lower`** so each outer frame sees only its own barrier
  slice — this is what makes "a `global` in an intermediate function shadows
  outright, a `global` in the owner shadows only if later than the local" fall
  out naturally (same invariant `ctc_shadowed_by_global` encoded, now applied
  per-frame during resolution).
- The existing-upvalue reuse (`search_upvalue`, lib.rs:3363) must run only AFTER
  the current frame's global check fails to shadow (codex HIGH-2's
  `local y=x; global x; ... g() return x` case: `outer` already has upvalue `x`,
  but the later `global x` in `outer` must still shadow g's reference — so the
  per-frame global check precedes the upvalue-reuse shortcut).

## 4. Removals (these become redundant / are proven wrong)

- **`global_function_names` stack** (lib.rs:3882, push/pop at :6074) and the
  `UpVal`/whatever arm that consults it — codex HIGH-3: reference gives a
  global-function name NO special precedence; it is an ordinary `GDKREG`
  declaration subject to the same per-frame ordering. Remove it entirely; the
  in-frame global check subsumes it (a `global function x` is a `global x`
  barrier at that frame's level).
- **The post-hoc barrier block in `singlevar`** (lib.rs:3395-3421, the
  `Local`/`Const`/`UpVal` arms) — once the decision moves into `singlevaraux`,
  these are redundant. In particular **`ctc_shadowed_by_global`** (the #300
  helper) is subsumed: the `Const` case is now decided in-frame like every other
  declaration. REMOVE `ctc_shadowed_by_global` and fold its logic into the
  per-frame check; keep `scope_level_of_local_in` (reused). The #300 CTC oracle
  cases MUST still pass under the new structure (they are the regression guard).
- `latest_matching_global_barrier_current` and `has_active_global_barrier`: check
  whether they still have callers after the refactor; remove if dead, keep if the
  strict-mode/`global`-decl error paths still use them.

## 5. `_ENV` audit (codex MEDIUM-6)

`singlevaraux` is called directly to resolve `_ENV` (lib.rs:3466, 6062, 6144).
Once a frame can return `Void` for a shadowing global, those callers must map the
`_ENV`-shadowed outcome to the reference ERROR (`"_ENV is global when accessing
variable ..."`), not pass `Void` to codegen. Reference centralizes this in
`buildglobal`/the `_ENV` check (`lparser.c:502`). Cases reference REJECTS that the
port currently accepts:
```lua
global function _ENV() end
global _ENV; global function x() end
global _ENV; global x = 1
```
Make the global-resolution outcome explicit enough that the `_ENV` callers can
raise the reference error. Do NOT silently regress ordinary global access.

## 6. Version gating

Structural: `scope_barriers` is empty on ≤5.4, so `slice` is empty, `global_level`
is always `None`, and the local always wins — identical to today. The threading
of `upper` must be a no-op on ≤5.4. PROVE ≤5.4 byte-identical: multiversion
oracle + a 5.4 bytecode-parity chunk with a captured local AND a 5.3 chunk.

## 7. Kit + oracle plan

**Inner-loop kit** (5.5, via `debug.getupvalue` + evaluated result), asserting
BOTH the value AND the upvalue list (no phantom upvalue):
- (a) divergence 1 owner-after → `g` has NO upvalue `x`, `g()==nil`;
- (b) owner-BEFORE (`global x; local x=1`) → local wins, upvalue present, `g()==1`;
- (c) intermediate-function global → nil, no upvalue;
- (d) two-level chain, both orderings;
- (e) **existing-upvalue shortcut** (codex HIGH-2): `local y=x; global x` in the
  same function then `g() return x` → `y==1`, `g()==nil`;
- (f) **global-function inner-local** (divergence 2) → inner `g()==1`, only `x`;
- (g) **`global function x` with enclosing local** (divergence 4) → function gets
  only `_ENV`, no phantom `x`;
- (h) **direct named-vararg** (divergence 3): `f(...x) global x; return x` → nil;
- (i) **captured named-vararg** shadowed by an enclosing `global x` → nil, no
  vararg-table capture;
- (j) same-function `global x` after a local → global;
- (k) the **`_ENV` rejection** cases (§5) → the reference error;
- (l) non-regression control: a captured local with NO shadowing global → upvalue
  present, correct value;
- (m) ≤5.4: identical code captures the local; byte-identical.
Negative-verify by DISABLING the in-frame global check (the post-hoc arms are
gone, so "revert the UpVal arm" no longer applies) — (a)/(c)/(d)/(f)/(g)/(h)/(i)
go red.

**KEEP the #300 CTC cases** (`const_fold_kit.rs:286` region) green — they are the
primary regression guard that the refactor preserved const folding + shadowing.

**Oracle:** every §1 + §5 + §7 case via `specs/oracle/diff_one.sh 5.5`; full
official suite (run `harness/run_official_all.sh` and report the actual pass
count — do NOT hardcode a number, per CLAUDE.md); `multiversion_oracle`;
`check.sh 5.3/5.4/5.5`; the ≤5.4 byte-parity chunks (§6).

## 8. Keep-vs-nuke / execution guardrails

This is a core-resolution refactor — execute incrementally and gate hard:

- Land the mechanical removals + the in-frame check as reviewable commits;
  keep each `cargo build -p lua-parse` green.
- The invariant that settles correctness: **every §1/§5/§7 case matches the 5.5
  reference, the #300 CTC cases stay green, and ≤5.4 is byte-identical.** If the
  refactor cannot hold ≤5.4 byte-identity, STOP — that means the threading
  perturbed the non-5.5 path.
- Ships iff the above + full official green + a codex round finds no unaddressed
  resolution defect (phantom upvalue, wrong precedence, `_ENV` mishandling).
- If the `_ENV` audit (§5) or the removal of the post-hoc arms reaches beyond the
  resolution path (e.g. into codegen or the strict-mode error machinery in a way
  that isn't a clean substitution), STOP and escalate — the reference-faithful
  restructure is the goal, not a rewrite of adjacent subsystems.

Because this fix corrects FOUR divergences and removes two helper mechanisms
(`global_function_names`, `ctc_shadowed_by_global`), the PR closes #304 and should
note it also fixes the global-function-precedence, direct-vararg, and `_ENV`
divergences codex surfaced — verify each against the 5.5 reference before
claiming it.
