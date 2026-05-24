//! `GcRef<T>` — the GC-managed reference handle.
//!
//! Phase A/B/C: thin newtype around `Rc<T>`.
//! Phase D-1e (current): newtype around `lua_gc::Gc<T>` — Copy under the hood,
//! tracks allocation in the active `Heap` (via `lua_gc::with_current_heap(...)`).
//!
//! Surface kept stable across the swap: `new`, `ptr_eq`, `identity`,
//! `strong_count`, `weak_count`, `downgrade`. Existing code touching
//! `gc.0` continues to work — `.0` is now `Gc<T>` instead of `Rc<T>`.
//!
//! # Weak refs (D-1)
//!
//! `GcWeak<T>` is currently a no-op wrapper: `upgrade` always returns
//! `Some`, `strong_count` always returns `1`. Real weak semantics arrive
//! in D-2 when the heap learns to mark weak refs separately. For D-1, weak
//! tables ARE a known semantic gap (see PHASE_D_PLAN.md "Locked Decisions").

use lua_gc::{Gc, Marker, Trace};

/// A GC-managed pointer to a Lua collectable object. Newtype over
/// `lua_gc::Gc<T>` so callers preserve `gc.0`-shape access while the
/// backend swaps under them.
#[derive(Debug)]
pub struct GcRef<T: Trace + 'static>(pub Gc<T>);

impl<T: Trace + 'static> GcRef<T> {
    /// Allocate a new GC-tracked value. If a `HeapGuard` is active (set by
    /// `state.run()` / `pcall_k`), the new allocation joins that heap's
    /// allgc chain. Otherwise it allocates "uncollected" — leaks until
    /// process exit, same as the old `Rc::new` behavior.
    pub fn new(value: T) -> Self {
        let gc = lua_gc::with_current_heap(|heap| match heap {
            Some(heap) => heap.allocate(value),
            None => Gc::new_uncollected(value),
        });
        GcRef(gc)
    }

    /// Cycle-aware trace dispatch.
    ///
    /// During D-0/D-1 (before a real Color::Gray flag is reachable from
    /// inside Trace impls), `Marker::try_visit` records the underlying
    /// allocation's identity and short-circuits a second recursion. Once
    /// D-2 ships the in-header color flag, this helper collapses to
    /// `m.mark(self.0)`.
    pub fn trace_obj(&self, m: &mut Marker) {
        if m.try_visit(self.identity()) {
            (**self).trace(m);
        }
    }
}

impl<T: Trace + 'static> GcRef<T> {
    /// Two `GcRef`s are identity-equal iff they point at the same box.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Gc::ptr_eq(a.0, b.0)
    }

    /// Identity as a usize — used as a HashMap key for "same object" tests.
    pub fn identity(&self) -> usize {
        self.0.identity()
    }

    /// Number of strong references. Phase D-1: always returns 1 (no
    /// refcount semantics). Real value once weak refs land in D-2.
    pub fn strong_count(&self) -> usize {
        1
    }

    /// Number of weak references. Phase D-1: always returns 0.
    pub fn weak_count(&self) -> usize {
        0
    }

    /// Get a weak handle. Phase D-1: GcWeak is a thin wrapper that always
    /// upgrades; real weak semantics arrive in D-2.
    pub fn downgrade(&self) -> GcWeak<T> {
        GcWeak(self.0)
    }
}

/// A weak handle to a `GcRef<T>`. Phase D-1 placeholder; D-2 will give
/// this real semantics (None once the referent is swept).
#[derive(Debug)]
pub struct GcWeak<T: Trace + 'static>(pub Gc<T>);

impl<T: Trace + 'static> GcWeak<T> {
    /// Try to promote to a strong reference. Phase D-1: always Some
    /// (weak semantics are not yet implemented).
    pub fn upgrade(&self) -> Option<GcRef<T>> {
        Some(GcRef(self.0))
    }

    /// Strong reference count of the target. Phase D-1: always 1.
    pub fn strong_count(&self) -> usize {
        1
    }
}

impl<T: Trace + 'static> Clone for GcWeak<T> {
    fn clone(&self) -> Self {
        GcWeak(self.0)
    }
}

impl<T: Trace + 'static> Clone for GcRef<T> {
    fn clone(&self) -> Self {
        GcRef(self.0)
    }
}

impl<T: Trace + 'static> Copy for GcRef<T> {}

impl<T: Trace + 'static> std::ops::Deref for GcRef<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &*self.0
    }
}

impl<T: Trace + 'static> AsRef<T> for GcRef<T> {
    fn as_ref(&self) -> &T {
        &*self.0
    }
}

impl<T: PartialEq + Trace + 'static> PartialEq for GcRef<T> {
    fn eq(&self, other: &Self) -> bool {
        Gc::ptr_eq(self.0, other.0) || **self == **other
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        n/a (GcRef public wrapper around lua-gc::Gc<T>)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Thin wrapper type so consumers across crates don't depend on lua-gc's
//                  raw Gc<T>. Clone/Deref/PartialEq forwarded; no unsafe surface.
// ──────────────────────────────────────────────────────────────────────────────
