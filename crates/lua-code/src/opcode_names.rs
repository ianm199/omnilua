//! Opcode name table for debug/disassembly output.
//!
//! Direct port of `src/lopnames.h` from Lua 5.4.7. Order must match the
//! `OpCode` enum (`src/lopcodes.h`); `ORDER OP` invariant.
//!
//! The C source is preserved inline as `
//! review.

//
// PORT NOTE: dropped the trailing NULL sentinel. Length is `OP_COUNT` known
// at compile time; Rust slice + bounds-check serves the role of the
// sentinel.

/// Total number of opcodes. Must equal `OpCode::Count as usize` once the
/// enum lands; trailer-required hook checks this constant exists.
pub const OP_COUNT: usize = 84;

/// Opcode names, indexed by `OpCode as usize`. ORDER OP — must match the
/// `OpCode` enum order in `lopcodes.h` exactly.
pub const OPNAMES: [&str; OP_COUNT] = [
    "MOVE", "LOADI", "LOADF", "LOADK", "LOADKX", "LOADFALSE", "LFALSESKIP",
    "LOADTRUE", "LOADNIL", "GETUPVAL", "SETUPVAL", "GETTABUP", "GETTABLE",
    "GETI", "GETFIELD", "SETTABUP", "SETTABLE", "SETI", "SETFIELD",
    "NEWTABLE", "SELF", "ADDI", "ADDK", "SUBK", "MULK", "MODK", "POWK",
    "DIVK", "IDIVK", "BANDK", "BORK", "BXORK", "SHRI", "SHLI", "ADD",
    "SUB", "MUL", "MOD", "POW", "DIV", "IDIV", "BAND", "BOR", "BXOR",
    "SHL", "SHR", "MMBIN", "MMBINI", "MMBINK", "UNM", "BNOT", "NOT",
    "LEN", "CONCAT", "CLOSE", "TBC", "JMP", "EQ", "LT", "LE", "EQK",
    "EQI", "LTI", "LEI", "GTI", "GEI", "TEST", "TESTSET", "CALL",
    "TAILCALL", "RETURN", "RETURN0", "RETURN1", "FORLOOP", "FORPREP",
    "TFORPREP", "TFORCALL", "TFORLOOP", "SETLIST", "CLOSURE", "VARARG",
    "VARARGPREP", "EXTRAARG", "ERRNNIL",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_count_matches_table() {
        assert_eq!(OPNAMES.len(), OP_COUNT);
    }

    #[test]
    fn first_and_last_opcodes() {
        assert_eq!(OPNAMES[0], "MOVE");
        assert_eq!(OPNAMES[OP_COUNT - 1], "ERRNNIL");
    }
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lopnames.h (103 lines, 1 static array)
//   target_crate:  lua-code
//   confidence:    high
//   todos:         0
//   port_notes:    1   (dropped NULL sentinel — Rust length is exact)
//   unsafe_blocks: 0
//   notes:         opcode name table only; OpCode enum lands separately
// ──────────────────────────────────────────────────────────────────────────
