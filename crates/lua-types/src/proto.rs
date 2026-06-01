//! `LuaProto` — compiled function prototype. Mirrors C-Lua's `Proto` struct
//! but uses Rust idioms (Vec instead of pointer+size pairs).

use crate::closure::LuaLClosure;
use crate::gc::GcRef;
use crate::opcode::Instruction;
use crate::string::LuaString;
use crate::value::LuaValue;
use core::cell::RefCell;

#[derive(Debug)]
pub struct LuaProto {
    pub numparams: u8,
    pub is_vararg: bool,
    pub maxstacksize: u8,
    pub upvalues: Vec<UpvalDesc>,
    pub k: Vec<LuaValue>,
    pub code: Vec<Instruction>,
    pub p: Vec<GcRef<LuaProto>>,
    pub lineinfo: Vec<i8>,
    pub abslineinfo: Vec<AbsLineInfo>,
    pub locvars: Vec<LocalVar>,
    pub linedefined: i32,
    pub lastlinedefined: i32,
    pub source: Option<GcRef<LuaString>>,
    /// Last closure instantiated from this proto, reused by `OP_CLOSURE` when a
    /// new instantiation would capture the identical upvalues. Mirrors C-Lua's
    /// `Proto.cache` (5.2/5.3 only — added in 5.2, removed in 5.4), which is why
    /// loop-built closures with shared upvalues compare `==` on those versions.
    /// Populated only under 5.2/5.3 in `push_closure`; `None` otherwise. Traced
    /// (so it cannot dangle); unlike C's GC-cleared weak cache this pins the one
    /// cached closure to the proto's lifetime, which is bounded and safe.
    pub cache: RefCell<Option<GcRef<LuaLClosure>>>,
    /// Lua 5.5 named varargs (`function f(...t)`): the register holding the
    /// packed vararg table `t`. When set, `...` unpacks live from that table
    /// (count = its `n` field) rather than the frame's extra-arg slots, so
    /// mutating `t` is observable through a later `...` (shared storage). `None`
    /// for ordinary `...` and all pre-5.5 functions. Mirrors upstream's
    /// `needvatab` proto flag + the vararg-table register.
    pub vararg_table_reg: Option<u8>,
}

impl LuaProto {
    pub fn placeholder() -> Self {
        LuaProto {
            numparams: 0,
            is_vararg: false,
            maxstacksize: 2,
            upvalues: Vec::new(),
            k: Vec::new(),
            code: Vec::new(),
            p: Vec::new(),
            lineinfo: Vec::new(),
            abslineinfo: Vec::new(),
            locvars: Vec::new(),
            linedefined: 0,
            lastlinedefined: 0,
            source: None,
            cache: RefCell::new(None),
            vararg_table_reg: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpvalDesc {
    pub name: Option<GcRef<LuaString>>,
    pub instack: bool,
    pub idx: u8,
    pub kind: u8,
}

#[derive(Debug, Clone)]
pub struct LocalVar {
    pub varname: GcRef<LuaString>,
    pub startpc: i32,
    pub endpc: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct AbsLineInfo {
    pub pc: i32,
    pub line: i32,
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lobject.h (Proto struct)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Function prototype: bytecode, constants, line info, debug info,
//                  upvalue descriptors. Faithful layout of C's Proto struct using
//                  Vec<T> in place of T*+size pairs.
// ──────────────────────────────────────────────────────────────────────────────
