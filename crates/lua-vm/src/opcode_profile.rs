use std::cell::RefCell;
use std::io::Write;
use std::sync::OnceLock;

use crate::vm::OpCode;

const OP_COUNT: usize = 85;

const OP_NAMES: [&str; OP_COUNT] = [
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
    "VARARGPREP", "EXTRAARG", "ERRNNIL", "VARARGPACK",
];

thread_local! {
    static COUNTS: RefCell<[u64; OP_COUNT]> = RefCell::new([0; OP_COUNT]);
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("LUA_RS_OPCODE_PROFILE").is_some())
}

#[inline(always)]
pub fn record(op: OpCode) {
    if !enabled() {
        return;
    }
    COUNTS.with(|counts| {
        counts.borrow_mut()[op as usize] += 1;
    });
}

pub fn snapshot() -> [u64; OP_COUNT] {
    COUNTS.with(|counts| *counts.borrow())
}

pub fn reset() {
    COUNTS.with(|counts| counts.borrow_mut().fill(0));
}

pub fn write_tsv(mut writer: impl Write) -> std::io::Result<()> {
    let counts = snapshot();
    let total: u64 = counts.iter().sum();
    let mut rows: Vec<(usize, u64)> = counts
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, count)| *count != 0)
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    writeln!(writer, "opcode\tcount\tpct")?;
    for (idx, count) in rows {
        let pct = if total == 0 {
            0.0
        } else {
            100.0 * count as f64 / total as f64
        };
        writeln!(writer, "{}\t{}\t{:.2}", OP_NAMES[idx], count, pct)?;
    }
    writeln!(writer, "TOTAL\t{}\t100.00", total)
}

pub fn write_report_from_env() -> std::io::Result<()> {
    let Some(path) = std::env::var_os("LUA_RS_OPCODE_PROFILE") else {
        return Ok(());
    };
    if path == "-" {
        return write_tsv(std::io::stderr().lock());
    }
    let file = std::fs::File::create(path)?;
    write_tsv(file)
}
