//! Lua 5.4 garbage collector.
//!
//! Modules:
//!   heap — Phase-D production mark-sweep (Gc<T>, Trace, Heap)
//!
//! Historical Phase-A partial ports of C-Lua's `lgc.c`/`lmem.c` were removed
//! once `heap.rs` became the production collector. Keep this crate's source
//! tree limited to compiled modules so unsafe audits reflect the active build.

pub mod heap;

pub use heap::{
    with_current_heap, Color, FinalizerEntry, FinalizerRegistry, FinalizerRegistryStats, Gc, GcAge,
    GcBox, GcHeader, GcState, Heap, HeapGuard, HeapRef, Marker, StepBudget, StepOutcome, Trace,
    Udata51Probe, WeakEntry, WeakListKind, WeakRegistry, WeakRegistrySnapshot, WeakRegistryStats,
};

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
//                  heap.rs. Reference-only Phase-A partial ports are not kept
//                  in src/, so source scans track the active build.
// ──────────────────────────────────────────────────────────────────────────
