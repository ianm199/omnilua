//! Lua 5.4 garbage collector.
//!
//! Modules:
//!   heap — Phase-D production mark-sweep (Gc<T>, Trace, Heap)
//!
//! `gc.rs` and `mem.rs` exist on disk as reference-only partial ports of
//! C-Lua's lgc.c and lmem.c — they are not declared as modules here because
//! they import `LuaState` from `lua-vm` (which now depends on this crate,
//! and a cycle is rejected by cargo). Re-introducing them as a build target
//! requires inverting the dependency: lua-vm exposes a Heap-aware trait
//! and the legacy ports operate against the trait. Out of scope for D-0.

pub mod heap;

pub use heap::{Color, Gc, GcBox, GcHeader, GcState, Heap, HeapGuard, Marker, Trace, current_heap};

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (module aggregator)
//   target_crate:  lua-gc
//   confidence:    high
//   notes:         per-file ports own their own trailers
// ──────────────────────────────────────────────────────────────────────────
