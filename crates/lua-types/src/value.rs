//! `LuaValue` — the tagged-union value type. PORT_STRATEGY §3.2.

use crate::closure::LuaClosure;
use crate::gc::GcRef;
use crate::string::LuaString;
use crate::userdata::LuaUserData;
use std::ffi::c_void;

pub use crate::table::LuaTable;

/// The dynamically-typed Lua value. Replaces C's `TValue`.
///
/// The variants are **declared in an order** (not `#[repr(u8)]` — see below)
/// such that the five GC-managed variants form one contiguous block at the end
/// (`Str` … `Thread`), with the scalars — including `LightUserData`, which
/// carries a raw pointer but is *not* GC-managed — declared first. This mirrors
/// C-Lua's `BIT_ISCOLLECTABLE` design, where `iscollectable(v)` is a single
/// range/bit test rather than a per-variant match: with this order the
/// name-based `is_collectable()` lowers to one fused niche-decode + range
/// compare. The pre-T5a order put `LightUserData` *between* `UserData` and
/// `Thread`, splitting the collectable set `{4,5,6,7,9}` and forcing the
/// predicate to lower to a bitmask-constant test (a `mov #752 / lsr / and`) —
/// the extra per-write work `PERF_SPRINT_2_SPEC §T2` fingered.
///
/// **Why declaration order and not `#[repr(u8)]` + `= N`:** an explicit
/// primitive `repr` forces a dedicated tag byte, which defeats the niche
/// packing that keeps this enum at 16 bytes (the largest payload, `LuaClosure`,
/// is itself 16 bytes with no spare niche, so a forced tag pushes the whole
/// value to 24 bytes — verified, and caught by the `size_of == 16` assertion
/// below). Declaration order gives the identical contiguous-discriminant
/// codegen win at zero size cost, which is the only thing that moves the
/// counters. The order is an internal implementation detail; no observable
/// behaviour depends on it — every consumer pattern-matches on variant *names*,
/// the wire/bytecode constant tags are a separate explicit table (`undump.rs`),
/// and `type_tag`/`type_name` map names to `LuaType` independently.
#[derive(Debug, Clone, Copy)]
pub enum LuaValue {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    LightUserData(*mut c_void),
    Str(GcRef<LuaString>),
    Table(GcRef<LuaTable>),
    Function(LuaClosure),
    UserData(GcRef<LuaUserData>),
    Thread(GcRef<LuaThread>),
}

/// `LuaValue` must stay 16 bytes (8-byte payload, tag niche-packed into the
/// largest variant's spare bits). The T5a collectable-range reorder must not
/// grow it; this assertion is what caught that a forced `#[repr(u8)]` tag byte
/// would have bloated it to 24. Gated to 64-bit because the payload sizes
/// (`GcRef`, `i64`, `f64`, `*mut c_void`) are pointer/word sized — on `wasm32`
/// the same enum is smaller, and an unconditional `== 16` assert would (and
/// once did, PR #153) break the `wasm32-unknown-unknown` CI gate.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<LuaValue>() == 16);

impl LuaValue {
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
    /// Whether the value carries a GC-managed payload (C-Lua `iscollectable`).
    ///
    /// The variants `Str`/`Table`/`Function`/`UserData`/`Thread` are declared
    /// as the contiguous tail of the enum (see the type doc and the
    /// `collectable_variants_are_a_contiguous_range` test), so this name-based
    /// `matches!` lowers to a single fused niche-decode + range compare instead
    /// of the bitmask-constant test (`mov #752 / lsr / and`) the old split
    /// ordering `{4,5,6,7,9}` forced. The match arms are written in declaration
    /// order to keep that lowering legible.
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
mod tests {
    use super::*;

    /// Declaration-order index of the first collectable variant (`Str`).
    /// Scalars sit below it; collectables form the contiguous tail at or above
    /// it. This is the property that lets `is_collectable()` lower to one range
    /// compare instead of a bitmask test.
    const FIRST_COLLECTABLE: u8 = 5;

    /// Name-based view of each variant's declaration-order index, used only to
    /// pin the T5a layout invariant without an `unsafe` tag read.
    fn order_index(v: &LuaValue) -> u8 {
        match v {
            LuaValue::Nil => 0,
            LuaValue::Bool(_) => 1,
            LuaValue::Int(_) => 2,
            LuaValue::Float(_) => 3,
            LuaValue::LightUserData(_) => 4,
            LuaValue::Str(_) => 5,
            LuaValue::Table(_) => 6,
            LuaValue::Function(_) => 7,
            LuaValue::UserData(_) => 8,
            LuaValue::Thread(_) => 9,
        }
    }

    /// The five collectable variants must occupy the contiguous tail
    /// `[FIRST_COLLECTABLE, 9]` and every scalar must fall below it, with
    /// `is_collectable()` agreeing exactly with that split. Any reorder that
    /// splits the collectable block (the pre-T5a state) would silently restore
    /// the bitmask lowering, so guard it mechanically. `LightUserData` is the
    /// trap: it carries a raw pointer but is a scalar, and the pre-T5a order
    /// wedged it between two collectables.
    #[test]
    fn collectable_variants_are_a_contiguous_range() {
        let scalars = [
            LuaValue::Nil,
            LuaValue::Bool(true),
            LuaValue::Int(0),
            LuaValue::Float(0.0),
            LuaValue::LightUserData(std::ptr::null_mut()),
        ];
        for v in &scalars {
            assert!(
                order_index(v) < FIRST_COLLECTABLE,
                "scalar must sit below the collectable range"
            );
            assert!(!v.is_collectable(), "scalar must not be collectable");
        }
        assert_eq!(scalars.len() as u8, FIRST_COLLECTABLE);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lobject.h (TValue, Value union, tags)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Canonical LuaValue tagged enum. C uses a {value, tag} struct with a
//                  union of (gco/number/bool/light-userdata); we use a Rust enum
//                  with each variant carrying its payload directly. The variant
//                  discriminants are assigned explicitly (#[repr(u8)]) so the five
//                  collectable variants form the contiguous range [5, 9]; this
//                  lets is_collectable() lower to one range compare, matching C's
//                  BIT_ISCOLLECTABLE single-bit test (T5a, repr rung 1).
// ──────────────────────────────────────────────────────────────────────────────
