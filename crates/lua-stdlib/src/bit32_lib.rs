//! `bit32` — the Lua 5.2/5.3 32-bit bitwise library (port of `lbitlib.c`).
//!
//! `bit32` was introduced in Lua 5.2 and removed in 5.4 once native 64-bit
//! bitwise operators (`&` `|` `~` `<<` `>>`) arrived in 5.3. In a stock build it
//! is present in **both** 5.2 and 5.3 — 5.3 keeps it under the default-on
//! `LUA_COMPAT_BITLIB` flag — so [`init`](crate::init) registers it under the
//! `V52 | V53` gate. Verified against the reference binaries: `type(bit32)` is
//! nil / table / table / nil for 5.1 / 5.2 / 5.3 / 5.4. That gate is
//! load-bearing; narrowing it to 5.2-only would drop a 5.3 builtin.
//!
//! Every operation masks its operands and result to **32 bits** (`mod 2^32`):
//! this unsigned 32-bit window is the library's defining semantics and is what
//! distinguishes it from 5.3's native 64-bit operators.
//!
//! ## Graduation (idiomatization sprint 2)
//!
//! The whole 5.2/5.3 surface is implemented and reference-pinned:
//! `band` `bor` `bxor` `bnot` `btest` `lshift` `rshift` `arshift` `lrotate`
//! `rrotate` `extract` `replace`. The behavioral net lives in
//! `crates/lua-stdlib/tests/bit32_strengthen.rs` (pinned to lua5.2.4, with a
//! lua5.3.6 contrast for the one version-specific behavior — see [`arg_u32`]).

use crate::state_stub::{LuaState, LuaStateStubExt as _};
use lua_types::{LuaError, LuaValue, NumberModel};

type LuaCFunction = fn(&mut LuaState) -> Result<usize, LuaError>;

/// Coerce a Lua number argument to its unsigned 32-bit image, matching
/// `lbitlib.c`'s argument handling — which differs by host version:
///
/// - **5.2 (the `FloatOnly` number model):** `lbitlib.c` uses
///   `luaL_checkunsigned` → `lua_tounsigned`, which rounds the number to the
///   nearest integer (ties to even) and reduces it `mod 2^32`. A fractional
///   float is therefore accepted, e.g. `bit32.band(1.5) == 2`.
/// - **5.3 (the `Dual` model, under `LUA_COMPAT_BITLIB`):** `lbitlib.c` uses
///   `luaL_checkinteger`, which rejects a non-integer-valued float with
///   `number has no integer representation`.
///
/// A non-number argument raises `number expected, got <type>` in both, which is
/// exactly what `check_number` / `check_integer` already produce.
fn arg_u32(state: &mut LuaState, arg: i32) -> Result<u32, LuaError> {
    let model = state.global().lua_version.number_model();
    match model {
        NumberModel::FloatOnly => {
            let n = state.check_number(arg)?;
            Ok(n.round_ties_even().rem_euclid(4_294_967_296.0) as u32)
        }
        NumberModel::Dual => {
            let n = state.check_integer(arg)?;
            Ok(n as u32)
        }
    }
}

/// Coerce a Lua number argument to a signed integer for `bit32`'s **count**
/// arguments — the shift/rotate displacement and the `extract`/`replace` field
/// and width. `lbitlib.c` reads these with `luaL_checkint`/`luaL_checkinteger`,
/// which (unlike the operand path in [`arg_u32`]) **truncates toward zero**
/// under 5.2's `FloatOnly` model — e.g. `bit32.lshift(1, 1.5)` shifts by 1, and
/// `bit32.extract(0xAA, 1.5)` reads bit 1. 5.3 keeps `luaL_checkinteger`'s
/// reject-on-fraction behavior.
fn arg_int(state: &mut LuaState, arg: i32) -> Result<i64, LuaError> {
    let model = state.global().lua_version.number_model();
    match model {
        NumberModel::FloatOnly => Ok(state.check_number(arg)?.trunc() as i64),
        NumberModel::Dual => state.check_integer(arg),
    }
}

/// Push an unsigned 32-bit result as a Lua integer.
fn push_u32(state: &mut LuaState, v: u32) {
    state.push(LuaValue::Int(v as i64));
}

/// Fold a variadic AND/OR/XOR over every argument, starting from `init`.
fn fold(state: &mut LuaState, init: u32, op: fn(u32, u32) -> u32) -> Result<usize, LuaError> {
    let top = state.get_top();
    let mut acc = init;
    for i in 1..=top {
        acc = op(acc, arg_u32(state, i)?);
    }
    push_u32(state, acc);
    Ok(1)
}

fn bit_band(state: &mut LuaState) -> Result<usize, LuaError> {
    fold(state, 0xFFFF_FFFF, |a, b| a & b)
}

fn bit_bor(state: &mut LuaState) -> Result<usize, LuaError> {
    fold(state, 0, |a, b| a | b)
}

fn bit_bxor(state: &mut LuaState) -> Result<usize, LuaError> {
    fold(state, 0, |a, b| a ^ b)
}

fn bit_bnot(state: &mut LuaState) -> Result<usize, LuaError> {
    let a = arg_u32(state, 1)?;
    push_u32(state, !a);
    Ok(1)
}

fn bit_lshift(state: &mut LuaState) -> Result<usize, LuaError> {
    let a = arg_u32(state, 1)?;
    let disp = arg_int(state, 2)?;
    push_u32(state, shift(a, disp));
    Ok(1)
}

fn bit_rshift(state: &mut LuaState) -> Result<usize, LuaError> {
    let a = arg_u32(state, 1)?;
    let disp = arg_int(state, 2)?;
    push_u32(state, shift(a, -disp));
    Ok(1)
}

/// `bit32` logical shift: positive `disp` shifts left, negative shifts right;
/// a displacement of 32 or more (in magnitude) yields 0, matching the reference.
fn shift(x: u32, disp: i64) -> u32 {
    if disp <= -32 || disp >= 32 {
        0
    } else if disp >= 0 {
        x << disp
    } else {
        x >> (-disp)
    }
}

/// `w` low bits set, matching `bit32`'s field mask (`width` in `1..=32`).
fn mask_w(w: u32) -> u32 {
    if w >= 32 {
        0xFFFF_FFFF
    } else {
        (1u32 << w) - 1
    }
}

/// Validate and return the `(field, width)` pair for `extract`/`replace`,
/// matching Lua 5.2's `fieldargs` bounds checks. `width_arg` defaults to 1.
fn field_args(
    state: &mut LuaState,
    field_arg: i32,
    width_arg: i32,
) -> Result<(u32, u32), LuaError> {
    let f = arg_int(state, field_arg)?;
    let w = if state.get_top() >= width_arg {
        arg_int(state, width_arg)?
    } else {
        1
    };
    if f < 0 {
        return Err(LuaError::arg_error(field_arg, "field cannot be negative"));
    }
    if w < 1 {
        return Err(LuaError::arg_error(width_arg, "width must be positive"));
    }
    if f + w > 32 {
        return Err(LuaError::arg_error(
            field_arg,
            "trying to access non-existent bits",
        ));
    }
    Ok((f as u32, w as u32))
}

/// `bit32.btest(...)` — true iff the AND of all arguments is non-zero.
fn bit_btest(state: &mut LuaState) -> Result<usize, LuaError> {
    let top = state.get_top();
    let mut acc: u32 = 0xFFFF_FFFF;
    for i in 1..=top {
        acc &= arg_u32(state, i)?;
    }
    state.push(LuaValue::Bool(acc != 0));
    Ok(1)
}

/// `bit32.extract(n, field [, width])` — the `width` bits of `n` at `field`.
fn bit_extract(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = arg_u32(state, 1)?;
    let (f, w) = field_args(state, 2, 3)?;
    push_u32(state, (n >> f) & mask_w(w));
    Ok(1)
}

/// `bit32.replace(n, v, field [, width])` — `n` with its `width` bits at
/// `field` replaced by the low bits of `v`.
fn bit_replace(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = arg_u32(state, 1)?;
    let v = arg_u32(state, 2)?;
    let (f, w) = field_args(state, 3, 4)?;
    let m = mask_w(w);
    push_u32(state, (n & !(m << f)) | ((v & m) << f));
    Ok(1)
}

/// `bit32.arshift(x, disp)` — arithmetic right shift (sign-propagating);
/// negative `disp` shifts left.
fn bit_arshift(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = arg_u32(state, 1)?;
    let disp = arg_int(state, 2)?;
    let r = if disp < 0 {
        shift(x, -disp)
    } else if disp >= 32 {
        if x & 0x8000_0000 != 0 {
            0xFFFF_FFFF
        } else {
            0
        }
    } else if x & 0x8000_0000 != 0 {
        (x >> disp) | !(0xFFFF_FFFFu32 >> disp)
    } else {
        x >> disp
    };
    push_u32(state, r);
    Ok(1)
}

/// 32-bit rotate left by `disp` (mod 32); negative rotates right.
fn rotate(x: u32, disp: i64) -> u32 {
    let d = (((disp % 32) + 32) % 32) as u32;
    if d == 0 {
        x
    } else {
        (x << d) | (x >> (32 - d))
    }
}

fn bit_lrotate(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = arg_u32(state, 1)?;
    let disp = arg_int(state, 2)?;
    push_u32(state, rotate(x, disp));
    Ok(1)
}

fn bit_rrotate(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = arg_u32(state, 1)?;
    let disp = arg_int(state, 2)?;
    push_u32(state, rotate(x, -disp));
    Ok(1)
}

/// The `bit32` function roster — the full Lua 5.2/5.3 surface.
const BIT32_FUNCS: &[(&[u8], LuaCFunction)] = &[
    (b"band", bit_band),
    (b"bor", bit_bor),
    (b"bxor", bit_bxor),
    (b"bnot", bit_bnot),
    (b"lshift", bit_lshift),
    (b"rshift", bit_rshift),
    (b"btest", bit_btest),
    (b"extract", bit_extract),
    (b"replace", bit_replace),
    (b"arshift", bit_arshift),
    (b"lrotate", bit_lrotate),
    (b"rrotate", bit_rrotate),
];

/// Open the `bit32` library, leaving the populated table on the stack.
pub fn open_bit32(state: &mut LuaState) -> Result<usize, LuaError> {
    state.new_lib(BIT32_FUNCS)?;
    Ok(1)
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lbitlib.c (Lua 5.2/5.3)
//   target_crate:  lua-stdlib
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         Full 5.2/5.3 bit32 surface (band/bor/bxor/bnot/btest/lshift/
//                  rshift/arshift/lrotate/rrotate/extract/replace), registered
//                  under the V52|V53 gate. mod-2^32 masking and the version-
//                  specific float coercion (5.2 rounds, 5.3 rejects fractions)
//                  are reference-pinned in tests/bit32_strengthen.rs.
// ──────────────────────────────────────────────────────────────────────────
