//! `Instruction` — a single packed bytecode word. C-Lua uses `u32`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Instruction(pub u32);

impl Instruction {
    pub const fn new(raw: u32) -> Self { Instruction(raw) }
    pub const fn raw(self) -> u32 { self.0 }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lopcodes.h, src/lopcodes.c
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         OpCode enum + Instruction word layout. Mirrors C's opcode numbering and
//                  the iABC/iABx/iAsBx/iAx/isJ encoding macros.
// ──────────────────────────────────────────────────────────────────────────────
