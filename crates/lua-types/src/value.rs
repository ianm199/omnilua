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

    pub fn type_name(&self) -> &'static str {
        match self {
            LuaValue::Nil               => "nil",
            LuaValue::Bool(_)           => "boolean",
            LuaValue::Int(_)            => "number",
            LuaValue::Float(_)          => "number",
            LuaValue::Str(_)            => "string",
            LuaValue::Table(_)          => "table",
            LuaValue::Function(_)       => "function",
            LuaValue::UserData(_)       => "userdata",
            LuaValue::LightUserData(_)  => "userdata",
            LuaValue::Thread(_)         => "thread",
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

// Heap-allocated value types. LuaTable now holds real (Vec-backed) storage —
// previously this was a placeholder unit struct and writes/reads were no-ops,
// causing `print` registration to silently fail in `open_libs`. The rich
// array+hash version in `lua-vm/src/table.rs` is a Phase-D performance
// upgrade target; the simple Vec-pair implementation here is correct for
// Lua semantics and unblocks the runtime.

use std::cell::{Cell, RefCell};

const WEAK_KEYS: u8 = 1 << 0;
const WEAK_VALUES: u8 = 1 << 1;

#[derive(Debug, Default)]
pub struct LuaTable {
    entries: RefCell<Vec<(LuaValue, LuaValue)>>,
    metatable: RefCell<Option<GcRef<LuaTable>>>,
    weak_mode: Cell<u8>,
}

impl LuaTable {
    pub fn placeholder() -> Self { Self::default() }

    /// Read a key; returns `LuaValue::Nil` if absent or if key is nil.
    ///
    /// If this table has weak keys or values (`__mode`), any matching entry
    /// whose weakly-tagged component has no other strong holders (Rc
    /// `strong_count == 1`) is removed and `Nil` is returned. This is the
    /// Phase-A/B/C stand-in for the real incremental sweeper.
    pub fn get(&self, k: &LuaValue) -> LuaValue {
        if matches!(k, LuaValue::Nil) { return LuaValue::Nil; }
        let mode = self.weak_mode.get();
        if mode != 0 {
            let entries = self.entries.borrow();
            let found = entries.iter().position(|(ek, _)| lua_key_eq(ek, k));
            if let Some(i) = found {
                let (ek, ev) = &entries[i];
                let dead = entry_is_weakly_dead(ek, ev, mode);
                if dead {
                    drop(entries);
                    self.entries.borrow_mut().swap_remove(i);
                    return LuaValue::Nil;
                }
                return ev.clone();
            }
            return LuaValue::Nil;
        }
        for (ek, ev) in self.entries.borrow().iter() {
            if lua_key_eq(ek, k) { return ev.clone(); }
        }
        LuaValue::Nil
    }

    /// Lookup by short-string key (used by metatable __index lookups).
    pub fn get_short_str(&self, k: &GcRef<crate::string::LuaString>) -> LuaValue {
        let key = LuaValue::Str(k.clone());
        self.get(&key)
    }

    /// Raw set without metamethod dispatch. nil keys are rejected (Lua
    /// semantics: `table[nil] = x` is an error; we silently ignore here
    /// since callers should validate). Setting a value to nil removes it.
    pub fn raw_set(&self, k: LuaValue, v: LuaValue) {
        if matches!(k, LuaValue::Nil) { return; }
        let mut entries = self.entries.borrow_mut();
        for i in 0..entries.len() {
            if lua_key_eq(&entries[i].0, &k) {
                if matches!(v, LuaValue::Nil) {
                    entries.swap_remove(i);
                } else {
                    entries[i].1 = v;
                }
                return;
            }
        }
        if !matches!(v, LuaValue::Nil) {
            entries.push((k, v));
        }
    }

    pub fn metatable(&self) -> Option<GcRef<LuaTable>> {
        self.metatable.borrow().clone()
    }

    pub fn set_metatable(&self, mt: Option<GcRef<LuaTable>>) {
        let mode = mt.as_ref().map(|t| extract_weak_mode(t)).unwrap_or(0);
        eprintln!("[set_metatable] mode={} mt_has_entries={}", mode, mt.as_ref().map(|t| t.entries.borrow().len()).unwrap_or(0));
        self.weak_mode.set(mode);
        *self.metatable.borrow_mut() = mt;
    }

    pub fn len(&self) -> usize { self.entries.borrow().len() }
    pub fn is_empty(&self) -> bool { self.entries.borrow().is_empty() }

    /// Implements Lua's `next(t, k)` for iteration. When `k` is `Nil`,
    /// returns the first entry. Otherwise returns the entry that follows
    /// `k` in insertion order. Returns `None` when iteration is done.
    ///
    /// Skips weakly-dead entries on the way; this keeps `pairs()` over a
    /// weak table from observing collected slots.
    pub fn next_pair(&self, k: &LuaValue) -> Option<(LuaValue, LuaValue)> {
        let mode = self.weak_mode.get();
        if mode != 0 {
            let mut entries = self.entries.borrow_mut();
            entries.retain(|(ek, ev)| !entry_is_weakly_dead(ek, ev, mode));
        }
        let entries = self.entries.borrow();
        if matches!(k, LuaValue::Nil) {
            return entries.first().cloned();
        }
        let mut idx = None;
        for (i, (ek, _)) in entries.iter().enumerate() {
            if lua_key_eq(ek, k) { idx = Some(i); break; }
        }
        match idx {
            Some(i) => entries.get(i + 1).cloned(),
            None    => None,
        }
    }
}

/// Inspect a metatable's `__mode` field (a string of any combination of
/// 'k' and 'v') and produce the corresponding `WEAK_KEYS | WEAK_VALUES`
/// bitmask. Returns 0 when no `__mode` is set or it is not a string.
fn extract_weak_mode(mt: &LuaTable) -> u8 {
    let entries = mt.entries.borrow();
    for (k, v) in entries.iter() {
        if let LuaValue::Str(ks) = k {
            if ks.as_bytes() == b"__mode" {
                if let LuaValue::Str(vs) = v {
                    let bytes = vs.as_bytes();
                    let mut mode = 0u8;
                    if bytes.iter().any(|b| *b == b'k') { mode |= WEAK_KEYS; }
                    if bytes.iter().any(|b| *b == b'v') { mode |= WEAK_VALUES; }
                    return mode;
                }
                return 0;
            }
        }
    }
    0
}

/// True when the entry's weakly-tagged component(s) are held only by this
/// table (Rc `strong_count == 1`). Strings are skipped — they tend to be
/// interned in long-lived caches and would otherwise vanish from weak
/// caches almost immediately.
fn entry_is_weakly_dead(k: &LuaValue, v: &LuaValue, mode: u8) -> bool {
    if (mode & WEAK_KEYS) != 0 && weakly_held_alone(k) {
        return true;
    }
    if (mode & WEAK_VALUES) != 0 && weakly_held_alone(v) {
        return true;
    }
    false
}

fn weakly_held_alone(v: &LuaValue) -> bool {
    use std::rc::Rc;
    match v {
        LuaValue::Table(t)    => Rc::strong_count(&t.0) == 1,
        LuaValue::UserData(u) => Rc::strong_count(&u.0) == 1,
        LuaValue::Thread(th)  => Rc::strong_count(&th.0) == 1,
        LuaValue::Function(c) => match c {
            LuaClosure::Lua(x)    => Rc::strong_count(&x.0) == 1,
            LuaClosure::C(x)      => Rc::strong_count(&x.0) == 1,
            LuaClosure::LightC(_) => false,
        },
        _ => false,
    }
}

fn closure_eq(a: &LuaClosure, b: &LuaClosure) -> bool {
    match (a, b) {
        (LuaClosure::Lua(x), LuaClosure::Lua(y)) => GcRef::ptr_eq(x, y),
        (LuaClosure::C(x), LuaClosure::C(y)) => GcRef::ptr_eq(x, y),
        (LuaClosure::LightC(x), LuaClosure::LightC(y)) => x == y,
        _ => false,
    }
}

/// Key equality for hash-table lookup. Matches Lua semantics:
///   - Nil never equals anything (handled at call sites)
///   - Bool/Int/Float/String compare by value
///   - Int <-> Float compare numerically (Lua coerces)
///   - Table/Function/UserData/Thread compare by GcRef identity
fn lua_key_eq(a: &LuaValue, b: &LuaValue) -> bool {
    match (a, b) {
        (LuaValue::Nil, LuaValue::Nil) => true,
        (LuaValue::Bool(x), LuaValue::Bool(y)) => x == y,
        (LuaValue::Int(x), LuaValue::Int(y)) => x == y,
        (LuaValue::Float(x), LuaValue::Float(y)) => x == y,
        (LuaValue::Int(i), LuaValue::Float(f)) | (LuaValue::Float(f), LuaValue::Int(i)) => *f == *i as f64,
        (LuaValue::Str(x), LuaValue::Str(y)) => x.as_bytes() == y.as_bytes(),
        (LuaValue::Table(x), LuaValue::Table(y)) => GcRef::ptr_eq(x, y),
        (LuaValue::UserData(x), LuaValue::UserData(y)) => GcRef::ptr_eq(x, y),
        (LuaValue::Thread(x), LuaValue::Thread(y)) => GcRef::ptr_eq(x, y),
        (LuaValue::Function(x), LuaValue::Function(y)) => closure_eq(x, y),
        (LuaValue::LightUserData(x), LuaValue::LightUserData(y)) => x == y,
        _ => false,
    }
}

#[derive(Debug)]
pub struct LuaThread {
    _private: (),
}
impl LuaThread {
    pub fn placeholder() -> Self { LuaThread { _private: () } }
}
