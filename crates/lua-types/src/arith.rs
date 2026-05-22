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
