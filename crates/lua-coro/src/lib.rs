//! Lua coroutines via stackful context switching (`corosensei`). Phase E scope.
//!
//! This crate currently has no unsafe implementation. A future stackful backend
//! must raise its explicit unsafe budget in `harness/unsafe-budgets.toml`.

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (none — skeleton; Phase E populates from lcorolib.c)
//   target_crate:  lua-coro
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         placeholder for stackful coroutine port
// ──────────────────────────────────────────────────────────────────────────
