//! Lua 5.4 standard library — runtime stdlib crate.
//!
//! Each module corresponds to one C source under reference/lua-5.4.7/src/.
//! See ANALYSES/file_deps.txt for the mapping.

pub mod auxlib;
pub mod base;
#[cfg(feature = "bit32")]
pub mod bit32_lib;
#[cfg(feature = "coroutine")]
pub mod coro_lib;
#[cfg(feature = "debug")]
pub mod debug_lib;
pub mod init;
#[cfg(feature = "io")]
pub mod io_lib;
#[cfg(feature = "package")]
pub mod loadlib;
pub mod math_lib;
#[cfg(feature = "os")]
pub mod os_lib;
pub mod sandbox;
pub mod state_stub;
pub mod string_lib;
pub mod table_lib;
#[cfg(feature = "utf8")]
pub mod utf8_lib;

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (module aggregator)
//   target_crate:  lua-stdlib
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Each pub mod maps to one stdlib C file.
// ──────────────────────────────────────────────────────────────────────────
