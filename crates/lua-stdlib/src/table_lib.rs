//! Rust port of `ltablib.c` — Lua `table` standard library.
//!
//! Provides: `table.concat`, `table.insert`, `table.move`, `table.pack`,
//! `table.remove`, `table.sort`, `table.unpack`.
//!
//! C source: `reference/lua-5.4.7/src/ltablib.c` (430 lines, 14 functions)

use crate::state_stub::{CompareOp, LuaState, LuaStateStubExt as _};
use lua_types::{GcRef, LuaError, LuaTable, LuaType, LuaValue};
use lua_vm::state::LuaTableRefExt as _;

// ─── Operation flags ──────────────────────────────────────────────────────────
const TAB_R: u32 = 1;
const TAB_W: u32 = 2;
const TAB_L: u32 = 4;
const TAB_RW: u32 = TAB_R | TAB_W;

const RANLIMIT: u32 = 100;

type IdxT = u32;

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Raw-gets `key` from the table currently sitting at stack depth `n` (a
/// metatable, in practice); returns `true` if the looked-up value is not nil.
/// Pushes `key`, then `raw_get(-n)` replaces it with the result in place.
fn check_field(state: &mut LuaState, key: &[u8], n: i32) -> Result<bool, LuaError> {
    state.push_string(key)?;
    // raw_get(-n): looks up MT[key] (MT is at -n after the key push), replaces key with value
    let ty = state.raw_get(-n)?;
    Ok(ty != LuaType::Nil)
}

/// Accepts `arg` if it is a table, or a non-table that carries a metatable with
/// every metamethod `what` requires (`TAB_R` → `__index`, `TAB_W` →
/// `__newindex`, `TAB_L` → `__len`). The fields are checked left-to-right and
/// short-circuit: a missing field stops the scan and falls through to the
/// table-type error. Otherwise raises "table expected".
///
/// DEFERRED (behaviorally inert): on the failure path the metatable and any
/// field values pushed during the scan are not popped before raising. In C the
/// `longjmp` unwinds them; here the `LuaError` propagates and the call frame is
/// torn down, so no observable behavior differs — but the stack is left dirty
/// on that path. A tidy fix would pop `n` before `check_arg_type`.
fn check_tab(state: &mut LuaState, arg: i32, what: u32) -> Result<(), LuaError> {
    if state.type_at(arg) == LuaType::Table {
        return Ok(());
    }
    // `n` tracks how many items have been pushed (MT + checked field values).
    let mut n: i32 = 1;
    let has_mt = state.get_metatable(arg)?;
    let mut ok = has_mt;

    // Short-circuit: each field is only checked if all previous checks passed.
    if ok && (what & TAB_R) != 0 {
        n += 1;
        ok = check_field(state, b"__index", n)?;
    }
    if ok && (what & TAB_W) != 0 {
        n += 1;
        ok = check_field(state, b"__newindex", n)?;
    }
    if ok && (what & TAB_L) != 0 {
        n += 1;
        ok = check_field(state, b"__len", n)?;
    }

    if ok {
        state.pop_n(n as usize);
        Ok(())
    } else {
        state.check_arg_type(arg, LuaType::Table)
    }
}

/// Check that argument `n` is a table (or table-like per `w`) and return its
/// length (the `#` border). This is the shared front-door for every table
/// function that needs a length. (Ports C's `aux_getn`.)
fn check_table_and_get_len(state: &mut LuaState, n: i32, w: u32) -> Result<i64, LuaError> {
    check_tab(state, n, w | TAB_L)?;
    state.length_at(n)
}

#[inline]
fn plain_table_at(state: &mut LuaState, idx: i32) -> Option<GcRef<LuaTable>> {
    match state.value_at(idx) {
        LuaValue::Table(tbl) if tbl.metatable().is_none() => Some(tbl),
        _ => None,
    }
}

#[inline]
fn raw_set_int(
    state: &mut LuaState,
    tbl: GcRef<LuaTable>,
    key: i64,
    value: LuaValue,
) -> Result<(), LuaError> {
    state.gc_table_barrier_back(&tbl, &value);
    tbl.raw_set_int(state, key, value)
}

// ─── table.insert ─────────────────────────────────────────────────────────────

/// `table.insert(t, [pos,] v)`. With two args, appends `v` at border+1. With
/// three, inserts `v` at `pos` (1 <= pos <= border+1) after shifting the tail
/// up by one; any other arity raises "wrong number of arguments to 'insert'".
/// The bounds check uses a wrapping unsigned subtract so `pos <= 0` is rejected
/// alongside `pos > border+1`. Note `border` here is `#t`, which honors a
/// `__len` metamethod on 5.2+ and uses the primitive length on 5.1.
pub fn insert(state: &mut LuaState) -> Result<usize, LuaError> {
    let mut e = check_table_and_get_len(state, 1, TAB_RW)?;
    e = (e as u64).wrapping_add(1) as i64;
    let plain_table = plain_table_at(state, 1);

    let pos: i64 = match state.get_top() {
        2 => {
            if let Some(tbl) = plain_table {
                let value = state.value_at(2);
                raw_set_int(state, tbl, e, value)?;
                state.pop_n(1);
                return Ok(0);
            }
            e
        }
        3 => {
            let pos = state.check_arg_integer(2)?;
            // Checks 1 <= pos <= e (wrapping subtraction catches pos <= 0)
            if !((pos as u64).wrapping_sub(1) < (e as u64)) {
                return Err(lua_vm::debug::arg_error_impl(
                    state,
                    2,
                    b"position out of bounds",
                ));
            }
            if let Some(tbl) = plain_table {
                let value = state.value_at(3);
                let mut i = e;
                while i > pos {
                    let shifted = tbl.get_int(i - 1);
                    raw_set_int(state, tbl, i, shifted)?;
                    i -= 1;
                }
                raw_set_int(state, tbl, pos, value)?;
                state.pop_n(1);
                return Ok(0);
            }
            // Cache the table once to avoid re-resolving stack slot 1 on every
            // iteration of the shift loop. C's lua_geti is a single pointer
            // arithmetic operation; our index_to_value is a function call with
            // branches, so this saves ~2N index resolutions for shift count N.
            let tbl = state.value_at(1);
            let mut i = e;
            while i > pos {
                state.table_get_i_value(&tbl, i - 1)?;
                state.table_set_i_value(&tbl, i)?;
                i -= 1;
            }
            pos
        }
        _ => {
            return Err(LuaError::runtime(format_args!(
                "wrong number of arguments to 'insert'"
            )));
        }
    };
    state.table_set_i(1, pos)?;
    Ok(0)
}

// ─── table.remove ─────────────────────────────────────────────────────────────

/// `table.remove(t, [pos])`. Removes and returns `t[pos]` (default: the last
/// element, `#t`), shifting the tail down to close the gap.
///
/// The out-of-bounds handling is gated three ways across versions, each pinned
/// against its reference binary by `v_remove_out_of_bounds_arg_gate_crossversion`:
///
/// - **5.1** (legacy `ltablib.c`): there is NO `luaL_argcheck`. An out-of-range
///   `pos` (outside `[1, size]`) silently removes nothing and returns ZERO
///   results — never an error.
/// - **5.2 / 5.3**: `luaL_argcheck((lua_Unsigned)pos - 1u <= size, 1, ...)` —
///   the offending argument is reported as **#1**.
/// - **5.4 / 5.5**: the identical check, but the argument index is **#2**.
///
/// ```c
/// // 5.4.7
/// static int tremove (lua_State *L) {
///   lua_Integer size = aux_getn(L, 1, TAB_RW);
///   lua_Integer pos = luaL_optinteger(L, 2, size);
///   if (pos != size)
///     luaL_argcheck(L, (lua_Unsigned)pos - 1u <= (lua_Unsigned)size, 2,
///                      "position out of bounds");
///   lua_geti(L, 1, pos);
///   for ( ; pos < size; pos++) {
///     lua_geti(L, 1, pos + 1);
///     lua_seti(L, 1, pos);
///   }
///   lua_pushnil(L);
///   lua_seti(L, 1, pos);
///   return 1;
/// }
/// // 5.1.5
/// static int tremove (lua_State *L) {
///   int e = aux_getn(L, 1);
///   int pos = luaL_optint(L, 2, e);
///   if (!(1 <= pos && pos <= e)) return 0;  // nothing to remove
///   ...
/// }
/// ```
pub fn remove(state: &mut LuaState) -> Result<usize, LuaError> {
    let size = check_table_and_get_len(state, 1, TAB_RW)?;
    let mut pos = state.opt_arg_integer(2, size)?;
    if state.global().lua_version == lua_types::LuaVersion::V51 {
        if !(1 <= pos && pos <= size) {
            return Ok(0);
        }
    } else if pos != size {
        if !((pos as u64).wrapping_sub(1) <= (size as u64)) {
            let argn = match state.global().lua_version {
                lua_types::LuaVersion::V52 | lua_types::LuaVersion::V53 => 1,
                _ => 2,
            };
            return Err(lua_vm::debug::arg_error_impl(
                state,
                argn,
                b"position out of bounds",
            ));
        }
    }
    // Cache the table once to avoid re-resolving stack slot 1 on every
    // iteration of the shift loop. C's lua_geti is a single pointer
    // arithmetic operation; our index_to_value is a function call with
    // branches, so this saves ~2N index resolutions for shift count N.
    if let Some(tbl) = plain_table_at(state, 1) {
        let result = tbl.get_int(pos);
        state.push(result);
        while pos < size {
            let shifted = tbl.get_int(pos + 1);
            raw_set_int(state, tbl, pos, shifted)?;
            pos += 1;
        }
        raw_set_int(state, tbl, pos, LuaValue::Nil)?;
        return Ok(1);
    }
    let tbl = state.value_at(1);
    state.table_get_i_value(&tbl, pos)?; // push element to be returned
    while pos < size {
        state.table_get_i_value(&tbl, pos + 1)?;
        state.table_set_i_value(&tbl, pos)?;
        pos += 1;
    }
    state.push(LuaValue::Nil);
    state.table_set_i_value(&tbl, pos)?; // remove last slot (table[pos] = nil)
    Ok(1)
}

// ─── table.move ───────────────────────────────────────────────────────────────

/// `table.move(a1, f, e, t, [a2])`. Copies `a1[f..e]` into `a2[t..]` (or
/// `a1[t..]` if `a2` is absent), reading source slots through `__index` and
/// writing destinations through `__newindex` one element at a time. To survive
/// an overlapping in-place range, it copies FORWARD (increasing index) when the
/// destination is clear of the source's tail (`t > e || t <= f`, or a distinct
/// destination table) and BACKWARD (decreasing) otherwise — the order pinned by
/// `v53_plus_move_*`. Returns the destination table. A 5.3 addition.
pub fn tmove(state: &mut LuaState) -> Result<usize, LuaError> {
    let f = state.check_arg_integer(2)?;
    let e = state.check_arg_integer(3)?;
    let t = state.check_arg_integer(4)?;
    let tt: i32 = if !matches!(state.type_at(5), LuaType::None | LuaType::Nil) {
        5
    } else {
        1
    };
    check_tab(state, 1, TAB_R)?;
    check_tab(state, tt, TAB_W)?;

    if e >= f {
        if !(f > 0 || e < i64::MAX + f) {
            return Err(lua_vm::debug::arg_error_impl(
                state,
                3,
                b"too many elements to move",
            ));
        }
        let n = e - f + 1;
        if !(t <= i64::MAX - n + 1) {
            return Err(lua_vm::debug::arg_error_impl(
                state,
                4,
                b"destination wrap around",
            ));
        }
        // Copy forward (increasing) when safe to do so; backward when ranges overlap.
        let copy_forward = t > e || t <= f || (tt != 1 && !state.compare(1, tt, CompareOp::Eq)?);
        if copy_forward {
            for i in 0..n {
                state.table_get_i(1, f + i)?;
                state.table_set_i(tt, t + i)?;
            }
        } else {
            for i in (0..n).rev() {
                state.table_get_i(1, f + i)?;
                state.table_set_i(tt, t + i)?;
            }
        }
    }
    state.push_value_at(tt)?;
    Ok(1)
}

// ─── table.concat ─────────────────────────────────────────────────────────────

/// Fetches `t[idx]`; if it is a string-or-number, appends its string form to
/// `buf` and pops it. A non-coercible element raises the exact "invalid value
/// (<type>) at index <idx> in table for 'concat'" message (pinned by
/// `v_table_concat_invalid_value_type_name`). The accumulator is a borrowed
/// `Vec<u8>` rather than C's stack-backed `luaL_Buffer`.
fn add_field(state: &mut LuaState, buf: &mut Vec<u8>, idx: i64) -> Result<(), LuaError> {
    state.table_get_i(1, idx)?;
    if !matches!(state.type_at(-1), LuaType::String | LuaType::Number) {
        let type_name = state.type_name_str_at(-1);
        let msg = format!(
            "invalid value ({}) at index {} in table for 'concat'",
            String::from_utf8_lossy(type_name),
            idx
        );
        return crate::auxlib::lua_error(state, msg.as_bytes()).map(|_| ());
    }
    let bytes = state
        .to_bytes_at(-1)
        .ok_or_else(|| LuaError::runtime(format_args!("invalid value at index {}", idx)))?;
    buf.extend_from_slice(&bytes);
    state.pop_n(1);
    Ok(())
}

/// `table.concat(t, [sep, [i, [j]]])`. Joins `t[i..j]` (defaults `i=1`,
/// `j=#t`) with `sep` (default empty) into one string. Each element must be a
/// string or number; the first that is not raises through [`add_field`].
pub fn concat(state: &mut LuaState) -> Result<usize, LuaError> {
    let last = check_table_and_get_len(state, 1, TAB_R)?;
    // Clone the separator before any stack-mutating calls that might invalidate it.
    let sep: Vec<u8> = state.opt_arg_lstring(2, Some(b""))?.unwrap_or_default();
    let mut i = state.opt_arg_integer(3, 1)?;
    let last = state.opt_arg_integer(4, last)?;

    // A borrowed Vec<u8> accumulates the result, pushed once at the end —
    // rather than C's luaL_Buffer, which can back-patch the live Lua stack.
    let mut buf: Vec<u8> = Vec::new();
    while i < last {
        add_field(state, &mut buf, i)?;
        buf.extend_from_slice(&sep);
        i += 1;
    }
    if i == last {
        add_field(state, &mut buf, i)?;
    }
    state.push_lstring(&buf)?;
    Ok(1)
}

// ─── table.pack / table.unpack ────────────────────────────────────────────────

/// `table.pack(...)`. Creates a new table with the arguments at integer keys
/// `1..n` and `t.n` set to the *literal* argument count `n` — holes and
/// trailing nils included, so `.n` recovers an arity that a `#t` border would
/// lose (pinned by `v52_plus_pack_n_field_*`). A 5.2 addition.
pub fn pack(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.get_top();
    state.create_table(n, 1)?;
    state.insert(1)?;
    // table_set_i pops the top; args shift from n+1..=2 down to 1..=n as we pop
    for i in (1..=n).rev() {
        state.table_set_i(1, i as i64)?;
    }
    state.push(LuaValue::Int(n as i64));
    state.set_field(1, b"n")?;
    Ok(1)
}

/// `table.unpack(t, [i, [j]])`. Pushes `t[i], t[i+1], …, t[j]` (defaults
/// `i=1`, `j=#t`) and returns the count. An `i > e` range is empty (zero
/// results); a span of `INT_MAX` or more — including the i64-extreme wrap where
/// `e - i` overflows to a huge unsigned value — raises "too many results to
/// unpack" rather than attempting the push (pinned by
/// `v52_plus_unpack_*` / `v53_plus_unpack_*`). A 5.2 addition.
pub fn unpack(state: &mut LuaState) -> Result<usize, LuaError> {
    let i = state.opt_arg_integer(2, 1)?;
    let e = if matches!(state.type_at(3), LuaType::None | LuaType::Nil) {
        state.length_at(1)?
    } else {
        state.check_arg_integer(3)?
    };
    if i > e {
        return Ok(0); // empty range
    }
    let n = (e as u64).wrapping_sub(i as u64);
    // The size check uses the pre-increment value so that a wrapped-to-0 result
    // (e.g. i=minI, e=maxI yields n = 2^64-1 pre-inc, 0 post-inc) still trips
    // the error rather than silently entering a 2^64-iteration loop.
    if n >= i32::MAX as u64 {
        return Err(LuaError::runtime(format_args!(
            "too many results to unpack"
        )));
    }
    let n = n + 1;
    if !state.check_stack_growth(n as i32) {
        return Err(LuaError::runtime(format_args!(
            "too many results to unpack"
        )));
    }
    let n = n as i64;
    let mut k = i;
    while k < e {
        state.table_get_i(1, k)?;
        k += 1;
    }
    state.table_get_i(1, e)?; // push last element
    Ok(n as usize)
}

// ─── Quicksort ────────────────────────────────────────────────────────────────

/// selection when a partition is severely imbalanced.
///
/// `unsigned int` array whose elements are summed.
///
/// PORT NOTE: C uses a small randomised pivot guard to avoid pathological sort
/// partitions. The Rust port asks the host for entropy when available and falls
/// back to a deterministic pivot value in sandboxed/bare-WASM hosts.
fn randomize_pivot(state: &LuaState) -> u32 {
    let entropy = state.global().entropy_hook.map(|hook| hook()).unwrap_or(0);
    let mixed = entropy ^ entropy.wrapping_shr(32);
    (mixed as u32) ^ (mixed as u32).wrapping_shr(16)
}

/// `table[i]` and `table[j]` respectively (table is at stack position 1).
///
/// ```c
/// static void set2 (lua_State *L, IdxT i, IdxT j) {
///   lua_seti(L, 1, i);
///   lua_seti(L, 1, j);
/// }
/// ```
fn set2(state: &mut LuaState, i: IdxT, j: IdxT) -> Result<(), LuaError> {
    // First seti pops the stack top; second seti pops the new top.
    state.table_set_i(1, i as i64)?;
    state.table_set_i(1, j as i64)?;
    Ok(())
}

/// sort order: either the `<` operator (if arg 2 is nil) or the user's
/// comparison function at stack position 2.
///
/// ```c
/// static int sort_comp (lua_State *L, int a, int b) {
///   if (lua_isnil(L, 2))
///     return lua_compare(L, a, b, LUA_OPLT);
///   else {
///     int res;
///     lua_pushvalue(L, 2);
///     lua_pushvalue(L, a-1);
///     lua_pushvalue(L, b-2);
///     lua_call(L, 2, 1);
///     res = lua_toboolean(L, -1);
///     lua_pop(L, 1);
///     return res;
///   }
/// }
/// ```
///
/// The offsets `a-1` and `b-2` compensate for the function and first-argument
/// copies pushed before the respective values: `a-1` accounts for the function
/// push; `b-2` accounts for both the function push and the copy of `a`.
fn sort_comp(state: &mut LuaState, a: i32, b: i32) -> Result<bool, LuaError> {
    if state.type_at(2) == LuaType::Nil {
        // No user comparator: use the default `<` operator.
        return state.compare(a, b, CompareOp::Lt);
    }
    // User comparator at stack position 2.
    state.push_value_at(2)?; // push function
    state.push_value_at(a - 1)?; // push copy of a (compensate for function push)
    state.push_value_at(b - 2)?; // push copy of b (compensate for function + a copy)
    state.call(2, 1)?;
    let res = state.to_boolean(-1);
    state.pop_n(1);
    Ok(res)
}

/// is already on the top of the Lua stack.
///
/// Precondition: `a[lo] <= P == a[up-1] <= a[up]` and `P` is at stack top.
/// Postcondition: `a[lo..i-1] <= a[i] == P <= a[i+1..up]`; stack is clean.
/// Returns the final pivot index `i`.
///
/// ```c
/// static IdxT partition (lua_State *L, IdxT lo, IdxT up) {
///   IdxT i = lo;
///   IdxT j = up - 1;
///   for (;;) {
///     while ((void)lua_geti(L, 1, ++i), sort_comp(L, -1, -2)) {
///       if (l_unlikely(i == up - 1))
///         luaL_error(L, "invalid order function for sorting");
///       lua_pop(L, 1);
///     }
///     while ((void)lua_geti(L, 1, --j), sort_comp(L, -3, -1)) {
///       if (l_unlikely(j < i))
///         luaL_error(L, "invalid order function for sorting");
///       lua_pop(L, 1);
///     }
///     if (j < i) {
///       lua_pop(L, 1);
///       set2(L, up - 1, i);
///       return i;
///     }
///     set2(L, i, j);
///   }
/// }
/// ```
fn partition(state: &mut LuaState, lo: IdxT, up: IdxT) -> Result<IdxT, LuaError> {
    let mut i: IdxT = lo;
    let mut j: IdxT = up - 1;
    // Entry: stack top is P (pivot value).
    loop {
        // Advance i: find first a[i] >= P.
        // Stack during i-loop body: P(-2), a[i](-1)
        loop {
            i += 1;
            state.table_get_i(1, i as i64)?; // push a[i]
            if !sort_comp(state, -1, -2)? {
                // a[i] >= P: leave a[i] on stack and exit
                break;
            }
            // a[i] < P; check for invalid comparator
            if i == up - 1 {
                return Err(LuaError::runtime(format_args!(
                    "invalid order function for sorting"
                )));
            }
            state.pop_n(1); // remove a[i]
        }
        // Retreat j: find last a[j] <= P.
        // Stack during j-loop body: P(-3), a[i](-2), a[j](-1)
        loop {
            // PERF(port): wrapping_sub mirrors C unsigned IdxT behaviour for edge cases
            j = j.wrapping_sub(1);
            state.table_get_i(1, j as i64)?; // push a[j]
            if !sort_comp(state, -3, -1)? {
                // P >= a[j]: leave a[j] on stack and exit
                break;
            }
            // P < a[j]; check for invalid comparator
            if j < i {
                return Err(LuaError::runtime(format_args!(
                    "invalid order function for sorting"
                )));
            }
            state.pop_n(1); // remove a[j]
        }
        // Stack: P(-3), a[i](-2), a[j](-1)
        if j < i {
            // No out-of-place elements; finalize: place pivot at position i.
            state.pop_n(1); // pop a[j]; stack: P(-2), a[i](-1)
            set2(state, up - 1, i)?; // table[up-1] = a[i], table[i] = P; stack clean
            return Ok(i);
        }
        // Swap a[i] and a[j] to restore loop invariant.
        // set2: table[i] = a[j] (pops -1), table[j] = a[i] (pops new -1); stack: P(-1)
        set2(state, i, j)?;
    }
}

/// `[lo, up]`, randomised by `rnd`.
///
/// ```c
/// static IdxT choosePivot (IdxT lo, IdxT up, unsigned int rnd) {
///   IdxT r4 = (up - lo) / 4;
///   IdxT p = rnd % (r4 * 2) + (lo + r4);
///   lua_assert(lo + r4 <= p && p <= up - r4);
///   return p;
/// }
/// ```
fn choose_pivot(lo: IdxT, up: IdxT, rnd: u32) -> IdxT {
    let r4 = (up - lo) / 4; // range / 4
    let p = rnd % (r4 * 2) + (lo + r4);
    debug_assert!(lo + r4 <= p && p <= up - r4);
    p
}

///
/// Sorts `table[lo..=up]` in place, recursing on the smaller partition and
/// tail-looping on the larger (to bound Rust's call stack). Randomises pivot
/// selection when a partition is badly imbalanced.
///
/// ```c
/// static void auxsort (lua_State *L, IdxT lo, IdxT up, unsigned int rnd) {
///   while (lo < up) {
///     IdxT p, n;
///     lua_geti(L, 1, lo); lua_geti(L, 1, up);
///     if (sort_comp(L, -1, -2)) set2(L, lo, up); else lua_pop(L, 2);
///     if (up - lo == 1) return;
///     if (up - lo < RANLIMIT || rnd == 0) p = (lo + up)/2;
///     else p = choosePivot(lo, up, rnd);
///     lua_geti(L, 1, p); lua_geti(L, 1, lo);
///     if (sort_comp(L, -2, -1)) set2(L, p, lo);
///     else {
///       lua_pop(L, 1); lua_geti(L, 1, up);
///       if (sort_comp(L, -1, -2)) set2(L, p, up); else lua_pop(L, 2);
///     }
///     if (up - lo == 2) return;
///     lua_geti(L, 1, p); lua_pushvalue(L, -1); lua_geti(L, 1, up - 1);
///     set2(L, p, up - 1);
///     p = partition(L, lo, up);
///     if (p - lo < up - p) {
///       auxsort(L, lo, p - 1, rnd); n = p - lo; lo = p + 1;
///     } else {
///       auxsort(L, p + 1, up, rnd); n = up - p; up = p - 1;
///     }
///     if ((up - lo) / 128 > n) rnd = l_randomizePivot();
///   }
/// }
/// ```
fn aux_sort(
    state: &mut LuaState,
    mut lo: IdxT,
    mut up: IdxT,
    mut rnd: u32,
) -> Result<(), LuaError> {
    while lo < up {
        // Step 1: ensure a[lo] <= a[up] (cheap two-element sort)
        state.table_get_i(1, lo as i64)?; // push a[lo]
        state.table_get_i(1, up as i64)?; // push a[up]
        if sort_comp(state, -1, -2)? {
            set2(state, lo, up)?; // swap so a[lo] <= a[up]
        } else {
            state.pop_n(2);
        }
        if up - lo == 1 {
            return Ok(()); // only 2 elements, now sorted
        }

        // Step 2: choose pivot index
        let mut p: IdxT = if up - lo < RANLIMIT || rnd == 0 {
            (lo + up) / 2 // midpoint pivot for small/non-random runs
        } else {
            choose_pivot(lo, up, rnd)
        };

        // Step 3: median-of-three: sort a[lo], a[p], a[up]
        state.table_get_i(1, p as i64)?; // push a[p]
        state.table_get_i(1, lo as i64)?; // push a[lo]
        if sort_comp(state, -2, -1)? {
            set2(state, p, lo)?; // swap a[p] ↔ a[lo]; stack clean
        } else {
            state.pop_n(1); // remove a[lo]; stack: a[p]
            state.table_get_i(1, up as i64)?; // push a[up]; stack: a[p], a[up]
            if sort_comp(state, -1, -2)? {
                set2(state, p, up)?; // swap a[p] ↔ a[up]; stack clean
            } else {
                state.pop_n(2); // remove a[up] and a[p]; stack clean
            }
        }
        // Stack is clean at this point.
        if up - lo == 2 {
            return Ok(()); // only 3 elements, now sorted
        }

        // Step 4: move pivot to a[up-1] and call partition.
        //
        // Stack evolution:
        //   table_get_i(p):    a[p]  (-1)
        //   push_value_at(-1): a[p]  (-2), a[p]_copy  (-1)
        //   table_get_i(up-1): a[p]  (-3), a[p]_copy  (-2), a[up-1]  (-1)
        //   set2(p, up-1):     table[p] = a[up-1], table[up-1] = a[p]_copy;
        //                      stack: a[p] (-1)  ← pivot for partition
        state.table_get_i(1, p as i64)?;
        state.push_value_at(-1)?; // duplicate: two copies of pivot on stack
        state.table_get_i(1, (up - 1) as i64)?;
        set2(state, p, up - 1)?;
        // One copy of the pivot value remains at the stack top for partition.

        p = partition(state, lo, up)?;
        // Stack is clean after partition returns.

        // Step 5: recurse on smaller partition; tail-loop on larger.
        let n: IdxT;
        if p - lo < up - p {
            aux_sort(state, lo, p - 1, rnd)?;
            n = p - lo;
            lo = p + 1; // tail: sort [p+1 .. up]
        } else {
            aux_sort(state, p + 1, up, rnd)?;
            n = up - p;
            up = p - 1; // tail: sort [lo .. p-1]
        }

        // Re-randomise if the partition was severely imbalanced.
        if (up - lo) / 128 > n {
            rnd = randomize_pivot(state);
        }
    }
    Ok(())
}

///
/// ```c
/// static int sort (lua_State *L) {
///   lua_Integer n = aux_getn(L, 1, TAB_RW);
///   if (n > 1) {
///     luaL_argcheck(L, n < INT_MAX, 1, "array too big");
///     if (!lua_isnoneornil(L, 2))
///       luaL_checktype(L, 2, LUA_TFUNCTION);
///     lua_settop(L, 2);
///     auxsort(L, 1, (IdxT)n, 0);
///   }
///   return 0;
/// }
/// ```
pub fn sort(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = check_table_and_get_len(state, 1, TAB_RW)?;
    if n > 1 {
        if !(n < i32::MAX as i64) {
            return Err(lua_vm::debug::arg_error_impl(state, 1, b"array too big"));
        }
        if !matches!(state.type_at(2), LuaType::None | LuaType::Nil) {
            state.check_arg_type(2, LuaType::Function)?;
        }
        // Must go through the public C-API set_top (relative to the call
        // frame); the inherent LuaState::set_top treats its argument as
        // an absolute stack slot and would corrupt the frame.
        lua_vm::api::set_top(state, 2)?;
        aux_sort(state, 1, n as IdxT, 0)?;
    }
    Ok(0)
}

// ─── Registration ─────────────────────────────────────────────────────────────

/// The core `table` roster shared by 5.2-5.5. `move` is filtered out for 5.2
/// (a 5.3 addition) and `move`/`pack`/`unpack` for 5.1 by [`open_table`], which
/// also layers the version-specific extras (5.1 legacy, 5.5 `create`) on top.
pub const TABLE_FUNCS: &[(&[u8], fn(&mut LuaState) -> Result<usize, LuaError>)] = &[
    (b"concat", concat),
    (b"insert", insert),
    (b"pack", pack),
    (b"unpack", unpack),
    (b"remove", remove),
    (b"move", tmove),
    (b"sort", sort),
];

/// `table.create(nseq [, nrec])` — Lua 5.5 addition
/// (`specs/research/5.5-upstream-delta.md` §5, `ltablib.c`).
///
/// Preallocates a table with `nseq` array (sequence) slots and `nrec` hash
/// (record) slots, returning the empty table. Preallocation is purely a
/// capacity hint; the returned table is observably empty (length 0, no keys),
/// so this implementation is behaviorally faithful even though our
/// `create_table` may treat the sizes as advisory.
///
/// Registered into the `table` roster only under [`lua_types::LuaVersion::V55`]
/// (see [`open_table`]); absent under 5.1-5.4, matching upstream.
pub fn create(state: &mut LuaState) -> Result<usize, LuaError> {
    let nseq = state.check_arg_integer(1)?;
    let nrec = state.opt_arg_integer(2, 0)?;
    if nseq < 0 || nseq > i32::MAX as i64 {
        return Err(LuaError::runtime(format_args!(
            "bad argument #1 to 'create' (size out of range)"
        )));
    }
    if nrec < 0 || nrec > i32::MAX as i64 {
        return Err(LuaError::runtime(format_args!(
            "bad argument #2 to 'create' (size out of range)"
        )));
    }
    state.create_table(nseq as i32, nrec as i32)?;
    Ok(1)
}

// ─── Lua 5.1 legacy compat functions (`getn`/`setn`/`maxn`/`foreach`/`foreachi`) ──
//
// These predate the `#` operator and the 5.2 roster cleanup; they ship only in
// the default lua5.1.5 build (`ltablib.c`) and are registered under the V51
// backend by `open_table`. Verified against lua5.1.5; see
// specs/followup/5.1-roster-syntax.md §1.

/// `table.getn(t)` — the "size" of a sequence, i.e. the border `#t` reports.
///
/// In 5.1 `aux_getn` is `luaL_checktype(TABLE)` followed by `luaL_getn`, which
/// resolves to the primitive length. Mirrors `getn` in 5.1 `ltablib.c`.
fn getn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    let n = state.length_at(1)?;
    state.push(LuaValue::Int(n));
    Ok(1)
}

/// `table.setn(t, n)` — obsolete gravestone. In 5.1 the default build defines
/// `luaL_setn` as a no-op, so `setn` raises `'setn' is obsolete`. Verified
/// against lua5.1.5 (`pcall`-able to that exact message).
fn setn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    Err(LuaError::runtime(format_args!("'setn' is obsolete")))
}

/// `table.maxn(t)` — the largest positive numeric key (0 if none). Iterates the
/// raw table via `next`, tracking the max numeric key. Mirrors `maxn` in 5.1
/// `ltablib.c`.
fn maxn(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    let mut max: f64 = 0.0;
    state.push(LuaValue::Nil);
    while state.table_next(1)? {
        // Stack: ..., key, value. Drop the value, inspect the key.
        state.pop_n(1);
        if matches!(state.type_at(-1), LuaType::Number) {
            if let Some(v) = state.to_number(-1) {
                if v > max {
                    max = v;
                }
            }
        }
    }
    state.push(LuaValue::Float(max));
    Ok(1)
}

/// `table.foreachi(t, f)` — call `f(i, t[i])` for `i` in `1..#t`, stopping early
/// if `f` returns a non-nil value (which is then returned). Mirrors `foreachi`
/// in 5.1 `ltablib.c`.
fn foreachi(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    state.check_arg_type(2, LuaType::Function)?;
    let n = state.length_at(1)?;
    let mut i: i64 = 1;
    while i <= n {
        state.push_value_at(2)?;
        state.push(LuaValue::Int(i));
        state.table_get_i(1, i)?;
        state.call(2, 1)?;
        if !matches!(state.type_at(-1), LuaType::Nil) {
            return Ok(1);
        }
        state.pop_n(1);
        i += 1;
    }
    Ok(0)
}

/// `table.foreach(t, f)` — call `f(k, v)` for every pair, stopping early if `f`
/// returns a non-nil value (which is then returned). Mirrors `foreach` in 5.1
/// `ltablib.c`.
fn foreach(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Table)?;
    state.check_arg_type(2, LuaType::Function)?;
    state.push(LuaValue::Nil);
    while state.table_next(1)? {
        // Stack: ..., key, value.
        state.push_value_at(2)?; // function
        state.push_value_at(-3)?; // key copy
        state.push_value_at(-3)?; // value copy
        state.call(2, 1)?;
        if !matches!(state.type_at(-1), LuaType::Nil) {
            return Ok(1);
        }
        state.pop_n(2); // remove value and result, leaving key for next()
    }
    Ok(0)
}

// ─── Module opener ────────────────────────────────────────────────────────────

/// Builds the `table` library table for the running version. The base roster is
/// [`TABLE_FUNCS`], from which 5.1 and 5.2 drop the functions they lack and onto
/// which 5.1's legacy roster and 5.5's `create` are layered. The per-version
/// deltas below are each verified against that version's reference binary.
pub fn open_table(state: &mut LuaState) -> Result<usize, LuaError> {
    // Per-version roster deltas:
    //  - `table.move` is a Lua 5.3 addition, absent in 5.1/5.2 (verified against
    //    lua5.2.4: `type(table.move)` == "nil").
    //  - `table.pack`/`table.unpack` are Lua 5.2 additions; in 5.1 `unpack` is a
    //    *global* and there is no `table.pack` (verified against lua5.1.5: both
    //    `table.unpack` and `table.pack` are nil). 5.1 instead carries the legacy
    //    `getn`/`setn`/`maxn`/`foreach`/`foreachi` roster.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        let legacy: Vec<(&[u8], fn(&mut LuaState) -> Result<usize, LuaError>)> = TABLE_FUNCS
            .iter()
            .filter(|(name, _)| {
                *name != b"move".as_slice()
                    && *name != b"pack".as_slice()
                    && *name != b"unpack".as_slice()
            })
            .copied()
            .collect();
        state.new_lib(&legacy)?;
        const LEGACY_FUNCS: &[(&[u8], fn(&mut LuaState) -> Result<usize, LuaError>)] = &[
            (b"getn", getn),
            (b"setn", setn),
            (b"maxn", maxn),
            (b"foreach", foreach),
            (b"foreachi", foreachi),
        ];
        state.set_funcs_with_upvalues(LEGACY_FUNCS, 0)?;
    } else if matches!(state.global().lua_version, lua_types::LuaVersion::V52) {
        let without_move: Vec<(&[u8], fn(&mut LuaState) -> Result<usize, LuaError>)> = TABLE_FUNCS
            .iter()
            .filter(|(name, _)| *name != b"move".as_slice())
            .copied()
            .collect();
        state.new_lib(&without_move)?;
    } else {
        state.new_lib(TABLE_FUNCS)?;
    }
    // Per-version roster delta: `table.create` is a Lua 5.5 addition
    // (`specs/research/5.5-upstream-delta.md` §5), absent in 5.1-5.4. Register
    // it only on the V55 backend so the version seam carries a real,
    // script-observable stdlib difference. `new_lib` leaves the new table on
    // the stack top, so we register `create` into it directly.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V55) {
        const CREATE_FUNCS: &[(&[u8], fn(&mut LuaState) -> Result<usize, LuaError>)] =
            &[(b"create", create)];
        state.set_funcs_with_upvalues(CREATE_FUNCS, 0)?;
    }
    Ok(1)
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/ltablib.c  (430 lines, 14 functions)
//   target_crate:  lua-stdlib
//   confidence:    medium
//   todos:         17
//   port_notes:    5
//   unsafe_blocks: 0
//   notes:         Logic is faithfully translated. All TODOs are method-name
//                  uncertainties for LuaState API calls (table_get_i, table_set_i,
//                  opt_arg_integer, opt_arg_lstring, get_metatable, to_bytes_at,
//                  push_lstring, push_value_at, compare, new_lib, set_top,
//                  check_stack_growth, type_name_str_at, create_table) — Phase B
//                  maps these to the real method names once lua-vm is drafted.
//                  Stack cleanup on error paths is elided (C uses longjmp);
//                  needs Phase B attention in check_tab and add_field.
//                  PERF: remove() and insert() shift loops now cache the table
//                  value once (via value_at) and call table_get_i_value /
//                  table_set_i_value, bypassing the per-iteration index_to_value
//                  call. This shrank the index_to_value hot frame and improved
//                  table_ops_long from ~4.76x to ~4.02x vs reference.
// ──────────────────────────────────────────────────────────────────────────────
