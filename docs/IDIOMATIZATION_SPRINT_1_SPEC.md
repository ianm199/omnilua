# Idiomatization Sprint 1 — live spec + recipe ledger

Owner: Fable (supervisor: design sign-off + verification). Execution: Opus
subagents per subsystem. Plan of record: `docs/IDIOMATIZATION_ROADMAP.md`
(Stage-2; read first). This is the live checklist, the Phase-0 scaffolding
(recipe format + graduation template + gate template), and the recipe ledger.

Baseline (2026-06-13): bytecode-parity oracle GREEN across the bench corpus at
main 5abb986; `omnilua` 0.2.0; official suite passing.

## Checklist (tick only with evidence)

- [x] P0: scaffolding — recipe format, graduation declaration template, gate
      template (this doc). Exercised end-to-end by the P1a pilot.
- [x] P1a: LEXER (lua-lex) idiomatized on `idiom/lexer` — bytecode parity (bench
      all-OK + broad-corpus counts identical) + behavioral suite green (oracle
      165, official 33/33, literals/errors, check.sh 5.1/5.4); 5 recipe entries +
      graduation doc (`crates/lua-lex/GRADUATED.md`) + 17 unit tests.
      **MERGED — PR #193 (`b113ab7`), CI green; supervisor re-verified the gate.**
- [x] P1b: PARSER (lua-parse) idiomatized on `idiom/parser` — bytecode parity
      (bench all-OK + broad-corpus counts identical) + behavioral suite green
      (oracle 165, official 33/33, literals/errors, check.sh 5.1..5.5
      57/54/23/7/10); 3 recipe entries + 2 detailed honest-negatives (RAII,
      ExprPayload enum) + graduation doc (`crates/lua-parse/GRADUATED.md`) + 4
      unit tests; `unsafe` 0 → 0.
      **MERGED — PR #194 (`2ee0136e`), CI green; supervisor re-verified the gate.**
- [x] P1c: CODEGEN — **SUBSUMED into P1b.** `lcode.c` is folded into `lua-parse`
      (86 `cg_*` functions); the standalone `lua-code` crate is only the opcode
      tables / `Instruction` encoding, so there is no separate codegen crate to
      idiomatize. The emit/register/jump/line-info/constant-fold core was
      deliberately left structurally faithful (the hot-loop exception). No
      separate P1c work item remains.
- [x] REFLECT: `docs/IDIOMATIZATION_REFLECTION_1.md` written — recipes, the
      verification-model shift, the "idiomatization debt is not uniform" finding,
      honest-negatives as the richest output (incl. the two-phase technique for a
      future `ExprPayload` enum attempt), roadmap adjustments, and an explicit
      **Phase-2 GO** (with a coverage-check precondition + recommended
      easy-pure-first → hot-string-matcher sequence). Phase 2 NOT started in this
      run, per scope.
- [x] CLOSE: PR #193 + PR #194 merged CI-green (`b113ab7`, `2ee0136e`); the
      reflection + this checklist land via the reflection/closeout PR; board row
      closed on `../AGENT_COORDINATION_BOARD.md`.

## Phase-0 scaffolding

### The gate (Phase 1 — bytecode-parity-net subsystems)
A subsystem is idiomatized only when ALL are green:
1. **Bytecode parity**: `python3 harness/bench/bytecode-parity.py <targets>`
   byte-identical to `luac -l -l` (allowlist `bytecode-parity-allow.txt`
   unchanged — do NOT add entries to dodge a regression). Run against BOTH the
   bench corpus AND a broad set of official-test `.lua` files (lexer/parser
   need wide token/grammar coverage; pass the file list as argv).
2. **Behavioral suite**: `harness/run_official_all.sh` (full pass) +
   `cargo test -p omnilua --test multiversion_oracle` (165) + the
   lexical-error/line-number behavioral tests (errors.lua, the syntax-error
   and line-attribution cases, `specs/oracle/check.sh 5.1`..`5.5`).
3. Crate gates: `cargo test -p <crate>`, `cargo test --workspace`,
   `cargo check --target wasm32-unknown-unknown`.
These subsystems are COLD (run at load, not per-op) — no perf arbiter needed;
bytecode parity is the structural oracle and it SURVIVES idiomatizing the
producer (you change the internals; the emitted bytecode must not move).

### Recipe-catalogue format
Each idiomatization records, in this doc's "Recipe ledger" section, entries of:
- **Pattern name** (e.g. `c-charptr-scan -> peekable-iterator`)
- **Before** (the C-port shape, 1-3 lines) → **After** (the idiomatic shape)
- **Behavioral invariant that replaced the structural one**: what you now
  trust instead of "matches llex.c line N" (e.g. "token stream yields identical
  bytecode; lexical errors byte-identical per errors.lua").
- **Caveats / where it doesn't apply.**

### Graduation declaration (per subsystem)
On merge, each idiomatized subsystem gets a short `## Graduated: <crate>` note
(in its crate CLAUDE.md or a `GRADUATED.md`) stating: the C correspondence is
intentionally gone; the oracle that now guards it; what a future debugger should
trust instead of the C source. This is the load-bearing artifact — it tells the
next person the structural crutch is removed and what replaced it.

## Recipe ledger
(append transformation recipes here as subsystems graduate)

### P1a — lua-lex (the pilot), 2026-06-14

Five transformations, one commit each, the Sprint-1 gate green after every one
(bytecode parity bench all-OK + broad-corpus divergent-op counts identical to
baseline; multiversion_oracle 165; official 33/33; literals.lua + errors.lua;
check.sh 5.1 57/0, 5.4 7/0; `unsafe` stayed 0).

A note that shaped every recipe below: **the public boundary is `lua-parse`.**
`lua-parse` calls `lua_lex::{next,lookahead,set_input,lex_error,syntax_error,
sem_error,token2str}`, constructs `LexState`/`Token`/`TokenValue`/`ZIO`/
`LexBuffer` by their public fields, and reads the `TK_*` `i32` codes. So every
idiomatization had to stay **inside** those signatures. The recurring lesson:
*idiomatize the internals up to the boundary, then stop and document the
boundary as deliberate* — half the value of a recipe is knowing where it must
not reach.

---

**Recipe: `c-index-triple → single-cursor reader`**
- Pattern: a C buffer-pointer struct carrying `(n /* remaining */, p /* pos */,
  buf)` where `n` is hand-decremented in lockstep with `p`.
- Before:
  ```rust
  if self.n > 0 { self.n -= 1; let b = self.current_chunk[self.p]; self.p += 1; b as i32 }
  else { self.fill() } // fill sets n = chunk.len()-1, p = 0, returns chunk[0]
  ```
- After: keep only the cursor; derive "remaining" from `chunk.len() - cursor`.
  ```rust
  match self.chunk.get(self.cursor) { Some(&b) => { self.cursor += 1; b as i32 }
                                      None => self.fill() }  // fill: cursor = 1
  ```
- Invariant that replaced the structural one: the unit test `zio_yields_bytes_then_eoz`
  (byte sequence then a *stable* `EOZ`, empty source immediately `EOZ`) plus the
  whole bytecode-parity net — the cursor feeds every token.
- Caveat: preserve the *empty-chunk-without-committing* behaviour (an empty chunk
  reports `EOZ` but does not advance, so an interactive reader can be re-polled).
  Collapsing `None` and empty-chunk into one arm is fine **only** because both
  already produced `EOZ` without committing.

**Recipe: `c-int-status + out-param → Option`**
- Pattern: a C function returning `0`/nonzero status and writing its real result
  through an out-pointer (`int f(in, T *out)`).
- Before:
  ```rust
  let mut obj = LuaValue::Nil;
  if str2num(bytes, &mut obj) == 0 { return Err(lex_error(..., "malformed number", TK_FLT)); }
  match obj { Int(i) => ..., Float(f) => ..., _ => unreachable!() }
  ```
- After: a private wrapper that reduces the convention to `Option`, then `match`:
  ```rust
  fn parse_numeral(bytes: &[u8]) -> Option<LuaValue> { /* 0 => None else Some(out) */ }
  match parse_numeral(...) { None => Err(...), Some(Int(i)) => ..., Some(Float(f)) => ... }
  ```
- Invariant that replaced the structural one: `numeral_int_float_boundary` +
  `float_only_versions_widen_integer_literals` unit tests, and the malformed-number
  error path (line + `TK_FLT` tag) checked by `errors.lua`.
- Caveat: the *callee's* signature was out of scope (`lua_vm::object::str2num`
  lives in another crate); idiomatize at the **callsite** with a wrapper, don't
  chase the convention across the crate boundary.

**Recipe: `borrow-dance copy → named extraction method`**
- Pattern: `let v = buf[..].to_vec(); use(&mut owner, &v)` blocks whose `.to_vec()`
  exists *only* to end an immutable borrow before a `&mut` call — papered over
  per-site with a `PORT NOTE`.
- Before (×3, plus two inline trailing-NUL trims):
  ```rust
  // PORT NOTE: copy to drop the borrow before new_string
  let buf = ls.buff.as_slice();
  let content: Vec<u8> = buf[sep..buf.len()-sep].to_vec();
  ```
- After: name the *intent* on the buffer type; document the owned copy **once**
  on the method as a real ownership requirement, not a C idiom.
  ```rust
  let content = ls.buff.trim_ends(sep);   // and to_owned_text(), without_trailing_nul()
  ```
- Invariant that replaced the structural one: `lexbuffer_extraction_helpers`,
  `long_brackets_with_levels`, `hex_escape_decodes_to_bytes`, and the `near '...'`
  snippet exactness in `errors.lua` (which depends on the NUL-trim).
- Caveat: the copy is **genuinely required** (the callee mutates `ls`); the recipe
  is about *naming* it, not eliminating it. Don't try to hand back a borrow — that
  fights the borrow checker for no behavioral gain.

**Recipe: `integer-tag dispatch → internal enum at a stable boundary`**
- Pattern: a hot `match` on a raw integer that also crosses a public boundary as
  an integer; you want the typed-match ergonomics without changing the boundary.
- Before:
  ```rust
  match ls.current {                       // i32, EOZ buried as a guard
      c if c == b'=' as i32 => ...,
      c if c == EOZ => return Ok(TK_EOS),
      c => { if is_lalpha(c) ... }
  }
  ```
- After: an **internal** enum for the lexer's own classification; the emitted
  token kind still leaves as `i32`.
  ```rust
  enum Peek { Byte(u8), Eoz }
  fn peek(ls) -> Peek { match u8::try_from(ls.current) { Ok(b) => Byte(b), Err(_) => Eoz } }
  match peek(ls) { Peek::Byte(b'=') => ..., Peek::Eoz => return Ok(TK_EOS),
                   Peek::Byte(b'0'..=b'9') => ..., Peek::Byte(c) => ... }
  ```
- Invariant that replaced the structural one: the whole bytecode-parity net
  (token kinds unchanged) + `peek_maps_byte_and_eoz`, `multichar_symbols_and_dots`,
  `version_gated_operators`.
- Caveat: this is the **only** way the "tag → enum" idiom applies when the tag is a
  cross-crate boundary. Resolve the deferred "TokenKind enum" note *internally*;
  do not push the enum out to `Token.kind` (that would force `lua-parse` changes).
  `u8::try_from` maps EOZ (`-1`) cleanly to the `Err` arm — no sentinel handling.

**Recipe: `crutch removal on graduation`**
- Pattern: once a subsystem is idiomatic, its line-by-line C scaffolding is pure
  noise that actively misleads (it claims a correspondence that no longer holds).
- Action: delete the `# C source` verbatim-C doc blocks, the `file.c:NNN` refs,
  the `*.tsv` annotation comments, the C-correspondence `PORT NOTE`s, and the
  Phase-A/B stub `TODO`s (≈480 lines here). Add a `GRADUATED.md` + a module-doc /
  PORT-STATUS-trailer declaration of the new oracle.
- **Keep-list (the load-bearing half):** behavioral comments survive — every
  version gate, deliberate harness deviations (the `#`-shebang hack), correctness
  rationale (`\r\n` pairing, the `\u{}` check-before-shift order, capacity-vs-len
  in `resize`), and the new idiomatic API docs. Distinguish *"this is what the C
  did"* (delete) from *"this is why the behavior is what it is"* (keep).
- Caveat: comment-only, but it can still break the build (a blank line can detach
  a `///` from its item; an orphaned ```` ``` ```` fence). Gate it the same as a
  code change. Best done after the structural transformations land, not before —
  you want the idiomatic shape in place so it's obvious which comments are now
  redundant.

### P1b — lua-parse (parser + folded-in codegen), 2026-06-14

`crates/lua-parse/src/lib.rs` is a ~6.1k-line port of `lparser.c` **with
`lcode.c` codegen folded in** (86 `cg_*` functions). The recon premise held:
this file was **already mostly idiomatic** (Result/`?`, `Option<Box>` chains,
proper enums, ZERO unsafe). Its C-residue is narrow, so the discipline here was
the *inverse* of P1a — find the genuine residue, idiomatize it, and **leave the
already-idiomatic majority and the hot codegen core alone**. Three code
transformations landed; two planned ones are recorded honest-negatives (below).

The dominant new lesson: **the bytecode-parity oracle is so strong on this
crate (parser + codegen → `luac -l -l` byte-for-byte) that "already idiomatic"
and "load-bearing structural" are the two reasons NOT to change code, and you
must actively resist churning the 84-site `.u.info` / 202-site `match e.k`
surface just because it "looks like C".** The marquee enum recipe is a negative
here precisely because the surface is large *and* the tag/data are set apart.

---

**Recipe: `manual reverse index walk → .rev() range iterator`**
- Pattern: a C-style descending index scan with a hand-decremented counter and a
  `>= 0` guard, used to find the innermost (most-recently-declared) match.
- Before (`searchvar`):
  ```rust
  let mut i = fs.nactvar as i32 - 1;
  while i >= 0 { let vd = get_local_var_desc(ls, fs, i); /* ... */ i -= 1; }
  -1
  ```
- After: a reversed range; the early-return-on-match is a plain `return`.
  ```rust
  for i in (0..fs.nactvar as i32).rev() { let vd = get_local_var_desc(ls, fs, i); /* ... */ }
  -1
  ```
- Invariant that replaced the structural one: **resolution order** — the
  innermost shadowing declaration still wins, because `(0..n).rev()` yields
  `n-1 .. 0` (identical sequence) and the `n==0` empty case matches `i = -1`.
  Proven by bytecode parity (searchvar is on the variable-resolution hot path).
- Caveat: only applies where the index sequence and early-exit are the entire
  loop semantics. Do NOT convert a count-down loop whose counter is entangled
  with deferred side effects — `remove_vars` here keeps its manual `while
  nactvar > tolevel { nactvar -= 1; ... }` because the truncate is deliberately
  deferred until after the loop walks each soon-to-be-removed slot (documented).

**Recipe: `sentinel-terminated chain walk → named lending cursor`**
- Pattern: a singly-linked list threaded through a sentinel (`NO_JUMP`), walked
  by N hand-written `while pc != NO_JUMP { ...; pc = get_next(fs, pc) }` loops,
  where each loop body **mutates the same structure** the chain lives in.
- Before (×4: `cg_remove_values`, `cg_need_value`, `cg_patch_list_aux`,
  `cg_concat`):
  ```rust
  let mut list = list;
  while list != NO_JUMP { let next = cg_get_jump(fs, list); /* mutate node */ list = next; }
  ```
- After: a `JumpList` **lending cursor** (not a plain `Iterator`, because every
  body needs `&mut FuncState` — the chain can't be borrowed across steps, so
  `fs` is handed back in per call):
  ```rust
  struct JumpList { cur: i32 }
  impl JumpList { fn next(&mut self, fs: &FuncState) -> Option<i32> {
      if self.cur == NO_JUMP { return None; }
      let pc = self.cur; self.cur = cg_get_jump(fs, pc); Some(pc) }}
  // visit-every:  while let Some(pc) = walk.next(fs) { /* mutate pc */ }
  // find-tail:    while let Some(pc) = walk.next(fs) { tail = pc; }
  ```
- Invariant that replaced the structural one: **the cursor yields the same pc
  sequence as the manual walk, so the jump offsets that get patched do not
  move.** `next` computes the successor *before* returning the current node, so
  a body may rewrite the yielded node without breaking the walk (the
  read-next-then-mutate discipline the hand loops relied on). Proven by bytecode
  parity AND three new unit tests (same-pcs-as-manual-walk, empty/single,
  cg_concat tail-link).
- Caveat: when the visit body mutates the structure the chain lives in, a
  borrowing `Iterator<Item=pc>` will NOT compile (the `&fs` it holds conflicts
  with the body's `&mut fs`). The *lending* cursor (`next(&mut self, fs)`) is the
  idiomatic answer — and it serves both "visit every node" and "find the tail"
  shapes from one type.

**Recipe: `crutch removal on graduation` (parser extension)**
- Same pattern as P1a, with two parser-specific judgment calls worth recording:
- **A bare `file.c:NNN` ref embedded in an otherwise-behavioral comment**: strip
  the `(lparser.c:1862)` coordinate, KEEP the behavioral prose (which attributes
  are legal, the exact error wording). The coordinate is the crutch; the wording
  is the spec.
- **Genuine vs stale `TODO(port)`**: this crate had 14. Most "when lua-X lands"
  TODOs were STALE (the dep landed; verify with grep before deleting — e.g.
  `lex_next`/`lex_lookahead` actually call `lua_lex::next`/`lookahead` now). But
  **8 are GENUINE deferred-codegen markers** (integer stack-check, debug-var
  startpc, local const-fold, explicit `fix_line`, GC proto allocation,
  single-vs-multret arg, an unnamed-var defensive branch). They were KEPT: the
  bytecode-parity oracle proves they don't move output, so they document a real,
  invisible divergence from full `lcode.c` — deleting them would hide it. The
  PORT STATUS trailer counts them honestly (`todos: 8`, `port_notes: 0`).
- Caveat: the most *misleading* crutch here was a section header claiming codegen
  "cannot yet be called from lua-parse ... emits the small subset of bytecode
  required to execute simple programs" — flatly false (the full single-pass
  generator is folded in and passes parity on the whole corpus). The
  graduation-era danger is exactly this: a stale comment that *understates* how
  done the code is. Replace it with what the code actually does.

## Verdict ledger
(append per-subsystem outcomes — graduated OR honest-negative-with-reason)

### P1a — lua-lex: **GRADUATED** (2026-06-14)

5/5 planned transformations landed, each its own commit, the full gate green
after every one. `GRADUATED.md` written; module doc + PORT STATUS trailer
declare the new oracle. 17 crate-local unit tests added (tier-2 net). `unsafe`
0 → 0. No entry added to `bytecode-parity-allow.txt`. `lua-parse` untouched.

**Honest-negatives (transformations attempted/considered and deliberately not
forced):**
- *Pushing `TokenKind` out to `Token.kind`* — the original deferred Phase-B note
  asked for a `TokenKind` enum replacing `kind: i32`. NOT done at the boundary:
  `lua-parse` and the error formatters read the `TK_*` `i32` codes directly, so a
  boundary enum would require editing `lua-parse` (out of scope) and provides no
  behavioral gain. Resolved internally via `Peek` instead; recorded the `i32`
  boundary as deliberate in `Token`'s doc. (Not a parity break — a scope wall.)
- *Returning `Err(...)` from the error constructors* — making `lex_error` etc.
  return `Result`/`!` instead of a bare `LuaError` would read marginally more
  idiomatic, but `lua-parse` wraps them as `Err(lex_error(...))` across the crate
  boundary, so the by-value return is a public contract. Left as-is and documented
  as the boundary rather than churned.
- *Moving `ZIO`/`LexBuffer` to `lua_vm::zio`* — explicitly out of scope (a
  separate planned refactor). Idiomatized in place; one `TODO` each marks the move.

No transformation had to be reverted for a parity/behavior break — every gate
stayed green on the first landing.

### P1b — lua-parse: **GRADUATED** (2026-06-14)

Three code transformations landed (crutch removal → `searchvar` `.rev()`
iterator → `JumpList` lending cursor), one commit each, the full Sprint-1 gate
green after every one: bytecode parity bench all-OK + broad-corpus divergent-op
counts **identical to baseline** (33 files; the pre-existing const-fold/LOADNIL
divergences did not move); `multiversion_oracle` 165; official 33/33;
`literals.lua` + `errors.lua` PASS; `check.sh` 5.1/5.2/5.3/5.4/5.5 =
57/54/23/7/10, 0 fail; `cargo test -p lua-parse` (4 new unit tests) +
`--workspace` green; wasm `cargo check` clean. `unsafe` 0 → 0. **No entry added
to `bytecode-parity-allow.txt`.** `GRADUATED.md` written; module doc + PORT
STATUS trailer declare the new oracle. `lua-lex`/`lua-vm`/`lua-code` untouched.

**P1c (CODEGEN) is SUBSUMED into P1b.** `lcode.c` is folded into `lua-parse`
(the 86 `cg_*` functions); the standalone `lua-code` crate is *only* the opcode
tables / `Instruction` encoding. There is no separate codegen crate to
idiomatize. The emit / register-alloc / jump-offset / line-info / constant-fold
**core was deliberately left structurally faithful** (the hot-loop exception in
the roadmap): its instruction ordering, register LIFO discipline, and
constant-insertion order are exactly what bytecode parity pins. We idiomatized
*around* that core (the jump-list *walks*, not the jump *offset math*), never
*through* it.

**Honest-negatives (transformations planned and deliberately NOT forced):**

- *`enter_level`/`leave_level` and `enter_block`/`leave_block` → RAII `Drop`* —
  **NOT done; left as explicit calls.** Two independent blockers:
  1. **Borrow + public-API.** A `Drop` guard must own a handle to the thing it
     cleans up. The depth counter (`recursion_depth`) and the block chain live
     *inside* `LexState`, which the guarded function bodies also mutate (`subexpr`
     calls `lex_next(ls, …)` etc.). A guard holding `&mut LexState` would forbid
     the body from touching `ls` at all. The borrow-safe alternative — making
     `recursion_depth` a `Cell<u32>` and holding `&Cell` — changes a **public**
     `LexState` field type, which the task forbids (byte-stable public boundary).
     `unsafe` is banned. So no borrow-safe, API-stable RAII exists.
  2. **`leave_block` ordering + extra args.** `leave_block` is ordering-sensitive
     (it snapshots the block's fields *before* popping, because `createlabel` /
     goto-resolution read `fs->bl` while it still points at the block; then
     restores 5.5 global-scope state, `remove_vars`, emits `OP_CLOSE`, resolves
     gotos, *then* pops). `Drop` cannot take the `&mut LuaState` it needs to emit
     `CLOSE`, and folding this sequence into `Drop` risks reordering the
     load-bearing snapshot→restore→resolve→pop chain.
  Conclusion: the explicit paired calls **are** the correct Rust shape here. The
  one "defect" RAII would fix (the depth decrement is skipped on `?`-error paths)
  is unobservable — a parse error aborts the whole `parse()`, after which
  `recursion_depth` is never read again, matching the C longjmp abort. Zero
  behavioral gain for real cost. (Not a parity break — a borrow/API/ordering
  wall.)

- *`ExprPayload` flat struct → tagged `enum` (THE marquee parser recipe)* —
  **NOT attempted as code; recorded honest-negative.** The recon's "all-or-
  nothing-safe (disjoint fields, no cross-talk)" was true at the field level but
  missed the access *shape*, which makes a faithful conversion a hundreds-of-site
  big-bang that is only gate-able at the very end. The structural blockers:
  1. **Generic setter writes the wrong-looking field for many kinds.**
     `init_exp(e, k, i)` sets `e.k = k; e.u.info = i` for **12 different
     ExprKinds**, including the no-payload `Nil`/`True`/`False`/`Void` and even
     `KInt`/`KFlt` (where the caller then overwrites `ival`/`nval` separately).
     It relies on the flat struct's "write `info` even when unused" semantics —
     an enum where the variant *is* the data cannot express that.
  2. **Tag and data are set in SEPARATE statements**, pervasively and sometimes
     far apart: `init_exp(v, KInt, 0)` then `v.u.ival = …` (simpleexp);
     `e1.k = ExprKind::KFlt` then `e1.u.nval = v` (constant folding). An enum has
     no valid intermediate state between "set the tag" and "set the data".
  3. **Helpers pass tag and payload as separate args** (`promote(e1.k, &e1.u)`).
  4. **Scale, all-or-nothing, hot path.** 12 `match e.k` + 48 `.k ==/!=` + 45
     `.k = ExprKind` + **161 `.u.field` accesses**, none of which compile (let
     alone gate) until the *entire* conversion is done, against the codegen hot
     path where a single mis-mapped field silently moves a constant index or
     register — invisible until the final parity run. This is exactly the
     "balloons beyond what you can gate cleanly → REVERT entirely and record a
     detailed honest-negative" outcome the task sanctioned. `ExprPayload` and the
     parallel `VarDesc` stay flat structs; their doc-comments now record this as
     the deliberate decision (the bytecode-parity oracle, not the type system,
     holds the "which field each kind uses" invariant).

No transformation that *was* landed had to be reverted for a parity/behavior
break — every gate stayed green on the first landing. The two negatives above
were never coded (the analysis ruled them out before writing risky code).
