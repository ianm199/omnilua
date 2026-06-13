//! `LuaClosure` — the function variant of `LuaValue`. Three sub-kinds:
//! Lua closure (compiled Proto + upvalues), C closure (function pointer +
//! upvalues), light C function (function pointer, no upvalues).

use std::cell::{Cell, RefCell};

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

#[derive(Debug, Clone, Copy)]
pub enum LuaClosure {
    Lua(GcRef<LuaLClosure>),
    C(GcRef<LuaCClosure>),
    LightC(LuaCFnPtr),
}

#[derive(Debug)]
pub struct LuaLClosure {
    pub proto: GcRef<LuaProto>,
    /// Each upvalue slot is held in a `Cell` so that `debug.upvaluejoin`
    /// can replace an entry with another closure's slot without rebuilding
    /// the (shared) closure. `GcRef<UpVal>` is `Copy` (thin wrapper over
    /// `Gc<UpVal>`), so a plain `Cell` is sufficient and skips RefCell
    /// borrow tracking on every upvalue read — critical for the
    /// `upvalue_get` hot path.
    ///
    /// The slice is `Box<[_]>`, not `Vec<_>`: the upvalue count is fixed at
    /// closure creation (`proto.upvalues.len()`) and never grows or shrinks
    /// afterwards (`set_upval` is a `Cell::set` into an existing slot, not a
    /// resize). Dropping the `Vec` capacity word shrinks `LuaLClosure` by one
    /// pointer and makes `buffer_bytes` an exact GC-accounting figure.
    pub upvals: Box<[Cell<GcRef<UpVal>>]>,
}

/// `LuaLClosure` is a GC-traced heap object; every byte multiplies across the
/// live closure population. Switching `upvals` from `Vec` (ptr+len+cap) to
/// `Box<[_]>` (ptr+len) drops the capacity word, taking the struct from 32 to
/// 24 bytes on a 64-bit target. Gated to 64-bit because the byte count is a
/// pointer-width claim (the wasm32 CI build has a 32-bit layout).
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<LuaLClosure>() == 24);

#[derive(Debug)]
pub struct LuaCClosure {
    pub func: LuaCFnPtr,
    pub upvalues: RefCell<Vec<LuaValue>>,
}

impl LuaLClosure {
    pub fn placeholder() -> Self {
        LuaLClosure {
            proto: GcRef::new(LuaProto::placeholder()),
            upvals: Box::new([]),
        }
    }

    /// Returns the upvalue slot at index `i`. Cheap (Copy of a one-pointer
    /// `GcRef<UpVal>`).
    #[inline(always)]
    pub fn upval(&self, i: usize) -> GcRef<UpVal> {
        self.upvals[i].get()
    }

    /// Replaces the upvalue slot at index `i` with `new`. Used by
    /// `debug.upvaluejoin` to share an upvalue between two closures.
    pub fn set_upval(&self, i: usize, new: GcRef<UpVal>) {
        self.upvals[i].set(new);
    }

    /// Bytes owned outside the `GcBox` header/object allocation.
    ///
    /// `Box<[_]>` has no separate capacity; `len` is the exact slot count,
    /// which is also the faithful GC-accounting figure.
    pub fn buffer_bytes(&self) -> usize {
        self.upvals.len() * std::mem::size_of::<Cell<GcRef<UpVal>>>()
    }
}

impl LuaCClosure {
    /// Bytes owned outside the `GcBox` header/object allocation.
    pub fn buffer_bytes(&self) -> usize {
        self.upvalues.borrow().capacity() * std::mem::size_of::<LuaValue>()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lobject.h (CClosure / LClosure / Closure union)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         LuaClosure enum covering the C-Lua C/LightC/Lua closure variants.
//                  C uses a union with a common header; we use a tagged enum.
//                  LuaLClosure.upvals uses Cell<GcRef<UpVal>> (not RefCell) so per-
//                  upvalue reads avoid borrow-tracking; GcRef<UpVal> is Copy. The
//                  upvals slice is Box<[_]> (fixed count, never resized after
//                  construction), making LuaLClosure 24 bytes on 64-bit.
// ──────────────────────────────────────────────────────────────────────────────
