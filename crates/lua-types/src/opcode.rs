//! `Instruction` — a single packed bytecode word. C-Lua uses `u32`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Instruction(pub u32);

impl Instruction {
    pub const fn new(raw: u32) -> Self { Instruction(raw) }
    pub const fn raw(self) -> u32 { self.0 }
}
