//! Arithmetic operations — used by lvm's arith dispatch.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ArithOp {
    Add  = 0,
    Sub  = 1,
    Mul  = 2,
    Mod  = 3,
    Pow  = 4,
    Div  = 5,
    Idiv = 6,
    Band = 7,
    Bor  = 8,
    Bxor = 9,
    Shl  = 10,
    Shr  = 11,
    Unm  = 12,
    Bnot = 13,
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lvm.c arith helpers (luaV_arith / luaV_lessthan / ...)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Arith opcode kind enum and operand-promotion helpers shared by
//                  lua-vm's OP_ADD / OP_SUB / ... handlers. Mirrors the C lvm.c
//                  intop+/tonumber+ patterns.
// ──────────────────────────────────────────────────────────────────────────────
