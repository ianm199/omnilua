//! Bytecode emitter and opcode definitions.
//!
//! Phase A scope. See PORT_STRATEGY.md §4.

pub mod codegen;
pub mod opcode_names;
pub mod opcodes;

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (composite crate; see opcode_names.rs for the first port)
//   target_crate:  lua-code
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         module aggregator
// ──────────────────────────────────────────────────────────────────────────
