//! `LuaValue` — the tagged-union value type. PORT_STRATEGY §3.2.

use crate::closure::LuaClosure;
use crate::gc::GcRef;
use crate::string::LuaString;
use crate::userdata::LuaUserData;
use std::ffi::c_void;

pub use crate::table::LuaTable;

/// Copies a value as two separate 8-byte loads.
///
/// Inline asm keeps LLVM from merging the two loads into one 16-byte load.
/// Miri cannot run inline asm, so it takes the plain copy below.
#[cfg(all(target_arch = "x86_64", not(miri)))]
#[allow(unsafe_code)]
#[inline(always)]
fn load_words(v: &LuaValue) -> LuaValue {
    /// The two words of a `LuaValue`: the tag word, then the payload word,
    /// which holds the pointer for every pointer-carrying variant.
    #[repr(C)]
    struct Words {
        tag: u64,
        payload: *const u8,
    }
    const _: () = {
        assert!(core::mem::size_of::<LuaValue>() == core::mem::size_of::<Words>());
        assert!(core::mem::align_of::<LuaValue>() == core::mem::align_of::<Words>());
    };
    let p = v as *const LuaValue as *const u8;
    let (tag, payload): (u64, *const u8);
    // SAFETY: `v` is a valid, initialized 16-byte `LuaValue` and the asm only
    // reads those 16 bytes. The payload is a pointer-typed output, so a GC or
    // light-userdata pointer keeps the provenance it had in `*v`. The two
    // words are the exact bytes of `*v`, so the transmute yields a bitwise
    // copy of a valid value, and `LuaValue` is `Copy`.
    unsafe {
        core::arch::asm!(
            "mov {tag}, qword ptr [{p}]",
            "mov {payload}, qword ptr [{p} + 8]",
            p = in(reg) p,
            tag = out(reg) tag,
            payload = lateout(reg) payload,
            options(nostack, preserves_flags, readonly, pure),
        );
        core::mem::transmute::<Words, LuaValue>(Words { tag, payload })
    }
}

#[cfg(not(all(target_arch = "x86_64", not(miri))))]
#[inline(always)]
fn load_words(v: &LuaValue) -> LuaValue {
    *v
}

/// The dynamically-typed Lua value. Replaces C's `TValue`.
#[derive(Debug, Clone, Copy)]
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
    /// Copies the value, reading its tag word and payload word separately.
    ///
    /// A plain copy compiles to one 16-byte load. Writing a fresh value
    /// compiles to a tag store and a payload store, and x86 cannot forward
    /// two narrower in-flight stores into one wider load, so a copy that
    /// closely follows such a write stalls until the stores retire. Two
    /// 8-byte loads forward from either store shape. Use this for stack-slot
    /// copies on the interpreter's hot paths.
    #[inline(always)]
    pub fn load(&self) -> LuaValue {
        load_words(self)
    }

    pub fn type_tag(&self) -> crate::LuaType {
        use crate::LuaType::*;
        match self {
            LuaValue::Nil => Nil,
            LuaValue::Bool(_) => Boolean,
            LuaValue::Int(_) => Number,
            LuaValue::Float(_) => Number,
            LuaValue::Str(_) => String,
            LuaValue::Table(_) => Table,
            LuaValue::Function(_) => Function,
            LuaValue::UserData(_) => UserData,
            LuaValue::LightUserData(_) => LightUserData,
            LuaValue::Thread(_) => Thread,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            LuaValue::Nil => "nil",
            LuaValue::Bool(_) => "boolean",
            LuaValue::Int(_) => "number",
            LuaValue::Float(_) => "number",
            LuaValue::Str(_) => "string",
            LuaValue::Table(_) => "table",
            LuaValue::Function(_) => "function",
            LuaValue::UserData(_) => "userdata",
            LuaValue::LightUserData(_) => "userdata",
            LuaValue::Thread(_) => "thread",
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, LuaValue::Nil)
    }
    pub fn is_falsy(&self) -> bool {
        matches!(self, LuaValue::Nil | LuaValue::Bool(false))
    }
    pub fn is_truthy(&self) -> bool {
        !self.is_falsy()
    }
    pub fn is_collectable(&self) -> bool {
        matches!(
            self,
            LuaValue::Str(_)
                | LuaValue::Table(_)
                | LuaValue::Function(_)
                | LuaValue::UserData(_)
                | LuaValue::Thread(_)
        )
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            LuaValue::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_float(&self) -> Option<f64> {
        match self {
            LuaValue::Float(f) => Some(*f),
            _ => None,
        }
    }
    pub fn as_string(&self) -> Option<&GcRef<LuaString>> {
        match self {
            LuaValue::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&GcRef<LuaTable>> {
        match self {
            LuaValue::Table(t) => Some(t),
            _ => None,
        }
    }
}

impl Default for LuaValue {
    fn default() -> Self {
        LuaValue::Nil
    }
}

impl PartialEq for LuaValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LuaValue::Nil, LuaValue::Nil) => true,
            (LuaValue::Bool(a), LuaValue::Bool(b)) => a == b,
            (LuaValue::Int(a), LuaValue::Int(b)) => a == b,
            (LuaValue::Float(a), LuaValue::Float(b)) => a == b,
            (LuaValue::Str(a), LuaValue::Str(b)) => {
                GcRef::ptr_eq(a, b) || (a.hash() == b.hash() && a.as_bytes() == b.as_bytes())
            }
            (LuaValue::Table(a), LuaValue::Table(b)) => GcRef::ptr_eq(a, b),
            (LuaValue::Function(a), LuaValue::Function(b)) => closure_eq(a, b),
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

// LuaTable now lives in `crate::table` as the canonical array+hash
// implementation. The variant signature stays `LuaValue::Table(GcRef<LuaTable>)`.

fn closure_eq(a: &LuaClosure, b: &LuaClosure) -> bool {
    match (a, b) {
        (LuaClosure::Lua(x), LuaClosure::Lua(y)) => GcRef::ptr_eq(x, y),
        (LuaClosure::C(x), LuaClosure::C(y)) => GcRef::ptr_eq(x, y),
        (LuaClosure::LightC(x), LuaClosure::LightC(y)) => x == y,
        _ => false,
    }
}

/// Identity of a Lua thread (coroutine).
///
/// The real per-thread `LuaState` lives in `lua-vm` and is held by
/// `GlobalState` keyed by this id. `LuaValue::Thread` carries a
/// `GcRef<LuaThread>` so that pointer-equality of the wrapping `GcRef`
/// still implements thread-identity comparison, but the only payload is
/// the registry key — keeping `LuaState` outside `lua-types` avoids the
/// `lua-types` → `lua-vm` crate cycle.
///
/// Convention: `id == 0` is reserved for the main thread. Coroutines are
/// assigned ids starting at 1.
#[derive(Debug)]
pub struct LuaThread {
    pub id: u64,
}
impl LuaThread {
    pub fn new(id: u64) -> Self {
        LuaThread { id }
    }
    pub fn placeholder() -> Self {
        LuaThread { id: 0 }
    }
}

#[cfg(test)]
mod load_tests {
    use super::LuaValue;

    fn same_bits(a: &LuaValue, b: &LuaValue) -> bool {
        match (a, b) {
            (LuaValue::Float(x), LuaValue::Float(y)) => x.to_bits() == y.to_bits(),
            _ => a == b,
        }
    }

    #[test]
    fn load_copies_immediate_values() {
        let mut light = 7u8;
        let values = [
            LuaValue::Nil,
            LuaValue::Bool(false),
            LuaValue::Bool(true),
            LuaValue::Int(0),
            LuaValue::Int(i64::MIN),
            LuaValue::Int(-1),
            LuaValue::Float(-0.0),
            LuaValue::Float(f64::from_bits(0x7ff8_0000_dead_beef)),
            LuaValue::LightUserData(&mut light as *mut u8 as *mut std::ffi::c_void),
        ];
        for v in &values {
            assert!(same_bits(&v.load(), v), "{v:?}");
        }
    }
}
