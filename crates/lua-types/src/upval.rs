//! `UpVal` — closure upvalues. PORT_STRATEGY §3.8.

use crate::StackIdx;
use crate::value::LuaValue;

/// A closure upvalue. Open upvalues point at a slot on a thread's stack
/// (referred to by index, since the stack reallocates). Closed upvalues
/// own the value.
///
/// PORT_STRATEGY §3.8: this is a Rust enum rather than `Rc<RefCell<TValue>>`
/// for every upvalue, because that throws away C-Lua's open/closed optimization.
#[derive(Debug, Clone)]
pub enum UpVal {
    /// Lives on a thread's stack at the given index.
    Open {
        thread_id: usize, // identity-only; resolved through GlobalState.threads
        idx: StackIdx,
    },
    /// Has been "closed" — value is owned here.
    Closed(LuaValue),
}

impl UpVal {
    pub fn is_open(&self) -> bool { matches!(self, UpVal::Open { .. }) }
    pub fn is_closed(&self) -> bool { matches!(self, UpVal::Closed(_)) }
}
