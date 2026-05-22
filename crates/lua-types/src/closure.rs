//! `LuaClosure` — the function variant of `LuaValue`. Three sub-kinds:
//! Lua closure (compiled Proto + upvalues), C closure (function pointer +
//! upvalues), light C function (function pointer, no upvalues).

use crate::gc::GcRef;
use crate::proto::LuaProto;
use crate::upval::UpVal;
use crate::value::LuaValue;

/// Forward-declared function type that takes any "Lua-state-like" arg.
/// Real lua-vm crate will narrow this once `LuaState` exists. For now it's
/// a void pointer so callers can store function pointers without circular
/// deps. The Compiler-fixer pass replaces this with the real signature.
pub type LuaCFnPtr = unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;

#[derive(Debug, Clone)]
pub enum LuaClosure {
    Lua(GcRef<LuaLClosure>),
    C(GcRef<LuaCClosure>),
    LightC(LuaCFnPtr),
}

#[derive(Debug)]
pub struct LuaLClosure {
    pub proto: GcRef<LuaProto>,
    pub upvals: Vec<GcRef<UpVal>>,
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
}
