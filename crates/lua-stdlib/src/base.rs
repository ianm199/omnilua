//! Base library — Lua's built-in functions (`print`, `type`, `pairs`, `pcall`, …).
//!
//! Translated from: `reference/lua-5.4.7/src/lbaselib.c` (549 lines, 32 functions)
//! Target crate: `lua-stdlib`

use crate::state_stub::{LuaState, LuaStateStubExt as _};
use lua_types::{closure::LuaClosure, error::LuaError, value::LuaValue, LuaStatus, LuaType};

// ── Module-level constants ────────────────────────────────────────────────────

/// ASCII whitespace characters used by `b_str2int` for strspn-style skipping.
const SPACECHARS: &[u8] = b" \x0c\n\r\t\x0b";

/// Reserved stack slot used by `generic_reader` to anchor the current chunk
/// string so it is not collected while `lua_load` is running.
const RESERVED_SLOT: i32 = 5;

/// Name of the global environment table stored as a global itself.
const LUA_GNAME: &[u8] = b"_G";

/// Sentinel indicating "all return values" for call/pcall helpers.
const LUA_MULTRET: i32 = -1;

// ── GC operation codes ────────────────────────────────────────────────────────

/// Identifies a GC control operation passed to the `collectgarbage` built-in.
/// Mirrors the `LUA_GC*` integer constants from `lua.h`.
/// TODO(port): define as a proper type in lua-types once the GC API is finalised.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GcOp {
    Stop = 0,
    Restart = 1,
    Collect = 2,
    Count = 3,
    #[expect(
        dead_code,
        reason = "ported stdlib helper; not yet wired into the runtime"
    )]
    CountB = 4,
    Step = 5,
    SetPause = 6,
    SetStepMul = 7,
    IsRunning = 9,
    Gen = 10,
    Inc = 11,
    Param = 12,
}

// ── LuaState forward declaration ─────────────────────────────────────────────

// LuaState is provided by crate::state_stub.

// ── Type alias for standard Lua-callable functions ────────────────────────────

/// Rust equivalent of `lua_CFunction`: a bare function that receives the
/// interpreter state and returns a count of pushed results.
pub(crate) type LuaLibFn = fn(&mut LuaState) -> Result<usize, LuaError>;

// ── Helper: push_mode ─────────────────────────────────────────────────────────

/// Push the GC mode string ("incremental" or "generational") onto the stack,
/// or push `nil` (fail) when `oldmode == -1` (invalid call inside a finalizer).
///
fn push_mode(state: &mut LuaState, oldmode: i32) -> Result<usize, LuaError> {
    if oldmode == -1 {
        state.push(LuaValue::Nil);
    } else {
        let s: &[u8] = if oldmode == GcOp::Inc as i32 {
            b"incremental"
        } else {
            b"generational"
        };
        state.push_string(s)?;
    }
    Ok(1)
}

// ── Helper: finish_pcall ──────────────────────────────────────────────────────

/// Shared result-adjustment logic for `pcall` and `xpcall`.
///
/// On success: returns the count of values already on the stack minus `extra`
/// skipped sentinel values.  On failure: replaces whatever is on the stack
/// with `[false, error_message]` and returns 2.
///
fn finish_pcall(state: &mut LuaState, ok: bool, extra: i32) -> Result<usize, LuaError> {
    if !ok {
        state.push(LuaValue::Bool(false));
        state.push_copy(-2)?;
        return Ok(2);
    }
    Ok((state.top() as i32 - extra) as usize)
}

// ── Helper: b_str2int ─────────────────────────────────────────────────────────

/// Parse an integer in an arbitrary base from the byte slice `s`.
///
/// Returns `Some((consumed, value))` on success, where `consumed` is the number
/// of bytes from the start of `s` that were processed (leading and trailing
/// ASCII whitespace included).  Returns `None` when the slice contains no valid
/// numeral in `base`.
///
/// The caller checks `consumed == s.len()` to verify the whole string was used.
///
fn b_str2int(s: &[u8], base: u32) -> Option<(usize, i64)> {
    let mut pos = 0usize;
    while pos < s.len() && SPACECHARS.contains(&s[pos]) {
        pos += 1;
    }
    let neg = if pos < s.len() && s[pos] == b'-' {
        pos += 1;
        true
    } else {
        if pos < s.len() && s[pos] == b'+' {
            pos += 1;
        }
        false
    };
    if pos >= s.len() || !s[pos].is_ascii_alphanumeric() {
        return None;
    }
    let mut n: u64 = 0u64;
    loop {
        let byte = s[pos];
        let digit = if byte.is_ascii_digit() {
            (byte - b'0') as u32
        } else {
            (byte.to_ascii_uppercase() - b'A') as u32 + 10
        };
        if digit >= base {
            return None;
        }
        n = n.wrapping_mul(base as u64).wrapping_add(digit as u64);
        pos += 1;
        if pos >= s.len() || !s[pos].is_ascii_alphanumeric() {
            break;
        }
    }
    while pos < s.len() && SPACECHARS.contains(&s[pos]) {
        pos += 1;
    }
    let value: i64 = if neg {
        0u64.wrapping_sub(n) as i64
    } else {
        n as i64
    };
    Some((pos, value))
}

// ── Helper: load_aux ──────────────────────────────────────────────────────────

/// Shared post-load logic for `load` and `loadfile`.
///
/// On success (status_ok == true): optionally installs an environment upvalue,
/// then returns 1 (the chunk function is on the stack).
/// On failure: pushes nil then moves it before the error message, returns 2.
///
fn load_aux(state: &mut LuaState, status_ok: bool, envidx: i32) -> Result<usize, LuaError> {
    if status_ok {
        if envidx != 0 {
            state.push_copy(envidx)?;
            if state.set_upvalue(-2, 1)?.is_none() {
                state.pop_n(1);
            }
        }
        Ok(1)
    } else {
        state.push(LuaValue::Nil);
        state.insert(-2)?;
        Ok(2)
    }
}

fn check_load_mode(state: &mut LuaState, idx: i32, default: &[u8]) -> Result<Vec<u8>, LuaError> {
    let mode = state.opt_arg_string(idx, default)?;
    if matches!(state.global().lua_version, lua_types::LuaVersion::V55) && mode.contains(&b'B') {
        return Err(lua_vm::debug::arg_error_impl(state, idx, b"invalid mode"));
    }
    Ok(mode)
}

// ── print ─────────────────────────────────────────────────────────────────────

/// Converts each argument to a string, separates them with tabs, writes them to
/// standard output, and finishes with a newline.
///
/// The conversion mechanism is a genuine cross-version split:
///
/// - Lua 5.1/5.2/5.3 `luaB_print` fetch the **global** `tostring` and *call* it
///   on each argument. Redefining global `tostring` therefore changes `print`,
///   a `nil` global makes `print` raise `attempt to call a nil value`, and a
///   result that is neither a string nor a coercible number raises
///   `'tostring' must return a string to 'print'`.
/// - Lua 5.4/5.5 `luaB_print` use `luaL_tolstring` directly: it honors the
///   `__tostring` / `__name` metafields but ignores the global `tostring`.
///
pub(crate) fn print_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let calls_global_tostring = matches!(
        state.global().lua_version,
        lua_types::LuaVersion::V51 | lua_types::LuaVersion::V52 | lua_types::LuaVersion::V53
    );
    if calls_global_tostring {
        return print_via_global_tostring(state);
    }
    let n = state.top();
    for i in 1..=n {
        // luaL_tolstring converts via tostring() metamethod, pushes result,
        // returns a pointer. In Rust we get a GcRef and use its bytes.
        let display_ref = state.to_display_string(i)?;
        if i > 1 {
            state.write_output(b"\t")?;
        }
        let bytes = display_ref.clone();
        state.write_output(&bytes)?;
        state.pop_n(1);
    }
    state.write_output(b"\n")?;
    Ok(0)
}

/// Faithful port of the Lua 5.1/5.2/5.3 `luaB_print`: fetch the global
/// `tostring` once, then call it on each argument.
///
fn print_via_global_tostring(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.top();
    lua_vm::api::get_global(state, b"tostring")?;
    for i in 1..=n {
        state.push_copy(-1)?;
        state.push_copy(i)?;
        state.call(1, 1)?;
        // lua_tolstring returns NULL for anything that is neither a string nor a
        // coercible number; the reference raises in that case.
        if !matches!(state.type_at(-1), LuaType::String | LuaType::Number) {
            return Err(state.where_error(1, b"'tostring' must return a string to 'print'"));
        }
        let bytes = state
            .to_lua_string_bytes(-1)
            .expect("string/number coerces to bytes");
        if i > 1 {
            state.write_output(b"\t")?;
        }
        state.write_output(&bytes)?;
        state.pop_n(1);
    }
    state.write_output(b"\n")?;
    Ok(0)
}

// ── warn ──────────────────────────────────────────────────────────────────────

/// Validates that every argument is a string, then forwards them as a
/// multi-part warning message via the state's warning hook.
///
pub(crate) fn warn_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.top();
    state.check_arg_string(1)?;
    for i in 2..=n {
        state.check_arg_string(i)?;
    }
    for i in 1..n {
        // Clone bytes before further mutation to avoid borrow conflict.
        // PORTING.md §8: "No &LuaValue across a stack-mutating call."
        let s: Vec<u8> = state
            .to_lua_string_bytes(i)
            .map(|b| b.to_vec())
            .unwrap_or_default();
        // continue = true (1) — more parts follow
        state.warning(&s, true)?;
    }
    let s: Vec<u8> = state
        .to_lua_string_bytes(n)
        .map(|b| b.to_vec())
        .unwrap_or_default();
    state.warning(&s, false)?;
    Ok(0)
}

// ── tonumber ──────────────────────────────────────────────────────────────────

/// Converts a value to a number, optionally in a given numeric base (2–36).
///
pub(crate) fn tonumber_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    if matches!(state.type_at(2), LuaType::None | LuaType::Nil) {
        if state.type_at(1) == LuaType::Number {
            lua_vm::api::set_top(state, 1)?;
            return Ok(1);
        }
        // lua_stringtonumber returns bytes consumed including the NUL terminator,
        // so success iff consumed == string_length + 1.
        if let Some(len) = state.to_lua_string_len(1) {
            if let Some(consumed) = state.string_to_number(1) {
                if consumed == len + 1 {
                    return Ok(1);
                }
            }
        }
        state.check_arg_any(1)?;
    } else {
        let base = state.check_arg_integer(2)?;
        state.check_arg_type(1, LuaType::String)?;
        // Clone before further state ops (PORTING.md §8).
        let bytes: Vec<u8> = state
            .to_lua_string_bytes(1)
            .map(|b| b.to_vec())
            .unwrap_or_default();
        if !(2..=36).contains(&base) {
            return Err(lua_vm::debug::arg_error_impl(
                state,
                2,
                b"base out of range",
            ));
        }
        if let Some((consumed, n)) = b_str2int(&bytes, base as u32) {
            if consumed == bytes.len() {
                state.push(LuaValue::Int(n));
                return Ok(1);
            }
        }
    }
    state.push(LuaValue::Nil);
    Ok(1)
}

// ── error ─────────────────────────────────────────────────────────────────────

/// Raises the value at stack[1] as a Lua error, optionally prepending
/// source-location information for string errors when `level > 0`.
///
pub(crate) fn error_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let level = state.opt_arg_integer(2, 1)? as i32;
    lua_vm::api::set_top(state, 1)?;
    if state.type_at(1) == LuaType::String && level > 0 {
        state.push_where(level)?;
        state.push_copy(1)?;
        state.concat(2)?;
    }
    Err(LuaError::from_value(state.pop()))
}

// ── getmetatable ──────────────────────────────────────────────────────────────

/// Returns the metatable of the first argument, or the `__metatable` field of
/// the metatable if that field exists (protecting the raw metatable).
///
pub(crate) fn getmetatable_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_any(1)?;
    if !state.get_metatable(1)? {
        state.push(LuaValue::Nil);
        return Ok(1);
    }
    // Returns LuaType::Nil if metatable has no __metatable; otherwise pushes it.
    state.get_metafield(1, b"__metatable")?;
    Ok(1)
}

// ── setmetatable ──────────────────────────────────────────────────────────────

/// Sets the metatable of the table at argument 1 to the value at argument 2
/// (nil clears it).  Raises an error if the current metatable is protected via
/// `__metatable`.
///
pub(crate) fn setmetatable_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let t = state.type_at(2);
    state.check_arg_type(1, LuaType::Table)?;
    if !(t == LuaType::Nil || t == LuaType::Table) {
        let got = state.value_at(2);
        return Err(LuaError::type_arg_error(2, "nil or table", &got));
    }
    if state.get_metafield(1, b"__metatable")? != LuaType::Nil {
        return Err(LuaError::runtime(format_args!(
            "cannot change a protected metatable"
        )));
    }
    lua_vm::api::set_top(state, 2)?;
    state.set_metatable(1)?;
    Ok(1)
}

// ── rawequal ──────────────────────────────────────────────────────────────────

/// Raw equality check (no metamethods).
///
pub(crate) fn rawequal_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_any(1)?;
    state.check_arg_any(2)?;
    let eq = state.raw_equal(1, 2)?;
    state.push(LuaValue::Bool(eq));
    Ok(1)
}

// ── rawlen ────────────────────────────────────────────────────────────────────

/// Raw length (#) without metamethods; accepts tables and strings only.
///
pub(crate) fn rawlen_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let t = state.type_at(1);
    if !(t == LuaType::Table || t == LuaType::String) {
        let got = state.value_at(1);
        return Err(LuaError::type_arg_error(1, "table or string", &got));
    }
    let len = state.raw_len(1);
    state.push(LuaValue::Int(len));
    Ok(1)
}

// ── rawget ────────────────────────────────────────────────────────────────────

/// Raw table read (no metamethods).
///
pub(crate) fn rawget_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    state.check_arg_any(2)?;
    lua_vm::api::set_top(state, 2)?;
    state.raw_get(1)?;
    Ok(1)
}

// ── rawset ────────────────────────────────────────────────────────────────────

/// Raw table write (no metamethods).
///
pub(crate) fn rawset_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    state.check_arg_any(2)?;
    state.check_arg_any(3)?;
    lua_vm::api::set_top(state, 3)?;
    state.raw_set(1)?;
    Ok(1)
}

// ── collectgarbage ────────────────────────────────────────────────────────────

/// Expose GC control to Lua scripts.  The first argument selects the operation;
/// subsequent arguments are operation-specific parameters.
///
///
/// PORT NOTE: C's `checkvalres(x)` macro breaks out of the `switch` to the
/// trailing `luaL_pushfail` when `x == -1` (called inside a finalizer).
/// In Rust we model this with an explicit early-return to the pushfail path
/// using a boolean flag, avoiding labeled blocks.
pub(crate) fn collectgarbage_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    // Explicit collections bypass the checkpoint wrappers, so the dead
    // stack slices must be cleared here before any collect dispatch
    // (C parity: traversethread's atomic clear; see #140 / GC_ROOTS.md).
    state.gc_clear_dead_stack_tails();
    // The option set is version-gated. 5.4/5.3 expose `setpause`/`setstepmul`;
    // 5.5 removed both and added `param` (lbaselib.c). The version that owns
    // the running state decides which list/mapping applies.
    let version = state.global().lua_version;
    let is_v55 = version == lua_types::LuaVersion::V55;
    // Lua 5.1's `collectgarbage` accepts only `collect/stop/restart/count/step/
    // setpause/setstepmul`; the 5.2 `isrunning`/`generational`, the 5.4
    // `incremental`, and the 5.5 `param` must be rejected with `invalid option`.
    // Verified against lua5.1.5: `collectgarbage("isrunning")` errors. (5.2 DOES
    // accept `isrunning`/`generational`, so it stays on OPTS_54.) See
    // specs/followup/5.1-roster-syntax.md §1.
    static OPTS_51: &[&[u8]] = &[
        b"stop",
        b"restart",
        b"collect",
        b"count",
        b"step",
        b"setpause",
        b"setstepmul",
    ];
    static OPTS_NUM_51: &[GcOp] = &[
        GcOp::Stop,
        GcOp::Restart,
        GcOp::Collect,
        GcOp::Count,
        GcOp::Step,
        GcOp::SetPause,
        GcOp::SetStepMul,
    ];
    static OPTS_54: &[&[u8]] = &[
        b"stop",
        b"restart",
        b"collect",
        b"count",
        b"step",
        b"setpause",
        b"setstepmul",
        b"isrunning",
        b"generational",
        b"incremental",
    ];
    static OPTS_NUM_54: &[GcOp] = &[
        GcOp::Stop,
        GcOp::Restart,
        GcOp::Collect,
        GcOp::Count,
        GcOp::Step,
        GcOp::SetPause,
        GcOp::SetStepMul,
        GcOp::IsRunning,
        GcOp::Gen,
        GcOp::Inc,
    ];
    static OPTS_55: &[&[u8]] = &[
        b"stop",
        b"restart",
        b"collect",
        b"count",
        b"step",
        b"isrunning",
        b"generational",
        b"incremental",
        b"param",
    ];
    static OPTS_NUM_55: &[GcOp] = &[
        GcOp::Stop,
        GcOp::Restart,
        GcOp::Collect,
        GcOp::Count,
        GcOp::Step,
        GcOp::IsRunning,
        GcOp::Gen,
        GcOp::Inc,
        GcOp::Param,
    ];
    let (opts, opts_num): (&[&[u8]], &[GcOp]) = if is_v55 {
        (OPTS_55, OPTS_NUM_55)
    } else if matches!(version, lua_types::LuaVersion::V51) {
        (OPTS_51, OPTS_NUM_51)
    } else {
        (OPTS_54, OPTS_NUM_54)
    };
    let idx = state.check_arg_option(1, Some(b"collect"), opts)?;
    let op = opts_num[idx];

    // Each arm either returns early on success, or evaluates to `false`
    // (meaning checkvalres fired — fall through to pushfail).
    let valid: bool = match op {
        GcOp::Count => {
            // TODO(port): gc_count / gc_count_b are stubs in Phase A.
            let k = state.gc_count()?;
            let b = state.gc_count_b()?;
            if k == -1 {
                false
            } else {
                state.push(LuaValue::Float(k as f64 + b as f64 / 1024.0));
                return Ok(1);
            }
        }
        GcOp::Step => {
            let step = state.opt_arg_integer(2, 0)? as i32;
            // TODO(port): gc_step is a stub in Phase A.
            let res = state.gc_step(step)?;
            if res == -1 {
                false
            } else {
                state.push(LuaValue::Bool(res != 0));
                return Ok(1);
            }
        }
        GcOp::SetPause | GcOp::SetStepMul => {
            let p = state.opt_arg_integer(2, 0)? as i32;
            // TODO(port): gc_set_param is a stub in Phase A.
            let previous = state.gc_set_param(op as i32, p)?;
            if previous == -1 {
                false
            } else {
                state.push(LuaValue::Int(previous as i64));
                return Ok(1);
            }
        }
        GcOp::IsRunning => {
            let res = state.gc_is_running()?;
            state.push(LuaValue::Bool(res));
            return Ok(1);
        }
        GcOp::Gen => {
            let minormul = state.opt_arg_integer(2, 0)? as i32;
            let majormul = state.opt_arg_integer(3, 0)? as i32;
            // TODO(port): gc_gen is a stub in Phase A.
            let oldmode = state.gc_gen(minormul, majormul)?;
            return push_mode(state, oldmode);
        }
        GcOp::Inc => {
            let pause = state.opt_arg_integer(2, 0)? as i32;
            let stepmul = state.opt_arg_integer(3, 0)? as i32;
            let stepsize = state.opt_arg_integer(4, 0)? as i32;
            // TODO(port): gc_inc is a stub in Phase A.
            let oldmode = state.gc_inc(pause, stepmul, stepsize)?;
            return push_mode(state, oldmode);
        }
        GcOp::Param => {
            // 5.5 collectgarbage("param", name [, value]): read or write a GC
            // parameter, always returning the OLD integer value. arg2 selects
            // the param; arg3 (default -1 = read-only) is the new value.
            static PARAMS: &[&[u8]] = &[
                b"minormul",
                b"majorminor",
                b"minormajor",
                b"pause",
                b"stepmul",
                b"stepsize",
            ];
            let pidx = state.check_arg_option(2, None, PARAMS)?;
            let value = state.opt_arg_integer(3, -1)?;
            let old = state.gc_param(pidx, value)?;
            state.push(LuaValue::Int(old));
            return Ok(1);
        }
        _ => {
            // TODO(port): gc_control_simple is a stub in Phase A.
            let res = state.gc_control_simple(op as i32)?;
            if res == -1 {
                false
            } else {
                state.push(LuaValue::Int(res as i64));
                return Ok(1);
            }
        }
    };
    debug_assert!(
        !valid,
        "valid arms return early; reaching here means checkvalres fired"
    );
    state.push(LuaValue::Nil);
    Ok(1)
}

// ── type ──────────────────────────────────────────────────────────────────────

/// Returns the type name of its argument as a string.
///
pub(crate) fn type_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let t = state.type_at(1);
    if t == LuaType::None {
        return Err(lua_vm::debug::arg_error_impl(state, 1, b"value expected"));
    }
    // Clone the bytes before the push to avoid borrow conflict with state.
    let name: Vec<u8> = state.type_name(t).to_vec();
    state.push_string(&name)?;
    Ok(1)
}

// ── getfenv / setfenv (Lua 5.1 fenv globals) ──────────────────────────────────

/// Truncate a numeric `getfenv`/`setfenv` level toward zero.
///
/// 5.1's `luaL_checkint` casts `lua_Number` to a C `int`, truncating toward
/// zero, so `getfenv(1.9)` is level 1 and `getfenv(-0.5)` is level 0. Under the
/// float-only V51 model every number arrives as a `Float`; the `Int` arm is a
/// defensive no-op. A non-number never reaches this helper.
fn fenv_level(v: &LuaValue) -> i64 {
    match v {
        LuaValue::Float(f) => f.trunc() as i64,
        LuaValue::Int(i) => *i,
        _ => 0,
    }
}

/// Resolve the function value targeted by a `getfenv`/`setfenv` first argument.
///
/// Returns the `LuaValue::Function` whose environment is being read or written.
/// `arg1` is interpreted exactly as Lua 5.1's `getfunc`/`setfunc`
/// (lbaselib.c): a function value targets that function directly; a number is a
/// stack *level* (floored toward zero), where level 1 is the function calling
/// `getfenv`/`setfenv`. Level 0 is handled by the callers (it denotes the
/// running thread's global table, not a function) and never reaches here.
///
/// Errors mirror lua5.1.5:
/// - negative level → `level must be non-negative`
/// - level past the stack → `invalid level`
/// - neither number nor function → `number expected, got <type>`
fn fenv_getfunc(state: &mut LuaState, level: i64) -> Result<LuaValue, LuaError> {
    if level < 0 {
        return Err(lua_vm::debug::arg_error_impl(
            state,
            1,
            b"level must be non-negative",
        ));
    }
    let mut ar = lua_vm::debug::LuaDebug::default();
    if !lua_vm::debug::get_stack(state, level as i32, &mut ar) {
        return Err(lua_vm::debug::arg_error_impl(state, 1, b"invalid level"));
    }
    let ci_idx = ar
        .i_ci
        .ok_or_else(|| lua_vm::debug::arg_error_impl(state, 1, b"invalid level"))?;
    let func_slot = state.get_ci(ci_idx).func;
    Ok(state.get_at(func_slot))
}

/// Index of a Lua closure's `_ENV` upvalue, by upvalue name.
///
/// The reused modern parser threads an upvalue literally named `_ENV` and
/// resolves every free (global) name through it; under V51 that upvalue *is* the
/// function environment. It is NOT always upvalue 0 — a nested closure that
/// captures locals places those first, with `_ENV` at a later index — so it must
/// be located by name, not position. A closure that references no free names has
/// no `_ENV` upvalue and returns `None`.
fn fenv_env_upval_index(
    lcl: &lua_types::gc::GcRef<lua_types::closure::LuaLClosure>,
) -> Option<usize> {
    lcl.proto
        .upvalues
        .iter()
        .position(|ud| ud.name.as_ref().map(|s| s.as_bytes()) == Some(b"_ENV"))
}

/// Read the environment of a resolved function value.
///
/// A Lua closure's environment is its `_ENV` upvalue. A C/Rust function (or a
/// Lua closure that references no globals, hence has no `_ENV` upvalue) is given
/// the thread global table as its environment — matching the common 5.1 case
/// and the documented `LUA_ENVIRONINDEX` gap (specs/followup/5.1-fenv.md §4).
fn fenv_read(state: &LuaState, func: &LuaValue) -> LuaValue {
    if let LuaValue::Function(LuaClosure::Lua(lcl)) = func {
        if let Some(idx) = fenv_env_upval_index(lcl) {
            return state.upvalue_get(lcl, idx);
        }
    }
    state.global().globals.clone()
}

/// `getfenv([f])` — Lua 5.1 only.
///
/// Returns the environment of the function `f` (a function value or a stack
/// level), or the running function's environment when the argument is absent or
/// `1`. Level `0` returns the running thread's global table. See
/// `specs/followup/5.1-fenv.md` §2.
pub(crate) fn getfenv_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let arg1 = state.value_at(1);
    let func = match &arg1 {
        LuaValue::Function(_) => arg1.clone(),
        LuaValue::Nil if state.type_at(1) == LuaType::None => {
            // No argument => level 1 (the running function).
            fenv_getfunc(state, 1)?
        }
        LuaValue::Float(_) | LuaValue::Int(_) => {
            let level = fenv_level(&arg1);
            if level == 0 {
                let g = state.global().globals.clone();
                state.push(g);
                return Ok(1);
            }
            fenv_getfunc(state, level)?
        }
        other => {
            let got = state.obj_type_name(other);
            let msg = format!("number expected, got {}", String::from_utf8_lossy(&got));
            return Err(lua_vm::debug::arg_error_impl(state, 1, msg.as_bytes()));
        }
    };
    let env = fenv_read(state, &func);
    state.push(env);
    Ok(1)
}

/// `setfenv(f, table)` — Lua 5.1 only.
///
/// Sets the environment of the function `f` (a function value or a stack level)
/// to `table`. `setfenv(0, t)` sets the running thread's global table. Returns
/// the affected function (or the running thread for level 0). A C/Rust function
/// (or any non-Lua object) cannot have its environment changed and raises,
/// matching lua5.1.5. See `specs/followup/5.1-fenv.md` §2.
pub(crate) fn setfenv_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(2, LuaType::Table)?;
    let new_env = state.value_at(2);

    let arg1 = state.value_at(1);
    let is_level_zero =
        matches!(&arg1, LuaValue::Int(0)) || matches!(&arg1, LuaValue::Float(f) if *f == 0.0);
    if is_level_zero {
        // Level 0: replace the running thread's global table and return the
        // running thread. Subsequently-loaded top-level chunks take this env.
        state.global_mut().globals = new_env;
        lua_vm::api::push_thread(state);
        return Ok(1);
    }

    let func = match &arg1 {
        LuaValue::Function(_) => arg1.clone(),
        LuaValue::Float(_) | LuaValue::Int(_) => {
            let level = fenv_level(&arg1);
            fenv_getfunc(state, level)?
        }
        other => {
            let got = state.obj_type_name(other);
            let msg = format!("number expected, got {}", String::from_utf8_lossy(&got));
            return Err(lua_vm::debug::arg_error_impl(state, 1, msg.as_bytes()));
        }
    };

    match &func {
        LuaValue::Function(LuaClosure::Lua(lcl)) => {
            if let Some(idx) = fenv_env_upval_index(lcl) {
                // Give the closure a PRIVATE environment: replace its `_ENV`
                // upvalue *cell* with a fresh closed upvalue holding `new_env`.
                // Mutating the existing cell's value (`upvalue_set`) would alter
                // every closure sharing that upvalue (e.g. the main chunk's
                // `_G`), which is wrong — `setfenv(f, e)` must not change the
                // caller's globals. A new cell isolates `f`.
                let uv = state.new_upval_closed(new_env);
                lcl.set_upval(idx, uv);
                state.gc().obj_barrier(lcl, &uv);
            }
            // A Lua closure that references no globals has no `_ENV` upvalue and
            // nothing reads globals through it, so the set is inert; 5.1 still
            // accepts it and returns the function. (Gap: a subsequent
            // `getfenv` on such a closure returns the thread globals rather than
            // the set table — see specs/followup/5.1-fenv.md §4.)
        }
        _ => {
            // C/Rust functions cannot have their environment changed. 5.1
            // raises this exact message (via luaL_error, so it carries the
            // caller's source location) for any object whose env is fixed.
            return Err(
                state.where_error(1, b"'setfenv' cannot change environment of given object")
            );
        }
    }
    state.push(func);
    Ok(1)
}

/// Set the environment of the Lua closure `level` frames up the running stack
/// to `new_env`, the internal equivalent of `setfenv(level, new_env)`.
///
/// Used by `module` (5.1 `package` library), which sets its caller's
/// environment to the module table. A non-Lua function (or a closure with no
/// `_ENV` upvalue) is left unchanged, matching the inert-set behavior of
/// `setfenv`. See specs/followup/5.1-fenv.md.
pub(crate) fn set_func_env_at_level(
    state: &mut LuaState,
    level: i64,
    new_env: LuaValue,
) -> Result<(), LuaError> {
    let func = fenv_getfunc(state, level)?;
    if let LuaValue::Function(LuaClosure::Lua(lcl)) = &func {
        if let Some(idx) = fenv_env_upval_index(lcl) {
            let uv = state.new_upval_closed(new_env);
            lcl.set_upval(idx, uv);
            state.gc().obj_barrier(lcl, &uv);
        }
    }
    Ok(())
}

// ── next ──────────────────────────────────────────────────────────────────────

/// Table traversal iterator: given a table and a key, pushes the next key-value
/// pair.  Pushes nil and returns 1 when the traversal is exhausted.
///
pub(crate) fn next_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    lua_vm::api::set_top(state, 2)?;
    if state.table_next(1)? {
        Ok(2)
    } else {
        state.push(LuaValue::Nil);
        Ok(1)
    }
}

// ── pairs continuation (coroutine stub) ───────────────────────────────────────

/// Continuation for `pairs` when the `__pairs` metamethod yields.
/// Re-invoked by `finishCcall` after the yielded `__pairs` resumes.
///
fn pairs_cont(state: &mut LuaState, _status: i32, _ctx: isize) -> Result<usize, LuaError> {
    if state.global().lua_version == lua_types::LuaVersion::V55 {
        Ok(4)
    } else {
        Ok(3)
    }
}

// ── pairs ─────────────────────────────────────────────────────────────────────

/// Returns the `next` function, the table, and nil (or invokes a `__pairs`
/// metamethod).
///
pub(crate) fn pairs_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_any(1)?;
    // Lua 5.1 has no `__pairs` metamethod; `pairs(t)` always iterates the raw
    // table even when a `__pairs` is set (it is silently ignored). Lua 5.5
    // extends the result list with a fourth to-be-closed object.
    let consult_pairs_tm = !matches!(state.global().lua_version, lua_types::LuaVersion::V51);
    let nresults = if state.global().lua_version == lua_types::LuaVersion::V55 {
        4
    } else {
        3
    };
    if !consult_pairs_tm || state.get_metafield(1, b"__pairs")? == LuaType::Nil {
        state.push_c_function(next_fn)?;
        state.push_copy(1)?;
        state.push(LuaValue::Nil);
        if nresults == 4 {
            state.push(LuaValue::Nil);
        }
    } else {
        state.push_copy(1)?;
        state.call_k(1, nresults as i32, 0, Some(pairs_cont))?;
    }
    Ok(nresults)
}

// ── ipairs auxiliary ──────────────────────────────────────────────────────────

/// Iterator step function for `ipairs`: increments the counter and fetches
/// the next array element.  Returns the index + value, or just the index when
/// the value is nil (signalling end-of-iteration).
///
fn ipairs_aux(state: &mut LuaState) -> Result<usize, LuaError> {
    let i = match lua_vm::api::positive_index_value(state, 2) {
        LuaValue::Int(i) => i,
        _ => state.check_arg_integer(2)?,
    };
    // luaL_intop(+, a, b) → wrapping integer addition (PORTING.md §9 / macros.tsv `intop`)
    let i = (i as u64).wrapping_add(1u64) as i64;
    state.push(LuaValue::Int(i));
    let table = lua_vm::api::positive_index_value(state, 1);
    let t = state.table_get_i_value(&table, i)?;
    if t == LuaType::Nil {
        Ok(1)
    } else {
        Ok(2)
    }
}

// ── ipairs ────────────────────────────────────────────────────────────────────

/// Returns the `ipairsaux` iterator, the table, and 0 as the initial counter.
///
pub(crate) fn ipairs_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_any(1)?;
    state.push_c_function(ipairs_aux)?;
    state.push_copy(1)?;
    state.push(LuaValue::Int(0));
    Ok(3)
}

// ── loadfile ──────────────────────────────────────────────────────────────────

/// Loads a Lua chunk from a file.
///
pub(crate) fn loadfile_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let fname: Option<Vec<u8>> = state.opt_arg_lstring(1, None)?;
    let mode: Option<Vec<u8>> = if state.is_none_or_nil(2) {
        None
    } else {
        Some(check_load_mode(state, 2, b"bt")?)
    };
    let env = if state.type_at(3) != LuaType::None {
        3
    } else {
        0
    };
    let status_ok = state.load_file_ex(fname.as_deref(), mode.as_deref())?;
    load_aux(state, status_ok, env)
}

// ── generic_reader ────────────────────────────────────────────────────────────

/// Reader callback for `luaB_load` when the chunk source is a Lua function.
/// Calls the function at stack[1] repeatedly to obtain successive chunks.
///
///
/// PORT NOTE: In C this is a `lua_Reader` function pointer passed to
/// `lua_load`. In Rust, readers are closures — but `generic_reader` itself
/// needs `&mut LuaState`, which conflicts with `state.load_with_reader`'s
/// own borrow.  The current translation materialises the reader as a free
/// function for documentation purposes; Phase B must resolve the design
/// (e.g., a separate reader-context type, or a split between "advance reader"
/// and "run Lua call" phases).
/// TODO(port): generic_reader — self-referential &mut borrow when used as lua_load callback.
fn generic_reader(state: &mut LuaState) -> Result<Option<Vec<u8>>, LuaError> {
    state.ensure_stack(2, b"too many nested functions")?;
    state.push_copy(1)?;
    state.call(0, 1)?;
    if state.type_at(-1) == LuaType::Nil {
        state.pop_n(1);
        return Ok(None);
    }
    //      luaL_error(L, "reader function must return a string");
    // lua_isstring in C is true for strings AND coercible numbers.
    if !matches!(state.type_at(-1), LuaType::String | LuaType::Number) {
        return Err(LuaError::runtime(format_args!(
            "reader function must return a string"
        )));
    }
    state.replace(RESERVED_SLOT)?;
    let bytes = state.to_lua_string_bytes(RESERVED_SLOT).map(|b| b.to_vec());
    Ok(bytes)
}

// ── load ──────────────────────────────────────────────────────────────────────

/// Loads a Lua chunk from a string or a reader function.
///
pub(crate) fn load_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    // Lua 5.1's `load` takes a *reader function only* — string loading is
    // `loadstring`'s job. `load("...")` errors with `function expected, got
    // string`. The string-or-function overload is a 5.2 addition. Verified
    // against lua5.1.5; see specs/followup/5.1-roster-syntax.md §1.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        state.check_arg_type(1, LuaType::Function)?;
    }
    // Determine whether argument 1 is a string (load from buffer) or a
    // function (load from reader).
    let is_string = matches!(state.type_at(1), LuaType::String | LuaType::Number);
    let mode: Vec<u8> = check_load_mode(state, 3, b"bt")?;
    let env = if state.type_at(4) != LuaType::None {
        4
    } else {
        0
    };
    let status_ok = if is_string {
        let chunk: Vec<u8> = state.to_lua_string_bytes(1).unwrap_or_default();
        let chunkname: Vec<u8> = if state.is_none_or_nil(2) {
            chunk.clone()
        } else {
            state.check_arg_string(2)?
        };
        state.load_buffer_ex(&chunk, &chunkname, &mode)?
    } else {
        let chunkname: Vec<u8> = state
            .opt_arg_string_bytes(2)
            .unwrap_or_else(|_| b"=(load)".to_vec());
        state.check_arg_type(1, LuaType::Function)?;
        lua_vm::api::set_top(state, RESERVED_SLOT)?;
        // TODO(port): generic_reader cannot be passed directly due to self-referential
        // &mut borrow — see generic_reader's PORT NOTE. Phase B resolves this.
        state.load_with_reader(generic_reader, &chunkname, &mode)?
    };
    load_aux(state, status_ok, env)
}

/// `loadstring(s [, chunkname])` — Lua 5.1 only.
///
/// Loads a string as a Lua chunk. In 5.1 this is the string-loading counterpart
/// to `load` (which takes a reader function only). The second argument is the
/// chunk name. Verified against lua5.1.5; see
/// specs/followup/5.1-roster-syntax.md §1.
pub(crate) fn loadstring_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let chunk: Vec<u8> = state.check_arg_string(1)?;
    let chunkname: Vec<u8> = if state.is_none_or_nil(2) {
        chunk.clone()
    } else {
        state.check_arg_string(2)?
    };
    let status_ok = state.load_buffer_ex(&chunk, &chunkname, b"bt")?;
    load_aux(state, status_ok, 0)
}

/// `gcinfo()` — Lua 5.1 only. Returns the amount of memory in use by Lua, in
/// kilobytes. A deprecated holdover of `collectgarbage("count")` that returns
/// just the integer KB count. Verified against lua5.1.5: returns a number. See
/// specs/followup/5.1-roster-syntax.md §1.
pub(crate) fn gcinfo_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let k = state.gc_count()?;
    state.push(LuaValue::Int(k as i64));
    Ok(1)
}

/// `newproxy([boolean | proxy])` — Lua 5.1 only.
///
/// Creates a zero-size userdata (a "proxy"). With no argument or `false`, the
/// proxy has no metatable. With `true`, it gets a fresh empty metatable (so a
/// host can install `__gc`/`__len`, the userdata idiom these metamethods need
/// in 5.1). With another proxy, it shares that proxy's metatable. Mirrors
/// `luaB_newproxy` in 5.1 `lbaselib.c`; see specs/followup/5.1-roster-syntax.md
/// §1. The C version validates the proxy argument against a weak table of
/// metatables it created; this port instead accepts any userdata that carries a
/// metatable, which is observably equivalent for the proxy idiom.
pub(crate) fn newproxy_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    lua_vm::api::set_top(state, 1)?;
    // The new userdata is pushed at stack position 2.
    state.new_userdata_typed(b"", 0, 0)?;
    if !state.to_boolean(1) {
        return Ok(1); // no metatable
    }
    if matches!(state.type_at(1), LuaType::Boolean) {
        // `true`: create and attach a fresh empty metatable.
        let mt = state.new_table();
        state.push(LuaValue::Table(mt));
        state.set_metatable(2)?;
    } else {
        // A proxy argument: share its metatable. Validate it is a userdata that
        // carries one (the C version checks a weak table of valid metatables).
        let is_proxy = matches!(state.type_at(1), LuaType::UserData) && state.get_metatable(1)?;
        if !is_proxy {
            return Err(lua_vm::debug::arg_error_impl(
                state,
                1,
                b"boolean or proxy expected",
            ));
        }
        // get_metatable pushed arg1's metatable on top; attach it to the proxy.
        state.set_metatable(2)?;
    }
    Ok(1)
}

// ── dofile ────────────────────────────────────────────────────────────────────

/// Loads and runs a Lua file, forwarding all return values.
///
fn dofile_cont(state: &mut LuaState, _status: i32, _ctx: isize) -> Result<usize, LuaError> {
    Ok((state.top() as i32 - 1) as usize)
}

pub(crate) fn dofile_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let fname: Option<Vec<u8>> = state.opt_arg_lstring(1, None)?;
    lua_vm::api::set_top(state, 1)?;
    if !state.load_file(fname.as_deref())? {
        return Err(LuaError::from_value(state.pop()));
    }
    state.call_k(0, LUA_MULTRET, 0, Some(dofile_cont))?;
    dofile_cont(state, 0, 0)
}

// ── assert ────────────────────────────────────────────────────────────────────

/// Raises an error if the first argument is falsy, otherwise passes all
/// arguments through as return values.
///
pub(crate) fn assert_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    if state.to_boolean(1) {
        return Ok(state.top() as usize);
    }
    state.check_arg_any(1)?;
    state.remove(1)?;
    state.push_string(b"assertion failed!")?;
    lua_vm::api::set_top(state, 1)?;
    error_fn(state)
}

// ── select ────────────────────────────────────────────────────────────────────

/// Returns a slice of its arguments starting at the given index, or returns
/// the count of arguments when called with `"#"`.
///
pub(crate) fn select_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.top() as i64;
    // Check for '#' first byte without holding a borrow across subsequent ops.
    let first_is_hash = state.type_at(1) == LuaType::String && {
        state
            .to_lua_string_bytes(1)
            .and_then(|b| b.first().copied())
            == Some(b'#')
    };
    if first_is_hash {
        state.push(LuaValue::Int(n - 1));
        return Ok(1);
    }
    let mut i = state.check_arg_integer(1)?;
    if i < 0 {
        i = n + i;
    } else if i > n {
        i = n;
    }
    if i < 1 {
        return Err(lua_vm::debug::arg_error_impl(
            state,
            1,
            b"index out of range",
        ));
    }
    // The values at stack positions [i+1 .. n] are already in place; the
    // runtime picks up the top (n - i) of them as results.
    Ok((n - i) as usize)
}

// ── pcall ─────────────────────────────────────────────────────────────────────

/// Protected call: returns true + results on success, or false + error on
/// failure.
///
pub(crate) fn pcall_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_any(1)?;
    // Stack before: [f, a1, …, aN]
    // Stack after:  [true, f, a1, …, aN]
    state.push(LuaValue::Bool(true));
    state.insert(1)?;
    // nargs = gettop - 2 (subtract the sentinel `true` and the function).
    let nargs = state.top() as i32 - 2;
    let yieldable = state.is_yieldable();
    let ok = match state.protected_call_k(nargs, LUA_MULTRET, 0, 0, Some(finish_pcall_k)) {
        Ok(()) => true,
        // `LuaError::Yield` must bubble up to `lua_resume` so the continuation
        // saved on this frame can be invoked on resume.
        Err(LuaError::Yield) => return Err(LuaError::Yield),
        // A sandbox budget trip is uncatchable: re-raise instead of catching so
        // untrusted code cannot defeat the budget with `while true do pcall(..) end`.
        Err(e) if state.sandbox_aborting() => return Err(e),
        Err(e) if yieldable => return Err(e),
        Err(e) => {
            state.push(e.into_value());
            false
        }
    };
    finish_pcall(state, ok, 0)
}

/// Continuation matching `LuaKFunction`. Invoked by `finishCcall` on the
/// resume path after a yield through pcall (or after a `__close` ran during
/// pcall error recovery).
///
fn finish_pcall_k(state: &mut LuaState, status: i32, extra: isize) -> Result<usize, LuaError> {
    let ok = status == LuaStatus::Ok as i32 || status == LuaStatus::Yield as i32;
    finish_pcall(state, ok, extra as i32)
}

// ── xpcall ────────────────────────────────────────────────────────────────────

/// Protected call with a separate error-handler function.
///
pub(crate) fn xpcall_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    // Lua 5.1's `xpcall(f, h)` does NOT forward extra arguments to `f` — `f` is
    // always called with zero arguments. The extra-argument forwarding is a 5.2
    // addition. Verified against lua5.1.5: `xpcall(fn, h, 1,2,3)` calls `fn`
    // with `select("#",...) == 0`. Drop any args past the handler. See
    // specs/followup/5.1-roster-syntax.md §1.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) && state.top() > 2 {
        lua_vm::api::set_top(state, 2)?;
    }
    let n = state.top() as i32;
    state.check_arg_type(2, LuaType::Function)?;
    // Stack before rotate: [f, err, a1, …, aN, true, f]
    // Stack after rotate:  [f, err, true, f, a1, …, aN]
    state.push(LuaValue::Bool(true));
    state.push_copy(1)?;
    state.rotate(3, 2)?;
    // errfunc is at stack index 2; extra=2 means finishpcall skips 2 values.
    let yieldable = state.is_yieldable();
    let ok = match state.protected_call_k(n - 2, LUA_MULTRET, 2, 2, Some(finish_pcall_k)) {
        Ok(()) => true,
        Err(LuaError::Yield) => return Err(LuaError::Yield),
        // Uncatchable sandbox abort: re-raise without running the message
        // handler, so an `xpcall` handler can neither swallow nor loop on it.
        Err(e) if state.sandbox_aborting() => return Err(e),
        Err(e) if yieldable => return Err(e),
        Err(e) => {
            state.push(e.into_value());
            false
        }
    };
    finish_pcall(state, ok, 2)
}

// ── tostring ──────────────────────────────────────────────────────────────────

/// Converts any value to its string representation (calls `__tostring` if
/// present).
///
pub(crate) fn tostring_fn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_any(1)?;
    // to_display_string pushes the converted string and returns a handle to it.
    // TODO(port): to_display_string method needs implementing on LuaState.
    state.to_display_string(1)?;
    Ok(1)
}

// ── Registration table ────────────────────────────────────────────────────────

/// All base-library functions registered into the global table by `open`.
///
///
/// PORT NOTE: The C table includes placeholder entries
/// `{LUA_GNAME, NULL}` and `{"_VERSION", NULL}` that `luaopen_base` fills in
/// separately.  Those are omitted here; `open()` sets them explicitly.
pub(crate) const BASE_FUNCS: &[(&[u8], LuaLibFn)] = &[
    (b"assert", assert_fn),
    (b"collectgarbage", collectgarbage_fn),
    (b"dofile", dofile_fn),
    (b"error", error_fn),
    (b"getmetatable", getmetatable_fn),
    (b"ipairs", ipairs_fn),
    (b"loadfile", loadfile_fn),
    (b"load", load_fn),
    (b"next", next_fn),
    (b"pairs", pairs_fn),
    (b"pcall", pcall_fn),
    (b"print", print_fn),
    (b"warn", warn_fn),
    (b"rawequal", rawequal_fn),
    (b"rawlen", rawlen_fn),
    (b"rawget", rawget_fn),
    (b"rawset", rawset_fn),
    (b"select", select_fn),
    (b"setmetatable", setmetatable_fn),
    (b"tonumber", tonumber_fn),
    (b"tostring", tostring_fn),
    (b"type", type_fn),
    (b"xpcall", xpcall_fn),
];

// ── Module opener ─────────────────────────────────────────────────────────────

/// Open the base library: register all base functions into the global table,
/// then set `_G` (a self-reference) and `_VERSION`.
///
pub fn open(state: &mut LuaState) -> Result<usize, LuaError> {
    state.push_globals()?;
    state.set_funcs(BASE_FUNCS, 0)?;
    state.push_copy(-1)?;
    state.set_field(-2, LUA_GNAME)?;
    let version_str = state.global().lua_version.version_str();
    state.push_string(version_str.as_bytes())?;
    state.set_field(-2, b"_VERSION")?;
    // `warn` was introduced in Lua 5.4; it is absent on 5.1/5.2/5.3.
    if matches!(
        state.global().lua_version,
        lua_types::LuaVersion::V51 | lua_types::LuaVersion::V52 | lua_types::LuaVersion::V53
    ) {
        state.push(LuaValue::Nil);
        state.set_field(-2, b"warn")?;
    }
    // Lua 5.1/5.2 carry two globals that were removed in 5.3: `unpack` (an alias
    // of `table.unpack`) and `loadstring` (an alias of `load`). Verified against
    // lua5.2.4: both are functions. The base table is on the stack top here.
    if matches!(
        state.global().lua_version,
        lua_types::LuaVersion::V51 | lua_types::LuaVersion::V52
    ) {
        state.push_c_function(crate::table_lib::unpack)?;
        state.set_field(-2, b"unpack")?;
    }
    // `loadstring` aliases `load` in 5.2 (whose `load` accepts a string), but in
    // 5.1 `load` is reader-only, so `loadstring` is a distinct string-loader.
    // Both are absent in 5.3+. See specs/followup/5.1-roster-syntax.md §1.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V52) {
        state.push_c_function(load_fn)?;
        state.set_field(-2, b"loadstring")?;
    }
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        state.push_c_function(loadstring_fn)?;
        state.set_field(-2, b"loadstring")?;
        // `gcinfo()` and `newproxy()` are 5.1 holdovers absent in 5.2+.
        state.push_c_function(gcinfo_fn)?;
        state.set_field(-2, b"gcinfo")?;
        state.push_c_function(newproxy_fn)?;
        state.set_field(-2, b"newproxy")?;
        // `rawlen` is a Lua 5.2 addition; it is absent in 5.1. Verified against
        // lua5.1.5: `type(rawlen)` == "nil". It lives in BASE_FUNCS (registered
        // for every version), so withhold it under V51.
        state.push(LuaValue::Nil);
        state.set_field(-2, b"rawlen")?;
    }
    // Lua 5.1's fenv-based globals model: `getfenv`/`setfenv` read and write a
    // function's environment (its `_ENV` upvalue under the reused modern core)
    // or the running thread's global table for level 0. Both were removed in
    // 5.2 (which switched to lexical `_ENV`), so they are V51-only. See
    // specs/followup/5.1-fenv.md.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        state.push_c_function(getfenv_fn)?;
        state.set_field(-2, b"getfenv")?;
        state.push_c_function(setfenv_fn)?;
        state.set_field(-2, b"setfenv")?;
    }
    Ok(1)
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lbaselib.c  (549 lines, 32 functions)
//   target_crate:  lua-stdlib
//   confidence:    medium
//   todos:         21
//   port_notes:    5
//   unsafe_blocks: 0
//   notes:         All 32 C functions translated.  Main uncertainties are (1)
//                  LuaState method signatures (top/type_at/push/… — resolved
//                  in Phase B when lua-vm is compiled), (2) generic_reader's
//                  self-referential &mut borrow needs architectural resolution,
//                  (3) GC API stubs (gc_count, gc_step, …) need Phase D
//                  implementations, (4) I/O host capabilities now route through
//                  state/global hooks, but stdin/env/time/temp remain incomplete,
//                  (5) pcallk / callk continuations are
//                  stubbed pending coroutine support in Phase E.  The fake
//                  `struct LuaState;` placeholder here avoids duplicate-definition
//                  errors while keeping the file self-contained; Phase B removes it.
// ──────────────────────────────────────────────────────────────────────────────
