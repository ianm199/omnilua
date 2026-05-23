//! `UpVal` — closure upvalues. PORT_STRATEGY §3.8.

use std::cell::{Ref, RefCell};
use crate::StackIdx;
use crate::value::LuaValue;

/// Discriminator state for an upvalue: either still pointing at a thread's
/// stack slot, or owning the value after close.
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
/// Wrapped in a `RefCell` so multiple closures sharing the same `GcRef<UpVal>`
/// observe the Open→Closed transition done by `luaF_close`. The outer
/// `GcRef<T>` is `Rc<T>` in Phase A–C, which has no built-in interior
/// mutability — `RefCell` provides it.
#[derive(Debug)]
pub struct UpVal {
    pub state: RefCell<UpValState>,
}

impl UpVal {
    pub fn open(thread_id: usize, idx: StackIdx) -> Self {
        UpVal { state: RefCell::new(UpValState::Open { thread_id, idx }) }
    }
    pub fn closed(v: LuaValue) -> Self {
        UpVal { state: RefCell::new(UpValState::Closed(v)) }
    }
    pub fn slot(&self) -> Ref<'_, UpValState> { self.state.borrow() }
    pub fn is_open(&self) -> bool { matches!(*self.state.borrow(), UpValState::Open { .. }) }
    pub fn is_closed(&self) -> bool { matches!(*self.state.borrow(), UpValState::Closed(_)) }
    pub fn close_with(&self, v: LuaValue) {
        *self.state.borrow_mut() = UpValState::Closed(v);
    }
    pub fn set_closed_value(&self, v: LuaValue) {
        let mut g = self.state.borrow_mut();
        match &mut *g {
            UpValState::Closed(slot) => *slot = v,
            UpValState::Open { .. } => *g = UpValState::Closed(v),
        }
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
//                  use an enum with the equivalent two states.
// ──────────────────────────────────────────────────────────────────────────────
