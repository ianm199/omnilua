//! `LuaString` — Lua's byte-string (NOT UTF-8). PORT_STRATEGY §3.3.
//!
//! Phase A-C: a simple `Rc<[u8]>`-backed struct with a short/long flag.
//! Phase D may revisit for interning + content-hash equality.

#[derive(Debug, Clone)]
pub struct LuaString {
    bytes: std::rc::Rc<[u8]>,
    is_short: bool,
    hash: u32,
}

impl LuaString {
    pub fn from_bytes(b: Vec<u8>) -> Self {
        let is_short = b.len() <= 40;
        let hash = Self::hash_bytes(&b, 0);
        LuaString { bytes: b.into(), is_short, hash }
    }

    pub fn placeholder() -> Self { Self::from_bytes(Vec::new()) }

    pub fn as_bytes(&self) -> &[u8] { &self.bytes }
    pub fn len(&self) -> usize { self.bytes.len() }
    pub fn is_empty(&self) -> bool { self.bytes.is_empty() }
    pub fn is_short(&self) -> bool { self.is_short }
    pub fn is_long(&self) -> bool { !self.is_short }
    pub fn hash(&self) -> u32 { self.hash }

    pub fn is_reserved_word(&self) -> bool {
        // TODO(port): proper reserved-word check via lexer's token enum.
        false
    }

    pub fn hash_bytes(bytes: &[u8], seed: u32) -> u32 {
        // Stub WyHash. Real impl ports bun_wyhash. Stable for now so
        // intern-table equality works.
        let mut h: u32 = seed.wrapping_add(0x9e3779b9);
        for &b in bytes {
            h = h.wrapping_mul(31).wrapping_add(b as u32);
        }
        h
    }

    pub fn hash_long(&mut self) -> u32 { self.hash }
}

impl PartialEq for LuaString {
    fn eq(&self, other: &Self) -> bool { self.bytes == other.bytes }
}
impl Eq for LuaString {}
