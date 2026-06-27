# #234 — Multi-version capability seam: implementation spec (rev 4, final)

**Status:** spec, post-codex-review rounds 1–3 (each REVISE; all findings accepted
and folded in). Architecture decisions (Engine-struct deferral §0, direct-host-only
`Unsupported` §3) were explicitly **approved** in round 2. The round-3 findings are
probe/API-detail corrections (now incorporated), not architecture — so the design
is settled and this proceeds to execute. The oracle (reference-fixture generation +
official suites) is the final truth-teller, per project rule.

> **Round-3 review note (codex ran the reference binaries to verify).** Three
> concrete fixes folded in: (1) parse probes are not 5.1-safe — `load(string)`
> errors on 5.1 (`loadstring` is the loader); all probes now `pcall` +
> `(loadstring or load)` (§2.3). (2) `EnvSandbox` is a two-part semantic probe
> covering both `_ENV` and `load(.., env)` (§2.3). (3) `Lua::supports` must AND
> version capability with compile-time stdlib-feature availability, distinct from
> the build-independent `LuaVersion::supports` (§2.4/§4).

> **Round-2 review note.** Codex caught the cardinal-rule violation: rev 2's
> cross-check probed *our own engine* (`Lua::new_versioned`), which is circular
> once `init.rs` consumes the matrix — a bad row removes the lib and the
> self-probe agrees. **The reference binaries are the only authority.** Rev 3
> makes a *reference-generated fixture* the truth of record (§2.3), drops two
> rows whose reference probe is not clean (`GcGenerationalMode`,
> `TableGcMetamethod`), corrects the under-covering probes (`EnvSandbox`,
> `StringPack`, `NativeBitwise`, `GcParam`), and re-exports `Feature`/`Unsupported`
> from `omnilua` (§3).
**Parent:** `specs/WEBLUA_MULTIVERSION_API_SPEC.md` (the design); this is the
*implementation* contract reconciled with the code as built.
**Predecessor:** slice 1 (host→Lua number-model seam: `LossyIntPolicy`,
`lower_host_int`, `LuaVersion::number_model`) shipped in 0.3.7.

> **Round-1 review note.** Codex caught a real bug: my draft matrix said `bit32`
> is "5.2 only" (copied from parent §3.4); `bit32` is present on **5.2 and 5.3**
> (verified: `type(bit32)=="table"` on both, matching the reference;
> `init.rs:116`). That I could not hand-write the matrix correctly is the
> argument *for* making the live engine the authority (§2.3). All seven findings
> are folded in below.

---

## 0. The central reconciliation (read first)

`WEBLUA_MULTIVERSION_API_SPEC.md` §4.1/§6 specifies internal dispatch as an
`enum Engine` with **per-version backend structs** (`v51::Engine`, …) and a §6.5
plan for 5.1/5.2 as a **separate core**.

**This spec does not build per-version backend structs, for this release.**
omniLua runs all five versions from a *single versioned core*: version is resolved
once in a cold path (`GlobalState.lua_version`, the `legacy_for` flag;
`lib.rs:399`), the hot bytecode loop is version-free, and all five official suites
pass against it. There is **no correctness or performance reason** to refactor
into per-version engines while that holds, and doing so now would be a large
behavior-risking change to reach goals the single core already meets
(multi-version from one binary, version chosen at runtime, common cases
byte-identical).

This is **not** a claim that §4.1 is wrong forever. Its `#[cfg]`-gated-variant
argument — a slim single-version build collapsing to a no-op dispatch for
mlua-class size/perf — is a **legitimate future build-size lever**. It is simply
*orthogonal* to making the multi-version surface usable, and belongs to its own
issue with its own justification. We are deferring it, not refuting it.

**What §4.1's `Backend` trait actually wants here is a version-indexed capability
descriptor — data, not VM structs.** Its listed contract (`number_model()`,
roster, gc surface, *the divergence registry*) is a per-version profile;
`number_model()` already lives on `LuaVersion`. This spec adds the rest as a
**`VersionProfile` / capability registry** keyed by `LuaVersion`. Trait-as-contract
becomes table-as-contract, because our dispatch is one core, not N.

The implementable, strategically-valuable core of #234:

> **Make the version capability matrix queryable at the API boundary, have the
> code that already gates on version *consume* it (one source where cheap), guard
> the rest with an exhaustive live-engine cross-check, and give the host a typed
> `Unsupported` error (plus a pre-check) at host-API verbs that name a
> version-absent feature.**

This closes the audit gap: *the multi-version differentiator is inert at the API
level.* After this a host can ask `lua.supports(Feature::Utf8Lib)`, render the
matrix, and get a typed error instead of a bare Lua "index nil" when it drives a
version-absent host verb.

---

## 1. Scope

### In scope
- **A. `Feature` enum + capability matrix** in `lua-types` (`version.rs`),
  capability-granular (no bundled or behavioral rows), `LuaVersion::supports`,
  `LuaVersion::features()`.
- **B. Single-source where cheap (Finding 2):** retrofit the *existing* inline
  version gates for library presence in `lua-stdlib/init.rs` (`utf8`, `bit32`) to
  **consume `version.supports(Feature::…)`**, so the matrix is the genuine source
  for those rows — not a second copy beside them.
- **C. Reference-backed fixture is the authority (round-2 Finding 1):** a
  committed `ANALYSES/version_feature_matrix.tsv`, generated by probing the
  *reference binaries* for every `(version, feature)`, against which a test
  asserts `supports()`. The matrix is never trusted on its own, and never
  validated against our own engine (which would be circular once §B lands).
- **D. Typed `Unsupported { feature, version }`** on the public `omnilua::Error`
  as a **direct-host-only** classification (Finding 3): one constructor couples
  message+payload; `as_unsupported()`/`is_unsupported()` detect it; docs and a
  test make explicit it does **not** survive an `Error→LuaError→Lua` round-trip
  (the trampoline's `From<Error> for LuaError` drops wrapper metadata,
  `lib.rs:226`). #234's only wiring returns it directly to the host, so this is
  sufficient and honest.
- **E. Wire one real host-API divergence point:** `gc().is_running()` on a
  version lacking it returns `Err(Error::unsupported(Feature::GcIsRunning, v))`
  instead of the raw "invalid option" Lua error; `Lua::supports` is the
  pre-check. The gate consumes the matrix (Finding 7), not an inline `>= V52`.

### Out of scope (explicitly)
- **Per-version `Engine`/`Backend` structs** and §6.5's separate 5.1/5.2 core —
  deferred to a future slim-build issue (§0), not refuted.
- **Reclassifying *script-level* feature use** (a 5.1 script calling `utf8.len`).
  That is an oracle-correct runtime "index nil" error; `Unsupported` is for
  host-API verbs we own the entry point to.
- **Converging *syntax/behavioral* gates (parser/lexer: goto, `<close>`, native
  bitwise) onto the matrix.** Those gates are deeper and more scattered;
  retrofitting them is its own change. Here they are **cross-check-tested only**
  (§C), with convergence called out as follow-up. The matrix is honestly a
  *tested mirror* for those rows, a genuine *source* only for the stdlib rows in
  §B.
- **Behavioral divergences** (`<=`-from-`__lt`, `for`-wrap, RNG stream): these are
  *not* `Feature`s (same call, different result, not present/absent). They live in
  a one-line doc list, not the enum (Finding 5).
- **Typed `LossyIntConversion`** (promoting slice 1's string error): optional
  polish, sequenced after A–E, dropped if review says the string suffices.

---

## 2. Part A/B/C — the capability matrix

### 2.1 `Feature` (capability-granular; `lua-types/src/version.rs`)

Each variant is a **host-visible present/absent capability**, split fine enough
that a host gating on one thing doesn't accidentally gate on five (Finding 5).
Behavioral divergences are excluded by construction.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Feature {
    IntegerSubtype,       // integer/float subtypes, math.type — 5.3+
    EnvSandbox,           // _ENV, load(.., env) — 5.2+
    FenvSandbox,          // setfenv/getfenv — 5.1 only
    GotoLabels,           // goto / ::labels:: — 5.2+
    NativeBitwise,        // & | ~ << >> and // — 5.3+
    Bit32Lib,             // bit32 library — 5.2, 5.3   (FIXED: not 5.2-only)
    Utf8Lib,              // utf8 library — 5.3+
    StringPack,           // string.pack/unpack/packsize — 5.3+
    CloseAttribute,       // <close> + __close — 5.4+
    ConstAttribute,       // <const> — 5.4+
    CoroutineClose,       // coroutine.close — 5.4+
    WarnFunction,         // warn — 5.4+
    TableLenMetamethod,   // __len on tables — 5.2+
    GcIsRunning,          // collectgarbage("isrunning") — 5.2+
    GcParam,              // collectgarbage("param", ...) — 5.5
    GlobalKeyword,        // `global` decl + declared-global scope — 5.5
    NamedVararg,          // function f(a, ...t) — 5.5
    TableCreate,          // table.create — 5.5
}
```

Two rows from rev 2 are **dropped from v1** because no clean reference probe
exists (Codex round-2 Findings 1–2 — a row the oracle can't unambiguously verify
is the drift bug, re-introduced): `GcGenerationalMode` (option-acceptance ≠ mode:
the `"generational"` option is accepted on 5.2 returning `0`, rejected on 5.3,
returns mode strings on 5.4/5.5 — "can select generational mode" has no
single-call probe) and `TableGcMetamethod` (a `__gc`-on-tables probe depends on
collection timing — flaky as an oracle assertion). Both can return later with a
precise reference probe.

`Feature::ALL: [Feature; N]` sits adjacent; a unit test asserts `ALL` is complete
by exhaustive `match` (adding a variant without adding to `ALL` fails to compile
the test).

### 2.2 `supports` — the matrix

```rust
impl LuaVersion {
    pub fn supports(self, f: Feature) -> bool {
        use Feature::*; use LuaVersion::*;
        match f {
            IntegerSubtype | NativeBitwise | Utf8Lib | StringPack
                                     => matches!(self, V53 | V54 | V55),
            EnvSandbox | GotoLabels | TableLenMetamethod | GcIsRunning
                                     => matches!(self, V52 | V53 | V54 | V55),
            FenvSandbox              => self == V51,
            Bit32Lib                 => matches!(self, V52 | V53),
            CloseAttribute | ConstAttribute | CoroutineClose | WarnFunction
                                     => matches!(self, V54 | V55),
            GcParam | GlobalKeyword | NamedVararg | TableCreate
                                     => self == V55,
        }
    }
    pub fn features(self) -> impl Iterator<Item = Feature> { /* ALL.filter */ }
}
```

These rows are **claims to be proven against the engine**, not trusted. §2.3 is
what makes them safe.

### 2.3 The **reference binary** is the authority, via a generated fixture (round-2 Finding 1)

The matrix must be validated against the *reference Lua binaries* — the project's
only truth-teller — **not** our own engine. Probing `Lua::new_versioned(v)` is
circular once §2.4 makes `init.rs` consume the matrix: a wrong row removes the
library *and* the self-probe agrees. So:

1. **Generator** `specs/oracle/gen_feature_matrix.sh` runs each feature probe
   against the *reference* binary for every version (resolved exactly as
   `diff_one.sh` does: `reference/lua-5.x` then `/tmp/lua-refs/bin/lua5.x`) and
   emits a committed fixture `ANALYSES/version_feature_matrix.tsv`
   (`version<TAB>feature<TAB>supported`). Provenance — "generated from the
   reference binaries" — is in the file header. This is the harness
   *pre-computed-analysis* pattern: the reference truth, captured once.
2. **Rust test** `omnilua/tests/version_support.rs` reads the fixture and asserts
   `LuaVersion::supports(f) == fixture[(v, f)]` for **every** `(version, feature)`.
   Nothing hand-asserts a row; the reference is the source of record.
3. A second, *secondary* assertion may probe our own `Lua::new_versioned(v)` and
   require it to match the fixture too — but that is a check that our engine
   agrees with the reference (which the official suites already largely enforce),
   explicitly **not** the authority for the matrix.

**Every probe runs inside `pcall` and loads source via `(loadstring or load)`
(round-3 Finding 1).** On Lua 5.1 `load` takes a *reader function*, not a string
(`load("x")` → `bad argument #1 ... (function expected, got string)`); the string
loader is `loadstring`. A probe that called `load("…")` directly would fail on 5.1
for the wrong reason and could abort the generator. So each parse probe is
`pcall(function() return (loadstring or load)(SRC) ~= nil end)` and a `false`/error
result means "absent". Verified against `lua5.1.5` (`load(string)` errors there;
`loadstring(string)` works).

The probes — each must cover the *whole named capability* (round-2 Finding 4) and
test *semantics* where parsing alone is ambiguous (round-2/3 Findings 3):

| Feature | Reference probe (truthy ⇒ supported) |
|---|---|
| `Bit32Lib` | `type(bit32)=="table"` |
| `Utf8Lib` | `type(utf8)=="table"` |
| `StringPack` | `type(string.pack)=="function" and type(string.unpack)=="function" and type(string.packsize)=="function"` |
| `IntegerSubtype` | `type(math.type)=="function"` |
| `CoroutineClose` | `type(coroutine.close)=="function"` |
| `WarnFunction` | `type(warn)=="function"` |
| `TableCreate` | `type(table.create)=="function"` |
| `FenvSandbox` | `type(setfenv)=="function" and type(getfenv)=="function"` |
| `GotoLabels` | `load("do goto l ::l:: end") ~= nil` |
| `NativeBitwise` | `load("return (6 & 3) ~ (1 << 2) // 1") ~= nil` (bitwise **and** `//`) |
| `CloseAttribute` | `load("local x <close> = nil") ~= nil` |
| `ConstAttribute` | `load("local x <const> = 1") ~= nil` |
| `GlobalKeyword` | `load("global g = 1") ~= nil` |
| `NamedVararg` | `load("local function f(a, ...t) end") ~= nil` |
| `GcIsRunning` | `pcall(collectgarbage, "isrunning")` truthy |
| `GcParam` | `pcall(collectgarbage, "param", "pause")` truthy (param **name** required) |
| `EnvSandbox` | **semantic, two-part** — both `local _ENV={x=3}; return x` evaluates to `3` *and* `load("return y", nil, nil, {y=4})()` evaluates to `4` (the `load(.., env)` form). 5.1: `_ENV` is an ordinary name → `x` stays a global → `nil`, and 5.1 `load` has no env arg → `false`. 5.2+: both `3` and `4`. Codex-verified against `lua5.1.5`..`lua5.5.0` (5.1 → false, 5.2+ → true). |
| `TableLenMetamethod` | **semantic** — `#setmetatable({}, {__len=function() return 42 end}) == 42` (5.1 ignores `__len` on tables → `0`) |

`GcParam` and `param` printing: probe with `pcall` so a rejecting version is
`false`, not an error escaping the generator.

### 2.4 Single-source retrofit (Finding 2, Part B)

`init.rs` currently gates with inline `matches!(version, V51|V52)` (utf8) and
`matches!(version, V52|V53)` (bit32). Change both to consult
`version.supports(Feature::Utf8Lib)` / `Feature::Bit32Lib` — **keeping the existing
`#[cfg(feature = "utf8"/"bit32")]` compile-time gate** (registration =
`cfg(feature) && version.supports(f)`). Now the matrix is the *source* for the
version dimension of library registration, not a parallel copy. `lua-stdlib`
already depends on `lua-types`; the official suites guard that behavior is
unchanged.

**`LuaVersion::supports` vs `Lua::supports` — two distinct semantics (round-3
Finding 3).** Because library modules are Cargo-feature-gated (#223: a lean build
can compile out `utf8`/`bit32`/`coroutine`), the two queries must differ:
- `LuaVersion::supports(f)` — pure **version capability**, the reference-backed
  matrix (§2.3). Build-independent. This is what `init.rs` consumes.
- `Lua::supports(f)` — **this instance, in this build**: `version().supports(f)`
  ANDed with compile-time availability for library-backed rows
  (`Bit32Lib`/`Utf8Lib` gated by `cfg!(feature=...)`; non-library rows pass
  through). A build with `--no-default-features` (no `utf8`) reports
  `lua53.supports(Utf8Lib) == false` even though the *version* has it — matching
  what the host can actually call. A test asserts this under the gated build.

---

## 3. Part D — typed `Unsupported`, scoped honestly

`Feature` + `Unsupported` are pure data in `lua-types`; the public `omnilua::Error`
carries the classification as a typed side-channel. We do **not** add a variant to
the internal VM `LuaError` enum (matched across the whole VM; invasive and
layering-wrong for a host-API concept).

```rust
// lua-types
pub struct Unsupported { pub feature: Feature, pub version: LuaVersion }

// omnilua::Error gains a private `unsupported: Option<Unsupported>`.
impl Error {
    /// The ONLY way to build an Unsupported error: couples the typed payload with
    /// the single-sourced message so the two cannot desync (Finding 4).
    pub(crate) fn unsupported(feature: Feature, version: LuaVersion) -> Self;
    pub fn as_unsupported(&self) -> Option<&Unsupported>;
    pub fn is_unsupported(&self) -> bool;
}
```

Message is single-sourced from `Feature` (`Feature: Display`, e.g. `"bit32"`):
`"{feature} is not available in Lua {version}"`. `Display`/`message_lossy` keep
working for hosts that don't match the typed form.

**Re-export (round-2 Finding 5).** `omnilua` must re-export `Feature` and
`Unsupported` from `lua-types` (it already re-exports `LuaVersion`/`NumberModel`),
so a host writes `omnilua::Feature` / `omnilua::Unsupported` and the stable
embedding API does not leak the internal `lua-types` crate.

**Direct-host-only — stated and tested (Finding 3).** The classification rides on
the `omnilua::Error` wrapper. If such an error is returned from a Rust callback
*into Lua*, the trampoline converts it via `From<Error> for LuaError`
(`lib.rs:226`) and the side-channel is dropped — only the message survives. #234's
sole producer (`gc().is_running()`) returns the error **directly** to the host,
never across a Lua boundary, so the classification always survives there. The spec
commits to:
- a doc line on `as_unsupported()` stating it reflects classification only for
  errors returned directly from the host API, not ones re-raised through Lua;
- a doc line that `kind()`/`as_lua_error()` still return the inner `Runtime(_)`
  payload (so a host matching only on `kind()` sees a normal runtime error — they
  must call `as_unsupported()` for the classification);
- a test asserting the direct path sets `is_unsupported()`, **and** a test
  asserting (documenting) that round-tripping through a `pcall` in Lua preserves
  the *message* but not the typed classification. If a future feature needs
  cross-Lua matchability, that is the trigger to promote it to a real inner
  variant — noted, not built now.

---

## 4. Part E — wire `gc().is_running()`

```rust
pub fn is_running(&self) -> Result<bool> {
    let v = self.lua.version();
    if !v.supports(Feature::GcIsRunning) {            // consumes the matrix
        return Err(Error::unsupported(Feature::GcIsRunning, v));
    }
    self.collectgarbage()?.call("isrunning")
}
```

`Lua::supports(&self, f) -> bool` is the host pre-check, with the build-aware
semantics from §2.4 (version capability AND compile-time availability). The
`is_running` gate above uses `GcIsRunning`, which is not library-feature-gated, so
for that row `Lua::supports` and `version().supports` agree. This replaces the
current behavior where 5.1 `is_running()` surfaces a raw `invalid option
'isrunning'` Lua error.

---

## 5. Part F (secondary, optional) — typed `LossyIntConversion`
Promote slice 1's inexact string error to a typed side-channel mirroring §3.
Behavior already correct; matchability polish; after A–E; drop if review says no.

---

## 6. Test plan (oracle is the truth-teller)
- `lua-types`: `supports()` rows; `Feature::ALL` completeness (compile-time
  exhaustive match).
- `ANALYSES/version_feature_matrix.tsv`: generated from the reference binaries by
  `specs/oracle/gen_feature_matrix.sh`, committed; the source of record (§2.3).
- `omnilua/tests/version_support.rs`: assert `supports() == fixture` for every
  `(version, feature)`; `Lua::supports`; `is_running()` on 5.1 →
  `is_unsupported()` true with `feature`/`version` set, on 5.4 → `Ok`;
  direct-host classification test, and a test documenting that a `pcall`
  round-trip through Lua preserves the *message* but not the typed classification
  (§3). Optional secondary: our engine matches the fixture too.
- Retrofit guard: official suites + multiversion oracle unchanged after the
  `init.rs` gate swap (behavior-preserving).
- Gate: `cargo test --workspace`, `harness/run_official_all.sh`,
  `specs/oracle/check.sh ×5`, hooks.

## 7. Resolved review questions
- **Round-1/2 accepted in full.** Engine-struct deferral (§0) and direct-host-only
  `Unsupported` (§3) were explicitly approved by round-2; they ship as specified.
- `GcGenerationalMode`/`TableGcMetamethod` **dropped from v1** (§2.1) — no clean
  reference probe; revisit with a precise probe later.
- Matrix authority is the **reference fixture**, never our engine (§2.3).
- Remaining genuine follow-up (not blockers): converging the *syntax/behavioral*
  gates (parser/lexer) onto the matrix — these are cross-check-validated via the
  fixture now but still have their own inline gates; convergence is a later change
  with its own behavior-risk surface.
