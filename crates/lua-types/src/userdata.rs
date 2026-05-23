//! `LuaUserData` — Lua's heap-allocated userdata. Carries a typed byte
//! buffer plus optional user values (a Vec of TValues).

use std::cell::RefCell;

use crate::gc::GcRef;
use crate::table::LuaTable;
use crate::value::LuaValue;

#[derive(Debug)]
pub struct LuaUserData {
    pub data: Box<[u8]>,
    pub uv: Vec<LuaValue>,
    pub metatable: RefCell<Option<GcRef<LuaTable>>>,
}

impl LuaUserData {
    pub fn placeholder() -> Self {
        LuaUserData {
            data: Box::new([]),
            uv: Vec::new(),
            metatable: RefCell::new(None),
        }
    }

    pub fn metatable(&self) -> Option<GcRef<LuaTable>> {
        self.metatable.borrow().clone()
    }

    pub fn set_metatable(&self, mt: Option<GcRef<LuaTable>>) {
        *self.metatable.borrow_mut() = mt;
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lobject.h (Udata + Udata0)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         LuaUserData type, including metatable slot and uservalues. C uses a
//                  flexible-array trailing payload; we use a typed Vec / Box of the
//                  user payload + uservalues vector.
// ──────────────────────────────────────────────────────────────────────────────
