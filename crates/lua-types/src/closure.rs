//! `LuaClosure` — the function variant of `LuaValue`. Three sub-kinds:
//! Lua closure (compiled Proto + upvalues), C closure (function pointer +
//! upvalues), light C function (function pointer, no upvalues).

use std::cell::RefCell;

use crate::gc::GcRef;
use crate::proto::LuaProto;
use crate::upval::UpVal;
use crate::value::LuaValue;

/// Opaque registry index into `GlobalState.c_functions`, where the real
/// `lua_CFunction` (`fn(&mut LuaState) -> Result<usize, LuaError>`) is stored.
/// Lua-types can't reference `LuaState` without a circular dep, so we keep
/// the closure variant type-erased here and resolve through the registry at
/// call time.
pub type LuaCFnPtr = usize;

#[derive(Debug, Clone)]
pub enum LuaClosure {
    Lua(GcRef<LuaLClosure>),
    C(GcRef<LuaCClosure>),
    LightC(LuaCFnPtr),
}

#[derive(Debug)]
pub struct LuaLClosure {
    pub proto: GcRef<LuaProto>,
    /// Each upvalue slot is held in a `RefCell` so that `debug.upvaluejoin`
    /// can replace an entry with another closure's slot without rebuilding
    /// the (shared) closure. The inner `GcRef<UpVal>` itself already carries
    /// interior mutability for the `Open → Closed` transition.
    pub upvals: Vec<RefCell<GcRef<UpVal>>>,
}

#[derive(Debug)]
pub struct LuaCClosure {
    pub func: LuaCFnPtr,
    pub upvalues: Vec<LuaValue>,
}

impl LuaLClosure {
    pub fn placeholder() -> Self {
        LuaLClosure {
            proto: GcRef::new(LuaProto::placeholder()),
            upvals: Vec::new(),
        }
    }

    /// Clones the upvalue slot at index `i`. Cheap (Rc clone).
    pub fn upval(&self, i: usize) -> GcRef<UpVal> {
        self.upvals[i].borrow().clone()
    }

    /// Replaces the upvalue slot at index `i` with `new`. Used by
    /// `debug.upvaluejoin` to share an upvalue between two closures.
    pub fn set_upval(&self, i: usize, new: GcRef<UpVal>) {
        *self.upvals[i].borrow_mut() = new;
    }
}
