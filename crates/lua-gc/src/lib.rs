//! Lua 5.4 garbage collector — incremental tri-color.
//!
//! Modules:
//!   gc  — lgc.c port (mark/sweep)
//!   mem — lmem.c port (allocator wrappers)

pub mod gc;
pub mod mem;

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (module aggregator)
//   target_crate:  lua-gc
//   confidence:    high
//   notes:         per-file ports own their own trailers
// ──────────────────────────────────────────────────────────────────────────
