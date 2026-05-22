//! Phase D-3.5 acceptance-test fixture: a Rust-native Lua module exposed as a
//! `cdylib` so the `dynlib_*_hook` path through `package.loadlib` can be
//! exercised end-to-end.
//!
//! The exported symbol matches this build's Rust-native module ABI:
//!
//! ```ignore
//! fn(&mut LuaState) -> Result<usize, LuaError>
//! ```
//!
//! That signature is what `DynamicSymbol::RustNative` carries, and what the
//! `lua-cli` `dynlib_symbol` hook hands back when the symbol name does not
//! start with `luaopen_`. The acceptance test calls
//! `package.loadlib("./libtest.so", "rust_open")` so the symbol resolves
//! through the Rust-native path; the function pushes the integer `42` and
//! returns 1 to signal "one return value on the stack".

use lua_types::error::LuaError;
use lua_types::value::LuaValue;
use lua_vm::state::LuaState;

/// Pushes the integer `42` onto the Lua stack and returns `1`. Conforms to
/// `LuaCFunction = fn(&mut LuaState) -> Result<usize, LuaError>`.
///
/// Exported with `#[no_mangle]` so `dlsym` resolves the symbol by name.
/// Naming intentionally avoids the `luaopen_` prefix so the `lua-cli`
/// `dynlib_symbol` hook treats it as Rust-native rather than stock C-Lua.
#[no_mangle]
pub fn rust_open(state: &mut LuaState) -> Result<usize, LuaError> {
    state.push(LuaValue::Int(42));
    Ok(1)
}

/// A `luaopen_*`-named symbol used by the third acceptance test to verify
/// that stock Lua C ABI modules are refused with a clear `"init"` failure.
/// The function body is irrelevant — `lua-cli`'s `dynlib_symbol` hook treats
/// any symbol starting with `luaopen_` as C-ABI and returns
/// `DynamicSymbol::LuaCAbi`, which `package.loadlib` maps to the
/// unsupported-ABI message without calling the function.
///
/// Marked `extern "C"` so its real signature is plausible for an upstream
/// Lua 5.4 module; this keeps the symbol resolvable but ensures we never
/// actually invoke it from the Rust side.
#[no_mangle]
pub extern "C" fn luaopen_test() -> i32 {
    0
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (test fixture; no C origin)
//   target_crate:  lua-cli-test-rust-module
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         cdylib exporting a single Rust-native symbol used by the
//                  Phase D-3.5 dynlib_*_hook acceptance test. Builds against
//                  the same lua-vm/lua-types as the host so the
//                  `fn(&mut LuaState) -> Result<usize, LuaError>` ABI is
//                  identical on both sides.
// ──────────────────────────────────────────────────────────────────────────
