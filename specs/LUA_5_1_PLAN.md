# Lua 5.1 implementation plan (the "dessert")

Concrete plan for adding 5.1 (and 5.2 as the bridge) behind `LuaVersion::V51`,
informed by probing the reference binaries (`/tmp/lua-refs/bin/lua5.1.5`,
`lua5.2.4`) against `lua5.4.7`. Builds on the research in
`specs/research/5.1-5.2-upstream.md`. Status: PR #75 currently **refuses** 5.1/5.2
(no masquerade, review H2); this is the roadmap to make them real. A first spike
lives on the `lua-5.1-spike` branch.

## The surprise: most of 5.1's *observable* number behavior is cheap

The 5.1 "float-only" model sounds like it needs a second value representation,
but probing shows the observable surface is small, because **5.1 has no
`math.type`, no `//`, and no bitwise operators** — so whether `3` is stored as
an integer or a float internally is *not observable* in almost all programs. The
two things that actually show:

1. **`.0`-suppression in float `tostring`.** 5.1 prints integer-valued floats
   without a trailing `.0`: `10/2` → `5` (5.4: `5.0`), `2^2` → `4` (5.4: `4.0`).
   This is one branch in `number_to_str_buf` (skip the `.0`-append for
   5.1/5.2) — we already have the hook.
2. **`math.type` (and `math.maxinteger`/`mininteger`/`tointeger`) absent.** A
   stdlib-roster gate, exactly like the 5.3 `bit32`/`warn` gates.

`tostring(3)`, `3 == 3.0`, `math.floor(3.5)`, `1e15` all already match 5.1. So we
can keep the modern core's internal int/float values and reproduce 5.1's
*observable* numbers with `.0`-suppression + the `math.type` gate. (Genuinely
float-only internals would only matter for deep edge cases; defer.)

## Cheap & safe (gate like the 5.3 deltas — no number-model rewrite)

- **`.0`-suppression** under 5.1/5.2 (formatting).
- **Stdlib roster**: remove `math.type`/`math.maxinteger`/`math.mininteger`/
  `math.tointeger`; add `loadstring` (= `load`), global `unpack` (= `table.unpack`),
  `math.mod`/`math.log10`/`math.pow`/`math.atan2`/`math.ldexp`/`math.cosh`/…
  (the 5.1 math surface), `newproxy`, `table.getn`/`setn`/`maxn`. (5.1-only.)
- **Syntax rejection** under 5.1: `//`, bitwise `& | ~ << >>`, `goto`/`::labels::`,
  `<const>`/`<close>` attributes, `\x`/`\z`/`\u` string escapes, hex-float
  literals. (5.2 re-allows `goto`, `\x`, `\z`; gate per-version.)
- **`_VERSION`** already works off the flag.

This subset makes a large fraction of real 5.1 scripts run correctly and is all
version-gated, so it can't regress 5.3/5.4/5.5.

## The hard part: function environments (fenv globals)

5.1 has **no `_ENV`**. Globals go through a per-function *environment table*
reached by `OP_GETGLOBAL`/`OP_SETGLOBAL`, and `getfenv`/`setfenv` read/replace a
function's environment. Our modern core is built entirely on the `_ENV` upvalue
model (5.2+). Faithfully supporting 5.1 globals means either:

- (a) lowering 5.1 global access onto the existing `_ENV`-table machinery and
  emulating `getfenv`/`setfenv` by swapping the `_ENV` upvalue's table — viable
  for the common cases, leaky for the reflective ones; or
- (b) a real per-closure environment slot — closer to 5.1 but a deeper change.

This is the main reason 5.1 is "dessert," not a quick gate. **5.2 is the bridge**:
it is float-only (shares 5.1's number observability) but already uses `_ENV`
(shares the modern globals path), so implementing 5.2 first exercises the
float-only + roster + syntax work *without* the fenv fork, and 5.1 then adds only
the globals subsystem on top.

## Recommended sequencing

1. **5.2 first** (bridge): `.0`-suppression + roster + syntax gates + `_ENV`
   (reused). Oracle: `lua5.2.4`. No fenv, no new number representation.
2. **5.1**: 5.2 minus `goto`/`\x`/`\z`/`bit32`, plus the fenv globals subsystem
   (`getfenv`/`setfenv`, `OP_GETGLOBAL`-equivalent lowering) and the 5.1 stdlib
   add-backs (`loadstring`, `unpack`, `module`, `table.getn`, …).
3. Decide later whether genuinely float-only internals are worth it (only matters
   for edge cases that 5.1 can't even observe today).

## Oracle

Pin `lua-5.1.5` and `lua-5.2.4` (built `make macosx`, already in `/tmp/lua-refs`).
Extend `specs/oracle/diff_one.sh`/`check.sh` with 5.1/5.2 cases. Note the 5.1
official test suite is older/differently-shaped (no drop-in `all.lua`); plan a
curated corpus rather than the whole suite.

## Effort

Cheap subset (5.2-shaped, no fenv): comparable to the 5.3 delta work — days.
The fenv globals subsystem for 5.1: the real cost — meaningfully larger, its own
oracle-gated change. Net: 5.2 is near-term; full 5.1 is a genuine project.
