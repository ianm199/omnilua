//! Standard mathematical library — `math.*`.
//!
//! The PRNG is xoshiro256** operating on four 64-bit words (the single 64-bit
//! code path; there is no 32-bit fallback to keep).
//!
//! The deprecated `LUA_COMPAT_MATHLIB` roster (`cosh`, `sinh`, `tanh`, `pow`,
//! `log10`, `atan2`) ships in the default 5.1/5.2/5.3/5.4 builds and is dropped
//! in 5.5; `frexp`/`ldexp` survive into 5.5. `atan2` is an alias of `math_atan`.
//! Which functions exist per version is the registration logic in
//! [`luaopen_math`].

use crate::state_stub::{LuaState, LuaStateStubExt as _};
use lua_types::{LuaError, LuaType, LuaValue};

// ── Constants ──────────────────────────────────────────────────────────────

/// `math.pi`. Higher precision than `std::f64::consts::PI` (which is rounded to
/// the nearest `f64`); both round-trip to the same `f64` bit pattern.
const PI: f64 = 3.141592653589793238462643383279502884_f64;

/// Number of binary digits in the `f64` mantissa (`DBL_MANT_DIG`).
const FIGS: u32 = 53;

/// Bits to discard from a 64-bit random word before float conversion (`= 11`):
/// the word is reduced to its top [`FIGS`] significant bits.
const SHIFT64_FIG: u32 = 64 - FIGS;

// ── Type aliases for library registration ─────────────────────────────────

/// A Lua C-style function: takes the Lua state, returns count of pushed values.
type LuaCFunction = fn(&mut LuaState) -> Result<usize, LuaError>;

// ── PRNG state ────────────────────────────────────────────────────────────

/// State for the xoshiro256** PRNG.
///
/// In C this is stored as raw `lua_newuserdatauv` memory and accessed by
/// casting the userdata pointer. Until typed-userdata closure upvalues land
/// in Phase B, we keep the PRNG state in a thread-local cell so that
/// `math.random` and `math.randomseed` are callable from Lua. This collapses
/// per-lua_State PRNG isolation to per-thread, which is sufficient for the
/// 5.4 test corpus.
struct RanState {
    s: [u64; 4],
}

thread_local! {
    static RAN_STATE: std::cell::RefCell<RanState> =
        std::cell::RefCell::new(RanState { s: [0xff, 0xff, 0xff, 0xff] });
}

// ── Pure PRNG algorithms ──────────────────────────────────────────────────

/// Advance the xoshiro256** state by one step and return the next raw 64-bit
/// pseudo-random value.
///
fn next_rand(s: &mut [u64; 4]) -> u64 {
    let s0 = s[0];
    let s1 = s[1];
    let s2 = s[2] ^ s0;
    let s3 = s[3] ^ s1;
    let res = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
    s[0] = s0 ^ s3;
    s[1] = s1 ^ s2;
    s[2] = s2 ^ (s1 << 17);
    s[3] = s3.rotate_left(45);
    res
}

/// Convert a raw 64-bit PRNG output to a float in [0.0, 1.0).
///
/// Takes the top FIGS=53 bits, interprets them as a signed integer, scales
/// by `scaleFIG = 0.5 / 2^52`, then corrects the two's-complement sign.
fn rand_to_float(x: u64) -> f64 {
    let sx = (x >> SHIFT64_FIG) as i64;
    let scale_fig: f64 = 0.5 / ((1u64 << (FIGS - 1)) as f64);
    let mut res = (sx as f64) * scale_fig;
    if sx < 0 {
        res += 1.0;
    }
    debug_assert!(0.0 <= res && res < 1.0);
    res
}

/// Initialise the four PRNG words from two seed values.
///
/// `s[1]` is forced to `0xff` so the state is never all-zero (xoshiro's
/// fixed point), and sixteen draws are discarded to spread the seed bits before
/// the first observable output. Takes the word array (not the [`LuaState`]) so
/// the caller can push the seed values without a borrow conflict against the
/// `RanState` upvalue.
fn set_seed_words(s: &mut [u64; 4], n1: u64, n2: u64) {
    s[0] = n1;
    s[1] = 0xff;
    s[2] = n2;
    s[3] = 0;
    for _ in 0..16 {
        next_rand(s);
    }
}

/// Project `ran` uniformly into `[0, n]` by rejection sampling.
///
/// When `n + 1` is a power of two the low bits are already uniform, so a single
/// mask suffices. Otherwise `lim` is built as the smallest `2^b - 1` not smaller
/// than `n` (the bit-smear), and draws outside `[0, n]` are rejected and
/// redrawn. The `>> 32` smear step is unconditional here because `u64` always
/// has 64 bits; the C source guards it behind an `#if` on the integer width.
/// Takes the word array (not `&mut RanState`) to avoid nested borrows at the
/// call sites.
fn project(mut ran: u64, n: u64, s: &mut [u64; 4]) -> u64 {
    if (n & n.wrapping_add(1)) == 0 {
        return ran & n;
    }
    let mut lim = n;
    lim |= lim >> 1;
    lim |= lim >> 2;
    lim |= lim >> 4;
    lim |= lim >> 8;
    lim |= lim >> 16;
    lim |= lim >> 32;
    debug_assert!((lim & lim.wrapping_add(1)) == 0);
    debug_assert!(lim >= n);
    debug_assert!((lim >> 1) < n);
    loop {
        ran &= lim;
        if ran <= n {
            break;
        }
        ran = next_rand(s);
    }
    ran
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Whether the argument at stack index `n` carries the integer subtype.
///
/// Several `math.*` functions (`abs`, `floor`, `ceil`, `fmod`, `modf`, `type`)
/// branch on whether an argument is an `Int` versus a `Float`, preserving the
/// integer subtype on the integer path. This names that recurring predicate so
/// the branch reads as intent rather than as a raw `matches!` on the stack slot.
fn arg_is_int(state: &mut LuaState, n: i32) -> bool {
    matches!(state.value_at(n), LuaValue::Int(_))
}

/// The smallest `f64` whose value equals an `i64` exactly: `i64::MIN` as a
/// float (`-2^63`, exactly representable).
const I64_MIN_AS_F64: f64 = i64::MIN as f64;

/// One past the largest representable `i64`, as a float: `2^63`. `i64::MAX`
/// (`2^63 - 1`) is not exactly representable in `f64`, so the in-range test
/// uses a strict `<` against this boundary rather than `<= i64::MAX as f64`.
const I64_MAX_PLUS_1_AS_F64: f64 = -(i64::MIN as f64);

/// Push `d` as an `Int` when it has an integer value that fits exactly in an
/// `i64`, otherwise push it as a `Float`.
///
/// `floor`/`ceil`/`modf` use this to keep their result an integer subtype when
/// it round-trips through `i64` without loss, falling back to a float for
/// magnitudes outside the `i64` range. The bound check is half-open
/// (`I64_MIN_AS_F64 <= d < I64_MAX_PLUS_1_AS_F64`) because `i64::MAX` itself is
/// not exactly representable as an `f64`.
fn push_int_or_float(state: &mut LuaState, d: f64) {
    if d >= I64_MIN_AS_F64 && d < I64_MAX_PLUS_1_AS_F64 {
        state.push(LuaValue::Int(d as i64));
    } else {
        state.push(LuaValue::Float(d));
    }
}

/// Leave the first argument as the sole result (an integer floor/ceil/modf is
/// already its own integer part — return it unchanged, preserving subtype).
///
/// This goes through the public, frame-relative [`lua_vm::api::set_top`] so the
/// truncation is relative to the call frame; the inherent `LuaState::set_top`
/// would treat its argument as an absolute `StackIdx` and discard the argument.
fn keep_first_arg(state: &mut LuaState) -> Result<(), LuaError> {
    lua_vm::api::set_top(state, 1)
}

// ── Basic math functions ──────────────────────────────────────────────────

/// `math.abs(x)` — absolute value, preserving integer type when possible.
///
fn math_abs(state: &mut LuaState) -> Result<usize, LuaError> {
    if arg_is_int(state, 1) {
        let n = state.to_integer(1).unwrap_or(0);
        let n = if n < 0 {
            (0u64.wrapping_sub(n as u64)) as i64
        } else {
            n
        };
        state.push(LuaValue::Int(n));
    } else {
        let x = state.check_number(1)?;
        state.push(LuaValue::Float(x.abs()));
    }
    Ok(1)
}

/// `math.sin(x)` — sine (radians).
///
fn math_sin(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.sin()));
    Ok(1)
}

/// `math.cos(x)` — cosine (radians).
///
fn math_cos(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.cos()));
    Ok(1)
}

/// `math.tan(x)` — tangent (radians).
///
fn math_tan(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.tan()));
    Ok(1)
}

/// `math.asin(x)` — arc-sine, result in radians.
///
fn math_asin(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.asin()));
    Ok(1)
}

/// `math.acos(x)` — arc-cosine, result in radians.
///
fn math_acos(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.acos()));
    Ok(1)
}

/// `math.atan(y [, x])` — arc-tangent of y/x (defaults x=1), result in
/// radians. Subsumes C's `atan2` when x is provided.
///
fn math_atan(state: &mut LuaState) -> Result<usize, LuaError> {
    let y = state.check_number(1)?;
    let x = state.opt_number(2, 1.0)?;
    state.push(LuaValue::Float(y.atan2(x)));
    Ok(1)
}

/// `math.cosh(x)` — hyperbolic cosine. Deprecated `LUA_COMPAT_MATHLIB`
/// function, registered only under the 5.3 backend.
///
fn math_cosh(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.cosh()));
    Ok(1)
}

/// `math.sinh(x)` — hyperbolic sine. Deprecated `LUA_COMPAT_MATHLIB`
/// function, registered only under the 5.3 backend.
///
fn math_sinh(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.sinh()));
    Ok(1)
}

/// `math.tanh(x)` — hyperbolic tangent. Deprecated `LUA_COMPAT_MATHLIB`
/// function, registered only under the 5.3 backend.
///
fn math_tanh(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.tanh()));
    Ok(1)
}

/// `math.pow(x, y)` — x raised to the power y, always returning a float.
/// Deprecated `LUA_COMPAT_MATHLIB` function, registered only under the 5.3
/// backend. Mirrors C `pow(luaL_checknumber, luaL_checknumber)`.
///
fn math_pow(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    let y = state.check_number(2)?;
    state.push(LuaValue::Float(x.powf(y)));
    Ok(1)
}

/// `math.log10(x)` — base-10 logarithm. Deprecated `LUA_COMPAT_MATHLIB`
/// function, registered only under the 5.3 backend.
///
fn math_log10(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.log10()));
    Ok(1)
}

/// `math.ldexp(x, e)` — `x * 2^e`. Deprecated `LUA_COMPAT_MATHLIB` function,
/// registered only under the 5.3 backend. The exponent is an integer argument
/// truncated to C `int` range, matching `ldexp(x, (int)luaL_checkinteger)`.
///
fn math_ldexp(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    let e = state.check_integer(2)? as i32;
    state.push(LuaValue::Float(ldexp(x, e)));
    Ok(1)
}

/// Pure `ldexp`: returns `x * 2^exp` with C `ldexp` semantics.
///
/// A naive `x * 2f64.powi(exp)` underflows (or overflows) the intermediate
/// `2^exp` for large-magnitude exponents, losing subnormal results such as
/// `ldexp(1.0, -1074) == 5e-324`. The scaling is therefore applied in bounded
/// steps so no intermediate factor under/overflows: each step multiplies by a
/// power of two whose magnitude stays inside the normal `f64` range.
fn ldexp(x: f64, exp: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let mut result = x;
    let mut e = exp;
    // 2^1023 is the largest power of two representable as a normal f64; chunk
    // the exponent so each `from_bits` factor is always finite and nonzero.
    while e > 1023 {
        result *= f64::from_bits(0x7feu64 << 52); // 2^1023
        e -= 1023;
    }
    while e < -1022 {
        result *= f64::from_bits(0x001u64 << 52); // 2^-1022 (smallest normal)
        e += 1022;
    }
    result * f64::from_bits(((e + 1023) as u64) << 52)
}

/// `math.frexp(x)` — split x into a normalized mantissa and an exponent such
/// that `x == mantissa * 2^exponent` with `0.5 <= |mantissa| < 1`. Returns the
/// float mantissa followed by the **integer** exponent, matching C
/// `frexp` + `lua_pushinteger`. Deprecated `LUA_COMPAT_MATHLIB` function,
/// registered only under the 5.3 backend.
///
/// Rust std has no `frexp`; this replicates C `frexp` via `f64` bit
/// manipulation, including the `frexp(0.0) == (0.0, 0)` special case (and the
/// matching `-0.0`, infinity, and NaN cases, which C leaves unchanged with a
/// zero exponent).
fn math_frexp(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    let (mantissa, exponent) = frexp(x);
    state.push(LuaValue::Float(mantissa));
    state.push(LuaValue::Int(exponent as i64));
    Ok(2)
}

/// Pure `frexp`: returns `(mantissa, exponent)` with `x == mantissa * 2^exp`.
///
/// Replicates C `frexp` semantics for f64. Zero, infinity, and NaN are
/// returned unchanged with a zero exponent.
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    if raw_exp == 0 {
        // Subnormal: scale up by 2^54 to normalize, then correct the exponent.
        let (m, e) = frexp(x * (1u64 << 54) as f64);
        return (m, e - 54);
    }
    // Bias the exponent so the mantissa lands in [0.5, 1): set the stored
    // exponent field to 0x3fe (unbiased -1).
    let exponent = raw_exp - 1022;
    let mantissa_bits = (bits & !(0x7ffu64 << 52)) | (0x3feu64 << 52);
    (f64::from_bits(mantissa_bits), exponent)
}

/// `math.tointeger(x)` — return `x` as an integer, or the fail value when it
/// has no exact integer representation.
///
/// The fail value is `nil`, not `false`: the default 5.3/5.4/5.5 builds expand
/// `luaL_pushfail` to a nil push (only a `LUA_FAILISFALSE` build would push
/// `false`), and the oracle contract pins the nil.
fn math_toint(state: &mut LuaState) -> Result<usize, LuaError> {
    if let Some(n) = state.to_integer_opt(1) {
        state.push(LuaValue::Int(n));
    } else {
        state.check_any(1)?;
        state.push(LuaValue::Nil);
    }
    Ok(1)
}

/// `math.floor(x)` — largest integer ≤ x.
///
fn math_floor(state: &mut LuaState) -> Result<usize, LuaError> {
    if arg_is_int(state, 1) {
        keep_first_arg(state)?;
    } else {
        let d = state.check_number(1)?.floor();
        push_int_or_float(state, d);
    }
    Ok(1)
}

/// `math.ceil(x)` — smallest integer ≥ x.
///
fn math_ceil(state: &mut LuaState) -> Result<usize, LuaError> {
    if arg_is_int(state, 1) {
        keep_first_arg(state)?;
    } else {
        let d = state.check_number(1)?.ceil();
        push_int_or_float(state, d);
    }
    Ok(1)
}

/// `math.fmod(x, y)` — floating-point remainder (same sign as x).
///
fn math_fmod(state: &mut LuaState) -> Result<usize, LuaError> {
    if arg_is_int(state, 1) && arg_is_int(state, 2) {
        let a = state.to_integer(1).unwrap_or(0);
        let d = state.to_integer(2).unwrap_or(0);
        if (d as u64).wrapping_add(1) <= 1 {
            if d == 0 {
                return Err(lua_vm::debug::arg_error_impl(state, 2, b"zero"));
            }
            state.push(LuaValue::Int(0));
        } else {
            state.push(LuaValue::Int(a % d));
        }
    } else {
        let x = state.check_number(1)?;
        let y = state.check_number(2)?;
        state.push(LuaValue::Float(x % y));
    }
    Ok(1)
}

/// `math.modf(x)` — split into integer and fractional parts; returns both.
///
/// The integer part is computed with `ceil`/`floor` + subtraction rather than
/// libc `modf`, avoiding a `double *` out-parameter ABI. An integer argument is
/// its own integer part with a `0.0` fractional part.
fn math_modf(state: &mut LuaState) -> Result<usize, LuaError> {
    if arg_is_int(state, 1) {
        keep_first_arg(state)?;
        state.push(LuaValue::Float(0.0));
    } else {
        let n = state.check_number(1)?;
        let ip = if n < 0.0 { n.ceil() } else { n.floor() };
        push_int_or_float(state, ip);
        let frac = if n == ip { 0.0 } else { n - ip };
        state.push(LuaValue::Float(frac));
    }
    Ok(2)
}

/// `math.sqrt(x)` — square root.
///
fn math_sqrt(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.sqrt()));
    Ok(1)
}

/// `math.ult(m, n)` — unsigned less-than on integers.
///
fn math_ult(state: &mut LuaState) -> Result<usize, LuaError> {
    let a = state.check_integer(1)?;
    let b = state.check_integer(2)?;
    state.push(LuaValue::Bool((a as u64) < (b as u64)));
    Ok(1)
}

/// `math.log(x [, base])` — logarithm; natural if base omitted.
///
fn math_log(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    // Lua 5.1's `math.log` takes a single argument and silently ignores any
    // second; the two-argument base form is a 5.2 addition. Verified against
    // lua5.1.5: `math.log(8,2) == math.log(8) == ln(8)`, and a second arg never
    // errors. See specs/followup/5.1-roster-syntax.md §1.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        state.push(LuaValue::Float(x.ln()));
        return Ok(1);
    }
    let res = if matches!(state.type_at(2), LuaType::None | LuaType::Nil) {
        x.ln()
    } else {
        let base = state.check_number(2)?;
        if base == 2.0 {
            x.log2()
        } else if base == 10.0 {
            x.log10()
        } else {
            x.ln() / base.ln()
        }
    };
    state.push(LuaValue::Float(res));
    Ok(1)
}

/// `math.exp(x)` — e raised to the power x.
///
fn math_exp(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x.exp()));
    Ok(1)
}

/// `math.deg(x)` — convert radians to degrees.
///
fn math_deg(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x * (180.0 / PI)));
    Ok(1)
}

/// `math.rad(x)` — convert degrees to radians.
///
fn math_rad(state: &mut LuaState) -> Result<usize, LuaError> {
    let x = state.check_number(1)?;
    state.push(LuaValue::Float(x * (PI / 180.0)));
    Ok(1)
}

/// `math.min(x, ...)` — minimum of all arguments (uses Lua `<` comparison).
///
fn math_min(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.get_top();
    let mut imin: i32 = 1;
    if n < 1 {
        return Err(lua_vm::debug::arg_error_impl(state, 1, b"value expected"));
    }
    for i in 2..=n {
        if state.compare_lt(i, imin)? {
            imin = i;
        }
    }
    state.push_value(imin)?;
    Ok(1)
}

/// `math.max(x, ...)` — maximum of all arguments (uses Lua `<` comparison).
///
fn math_max(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.get_top();
    let mut imax: i32 = 1;
    if n < 1 {
        return Err(lua_vm::debug::arg_error_impl(state, 1, b"value expected"));
    }
    for i in 2..=n {
        if state.compare_lt(imax, i)? {
            imax = i;
        }
    }
    state.push_value(imax)?;
    Ok(1)
}

/// `math.type(x)` — return `"integer"`, `"float"`, or the fail value for
/// non-numbers.
///
/// The non-number fail value is `nil`, not `false`, for the same reason as
/// [`math_toint`]: `luaL_pushfail` pushes nil in the default builds and the
/// oracle contract pins the nil.
fn math_type(state: &mut LuaState) -> Result<usize, LuaError> {
    if matches!(state.type_at(1), LuaType::Number) {
        if arg_is_int(state, 1) {
            state.push_string(b"integer")?;
        } else {
            state.push_string(b"float")?;
        }
    } else {
        state.check_any(1)?;
        state.push(LuaValue::Nil);
    }
    Ok(1)
}

// ── PRNG-backed Lua functions ─────────────────────────────────────────────

/// `math.random([m [, n]])` — pseudo-random number generation.
///
/// With no arguments: float in `[0, 1)`. With one argument `n`: integer in
/// `[1, n]` (or a full-range `u64` when `n == 0`, on 5.4/5.5). With two
/// arguments `m, n`: integer in `[m, n]`. The PRNG word is advanced first, then
/// the arguments are read via a separate borrow.
///
/// TODO: the PRNG state is a thread-local rather than per-`lua_State` typed
/// userdata in closure upvalue 1; migrating it to `GcRef<RefCell<RanState>>`
/// (so each state has its own generator) is deferred to the upvalue-userdata
/// work, not changed here.
fn math_random(state: &mut LuaState) -> Result<usize, LuaError> {
    let rv = advance_prng(state)?;
    let n_args = state.get_top();

    if n_args == 0 {
        state.push(LuaValue::Float(rand_to_float(rv)));
        return Ok(1);
    }

    let version = state.global().lua_version;
    let is_v53 = version == lua_types::LuaVersion::V53;
    // 5.1/5.2 are float-only and use the C `rand()` contract: there is no
    // `random(0)` full-range special case (that is a 5.4/5.5 addition), the
    // empty-interval error for `random(m, n)` reports argument index 2 (the
    // upper bound), and integer-valued results are pushed as `Float` to honour
    // the never-construct-`Int` invariant under `FloatOnly`. See
    // specs/followup/5.1-numbers-prng.md §"Impl seams".
    let float_only = version.number_model() == lua_types::NumberModel::FloatOnly;

    let (low, up, empty_arg) = match n_args {
        1 => {
            let up = state.check_integer(1)?;
            // 5.4/5.5 `random(0)` returns a full-range integer; 5.1/5.2/5.3 have
            // no such special case — it is `[1, 0]`, an empty interval.
            if up == 0 && !is_v53 && !float_only {
                // I2UInt(rv) = rv (trivial for u64)
                state.push(LuaValue::Int(rv as i64));
                return Ok(1);
            }
            (1i64, up, 1)
        }
        2 => {
            let low = state.check_integer(1)?;
            let up = state.check_integer(2)?;
            // 5.1's `luaL_checkint(L, 2)` for the upper bound means its
            // empty-interval `luaL_argerror` reports argument #2; the modern
            // bodies report #1.
            let empty_arg = if float_only { 2 } else { 1 };
            (low, up, empty_arg)
        }
        _ => {
            return Err(LuaError::runtime(format_args!("wrong number of arguments")));
        }
    };

    if low > up {
        return Err(lua_vm::debug::arg_error_impl(
            state,
            empty_arg,
            b"interval is empty",
        ));
    }

    // 5.3 `math_random` rejects intervals whose width overflows a signed integer
    // (`low >= 0 || up <= LUA_MAXINTEGER + low`). 5.4/5.5 use the `project`
    // bit-mask algorithm, which handles the full range without erroring.
    if is_v53 && !(low >= 0 || up <= i64::MAX.wrapping_add(low)) {
        return Err(lua_vm::debug::arg_error_impl(
            state,
            1,
            b"interval too large",
        ));
    }

    let range = (up as u64).wrapping_sub(low as u64);
    let p = project_from_upvalue(state, rv, range)?;
    let result = (p as u64).wrapping_add(low as u64) as i64;
    if float_only {
        state.push(LuaValue::Float(result as f64));
    } else {
        state.push(LuaValue::Int(result));
    }
    Ok(1)
}

/// `math.randomseed([x [, y]])` — seed the PRNG.
///
/// The return shape and auto-seed behavior are version-gated and load-bearing:
/// - 5.1/5.2 (float-only) REQUIRE the seed argument (a missing one raises
///   "number expected, got no value"), take a single seed word, and return NO
///   values.
/// - 5.3+ auto-seed from host entropy when the argument is absent and return the
///   two seed words.
///
/// See `specs/followup/5.1-numbers-prng.md`.
fn math_randomseed(state: &mut LuaState) -> Result<usize, LuaError> {
    let float_only = state.global().lua_version.number_model() == lua_types::NumberModel::FloatOnly;

    if matches!(state.type_at(1), LuaType::None) {
        if float_only {
            let n1 = state.check_integer(1)? as u64;
            apply_set_seed_quiet(state, n1, 0);
            return Ok(0);
        }
        apply_random_seed(state)?;
    } else {
        let n1 = state.check_integer(1)? as u64;
        if float_only {
            apply_set_seed_quiet(state, n1, 0);
            return Ok(0);
        }
        let n2 = state.opt_integer(2, 0)? as u64;
        apply_set_seed(state, n1, n2)?;
    }
    Ok(2)
}

/// Advance the PRNG in the thread-local [`RAN_STATE`] and return the raw 64-bit
/// output.
///
/// The thread-local is the only difference from the C source, which draws from
/// userdata in closure upvalue 1; the [`next_rand`] algorithm is identical (see
/// [`math_random`] for the deferred per-state migration).
fn advance_prng(_state: &mut LuaState) -> Result<u64, LuaError> {
    Ok(RAN_STATE.with(|r| next_rand(&mut r.borrow_mut().s)))
}

/// Project a raw draw into `[0, n]` using the thread-local PRNG for any
/// rejection redraws (see [`advance_prng`] for the thread-local rationale).
fn project_from_upvalue(_state: &mut LuaState, ran: u64, n: u64) -> Result<u64, LuaError> {
    Ok(RAN_STATE.with(|r| project(ran, n, &mut r.borrow_mut().s)))
}

/// Seed the PRNG from the host entropy hook (the 5.3+ auto-seed path).
///
/// The second seed word is derived deterministically from the entropy value;
/// the C source additionally mixes address entropy, which is deferred until a
/// richer host entropy API exists.
fn apply_random_seed(state: &mut LuaState) -> Result<(), LuaError> {
    let entropy = state.global().entropy_hook.map(|hook| hook()).unwrap_or(0);
    let seed1 = entropy;
    let seed2: u64 = entropy.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15;
    apply_set_seed(state, seed1, seed2)
}

/// Apply explicit seeds to the thread-local PRNG and push them onto the stack
/// (the 5.3+ `randomseed` return shape — the two seed words).
fn apply_set_seed(state: &mut LuaState, n1: u64, n2: u64) -> Result<(), LuaError> {
    RAN_STATE.with(|r| set_seed_words(&mut r.borrow_mut().s, n1, n2));
    state.push(LuaValue::Int(n1 as i64));
    state.push(LuaValue::Int(n2 as i64));
    Ok(())
}

/// Seed the PRNG without pushing the seed words onto the stack.
///
/// 5.1/5.2 `math.randomseed` returns no values, so its seeding path must not
/// push (unlike the modern [`apply_set_seed`], which returns the two words).
fn apply_set_seed_quiet(_state: &mut LuaState, n1: u64, n2: u64) {
    RAN_STATE.with(|r| set_seed_words(&mut r.borrow_mut().s, n1, n2));
}

/// Register `math.random` and `math.randomseed` on the math library table at
/// stack top, after seeding the PRNG.
///
/// C binds the PRNG state as upvalue 1 of both closures; with the PRNG in a
/// thread-local ([`RAN_STATE`]) these are registered as plain (non-closure)
/// entries instead.
fn set_rand_func(state: &mut LuaState) -> Result<(), LuaError> {
    apply_random_seed(state)?;
    state.pop_n(2);

    state.push_c_function(math_random)?;
    state.set_field(-2, b"random")?;
    state.push_c_function(math_randomseed)?;
    state.set_field(-2, b"randomseed")?;
    Ok(())
}

// ── Library registration table ────────────────────────────────────────────

static MATHLIB_FUNCS: &[(&[u8], LuaCFunction)] = &[
    (b"abs", math_abs),
    (b"acos", math_acos),
    (b"asin", math_asin),
    (b"atan", math_atan),
    (b"ceil", math_ceil),
    (b"cos", math_cos),
    (b"deg", math_deg),
    (b"exp", math_exp),
    (b"tointeger", math_toint),
    (b"floor", math_floor),
    (b"fmod", math_fmod),
    (b"ult", math_ult),
    (b"log", math_log),
    (b"max", math_max),
    (b"min", math_min),
    (b"modf", math_modf),
    (b"rad", math_rad),
    (b"sin", math_sin),
    (b"sqrt", math_sqrt),
    (b"tan", math_tan),
    (b"type", math_type),
    // `frexp`/`ldexp` survive into 5.5, unlike the rest of the compat roster:
    // they are registered unconditionally on 5.3/5.4/5.5. Verified against all
    // three reference binaries (`type(math.frexp)`/`type(math.ldexp)` ==
    // "function" on 5.3.6, 5.4.7, 5.5.0).
    (b"frexp", math_frexp),
    (b"ldexp", math_ldexp),
];

// ── Module entry point ────────────────────────────────────────────────────

/// Open the `math` library: create the table, register the version-agnostic
/// functions and the per-version roster delta, populate the constants, and wire
/// up the PRNG functions.
pub fn luaopen_math(state: &mut LuaState) -> Result<usize, LuaError> {
    state.new_lib(MATHLIB_FUNCS)?;

    // Per-version roster delta: the `LUA_COMPAT_MATHLIB`-gated functions
    // (`atan2` as an alias of `math_atan`, plus cosh/sinh/tanh/pow/log10) ship
    // in the default lua5.3.6 build (`LUA_COMPAT_MATHLIB` on) AND the default
    // lua5.4.7 build (its `LUA_COMPAT_5_3` umbrella turns `LUA_COMPAT_MATHLIB`
    // on), but were dropped in lua5.5.0 (macro commented out). Verified by
    // probing all three reference binaries directly. `frexp`/`ldexp` are NOT in
    // this set — they survive into 5.5 and live in the agnostic roster above.
    // `new_lib` leaves the new table on the stack top, so we register into it
    // directly. See `specs/followup/5.3-math.md` (whose 5.4/5.5-absence claim
    // is corrected here against the binaries, the binding oracle).
    // The `LUA_COMPAT_MATHLIB` deprecated roster also ships in the default
    // lua5.2.4 build (verified against the reference binary: `type(math.atan2)`
    // etc. == "function" on 5.2.4). 5.5 drops them.
    if matches!(
        state.global().lua_version,
        lua_types::LuaVersion::V51
            | lua_types::LuaVersion::V52
            | lua_types::LuaVersion::V53
            | lua_types::LuaVersion::V54
    ) {
        const COMPAT_MATH_FUNCS: &[(&[u8], LuaCFunction)] = &[
            (b"atan2", math_atan),
            (b"cosh", math_cosh),
            (b"sinh", math_sinh),
            (b"tanh", math_tanh),
            (b"pow", math_pow),
            (b"log10", math_log10),
        ];
        state.set_funcs_with_upvalues(COMPAT_MATH_FUNCS, 0)?;
    }

    // Lua 5.1 carries `math.mod`, a compat alias of `fmod` predating the rename
    // (`math.mod(7,3) == 1`). It was removed in 5.2. Verified against
    // lua5.1.5: `type(math.mod)` == "function". See
    // specs/followup/5.1-roster-syntax.md §1.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        state.push_c_function(math_fmod)?;
        state.set_field(-2, b"mod")?;
    }

    state.push(LuaValue::Float(PI));
    state.set_field(-2, b"pi")?;

    state.push(LuaValue::Float(f64::INFINITY));
    state.set_field(-2, b"huge")?;

    // LUA_MAXINTEGER = i64::MAX (lua_Integer is int64_t in default config).
    state.push(LuaValue::Int(i64::MAX));
    state.set_field(-2, b"maxinteger")?;

    state.push(LuaValue::Int(i64::MIN));
    state.set_field(-2, b"mininteger")?;

    // Lua 5.1/5.2 are float-only: the integer-subtype helpers (`math.type`,
    // `math.tointeger`, `math.ult`) and the integer bounds
    // (`math.maxinteger`/`mininteger`) are 5.3 additions and are absent there.
    // Verified against lua5.2.4: each is `nil`.
    if matches!(
        state.global().lua_version,
        lua_types::LuaVersion::V51 | lua_types::LuaVersion::V52
    ) {
        for field in [
            &b"type"[..],
            &b"tointeger"[..],
            &b"ult"[..],
            &b"maxinteger"[..],
            &b"mininteger"[..],
        ] {
            state.push(LuaValue::Nil);
            state.set_field(-2, field)?;
        }
    }

    set_rand_func(state)?;

    Ok(1)
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   target_crate:  lua-stdlib
//   unsafe_blocks: 0
//   deferred:      one TODO — the PRNG state is a thread-local rather than
//                  per-lua_State typed userdata in a closure upvalue; see
//                  math_random. The PRNG/ldexp/frexp/version-gate behavior is
//                  pinned by the behavioral net (multiversion_oracle PRNG
//                  sequence + subnormal tests, the lua-stdlib FloatOnly test,
//                  math.lua, and check.sh 5.1-5.5). See GRADUATED.md "math".
// ──────────────────────────────────────────────────────────────────────────
