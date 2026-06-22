//! Phase-D `Trace` implementations for types defined in this crate.
//!
//! Each impl enumerates the type's GC-bearing fields and either calls
//! `field.trace(m)` (delegating to the field's own `Trace` impl) or
//! `m.mark(field)` (when the field is a `Gc<T>` from `lua-gc`). During the
//! Phase A/B/C/D-0 window `GcRef<T>` is still an `Rc<T>` newtype rather
//! than the real `Gc<T>`, so the mark-queue path is not yet reachable —
//! method resolution dispatches through `Deref` to each underlying type's
//! own `trace` method.

use crate::closure::{LuaCClosure, LuaClosure, LuaLClosure};
use crate::gc::GcRef;
use crate::proto::LuaProto;
use crate::string::LuaString;
use crate::table::LuaTable;
use crate::upval::UpVal;
use crate::userdata::LuaUserData;
use crate::value::LuaThread;
use crate::value::LuaValue;
use lua_gc::{Marker, Trace};

/// Forwarder for `GcRef<T>`. Now that `GcRef` wraps a real `lua_gc::Gc<T>`
/// (D-1e), tracing must enqueue the box onto the gray queue via
/// `Marker::mark` — that is what flips its header color from White to Gray
/// and ultimately to Black during gray-queue drainage. The previous
/// `try_visit` short-circuit was a Phase A-D-0 workaround for the
/// `Rc`-backed handle (no header, no color), and produced a silent bug
/// post-D-1e: every GC-tracked allocation stayed White and was freed in
/// the sweep on the first `collectgarbage()`. Cycles are now handled
/// natively by the heap's gray-queue (Color::Gray check in `mark` makes
/// re-visits idempotent).
impl<T: Trace + 'static> Trace for GcRef<T> {
    fn trace(&self, m: &mut Marker) {
        m.mark(self.0);
    }
}

/// LuaValue — central enum. Variants Nil/Bool/Int/Float/LightUserData carry
/// no GC; Str/Table/Function/UserData/Thread carry collectable payloads.
impl Trace for LuaValue {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        match self {
            LuaValue::Nil
            | LuaValue::Bool(_)
            | LuaValue::Int(_)
            | LuaValue::Float(_)
            | LuaValue::LightUserData(_) => {}
            LuaValue::Str(s) => s.trace(m),
            LuaValue::Table(t) => t.trace(m),
            LuaValue::Function(c) => c.trace(m),
            LuaValue::UserData(u) => {
                u.trace(m);
            }
            LuaValue::Thread(t) => {
                // Mark the thread identity itself. lua-vm's GC post-mark hook
                // uses the visited identities to trace only reachable
                // suspended LuaState stacks.
                t.trace(m);
            }
        }
    }
}

/// LuaString — interned byte string. The `Rc<[u8]>` backing buffer is
/// owned, not GC-managed, so this impl is intentionally empty.
impl Trace for LuaString {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, _m: &mut Marker) {}
}

/// UpVal — Open (refers to a thread stack slot by index) or Closed (owns a
/// LuaValue). The Open variant carries no direct GC reference; the slot it
/// points at is traced through the owning thread's stack walk.
impl Trace for UpVal {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        if self.try_open_payload().is_some() {
            return;
        }
        if let Some(v) = self.try_closed_value() {
            v.trace(m);
        }
    }
}

/// LuaTable — array+hash entries plus optional metatable.
///
/// Weak-table semantics (matches `lgc.c::traversetable`):
///   * `__mode = "v"` — strong keys, weak values. Trace keys here; value
///     side is deferred — string values get marked in `prune_weak_dead`'s
///     surviving-entry pass (Lua's `iscleared`), non-string dead values
///     trigger entry removal.
///   * `__mode = "kv"` — both sides weak. Trace NEITHER here; everything
///     is handled by `prune_weak_dead` (matches Lua's "just add to allweak,
///     traverse nothing" path).
///   * `__mode = "k"` — weak keys, strong values. Trace NEITHER here. The
///     post-mark ephemeron convergence pass walks each weak-key table's
///     entries and marks values only for entries whose keys are
///     independently reachable. String keys get marked in `prune_weak_dead`.
///   * No `__mode` — trace both unconditionally.
///
/// Marking strings inline for weak slots (the previous behavior) would
/// pin them alive even when their containing entry is about to be cleared
/// because the other side died — breaking the `gc.lua` weak-string-key
/// block, which expects unreferenced long strings to free their bytes
/// after a single `collectgarbage()` cycle.
impl Trace for LuaTable {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        const WEAK_KEYS: u8 = 1;
        const WEAK_VALUES: u8 = 1 << 1;
        let mode = self.weak_mode();
        let trace_keys = (mode & WEAK_KEYS) == 0;
        let trace_values = (mode & WEAK_VALUES) == 0 && trace_keys;
        if trace_keys && trace_values {
            self.trace_entries_with_clearkey(|v| v.trace(m));
        } else {
            self.for_each_entry(|k, v| {
                if trace_keys {
                    k.trace(m);
                }
                if trace_values {
                    v.trace(m);
                }
            });
        }
        if let Some(mt) = self.metatable() {
            mt.trace(m);
        }
    }
}

/// LuaProto — bytecode prototype. k (constants), p (child protos),
/// source, upvalue names, locvar names.
///
/// The `cache` field is the last closure instantiated from this proto
/// (5.2/5.3 `LUA_COMPAT` closure caching). It is a non-owning weak edge,
/// mirroring C's `traverseproto` (`lgc.c` 5.3.6:481-482): `if (f->cache &&
/// iswhite(f->cache)) f->cache = NULL;`. C drops a white cache during the
/// proto traversal so the cached closure is free to be collected; it never
/// marks the cache. We do the same — drop the cache when its target has not
/// been independently reached (`!is_visited`) and never `mark` it. The cache
/// is purely an optimization, so an early drop only costs a later
/// `OP_CLOSURE` cache miss (it rebuilds a fresh closure), never correctness.
/// Marking it strongly (the previous behavior) over-rooted the last closure
/// of every proto, which kept a `__gc` closure stored as a weak metatable
/// value alive past the cycle that should have cleared it — making the
/// `gc.lua` `__gc x weak tables` finalizer fire when it must not.
impl Trace for LuaProto {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        for v in self.k.iter() {
            v.trace(m);
        }
        for p in self.p.iter() {
            p.trace(m);
        }
        if let Some(src) = &self.source {
            src.trace(m);
        }
        for uv in self.upvalues.iter() {
            if let Some(name) = &uv.name {
                name.trace(m);
            }
        }
        for lv in self.locvars.iter() {
            lv.varname.trace(m);
        }
        let drop_cache = self
            .cache
            .borrow()
            .as_ref()
            .map_or(false, |c| !m.is_visited(c.identity()));
        if drop_cache {
            *self.cache.borrow_mut() = None;
        }
    }
}

/// LuaLClosure — Lua closure carrying a Proto and its captured upvalues.
impl Trace for LuaLClosure {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        self.proto.trace(m);
        for uv in self.upvals.iter() {
            uv.get().trace(m);
        }
    }
}

/// LuaClosure — dispatch to Lua/C variants; LightC is a bare function-ptr
/// index with no payload.
impl Trace for LuaClosure {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        match self {
            LuaClosure::Lua(l) => l.trace(m),
            LuaClosure::C(c) => c.trace(m),
            LuaClosure::LightC(_) => {}
        }
    }
}

/// LuaCClosure — Rust-side C closure carrying captured upvalues.
impl Trace for LuaCClosure {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        for v in self.upvalues.borrow().iter() {
            v.trace(m);
        }
    }
}

/// LuaUserData — boxed payload + optional metatable + user values.
impl Trace for LuaUserData {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, m: &mut Marker) {
        if let Some(mt) = self.metatable() {
            mt.trace(m);
        }
        for v in self.uv.borrow().iter() {
            v.trace(m);
        }
    }
}

/// LuaThread — value-side thread identity. Carries only a `ThreadId`
/// (the registry key); the real per-thread `LuaState` lives in
/// `lua-vm`'s `GlobalState::threads` map and is traced from
/// `GlobalState::trace` as a root.
impl Trace for LuaThread {

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn trace(&self, _m: &mut Marker) {}
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        n/a (GC Trace impls scoped to lua-types public surface)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Trace impls for GC visitor over the canonical type set. No C analogue;
//                  the C GC walks struct fields directly via macros.
// ──────────────────────────────────────────────────────────────────────────────
