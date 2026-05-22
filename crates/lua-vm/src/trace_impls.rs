//! Phase-D `Trace` implementations for every GC-rooted type.
//!
//! Each impl below is a `todo!("phase-d: trace X — visit every Gc field")`
//! stub. The panic-driven mega-loop will surface each one when a runtime
//! exercises a code path that triggers `Heap::full_collect`. The
//! family-aware DEBUG agent for `phase-d: trace X` should:
//!
//! 1. Read the type definition to enumerate every field
//! 2. For every field of type `Gc<T>`, `GcRef<T>`, or a container thereof
//!    (Vec, Option, HashMap), call `m.mark(field)` or `field.trace(m)`
//!    appropriately
//! 3. Skip non-GC fields (primitives, `String`, `Vec<u8>`)
//! 4. Skip fields documented as "intentionally not traced" (e.g. weak refs)
//! 5. Reference `reference/lua-5.4.7/src/lgc.c`'s `reallymarkobject` switch
//!    for the canonical C version
//!
//! Each agent works on ONE type. No family expansion within this file (each
//! Trace impl has subtle invariants — better to dispatch precisely).

use lua_gc::{Marker, Trace};
use lua_types::value::LuaValue;
use lua_types::upval::UpVal;
use lua_types::LuaString;
use crate::state::{LuaState, GlobalState, LuaTable, LuaProto, LuaClosure, LuaClosureLua};

// ─────────────────────────────────────────────────────────────────────────
// LuaValue — the central enum. Most-called Trace impl. Must visit every
// GC variant (Str, Table, Function, UserData, Thread). Nil/Bool/Int/Float
// have no GC content and are no-ops.
// ─────────────────────────────────────────────────────────────────────────
impl Trace for LuaValue {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace LuaValue — match each GC-bearing variant (Str, Table, Function, UserData, Thread) and mark via m.mark");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LuaString — interned byte string. No outgoing GC edges. trace = no-op.
// (Listed for completeness; agent should implement as empty body, NOT
// todo!.)
// ─────────────────────────────────────────────────────────────────────────
impl Trace for LuaString {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace LuaString — verify no GC fields; body should be empty");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LuaTable — array part + hash part + metatable. The hot path of GC.
// Visit every value in array, every (k, v) in hash, the metatable, and
// any cached __index/__newindex (if cached as separate fields).
// ─────────────────────────────────────────────────────────────────────────
impl Trace for LuaTable {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace LuaTable — array, hash (k+v), metatable");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LuaProto — bytecode prototype. Visits k (constants), p (child protos),
// upvalues (their default-value refs), source (Gc<LuaString>), debug info
// (locvars names, upvalue names).
// ─────────────────────────────────────────────────────────────────────────
impl Trace for LuaProto {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace LuaProto — k constants, child protos, source, upvalue names, locvar names");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LuaClosureLua — Lua closure = proto + captured upvalues. (LuaCClosure
// for C closures handled via the LuaClosure enum's other variant.)
// ─────────────────────────────────────────────────────────────────────────
impl Trace for LuaClosureLua {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace LuaClosureLua — proto + upvalues");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LuaClosure (enum) — dispatches to Lua/C variants.
// ─────────────────────────────────────────────────────────────────────────
impl Trace for LuaClosure {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace LuaClosure — match Lua/C variants; LightC has only a usize");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// UpVal — open (points into thread stack) or closed (owns value).
// ─────────────────────────────────────────────────────────────────────────
impl Trace for UpVal {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace UpVal — Closed(value).trace; Open variant references stack, no direct GC field");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LuaState — a Lua coroutine. Has its own stack, callinfo chain, openupval
// list, plus a ref back to GlobalState (NOT traced — GlobalState owns the
// heap, would be circular).
// ─────────────────────────────────────────────────────────────────────────
impl Trace for LuaState {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace LuaState — stack, openupval, call_stack proto refs; do NOT trace global (held by Rc<RefCell<GlobalState>>)");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// GlobalState — root of all roots. Walks: registry table, mainthread,
// string pool (intern table values), per-type metatables, tmname array,
// memerrmsg, c_functions (no GC), twups, fixedgc.
// ─────────────────────────────────────────────────────────────────────────
impl Trace for GlobalState {
    fn trace(&self, _m: &mut Marker) {
        todo!("phase-d: trace GlobalState — l_registry, mainthread, strt pool, mt[..], tmname[..], memerrmsg, twups, fixedgc");
    }
}
