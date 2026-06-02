//! `UpVal` — closure upvalues. PORT_STRATEGY §3.8.

use std::cell::{Cell, Ref, RefCell};
use crate::StackIdx;
use crate::value::LuaValue;

/// Discriminator state for an upvalue: either still pointing at a thread's
/// stack slot, or owning the value after close.
///
/// Retained as the public read-side enum for out-of-crate consumers that
/// pattern-match through `UpVal::slot()`. The canonical storage on `UpVal`
/// is now a `Cell`-tagged shape; the `RefCell<UpValState>` mirror is
/// kept for existing `slot()` callers that need the open/closed shape.
#[derive(Debug, Clone)]
pub enum UpValState {
    Open {
        thread_id: usize,
        idx: StackIdx,
    },
    Closed(LuaValue),
}

/// A closure upvalue. Open upvalues point at a slot on a thread's stack
/// (referred to by index, since the stack reallocates). Closed upvalues
/// own the value.
///
/// Canonical state lives in two `Cell` fields (the tag and the open payload)
/// plus a `Cell<LuaValue>` holding the closed payload. The `state`
/// `RefCell<UpValState>` mirror is kept for cold consumers that still call
/// `slot()` to inspect the open/closed shape. Scalar-to-scalar closed writes
/// may leave the mirror's scalar payload stale; callers that need the current
/// closed value must use `closed_value` / `try_closed_value`. The split lets
/// `state.rs::upvalue_get` / `upvalue_set` short-circuit the Open path with
/// zero `RefCell` borrow overhead, which is the dominant cost in
/// fibonacci-class recursion benchmarks.
#[derive(Debug)]
pub struct UpVal {
    open_thread_id: Cell<i64>,
    open_idx: Cell<u32>,
    closed_value: Cell<LuaValue>,
    pub state: RefCell<UpValState>,
}

/// Sentinel placed in `open_thread_id` once the upvalue has been closed.
/// Valid thread ids are non-negative (the main thread is id 0), so -1 is
/// unambiguous.
const CLOSED_TAG: i64 = -1;

impl UpVal {
    pub fn open(thread_id: usize, idx: StackIdx) -> Self {
        UpVal {
            open_thread_id: Cell::new(thread_id as i64),
            open_idx: Cell::new(idx.0),
            closed_value: Cell::new(LuaValue::Nil),
            state: RefCell::new(UpValState::Open { thread_id, idx }),
        }
    }

    pub fn closed(v: LuaValue) -> Self {
        UpVal {
            open_thread_id: Cell::new(CLOSED_TAG),
            open_idx: Cell::new(0),
            closed_value: Cell::new(v),
            state: RefCell::new(UpValState::Closed(v)),
        }
    }

    /// Backwards-compat handle on the full `UpValState`. Out-of-crate code
    /// matches against this through `Ref::deref`. Hot-path callers should
    /// use `try_open_payload` / `closed_value` instead.
    pub fn slot(&self) -> Ref<'_, UpValState> { self.state.borrow() }

    pub fn is_open(&self) -> bool { self.open_thread_id.get() >= 0 }
    pub fn is_closed(&self) -> bool { self.open_thread_id.get() < 0 }

    /// Zero-`RefCell` fast path used by `upvalue_get` / `upvalue_set`.
    /// Returns `Some((thread_id, idx))` when the upvalue is still open,
    /// `None` otherwise.
    #[inline(always)]
    pub fn try_open_payload(&self) -> Option<(usize, StackIdx)> {
        let tid = self.open_thread_id.get();
        if tid < 0 {
            None
        } else {
            Some((tid as usize, StackIdx(self.open_idx.get())))
        }
    }

    /// Returns the closed-side value. Callers must have confirmed the
    /// upvalue is closed (`try_open_payload` returned `None`).
    #[inline(always)]
    pub fn closed_value(&self) -> LuaValue { self.closed_value.get() }

    pub fn close_with(&self, v: LuaValue) {
        self.open_thread_id.set(CLOSED_TAG);
        self.open_idx.set(0);
        self.closed_value.set(v);
        *self.state.borrow_mut() = UpValState::Closed(v);
    }

    pub fn set_closed_value(&self, v: LuaValue) {
        self.open_thread_id.set(CLOSED_TAG);
        self.open_idx.set(0);
        let old_collectable = self.closed_value.get().is_collectable();
        self.closed_value.set(v);
        if old_collectable || v.is_collectable() {
            *self.state.borrow_mut() = UpValState::Closed(v);
        }
    }

    pub fn try_closed_value(&self) -> Option<LuaValue> {
        if self.is_closed() {
            Some(self.closed_value.get())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_scalar_write_updates_canonical_value() {
        let uv = UpVal::closed(LuaValue::Int(1));

        uv.set_closed_value(LuaValue::Int(2));

        assert_eq!(uv.closed_value(), LuaValue::Int(2));
        assert_eq!(uv.try_closed_value(), Some(LuaValue::Int(2)));
        match &*uv.slot() {
            UpValState::Closed(v) => assert_eq!(*v, LuaValue::Int(1)),
            UpValState::Open { .. } => panic!("closed upvalue mirror became open"),
        };
    }

    #[test]
    fn close_with_refreshes_legacy_mirror() {
        let uv = UpVal::open(7, StackIdx(3));

        uv.close_with(LuaValue::Bool(true));

        assert_eq!(uv.closed_value(), LuaValue::Bool(true));
        match &*uv.slot() {
            UpValState::Closed(v) => assert_eq!(*v, LuaValue::Bool(true)),
            UpValState::Open { .. } => panic!("closed upvalue mirror stayed open"),
        };
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lfunc.h, src/lfunc.c (UpVal struct)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         UpVal + UpValState (Open/Closed). C uses a TValue* that switches
//                  between stack-pointing (open) and embedded (closed) via union; we
//                  use an enum with the equivalent two states. Canonical storage is
//                  Cell-tagged (open_thread_id, open_idx, closed_value) so hot-path
//                  upvalue_get/_set skip RefCell borrow guards on the Open branch.
//                  The RefCell<UpValState> mirror is retained for existing
//                  out-of-crate slot() consumers (api.rs, debug.rs, coro_lib.rs,
//                  func.rs). Scalar closed-value writes update the canonical payload
//                  without refreshing the mirror payload, so value reads should use
//                  closed_value()/try_closed_value().
// ──────────────────────────────────────────────────────────────────────────────
