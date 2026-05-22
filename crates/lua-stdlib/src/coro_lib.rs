//! Coroutine library — port of `lcorolib.c`.
//!
//! Provides the `coroutine.*` standard-library table: `create`, `resume`,
//! `running`, `status`, `wrap`, `yield`, `isyieldable`, and `close`.
//!
//! # Phase A–D stub notice
//!
//! Every function that requires actual coroutine execution (`resume`, `yield`,
//! cross-thread `xmove`, `new_thread`, `close_thread`) is **unimplemented** and
//! will panic at runtime.  The argument-checking and result-packaging logic is
//! translated faithfully so that Phase E can drop in the real implementations
//! without restructuring.  Phase E wires real stackful coroutines via
//! `corosensei`.  See PORTING.md §2 #6.
//!
//! Translated from: `reference/lua-5.4.7/src/lcorolib.c` (210 lines, 12 functions)
//! Target crate: `lua-stdlib`

// TODO(port): LuaState, GcRef<LuaState>, LuaStatus, and related types live in
// lua-vm / lua-types; all unresolved imports will be fixed in Phase B.
use lua_types::{
    error::LuaError,
    value::LuaValue,
    LuaType,
};

// ── Coroutine status codes ────────────────────────────────────────────────────

// C: #define COS_RUN   0
// C: #define COS_DEAD  1
// C: #define COS_YIELD 2
// C: #define COS_NORM  3

/// Coroutine is the currently running thread.
const COS_RUN: i32 = 0;

/// Coroutine has finished execution or encountered an error.
const COS_DEAD: i32 = 1;

/// Coroutine is suspended — either yielded or not yet started.
const COS_YIELD: i32 = 2;

/// Coroutine is normal — it resumed another coroutine and is waiting.
const COS_NORM: i32 = 3;

/// Human-readable status strings indexed by the `COS_*` constants above.
/// Pushed onto the Lua stack as byte strings.
///
/// C: `static const char *const statname[] = {"running","dead","suspended","normal"};`
const STAT_NAMES: [&[u8]; 4] = [b"running", b"dead", b"suspended", b"normal"];

// ── Registration table ────────────────────────────────────────────────────────

/// Registration table for the `coroutine` standard library.
///
/// C: `static const luaL_Reg co_funcs[]`
///
/// Each entry is `(name_bytes, function_pointer)`. Phase B resolves
/// `lua_CFunction` to the canonical type alias from `lua-types`.
pub const CO_FUNCS: &[(&[u8], lua_CFunction)] = &[
    (b"create",      co_create),
    (b"resume",      co_resume),
    (b"running",     co_running),
    (b"status",      co_status),
    (b"wrap",        co_wrap),
    (b"yield",       co_yield),
    (b"isyieldable", co_isyieldable),
    (b"close",       co_close),
];

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Converts a one-based upvalue number to the pseudo-index used to access it.
///
/// C: `lua_upvalueindex(i)` macro → free function `upvalue_index(i: i32) -> i32`
/// per ANALYSES/macros.tsv.
#[inline]
fn upvalue_index(i: i32) -> i32 {
    // TODO(port): exact formula is `LUA_REGISTRYINDEX - i`; the constant lives in
    // lua-vm.  Phase B supplies the real implementation.
    lua_types::upvalue_index(i)
}

/// Retrieves the coroutine thread at stack index 1, raising a type error if
/// the argument is absent or not a thread.
///
/// C: `static lua_State *getco(lua_State *L)`
fn get_co(state: &mut LuaState) -> Result<GcRef<LuaState>, LuaError> {
    // C: lua_State *co = lua_tothread(L, 1);
    let co = state.to_thread(1);
    // C: luaL_argexpected(L, co, 1, "thread");
    if co.is_none() {
        return Err(LuaError::type_arg_error(1, b"thread", state.arg(1)));
    }
    Ok(co.expect("checked above"))
}

/// Returns one of the `COS_*` status codes describing `co` relative to the
/// calling thread `state`.
///
/// C: `static int auxstatus(lua_State *L, lua_State *co)`
fn aux_status(state: &mut LuaState, co: &GcRef<LuaState>) -> i32 {
    // C: if (L == co) return COS_RUN;
    // TODO(port): coroutine stub — GcRef::ptr_eq (or equivalent) needed to compare
    // two LuaState references; Phase B wire-up required.
    if state.is_same_thread(co) {
        return COS_RUN;
    }
    // C: switch (lua_status(co)) { ... }
    // TODO(port): coroutine stub — thread_status() reads the LuaStatus field of
    // the *other* thread; requires cross-thread access (Phase E).
    match co.thread_status() {
        // C: case LUA_YIELD: return COS_YIELD;
        LuaStatus::Yield => COS_YIELD,
        // C: case LUA_OK: { if (lua_getstack(co,0,&ar)) ... else ... }
        LuaStatus::Ok => {
            // C: lua_Debug ar; if (lua_getstack(co, 0, &ar)) return COS_NORM;
            // TODO(port): coroutine stub — has_frames() probes co's call-stack
            // depth to distinguish "running in another frame" from initial state;
            // Phase E needed.
            if co.has_frames() {
                COS_NORM
            // C: else if (lua_gettop(co) == 0) return COS_DEAD;
            } else if co.get_top() == 0 {
                COS_DEAD
            } else {
                // C: else return COS_YIELD; /* initial state */
                COS_YIELD
            }
        }
        // C: default: return COS_DEAD;
        _ => COS_DEAD,
    }
}

/// Transfers `narg` arguments from `state` to `co`, resumes the coroutine,
/// then transfers results (or error message) back to `state`.
///
/// Returns the number of result values (≥ 0) on success, or `-1` on error
/// with the error object left on top of `state`'s stack.
///
/// C: `static int auxresume(lua_State *L, lua_State *co, int narg)`
fn aux_resume(state: &mut LuaState, _co: GcRef<LuaState>, _narg: i32) -> i32 {
    // TODO(port): coroutine stub — the complete body requires all of:
    //
    //   1. if !lua_checkstack(co, narg) { push "too many arguments"; return -1; }
    //      → co.ensure_stack(narg) — grow co's stack for the incoming arguments.
    //
    //   2. lua_xmove(L, co, narg)
    //      → state.xmove(&co, narg) — transfer narg values from L's stack to co's.
    //
    //   3. status = lua_resume(co, L, narg, &nres)
    //      → co.resume(state, narg) — execute co until it yields, returns, or errors.
    //        Returns (LuaStatus, nres: i32).
    //
    //   4. On LUA_OK / LUA_YIELD success path:
    //      if !lua_checkstack(L, nres + 1) { lua_pop(co, nres); push "too many results"; return -1; }
    //      lua_xmove(co, L, nres) — move results back.
    //      return nres;
    //
    //   5. On error path:
    //      lua_xmove(co, L, 1) — move error message back.
    //      return -1;
    //
    // All cross-thread operations depend on Phase E / corosensei runtime support.
    let _ = state;
    panic!(
        "coroutine.resume is not yet implemented (Phase A–D stub; see PORTING.md §2 #6)"
    );
}

// ── Public library functions ──────────────────────────────────────────────────

/// `coroutine.resume(co [, val1, ...])` — attempt to resume coroutine `co`.
///
/// On success pushes `true` followed by all values yielded or returned by `co`.
/// On failure pushes `false` followed by the error object.
///
/// C: `static int luaB_coresume(lua_State *L)`
pub fn co_resume(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: lua_State *co = getco(L);
    let co = get_co(state)?;
    // C: r = auxresume(L, co, lua_gettop(L) - 1);
    // PORT NOTE: lua_gettop returns the argument count; -1 excludes the coroutine
    // itself which sits at index 1.
    let narg = state.get_top() - 1;
    let r = aux_resume(state, co, narg);
    if r < 0 {
        // C: lua_pushboolean(L, 0); lua_insert(L, -2); return 2;
        state.push(LuaValue::Bool(false));
        state.insert(-2);
        Ok(2)
    } else {
        // C: lua_pushboolean(L, 1); lua_insert(L, -(r + 1)); return r + 1;
        state.push(LuaValue::Bool(true));
        state.insert(-(r + 1));
        Ok((r + 1) as usize)
    }
}

/// Closure body installed by `coroutine.wrap`.  The wrapped coroutine is
/// stored in upvalue slot 1.
///
/// On error the message is augmented with location info (if a string), then
/// re-raised.  If the coroutine is in an error state (not simply suspended),
/// its to-be-closed variables are cleaned up before propagation.
///
/// C: `static int luaB_auxwrap(lua_State *L)`
fn aux_wrap(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: lua_State *co = lua_tothread(L, lua_upvalueindex(1));
    // TODO(port): coroutine stub — upvalue_index(1) gives the first C-closure
    // upvalue pseudo-index; to_thread reads GcRef<LuaState> from that slot.
    let co = match state.to_thread(upvalue_index(1)) {
        Some(c) => c,
        None => panic!("coroutine.wrap: upvalue 1 is not a thread (Phase A–D stub)"),
    };
    // C: int r = auxresume(L, co, lua_gettop(L));
    let narg = state.get_top();
    let r = aux_resume(state, co.clone(), narg);
    if r < 0 {
        // C: int stat = lua_status(co);
        // TODO(port): coroutine stub — thread_status() reads the co thread's status.
        let mut stat = co.thread_status();
        // C: if (stat != LUA_OK && stat != LUA_YIELD) { close; xmove error }
        if stat != LuaStatus::Ok && stat != LuaStatus::Yield {
            // C: stat = lua_closethread(co, L);
            // TODO(port): coroutine stub — close_thread runs co's to-be-closed
            // variable finalizers and returns the resulting status; Phase E needed.
            stat = co.close_thread(state);
            // C: lua_assert(stat != LUA_OK);
            debug_assert!(stat != LuaStatus::Ok);
            // C: lua_xmove(co, L, 1);  /* move error message to the caller */
            // TODO(port): coroutine stub — xmove between two live threads requires
            // Phase E coroutine support.
        }
        // C: if (stat != LUA_ERRMEM && lua_type(L, -1) == LUA_TSTRING) { ... }
        if stat != LuaStatus::ErrMem && matches!(state.type_at(-1), LuaType::String) {
            // C: luaL_where(L, 1); lua_insert(L, -2); lua_concat(L, 2);
            // TODO(port): where_error(1) pushes a "source:line:" location prefix;
            // Phase B wire-up for where_error / concat needed.
            state.where_error(1);
            state.insert(-2);
            state.concat(2)?;
        }
        // C: return lua_error(L);
        Err(LuaError::from_value(state.pop()))
    } else {
        // C: return r;
        Ok(r as usize)
    }
}

/// `coroutine.create(f)` — create a new coroutine that will run function `f`.
///
/// Pushes the new thread value and returns 1.
///
/// C: `static int luaB_cocreate(lua_State *L)`
pub fn co_create(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: luaL_checktype(L, 1, LUA_TFUNCTION);
    state.check_arg_type(1, LuaType::Function)?;
    // C: NL = lua_newthread(L);
    // TODO(port): coroutine stub — new_thread allocates a fresh LuaState coroutine
    // and pushes a Thread value for it; Phase E needed.
    let _nl = state.new_thread()?;
    // C: lua_pushvalue(L, 1);  /* move function to top */
    state.push_value(1);
    // C: lua_xmove(L, NL, 1);  /* move function from L to NL */
    // TODO(port): coroutine stub — xmove transfers the function from L's stack to
    // NL's stack so it becomes the coroutine body; Phase E needed.
    // state.xmove(&nl, 1)?;
    Ok(1)
}

/// `coroutine.wrap(f)` — create a coroutine and return a resuming function.
///
/// The returned function, when called, resumes the coroutine as if by
/// `coroutine.resume`, but raises an error rather than returning `false`.
///
/// C: `static int luaB_cowrap(lua_State *L)`
pub fn co_wrap(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: luaB_cocreate(L);
    co_create(state)?;
    // C: lua_pushcclosure(L, luaB_auxwrap, 1);
    // TODO(port): push_cclosure(aux_wrap, 1) creates a C closure with the thread
    // (currently on top of the stack) as upvalue 1; Phase B wire-up needed.
    state.push_cclosure(aux_wrap, 1);
    Ok(1)
}

/// `coroutine.yield([...])` — suspend the running coroutine.
///
/// All arguments are passed back as results of the corresponding `resume`.
///
/// C: `static int luaB_yield(lua_State *L)`
pub fn co_yield(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: return lua_yield(L, lua_gettop(L));
    // TODO(port): coroutine stub — state.yield_(n) suspends the coroutine and
    // transfers n values back to the caller of resume; Phase E needed.
    let _nargs = state.get_top();
    panic!(
        "coroutine.yield is not yet implemented (Phase A–D stub; see PORTING.md §2 #6)"
    );
}

/// `coroutine.status(co)` — return a string describing `co`'s current status.
///
/// Returns one of `"running"`, `"dead"`, `"suspended"`, or `"normal"`.
///
/// C: `static int luaB_costatus(lua_State *L)`
pub fn co_status(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: lua_State *co = getco(L);
    let co = get_co(state)?;
    // C: lua_pushstring(L, statname[auxstatus(L, co)]);
    let idx = aux_status(state, &co) as usize;
    let name: &[u8] = STAT_NAMES[idx];
    state.push(LuaValue::Str(state.intern_str(name)));
    Ok(1)
}

/// `coroutine.isyieldable([co])` — test whether a coroutine (default: current)
/// is in a yieldable state.
///
/// C: `static int luaB_yieldable(lua_State *L)`
pub fn co_isyieldable(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: lua_State *co = lua_isnone(L, 1) ? L : getco(L);
    let is_yieldable = if matches!(state.type_at(1), LuaType::None) {
        // Checking the calling thread itself.
        // TODO(port): state.is_yieldable() reads the nCcalls non-yieldable counter;
        // Phase B wire-up needed.
        state.is_yieldable()
    } else {
        let co = get_co(state)?;
        // TODO(port): coroutine stub — is_yieldable() on a different thread requires
        // cross-thread field access (nCcalls.nny == 0); Phase E needed.
        co.is_yieldable()
    };
    // C: lua_pushboolean(L, lua_isyieldable(co));
    state.push(LuaValue::Bool(is_yieldable));
    Ok(1)
}

/// `coroutine.running()` — return the current coroutine plus a boolean.
///
/// The boolean is `true` when the current coroutine is the main thread.
///
/// C: `static int luaB_corunning(lua_State *L)`
pub fn co_running(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: int ismain = lua_pushthread(L);
    // TODO(port): push_thread pushes a Thread value for the current LuaState and
    // returns true iff it is the main thread; Phase B wire-up needed.
    let is_main = state.push_thread()?;
    // C: lua_pushboolean(L, ismain);
    state.push(LuaValue::Bool(is_main));
    Ok(2)
}

/// `coroutine.close(co)` — close a dead or suspended coroutine.
///
/// Runs the to-be-closed variable finalizers.  Returns `true` on success, or
/// `false` plus an error object on failure.  Raises an error if `co` is
/// running or normal.
///
/// C: `static int luaB_close(lua_State *L)`
pub fn co_close(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: lua_State *co = getco(L);
    let co = get_co(state)?;
    // C: int status = auxstatus(L, co);
    let status = aux_status(state, &co);
    match status {
        // C: case COS_DEAD: case COS_YIELD:
        s if s == COS_DEAD || s == COS_YIELD => {
            // C: status = lua_closethread(co, L);
            // TODO(port): coroutine stub — close_thread runs the coroutine's TBC
            // finalizers and returns a LuaStatus; Phase E needed.
            let close_status = co.close_thread(state);
            if close_status == LuaStatus::Ok {
                // C: lua_pushboolean(L, 1); return 1;
                state.push(LuaValue::Bool(true));
                Ok(1)
            } else {
                // C: lua_pushboolean(L, 0); lua_xmove(co, L, 1); return 2;
                state.push(LuaValue::Bool(false));
                // TODO(port): coroutine stub — xmove(co, L, 1) moves the error
                // message from co's stack to L's stack; Phase E needed.
                Ok(2)
            }
        }
        _ => {
            // C: return luaL_error(L, "cannot close a %s coroutine", statname[status]);
            // PORT NOTE: STAT_NAMES entries are ASCII Rust statics used only for
            // error message construction, so a match to &str is used here instead of
            // from_utf8 on &[u8] (which is banned for Lua data paths per PORTING.md).
            let name = match status {
                COS_RUN => "running",
                COS_NORM => "normal",
                _ => "unknown",
            };
            Err(LuaError::runtime(format_args!(
                "cannot close a {} coroutine",
                name
            )))
        }
    }
}

// ── Module entry point ────────────────────────────────────────────────────────

/// Opens the `coroutine` standard library by pushing a new table containing
/// all `coroutine.*` functions.
///
/// C: `LUAMOD_API int luaopen_coroutine(lua_State *L)` — `LUAMOD_API` → `pub`.
pub fn open_coroutine(state: &mut LuaState) -> Result<usize, LuaError> {
    // C: luaL_newlib(L, co_funcs);
    // TODO(port): state.new_lib(CO_FUNCS) creates a table from the registration
    // slice and leaves it on the stack; Phase B wire-up needed.
    state.new_lib(CO_FUNCS)?;
    Ok(1)
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lcorolib.c  (210 lines, 12 functions)
//   target_crate:  lua-stdlib
//   confidence:    medium
//   todos:         21
//   port_notes:    2
//   unsafe_blocks: 0
//   notes:         All coroutine execution primitives (resume, yield, xmove,
//                  new_thread, close_thread) are Phase E stubs that panic.
//                  Argument-checking / result-packaging logic is faithfully
//                  translated so Phase E can drop in real implementations.
//                  The CO_FUNCS table type references lua_CFunction which is
//                  resolved in Phase B.  LuaState / GcRef<LuaState> / LuaStatus
//                  imports are all deferred to Phase B.
// ──────────────────────────────────────────────────────────────────────────────
