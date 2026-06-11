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
//! # Weak refs
//!
//! Heap-tracked `GcWeak<T>` handles remember the heap active when they were
//! created plus the target's heap allocation token. They upgrade only while
//! that identity/token pair remains live. Handles to legacy uncollected boxes
//! still upgrade forever, matching their process-lifetime allocation model.

use lua_gc::{Gc, HeapRef, Marker, Trace};

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

    /// Number of strong references. `GcRef` is not reference-counted, so a live
    /// handle reports one owning GC reachability handle.
    pub fn strong_count(&self) -> usize {
        1
    }

    /// Number of weak references. Weak handles are not counted.
    pub fn weak_count(&self) -> usize {
        0
    }

    /// Get a weak handle. If this allocation belongs to the currently-active
    /// heap, the weak handle will stop upgrading once sweep removes that exact
    /// heap allocation.
    pub fn downgrade(&self) -> GcWeak<T> {
        let identity = self.identity();
        let tracked = lua_gc::with_current_heap(|heap| {
            heap.map(|heap| {
                let token = heap.register_allocation_token(identity);
                (HeapRef::from_heap(heap), token)
            })
        });
        let (heap, allocation_token) = match tracked {
            Some((heap, token)) => (Some(heap), token),
            None => (None, 0),
        };
        GcWeak {
            target: self.0,
            identity,
            allocation_token,
            heap,
        }
    }

    /// Charge (`delta > 0`) or refund (`delta < 0`) bytes of this object's
    /// owned heap buffers against the active heap's pacer, so collections
    /// fire at honest memory pressure. No-op on `delta == 0`, when no heap is
    /// active, or when the underlying box is uncollected (see
    /// [`lua_gc::Gc::account_buffer`]).
    pub fn account_buffer(&self, delta: isize) {
        if delta == 0 {
            return;
        }
        lua_gc::with_current_heap(|h| {
            if let Some(h) = h {
                self.0.account_buffer(h, delta)
            }
        })
    }
}

/// A weak handle to a `GcRef<T>`.
#[derive(Debug)]
pub struct GcWeak<T: Trace + 'static> {
    target: Gc<T>,
    identity: usize,
    allocation_token: usize,
    heap: Option<HeapRef>,
}

impl<T: Trace + 'static> GcWeak<T> {
    /// Try to promote to a strong reference.
    pub fn upgrade(&self) -> Option<GcRef<T>> {
        if let Some(heap) = self.heap {
            if !heap.contains_allocation(self.identity, self.allocation_token) {
                return None;
            }
        }
        Some(GcRef(self.target))
    }

    /// Strong reference count of the target from this weak handle's point of
    /// view: one while it can still upgrade, zero after sweep.
    pub fn strong_count(&self) -> usize {
        usize::from(self.upgrade().is_some())
    }

    pub fn identity(&self) -> usize {
        self.identity
    }
}

impl<T: Trace + 'static> Clone for GcWeak<T> {
    fn clone(&self) -> Self {
        GcWeak {
            target: self.target,
            identity: self.identity,
            allocation_token: self.allocation_token,
            heap: self.heap,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    struct NoRoots;

    impl Trace for NoRoots {
        fn trace(&self, _m: &mut Marker) {}
    }

    #[derive(Debug)]
    struct Cell0;

    impl Trace for Cell0 {
        fn trace(&self, _m: &mut Marker) {}
    }

    #[test]
    fn heap_tracked_weak_refs_stop_upgrading_after_sweep() {
        let heap = lua_gc::Heap::new();
        heap.unpause();
        let _guard = lua_gc::HeapGuard::push(&heap);

        let strong = GcRef::new(Cell0);
        let weak = strong.downgrade();
        assert!(weak.upgrade().is_some());
        assert_eq!(weak.strong_count(), 1);

        heap.full_collect(&NoRoots);
        assert!(weak.upgrade().is_none());
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn uncollected_weak_refs_keep_process_lifetime_behavior() {
        let strong = GcRef::new(Cell0);
        let weak = strong.downgrade();

        assert!(weak.upgrade().is_some());
        assert_eq!(weak.strong_count(), 1);
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
