//! `LuaStatus` — return codes matching C-Lua's LUA_OK / LUA_ERR* constants.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum LuaStatus {
    Ok        = 0,
    Yield     = 1,
    ErrRun    = 2,
    ErrSyntax = 3,
    ErrMem    = 4,
    ErrErr    = 5,
    ErrFile   = 6,
    ErrGc     = 7,
}

impl LuaStatus {
    pub fn from_raw(n: i32) -> Self {
        match n {
            0 => Self::Ok,
            1 => Self::Yield,
            2 => Self::ErrRun,
            3 => Self::ErrSyntax,
            4 => Self::ErrMem,
            5 => Self::ErrErr,
            6 => Self::ErrFile,
            _ => Self::ErrGc,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lua.h (LUA_OK / LUA_YIELD / LUA_ERR*)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         LuaStatus enum. Mirrors C's status constants used as return codes from
//                  lua_pcall / lua_resume / lua_load.
// ──────────────────────────────────────────────────────────────────────────────
