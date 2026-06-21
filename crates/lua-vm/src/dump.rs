//! Pre-compiled Lua chunk serializer.
//!
//! Translates `reference/lua-5.4.7/src/ldump.c` (230 lines, 9 functions + 1 public entry point).
//! Writes a `LuaProto` to a byte sink in the standard Lua 5.4 bytecode format.

// TODO(port): Adjust import paths once crate boundaries stabilise in Phase B.
// The types below are expected to resolve as follows:
//   GcRef        — lua_types (or lua-gc Phase D)
//   LuaError     — lua_types
//   LuaProto     — lua-vm (this crate) or lua-types
//   LuaString    — lua-vm / lua-types
//   LuaValue     — lua_types
//   LuaState     — lua-vm (this crate)
#[allow(unused_imports)]
use crate::prelude::*;
use std::mem::size_of;

use crate::state::LuaState;
use lua_types::proto::LuaProto;
use lua_types::{GcRef, LuaError, LuaString, LuaValue, LuaVersion};

// ── Constants from lundump.h ─────────────────────────────────────────────────

// dumpLiteral expands to dumpBlock(D, s, sizeof(s) - sizeof(char)).
// sizeof("\x1bLua") = 5; minus 1 = 4 bytes, no NUL terminator.
// b"\x1bLua" is &[u8; 4] in Rust — no NUL — so direct use is correct.
const LUA_SIGNATURE: &[u8] = b"\x1bLua";

// With LUA_VERSION_NUM = 504 (macros.tsv):
//   (504 / 100) * 16 + 504 % 100 = 5 * 16 + 4 = 84 = 0x54
const LUA_VERSION_NUM_DUMP_54: i32 = 504;
const LUAC_VERSION_54: u8 =
    ((LUA_VERSION_NUM_DUMP_54 / 100) * 16 + LUA_VERSION_NUM_DUMP_54 % 100) as u8;
const LUAC_VERSION_55: u8 = 0x55;

const LUAC_FORMAT: u8 = 0;

// sizeof("\x19\x93\r\n\x1a\n") = 7; minus 1 = 6 bytes written.
// b"\x19\x93\r\n\x1a\n" is &[u8; 6].
const LUAC_DATA: &[u8] = b"\x19\x93\r\n\x1a\n";

const LUAC_INT: i64 = 0x5678;

const LUAC_NUM: f64 = 370.5;

const LUAC_INT_55: i64 = -0x5678;

const LUAC_INST_55: u32 = 0x12345678;

const LUAC_NUM_55: f64 = -370.5;

const LUAC_VERSION_51: u8 = 0x51;

const LUAC_VERSION_52: u8 = 0x52;

const LUAC_VERSION_53: u8 = 0x53;

/// Legacy (5.1/5.2/5.3) header byte: `sizeof(int)` = 4.
const C_INT_SIZE: u8 = size_of::<i32>() as u8;

/// Legacy (5.1/5.2/5.3) header byte: `sizeof(size_t)`, the build target's pointer width.
const C_SIZET_SIZE: u8 = size_of::<usize>() as u8;

/// Legacy (5.1/5.2) endianness flag: 1 = little-endian (the build target).
const LUAC_ENDIAN_LITTLE: u8 = 1;

/// Legacy (5.1/5.2) integral flag: 0 = `lua_Number` is floating-point.
const LUAC_INTEGRAL_FLOAT: u8 = 0;

const INSTRUCTION_SIZE: u8 = size_of::<u32>() as u8;

const LUA_INTEGER_SIZE: u8 = size_of::<i64>() as u8;

const LUA_NUMBER_SIZE: u8 = size_of::<f64>() as u8;

// ── DumpState ────────────────────────────────────────────────────────────────

/// Internal state threaded through every dump operation.
///
///
/// PORT NOTE: `lua_State *L` removed — it was used only for `lua_lock`/`lua_unlock`, which are
/// no-ops in the default Lua build and dropped here (macros.tsv). `void *data` is folded into
/// the writer closure. `int status` is replaced by `Result<(), LuaError>` propagated with `?`.
struct DumpState<'a> {
    /// Byte-sink callback. C original: `lua_Writer writer` + `void *data` (combined).
    /// lua_Writer type is TBD in types.tsv; for dump we use a bare byte-slice callback.
    writer: &'a mut dyn FnMut(&[u8]) -> Result<(), LuaError>,
    /// When true, strip all debug information from the output.
    strip: bool,
    version: LuaVersion,
}

impl<'a> DumpState<'a> {
    // ── Low-level write primitives ────────────────────────────────────────────

    /// Write raw bytes to the output stream.
    ///
    ///
    /// PORT NOTE: C accumulates errors in `D->status` and skips subsequent writes once
    /// non-zero; Rust returns `Result<(), LuaError>` and short-circuits via `?`.
    /// `lua_lock`/`lua_unlock` are no-ops in the default build and are dropped (macros.tsv).
    fn dump_block(&mut self, data: &[u8]) -> Result<(), LuaError> {
        if !data.is_empty() {
            (self.writer)(data)?;
        }
        Ok(())
    }

    /// Write one byte.
    ///
    /// C body: `lu_byte x = (lu_byte)y; dumpVar(D, x);`
    /// (`dumpVar(D,x)` expands to `dumpVector(D,&x,1)` expands to `dumpBlock(D,&x,sizeof(x))`)
    fn dump_byte(&mut self, y: u8) -> Result<(), LuaError> {
        self.dump_block(&[y])
    }

    /// Write a `size_t` using Lua's variable-length encoding.
    ///
    ///
    /// Encoding (big-endian 7-bit groups, **last** byte marked with MSB = 1):
    /// - Each byte holds 7 payload bits.
    /// - Bytes are written most-significant group first.
    /// - The final byte (least-significant group) has its MSB set as an end marker.
    ///
    /// This differs from standard LEB128, which marks the *continuation* bytes rather than
    /// the terminating byte.
    ///
    fn dump_size(&mut self, mut x: usize) -> Result<(), LuaError> {
        // DIBS = (usize::BITS + 6) / 7; on 64-bit = (64+6)/7 = 10.
        const DIBS: usize = (usize::BITS as usize + 6) / 7;
        let mut buff = [0u8; DIBS];
        let mut n: usize = 0;

        loop {
            n += 1;
            buff[DIBS - n] = (x & 0x7f) as u8; // fill buffer in reverse order
            x >>= 7;
            if x == 0 {
                break;
            }
        }

        // The byte at buff[DIBS-1] is the first byte placed (least-significant group).
        // Setting its MSB marks it as the terminal byte of the encoding.
        buff[DIBS - 1] |= 0x80;

        self.dump_block(&buff[DIBS - n..])
    }

    /// Write an `int` as a variable-length size.
    ///
    ///
    /// PORT NOTE: C implicitly casts `int` → `size_t`. All call sites pass non-negative values
    /// (line numbers, instruction counts, vector lengths); a debug assertion guards this.
    fn dump_int(&mut self, x: i32) -> Result<(), LuaError> {
        debug_assert!(
            x >= 0,
            "dump_int: negative value {} cast to usize would wrap",
            x
        );
        self.dump_size(x as usize)
    }

    /// Write a `lua_Number` (f64) in the platform's native byte order.
    ///
    ///
    /// `dumpVar(D,x)` expands to `dumpBlock(D, &x, sizeof(lua_Number))` — 8 bytes, native order.
    /// `to_ne_bytes()` replicates native-endian serialisation. The bytecode header's `LUAC_NUM`
    /// sentinel (370.5) lets `lundump` detect byte-order mismatches at load time.
    fn dump_number(&mut self, x: f64) -> Result<(), LuaError> {
        self.dump_block(&x.to_ne_bytes())
    }

    /// Write a `lua_Integer` (i64) in the platform's native byte order.
    ///
    fn dump_integer(&mut self, x: i64) -> Result<(), LuaError> {
        self.dump_block(&x.to_ne_bytes())
    }

    fn dump_raw_i32(&mut self, x: i32) -> Result<(), LuaError> {
        self.dump_block(&x.to_ne_bytes())
    }

    fn dump_raw_u32(&mut self, x: u32) -> Result<(), LuaError> {
        self.dump_block(&x.to_ne_bytes())
    }

    // ── Mid-level serialisers ─────────────────────────────────────────────────

    /// Write an interned or long string, or a null sentinel (encoded size = 0).
    ///
    ///
    /// Encoding: `dumpSize(len + 1)` followed by `len` raw bytes; size 0 means null/absent.
    /// `tsslen(s)` → `s.len()` and `getstr(s)` → `s.as_bytes()` (macros.tsv).
    fn dump_string(&mut self, s: Option<&GcRef<LuaString>>) -> Result<(), LuaError> {
        match s {
            None => self.dump_size(0),

            Some(s) => {
                let bytes = s.as_bytes(); // tsslen → .len(); getstr → .as_bytes()
                self.dump_size(bytes.len() + 1)?;
                self.dump_block(bytes)
            }
        }
    }

    /// Write the bytecode instruction array.
    ///
    ///
    /// PORT NOTE: `f->sizecode` is covered by `Vec::len()` (types.tsv).
    fn dump_code(&mut self, proto: &LuaProto) -> Result<(), LuaError> {
        self.dump_int(proto.code.len() as i32)?;

        // dumpVector writes n * sizeof(Instruction) = n * 4 bytes in native byte order.
        for instr in &proto.code {
            // TODO(port): `Instruction` is a u32 newtype (types.tsv). Accessing the inner u32
            // via `.0` assumes a tuple-struct layout. If the Instruction API differs (e.g.,
            // exposes `.raw()` or `u32::from(*instr)`), adjust accordingly in Phase B.
            self.dump_block(&instr.0.to_ne_bytes())?;
        }
        Ok(())
    }

    /// Write the constant pool.
    ///
    ///
    /// Each constant is written as: one tag byte (`ttypetag`), followed by the payload
    /// (float: 8 bytes; integer: 8 bytes; string: variable-length; nil/bool: nothing).
    ///
    /// PORT NOTE: `f->sizek` is covered by `Vec::len()` (types.tsv).
    fn dump_constants(&mut self, proto: &LuaProto) -> Result<(), LuaError> {
        let n = proto.k.len();
        self.dump_int(n as i32)?;

        for constant in &proto.k {
            // ttypetag(o) → o.full_type_tag() (macros.tsv)
            // Returns the C-side tag byte: bits 0-3 base type, bits 4-5 variant, bit 6 collectable.
            let tag = constant.full_type_tag();
            self.dump_byte(tag)?;

            match constant {
                LuaValue::Float(f) => {
                    // fltvalue(o) → o.as_float().expect("not float") or `if let` (macros.tsv)
                    self.dump_number(*f)?;
                }
                LuaValue::Int(i) => {
                    self.dump_integer(*i)?;
                }
                LuaValue::Str(s) => {
                    // tsvalue(o) → o.as_string().expect("not string") (macros.tsv)
                    self.dump_string(Some(s))?;
                }
                LuaValue::Nil | LuaValue::Bool(_) => {
                    // Only the tag byte is written; nil and booleans carry no additional payload.
                    // lua_assert → debug_assert! (macros.tsv)
                    debug_assert!(
                        matches!(constant, LuaValue::Nil | LuaValue::Bool(_)),
                        "dump_constants: default branch reached for unexpected variant"
                    );
                }
                _ => {
                    // TODO(port): LuaValue variant not valid as a constant-pool entry.
                    // In C the default branch asserts nil/false/true only. Any other variant
                    // here indicates a malformed proto; flag for Phase B investigation.
                    debug_assert!(
                        false,
                        "dump_constants: unexpected LuaValue variant in constant pool"
                    );
                }
            }
        }
        Ok(())
    }

    /// Write nested function prototypes (sub-functions defined inside `proto`).
    ///
    ///
    /// PORT NOTE: `f->sizep` is covered by `Vec::len()` (types.tsv).
    /// The parent's source string is passed down so that children with identical source
    /// origins can omit the redundant source name (see `dump_function`).
    fn dump_protos(&mut self, proto: &LuaProto) -> Result<(), LuaError> {
        let n = proto.p.len();
        self.dump_int(n as i32)?;

        for sub in &proto.p {
            // sub: &GcRef<LuaProto>; deref coercion (&GcRef<LuaProto> → &LuaProto) expected
            // when GcRef<T>: Deref<Target=T> (true for Rc<T> in Phase A).
            self.dump_function(sub, proto.source.as_ref())?;
        }
        Ok(())
    }

    /// Write upvalue descriptors (instack / idx / kind for each upvalue slot).
    ///
    ///
    /// PORT NOTE: `f->sizeupvalues` is covered by `Vec::len()` (types.tsv).
    /// `Upvaldesc.instack` is `bool` in Rust (types.tsv); cast to `u8` for the wire format.
    fn dump_upvalues(&mut self, proto: &LuaProto) -> Result<(), LuaError> {
        let n = proto.upvalues.len();
        self.dump_int(n as i32)?;

        for upval in &proto.upvalues {
            // PORT NOTE: instack is bool in Rust (types.tsv); cast to u8: true→1, false→0.
            self.dump_byte(upval.instack as u8)?;
            self.dump_byte(upval.idx)?;
            self.dump_byte(upval.kind)?;
        }
        Ok(())
    }

    /// Write debug information: per-instruction line deltas, absolute line records,
    /// local-variable lifetimes, and upvalue names.
    ///
    /// All counts are written as zero when `self.strip` is true.
    ///
    ///
    /// PORT NOTE: all `f->size*` fields are covered by `Vec::len()` (types.tsv).
    fn dump_debug(&mut self, proto: &LuaProto) -> Result<(), LuaError> {
        let n_lineinfo = if self.strip { 0 } else { proto.lineinfo.len() };
        self.dump_int(n_lineinfo as i32)?;

        // lineinfo is Vec<i8> (ls_byte per types.tsv). C writes them as raw bytes (sizeof(i8)=1).
        // Cast each i8 to u8 (same bit pattern) before writing.
        // PERF(port): iterating one byte at a time vs. bulk write — profile in Phase B.
        // (A bulk write would require bytemuck::cast_slice or similar to avoid unsafe.)
        let lineinfo_bytes: Vec<u8> = proto.lineinfo[..n_lineinfo]
            .iter()
            .map(|&b| b as u8)
            .collect();
        self.dump_block(&lineinfo_bytes)?;

        let n_absline = if self.strip {
            0
        } else {
            proto.abslineinfo.len()
        };
        self.dump_int(n_absline as i32)?;

        for abs in proto.abslineinfo.iter().take(n_absline) {
            // AbsLineInfo.pc and .line are i32 (types.tsv); non-negative in valid bytecode.
            self.dump_int(abs.pc)?;
            self.dump_int(abs.line)?;
        }

        let n_locvars = if self.strip { 0 } else { proto.locvars.len() };
        self.dump_int(n_locvars as i32)?;

        for locvar in proto.locvars.iter().take(n_locvars) {
            // LocVar.varname is GcRef<LuaString> (types.tsv).
            self.dump_string(Some(&locvar.varname))?;
            self.dump_int(locvar.startpc)?;
            self.dump_int(locvar.endpc)?;
        }

        // (Re-uses upvalues.len() for the name-writing pass — separate from dumpUpvalues
        //  which wrote structural descriptors; here we write debug names.)
        let n_upval_names = if self.strip { 0 } else { proto.upvalues.len() };
        self.dump_int(n_upval_names as i32)?;

        for upval in proto.upvalues.iter().take(n_upval_names) {
            // PORT NOTE: UpvalDesc.name is GcRef<LuaString> per types.tsv (non-optional).
            // TODO(port): In C, `TString *name` can be NULL when an upvalue is unnamed (e.g.,
            // in bytecode compiled without debug info). Verify whether UpvalDesc.name should be
            // `Option<GcRef<LuaString>>` in the Rust model; if so, change call to pass the Option
            // directly instead of wrapping in Some.
            self.dump_string(upval.name.as_ref())?;
        }
        Ok(())
    }

    /// Write a complete function prototype: source name, header bytes, code, constants,
    /// upvalue descriptors, nested prototypes, and debug information.
    ///
    /// `psource` is the parent function's source string. When `f->source == psource` (pointer
    /// equality — Lua interns short strings so identical source names share an object), the
    /// source is written as null (size 0) to avoid duplication. The top-level call passes
    /// `None` to force writing the source.
    ///
    ///
    /// PORT NOTE: `f->source == psource` is a C pointer comparison exploiting string interning.
    /// In Rust we use `GcRef::ptr_eq` (equivalent to `Rc::ptr_eq` in Phase A) for identity.
    /// `is_vararg` is `bool` in Rust (types.tsv); cast to `u8` for the wire format.
    fn dump_function(
        &mut self,
        proto: &LuaProto,
        psource: Option<&GcRef<LuaString>>,
    ) -> Result<(), LuaError> {
        // Pointer-equality check: same interned string object means same source file.
        let same_source = match (psource, proto.source.as_ref()) {
            (Some(ps), Some(src)) => GcRef::ptr_eq(src, ps),
            _ => false,
        };

        if self.strip || same_source {
            self.dump_string(None)?;
        } else {
            self.dump_string(proto.source.as_ref())?;
        }

        self.dump_int(proto.linedefined)?;
        self.dump_int(proto.lastlinedefined)?;
        self.dump_byte(proto.numparams)?;
        // PORT NOTE: is_vararg is bool in Rust (types.tsv); true → 1u8, false → 0u8.
        self.dump_byte(proto.is_vararg as u8)?;
        self.dump_byte(proto.maxstacksize)?;

        self.dump_code(proto)?;
        self.dump_constants(proto)?;
        self.dump_upvalues(proto)?;
        self.dump_protos(proto)?;
        self.dump_debug(proto)?;
        Ok(())
    }

    /// Write the binary chunk header.
    ///
    /// The header allows `lundump` (and external tools) to verify the bytecode format,
    /// platform word sizes, and byte order before attempting to load the chunk.
    ///
    fn dump_header(&mut self) -> Result<(), LuaError> {
        // dumpLiteral(D,s) = dumpBlock(D, s, sizeof(s) - sizeof(char))
        // b"\x1bLua" is &[u8; 4] (no NUL terminator in Rust byte literals), matching the
        // C expansion of sizeof("\x1bLua")-1 = 4 bytes.
        self.dump_block(LUA_SIGNATURE)?;

        match self.version {
            LuaVersion::V51 => {
                self.dump_byte(LUAC_VERSION_51)?;
                self.dump_byte(LUAC_FORMAT)?;
                self.dump_byte(LUAC_ENDIAN_LITTLE)?;
                self.dump_byte(C_INT_SIZE)?;
                self.dump_byte(C_SIZET_SIZE)?;
                self.dump_byte(INSTRUCTION_SIZE)?;
                self.dump_byte(LUA_NUMBER_SIZE)?;
                self.dump_byte(LUAC_INTEGRAL_FLOAT)?;
            }
            LuaVersion::V52 => {
                self.dump_byte(LUAC_VERSION_52)?;
                self.dump_byte(LUAC_FORMAT)?;
                self.dump_byte(LUAC_ENDIAN_LITTLE)?;
                self.dump_byte(C_INT_SIZE)?;
                self.dump_byte(C_SIZET_SIZE)?;
                self.dump_byte(INSTRUCTION_SIZE)?;
                self.dump_byte(LUA_NUMBER_SIZE)?;
                self.dump_byte(LUAC_INTEGRAL_FLOAT)?;
                self.dump_block(LUAC_DATA)?;
            }
            LuaVersion::V53 => {
                self.dump_byte(LUAC_VERSION_53)?;
                self.dump_byte(LUAC_FORMAT)?;
                self.dump_block(LUAC_DATA)?;
                self.dump_byte(C_INT_SIZE)?;
                self.dump_byte(C_SIZET_SIZE)?;
                self.dump_byte(INSTRUCTION_SIZE)?;
                self.dump_byte(LUA_INTEGER_SIZE)?;
                self.dump_byte(LUA_NUMBER_SIZE)?;
                self.dump_integer(LUAC_INT)?;
                self.dump_number(LUAC_NUM)?;
            }
            LuaVersion::V55 => {
                self.dump_byte(LUAC_VERSION_55)?;
                self.dump_byte(LUAC_FORMAT)?;
                self.dump_block(LUAC_DATA)?;
                self.dump_byte(size_of::<i32>() as u8)?;
                self.dump_raw_i32(LUAC_INT_55 as i32)?;

                self.dump_byte(INSTRUCTION_SIZE)?;
                self.dump_raw_u32(LUAC_INST_55)?;

                self.dump_byte(LUA_INTEGER_SIZE)?;
                self.dump_integer(LUAC_INT_55)?;

                self.dump_byte(LUA_NUMBER_SIZE)?;
                self.dump_number(LUAC_NUM_55)?;
            }
            _ => {
                self.dump_byte(LUAC_VERSION_54)?;
                self.dump_byte(LUAC_FORMAT)?;
                self.dump_block(LUAC_DATA)?;
                self.dump_byte(INSTRUCTION_SIZE)?;
                self.dump_byte(LUA_INTEGER_SIZE)?;
                self.dump_byte(LUA_NUMBER_SIZE)?;
                self.dump_integer(LUAC_INT)?;
                self.dump_number(LUAC_NUM)?;
            }
        }

        Ok(())
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Serialize a compiled Lua function prototype as a precompiled bytecode chunk.
///
/// The `writer` callback receives successive slices of the serialised bytes and returns
/// `Err(LuaError)` to abort. `strip` omits debug info (line numbers, local names, etc.)
/// from the output.
///
///
/// PORT NOTE: `lua_Writer w` (fn pointer) + `void *data` (userdata) are collapsed into a
/// single `impl FnMut(&[u8]) -> Result<(), LuaError>` closure — the Rust idiom for the
/// callback + context pair. `_state` is retained in the signature for API parity but unused
/// in the body: the C code needed it only for `lua_lock`/`lua_unlock`, which are no-ops per
/// macros.tsv. Return type changes from `int` (0 = ok, non-zero = writer error) to
/// `Result<(), LuaError>`.
pub(crate) fn dump(
    state: &LuaState,
    proto: &GcRef<LuaProto>,
    writer: &mut dyn FnMut(&[u8]) -> Result<(), LuaError>,
    strip: bool,
) -> Result<(), LuaError> {
    let mut d = DumpState {
        writer,
        strip,
        version: state.global().lua_version,
    };

    d.dump_header()?;

    // PORT NOTE: f->sizeupvalues is covered by Vec::len(). Bounded by MAXUPVAL = 255
    // (macros.tsv), so truncation via `as u8` is safe for well-formed prototypes.
    d.dump_byte(proto.upvalues.len() as u8)?;

    // psource = None forces the top-level function to always write its source name.
    // Deref coercion: &GcRef<LuaProto> → &LuaProto (via Deref<Target=LuaProto> on GcRef/Rc).
    d.dump_function(proto, None)?;

    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/ldump.c  (230 lines, 10 functions)
//   target_crate:  lua-vm
//   confidence:    medium
//   todos:         4
//   port_notes:    12
//   unsafe_blocks: 0
//   notes:         Types/imports need Phase B wiring; logic should be faithful.
//                  Key uncertainties: (1) Instruction newtype inner-field access (.0 vs
//                  method); (2) UpvalDesc.name optionality; (3) GcRef::ptr_eq method
//                  existence. Lineinfo bulk-write is done via collect()+dump_block to
//                  avoid unsafe transmute of &[i8] → &[u8]; revisit with bytemuck in
//                  Phase B for performance. Native-endian serialisation via to_ne_bytes()
//                  matches C's raw-memory dumpVector behaviour.
// ────────────────────────────────────────────────────────────────────────────
