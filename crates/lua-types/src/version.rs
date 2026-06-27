//! `LuaVersion` — the single source of truth for which Lua language version a
//! runtime instance speaks.
//!
//! This lives in `lua-types`, the lowest shared crate, so every layer above
//! (parser, compiler, VM, stdlib, runtime) can name the version without a
//! dependency cycle. Per the multi-version architecture decision
//! (`specs/MULTIVERSION_ARCHITECTURE_DECISION.md` §4, §5), the version is a
//! *backend selector* threaded from construction; it never appears in a public
//! embedding-API type.

/// The numeric model a version uses for Lua numbers.
///
/// This is the single sharpest behavioral axis across versions: 5.1/5.2 are
/// float-only (one `number` type, every value an `f64`, no `math.type`), while
/// 5.3/5.4/5.5 carry the dual integer/float subtype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberModel {
    /// One `number` type; every numeric value is an `f64`. Lua 5.1/5.2.
    FloatOnly,
    /// Distinct integer (`i64`) and float (`f64`) subtypes. Lua 5.3/5.4/5.5.
    Dual,
}

/// Which Lua language version a runtime instance speaks.
///
/// `Default` is [`LuaVersion::V54`] — the version this codebase currently
/// implements end-to-end — so that `Lua::new()` and any other defaulted
/// construction keeps the existing 5.4 behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LuaVersion {
    /// Lua 5.1 — float-only, `fenv`-based globals. Deferred (separate core).
    V51,
    /// Lua 5.2 — float-only, modern `_ENV` globals. Deferred (separate core).
    V52,
    /// Lua 5.3 — dual subtype, modern `_ENV`. Deferred.
    V53,
    /// Lua 5.4 — the implemented baseline today.
    V54,
    /// Lua 5.5 — dual subtype, declared-globals scope model. Deferred.
    V55,
}

impl Default for LuaVersion {
    fn default() -> Self {
        LuaVersion::V54
    }
}

impl LuaVersion {
    /// The family-level numeric model for this version.
    pub fn number_model(self) -> NumberModel {
        match self {
            LuaVersion::V51 | LuaVersion::V52 => NumberModel::FloatOnly,
            LuaVersion::V53 | LuaVersion::V54 | LuaVersion::V55 => NumberModel::Dual,
        }
    }

    /// Whether the `__name` metafield overrides an object's type name (in
    /// `tostring` and in type-error messages). `__name` is a 5.3 addition; 5.1
    /// and 5.2 ignore it and always report the primitive type name.
    pub fn honors_name_metafield(self) -> bool {
        !matches!(self, LuaVersion::V51 | LuaVersion::V52)
    }

    /// Whether `io.lines(filename, ...)` returns the file as a fourth
    /// to-be-closed result. The to-be-closed value (`<close>`) mechanism is a
    /// 5.4 addition: 5.4/5.5 return four values (iterator, nil, nil, file) so a
    /// generic `for` can close the file on loop exit; 5.1–5.3 return only the
    /// iterator (one value).
    pub fn lines_returns_to_be_closed(self) -> bool {
        matches!(self, LuaVersion::V54 | LuaVersion::V55)
    }

    /// Whether this version has a real backend. The modern family (5.3/5.4/5.5)
    /// and 5.2 (float-only + `_ENV`) are complete. 5.1 reuses the 5.2 float-only
    /// core plus three faithful 5.1-specific axes:
    /// - **fenv globals** — `getfenv`/`setfenv` (the per-function environment
    ///   model, Option B over the reused `_ENV` upvalue; `specs/followup/5.1-fenv.md`).
    /// - **metamethod diffs** — `#t` never consults a table `__len`, no
    ///   `__pairs`/`__ipairs`, no `__gc` on tables (userdata only).
    /// - **roster + syntax** — `unpack` is global (no `table.unpack`/`pack`/
    ///   `move`); `loadstring` + reader-only `load`; `table.getn`/`setn`(stub)/
    ///   `maxn`/`foreach`/`foreachi`; `module`/`package.seeall`/`package.loaders`
    ///   (no `package.searchers`); `string.gfind`; `math.log` 1-arg +
    ///   `log10`/`atan2`/`pow`/`mod` (no `math.type`); `gcinfo`; `newproxy`;
    ///   `xpcall`-no-extra-args; `coroutine.running` nil in main; no `bit32`/
    ///   `utf8`/`rawlen`; `goto`/labels/`//`/bitwise/`<const>`/`\x`-`\z` escapes
    ///   rejected (`goto` stays a valid identifier). See
    ///   `specs/followup/5.1-roster-syntax.md`. Documented divergences: the
    ///   `math.random` C-`rand()` sequence and `os.execute` raw-status byte are
    ///   host-dependent (contract matches; exact bytes do not).
    pub fn is_supported(self) -> bool {
        matches!(
            self,
            LuaVersion::V51 | LuaVersion::V52 | LuaVersion::V53 | LuaVersion::V54 | LuaVersion::V55
        )
    }

    /// The `_VERSION` global string for this version (e.g. `"Lua 5.4"`).
    pub fn version_str(self) -> &'static str {
        match self {
            LuaVersion::V51 => "Lua 5.1",
            LuaVersion::V52 => "Lua 5.2",
            LuaVersion::V53 => "Lua 5.3",
            LuaVersion::V54 => "Lua 5.4",
            LuaVersion::V55 => "Lua 5.5",
        }
    }

    /// The `LUAC_VERSION` byte written into a `luac`/`string.dump` header for
    /// this version. Upstream encodes the version as `(major << 4) | minor`,
    /// e.g. 5.4 → `0x54`.
    pub fn luac_version_byte(self) -> u8 {
        match self {
            LuaVersion::V51 => 0x51,
            LuaVersion::V52 => 0x52,
            LuaVersion::V53 => 0x53,
            LuaVersion::V54 => 0x54,
            LuaVersion::V55 => 0x55,
        }
    }

    /// Whether this version has a given language [`Feature`] — the pure,
    /// build-independent capability matrix (issue #234). This is the version
    /// dimension only; an embedding instance additionally narrows
    /// library-backed features by what was compiled in (see
    /// `omnilua::Lua::supports`).
    ///
    /// The rows are the source of record `ANALYSES/version_feature_matrix.tsv`,
    /// generated by probing the reference binaries
    /// (`specs/oracle/gen_feature_matrix.sh`); a test asserts this function
    /// against that fixture so the matrix can never drift from upstream.
    pub fn supports(self, f: Feature) -> bool {
        use Feature::*;
        use LuaVersion::*;
        match f {
            IntegerSubtype | NativeBitwise | Utf8Lib | StringPack => {
                matches!(self, V53 | V54 | V55)
            }
            EnvSandbox | GotoLabels | TableLenMetamethod | GcIsRunning => {
                matches!(self, V52 | V53 | V54 | V55)
            }
            FenvSandbox => self == V51,
            Bit32Lib => matches!(self, V52 | V53),
            CloseAttribute | ConstAttribute | CoroutineClose | WarnFunction => {
                matches!(self, V54 | V55)
            }
            GcParam | GlobalKeyword | NamedVararg | TableCreate => self == V55,
        }
    }

    /// Iterate the [`Feature`]s this version supports (version dimension only).
    pub fn features(self) -> impl Iterator<Item = Feature> {
        Feature::ALL.iter().copied().filter(move |f| self.supports(*f))
    }
}

/// A version-divergent language capability — one *present-or-absent* row of the
/// support matrix (issue #234). Behavioral divergences (same call, different
/// result — e.g. `<=`-from-`__lt`, integer `for`-loop wraparound, the RNG stream)
/// are deliberately **not** features: they are resolved inside the core, not
/// gated. Query with [`LuaVersion::supports`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// Integer/float subtypes and `math.type` — 5.3+.
    IntegerSubtype,
    /// `_ENV` and `load(.., env)` lexical-environment sandboxing — 5.2+.
    EnvSandbox,
    /// `setfenv`/`getfenv` function-environment access — 5.1 only.
    FenvSandbox,
    /// `goto` and `::labels::` — 5.2+.
    GotoLabels,
    /// Native bitwise operators `& | ~ << >>` and integer division `//` — 5.3+.
    NativeBitwise,
    /// The `bit32` library — 5.2 and 5.3 (removed in 5.4).
    Bit32Lib,
    /// The `utf8` library — 5.3+.
    Utf8Lib,
    /// `string.pack`/`string.unpack`/`string.packsize` — 5.3+.
    StringPack,
    /// `<close>` to-be-closed variables and the `__close` metamethod — 5.4+.
    CloseAttribute,
    /// `<const>` variables — 5.4+.
    ConstAttribute,
    /// `coroutine.close` — 5.4+.
    CoroutineClose,
    /// The `warn` function — 5.4+.
    WarnFunction,
    /// The `__len` metamethod honored on tables (not just userdata) — 5.2+.
    TableLenMetamethod,
    /// `collectgarbage("isrunning")` — 5.2+.
    GcIsRunning,
    /// `collectgarbage("param", name [, value])` — 5.5.
    GcParam,
    /// The `global` declaration keyword and declared-global scope model — 5.5.
    GlobalKeyword,
    /// Named vararg table parameters `function f(a, ...t)` — 5.5.
    NamedVararg,
    /// `table.create` — 5.5.
    TableCreate,
}

impl Feature {
    /// Every [`Feature`], the iteration source for [`LuaVersion::features`] and
    /// the matrix-vs-reference test. A `match` in the test asserts this is
    /// exhaustive, so adding a variant without adding it here fails to compile.
    pub const ALL: [Feature; 18] = [
        Feature::IntegerSubtype,
        Feature::EnvSandbox,
        Feature::FenvSandbox,
        Feature::GotoLabels,
        Feature::NativeBitwise,
        Feature::Bit32Lib,
        Feature::Utf8Lib,
        Feature::StringPack,
        Feature::CloseAttribute,
        Feature::ConstAttribute,
        Feature::CoroutineClose,
        Feature::WarnFunction,
        Feature::TableLenMetamethod,
        Feature::GcIsRunning,
        Feature::GcParam,
        Feature::GlobalKeyword,
        Feature::NamedVararg,
        Feature::TableCreate,
    ];

    /// A short, stable token naming this feature, used in the fixture and in the
    /// [`Unsupported`] error message.
    pub fn name(self) -> &'static str {
        match self {
            Feature::IntegerSubtype => "integer subtype (math.type)",
            Feature::EnvSandbox => "_ENV sandboxing",
            Feature::FenvSandbox => "setfenv/getfenv",
            Feature::GotoLabels => "goto/labels",
            Feature::NativeBitwise => "native bitwise operators",
            Feature::Bit32Lib => "bit32 library",
            Feature::Utf8Lib => "utf8 library",
            Feature::StringPack => "string.pack",
            Feature::CloseAttribute => "<close> attribute",
            Feature::ConstAttribute => "<const> attribute",
            Feature::CoroutineClose => "coroutine.close",
            Feature::WarnFunction => "warn",
            Feature::TableLenMetamethod => "__len on tables",
            Feature::GcIsRunning => "collectgarbage('isrunning')",
            Feature::GcParam => "collectgarbage('param')",
            Feature::GlobalKeyword => "global declarations",
            Feature::NamedVararg => "named vararg tables",
            Feature::TableCreate => "table.create",
        }
    }
}

impl core::fmt::Display for Feature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// A typed record that a [`Feature`] absent on a given [`LuaVersion`] was
/// requested at a host-API verb (issue #234). Carried by the public embedding
/// error so a host can match the cause; see `omnilua::Error::as_unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported {
    /// The feature that was requested.
    pub feature: Feature,
    /// The version that lacks it.
    pub version: LuaVersion,
}

impl core::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} is not available in {}",
            self.feature,
            self.version.version_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_all_has_no_duplicates() {
        // The fixed-size `[Feature; 18]` plus the exhaustive (wildcard-free)
        // matches in `name()`/`supports()` are the compile-time completeness
        // guard: a new variant forces both to be updated and the array resized.
        // This test guards the remaining gap — a duplicate slipping into ALL.
        for (i, a) in Feature::ALL.iter().enumerate() {
            for b in &Feature::ALL[i + 1..] {
                assert_ne!(a, b, "duplicate feature in Feature::ALL");
            }
        }
    }

    #[test]
    fn default_is_v54() {
        assert_eq!(LuaVersion::default(), LuaVersion::V54);
    }

    #[test]
    fn number_model_split() {
        assert_eq!(LuaVersion::V51.number_model(), NumberModel::FloatOnly);
        assert_eq!(LuaVersion::V52.number_model(), NumberModel::FloatOnly);
        assert_eq!(LuaVersion::V53.number_model(), NumberModel::Dual);
        assert_eq!(LuaVersion::V54.number_model(), NumberModel::Dual);
        assert_eq!(LuaVersion::V55.number_model(), NumberModel::Dual);
    }

    #[test]
    fn version_str_and_byte() {
        assert_eq!(LuaVersion::V54.version_str(), "Lua 5.4");
        assert_eq!(LuaVersion::V54.luac_version_byte(), 0x54);
        assert_eq!(LuaVersion::V53.version_str(), "Lua 5.3");
        assert_eq!(LuaVersion::V53.luac_version_byte(), 0x53);
    }
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (foundation — multi-version seam, not ported from .c)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         LuaVersion + NumberModel. Default = V54 preserves the
//                  existing single-version behavior. V51-V55 complete; V51
//                  reuses the 5.2 float-only core plus the fenv-globals,
//                  metamethod-diff, and roster/syntax axes (all V51-gated).
// ──────────────────────────────────────────────────────────────────────────
