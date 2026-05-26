# Supporting multiple Lua versions (5.1–5.4) in one repo

Internal design note. How other projects manage multiple language versions in
one codebase, and what it would actually take for lua-rs.

## How mlua does it (and why it doesn't transfer)

mlua supports Lua 5.1 / 5.2 / 5.3 / 5.4 / 5.5 / LuaJIT / Luau from one crate via
**mutually-exclusive cargo features** (`lua54`, `lua53`, `lua52`, `lua51`,
`luajit`, `luau`, …). The features delegate to a `mlua-sys` FFI layer that links
the matching version. You pick one at build time:

```toml
mlua = { version = "0.11", features = ["lua54"] }   # swap to "lua51" etc.
```

The catch: **mlua is a *binding*. It links the real, prebuilt C Lua for each
version — it doesn't reimplement them.** Its feature flags select *which C
library to link*, and `#[cfg(feature = "lua54")]` gates the handful of Rust API
differences. That option isn't available to lua-rs, which is a from-scratch
implementation. "Support 5.1" for lua-rs means *implementing* 5.1's language and
stdlib, not linking a different `.a`.

So mlua's mechanism (cargo features for version selection) is a good model for
the *interface*, but the *cost* for us is entirely different: it's real
implementation work per version.

Most from-scratch implementations sidestep this by picking one version: Fengari
(JS) targets 5.3/5.4, LuaJIT is 5.1 + extensions. Supporting several versions in
one from-scratch codebase is genuinely rare.

## What's actually different between the versions

Grouped by the subsystem it touches. This is the real scope.

**5.1 → 5.2**
- `_ENV` lexical environments; removal of `setfenv`/`getfenv` and the old module
  system. (Parser + runtime; "the bulk of code adaptation" per the manuals.)
- `goto`/labels (lexer + parser).
- `bit32` library; `__pairs`/`__ipairs`; yieldable `pcall`/metamethods;
  finalizers for tables; light C functions.
- `unpack` → `table.unpack`, `loadstring` → `load` (stdlib).

**5.2 → 5.3 (the deep one)**
- **Integer subtype**: numbers split into 64-bit integer and float. This is the
  hardest change — it lives in the value representation (`LuaValue`) and touches
  every arithmetic, comparison, and string-coercion path.
- Bitwise operators `& | ~ << >>`; integer division `//` (lexer + parser + VM).
- `string.pack`/`unpack`, `utf8` library, `math.type` (stdlib).
- `bit32` deprecated; `__ipairs` removed.

**5.3 → 5.4**
- `<const>` and `<close>` attributes — to-be-closed variables (lexer + parser +
  VM + a closing mechanism on scope exit).
- Generational GC mode; `warn`/warning system; `coroutine.close`.
- Integer for-loop semantics; string-to-number coercion change (`"10"+1` is
  integer `11` in 5.4 vs float `11.0` in 5.3); `math.random` algorithm change.

The dominant cost is the **5.3 integer split** (it reshapes the value type and
all numeric paths) plus the **5.1/5.2 `_ENV`/module** differences. lua-rs is
already 5.4, so it has integers, `<close>`, generational GC, etc. — going
*backwards* to 5.1/5.2 means re-introducing float-only numbers and the old
environment model, which is awkward precisely because 5.4 assumes the newer ones.

## Architecture options for a from-scratch implementation

**A. Compile-time feature flags (mlua-style).** `features = ["lua54" | "lua53" |
"lua52" | "lua51"]`, mutually exclusive. `#[cfg]`-gate lexer keywords, parser
productions, opcodes, the value type, and stdlib sets.
- Pro: zero runtime cost; clean per-build; matches the ecosystem's mental model
  (mlua users already think in these features).
- Con: one version per binary; `#[cfg]` sprawl across the whole codebase
  (worst around the integer split); the test matrix multiplies by version.

**B. Runtime version enum.** A `LuaVersion` threaded through lexer / parser / VM
/ stdlib; branch on it. `Lua::new(Version::Lua51)`.
- Pro: one binary runs any version — ideal for an embedding API where the host
  picks per state (a game on 5.1 and an app on 5.4 use the same crate).
- Con: runtime branches in hot paths (perf); the value type must carry the
  integer representation always and merely *disable* it for 5.1/5.2 semantics
  (awkward); bigger binary.

**C. Shared core + per-version frontends.** Common VM/runtime; version-specific
lexer/parser modules and stdlib sets behind a thin semantic-config.
- Pro: shares the ~80% that's identical; most maintainable when divergences are
  large.
- Con: most up-front structure.

## Recommendation for lua-rs

- **Demand reality:** the versions people actually embed are **5.4** (modern) and
  **5.1 / LuaJIT** (games, Neovim, OpenResty — a huge install base). 5.2 and 5.3
  are comparatively low-demand. So the high-value target is 5.4 (done) + **5.1**,
  not all four. 80/20.
- **Architecture:** runtime selection (option B) is more valuable for the
  embedding API (host picks per state), but more work and a perf cost. Feature
  flags (option A) are simpler and match mlua's model. Lowest-friction path:
  start with feature-flags per version, accept one-version-per-build, and move to
  runtime selection only if the embedding API demands it.
- **Sequence:** factor the version into a config first (even a const), get the
  conformance suite running per-version, then tackle the integer split (the hard
  core for going below 5.3) and the `_ENV`/stdlib differences.
- **Honest scope:** 5.1–5.4 in one from-scratch implementation is a multi-month
  effort dominated by the integer split and `_ENV`. It's almost certainly not
  worth doing before the embedding API and sandboxing land. But **5.1 support**
  is plausibly the single highest-value addition *after* those, because it opens
  the large game/Neovim/OpenResty embedding market that's stuck on 5.1/LuaJIT.

## Sources

- mlua (Lua 5.1–5.5 + LuaJIT + Luau via cargo features): <https://github.com/mlua-rs/mlua>
- Lua 5.1→5.4 breaking changes: <https://gist.github.com/vadi2/febb785806e5eb962ed8c309000192ff>
- Lua version history: <https://www.lua.org/versions.html>
- Lua 5.3 incompatibilities (integer split): <https://www.lua.org/manual/5.3/manual.html#8>
- Lua 5.4 reference (to-be-closed, generational GC): <https://www.lua.org/manual/5.4/manual.html>
