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
/// updated in lockstep so existing `slot()` callers keep working.
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
/// plus a `RefCell<LuaValue>` holding the closed payload. The `state`
/// `RefCell<UpValState>` mirror is kept consistent so cold consumers that
/// still call `slot()` see the correct view. The split lets
/// `state.rs::upvalue_get` / `upvalue_set` short-circuit the Open path with
/// zero `RefCell` borrow overhead, which is the dominant cost in
/// fibonacci-class recursion benchmarks.
#[derive(Debug)]
pub struct UpVal {
    open_thread_id: Cell<i64>,
    open_idx: Cell<u32>,
    closed_value: RefCell<LuaValue>,
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
            closed_value: RefCell::new(LuaValue::Nil),
            state: RefCell::new(UpValState::Open { thread_id, idx }),
        }
    }

    pub fn closed(v: LuaValue) -> Self {
        UpVal {
            open_thread_id: Cell::new(CLOSED_TAG),
            open_idx: Cell::new(0),
            closed_value: RefCell::new(v.clone()),
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
    #[inline]
    pub fn try_open_payload(&self) -> Option<(usize, StackIdx)> {
        let tid = self.open_thread_id.get();
        if tid < 0 {
            None
        } else {
            Some((tid as usize, StackIdx(self.open_idx.get())))
        }
    }

    /// Borrows the closed-side value. Callers must have confirmed the
    /// upvalue is closed (`try_open_payload` returned `None`).
    #[inline]
    pub fn closed_value(&self) -> Ref<'_, LuaValue> { self.closed_value.borrow() }

    pub fn close_with(&self, v: LuaValue) {
        self.open_thread_id.set(CLOSED_TAG);
        self.open_idx.set(0);
        *self.closed_value.borrow_mut() = v.clone();
        *self.state.borrow_mut() = UpValState::Closed(v);
    }

    pub fn set_closed_value(&self, v: LuaValue) {
        self.open_thread_id.set(CLOSED_TAG);
        self.open_idx.set(0);
        *self.closed_value.borrow_mut() = v.clone();
        *self.state.borrow_mut() = UpValState::Closed(v);
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
//                  The RefCell<UpValState> mirror is updated in lockstep so existing
//                  out-of-crate slot() consumers (api.rs, debug.rs, coro_lib.rs,
//                  func.rs) keep working without migration.
// ──────────────────────────────────────────────────────────────────────────────
