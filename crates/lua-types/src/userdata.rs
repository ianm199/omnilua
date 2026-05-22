//! `LuaUserData` — Lua's heap-allocated userdata. Carries a typed byte
//! buffer plus optional user values (a Vec of TValues).

use std::cell::RefCell;

use crate::gc::GcRef;
use crate::value::{LuaTable, LuaValue};

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
