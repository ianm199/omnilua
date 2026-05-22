//! Phase-D `Trace` implementations for GC-rooted types defined in this
//! crate. Types in `lua-types` (LuaValue, LuaString, UpVal) have their
//! Trace impls in `lua-types/src/trace_impls.rs` because of Rust's orphan
//! rule.
//!
//! Each impl below is a `todo!("phase-d: trace X")` stub. The
//! panic-driven mega-loop surfaces each one when a runtime path triggers
//! `Heap::full_collect`. Each agent works on ONE type — no family
//! expansion (Trace impls have subtle invariants).
//!
//! Implementation guidance for agents:
//!   1. Read the type definition; enumerate every field
//!   2. For every `Gc<T>`, `GcRef<T>`, or container (Vec/Option/HashMap)
//!      thereof, call `m.mark(field)` or `field.trace(m)` appropriately
//!   3. Skip non-GC fields (primitives, `String`, `Vec<u8>`)
//!   4. Skip "intentionally not traced" fields (weak refs)
//!   5. Reference `reference/lua-5.4.7/src/lgc.c`'s `reallymarkobject`

use lua_gc::{Marker, Trace};
use crate::state::{LuaState, GlobalState};
use crate::string::{LuaStringImpl, LuaUserDataImpl};
use crate::table::LuaTable as VmLuaTable;

/// Phase-B internal richer LuaString. The byte buffer is a Rust `Rc<[u8]>`
/// (not GC-managed); no fields to mark.
impl Trace for LuaStringImpl {
    fn trace(&self, _m: &mut Marker) {}
}

/// Phase-B internal userdata. Both `metatable` and `uv` are currently
/// `Option<()>` / `Vec<()>` stubs — no GC edges to walk yet. Becomes
/// real when userdata machinery lands post-D-1.
impl Trace for LuaUserDataImpl {
    fn trace(&self, _m: &mut Marker) {}
}

/// Phase-B internal LuaTable (separate from lua-types::LuaTable
/// placeholder).
impl Trace for VmLuaTable {
    fn trace(&self, m: &mut Marker) {
        for slot in self.array.iter() {
            slot.trace(m);
        }
        for entry in self.node.iter() {
            entry.key.trace(m);
            entry.value.trace(m);
        }
        if let Some(mt) = self.metatable.as_ref() {
            mt.trace(m);
        }
    }
}

impl Trace for LuaState {
    fn trace(&self, m: &mut Marker) {
        // C: `traversethread` in lgc.c walks the live portion of the stack
        // (`stack..top`) and the open-upvalue list. Slots past `top` are
        // dead and must not be visited.
        let top = self.top.0 as usize;
        let end = top.min(self.stack.len());
        for slot in &self.stack[..end] {
            slot.val.trace(m);
        }

        for uv in self.openupval.iter() {
            uv.trace(m);
        }

        // PORT NOTE: `global` (Rc<RefCell<GlobalState>>) is reached from the
        // heap's root via GlobalState::trace; tracing it from each thread
        // would re-enter the root and is explicitly excluded.
        // PORT NOTE: `call_info` entries carry pc offsets and stack indices
        // but no direct GcRef fields. The active closure is reached through
        // the stack slot at `ci.func`, already covered by the stack walk.
        // PORT NOTE: `tbclist` holds StackIdx values only; the to-be-closed
        // objects themselves live on the stack and are traced there.
    }
}

impl Trace for GlobalState {
    fn trace(&self, m: &mut Marker) {
        // C: `restartcollection` in lgc.c marks mainthread, l_registry, the
        // per-type metatables, and pending finalizers. We expand the set to
        // include preallocated short strings (memerrmsg, tmname[]) and the
        // open-upvalue thread list, both of which the panic-driven Phase-D
        // mega-loop expects to see at the root.

        self.l_registry.trace(m);

        if let Some(t) = &self.mainthread {
            t.trace(m);
        }

        for slot in self.mt.iter() {
            if let Some(t) = slot {
                t.trace(m);
            }
        }

        for s in self.tmname.iter() {
            s.trace(m);
        }

        self.memerrmsg.trace(m);

        for th in self.twups.iter() {
            th.trace(m);
        }

        // PORT NOTE: `strt` (intern table) is a weak table in C; entries are
        // cleared during the atomic weak-table pass (`clearbykeys`), not
        // marked as roots. Same for `strcache` and `interned_lt`.
        // PORT NOTE: `fixedgc` holds objects pre-marked fixed/black at
        // allocation (`luaC_fix`); the mark phase never re-visits them, and
        // `dyn Collectable` does not implement `Trace` here.
        // PORT NOTE: `allgc`, `finobj`, `gray`, `grayagain`, `tobefnz`,
        // `weak`, `ephemeron`, `allweak` are GC bookkeeping lists owned by
        // `heap` — they are the universe of allocated objects, not roots.
    }
}
