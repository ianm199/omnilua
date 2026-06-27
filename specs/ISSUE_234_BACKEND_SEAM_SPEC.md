# #234 — Multi-version capability seam: implementation spec (rev 2)

**Status:** spec, post-codex-review round 1 (VERDICT: REVISE — all findings
accepted; this rev addresses them). deep-spec → codex-review → execute.
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
- **C. Exhaustive live-engine cross-check (Finding 1/4):** a test that probes a
  *real instance of every version* for *every* `Feature` and asserts
  `supports() == observed`. This is the mechanical authority; the matrix is not
  trusted on its own.
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
    TableGcMetamethod,    // __gc on tables — 5.2+
    GcIsRunning,          // collectgarbage("isrunning") — 5.2+
    GcGenerationalMode,   // generational collection mode — 5.4+
    GcParam,              // collectgarbage("param", ...) — 5.5
    GlobalKeyword,        // `global` decl + declared-global scope — 5.5
    NamedVararg,          // function f(a, ...t) — 5.5
    TableCreate,          // table.create — 5.5
}
```

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
            EnvSandbox | GotoLabels | TableLenMetamethod | TableGcMetamethod
                                     => matches!(self, V52 | V53 | V54 | V55),
            FenvSandbox              => self == V51,
            Bit32Lib                 => matches!(self, V52 | V53),
            GcIsRunning              => matches!(self, V52 | V53 | V54 | V55),
            CloseAttribute | ConstAttribute | CoroutineClose | WarnFunction
                | GcGenerationalMode => matches!(self, V54 | V55),
            GcParam | GlobalKeyword | NamedVararg | TableCreate
                                     => self == V55,
        }
    }
    pub fn features(self) -> impl Iterator<Item = Feature> { /* ALL.filter */ }
}
```

These rows are **claims to be proven against the engine**, not trusted. §2.3 is
what makes them safe.

### 2.3 The live-engine cross-check is the authority (Findings 1 & 4)

A hand-written matrix rotted in the draft (`bit32`). So the matrix is validated by
probing a **real instance of each version** for **each feature**, in
`omnilua/tests/version_support.rs`:

```rust
fn observed(lua: &Lua, f: Feature) -> bool {
    match f {
        Bit32Lib            => type_is(lua, "bit32", "table"),
        Utf8Lib             => type_is(lua, "utf8", "table"),
        StringPack          => type_is(lua, "string.pack", "function"),
        IntegerSubtype      => type_is(lua, "math.type", "function"),
        CoroutineClose      => type_is(lua, "coroutine.close", "function"),
        WarnFunction        => type_is(lua, "warn", "function"),
        TableCreate         => type_is(lua, "table.create", "function"),
        FenvSandbox         => type_is(lua, "setfenv", "function"),
        GotoLabels          => parses(lua, "do goto l ::l:: end"),
        NativeBitwise       => parses(lua, "return 6 & 3"),
        CloseAttribute      => parses(lua, "local x <close> = nil"),
        ConstAttribute      => parses(lua, "local x <const> = 1"),
        GlobalKeyword       => parses(lua, "global g = 1"),
        NamedVararg         => parses(lua, "local function f(a, ...t) end"),
        GcIsRunning         => gc_option_ok(lua, "isrunning"),
        GcParam             => gc_option_ok(lua, "param"),
        GcGenerationalMode  => gc_option_ok(lua, "generational"),  // see note
        EnvSandbox          => parses(lua, "return _ENV"),
        TableLenMetamethod  => len_metamethod_observed(lua),
        TableGcMetamethod   => gc_metamethod_observed(lua),
    }
}
// for v in all versions, for f in Feature::ALL: assert_eq!(v.supports(f), observed(&Lua::new_versioned(v), f))
```

**`GcGenerationalMode` caveat (Finding 1).** `collectgarbage("generational")` is
*option-accepted* on 5.2/5.4/5.5 but *rejected* on 5.3, while the actual
generational *mode* exists only 5.4+. The host-visible capability we expose is
"can select generational mode" = 5.4+. The probe therefore must distinguish
"option accepted" from "mode available"; if a clean probe isn't possible, this row
is defined as **5.4+ by spec and excluded from the option-acceptance probe** with
a comment, rather than asserting a misleading equivalence. Reviewer: confirm this
is acceptable or that the row should be dropped from v1.

### 2.4 Single-source retrofit (Finding 2, Part B)

`init.rs` currently gates with inline `matches!(version, V51|V52)` (utf8) and
`matches!(version, V52|V53)` (bit32). Change both to consult
`version.supports(Feature::Utf8Lib)` / `Feature::Bit32Lib`. Now the matrix is the
*source* for library registration, not a parallel copy. `lua-stdlib` already
depends on `lua-types`, so this is a local edit; the official suites are the
guard that behavior is unchanged.

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

`Lua::supports(&self, f) -> bool` = `self.version().supports(f)` is the host
pre-check. This replaces the current behavior where 5.1 `is_running()` surfaces a
raw `invalid option 'isrunning'` Lua error.

---

## 5. Part F (secondary, optional) — typed `LossyIntConversion`
Promote slice 1's inexact string error to a typed side-channel mirroring §3.
Behavior already correct; matchability polish; after A–E; drop if review says no.

---

## 6. Test plan (oracle is the truth-teller)
- `lua-types`: `supports()` rows; `Feature::ALL` completeness (compile-time
  exhaustive match).
- `omnilua/tests/version_support.rs`: **exhaustive** `supports()`-vs-live-engine
  cross-check for every (version, feature) (§2.3); `Lua::supports`;
  `is_running()` on 5.1 → `is_unsupported()` true with `feature`/`version` set,
  on 5.4 → `Ok`; direct-vs-cross-Lua classification test (§3).
- Retrofit guard: official suites + multiversion oracle unchanged after the
  `init.rs` gate swap (behavior-preserving).
- Gate: `cargo test --workspace`, `harness/run_official_all.sh`,
  `specs/oracle/check.sh ×5`, hooks.

## 7. Open questions for round-2 review
1. `GcGenerationalMode` (§2.3 caveat): keep as 5.4+ with a non-option-probe, or
   drop from v1 since option-acceptance ≠ mode-availability is genuinely murky?
2. Is the §3 direct-host-only scoping acceptable for shipping `Unsupported`, given
   the only producer is direct-return — or should `Unsupported` wait until it can
   be a cross-Lua-surviving inner variant (i.e. is a half-matchable error worse
   than none)?
3. Retrofitting only the two stdlib roster gates (utf8/bit32) to the matrix, and
   leaving syntax/behavioral rows as cross-check-tested mirrors — is that split
   honest enough, or should more gates converge now?
