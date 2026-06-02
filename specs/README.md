# specs/ — methodology + port evidence

Two kinds of document live here. **Read the "living" set; treat the rest as a
historical record** — point-in-time evidence captured while reverse-engineering
each Lua version against its reference binary. The historical docs are kept
because they are expensive-to-recreate knowledge, not because they describe the
current state. For current state: GitHub issues + `CHANGELOG.md` + the harness.

## Living (consult these)

| Doc | What |
|---|---|
| `MULTIVERSION_PLAYBOOK.md` | **The** methodology for adding/fixing a Lua version — non-negotiables, the iteration ladder, the version seam map, the release flow. Read before any version work. |
| `oracle/CONTRACT.md` | The pinned reference binaries (`/tmp/lua-refs/bin/lua5.x`), how to (re)build them, and which `LUA_COMPAT_*` defaults are part of the contract. |
| `oracle/diff_one.sh`, `oracle/check.sh` | The multi-version snippet oracle (see `harness/CLAUDE.md`). |
| `MULTIVERSION_ARCHITECTURE_DECISION.md` | Why one runtime-flagged core (not a `dyn` engine). The rationale of record. |
| `followup/issue-93-generational-gc-plan.md` | Living design for the generational GC frontier (#104/#113). |
| `followup/REPO_STRENGTHENING_IDEAS.md` | Running list of frontier ideas / honesty caveats. |

## Historical port evidence (keep, don't read as current)

These were captured during the 5.1–5.5 push and pin *what the reference did* at
the time. Useful when you re-open a version's behavior; **not** a status source.

- `followup/PHASE_*_REPORT.md`, `SHARED_CORE_REPORT.md`, `HARD_PROBLEMS_REPORT.md`
  — per-phase verification reports (also the re-entry notes for deferred items).
- `followup/5.x-*.md` — per-version research (numbers, fenv, roster/syntax,
  coercion/error wording, math, divergences).
- `research/*.md` — upstream-delta analyses (5.1/5.2, 5.3, 5.5).
- `adversarial/*.md` — adversarial test findings (the cases that broke naïve
  batteries).
- `LUA_5_1_PORT_SPEC.md`, `LUA_5_3_AND_5_5_PORT_SPEC.md`,
  `WEBLUA_MULTIVERSION_API_SPEC.md`, `MULTIVERSION_PRELIM_REVIEW.md` — point-in-time
  port specs / reviews.
The original Phase-A C→Rust translation rulebook is `../PORTING.md` (kept at root
because ~17 `.rs` files cite it in their PORT STATUS trailers). The port is
complete; the still-enforced code-style rules now live in `../CLAUDE.md`.

`followup/README.md` is the local index for the `followup/` subfolder.
