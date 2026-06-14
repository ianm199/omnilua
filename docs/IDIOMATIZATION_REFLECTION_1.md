# Idiomatization Sprint 1 — Reflection (Phase 1 complete)

Date: 2026-06-14. Author: supervisor (Fable). Scope reflected on: Phase 0
(scaffolding) + Phase 1 (lexer `lua-lex`, then parser+codegen `lua-parse`), both
idiomatized and merged CI-green (PR #193 `b113ab7`, PR #194 `2ee0136e`). Plan of
record: `IDIOMATIZATION_ROADMAP.md`. Live ledger: `IDIOMATIZATION_SPRINT_1_SPEC.md`.

This document is the **required reflection checkpoint** before any Phase-2
(stdlib) work. The methodology is the product; this consolidates it.

---

## 1. What actually happened

| Subsystem | Lines | Idiomatization surface found | Outcome |
|---|---|---|---|
| `lua-lex` (P1a) | ~1900 | **Heavy** C-residue: ZIO index-triple, `str2num` out-param, borrow-dance copies, ~480 lines of verbatim-C comments, 17 PORT NOTEs | **Rich harvest** — 5 transformations, 5 recipes, graduated |
| `lua-parse` (P1b, +codegen) | ~6100 | **Narrow** residue: already `Result`/`Option<Box>`/enums, 0 unsafe; genuine residue was a reverse-index walk, 4 jump-list chain walks, ~14 crutch comments | **Restraint** — 3 transformations, 3 recipes, **2 honest-negatives**, graduated |

Both stayed **bytecode-parity-green at every commit**, unsafe stayed 0→0, no
public-API change, no allowlist entry added. The gate (bytecode parity +
`multiversion_oracle` 165 + official 33/33 + `check.sh` 5.1–5.5 = 57/54/23/7/10)
was re-run independently by the supervisor on each branch before merge — not
taken on the subagent's self-report.

The two subsystems were almost opposite in character, and **that contrast is the
central lesson of this sprint** (§3).

---

## 2. The recipe catalogue that emerged

Full before→after in the spec's "Recipe ledger". Distilled, with the axis each
recipe lives on:

**Control-flow / iteration**
- `c-index-triple → single-cursor reader` (lexer): a hand-decremented
  `(n, p, buf)` cursor where `n` shadows `len-p` → keep the cursor, derive
  "remaining". The invariant moved from "matches `llex.c`'s pointer math" to a
  unit test pinning the byte-then-EOZ sequence + bytecode parity.
- `manual reverse index walk → .rev() range iterator` (parser): a count-down
  `while i >= 0` shadow-resolution scan → `(0..n).rev()`. Caveat that became a
  *rule*: only when the index sequence and early-exit are the *entire* loop
  semantics; a counter entangled with deferred side effects (e.g. `remove_vars`)
  must stay manual.
- `sentinel-terminated chain walk → named lending cursor` (parser): the most
  transferable. A `while pc != NO_JUMP` linked-list walk whose body **mutates the
  structure the chain lives in** cannot become a borrowing `Iterator<Item=pc>`
  (the `&fs` it holds conflicts with the body's `&mut fs`). The idiomatic answer
  is a **lending cursor** — `next(&mut self, fs) -> Option<pc>` that computes the
  successor *before* yielding, so the body may rewrite the yielded node. One type
  served both "visit every" and "find the tail".

**Error handling / types**
- `c-int-status + out-param → Option` (lexer): `int f(in, T* out)` → a private
  wrapper reducing the convention to `Option<T>`, then `match`. Idiomatize at the
  **callsite** with a wrapper; do not chase the convention across a crate
  boundary (`str2num` itself lives in `lua-vm`).
- `integer-tag dispatch → internal enum at a stable boundary` (lexer): an
  internal `enum Peek { Byte(u8), Eoz }` for the scan loop, while the `TK_*`/`i32`
  codes the parser reads stay `i32`. The boundary stayed; only the internals got
  the enum.

**Ownership / buffers**
- `borrow-dance copy → named extraction method` (lexer): `.to_vec()` blocks that
  existed only to end a borrow before a `&mut` call → name the *intent* on the
  buffer type (`trim_ends`/`to_owned_text`/`without_trailing_nul`) and document
  the owned copy once as a real ownership requirement, not a C idiom. The copy
  was genuinely required; the recipe is about *naming* it, not eliminating it.

**Crutch removal (every graduation)**
- Strip `# C source` blocks, `file.c:NNN` coordinates, `*.tsv` annotations, PORT
  NOTEs. Two judgment calls from the parser worth keeping: (a) strip the
  `(lparser.c:1862)` coordinate but **keep** the behavioral prose around it — the
  coordinate is the crutch, the wording is the spec; (b) distinguish **stale**
  TODOs (the dep landed — verify with grep) from **genuine** deferred-behavior
  TODOs (8 in the parser flag real, parity-invisible divergences from full
  `lcode.c` — deleting them would hide a real gap).

**The meta-recipe that shaped everything:** *idiomatize the internals up to the
public boundary, then STOP and document the boundary as deliberate.* Half the
value of a recipe is knowing where it must not reach. Both pilots kept their
public surface byte-stable specifically so the next subsystem wasn't dragged in.

---

## 3. The verification-model shift — what it felt like in practice

The roadmap's thesis was abstract: "bytecode parity survives idiomatizing the
producer." In practice it was **stronger and more load-bearing than expected**,
in two directions:

**As a safety net it is extraordinary.** Because the lexer/parser's entire job is
to drive identical bytecode, a single reordered emit, a wrong constant index, or
a mis-decremented register shows up *instantly* at the next parity run. Combined
with one-transformation-per-commit, every step was **bisectable** — if parity had
moved, exactly one small commit was the suspect. This made aggressive internal
rewrites (the lexer's cursor, the parser's lending cursor) feel *safe* in a way
the behavioral suite alone never could: behavioral tests catch wrong *outputs*;
bytecode parity catches wrong *structure that happens to still produce right
outputs on the tested inputs*. Losing the C correspondence was a non-event
because parity replaced it completely and more cheaply (no `llex.c` to open).

**As a reframing it is the surprise.** On the parser, the oracle's strength
became the argument *against* changing code. When the net is that tight,
"already idiomatic" and "load-bearing structural" are two distinct reasons to
**leave code alone**, and the discipline becomes resisting the urge to churn the
84-site `.u.info` / 202-site `match e.k` surface just because it "looks like C".
The oracle reframes the job from "make it pretty" to "find the *genuine* residue
and idiomatize exactly that." This is the opposite instinct from Phase A
(translate everything); Stage 2 is subtractive and selective.

**What bytecode parity did NOT cover, and what carried it:** parity ran on the
bench corpus, which is lexically/grammatically narrow. The **version gates and
exact error wording** — the most delicate surface — were held by the *behavioral*
nets: `multiversion_oracle` (165 version-pinned assertions), `check.sh` per
version, and `errors.lua`/`literals.lua`. The lesson: even in a
bytecode-parity-net subsystem, the structural oracle and the behavioral oracle
are **complementary, not redundant** — parity pins *structure on common inputs*,
behavioral pins *version-specific wording and edge inputs*. Both were required
to call a subsystem green.

---

## 4. The "idiomatization debt is not uniform" finding

The single most actionable surprise. The lexer and parser are both faithful
ports from the same era of the same port effort, yet:

- the **lexer** carried heavy C-shape (pointer arithmetic, out-params, 480 lines
  of C comments) and yielded 5 recipes;
- the **parser** was **already mostly idiomatic** (someone ported it into Rust
  shapes from the start) and yielded 3 small recipes + 2 deferrals.

Line count is a **terrible** proxy for idiomatization work: the 6100-line parser
had a *smaller* real surface than the 1900-line lexer. **Recon-before-sizing is
mandatory** — both packets were preceded by a read-only recon agent that mapped
the actual residue, and in the parser's case that recon is what prevented a
wasted "rewrite 6000 lines" framing. For Stage 2 generally: **measure a
subsystem's genuine residue first; size the packet to the residue, not the file.**

---

## 5. Honest-negatives are the richest output

Two transformations were analyzed and deliberately NOT forced. Both are more
instructive than the recipes that landed.

**`enter/leave` → RAII `Drop` (ruled out).** A `Drop` guard must own a handle to
what it cleans up, but the depth counter and block chain live inside `LexState`,
which the guarded bodies also mutate — a guard holding `&mut LexState` forbids the
body from touching `ls`. The borrow-safe alternative (a `Cell` field) changes a
public type; `unsafe` is banned. And `leave_block` is ordering-sensitive
(snapshot→restore→resolve→emit `CLOSE`→pop) and needs `&mut LuaState` that `Drop`
can't take. **The explicit paired calls *are* the idiomatic Rust shape here.**
The one defect RAII would fix (depth decrement skipped on `?`-error) is
unobservable: a parse error aborts `parse()`, mirroring the C `longjmp`. *Lesson:
RAII is not automatically more idiomatic when cleanup needs context the
destructor can't hold and ordering the destructor can't guarantee.*

**`ExprPayload` flat-struct → tagged enum — the marquee recipe, deferred.** This
is the canonical "tagged union → Rust enum" transformation and the most valuable
single piece of "how you go from C to idiomatic Rust" the sprint could have
produced. It was deferred — *and that deferral, with its reasoning, is the
deliverable.* Why the naive conversion is an un-gateable big-bang:
- `init_exp(e, k, i)` sets `e.k = k; e.u.info = i` for **12 ExprKinds**, including
  no-payload `Nil`/`True`/`False` and `KInt`/`KFlt` (where the caller then
  *separately* overwrites `ival`/`nval`). It relies on flat-struct "write `info`
  even when unused" semantics an enum cannot express.
- **Tag and data are set in separate statements**, pervasively and sometimes far
  apart (`init_exp(v, KInt, 0)` … later … `v.u.ival = x`). An enum has no valid
  intermediate state between "tag set" and "data set".
- **161 `.u.field` reads + 45 `.k =` writes + 48 `.k ==` + 12 `match e.k`**, none
  of which compile — let alone gate — until the *entire* conversion is done,
  against the codegen hot path where one mis-mapped field silently moves a
  constant index, invisible until the final parity run. Big-bang + un-bisectable +
  zero behavioral gain (bytecode parity, not the type system, already holds the
  per-kind field invariant).

**The technique a future attempt should use (not explored by the pilot — a
supervisor addition):** decompose the big-bang into **two gateable phases**.
*Phase 1:* introduce atomic constructor functions (`set_expr(e,
ExprKind::KInt(x))`) and convert all `init_exp(...) + separate .u.field=` write
sites to atomic construction **while keeping the flat struct** — each batch is
behavior-preserving and parity-gateable, killing the "separate-statement /
invalid-intermediate-state" blocker. *Phase 2:* mechanically flip the now-always-
atomically-constructed flat struct to an enum and convert the 161 read sites.
Phase 1 is low-risk and independently valuable (atomic construction *is* more
idiomatic than init-then-mutate); Phase 2 remains a read-side big-bang but is
purely mechanical once construction is atomic. **This is a hard, cosmetic,
hot-path change with zero behavioral gain — it belongs in a *supervised* session,
not an unattended run, and is the user's call to greenlight.**

**The hot-loop exception generalized.** The roadmap named `vm::execute` as the
one place to leave alone. The parser proved the principle applies *inside a cold
subsystem*: the emit/register-alloc/jump-offset/line-info/constant-fold core (the
`cg_*` functions) is load-bearing structure even though `lua-parse` as a whole is
cold. We idiomatized *around* it (the jump-list *walks* became a cursor) and never
*through* it (the offset *math* is untouched). **"Cold subsystem" does not mean
"all of it is safe to restructure" — a subsystem can contain a load-bearing core.**

---

## 6. Roadmap adjustments

1. **P1c (codegen) is subsumed into P1b.** `lcode.c` is folded into `lua-parse`
   (86 `cg_*` functions); the standalone `lua-code` crate is only opcode
   tables/`Instruction` encoding. The roadmap's "lexer → parser → codegen" is
   really **"lexer → parser(+codegen)"**. There is no third Phase-1 subsystem.
   Phase 1 is therefore **done**.
2. **Recon-before-sizing is a required step**, not optional. Size packets by
   genuine residue (§4).
3. **The hot-loop exception is per-*core*, not per-*crate*** (§5). A graduation
   doc must name the load-bearing core that was left structurally faithful.
4. **Both oracle tiers are required even in Phase 1** — bytecode parity (structure
   on common inputs) + behavioral (version wording/edges) are complementary (§3).
5. The graduation artifact set that worked and should be standard: per-subsystem
   `GRADUATED.md` (what the C correspondence was, the oracle that now guards it,
   what to trust instead, the load-bearing core, the honest-negatives) + recipe
   entries + verdict entry in the sprint spec + one-transformation-per-commit
   history.

---

## 7. Phase-2 go/no-go + recommended scope

**Verdict: GO — with a precondition and a sharper sequence.** Phase 2 (stdlib) is
the right next step, but it is where the **structural oracle disappears**. Stdlib
modules emit *behavior*, not bytecode; the only net is the behavioral suite. The
confidence that made Phase 1 feel safe (parity catches structural wrongness
instantly, bisectably) **drops sharply**. The recipes themselves transfer
(iterators, `Result`/`?`, lending cursors, crutch removal are net-independent),
but the *verification* gets coarser, exactly as the roadmap's "descending oracle
strength" predicted.

**Precondition (new, important):** before idiomatizing any stdlib module, *verify
its official-test coverage is strong enough to stand alone.* In Phase 1 we could
idiomatize freely because parity was near-total; in Phase 2, a module with weak
official coverage has a weak net, and idiomatizing it is unsafe. **Coverage-check
each module first; if coverage is thin, adding tests is the precondition, not an
afterthought.** This is the Phase-2 analog of "recon-before-sizing."

**Recommended Phase-2 sequence:**
1. **Easy, pure, well-covered first** — `math`, `os` date/time arithmetic,
   `table`. Behavior is fully pinned by the official suite; these build the
   "behavioral-only discipline" on the safest available ground (the same
   strongest-net-first logic, one tier down).
2. **Then the gnarly + hot** — the string-pattern matcher (`match_pat` et al.).
   This carries an **added gate the Phase-1 subsystems did not need**: it is hot,
   so it requires the Ir + cold-machine-wall perf arbiter (per
   `MEASUREMENT_PROTOCOL.md` and the T5a lesson), not just the behavioral oracle.
   Idiomatization that regresses CPI is a no-go even if behavior is identical.

**Recommended Phase-1 coda (optional, user's call):** the `ExprPayload` enum via
the two-phase technique (§5) — the marquee recipe the sprint deferred. High
methodology value, zero behavioral value, hot-path risk; **supervised only.** If
the user wants the canonical tagged-union→enum worked example, this is where it
lives. Otherwise the documented honest-negative stands as a complete deliverable.

**Do NOT** start Phase 2 in the current unattended run — this reflection is the
checkpoint; the go/no-go above is the input to a *deliberate* Phase-2 kickoff.

---

## 8. The one-paragraph version (for the harness retrospective)

Stage-2 idiomatization works, and the bytecode-parity oracle is what makes the
producer subsystems *safe* to idiomatize: it survives rewriting the producer, it
is bisectable per-commit, and it reframes the work from "beautify" to "find the
genuine residue." The two pilots spanned the full range — a heavily-C-shaped file
(rich recipe harvest) and an already-idiomatic file (the discipline of restraint
plus a well-reasoned deferral) — which together teach the actual skill:
**transform aggressively where the residue is real, leave the load-bearing core
and the already-idiomatic majority alone, and defer-with-a-plan where a canonical
recipe would be an un-gateable big-bang for zero behavioral gain.** The honest-
negative (when *not* to apply the textbook transformation, and how to de-risk it
if you must) is the most valuable training data the sprint produced — more than
any recipe that landed. Phase 2 is a go, but it trades the strongest net for the
behavioral one; coverage-check each module before touching it.
