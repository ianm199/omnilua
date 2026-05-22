//! Metamethod tags — matches C-Lua's TMS enum ordering.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TagMethod {
    Index    = 0,
    NewIndex = 1,
    Gc       = 2,
    Mode     = 3,
    Len      = 4,
    Eq       = 5,
    Add      = 6,
    Sub      = 7,
    Mul      = 8,
    Mod      = 9,
    Pow      = 10,
    Div      = 11,
    Idiv     = 12,
    Band     = 13,
    Bor      = 14,
    Bxor     = 15,
    Shl      = 16,
    Shr      = 17,
    Unm      = 18,
    Bnot     = 19,
    Lt       = 20,
    Le       = 21,
    Concat   = 22,
    Call     = 23,
    Close    = 24,
}

impl TagMethod {
    pub const N: usize = 25;
}
