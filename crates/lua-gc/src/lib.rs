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

pub use heap::{Color, Gc, GcBox, GcHeader, GcState, Heap, HeapGuard, Marker, StepBudget, StepOutcome, Trace, current_heap};

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (module aggregator; per-file ports own their own trailers)
//   target_crate:  lua-gc
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Module aggregator: re-exports the public surface of heap.rs
//                  (Gc, GcBox, GcHeader, Heap, HeapGuard, Marker, Trace, etc.).
//                  No code of its own. The mark-and-sweep collector lives in
//                  heap.rs; gc.rs and mem.rs are reference-only Phase-A partial
//                  ports kept on disk for future re-port (see preamble above).
// ──────────────────────────────────────────────────────────────────────────
