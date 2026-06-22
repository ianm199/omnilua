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
}

#[cfg(test)]
mod tests {
    use super::*;

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
