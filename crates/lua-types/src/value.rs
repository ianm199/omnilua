//! `LuaValue` — the tagged-union value type. PORT_STRATEGY §3.2.

use crate::closure::LuaClosure;
use crate::gc::GcRef;
use crate::string::LuaString;
use crate::userdata::LuaUserData;
use std::ffi::c_void;

/// The dynamically-typed Lua value. Replaces C's `TValue`.
#[derive(Debug, Clone)]
pub enum LuaValue {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(GcRef<LuaString>),
    Table(GcRef<LuaTable>),
    Function(LuaClosure),
    UserData(GcRef<LuaUserData>),
    LightUserData(*mut c_void),
    Thread(GcRef<LuaThread>),
}

impl LuaValue {
    pub fn type_tag(&self) -> crate::LuaType {
        use crate::LuaType::*;
        match self {
            LuaValue::Nil               => Nil,
            LuaValue::Bool(_)           => Boolean,
            LuaValue::Int(_)            => Number,
            LuaValue::Float(_)          => Number,
            LuaValue::Str(_)            => String,
            LuaValue::Table(_)          => Table,
            LuaValue::Function(_)       => Function,
            LuaValue::UserData(_)       => UserData,
            LuaValue::LightUserData(_)  => LightUserData,
            LuaValue::Thread(_)         => Thread,
        }
    }

    pub fn is_nil(&self) -> bool   { matches!(self, LuaValue::Nil) }
    pub fn is_falsy(&self) -> bool { matches!(self, LuaValue::Nil | LuaValue::Bool(false)) }
    pub fn is_truthy(&self) -> bool { !self.is_falsy() }
    pub fn is_collectable(&self) -> bool {
        matches!(self,
            LuaValue::Str(_) | LuaValue::Table(_) | LuaValue::Function(_) |
            LuaValue::UserData(_) | LuaValue::Thread(_))
    }

    pub fn as_int(&self) -> Option<i64> {
        match self { LuaValue::Int(i) => Some(*i), _ => None }
    }
    pub fn as_float(&self) -> Option<f64> {
        match self { LuaValue::Float(f) => Some(*f), _ => None }
    }
    pub fn as_string(&self) -> Option<&GcRef<LuaString>> {
        match self { LuaValue::Str(s) => Some(s), _ => None }
    }
    pub fn as_table(&self) -> Option<&GcRef<LuaTable>> {
        match self { LuaValue::Table(t) => Some(t), _ => None }
    }
}

impl Default for LuaValue {
    fn default() -> Self { LuaValue::Nil }
}

impl PartialEq for LuaValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LuaValue::Nil, LuaValue::Nil) => true,
            (LuaValue::Bool(a), LuaValue::Bool(b)) => a == b,
            (LuaValue::Int(a), LuaValue::Int(b)) => a == b,
            (LuaValue::Float(a), LuaValue::Float(b)) => a == b,
            (LuaValue::Str(a), LuaValue::Str(b)) => GcRef::ptr_eq(a, b) || a.as_bytes() == b.as_bytes(),
            (LuaValue::Table(a), LuaValue::Table(b)) => GcRef::ptr_eq(a, b),
            (LuaValue::Function(_), LuaValue::Function(_)) => false, // TODO(port): closure equality
            (LuaValue::UserData(a), LuaValue::UserData(b)) => GcRef::ptr_eq(a, b),
            (LuaValue::LightUserData(a), LuaValue::LightUserData(b)) => a == b,
            (LuaValue::Thread(a), LuaValue::Thread(b)) => GcRef::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Float-to-integer rounding mode (matches C-Lua's F2Imod).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum F2Imod {
    Floor,
    Ceil,
    Round,
}

// Forward declarations for the heap-allocated value types. The real impls
// live in `lua-vm`, but we need the type names visible here so `LuaValue`
// variants type-check.

#[derive(Debug)]
pub struct LuaTable {
    _private: (),
}
impl LuaTable {
    pub fn placeholder() -> Self { LuaTable { _private: () } }
}

#[derive(Debug)]
pub struct LuaThread {
    _private: (),
}
impl LuaThread {
    pub fn placeholder() -> Self { LuaThread { _private: () } }
}
