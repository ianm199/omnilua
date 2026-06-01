# `specs/followup/` — multi-version phase artifacts

Working notes from the phased push that brought **Lua 5.1–5.5** to oracle-verified
parity (shipped in v0.0.22). These are the discover/triage findings and the
per-phase reports — the durable record of *what diverged from the reference and
why*, captured against the upstream binaries. The reusable methodology distilled
from all of it lives in [`../MULTIVERSION_PLAYBOOK.md`](../MULTIVERSION_PLAYBOOK.md).

The workflow orchestration scripts that produced these (`phase*.mjs`) were
throwaway process scaffolding and are not retained.

## Per-phase reports (start here)

| Report | Phase |
|---|---|
| `PHASE_A_REPORT.md` | reference-fidelity bugs #76–79 (v0.0.20) |
| `PHASE_B_REPORT.md` | finish Lua 5.3 (v0.0.21) |
| `PHASE_C_REPORT.md` | finish Lua 5.5 |
| `SHARED_CORE_REPORT.md` | cross-version fidelity batch |
| `PHASE_D_5.2_REPORT.md` | Lua 5.2 (the float-only bridge) |
| `PHASE_D_5.1_REPORT.md` | Lua 5.1 (fenv legacy family) |
| `HARD_PROBLEMS_REPORT.md` | architectural deferrals; see issues #92–97 |

## Discover / triage / design notes (per topic)

- **5.3**: `5.3-divergences.md`, `5.3-math.md`, `5.3-coerce-err.md`
- **5.5**: `5.5-divergences.md`, `5.5-lang.md`, `5.5-stdlib-err.md`
- **5.2**: `5.2-numbers.md`, `5.2-syntax-roster.md`
- **5.1**: `5.1-fenv.md`, `5.1-roster-syntax.md`, `5.1-numbers-prng.md`
- **shared core**: `sharedcore-triage-1.md`, `-2.md`, `-3.md`
- **traceback**: `79d-design.md`
- **confirmations** (the #76–79 adversarial confirms): `confirm-76.md` … `confirm-79.md`

## The remaining architectural backlog

Tracked as GitHub issues #92–97: debug line-hook fidelity, generational GC
default mode, 5.5 named-vararg `...` aliasing, `break`-outside-loop wording,
5.3 loop-built-closure equality caching, `__le`-from-`__lt` across a yield.
Precise re-entry notes are in `SHARED_CORE_REPORT.md` and `HARD_PROBLEMS_REPORT.md`.
