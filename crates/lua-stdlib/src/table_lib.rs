//! Rust port of `ltablib.c` — Lua `table` standard library.
//!
//! Provides: `table.concat`, `table.insert`, `table.move`, `table.pack`,
//! `table.remove`, `table.sort`, `table.unpack`.
//!
//! C source: `reference/lua-5.4.7/src/ltablib.c` (430 lines, 14 functions)

use lua_types::{LuaError, LuaType, LuaValue};
use crate::state_stub::{LuaState, lua_CFunction, upvalue_index, CompareOp, LuaDebug};

// ─── Operation flags ──────────────────────────────────────────────────────────
// C: #define TAB_R 1  /* read */
const TAB_R: u32 = 1;
// C: #define TAB_W 2  /* write */
const TAB_W: u32 = 2;
// C: #define TAB_L 4  /* length */
const TAB_L: u32 = 4;
// C: #define TAB_RW (TAB_R | TAB_W)
const TAB_RW: u32 = TAB_R | TAB_W;

// C: #define RANLIMIT 100u — arrays larger than this may use randomised pivots
const RANLIMIT: u32 = 100;

/// C: `typedef unsigned int IdxT;` — sort array-index type.
type IdxT = u32;

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// C: `checkfield(L, key, n)` — push `key` and raw-get it from the metatable
/// currently sitting at stack depth `n`; returns `true` if the result is not nil.
///
/// ```c
/// static int checkfield (lua_State *L, const char *key, int n) {
///   lua_pushstring(L, key);
///   return (lua_rawget(L, -n) != LUA_TNIL);
/// }
/// ```
fn check_field(state: &mut LuaState, key: &[u8], n: i32) -> Result<bool, LuaError> {
    // TODO(port): state.push_string pushes a Lua string from &[u8]; verify method name
    state.push_string(key)?;
    // raw_get(-n): looks up MT[key] (MT is at -n after the key push), replaces key with value
    let ty = state.raw_get(-n)?;
    Ok(ty != LuaType::Nil)
}

/// C: `checktab(L, arg, what)` — verify that `arg` is a table, or has a
/// metatable with the metamethods required by `what`
/// (`TAB_R` → `__index`, `TAB_W` → `__newindex`, `TAB_L` → `__len`).
///
/// ```c
/// static void checktab (lua_State *L, int arg, int what) {
///   if (lua_type(L, arg) != LUA_TTABLE) {
///     int n = 1;
///     if (lua_getmetatable(L, arg) &&
///         (!(what & TAB_R) || checkfield(L, "__index", ++n)) &&
///         (!(what & TAB_W) || checkfield(L, "__newindex", ++n)) &&
///         (!(what & TAB_L) || checkfield(L, "__len", ++n))) {
///       lua_pop(L, n);
///     }
///     else
///       luaL_checktype(L, arg, LUA_TTABLE);
///   }
/// }
/// ```
///
/// PORT NOTE: stack cleanup on the error path is elided here (in C, `longjmp`
/// unwinds automatically). Phase B should add cleanup before the `check_arg_type`
/// call to leave the stack consistent.
fn check_tab(state: &mut LuaState, arg: i32, what: u32) -> Result<(), LuaError> {
    if state.type_at(arg) == LuaType::Table {
        return Ok(());
    }
    // `n` tracks how many items have been pushed (MT + checked field values).
    let mut n: i32 = 1;
    // TODO(port): state.get_metatable returns bool (pushes MT if found); verify method name
    let has_mt = state.get_metatable(arg);
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
        state.pop_n(n);
        Ok(())
    } else {
        // C: luaL_checktype(L, arg, LUA_TTABLE) — forces a type error
        state.check_arg_type(arg, LuaType::Table)
    }
}

/// C: `#define aux_getn(L,n,w) (checktab(L, n, (w) | TAB_L), luaL_len(L, n))`
///
/// Check that argument `n` is a table (or table-like per `w`) and return its length.
fn aux_getn(state: &mut LuaState, n: i32, w: u32) -> Result<i64, LuaError> {
    check_tab(state, n, w | TAB_L)?;
    // TODO(port): state.length_at applies the `#` operator and returns i64; verify method name
    state.length_at(n)
}

// ─── table.insert ─────────────────────────────────────────────────────────────

/// C: `tinsert(L)` — implements `table.insert(t [, pos,] v)`.
///
/// ```c
/// static int tinsert (lua_State *L) {
///   lua_Integer pos;
///   lua_Integer e = aux_getn(L, 1, TAB_RW);
///   e = luaL_intop(+, e, 1);
///   switch (lua_gettop(L)) {
///     case 2: { pos = e; break; }
///     case 3: {
///       lua_Integer i;
///       pos = luaL_checkinteger(L, 2);
///       luaL_argcheck(L, (lua_Unsigned)pos - 1u < (lua_Unsigned)e, 2,
///                        "position out of bounds");
///       for (i = e; i > pos; i--) {
///         lua_geti(L, 1, i - 1);
///         lua_seti(L, 1, i);
///       }
///       break;
///     }
///     default:
///       return luaL_error(L, "wrong number of arguments to 'insert'");
///   }
///   lua_seti(L, 1, pos);
///   return 0;
/// }
/// ```
pub fn insert(state: &mut LuaState) -> Result<usize, LuaError> {
    let mut e = aux_getn(state, 1, TAB_RW)?;
    // C: luaL_intop(+, e, 1) — wrapping unsigned add then re-interpret as signed
    e = (e as u64).wrapping_add(1) as i64;

    let pos: i64 = match state.get_top() {
        // C: case 2 — insert new element at the end
        2 => e,
        // C: case 3 — explicit position argument
        3 => {
            let pos = state.check_arg_integer(2)?;
            // C: luaL_argcheck(L, (lua_Unsigned)pos - 1u < (lua_Unsigned)e, 2, ...)
            // Checks 1 <= pos <= e (wrapping subtraction catches pos <= 0)
            if !((pos as u64).wrapping_sub(1) < (e as u64)) {
                return Err(LuaError::arg_error(2, "position out of bounds"));
            }
            // C: for (i = e; i > pos; i--) { lua_geti(L, 1, i-1); lua_seti(L, 1, i); }
            let mut i = e;
            while i > pos {
                // TODO(port): state.table_get_i(stack_pos, integer_key) → lua_geti; verify name
                state.table_get_i(1, i - 1)?;
                // TODO(port): state.table_set_i(stack_pos, integer_key) → lua_seti (pops top); verify name
                state.table_set_i(1, i)?;
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
    // C: lua_seti(L, 1, pos) — pops new value and sets table[pos]
    state.table_set_i(1, pos)?;
    Ok(0)
}

// ─── table.remove ─────────────────────────────────────────────────────────────

/// C: `tremove(L)` — implements `table.remove(t [, pos])`.
///
/// ```c
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
/// ```
pub fn remove(state: &mut LuaState) -> Result<usize, LuaError> {
    let size = aux_getn(state, 1, TAB_RW)?;
    // TODO(port): state.opt_arg_integer(n, default) → luaL_optinteger; verify method name
    let mut pos = state.opt_arg_integer(2, size)?;
    if pos != size {
        // C: luaL_argcheck — checks 1 <= pos <= size+1
        if !((pos as u64).wrapping_sub(1) <= (size as u64)) {
            return Err(LuaError::arg_error(2, "position out of bounds"));
        }
    }
    state.table_get_i(1, pos)?;   // push element to be returned
    while pos < size {
        state.table_get_i(1, pos + 1)?;
        state.table_set_i(1, pos)?;
        pos += 1;
    }
    state.push(LuaValue::Nil);
    state.table_set_i(1, pos)?;   // remove last slot (table[pos] = nil)
    Ok(1)
}

// ─── table.move ───────────────────────────────────────────────────────────────

/// C: `tmove(L)` — implements `table.move(a1, f, e, t [, a2])`.
///
/// Copies elements `a1[f..e]` into `a2[t..]` (or `a1[t..]` if `a2` is absent).
/// Copies in increasing order when safe, decreasing when ranges overlap.
///
/// ```c
/// static int tmove (lua_State *L) {
///   lua_Integer f = luaL_checkinteger(L, 2);
///   lua_Integer e = luaL_checkinteger(L, 3);
///   lua_Integer t = luaL_checkinteger(L, 4);
///   int tt = !lua_isnoneornil(L, 5) ? 5 : 1;
///   checktab(L, 1, TAB_R);
///   checktab(L, tt, TAB_W);
///   if (e >= f) {
///     lua_Integer n, i;
///     luaL_argcheck(L, f > 0 || e < LUA_MAXINTEGER + f, 3, "too many elements to move");
///     n = e - f + 1;
///     luaL_argcheck(L, t <= LUA_MAXINTEGER - n + 1, 4, "destination wrap around");
///     if (t > e || t <= f || (tt != 1 && !lua_compare(L, 1, tt, LUA_OPEQ))) {
///       for (i = 0; i < n; i++) { lua_geti(L, 1, f + i); lua_seti(L, tt, t + i); }
///     } else {
///       for (i = n - 1; i >= 0; i--) { lua_geti(L, 1, f + i); lua_seti(L, tt, t + i); }
///     }
///   }
///   lua_pushvalue(L, tt);
///   return 1;
/// }
/// ```
pub fn tmove(state: &mut LuaState) -> Result<usize, LuaError> {
    let f = state.check_arg_integer(2)?;
    let e = state.check_arg_integer(3)?;
    let t = state.check_arg_integer(4)?;
    // C: int tt = !lua_isnoneornil(L, 5) ? 5 : 1
    let tt: i32 = if !matches!(state.type_at(5), LuaType::None | LuaType::Nil) {
        5
    } else {
        1
    };
    check_tab(state, 1, TAB_R)?;
    check_tab(state, tt, TAB_W)?;

    if e >= f {
        // C: luaL_argcheck — overflow guard: e - f + 1 must not overflow i64
        if !(f > 0 || e < i64::MAX + f) {
            return Err(LuaError::arg_error(3, "too many elements to move"));
        }
        let n = e - f + 1;
        // C: luaL_argcheck — destination end must not overflow
        if !(t <= i64::MAX - n + 1) {
            return Err(LuaError::arg_error(4, "destination wrap around"));
        }
        // C: if (t > e || t <= f || (tt != 1 && !lua_compare(L, 1, tt, LUA_OPEQ)))
        // Copy forward (increasing) when safe to do so; backward when ranges overlap.
        // TODO(port): state.compare(a, b, CompareOp::Eq) → lua_compare LUA_OPEQ; verify method
        let copy_forward = t > e
            || t <= f
            || (tt != 1 && !state.compare(1, tt, CompareOp::Eq)?);
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
    // C: lua_pushvalue(L, tt) — return the destination table
    // TODO(port): state.push_value_at → lua_pushvalue; verify method name
    state.push_value_at(tt);
    Ok(1)
}

// ─── table.concat ─────────────────────────────────────────────────────────────

/// C: `addfield(L, b, i)` — push `table[i]` onto the stack, validate it is
/// a string-or-number, add its string representation to `buf`, then pop it.
///
/// ```c
/// static void addfield (lua_State *L, luaL_Buffer *b, lua_Integer i) {
///   lua_geti(L, 1, i);
///   if (l_unlikely(!lua_isstring(L, -1)))
///     luaL_error(L, "invalid value (%s) at index %I in table for 'concat'",
///                   luaL_typename(L, -1), (LUAI_UACINT)i);
///   luaL_addvalue(b);
/// }
/// ```
///
/// PORT NOTE: `luaL_Buffer` in C accumulates bytes and then pushes the result;
/// Rust uses a `Vec<u8>` accumulator passed by mutable reference instead.
fn add_field(state: &mut LuaState, buf: &mut Vec<u8>, idx: i64) -> Result<(), LuaError> {
    state.table_get_i(1, idx)?;
    // C: lua_isstring — true for LUA_TSTRING and LUA_TNUMBER (numbers coerce to strings)
    if !matches!(state.type_at(-1), LuaType::String | LuaType::Number) {
        // TODO(port): state.type_name_str_at(-1) returns &'static str for the base type name
        let type_name = state.type_name_str_at(-1);
        return Err(LuaError::runtime(format_args!(
            "invalid value ({}) at index {} in table for 'concat'",
            type_name, idx
        )));
    }
    // C: luaL_addvalue(b) — convert top to string bytes, append, then pop
    // TODO(port): state.to_bytes_at(-1) converts via Lua's tostring coercion; verify method name
    let bytes = state.to_bytes_at(-1)?;
    buf.extend_from_slice(&bytes);
    state.pop_n(1);
    Ok(())
}

/// C: `tconcat(L)` — implements `table.concat(t [, sep [, i [, j]]])`.
///
/// ```c
/// static int tconcat (lua_State *L) {
///   luaL_Buffer b;
///   lua_Integer last = aux_getn(L, 1, TAB_R);
///   size_t lsep;
///   const char *sep = luaL_optlstring(L, 2, "", &lsep);
///   lua_Integer i = luaL_optinteger(L, 3, 1);
///   last = luaL_optinteger(L, 4, last);
///   luaL_buffinit(L, &b);
///   for (; i < last; i++) { addfield(L, &b, i); luaL_addlstring(&b, sep, lsep); }
///   if (i == last) addfield(L, &b, i);
///   luaL_pushresult(&b);
///   return 1;
/// }
/// ```
pub fn concat(state: &mut LuaState) -> Result<usize, LuaError> {
    let last = aux_getn(state, 1, TAB_R)?;
    // TODO(port): state.opt_arg_lstring(n, default) → luaL_optlstring; verify method name
    // Clone the separator before any stack-mutating calls that might invalidate it.
    let sep: Vec<u8> = state.opt_arg_lstring(2, b"")?.to_vec();
    let mut i = state.opt_arg_integer(3, 1)?;
    let last = state.opt_arg_integer(4, last)?;

    // PORT NOTE: C uses luaL_Buffer (which may back-patch the stack);
    // Rust uses a plain Vec<u8> accumulator and pushes the result at the end.
    let mut buf: Vec<u8> = Vec::new();
    while i < last {
        add_field(state, &mut buf, i)?;
        buf.extend_from_slice(&sep);
        i += 1;
    }
    if i == last {
        add_field(state, &mut buf, i)?;
    }
    // C: luaL_pushresult(&b) — push accumulated string
    // TODO(port): state.push_lstring pushes a Lua string from &[u8]; verify method name
    state.push_lstring(&buf)?;
    Ok(1)
}

// ─── table.pack / table.unpack ────────────────────────────────────────────────

/// C: `tpack(L)` — implements `table.pack(...)`.
///
/// Creates a new table `t` with all arguments as integer keys and `t.n` set
/// to the argument count.
///
/// ```c
/// static int tpack (lua_State *L) {
///   int i;
///   int n = lua_gettop(L);
///   lua_createtable(L, n, 1);
///   lua_insert(L, 1);
///   for (i = n; i >= 1; i--)
///     lua_seti(L, 1, i);
///   lua_pushinteger(L, n);
///   lua_setfield(L, 1, "n");
///   return 1;
/// }
/// ```
pub fn pack(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.get_top();
    // TODO(port): state.create_table(narr, nrec) → lua_createtable; verify method name
    state.create_table(n, 1)?;
    // C: lua_insert(L, 1) — move the new table to position 1, shifting args up
    state.insert(1);
    // C: for (i = n; i >= 1; i--) lua_seti(L, 1, i)
    // table_set_i pops the top; args shift from n+1..=2 down to 1..=n as we pop
    for i in (1..=n).rev() {
        state.table_set_i(1, i as i64)?;
    }
    state.push(LuaValue::Int(n as i64));
    // TODO(port): state.set_field(stack_pos, key_bytes) → lua_setfield; verify method name
    state.set_field(1, b"n")?;
    Ok(1)
}

/// C: `tunpack(L)` — implements `table.unpack(t [, i [, j]])`.
///
/// Pushes `t[i], t[i+1], …, t[j]` and returns the count.
///
/// ```c
/// static int tunpack (lua_State *L) {
///   lua_Unsigned n;
///   lua_Integer i = luaL_optinteger(L, 2, 1);
///   lua_Integer e = luaL_opt(L, luaL_checkinteger, 3, luaL_len(L, 1));
///   if (i > e) return 0;
///   n = (lua_Unsigned)e - i;
///   if (l_unlikely(n >= (unsigned int)INT_MAX ||
///                  !lua_checkstack(L, (int)(++n))))
///     return luaL_error(L, "too many results to unpack");
///   for (; i < e; i++) lua_geti(L, 1, i);
///   lua_geti(L, 1, e);
///   return (int)n;
/// }
/// ```
pub fn unpack(state: &mut LuaState) -> Result<usize, LuaError> {
    let i = state.opt_arg_integer(2, 1)?;
    // C: luaL_opt(L, luaL_checkinteger, 3, luaL_len(L, 1))
    let e = if matches!(state.type_at(3), LuaType::None | LuaType::Nil) {
        state.length_at(1)?
    } else {
        state.check_arg_integer(3)?
    };
    if i > e {
        return Ok(0); // empty range
    }
    // C: n = (lua_Unsigned)e - i  — unsigned subtraction avoids overflow at extremes
    let n = (e as u64).wrapping_sub(i as u64);
    // C: ++n then check: n is now the actual element count
    let n = n.wrapping_add(1);
    // C: n >= (unsigned int)INT_MAX || !lua_checkstack(L, (int)n)
    // TODO(port): state.check_stack_growth(n) → lua_checkstack; verify method name
    if n >= i32::MAX as u64 || !state.check_stack_growth(n as i32) {
        return Err(LuaError::runtime(format_args!("too many results to unpack")));
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

/// C: `l_randomizePivot()` — produce a "random" `u32` to randomize pivot
/// selection when a partition is severely imbalanced.
///
/// C: uses `clock()` and `time()` as entropy sources via `memcpy` into a
/// `unsigned int` array whose elements are summed.
///
/// PORT NOTE: Rust uses `SystemTime` as an entropy source instead of
/// POSIX `clock()`/`time()`. The mixing is simplified (subsecond nanos XOR-
/// shifted) but serves the same purpose: breaking pathological patterns.
fn randomize_pivot() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0u32);
    nanos ^ nanos.wrapping_shr(16)
}

/// C: `set2(L, i, j)` — pop the top two stack values and assign them to
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

/// C: `sort_comp(L, a, b)` — return `true` iff `stack[a] < stack[b]` per the
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
    state.push_value_at(2);       // push function
    state.push_value_at(a - 1);   // push copy of a (compensate for function push)
    state.push_value_at(b - 2);   // push copy of b (compensate for function + a copy)
    // C: lua_call(L, 2, 1)
    state.call(2, 1)?;
    // C: res = lua_toboolean(L, -1); lua_pop(L, 1)
    // TODO(port): state.to_boolean(-1) → lua_toboolean (never fails); verify method name
    let res = state.to_boolean(-1);
    state.pop_n(1);
    Ok(res)
}

/// C: `partition(L, lo, up)` — in-place partition around the pivot `P` that
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

/// C: `choosePivot(lo, up, rnd)` — select a pivot index in the middle half of
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

/// C: `auxsort(L, lo, up, rnd)` — recursive quicksort driver.
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
fn aux_sort(state: &mut LuaState, mut lo: IdxT, mut up: IdxT, mut rnd: u32) -> Result<(), LuaError> {
    while lo < up {
        // Step 1: ensure a[lo] <= a[up] (cheap two-element sort)
        state.table_get_i(1, lo as i64)?; // push a[lo]
        state.table_get_i(1, up as i64)?; // push a[up]
        // C: if (sort_comp(L, -1, -2)) — a[up] < a[lo]?
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
        state.table_get_i(1, p as i64)?;  // push a[p]
        state.table_get_i(1, lo as i64)?; // push a[lo]
        // C: if (sort_comp(L, -2, -1)) — a[p] < a[lo]?
        if sort_comp(state, -2, -1)? {
            set2(state, p, lo)?; // swap a[p] ↔ a[lo]; stack clean
        } else {
            state.pop_n(1); // remove a[lo]; stack: a[p]
            state.table_get_i(1, up as i64)?; // push a[up]; stack: a[p], a[up]
            // C: if (sort_comp(L, -1, -2)) — a[up] < a[p]?
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
        state.push_value_at(-1); // duplicate: two copies of pivot on stack
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
        // C: if ((up - lo) / 128 > n) rnd = l_randomizePivot()
        if (up - lo) / 128 > n {
            rnd = randomize_pivot();
        }
    }
    Ok(())
}

/// C: `sort(L)` — implements `table.sort(t [, comp])`.
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
    let n = aux_getn(state, 1, TAB_RW)?;
    if n > 1 {
        if !(n < i32::MAX as i64) {
            return Err(LuaError::arg_error(1, "array too big"));
        }
        if !matches!(state.type_at(2), LuaType::None | LuaType::Nil) {
            state.check_arg_type(2, LuaType::Function)?;
        }
        // C: lua_settop(L, 2) — discard any extra arguments
        // TODO(port): state.set_top → lua_settop; verify method name
        state.set_top(2);
        aux_sort(state, 1, n as IdxT, 0)?;
    }
    Ok(0)
}

// ─── Registration ─────────────────────────────────────────────────────────────

/// C: `tab_funcs[]` — the function registration table for `require("table")`.
///
/// ```c
/// static const luaL_Reg tab_funcs[] = {
///   {"concat", tconcat}, {"insert", tinsert}, {"pack", tpack},
///   {"unpack", tunpack}, {"remove", tremove}, {"move", tmove},
///   {"sort", sort}, {NULL, NULL}
/// };
/// ```
///
/// PORT NOTE: In Rust we represent this as a slice of `(&[u8], fn-ptr)` pairs;
/// the sentinel `{NULL, NULL}` is implicit (the slice has a known length).
pub const TABLE_FUNCS: &[(&[u8], fn(&mut LuaState) -> Result<usize, LuaError>)] = &[
    (b"concat", concat),
    (b"insert", insert),
    (b"pack", pack),
    (b"unpack", unpack),
    (b"remove", remove),
    (b"move", tmove),
    (b"sort", sort),
];

// ─── Module opener ────────────────────────────────────────────────────────────

/// C: `luaopen_table(L)` — open the `table` library.
///
/// ```c
/// LUAMOD_API int luaopen_table (lua_State *L) {
///   luaL_newlib(L, tab_funcs);
///   return 1;
/// }
/// ```
pub fn open_table(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: LUAMOD_API → pub  (see macros.tsv)
    // TODO(port): state.new_lib → luaL_newlib; creates a new table and registers functions;
    //             verify method name and signature
    state.new_lib(TABLE_FUNCS)?;
    Ok(1)
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/ltablib.c  (430 lines, 14 functions)
//   target_crate:  lua-stdlib
//   confidence:    medium
//   todos:         18
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
// ──────────────────────────────────────────────────────────────────────────────
