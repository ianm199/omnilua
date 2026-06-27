//! Initialization of standard libraries for Lua.
//!
//! Opens all standard libraries via `require`-style loading and registers
//! them into the global table.
//!
//! Port of `src/linit.c` (66 lines, 1 function).

use crate::state_stub::{LuaState, LuaStateStubExt as _};
use lua_types::error::LuaError;

// Matches types.tsv: lua_CFunction → fn(&mut LuaState) -> Result<usize, LuaError>
type LuaCFunction = fn(&mut LuaState) -> Result<usize, LuaError>;

// ── Library-name byte-string constants ────────────────────────────────────
//
// These replace the C macros from lualib.h and lauxlib.h:
//   LUA_GNAME        = "_G"         (lauxlib.h)
//   LUA_LOADLIBNAME  = "package"    (lualib.h)
//   LUA_COLIBNAME    = "coroutine"  (lualib.h)
//   LUA_TABLIBNAME   = "table"      (lualib.h)
//   LUA_IOLIBNAME    = "io"         (lualib.h)
//   LUA_OSLIBNAME    = "os"         (lualib.h)
//   LUA_STRLIBNAME   = "string"     (lualib.h)
//   LUA_MATHLIBNAME  = "math"       (lualib.h)
//   LUA_UTF8LIBNAME  = "utf8"       (lualib.h)
//   LUA_DBLIBNAME    = "debug"      (lualib.h)
//
// Per PORTING.md §3.1 all Lua string data uses &[u8], not &str.

//   {LUA_GNAME, luaopen_base},
//   {LUA_LOADLIBNAME, luaopen_package},
//   {LUA_COLIBNAME, luaopen_coroutine},
//   {LUA_TABLIBNAME, luaopen_table},
//   {LUA_IOLIBNAME, luaopen_io},
//   {LUA_OSLIBNAME, luaopen_os},
//   {LUA_STRLIBNAME, luaopen_string},
//   {LUA_MATHLIBNAME, luaopen_math},
//   {LUA_UTF8LIBNAME, luaopen_utf8},
//   {LUA_DBLIBNAME, luaopen_debug},
//   {NULL, NULL}
// };
//
// PORT NOTE: C sentinel `{NULL, NULL}` dropped — Rust slices carry their
//   own length, so no terminator is needed.
//
// PORT NOTE: Per PORTING.md §7, `luaopen_X` → `open` inside the module
//   (e.g. `crate::base::open`, `crate::string_lib::open`).  As of Phase A
//   the individual stdlib modules exported inconsistent names:
//     base.rs        → `pub fn open`          (canonical; matches here)
//     string_lib.rs  → `pub fn luaopen_string` (needs rename in Phase B)
//     table_lib.rs   → `pub fn open_table`    (needs rename in Phase B)
//     math_lib.rs    → `pub fn luaopen_math`  (needs rename in Phase B)
//     io_lib.rs      → `pub fn luaopen_io`    (needs rename in Phase B)
//     os_lib.rs      → `pub fn open_os`       (needs rename in Phase B)
//     utf8_lib, debug_lib, coro_lib, loadlib  → not yet ported (Phase B)
//   Phase B should rename every stdlib opener to `pub fn open` and update
//   this table accordingly.
static LOADED_LIBS: &[(&[u8], LuaCFunction)] = &[
    (b"_G", crate::base::open),
    #[cfg(feature = "package")]
    (b"package", crate::loadlib::luaopen_package),
    #[cfg(feature = "coroutine")]
    (b"coroutine", crate::coro_lib::open_coroutine),
    (b"table", crate::table_lib::open_table),
    #[cfg(feature = "io")]
    (b"io", crate::io_lib::luaopen_io),
    #[cfg(feature = "os")]
    (b"os", crate::os_lib::open_os),
    (b"string", crate::string_lib::luaopen_string),
    (b"math", crate::math_lib::luaopen_math),
    #[cfg(feature = "utf8")]
    (b"utf8", crate::utf8_lib::open_utf8),
    #[cfg(feature = "debug")]
    (b"debug", crate::debug_lib::open_debug),
];

//   const luaL_Reg *lib;
//   /* "require" functions from 'loadedlibs' and set results to global table */
//   for (lib = loadedlibs; lib->func; lib++) {
//     luaL_requiref(L, lib->name, lib->func, 1);
//     lua_pop(L, 1);  /* remove lib */
//   }
// }
//
// PORT NOTE: `LUALIB_API` → `pub` (PORTING.md §4.1 / macros.tsv).
//   `luaL_requiref(L, name, func, 1)` → `state.require_lib(name, func, true)?`
//   The final `1` argument means "set global" — the loaded module value is
//   assigned to the global table under `name` and the value left on the
//   stack is then discarded by `lua_pop(L, 1)`.
//   `lua_pop(L, 1)` → `state.pop_n(1)` (macros.tsv).
/// Open all standard Lua libraries into `state`, registering each into the
/// global table.
///
/// Corresponds to `luaL_openlibs` in `linit.c`.
pub fn open_libs(state: &mut LuaState) -> Result<(), LuaError> {
    // Whether this version ships `utf8` (a 5.3 addition) is the #234 capability
    // matrix — the reference-backed single source — not a second inline version
    // check. The `#[cfg(feature = "utf8")]`-gated registration entries in
    // LOADED_LIBS still control compile-time availability.
    let has_utf8 = state
        .global()
        .lua_version
        .supports(lua_types::Feature::Utf8Lib);
    for &(name, func) in LOADED_LIBS {
        if name == b"utf8".as_slice() && !has_utf8 {
            continue;
        }
        state.require_lib(name, func, true)?;
        state.pop_n(1);
    }
    // `bit32` is present on 5.2 and 5.3 and removed in 5.4; the version dimension
    // comes from the #234 matrix, the compile-time dimension from the feature
    // gate (registration = `cfg(feature) && version.supports(Bit32Lib)`).
    #[cfg(feature = "bit32")]
    if state
        .global()
        .lua_version
        .supports(lua_types::Feature::Bit32Lib)
    {
        state.require_lib(b"bit32", crate::bit32_lib::open_bit32, true)?;
        state.pop_n(1);
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/linit.c  (66 lines, 1 function)
//   target_crate:  lua-stdlib
//   confidence:    high
//   todos:         1
//   port_notes:    3
//   unsafe_blocks: 0
//   notes:         Trivial file. Cross-crate refs (state.require_lib,
//                  state.pop_n, crate::*::open) resolve in Phase B.
//                  Phase B must also reconcile inconsistent open-function
//                  names in the existing stdlib modules (see PORT NOTEs).
// ──────────────────────────────────────────────────────────────────────────
