//! Load precompiled Lua chunks.
//!
//! The binary chunk format matches the reference C implementation
//! (`lundump.c`/`lundump.h`) byte-for-byte, so `string.dump` output and
//! precompiled chunks stay interchangeable with stock Lua.
//!
//! The public entry point is [`undump`], which reads a binary Lua chunk from
//! a [`ZIO`] stream and returns a Lua closure ready to call.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::state::LuaState;
use crate::zio::ZIO;
use lua_types::error::LuaError;
use lua_types::value::LuaValue;

use lua_types::closure::LuaLClosure;
use lua_types::gc::GcRef;
use lua_types::opcode::Instruction;
use lua_types::proto::{AbsLineInfo, LocalVar, LuaProto, UpvalDesc};
use lua_types::string::LuaString;
use lua_types::LuaVersion;

// ── Constants (from lundump.h) ─────────────────────────────────────────────

/// Six-byte data marker in the chunk header used to catch conversion errors.
const LUAC_DATA: &[u8] = b"\x19\x93\r\n\x1a\n";

/// Reference integer written in the header to detect integer endianness/size
/// mismatches.
const LUAC_INT: i64 = 0x5678;

/// Reference float written in the header to detect float format mismatches.
const LUAC_NUM: f64 = 370.5;

const LUAC_INT_55: i64 = -0x5678;

const LUAC_INST_55: u32 = 0x12345678;

const LUAC_NUM_55: f64 = -370.5;

// LUA_VERSION_NUM = 504 → ((5 * 16) + 4) = 0x54 = 84
/// One-byte version tag: upper nibble = major, lower nibble = minor.
const LUAC_VERSION_51: u8 = 0x51;
const LUAC_VERSION_52: u8 = 0x52;
const LUAC_VERSION_53: u8 = 0x53;
const LUAC_VERSION_54: u8 = 0x54;
const LUAC_VERSION_55: u8 = 0x55;

const LUAC_FORMAT: u8 = 0;

const LUA_SIGNATURE: &[u8] = b"\x1bLua";

const MAX_SHORT_LEN: usize = 40;

// ── Constant-pool type tags (from lobject.h makevariant) ───────────────────
//
// These are the byte values written by ldump.c into the constants array.
// makevariant(t, v) = t | (v << 4).
//
// The byte values used in the binary format are the raw tag integers from
// lobject.h, distinct from LuaValue's variant tags. Defined here as u8
// constants so the match in load_constants is self-documenting.

const TAG_NIL: u8 = 0x00;
const TAG_FALSE: u8 = 0x01;
const TAG_TRUE: u8 = 0x11;
const TAG_INT: u8 = 0x03;
const TAG_FLOAT: u8 = 0x13;
const TAG_SHORT_STR: u8 = 0x04;
const TAG_LONG_STR: u8 = 0x14;

// ── LoadState ──────────────────────────────────────────────────────────────

/// Loader state bundled for convenience: Lua state, input stream, and the
/// chunk name used in error messages.
///
/// Always stack-allocated inside [`undump`] and never escapes the call.
struct LoadState<'a> {
    state: &'a mut LuaState,
    z: &'a mut ZIO,
    /// 5.5 only: strings loaded so far, in order, for back-references
    /// (`lundump.c` `S->h`/`S->nstr`).
    saved_strings: Vec<GcRef<LuaString>>,
}

// ── Error helper ───────────────────────────────────────────────────────────

/// Build a syntax error for a malformed binary chunk.
///
/// Returns a `LuaError` for the caller to propagate with `?`, rather than
/// throwing via `longjmp` as the C reference does.
fn load_error(_s: &LoadState<'_>, why: &'static str) -> LuaError {
    LuaError::syntax(format_args!("bad binary format ({})", why))
}

// ── Low-level I/O ──────────────────────────────────────────────────────────

/// Read exactly `buf.len()` bytes from the stream into `buf`.
///
/// `ZIO::read` returns the number of bytes NOT read (0 = success).
fn load_block(s: &mut LoadState<'_>, buf: &mut [u8]) -> Result<(), LuaError> {
    if s.z.read(s.state, buf)? != 0 {
        return Err(load_error(s, "truncated chunk"));
    }
    Ok(())
}

/// Read a single byte from the stream.
fn load_byte(s: &mut LoadState<'_>) -> Result<u8, LuaError> {
    let b = s.z.getc(s.state)?;
    if b == crate::zio::EOZ {
        return Err(load_error(s, "truncated chunk"));
    }
    Ok(b as u8)
}

/// Read a variable-length unsigned integer (7 bits per byte, big-endian,
/// MSB-first continuation flag).
///
/// The encoding terminates when a byte with the high bit set is seen (the
/// *last* byte has bit 7 = 1) — the opposite of the more common LEB128, where
/// the continuation bit means "more follows".
fn load_unsigned(s: &mut LoadState<'_>, limit: usize) -> Result<usize, LuaError> {
    let mut x: usize = 0;
    let limit = limit >> 7;
    loop {
        let b = load_byte(s)? as usize;
        if x >= limit {
            return Err(load_error(s, "integer overflow"));
        }
        x = (x << 7) | (b & 0x7f);
        if (b & 0x80) != 0 {
            break;
        }
    }
    Ok(x)
}

/// Read a `size_t`-sized unsigned value.
fn load_size(s: &mut LoadState<'_>) -> Result<usize, LuaError> {
    load_unsigned(s, usize::MAX)
}

/// Read a signed `int`-sized value.
fn load_int(s: &mut LoadState<'_>) -> Result<i32, LuaError> {
    let v = load_unsigned(s, i32::MAX as usize)?;
    Ok(v as i32)
}

/// Read a `lua_Number` (f64) as eight raw native-endian bytes.
///
/// The binary format is host-endian for these fields; the header check
/// verifies endianness compatibility via the `LUAC_INT` and `LUAC_NUM`
/// sentinels.
fn load_number(s: &mut LoadState<'_>) -> Result<f64, LuaError> {
    let mut buf = [0u8; 8];
    load_block(s, &mut buf)?;
    Ok(f64::from_ne_bytes(buf))
}

/// Read a `lua_Integer` (i64) as eight raw native-endian bytes. Same
/// endianness reasoning as [`load_number`].
fn load_integer(s: &mut LoadState<'_>) -> Result<i64, LuaError> {
    let mut buf = [0u8; 8];
    load_block(s, &mut buf)?;
    Ok(i64::from_ne_bytes(buf))
}

fn load_raw_i32(s: &mut LoadState<'_>) -> Result<i32, LuaError> {
    let mut buf = [0u8; 4];
    load_block(s, &mut buf)?;
    Ok(i32::from_ne_bytes(buf))
}

fn load_raw_u32(s: &mut LoadState<'_>) -> Result<u32, LuaError> {
    let mut buf = [0u8; 4];
    load_block(s, &mut buf)?;
    Ok(u32::from_ne_bytes(buf))
}

// ── String loading ─────────────────────────────────────────────────────────

/// Load a nullable string.  Returns `None` if the stored size is zero.
///
/// The Lua binary format stores `actual_length + 1` so that size=0 is the
/// null-string sentinel. After reading `raw_size`, the actual byte count is
/// `raw_size - 1`.
///
/// Long strings are interned through the same `intern_str` path as short
/// strings; C creates long strings directly via `luaS_createlngstrobj`
/// without interning them.
///
/// The `_proto` parameter corresponds to C's `Proto *p`, used there only for
/// the `luaC_objbarrier(L, p, ts)` write barrier. That barrier is not invoked
/// here.
fn load_string_n(
    s: &mut LoadState<'_>,
    _proto: &LuaProto,
) -> Result<Option<GcRef<LuaString>>, LuaError> {
    let v55 = s.state.global().lua_version == LuaVersion::V55;
    let raw_size = load_size(s)?;
    if raw_size == 0 {
        if !v55 {
            return Ok(None);
        }
        let idx = load_size(s)?;
        if idx == 0 {
            return Ok(None);
        }
        return match s.saved_strings.get(idx - 1) {
            Some(ts) => Ok(Some(ts.clone())),
            None => Err(load_error(s, "invalid string index")),
        };
    }
    let size = raw_size - 1;

    // Read the raw bytes regardless of short/long distinction.
    let mut buf = vec![0u8; size];

    if size <= MAX_SHORT_LEN {
        load_block(s, &mut buf)?;
    } else {
        load_block(s, &mut buf)?;
    }

    let ts = s.state.intern_str(&buf)?;
    if v55 {
        s.saved_strings.push(ts.clone());
    }

    Ok(Some(ts))
}

/// Load a non-nullable string; error if the stream encodes a null string.
fn load_string(s: &mut LoadState<'_>, proto: &LuaProto) -> Result<GcRef<LuaString>, LuaError> {
    match load_string_n(s, proto)? {
        Some(ts) => Ok(ts),
        None => Err(load_error(s, "bad format for constant string")),
    }
}

// ── Proto-field loaders ────────────────────────────────────────────────────

/// Load the bytecode instruction array into a prototype.
///
/// Reads `n` raw 4-byte words in native-endian order, consistent with how
/// [`load_number`] and [`load_integer`] work.
fn load_code(s: &mut LoadState<'_>, f: &mut LuaProto) -> Result<(), LuaError> {
    let n = load_int(s)? as usize;
    let mut code = Vec::with_capacity(n);
    for _ in 0..n {
        let mut buf = [0u8; 4];
        load_block(s, &mut buf)?;
        code.push(Instruction(u32::from_ne_bytes(buf)));
    }
    f.code = code;
    Ok(())
}

/// Load the constant pool into a prototype.
///
/// Reads the tag byte for each constant, then its payload if any.
fn load_constants(s: &mut LoadState<'_>, f: &mut LuaProto) -> Result<(), LuaError> {
    let n = load_int(s)? as usize;
    let mut k = Vec::with_capacity(n);

    for _ in 0..n {
        let t = load_byte(s)?;
        let val = match t {
            TAG_NIL => LuaValue::Nil,
            TAG_FALSE => LuaValue::Bool(false),
            TAG_TRUE => LuaValue::Bool(true),
            TAG_FLOAT => LuaValue::Float(load_number(s)?),
            TAG_INT => LuaValue::Int(load_integer(s)?),

            TAG_SHORT_STR | TAG_LONG_STR => {
                let ts = load_string(s, f)?;
                LuaValue::Str(ts)
            }

            _ => {
                debug_assert!(false, "unknown constant type tag {:#04x}", t);
                LuaValue::Nil
            }
        };
        k.push(val);
    }

    f.k = k;
    Ok(())
}

/// Load nested function prototypes into a prototype.
///
/// C creates the proto first, as a GC anchor, then fills it. Here a default
/// `LuaProto` is built, filled, then wrapped in a `GcRef`.
fn load_protos(s: &mut LoadState<'_>, f: &mut LuaProto) -> Result<(), LuaError> {
    let n = load_int(s)? as usize;
    let mut protos = Vec::with_capacity(n);

    for _ in 0..n {
        let mut sub = LuaProto::placeholder();

        // Pass parent source as fallback.
        let parent_source = f.source.clone();
        load_function(s, &mut sub, parent_source)?;

        // A `LuaProto` is populated field-by-field through a `&mut` local and
        // then wrapped, because `GcRef` exposes only `Deref` (no `DerefMut`):
        // the canonical `state.new_proto()` would hand back a placeholder that
        // cannot be filled in place. The direct `GcRef::new` path is kept for
        // that reason, but `mark_gc_check_needed` is invoked here so a
        // precompiled-chunk load registers its allocations with the collector
        // exactly as `state.new_proto()` would (issue #276).
        s.state.mark_gc_check_needed();
        let sub_ref = GcRef::new(sub);
        sub_ref.account_buffer(sub_ref.buffer_bytes() as isize);
        protos.push(sub_ref);
    }

    f.p = protos;
    Ok(())
}

/// Load upvalue descriptors into a prototype.
///
/// C fills upvalue names first (`NULL`) for GC safety, then names are
/// attached separately. Here `UpvalDesc` values are built with `name: None`
/// and filled in later by [`load_debug`], which is why `UpvalDesc.name` is
/// `Option<GcRef<LuaString>>` rather than a bare `GcRef<LuaString>`.
fn load_upvalues(s: &mut LoadState<'_>, f: &mut LuaProto) -> Result<(), LuaError> {
    let n = load_int(s)? as usize;

    let mut upvalues = Vec::with_capacity(n);
    for _ in 0..n {
        let instack_raw = load_byte(s)?;
        let idx = load_byte(s)?;
        let kind = load_byte(s)?;

        upvalues.push(UpvalDesc {
            name: None, // filled by load_debug
            instack: instack_raw != 0,
            idx,
            kind,
        });
    }

    f.upvalues = upvalues;
    Ok(())
}

/// Load debug information into a prototype.
///
/// `lineinfo` is `ls_byte` (a signed byte) in C; each byte is read as `u8`
/// then cast to `i8`, which is safe since the two share the same in-memory
/// representation. `LocalVar.varname` and `UpvalDesc.name` are both
/// `Option<GcRef<LuaString>>` here because `loadStringN` can return `None`;
/// see also the note on [`load_upvalues`].
fn load_debug(s: &mut LoadState<'_>, f: &mut LuaProto) -> Result<(), LuaError> {
    let n = load_int(s)? as usize;
    let mut lineinfo = vec![0i8; n];
    for item in lineinfo.iter_mut() {
        *item = load_byte(s)? as i8;
    }
    f.lineinfo = lineinfo;

    let n = load_int(s)? as usize;
    let mut abslineinfo = Vec::with_capacity(n);
    for _ in 0..n {
        abslineinfo.push(AbsLineInfo {
            pc: load_int(s)?,
            line: load_int(s)?,
        });
    }
    f.abslineinfo = abslineinfo;

    let n = load_int(s)? as usize;

    let mut locvars = Vec::with_capacity(n);
    for _ in 0..n {
        let varname = load_string_n(s, f)?;
        let startpc = load_int(s)?;
        let endpc = load_int(s)?;
        let varname = match varname {
            Some(v) => v,
            None => s.state.new_string(b"")?,
        };
        locvars.push(LocalVar {
            varname,
            startpc,
            endpc,
        });
    }
    f.locvars = locvars;

    // If n == 0 there is no upvalue name info (stripped).
    let has_names = load_int(s)?;
    if has_names != 0 {
        let n_upvals = f.upvalues.len();
        for i in 0..n_upvals {
            let name = load_string_n(s, f)?;
            f.upvalues[i].name = name;
        }
    }

    Ok(())
}

// ── Function loader ────────────────────────────────────────────────────────

/// Load a complete function prototype from the stream.
///
/// `psource` is `None` at the top level; a nested prototype with no source of
/// its own inherits the parent's, expressed here by falling back to
/// `psource` when `loadStringN` returns `None`.
fn load_function(
    s: &mut LoadState<'_>,
    f: &mut LuaProto,
    psource: Option<GcRef<LuaString>>,
) -> Result<(), LuaError> {
    let source = load_string_n(s, f)?;
    f.source = source.or(psource);

    f.linedefined = load_int(s)?;
    f.lastlinedefined = load_int(s)?;
    f.numparams = load_byte(s)?;
    f.is_vararg = load_byte(s)? != 0;
    f.maxstacksize = load_byte(s)?;
    load_code(s, f)?;
    reconstruct_vararg_table_reg(f);
    load_constants(s, f)?;
    load_upvalues(s, f)?;
    load_protos(s, f)?;
    load_debug(s, f)?;

    Ok(())
}

/// Recover `LuaProto.vararg_table_reg` from the loaded bytecode instead of from
/// the wire format, so a precompiled chunk keeps Lua 5.5 named-vararg aliasing
/// (`function f(...t)`) without lua-rs's `string.dump` output diverging from
/// C's bytecode layout (which the structural oracle compares).
///
/// A named-vararg function emits exactly one `OP_VARARGPACK` (opcode 84) at
/// entry; its A operand is the register holding the shared vararg table. Its
/// k bit records whether the table must be materialized.
fn reconstruct_vararg_table_reg(f: &mut LuaProto) {
    const OP_VARARGPACK: u32 = 84;
    const OPCODE_MASK: u32 = 0x7F;
    const POS_K: u32 = 15;
    if let Some((reg, needed)) = f.code.iter().find_map(|inst| {
        let raw = inst.raw();
        (raw & OPCODE_MASK == OP_VARARGPACK).then(|| {
            let reg = ((raw >> 7) & 0xFF) as u8;
            let needed = ((raw >> POS_K) & 1) != 0;
            (reg, needed)
        })
    }) {
        f.vararg_table_reg = Some(reg);
        f.vararg_table_needed = needed;
    }
}

// ── Header validation ──────────────────────────────────────────────────────

/// Verify that the next `expected.len()` bytes in the stream match `expected`.
fn check_literal(
    s: &mut LoadState<'_>,
    expected: &[u8],
    msg: &'static str,
) -> Result<(), LuaError> {
    let mut buf = vec![0u8; expected.len()];
    load_block(s, &mut buf)?;
    if buf != expected {
        return Err(load_error(s, msg));
    }
    Ok(())
}

/// Verify that the next byte in the stream equals `expected_size`. `tname` is
/// always a Rust type-name string literal (ASCII) from the call sites.
fn fcheck_size(
    s: &mut LoadState<'_>,
    expected_size: usize,
    tname: &'static str,
) -> Result<(), LuaError> {
    let b = load_byte(s)? as usize;
    if b != expected_size {
        return Err(LuaError::syntax(format_args!("{} size mismatch", tname)));
    }
    Ok(())
}

/// Validate the binary chunk header.
///
/// The three fixed-size checks below cover `Instruction` (4 bytes, u32),
/// `lua_Integer` (8 bytes, i64), and `lua_Number` (8 bytes, f64).
///
/// The first byte of `LUA_SIGNATURE` (`\x1b`) is already consumed by the
/// caller before `check_header` is invoked, so only bytes 1.. of the
/// signature (`"Lua"`) are checked here.
fn check_header(s: &mut LoadState<'_>) -> Result<(), LuaError> {
    // Skip LUA_SIGNATURE[0] (\x1b) — already consumed by the caller.
    check_literal(s, &LUA_SIGNATURE[1..], "not a binary chunk")?;

    let version = s.state.global().lua_version;
    let expected_version = match version {
        LuaVersion::V51 => LUAC_VERSION_51,
        LuaVersion::V52 => LUAC_VERSION_52,
        LuaVersion::V53 => LUAC_VERSION_53,
        LuaVersion::V55 => LUAC_VERSION_55,
        _ => LUAC_VERSION_54,
    };
    let ver = load_byte(s)?;
    if ver != expected_version {
        return Err(load_error(s, "version mismatch"));
    }

    let fmt = load_byte(s)?;
    if fmt != LUAC_FORMAT {
        return Err(load_error(s, "format mismatch"));
    }

    match version {
        LuaVersion::V51 => {
            check_legacy_sizes(s)?;
        }
        LuaVersion::V52 => {
            check_legacy_sizes(s)?;
            check_literal(s, LUAC_DATA, "corrupted chunk")?;
        }
        LuaVersion::V53 => {
            check_literal(s, LUAC_DATA, "corrupted chunk")?;
            fcheck_size(s, size_of::<i32>(), "int")?;
            fcheck_size(s, size_of::<usize>(), "size_t")?;
            fcheck_size(s, 4, "Instruction")?;
            fcheck_size(s, 8, "lua_Integer")?;
            fcheck_size(s, 8, "lua_Number")?;
            if load_integer(s)? != LUAC_INT {
                return Err(load_error(s, "integer format mismatch"));
            }
            if load_number(s)? != LUAC_NUM {
                return Err(load_error(s, "float format mismatch"));
            }
        }
        LuaVersion::V55 => {
            check_literal(s, LUAC_DATA, "corrupted chunk")?;
            fcheck_size(s, 4, "int")?;
            if load_raw_i32(s)? != LUAC_INT_55 as i32 {
                return Err(load_error(s, "int format mismatch"));
            }

            fcheck_size(s, 4, "instruction")?;
            if load_raw_u32(s)? != LUAC_INST_55 {
                return Err(load_error(s, "instruction format mismatch"));
            }

            fcheck_size(s, 8, "Lua integer")?;
            if load_integer(s)? != LUAC_INT_55 {
                return Err(load_error(s, "Lua integer format mismatch"));
            }

            fcheck_size(s, 8, "Lua number")?;
            if load_number(s)? != LUAC_NUM_55 {
                return Err(load_error(s, "Lua number format mismatch"));
            }
        }
        _ => {
            check_literal(s, LUAC_DATA, "corrupted chunk")?;
            fcheck_size(s, 4, "Instruction")?;

            fcheck_size(s, 8, "lua_Integer")?;

            fcheck_size(s, 8, "lua_Number")?;

            let int_check = load_integer(s)?;
            if int_check != LUAC_INT {
                return Err(load_error(s, "integer format mismatch"));
            }

            let num_check = load_number(s)?;
            if num_check != LUAC_NUM {
                return Err(load_error(s, "float format mismatch"));
            }
        }
    }

    Ok(())
}

/// Validate the 5.1/5.2 endianness + size + integral-flag block: endian = 1
/// (little), `sizeof(int)` = 4, `sizeof(size_t)`, `sizeof(Instruction)` = 4,
/// `sizeof(lua_Number)` = 8, integral = 0. These versions have no integer
/// subtype, so there is no `lua_Integer` size byte and no `LUAC_INT`/`LUAC_NUM`
/// sentinel.
fn check_legacy_sizes(s: &mut LoadState<'_>) -> Result<(), LuaError> {
    if load_byte(s)? != 1 {
        return Err(load_error(s, "endianness mismatch"));
    }
    fcheck_size(s, size_of::<i32>(), "int")?;
    fcheck_size(s, size_of::<usize>(), "size_t")?;
    fcheck_size(s, 4, "Instruction")?;
    fcheck_size(s, 8, "lua_Number")?;
    if load_byte(s)? != 0 {
        return Err(load_error(s, "number format mismatch"));
    }
    Ok(())
}

// ── Public entry point ─────────────────────────────────────────────────────

/// Load a precompiled Lua chunk and return the top-level Lua closure.
///
/// This is the Rust equivalent of `luaU_undump` — the single public function
/// exported by `lundump.c`.
///
/// # Parameters
/// - `state` — the Lua thread state.
/// - `z` — input stream positioned at the start of the binary chunk
///   (the first byte `\x1b` of `LUA_SIGNATURE` must still be present).
/// - `name` — chunk name for error messages.  Stripped per Lua convention:
///   - `@…` → filename (strip `@`)
///   - `=…` → literal name (strip `=`)
///   - starts with `\x1b` → `"binary string"`
///   - otherwise used as-is.
///
/// Unlike the C reference — which anchors the half-built closure on the stack
/// (`setclLvalue2s` + `luaD_inctop`) so a mid-load emergency collection cannot
/// sweep it — no stack anchoring is needed here: `protected_parser` stops the
/// collector for the entire load window, and the closure is returned by value
/// (`do_.rs::f_parser` pushes it onto the stack after undump returns, which is
/// the real anchor point). `luai_verifycode`, a no-op in the default C build,
/// has no equivalent call here.
pub(crate) fn undump(
    state: &mut LuaState,
    z: &mut ZIO,
    _name: &[u8],
) -> Result<GcRef<LuaLClosure>, LuaError> {
    let mut s = LoadState {
        state,
        z,
        saved_strings: Vec::new(),
    };

    check_header(&mut s)?;

    // Reads the number of upvalues for the top-level closure.
    let nupvalues = load_byte(&mut s)? as usize;

    let mut proto = LuaProto::placeholder();

    load_function(&mut s, &mut proto, None)?;

    // Build-then-wrap the top proto (see the note in `load_protos` on why the
    // direct `GcRef::new` path is kept and `mark_gc_check_needed` fired here).
    s.state.mark_gc_check_needed();
    let proto_ref = GcRef::new(proto);
    proto_ref.account_buffer(proto_ref.buffer_bytes() as isize);

    debug_assert_eq!(
        nupvalues,
        proto_ref.upvalues.len(),
        "upvalue count mismatch between closure header and prototype"
    );

    // Route the closure through the canonical constructor: it fires
    // `mark_gc_check_needed`, fills the `nupvalues` upvalue slots with fresh
    // closed nil upvalues (Rust's `Cell<GcRef<UpVal>>` slots are non-nullable,
    // so C's `luaF_newLclosure`-with-NULL-slots followed by a later
    // `luaF_initupvals` fill collapses into this one construction step), and
    // accounts the closure's owned buffer (issue #276).
    let cl_ref = s.state.new_lclosure(proto_ref, nupvalues);

    Ok(cl_ref)
}
