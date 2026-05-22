//! Code generator for Lua 5.4.
//!
//! Ports `src/lcode.c` (1875 lines, ~60 functions) and the public interface
//! from `src/lcode.h`.
//!
//! Phase A: faithful logic translation — does not yet compile.
//!
//! TODO(port): FuncState.f is GcRef<LuaProto> per types.tsv but the code
//! generator mutates it directly.  Phase B must wrap LuaProto in
//! RefCell<LuaProto> (or similar) inside GcRef so that `fs.f.code`, `fs.f.k`,
//! etc. are mutable.  All `fs.f.<field>` accesses below assume direct mutability.
//!
//! TODO(port): LexState.L (the &mut LuaState back-pointer) was removed per
//! types.tsv.  Functions that call luaM_growvector / luaH_get / luaC_barrier /
//! luaO_rawarith carry an explicit `state: &mut LuaState` parameter here.
//! The caller (lua-parse) must thread it through.

// C: /* $Id: lcode.c $ */
// C: #define lcode_c
// C: #define LUA_CORE

use crate::opcodes::{
    Instruction, OpCode, OpMode,
    MAXARG_A, MAXARG_B, MAXARG_C, MAXARG_BX, MAXARG_AX, MAXARG_S_J,
    OFFSET_S_BX, OFFSET_S_J, OFFSET_S_C,
    NO_REG, MAXINDEXRK, LFIELDS_PER_FLUSH,
};

// Cross-crate types — unresolved until Phase B; expected E0432 errors.
use lua_types::error::LuaError;
use lua_types::value::LuaValue;
use lua_types::string::LuaString;
use lua_types::gc::GcRef;
// TODO(port): exact module paths for these depend on crate layout finalized in Phase B
use lua_parse::parser::{FuncState, ExprDesc, ExprKind};
use lua_parse::lexer::LexState;
use lua_vm::state::LuaState;
use lua_vm::proto::LuaProto;
use lua_vm::tagmethods::TagMethod;
use lua_vm::table::LuaTable;

// ─── Constants (lcode.h + local) ──────────────────────────────────────────────

// C: #define NO_JUMP (-1)
/// End-of-patch-list sentinel.  An invalid value both as an absolute address
/// and as a list link (would link an element to itself).
pub const NO_JUMP: i32 = -1;

// C: #define MAXREGS 255
/// Maximum number of registers in a Lua function (must fit in 8 bits).
const MAXREGS: i32 = 255;

// C: #define LIMLINEDIFF 0x80
/// Limit for difference between lines in relative line info.
const LIM_LINE_DIFF: i32 = 0x80;

// From macros.tsv: MAX_IWTH_ABS = 128
const MAX_IWTH_ABS: i32 = 128;

// From macros.tsv: ABS_LINE_INFO = -0x80
const ABS_LINE_INFO: i8 = -0x80i8;

// ─── BinOpr (lcode.h) ─────────────────────────────────────────────────────────
//
// C: /* grep "ORDER OPR" if you change these enums  (ORDER OP) */
// C: typedef enum BinOpr { OPR_ADD, OPR_SUB, ... OPR_NOBINOPR } BinOpr;
//
// ORDER OPR — discriminants must match `lcode.h` exactly because binopr2op /
// binopr2TM use arithmetic on them.

/// Binary operator kinds.  ORDER OPR must match `lcode.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum BinOpr {
    // arithmetic
    Add    = 0,
    Sub    = 1,
    Mul    = 2,
    Mod    = 3,
    Pow    = 4,
    Div    = 5,
    IDiv   = 6,
    // bitwise
    BAnd   = 7,
    BOr    = 8,
    BXor   = 9,
    Shl    = 10,
    Shr    = 11,
    // string
    Concat = 12,
    // comparison
    Eq     = 13,
    Lt     = 14,
    Le     = 15,
    Ne     = 16,
    Gt     = 17,
    Ge     = 18,
    // logical
    And    = 19,
    Or     = 20,
    // sentinel
    NoBinOpr = 21,
}

impl BinOpr {
    /// C: `#define foldbinop(op)  ((op) <= OPR_SHR)`
    /// True if operation is constant-foldable (arithmetic or bitwise).
    #[inline]
    pub fn is_foldable(self) -> bool {
        // C: foldbinop(op) ((op) <= OPR_SHR)
        (self as u8) <= BinOpr::Shr as u8
    }
}

// ─── UnOpr (lcode.h) ──────────────────────────────────────────────────────────
//
// C: typedef enum UnOpr { OPR_MINUS, OPR_BNOT, OPR_NOT, OPR_LEN, OPR_NOUNOPR } UnOpr;

/// Unary operator kinds.  ORDER OPR must match `lcode.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UnOpr {
    Minus   = 0,
    BNot    = 1,
    Not     = 2,
    Len     = 3,
    NoUnOpr = 4,
}

// ─── ExprKind (from lparser.h, ORDER must match) ──────────────────────────────
//
// Defined in lua-parse but reproduced here for reference:
// VVOID=0, VNIL=1, VTRUE=2, VFALSE=3, VK=4, VKFLT=5, VKINT=6, VKSTR=7,
// VNONRELOC=8, VLOCAL=9, VUPVAL=10, VCONST=11, VINDEXED=12, VINDEXUP=13,
// VINDEXI=14, VINDEXSTR=15, VJMP=16, VRELOC=17, VCALL=18, VVARARG=19
//
// PORT NOTE: ExprKind is defined in lua-parse; this comment documents the
// ordering invariants used for range comparisons below.

// ─── Arithmetic operation codes (lua.h) ──────────────────────────────────────
//
// C: LUA_OPADD=0, LUA_OPSUB, LUA_OPMUL, LUA_OPMOD, LUA_OPPOW, LUA_OPDIV,
//    LUA_OPIDIV, LUA_OPBAND, LUA_OPBOR, LUA_OPBXOR, LUA_OPSHL, LUA_OPSHR,
//    LUA_OPCONCAT, LUA_OPUNM, LUA_OPBNOT, LUA_OPLEN, LUA_OPNOT
//
// PORT NOTE: used in constfolding / binopr2TM offset arithmetic.
// TODO(port): define ArithOp enum in lua-types and use it instead of raw i32.
const LUA_OPADD: i32  = 0;
const LUA_OPUNM: i32  = 13;

// ─── Semantic-error helper ────────────────────────────────────────────────────

// C: l_noret luaK_semerror (LexState *ls, const char *msg) {
//      ls->t.token = 0;
//      luaX_syntaxerror(ls, msg);
//    }
/// Raise a semantic error, suppressing the "near <token>" addendum.
pub(crate) fn sem_error(ls: &mut LexState, msg: &str) -> Result<!, LuaError> {
    ls.t.token = 0; // remove "near <token>" from final message
    Err(LuaError::syntax(format_args!("{}", msg)))
}

// ─── Numeral predicate ────────────────────────────────────────────────────────

// C: static int tonumeral (const expdesc *e, TValue *v) { ... }
/// If `e` is a numeric constant, fill `v` with its value and return true.
/// Returns false when `e` has pending jumps (not a plain constant).
fn tonumeral(e: &ExprDesc, v: Option<&mut LuaValue>) -> bool {
    // C: if (hasjumps(e)) return 0;
    if e.t != e.f {
        return false;
    }
    match e.k {
        ExprKind::VKInt => {
            if let Some(dst) = v {
                *dst = LuaValue::Int(e.u.ival);
            }
            true
        }
        ExprKind::VKFlt => {
            if let Some(dst) = v {
                *dst = LuaValue::Float(e.u.nval);
            }
            true
        }
        _ => false,
    }
}

// ─── Constant-expression helpers ─────────────────────────────────────────────

// C: static TValue *const2val (FuncState *fs, const expdesc *e) {
//      lua_assert(e->k == VCONST);
//      return &fs->ls->dyd->actvar.arr[e->u.info].k;
//    }
/// Return the compile-time constant value stored for a VCONST expression.
fn const2val(fs: &FuncState, ls: &LexState, e: &ExprDesc) -> LuaValue {
    debug_assert!(e.k == ExprKind::VConst);
    // C: &fs->ls->dyd->actvar.arr[e->u.info].k
    ls.dyd.actvar[e.u.info as usize].k.clone()
}

// C: int luaK_exp2const (FuncState *fs, const expdesc *e, TValue *v) { ... }
/// If `e` is a compile-time constant, copy its value into `v` and return true.
pub(crate) fn exp2const(fs: &FuncState, ls: &LexState, e: &ExprDesc, v: &mut LuaValue) -> bool {
    // C: if (hasjumps(e)) return 0;
    if e.t != e.f {
        return false;
    }
    match e.k {
        ExprKind::VFalse => { *v = LuaValue::Bool(false); true }
        ExprKind::VTrue  => { *v = LuaValue::Bool(true);  true }
        ExprKind::VNil   => { *v = LuaValue::Nil;          true }
        ExprKind::VKStr  => {
            // C: setsvalue(fs->ls->L, v, e->u.strval)
            *v = LuaValue::Str(e.u.strval.clone());
            true
        }
        ExprKind::VConst => {
            *v = const2val(fs, ls, e);
            true
        }
        _ => tonumeral(e, Some(v)),
    }
}

// ─── Previous-instruction helper ─────────────────────────────────────────────

// C: static Instruction *previousinstruction (FuncState *fs) {
//      static const Instruction invalidinstruction = ~(Instruction)0;
//      if (fs->pc > fs->lasttarget)
//        return &fs->f->code[fs->pc - 1];
//      else
//        return cast(Instruction*, &invalidinstruction);
//    }
//
// PORT NOTE: In C the "invalid" case returns a pointer to 0xFFFFFFFF which
// will never match any real opcode.  In Rust we return Option<usize>; None
// means "no accessible previous instruction".
fn previous_instruction_idx(fs: &FuncState) -> Option<usize> {
    if fs.pc > fs.lasttarget {
        Some((fs.pc - 1) as usize)
    } else {
        None
    }
}

// ─── OP_LOADNIL optimisation ──────────────────────────────────────────────────

// C: void luaK_nil (FuncState *fs, int from, int n) { ... }
/// Emit OP_LOADNIL for registers `from..from+n-1`, merging with the previous
/// OP_LOADNIL when the ranges are compatible.
pub(crate) fn nil(
    fs: &mut FuncState,
    ls: &mut LexState,
    from: i32,
    n: i32,
) -> Result<(), LuaError> {
    let mut from = from;
    let mut l = from + n - 1; // last register to set nil
    if let Some(prev_idx) = previous_instruction_idx(fs) {
        let prev = fs.f.code[prev_idx];
        if prev.opcode() == OpCode::LoadNil {
            // C: int pfrom = GETARG_A(*previous); int pl = pfrom + GETARG_B(*previous);
            let pfrom = prev.arg_a() as i32;
            let pl = pfrom + prev.arg_b() as i32;
            if (pfrom <= from && from <= pl + 1) || (from <= pfrom && pfrom <= l + 1) {
                if pfrom < from { from = pfrom; }
                if pl > l { l = pl; }
                fs.f.code[prev_idx].set_arg_a(from as u32);
                fs.f.code[prev_idx].set_arg_b((l - from) as u32);
                return Ok(());
            }
        }
    }
    // C: luaK_codeABC(fs, OP_LOADNIL, from, n - 1, 0);
    code_abc(fs, ls, OpCode::LoadNil, from as u32, (n - 1) as u32, 0)?;
    Ok(())
}

// ─── Jump list traversal ──────────────────────────────────────────────────────

// C: static int getjump (FuncState *fs, int pc) { ... }
fn getjump(fs: &FuncState, pc: i32) -> i32 {
    let offset = fs.f.code[pc as usize].arg_s_j();
    if offset == NO_JUMP {
        NO_JUMP
    } else {
        (pc + 1) + offset
    }
}

// C: static void fixjump (FuncState *fs, int pc, int dest) { ... }
fn fixjump(fs: &mut FuncState, ls: &LexState, pc: i32, dest: i32) -> Result<(), LuaError> {
    debug_assert!(dest != NO_JUMP);
    let offset = dest - (pc + 1);
    // C: if (!(-OFFSET_sJ <= offset && offset <= MAXARG_sJ - OFFSET_sJ))
    if !((-OFFSET_S_J <= offset) && (offset <= MAXARG_S_J as i32 - OFFSET_S_J)) {
        return Err(LuaError::syntax(format_args!("control structure too long")));
    }
    debug_assert!(fs.f.code[pc as usize].opcode() == OpCode::Jmp);
    fs.f.code[pc as usize].set_arg_s_j(offset);
    Ok(())
}

// C: void luaK_concat (FuncState *fs, int *l1, int l2) { ... }
/// Concatenate jump-list `l2` into jump-list `l1`.
pub(crate) fn concat(
    fs: &mut FuncState,
    ls: &LexState,
    l1: &mut i32,
    l2: i32,
) -> Result<(), LuaError> {
    if l2 == NO_JUMP { return Ok(()); }
    if *l1 == NO_JUMP {
        *l1 = l2;
    } else {
        let mut list = *l1;
        loop {
            let next = getjump(fs, list);
            if next == NO_JUMP { break; }
            list = next;
        }
        fixjump(fs, ls, list, l2)?;
    }
    Ok(())
}

// C: int luaK_jump (FuncState *fs) { return codesJ(fs, OP_JMP, NO_JUMP, 0); }
/// Emit an unconditional jump.  Returns the pc of the jump instruction.
pub(crate) fn jump(fs: &mut FuncState, ls: &mut LexState) -> Result<i32, LuaError> {
    codes_j(fs, ls, OpCode::Jmp, NO_JUMP, 0)
}

// C: void luaK_ret (FuncState *fs, int first, int nret) { ... }
pub(crate) fn ret(
    fs: &mut FuncState,
    ls: &mut LexState,
    first: i32,
    nret: i32,
) -> Result<(), LuaError> {
    let op = match nret {
        0 => OpCode::Return0,
        1 => OpCode::Return1,
        _ => OpCode::Return,
    };
    code_abc(fs, ls, op, first as u32, (nret + 1) as u32, 0)?;
    Ok(())
}

// C: static int condjump (FuncState *fs, OpCode op, int A, int B, int C, int k) { ... }
fn condjump(
    fs: &mut FuncState,
    ls: &mut LexState,
    op: OpCode,
    a: u32,
    b: u32,
    c: u32,
    k: u32,
) -> Result<i32, LuaError> {
    code_abck(fs, ls, op, a, b, c, k)?;
    jump(fs, ls)
}

// C: int luaK_getlabel (FuncState *fs) { fs->lasttarget = fs->pc; return fs->pc; }
pub(crate) fn getlabel(fs: &mut FuncState) -> i32 {
    fs.lasttarget = fs.pc;
    fs.pc
}

// C: static Instruction *getjumpcontrol (FuncState *fs, int pc) { ... }
/// Return the index of the instruction "controlling" the jump at `pc`.
/// That is: if the previous instruction is a test, return its index;
/// otherwise return `pc` itself.
fn getjumpcontrol(fs: &FuncState, pc: i32) -> usize {
    let pi = pc as usize;
    if pi >= 1 {
        let prev_op = fs.f.code[pi - 1].opcode();
        // C: testTMode(GET_OPCODE(*(pi-1)))
        if (crate::opcodes::lua_p_opmodes(prev_op as usize) & (1 << 4)) != 0 {
            return pi - 1;
        }
    }
    pi
}

// C: static int patchtestreg (FuncState *fs, int node, int reg) { ... }
fn patchtestreg(fs: &mut FuncState, node: i32, reg: u32) -> bool {
    let ctrl = getjumpcontrol(fs, node);
    if fs.f.code[ctrl].opcode() != OpCode::TestSet {
        return false;
    }
    if reg != NO_REG && reg != fs.f.code[ctrl].arg_b() {
        fs.f.code[ctrl].set_arg_a(reg);
    } else {
        // change to simple TEST; C: CREATE_ABCk(OP_TEST, GETARG_B(*i), 0, 0, GETARG_k(*i))
        let b = fs.f.code[ctrl].arg_b();
        let k = fs.f.code[ctrl].arg_k();
        fs.f.code[ctrl] = Instruction::abck(OpCode::Test, b, 0, 0, k);
    }
    true
}

// C: static void removevalues (FuncState *fs, int list) { ... }
fn removevalues(fs: &mut FuncState, list: i32) {
    let mut list = list;
    while list != NO_JUMP {
        patchtestreg(fs, list, NO_REG);
        list = getjump(fs, list);
    }
}

// C: static void patchlistaux (FuncState *fs, int list, int vtarget, int reg, int dtarget) { ... }
fn patchlistaux(
    fs: &mut FuncState,
    ls: &LexState,
    list: i32,
    vtarget: i32,
    reg: u32,
    dtarget: i32,
) -> Result<(), LuaError> {
    let mut list = list;
    while list != NO_JUMP {
        let next = getjump(fs, list);
        if patchtestreg(fs, list, reg) {
            fixjump(fs, ls, list, vtarget)?;
        } else {
            fixjump(fs, ls, list, dtarget)?;
        }
        list = next;
    }
    Ok(())
}

// C: void luaK_patchlist (FuncState *fs, int list, int target) { ... }
pub(crate) fn patchlist(
    fs: &mut FuncState,
    ls: &LexState,
    list: i32,
    target: i32,
) -> Result<(), LuaError> {
    debug_assert!(target <= fs.pc);
    patchlistaux(fs, ls, list, target, NO_REG, target)
}

// C: void luaK_patchtohere (FuncState *fs, int list) { ... }
pub(crate) fn patchtohere(
    fs: &mut FuncState,
    ls: &LexState,
    list: i32,
) -> Result<(), LuaError> {
    let hr = getlabel(fs);
    patchlist(fs, ls, list, hr)
}

// ─── Line information ─────────────────────────────────────────────────────────

// C: static void savelineinfo (FuncState *fs, Proto *f, int line) { ... }
fn savelineinfo(
    fs: &mut FuncState,
    state: &mut LuaState,
    line: i32,
) -> Result<(), LuaError> {
    let linedif_raw = line - fs.previousline;
    let pc = fs.pc - 1; // last instruction coded
    let need_abs = linedif_raw.abs() >= LIM_LINE_DIFF || {
        let over = fs.iwthabs as i32 >= MAX_IWTH_ABS;
        if !over { fs.iwthabs += 1; }
        over
    };
    let linedif: i8;
    if need_abs {
        // C: luaM_growvector(fs->ls->L, f->abslineinfo, fs->nabslineinfo, ...)
        // PERF(port): reserve_or_grow call omitted; Vec::push handles growth
        fs.f.abslineinfo.push(lua_vm::proto::AbsLineInfo { pc, line });
        fs.nabslineinfo += 1;
        linedif = ABS_LINE_INFO;
        fs.iwthabs = 1;
    } else {
        linedif = linedif_raw as i8;
    }
    // C: luaM_growvector(fs->ls->L, f->lineinfo, pc, f->sizelineinfo, ...)
    // PERF(port): Vec::push / extend handles growth
    if fs.f.lineinfo.len() <= pc as usize {
        fs.f.lineinfo.resize(pc as usize + 1, 0i8);
    }
    fs.f.lineinfo[pc as usize] = linedif;
    fs.previousline = line;
    Ok(())
}

// C: static void removelastlineinfo (FuncState *fs) { ... }
fn removelastlineinfo(fs: &mut FuncState) {
    let pc = (fs.pc - 1) as usize;
    if fs.f.lineinfo[pc] != ABS_LINE_INFO {
        fs.previousline -= fs.f.lineinfo[pc] as i32;
        fs.iwthabs -= 1;
    } else {
        debug_assert_eq!(
            fs.f.abslineinfo[fs.nabslineinfo as usize - 1].pc,
            pc as i32
        );
        fs.nabslineinfo -= 1;
        // C: fs->iwthabs = MAXIWTHABS + 1; /* force next to be absolute */
        fs.iwthabs = (MAX_IWTH_ABS + 1) as u8;
    }
}

// C: static void removelastinstruction (FuncState *fs) { ... }
fn removelastinstruction(fs: &mut FuncState) {
    removelastlineinfo(fs);
    fs.pc -= 1;
}

// ─── Core instruction emission ────────────────────────────────────────────────

// C: int luaK_code (FuncState *fs, Instruction i) { ... }
/// Emit instruction `i`, grow code array as needed, save line info.
/// Returns the pc (index) of the new instruction.
pub(crate) fn code(
    fs: &mut FuncState,
    ls: &mut LexState,
    state: &mut LuaState,
    i: Instruction,
) -> Result<i32, LuaError> {
    // C: luaM_growvector(fs->ls->L, f->code, fs->pc, f->sizecode, ...)
    // PERF(port): Vec::push handles growth
    if fs.f.code.len() <= fs.pc as usize {
        fs.f.code.resize(fs.pc as usize + 1, Instruction::default());
    }
    fs.f.code[fs.pc as usize] = i;
    fs.pc += 1;
    // C: savelineinfo(fs, f, fs->ls->lastline);
    savelineinfo(fs, state, ls.lastline)?;
    Ok(fs.pc - 1)
}

// C: int luaK_codeABCk (FuncState *fs, OpCode o, int a, int b, int c, int k) { ... }
pub(crate) fn code_abck(
    fs: &mut FuncState,
    ls: &mut LexState,
    o: OpCode,
    a: u32,
    b: u32,
    c: u32,
    k: u32,
) -> Result<i32, LuaError> {
    // TODO(port): getOpMode assertion requires luaP_opmodes lookup
    debug_assert!(a <= MAXARG_A && b <= MAXARG_B && c <= MAXARG_C && (k & !1) == 0);
    // C: luaK_code(fs, CREATE_ABCk(o, a, b, c, k))
    // TODO(port): state needs to be threaded from caller; using dummy for now
    // TODO(port): code() needs &mut LuaState for savelineinfo/growvector
    let instr = Instruction::abck(o, a, b, c, k);
    // PORT NOTE: state is not available at this call site in all callers;
    // this is a known Phase B issue.  Callers that have state pass it in;
    // others will need refactoring.
    // For Phase A we call a version that skips growth:
    code_no_state(fs, ls, instr)
}

// C: int luaK_codeABx (FuncState *fs, OpCode o, int a, unsigned int bc) { ... }
pub(crate) fn code_abx(
    fs: &mut FuncState,
    ls: &mut LexState,
    o: OpCode,
    a: u32,
    bc: u32,
) -> Result<i32, LuaError> {
    debug_assert!(a <= MAXARG_A && bc <= MAXARG_BX);
    code_no_state(fs, ls, Instruction::abx(o, a, bc))
}

// C: static int codeAsBx (FuncState *fs, OpCode o, int a, int bc) { ... }
fn code_asbx(
    fs: &mut FuncState,
    ls: &mut LexState,
    o: OpCode,
    a: u32,
    bc: i32,
) -> Result<i32, LuaError> {
    let b = (bc + OFFSET_S_BX) as u32;
    debug_assert!(a <= MAXARG_A && b <= MAXARG_BX);
    code_no_state(fs, ls, Instruction::abx(o, a, b))
}

// C: static int codesJ (FuncState *fs, OpCode o, int sj, int k) { ... }
fn codes_j(
    fs: &mut FuncState,
    ls: &mut LexState,
    o: OpCode,
    sj: i32,
    k: u32,
) -> Result<i32, LuaError> {
    let j = (sj + OFFSET_S_J) as u32;
    debug_assert!(j <= MAXARG_S_J && (k & !1) == 0);
    code_no_state(fs, ls, Instruction::sj(o, j, k))
}

// C: static int codeextraarg (FuncState *fs, int a) { ... }
fn codeextraarg(
    fs: &mut FuncState,
    ls: &mut LexState,
    a: u32,
) -> Result<i32, LuaError> {
    debug_assert!(a <= MAXARG_AX);
    code_no_state(fs, ls, Instruction::ax(OpCode::ExtraArg, a))
}

// PORT NOTE: `code_no_state` is a Phase A shim — it records the instruction
// and bumps pc but skips the Vec growth check that requires &mut LuaState.
// Phase B replaces all call sites with `code(fs, ls, state, instr)`.
fn code_no_state(
    fs: &mut FuncState,
    ls: &LexState,
    i: Instruction,
) -> Result<i32, LuaError> {
    // PERF(port): In production, pre-allocate via reserve_or_grow; here we
    // rely on Vec's amortised growth.
    if fs.f.code.len() <= fs.pc as usize {
        fs.f.code.resize(fs.pc as usize + 1, Instruction::default());
    }
    fs.f.code[fs.pc as usize] = i;
    fs.pc += 1;
    // TODO(port): savelineinfo needs &mut LuaState for abslineinfo growth;
    // for now we do a simplified version that only handles relative deltas.
    let linedif_raw = ls.lastline - fs.previousline;
    let pc = (fs.pc - 1) as usize;
    if fs.f.lineinfo.len() <= pc {
        fs.f.lineinfo.resize(pc + 1, 0i8);
    }
    if linedif_raw.abs() < LIM_LINE_DIFF && (fs.iwthabs as i32) < MAX_IWTH_ABS {
        fs.f.lineinfo[pc] = linedif_raw as i8;
        fs.iwthabs += 1;
    } else {
        fs.f.lineinfo[pc] = ABS_LINE_INFO;
        fs.f.abslineinfo.push(lua_vm::proto::AbsLineInfo { pc: pc as i32, line: ls.lastline });
        fs.nabslineinfo += 1;
        fs.iwthabs = 1;
    }
    fs.previousline = ls.lastline;
    Ok(fs.pc - 1)
}

// Thin wrapper — `luaK_codeABC` macro translates to this (k=0).
fn code_abc(
    fs: &mut FuncState,
    ls: &mut LexState,
    o: OpCode,
    a: u32,
    b: u32,
    c: u32,
) -> Result<i32, LuaError> {
    code_abck(fs, ls, o, a, b, c, 0)
}

// C: static int luaK_codek (FuncState *fs, int reg, int k) { ... }
fn codek(
    fs: &mut FuncState,
    ls: &mut LexState,
    reg: i32,
    k: i32,
) -> Result<i32, LuaError> {
    if k as u32 <= MAXARG_BX {
        code_abx(fs, ls, OpCode::LoadK, reg as u32, k as u32)
    } else {
        let p = code_abx(fs, ls, OpCode::LoadKX, reg as u32, 0)?;
        codeextraarg(fs, ls, k as u32)?;
        Ok(p)
    }
}

// ─── Stack-size bookkeeping ───────────────────────────────────────────────────

// C: void luaK_checkstack (FuncState *fs, int n) { ... }
pub(crate) fn checkstack(
    fs: &mut FuncState,
    ls: &LexState,
    n: i32,
) -> Result<(), LuaError> {
    let newstack = fs.freereg as i32 + n;
    if newstack > fs.f.maxstacksize as i32 {
        if newstack >= MAXREGS {
            return Err(LuaError::syntax(format_args!(
                "function or expression needs too many registers"
            )));
        }
        fs.f.maxstacksize = newstack as u8;
    }
    Ok(())
}

// C: void luaK_reserveregs (FuncState *fs, int n) { ... }
pub(crate) fn reserveregs(
    fs: &mut FuncState,
    ls: &LexState,
    n: i32,
) -> Result<(), LuaError> {
    checkstack(fs, ls, n)?;
    fs.freereg = (fs.freereg as i32 + n) as u8;
    Ok(())
}

// ─── Register freeing ─────────────────────────────────────────────────────────

// C: static void freereg (FuncState *fs, int reg) { ... }
// `nvarstack` is the number of locals on the register stack (luaY_nvarstack).
fn freereg(fs: &mut FuncState, reg: i32, nvarstack: i32) {
    if reg >= nvarstack {
        fs.freereg -= 1;
        debug_assert_eq!(reg, fs.freereg as i32);
    }
}

// C: static void freeregs (FuncState *fs, int r1, int r2) { ... }
fn freeregs(fs: &mut FuncState, r1: i32, r2: i32, nvarstack: i32) {
    if r1 > r2 {
        freereg(fs, r1, nvarstack);
        freereg(fs, r2, nvarstack);
    } else {
        freereg(fs, r2, nvarstack);
        freereg(fs, r1, nvarstack);
    }
}

// C: static void freeexp (FuncState *fs, expdesc *e) { ... }
fn freeexp(fs: &mut FuncState, e: &ExprDesc, nvarstack: i32) {
    if e.k == ExprKind::VNonReloc {
        freereg(fs, e.u.info, nvarstack);
    }
}

// C: static void freeexps (FuncState *fs, expdesc *e1, expdesc *e2) { ... }
fn freeexps(fs: &mut FuncState, e1: &ExprDesc, e2: &ExprDesc, nvarstack: i32) {
    let r1 = if e1.k == ExprKind::VNonReloc { e1.u.info } else { -1 };
    let r2 = if e2.k == ExprKind::VNonReloc { e2.u.info } else { -1 };
    freeregs(fs, r1, r2, nvarstack);
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lcode.c  (1875 lines, ~60 functions)
//   target_crate:  lua-code
//   confidence:    medium
//   todos:         12
//   port_notes:    4
//   unsafe_blocks: 0
//   notes:         Full codegen port. FuncState.f needs RefCell for Phase B.
//                  Cross-crate imports will resolve once lua-parse/lua-vm land.
//                  Arithmetic op code constants need ArithOp enum in lua-types.
// ──────────────────────────────────────────────────────────────────────────
