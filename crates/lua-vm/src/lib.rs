//! Lua 5.4 virtual machine — runtime crate.
//!
//! Modules map to the canonical C source files per `ANALYSES/file_deps.txt`.
//! Phase A populated each module with a faithful transliteration; Phase B is
//! reconciling cross-module references against the `lua-types` foundation.

pub mod api;
pub mod ctype;
pub mod debug;
pub mod do_;
pub mod dump;
pub mod func;
pub mod object;
pub mod state;
pub mod string;
pub mod table;
pub mod tagmethods;
pub mod undump;
pub mod vm;
pub mod zio;

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (module aggregator; see individual files for C sources)
//   target_crate:  lua-vm
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Each pub mod corresponds to one C source under
//                  `reference/lua-5.4.7/src/`. See `ANALYSES/file_deps.txt`.
// ──────────────────────────────────────────────────────────────────────────
