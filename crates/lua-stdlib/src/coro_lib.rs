//! Coroutine library — the `coroutine.*` standard-library table: `create`,
//! `resume`, `running`, `status`, `wrap`, `yield`, `isyieldable`, and `close`.
//!
//! This module is the **cold shell** around coroutine execution: argument
//! checking, the `COS_*` status-string mapping, the `wrap` closure setup, the
//! cross-thread argument/result transfer scaffolding, and version-gated
//! registration. The actual control transfer — resume/yield stack save and
//! restore — lives in `lua-vm` (`lua_vm::do_::lua_resume` / `lua_yieldk`) and is
//! load-bearing; this module calls into it but does not implement it.
//!
//! # Graduation (Idiomatization Sprint 2, Phase 2 — `coroutine`)
//!
//! Idiomatized AROUND the resume/yield machinery, never through it. The
//! behavioral net guarding this module's cold surface is
//! `crates/lua-stdlib/tests/coro_strengthen.rs` (the version seams:
//! `running` arity 5.1-vs-5.2+, `isyieldable` 5.3+, `close` 5.4+ + its
//! suspended→dead transition and the 5.4-errors/5.5-unwinds self-close, the
//! resume/wrap error wording, status transitions across a yield) plus the
//! official `coroutine.lua` suite and `multiversion_oracle`. Net-strengthening
//! caught one real bug: the resume-of-running error used the 5.2+ wording on
//! 5.1 — fixed via `non_suspended_resume_message`. Left load-bearing: the
//! cross-thread snapshot/rooting (`RootedThreadBorrow`, the resume-pool
//! buffers, the GC stack snapshots), the `LuaThreadClose` panic-unwind path
//! that implements 5.5 self-close, and every version gate.

use std::cell::Cell;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use crate::state_stub::{lua_CFunction, upvalue_index, LuaState, LuaStateStubExt as _};
use lua_types::{error::LuaError, gc::GcRef, value::LuaValue, LuaStatus, LuaThreadClose, LuaType};

thread_local! {
    /// Per-thread suppression depth for [`LuaThreadClose`] unwind payloads.
    ///
    /// Incremented for the duration of each `catch_unwind` resume window by a
    /// [`SuppressGuard`], decremented (on every path, including a panic
    /// unwinding through the guard) when the guard drops. The process-global
    /// chaining hook installed by [`ensure_chaining_panic_hook`] silently
    /// swallows a `LuaThreadClose` payload only while this counter is non-zero
    /// **on the panicking thread**, and delegates every other payload — and
    /// `LuaThreadClose` outside a resume window — to the previously installed
    /// hook.
    ///
    /// It is a counter rather than a bool because resumes nest: a coroutine
    /// resumed from inside another resume must keep the suppression active for
    /// the outer window after the inner one exits. Because the state is
    /// thread-local, a `LuaThreadClose` unwind suppressed on one OS thread
    /// never silences a simultaneous unrelated panic on another OS thread —
    /// that thread reads its own zero counter and reaches the previous hook.
    static THREAD_CLOSE_SUPPRESS: Cell<u32> = const { Cell::new(0) };
}

/// One-shot install guard for the process-global chaining panic hook.
static CHAINING_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

/// RAII increment of [`THREAD_CLOSE_SUPPRESS`] for one resume window.
///
/// Constructing the guard increments the per-thread counter; dropping it
/// decrements. `catch_unwind` returns normally even when it catches a panic,
/// so the decrement in `Drop` covers both the caught-panic and the
/// normal-return paths; an uncaught panic unwinding through the guard runs the
/// same `Drop`, so the counter invariant holds on every exit.
struct SuppressGuard;

impl SuppressGuard {
    fn new() -> Self {
        THREAD_CLOSE_SUPPRESS.with(|c| c.set(c.get() + 1));
        SuppressGuard
    }
}

impl Drop for SuppressGuard {
    fn drop(&mut self) {
        THREAD_CLOSE_SUPPRESS.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

/// Install — exactly once for the process — a chaining panic hook that
/// suppresses the default panic printout for [`LuaThreadClose`] unwind
/// payloads while a resume window is active on the panicking thread, and
/// delegates everything else to the hook that was current at install time.
///
/// `LuaThreadClose` is the internal unwind used by `coroutine.close` (5.5
/// self-close) and coroutine teardown; it is control flow, not a Rust runtime
/// fault, so it must never reach the default printer. The previous per-resume
/// implementation paid 3–4 heap allocations plus four global hook-lock
/// operations on every resume to install and tear this suppression down around
/// each `catch_unwind`. This installs the hook once and scopes the suppression
/// with a thread-local counter ([`THREAD_CLOSE_SUPPRESS`]) instead, so the
/// per-resume cost is two TLS counter writes.
///
/// Suppression is gated on the counter so it is active only inside a resume
/// window: a `LuaThreadClose` that somehow escaped a resume would still reach
/// the previous hook, and — because the counter is thread-local — a
/// `LuaThreadClose` suppressed on one OS thread never silences a simultaneous
/// unrelated panic on another OS thread.
///
/// Accepted tradeoff (T2-B2): an embedder that calls `std::panic::set_hook`
/// **after** lua-rs's first resume displaces this chained hook permanently —
/// the previous implementation re-installed the suppression on every resume,
/// so it won each resume window even against a later embedder hook. Embedders
/// that need a custom hook should install it before the first resume; the
/// chaining hook then captures and delegates to it.
fn ensure_chaining_panic_hook() {
    CHAINING_HOOK_INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let suppress = info.payload().downcast_ref::<LuaThreadClose>().is_some()
                && THREAD_CLOSE_SUPPRESS.with(|c| c.get()) > 0;
            if !suppress {
                previous(info);
            }
        }));
    });
}

// ── Coroutine status codes ────────────────────────────────────────────────────

/// Coroutine is the currently running thread.
const COS_RUN: i32 = 0;

/// Coroutine has finished execution or encountered an error.
const COS_DEAD: i32 = 1;

/// Coroutine is suspended — either yielded or not yet started.
const COS_YIELD: i32 = 2;

/// Coroutine is normal — it resumed another coroutine and is waiting.
const COS_NORM: i32 = 3;

/// Human-readable status strings indexed by the `COS_*` constants above,
/// pushed onto the Lua stack as byte strings by `coroutine.status`.
const STAT_NAMES: [&[u8]; 4] = [b"running", b"dead", b"suspended", b"normal"];

// ── Registration table ────────────────────────────────────────────────────────

/// Registration table for the `coroutine` standard library — one
/// `(name_bytes, function_pointer)` entry per `coroutine.*` function. The
/// per-version roster (which entries actually register) is filtered in
/// [`open_coroutine`]; this table is the full superset.
pub const CO_FUNCS: &[(&[u8], lua_CFunction)] = &[
    (b"create", co_create),
    (b"resume", co_resume),
    (b"running", co_running),
    (b"status", co_status),
    (b"wrap", co_wrap),
    (b"yield", co_yield),
    (b"isyieldable", co_isyieldable),
    (b"close", co_close),
];

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Retrieves the coroutine thread at stack index 1, raising a type error if
/// the argument is absent or not a thread.
///
/// The error routes through `arg_error_impl` so it carries the calling
/// function's name (`bad argument #1 to 'coroutine.resume' (...)` on 5.2+; `'?'`
/// on 5.1). The `extramsg` body is version-gated to match each reference:
/// 5.1/5.2 say `coroutine expected`, 5.3 says `thread expected`, and 5.4/5.5 use
/// `luaL_argexpected` which appends `, got <type>`.
fn get_co(state: &mut LuaState) -> Result<GcRef<lua_types::value::LuaThread>, LuaError> {
    let co = state.to_thread(1);
    if let Some(co) = co {
        return Ok(co);
    }
    Err(thread_arg_error(state, 1))
}

/// Build the version-correct "expected a coroutine/thread" argument error for
/// argument `arg`, carrying the calling function's name via `arg_error_impl`.
///
/// See [`get_co`] for the per-version message forms.
fn thread_arg_error(state: &mut LuaState, arg: i32) -> LuaError {
    use lua_types::LuaVersion;
    let version = state.global().lua_version;
    if matches!(version, LuaVersion::V51 | LuaVersion::V52) {
        return lua_vm::debug::arg_error_impl(state, arg, b"coroutine expected");
    }
    if matches!(version, LuaVersion::V53) {
        return lua_vm::debug::arg_error_impl(state, arg, b"thread expected");
    }
    let got = state.value_at(arg);
    let got_name = match state.full_type_name(&got) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let mut extramsg = b"thread expected, got ".to_vec();
    extramsg.extend_from_slice(&got_name);
    lua_vm::debug::arg_error_impl(state, arg, &extramsg)
}

fn get_opt_co(state: &mut LuaState) -> Result<GcRef<lua_types::value::LuaThread>, LuaError> {
    if matches!(state.global().lua_version, lua_types::LuaVersion::V55)
        && state.type_at(1) == LuaType::None
    {
        let id = state.global().current_thread_id;
        return state
            .global()
            .thread_value_for(id)
            .ok_or_else(|| LuaError::runtime(format_args!("current thread is not registered")));
    }
    get_co(state)
}

/// Returns one of the `COS_*` status codes describing `co` relative to the
/// calling thread `state`, reading the target coroutine's `status`,
/// call-frame depth, and stack top through `GlobalState::threads`:
///
/// - `co` is the current thread → `COS_RUN` (running).
/// - `co` is the main thread (never stored in the registry) → `COS_NORM`.
/// - `co` is not in the registry → `COS_DEAD`.
/// - otherwise classify by the registered thread's `status`: a yielded thread
///   is `COS_YIELD`; a thread with live frames (it resumed a child) is
///   `COS_NORM`; an `Ok` thread with no frames is `COS_DEAD` if its stack is
///   empty, else `COS_YIELD` (suspended at its initial frame, function still
///   staged on the stack).
///
/// The transition table this produces is pinned by `status_transitions_*` in
/// `tests/coro_strengthen.rs`.
fn aux_status(state: &mut LuaState, co: &GcRef<lua_types::value::LuaThread>) -> i32 {
    let co_id = co.id;
    let entry_rc = {
        let g = state.global();
        if co_id == g.current_thread_id {
            return COS_RUN;
        }
        if co_id == g.main_thread_id {
            return COS_NORM;
        }
        match g.threads.get(&co_id) {
            Some(e) => e.state.clone(),
            None => return COS_DEAD,
        }
    };
    let co_state = match entry_rc.try_borrow() {
        Ok(state) => state,
        Err(_) => {
            // A thread already mutably borrowed is one that resumed a child and
            // is waiting up the call stack — i.e. a normal (active, not
            // suspended/dead) coroutine, so report COS_NORM.
            return COS_NORM;
        }
    };
    let raw_status = co_state.status;
    if raw_status == LuaStatus::Yield as u8 {
        return COS_YIELD;
    }
    if raw_status != LuaStatus::Ok as u8 {
        return COS_DEAD;
    }
    let has_frames = co_state.ci.as_usize() > 0;
    if has_frames {
        return COS_NORM;
    }
    let ci_func = co_state.call_info[0].func.0;
    let top = co_state.top.0;
    let lua_gettop = top as i64 - ci_func as i64 - 1;
    if lua_gettop == 0 {
        COS_DEAD
    } else {
        COS_YIELD
    }
}

/// Transfers `narg` arguments from `state` to `co`, resumes the coroutine,
/// then transfers results (or error message) back to `state`.
///
/// Returns the number of result values (≥ 0) on success, or `-1` on error
/// with the error object left on top of `state`'s stack.
///
/// Cross-thread open-upvalue mirroring rides the resume boundary: before
/// yielding control, the parent's open-upvalue values are snapshotted into
/// `GlobalState::cross_thread_upvals` so the coroutine body can read and write
/// them through `LuaState::upvalue_get` / `upvalue_set`. On resume return, the
/// (possibly mutated) cache entries are flushed back into the parent's stack.
/// This is the alternative to a stack-refactor that would let the parent's
/// `LuaState` be reached through `Rc<RefCell<_>>` while it is held by `&mut`
/// further up the call stack. Load-bearing: do not collapse the snapshot /
/// flush handshake — it is what keeps cross-thread upvalues coherent and
/// rooted across the resume.
fn aux_resume(state: &mut LuaState, co: GcRef<lua_types::value::LuaThread>, narg: i32) -> i32 {
    let co_id = co.id;
    let entry_rc = {
        let g = state.global();
        match g.threads.get(&co_id) {
            Some(e) => e.state.clone(),
            None => {
                drop(g);
                push_lit_or_nil(state, b"cannot resume dead coroutine");
                return -1;
            }
        }
    };
    let parent_thread_id = state.global().current_thread_id;
    let top_before = state.get_top();
    if top_before < narg {
        push_lit_or_nil(state, b"not enough arguments to resume");
        return -1;
    }
    let first_arg_idx = top_before - narg + 1;
    let mut args = pop_resume_value_buf(state);
    args.extend((first_arg_idx..=top_before).map(|i| state.value_at(i)));
    lua_vm::api::set_top(state, (top_before - narg) as i32).ok();

    let mut parent_open_upval_slots = pop_resume_slot_buf(state);
    parent_open_upval_slots.extend(state.openupval.iter().filter_map(|uv| {
        uv.try_open_payload()
            .map(|(thread_id, idx)| (thread_id as u64, idx))
    }));
    {
        let mut g = state.global_mut();
        for (tid, idx) in &parent_open_upval_slots {
            let val = state.get_at(*idx);
            g.cross_thread_upvals.insert((*tid, *idx), val);
        }
    }

    push_parent_gc_snapshot(state);

    let (status, results_or_err): (LuaStatus, Vec<LuaValue>) = {
        let mut co_state = match entry_rc.try_borrow_mut() {
            Ok(b) => b,
            Err(_) => {
                pop_parent_gc_snapshot(state);
                let mut g = state.global_mut();
                for (tid, idx) in &parent_open_upval_slots {
                    g.cross_thread_upvals.remove(&(*tid, *idx));
                }
                drop(g);
                return_resume_slot_buf(state, parent_open_upval_slots);
                return_resume_value_buf(state, args);
                let msg = non_suspended_resume_message(state);
                push_lit_or_nil(state, msg);
                return -1;
            }
        };
        if co_state.check_stack(narg + 1).is_err() {
            drop(co_state);
            pop_parent_gc_snapshot(state);
            let mut g = state.global_mut();
            for (tid, idx) in &parent_open_upval_slots {
                g.cross_thread_upvals.remove(&(*tid, *idx));
            }
            drop(g);
            return_resume_slot_buf(state, parent_open_upval_slots);
            return_resume_value_buf(state, args);
            push_lit_or_nil(state, b"too many arguments to resume");
            return -1;
        }
        for v in args.drain(..) {
            co_state.push(v);
        }
        return_resume_value_buf(state, args);
        co_state.global_mut().current_thread_id = co_id;
        let mut nres: i32 = 0;
        ensure_chaining_panic_hook();
        let resume_result = {
            let _suppress = SuppressGuard::new();
            catch_unwind(AssertUnwindSafe(|| {
                lua_vm::do_::lua_resume(&mut *co_state, Some(state), narg, &mut nres)
            }))
        };
        co_state.global_mut().current_thread_id = parent_thread_id;
        let status = match resume_result {
            Ok(status) => status,
            Err(payload) => {
                if let Some(close) = payload.downcast_ref::<LuaThreadClose>() {
                    close.0
                } else {
                    resume_unwind(payload);
                }
            }
        };
        let co_top = co_state.top_idx().0 as i32;
        let ci_func = co_state.current_call_info().func.0 as i32;
        let count = if status == LuaStatus::Ok || status == LuaStatus::Yield {
            nres
        } else {
            1
        };
        let start = co_top - count;
        let mut vals = pop_resume_value_buf(state);
        vals.extend((start..co_top).map(|i| co_state.get_at(lua_vm::state::StackIdx(i as u32))));
        let new_co_top = if status == LuaStatus::Ok || status == LuaStatus::Yield {
            (co_top - count).max(ci_func + 1)
        } else {
            co_top - count
        };
        co_state.set_top(lua_vm::state::StackIdx(new_co_top.max(0) as u32));
        (status, vals)
    };

    // Pop the parent stack snapshot — the coroutine has yielded or returned.
    pop_parent_gc_snapshot(state);

    {
        let mut flush = pop_resume_flush_buf(state);
        let mut g = state.global_mut();
        for (tid, idx) in &parent_open_upval_slots {
            if let Some(v) = g.cross_thread_upvals.remove(&(*tid, *idx)) {
                flush.push((*idx, v));
            }
        }
        drop(g);
        for (idx, v) in flush.drain(..) {
            state.set_at(idx, v);
        }
        return_resume_flush_buf(state, flush);
    }
    return_resume_slot_buf(state, parent_open_upval_slots);

    let mut results_or_err = results_or_err;
    match status {
        LuaStatus::Ok | LuaStatus::Yield => {
            if state.check_stack(results_or_err.len() as i32 + 1).is_err() {
                return_resume_value_buf(state, results_or_err);
                push_lit_or_nil(state, b"too many results to resume");
                return -1;
            }
            let n = results_or_err.len();
            for v in results_or_err.drain(..) {
                state.push(v);
            }
            return_resume_value_buf(state, results_or_err);
            n as i32
        }
        _ => {
            for v in results_or_err.drain(..) {
                state.push(v);
            }
            return_resume_value_buf(state, results_or_err);
            -1
        }
    }
}

fn push_parent_gc_snapshot(state: &mut LuaState) {
    let top = (state.top_idx().0 as usize).min(state.stack.len());
    let (mut stack_snapshot, mut upval_snapshot) = {
        let mut g = state.global_mut();
        (
            g.snapshot_stack_pool.pop().unwrap_or_default(),
            g.snapshot_upval_pool.pop().unwrap_or_default(),
        )
    };
    stack_snapshot.extend(state.stack[..top].iter().map(|sv| sv.val));
    upval_snapshot.extend(state.openupval.iter().cloned());
    let mut g = state.global_mut();
    g.suspended_parent_stacks.push(stack_snapshot);
    g.suspended_parent_open_upvals.push(upval_snapshot);
}

fn pop_parent_gc_snapshot(state: &mut LuaState) {
    let mut g = state.global_mut();
    if let Some(mut v) = g.suspended_parent_open_upvals.pop() {
        v.clear();
        g.snapshot_upval_pool.push(v);
    }
    if let Some(mut v) = g.suspended_parent_stacks.pop() {
        v.clear();
        g.snapshot_stack_pool.push(v);
    }
}

/// Borrow an empty open-upvalue slot buffer from the resume pool, or a fresh
/// one if the pool is empty (first resume at this nesting depth). The returned
/// buffer must be parked with [`return_resume_slot_buf`] on every exit path so
/// the capacity is retained instead of freed.
fn pop_resume_slot_buf(state: &mut LuaState) -> Vec<(u64, lua_vm::state::StackIdx)> {
    state.global_mut().resume_upval_slot_pool.pop().unwrap_or_default()
}

/// Park a (drained) open-upvalue slot buffer back in the resume pool, clearing
/// it first so the pooled buffer is always empty and roots nothing.
fn return_resume_slot_buf(state: &mut LuaState, mut buf: Vec<(u64, lua_vm::state::StackIdx)>) {
    buf.clear();
    state.global_mut().resume_upval_slot_pool.push(buf);
}

/// Borrow an empty `LuaValue` buffer from the resume pool for an argument or
/// result list, or a fresh one if the pool is empty (first use at this nesting
/// depth). Park with [`return_resume_value_buf`] once the buffer is drained.
fn pop_resume_value_buf(state: &mut LuaState) -> Vec<LuaValue> {
    state.global_mut().resume_value_pool.pop().unwrap_or_default()
}

/// Park a (drained) `LuaValue` buffer back in the resume pool, clearing it so
/// the pooled buffer is always empty and roots nothing.
fn return_resume_value_buf(state: &mut LuaState, mut buf: Vec<LuaValue>) {
    buf.clear();
    state.global_mut().resume_value_pool.push(buf);
}

/// Borrow an empty cross-thread upvalue flush buffer from the resume pool, or a
/// fresh one if the pool is empty. Park with [`return_resume_flush_buf`] once
/// the buffer has been drained back onto the parent stack.
fn pop_resume_flush_buf(state: &mut LuaState) -> Vec<(lua_vm::state::StackIdx, LuaValue)> {
    state.global_mut().resume_flush_pool.pop().unwrap_or_default()
}

/// Park a (drained) flush buffer back in the resume pool, clearing it so the
/// pooled buffer is always empty and roots nothing.
fn return_resume_flush_buf(state: &mut LuaState, mut buf: Vec<(lua_vm::state::StackIdx, LuaValue)>) {
    buf.clear();
    state.global_mut().resume_flush_pool.push(buf);
}

/// RAII borrow of another thread's `LuaState` that keeps the thread's stack
/// rooted while the borrow is held.
///
/// A coroutine whose `RefCell` is mutably borrowed at collect time cannot be
/// traced by `trace_reachable_threads` — its stack is invisible to the
/// marker for that whole cycle, so any object only it references is swept
/// while still live (issue #140 bug A: `debug.traceback(co)` held the borrow
/// across `push_vfstring`'s GC checkpoint). This guard rides the same
/// rooting structure as `coroutine.resume`: it pushes a snapshot of the
/// target's live stack and open upvalues onto `suspended_parent_stacks` for
/// the lifetime of the borrow and pops it on drop. Snapshots are strictly
/// LIFO — callers must not resume a coroutine while a guard is alive.
///
/// If the guarded section pushes new values onto the *target's* stack and
/// then allocates before consuming them (`lua_getinfo`'s 'L'/'f' pushes),
/// call [`RootedThreadBorrow::resnapshot`] after the pushes so the snapshot
/// covers them too.
#[cfg(feature = "debug")]
pub(crate) struct RootedThreadBorrow<'a> {
    inner: std::cell::RefMut<'a, LuaState>,
}

#[cfg(feature = "debug")]
impl RootedThreadBorrow<'_> {
    /// Re-copy the target's current live stack and open upvalues into the
    /// snapshot pushed at borrow time, covering values pushed onto the
    /// target since then.
    pub(crate) fn resnapshot(&mut self) {
        let top = (self.inner.top_idx().0 as usize).min(self.inner.stack.len());
        let stack_copy: Vec<LuaValue> = self.inner.stack[..top].iter().map(|sv| sv.val).collect();
        let upval_copy: Vec<GcRef<lua_types::UpVal>> = self.inner.openupval.to_vec();
        let mut g = self.inner.global_mut();
        if let Some(slot) = g.suspended_parent_stacks.last_mut() {
            slot.clear();
            slot.extend(stack_copy);
        }
        if let Some(slot) = g.suspended_parent_open_upvals.last_mut() {
            slot.clear();
            slot.extend(upval_copy);
        }
    }
}

#[cfg(feature = "debug")]
impl std::ops::Deref for RootedThreadBorrow<'_> {
    type Target = LuaState;
    fn deref(&self) -> &LuaState {
        &self.inner
    }
}

#[cfg(feature = "debug")]
impl std::ops::DerefMut for RootedThreadBorrow<'_> {
    fn deref_mut(&mut self) -> &mut LuaState {
        &mut self.inner
    }
}

#[cfg(feature = "debug")]
impl Drop for RootedThreadBorrow<'_> {
    fn drop(&mut self) {
        let mut g = self.inner.global_mut();
        if let Some(mut v) = g.suspended_parent_open_upvals.pop() {
            v.clear();
            g.snapshot_upval_pool.push(v);
        }
        if let Some(mut v) = g.suspended_parent_stacks.pop() {
            v.clear();
            g.snapshot_stack_pool.push(v);
        }
    }
}

/// Borrow `cell`'s thread state mutably with its stack rooted for the
/// duration (see [`RootedThreadBorrow`]). Panics if the cell is already
/// borrowed, matching the bare `borrow_mut()` call sites this replaces.
#[cfg(feature = "debug")]
pub(crate) fn borrow_thread_rooted<'a>(
    state: &mut LuaState,
    cell: &'a std::cell::RefCell<LuaState>,
) -> RootedThreadBorrow<'a> {
    let inner = cell.borrow_mut();
    let top = (inner.top_idx().0 as usize).min(inner.stack.len());
    let (mut stack_snapshot, mut upval_snapshot) = {
        let mut g = state.global_mut();
        (
            g.snapshot_stack_pool.pop().unwrap_or_default(),
            g.snapshot_upval_pool.pop().unwrap_or_default(),
        )
    };
    stack_snapshot.extend(inner.stack[..top].iter().map(|sv| sv.val));
    upval_snapshot.extend(inner.openupval.iter().cloned());
    let mut g = state.global_mut();
    g.suspended_parent_stacks.push(stack_snapshot);
    g.suspended_parent_open_upvals.push(upval_snapshot);
    drop(g);
    RootedThreadBorrow { inner }
}

/// The wording for "tried to resume a coroutine that is the running (or an
/// active normal) thread", which changed between versions: Lua 5.1 says
/// `cannot resume running coroutine`; 5.2 generalized it to
/// `cannot resume non-suspended coroutine` (the same message now covers a
/// normal coroutine too). Pinned by `double_resume_running_message_by_version`
/// in `tests/coro_strengthen.rs` against lua5.1.5 vs lua5.2.4+.
fn non_suspended_resume_message(state: &LuaState) -> &'static [u8] {
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        b"cannot resume running coroutine"
    } else {
        b"cannot resume non-suspended coroutine"
    }
}

/// Helper: push a string literal or fall back to Nil on intern failure.
fn push_lit_or_nil(state: &mut LuaState, bytes: &[u8]) {
    match state.intern_str(bytes) {
        Ok(s) => state.push(LuaValue::Str(s)),
        Err(_) => state.push(LuaValue::Nil),
    }
}

// ── Public library functions ──────────────────────────────────────────────────

/// `coroutine.resume(co [, val1, ...])` — attempt to resume coroutine `co`.
///
/// On success pushes `true` followed by all values yielded or returned by `co`.
/// On failure pushes `false` followed by the error object.
///
/// The argument count handed to [`aux_resume`] is the stack top minus one: the
/// coroutine itself sits at index 1 and is not forwarded as an argument.
///
/// A sandbox budget trip is uncatchable: it re-raises into the caller frame
/// instead of returning `false, msg`, so code cannot keep a runaway coroutine
/// alive by resuming it in a loop.
pub fn co_resume(state: &mut LuaState) -> Result<usize, LuaError> {
    let co = get_co(state)?;
    let narg = state.get_top() - 1;
    let r = aux_resume(state, co, narg);
    if r < 0 {
        if state.sandbox_aborting() {
            let top = state.get_top();
            let err_val = state.value_at(top);
            return Err(LuaError::from_value(err_val));
        }
        state.push(LuaValue::Bool(false));
        state.insert(-2)?;
        Ok(2)
    } else {
        state.push(LuaValue::Bool(true));
        state.insert(-(r + 1))?;
        Ok((r + 1) as usize)
    }
}

/// Closure body installed by `coroutine.wrap`. The wrapped coroutine
/// thread is stored in upvalue slot 1 as a `LuaValue::Thread`.
///
/// On call: forwards all args to `aux_resume` on the captured thread. On
/// success returns the yielded/returned values; on coroutine error raises
/// the error (matching `select(2, assert(resume(co, ...)))` semantics).
///
fn aux_wrap(state: &mut LuaState) -> Result<usize, LuaError> {
    let up = state.value_at(upvalue_index(1));
    let co = match up {
        LuaValue::Thread(t) => t,
        _ => {
            return Err(LuaError::runtime(format_args!(
                "coroutine.wrap: upvalue is not a thread"
            )))
        }
    };
    let narg = state.get_top();
    let r = aux_resume(state, co.clone(), narg);
    if r < 0 {
        let top = state.get_top();
        let mut err_val = state.value_at(top);
        if aux_status(state, &co) == COS_DEAD {
            let old_err = state.pop();
            let nclose = close_suspended_or_dead(state, co)?;
            err_val = if nclose >= 2 {
                let top = state.get_top();
                state.value_at(top)
            } else {
                old_err
            };
            state.pop_n(nclose);
        }
        Err(LuaError::from_value(err_val))
    } else {
        Ok(r as usize)
    }
}

/// `coroutine.create(f)` — create a new coroutine that will run function `f`.
///
/// Allocates a real `LuaState` registered in `GlobalState::threads`, with `f`
/// staged on the new thread's stack so `coroutine.status` reports
/// `"suspended"`. Pushes the new thread value and returns 1.
pub fn co_create(state: &mut LuaState) -> Result<usize, LuaError> {
    state.check_arg_type(1, LuaType::Function)?;
    // 5.1's `luaB_cocreate` additionally rejects C functions
    // (`luaL_argcheck(L, ... && !lua_iscfunction(L, 1), 1, "Lua function
    // expected")`); only Lua closures may become coroutine bodies. 5.2 moved
    // coroutines to `lcorolib.c` and dropped that restriction, so a C function
    // is accepted from 5.2 on. Verified against lua5.1.5 / lua5.2.4.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51)
        && state.is_c_function_at(1)
    {
        return Err(lua_vm::debug::arg_error_impl(
            state,
            1,
            b"Lua function expected",
        ));
    }
    let body = state.value_at(1);
    let _nl = state.new_thread(Some(body))?;
    Ok(1)
}

/// `coroutine.wrap(f)` — create a coroutine and return a resuming function.
///
/// The returned function, when called, resumes the coroutine as if by
/// `coroutine.resume`, but raises an error rather than returning `false`.
///
///
/// Captures the new coroutine thread as upvalue 1 of `aux_wrap`.
pub fn co_wrap(state: &mut LuaState) -> Result<usize, LuaError> {
    co_create(state)?;
    state.push_cclosure(aux_wrap, 1)?;
    Ok(1)
}

/// `coroutine.yield([...])` — suspend the running coroutine.
///
/// All arguments are passed back as results of the corresponding `resume`.
///
/// → `return lua_yield(L, lua_gettop(L));`
/// → `lua_yield(L,n)` is `lua_yieldk(L, n, 0, NULL)` (lua.h:316)
pub fn co_yield(state: &mut LuaState) -> Result<usize, LuaError> {
    let n = state.get_top();
    let r = lua_vm::do_::lua_yieldk(state, n, 0, None)?;
    Ok(r as usize)
}

/// `coroutine.status(co)` — return a string describing `co`'s current status.
///
/// Returns one of `"running"`, `"dead"`, `"suspended"`, or `"normal"`.
///
pub fn co_status(state: &mut LuaState) -> Result<usize, LuaError> {
    let co = get_co(state)?;
    let idx = aux_status(state, &co) as usize;
    let name: &[u8] = STAT_NAMES[idx];
    let interned = state.intern_str(name)?;
    state.push(LuaValue::Str(interned));
    Ok(1)
}

/// `coroutine.isyieldable([co])` — test whether a coroutine (default: current)
/// is in a yieldable state.
///
pub fn co_isyieldable(state: &mut LuaState) -> Result<usize, LuaError> {
    let is_yieldable = if matches!(state.type_at(1), LuaType::None) {
        state.is_yieldable()
    } else {
        let co = get_co(state)?;
        let co_id = co.id;
        let (is_main, is_current) = {
            let g = state.global();
            (co_id == g.main_thread_id, co_id == g.current_thread_id)
        };
        if is_main {
            false
        } else if is_current {
            state.is_yieldable()
        } else {
            let entry_rc = {
                let g = state.global();
                g.threads
                    .get(&co_id)
                    .expect("thread value carries an id that must resolve in GlobalState::threads")
                    .state
                    .clone()
            };
            let target_is_yieldable = match entry_rc.try_borrow() {
                Ok(b) => b.is_yieldable(),
                Err(_) => false,
            };
            target_is_yieldable
        }
    };
    state.push(LuaValue::Bool(is_yieldable));
    Ok(1)
}

/// `coroutine.running()` — return the current coroutine plus a boolean.
///
/// `push_thread` pushes the current `LuaState` as a thread value and returns
/// `true` iff it is the main thread.
///
/// The return arity is version-gated (pinned by `running_in_*_arity_by_version`
/// in `tests/coro_strengthen.rs`). From 5.2 the result is `(thread, ismain)`
/// where `ismain` is `true` for the main thread. Lua 5.1 has no `ismain`
/// boolean: it returns `nil` in the main coroutine and only the running thread
/// (one value) inside a coroutine (verified against lua5.1.5; see
/// `specs/followup/5.1-roster-syntax.md` §1).
pub fn co_running(state: &mut LuaState) -> Result<usize, LuaError> {
    let is_main = state.push_thread()?;
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        if is_main {
            state.pop_n(1);
            state.push(LuaValue::Nil);
        }
        return Ok(1);
    }
    state.push(LuaValue::Bool(is_main));
    Ok(2)
}

/// `coroutine.close(co)` — close a dead or suspended coroutine.
///
/// Closes a coroutine, running any pending to-be-closed variables via
/// `__close` and resetting its status. Valid only when the target is
/// suspended (`Yield`) or dead (`Ok` with no active frames).
/// Calling on a running or normal coroutine raises an error.
///
pub fn co_close(state: &mut LuaState) -> Result<usize, LuaError> {
    lua_vm::state::inc_c_stack(state)?;
    let result = (|| {
        let co = get_opt_co(state)?;
        let status = aux_status(state, &co);
        match status {
            COS_DEAD | COS_YIELD => close_suspended_or_dead(state, co),
            _ => {
                if matches!(state.global().lua_version, lua_types::LuaVersion::V55)
                    && status == COS_RUN
                    && state.global().closing_thread_id == Some(co.id)
                {
                    state.push(LuaValue::Bool(true));
                    return Ok(1);
                }
                if matches!(state.global().lua_version, lua_types::LuaVersion::V55)
                    && status == COS_RUN
                    && co.id == state.global().main_thread_id
                {
                    return Err(LuaError::runtime(format_args!("cannot close main thread")));
                }
                if matches!(state.global().lua_version, lua_types::LuaVersion::V55)
                    && status == COS_RUN
                    && co.id == state.global().current_thread_id
                {
                    state.global_mut().closing_thread_id = Some(co.id);
                    let in_status = state.status as i32;
                    let s = lua_vm::state::reset_thread(state, in_status);
                    state.global_mut().closing_thread_id = None;
                    state.n_ccalls = state.n_ccalls.saturating_sub(1);
                    std::panic::panic_any(LuaThreadClose(LuaStatus::from_raw(s)));
                }
                let name = if status == COS_RUN {
                    "running"
                } else {
                    "normal"
                };
                Err(LuaError::runtime(format_args!(
                    "cannot close a {} coroutine",
                    name
                )))
            }
        }
    })();
    state.n_ccalls -= 1;
    result
}

/// Performs the actual close for a suspended or dead coroutine.
fn close_suspended_or_dead(
    state: &mut LuaState,
    co: GcRef<lua_types::value::LuaThread>,
) -> Result<usize, LuaError> {
    let co_id = co.id;
    let entry_rc_opt = {
        let g = state.global();
        g.threads.get(&co_id).map(|e| e.state.clone())
    };
    let entry_rc = match entry_rc_opt {
        Some(rc) => rc,
        None => {
            state.push(LuaValue::Bool(true));
            return Ok(1);
        }
    };
    let parent_thread_id = state.global().current_thread_id;
    let caller_c_calls = state.c_calls();

    let mut parent_open_upval_slots = pop_resume_slot_buf(state);
    parent_open_upval_slots.extend(state.openupval.iter().filter_map(|uv| {
        uv.try_open_payload()
            .map(|(thread_id, idx)| (thread_id as u64, idx))
    }));
    {
        let mut g = state.global_mut();
        for (tid, idx) in &parent_open_upval_slots {
            let val = state.get_at(*idx);
            g.cross_thread_upvals.insert((*tid, *idx), val);
        }
    }

    push_parent_gc_snapshot(state);

    let (status, err_value): (i32, Option<LuaValue>) = {
        let mut co_state = entry_rc.borrow_mut();
        co_state.global_mut().current_thread_id = co_id;
        co_state.global_mut().closing_thread_id = Some(co_id);
        co_state.n_ccalls = caller_c_calls;
        let in_status = co_state.status as i32;
        let s = lua_vm::state::reset_thread(&mut *co_state, in_status);
        co_state.global_mut().closing_thread_id = None;
        co_state.global_mut().current_thread_id = parent_thread_id;
        if s == LuaStatus::Ok as i32 {
            (s, None)
        } else {
            let top = co_state.top_idx().0;
            if top > 0 {
                let err = co_state.get_at(lua_vm::state::StackIdx(top - 1));
                co_state.set_top(lua_vm::state::StackIdx(top - 1));
                (s, Some(err))
            } else {
                (s, Some(LuaValue::Nil))
            }
        }
    };

    pop_parent_gc_snapshot(state);

    {
        let mut flush = pop_resume_flush_buf(state);
        let mut g = state.global_mut();
        for (tid, idx) in &parent_open_upval_slots {
            if let Some(v) = g.cross_thread_upvals.remove(&(*tid, *idx)) {
                flush.push((*idx, v));
            }
        }
        drop(g);
        for (idx, v) in flush.drain(..) {
            state.set_at(idx, v);
        }
        return_resume_flush_buf(state, flush);
    }
    return_resume_slot_buf(state, parent_open_upval_slots);

    if status == LuaStatus::Ok as i32 {
        state.push(LuaValue::Bool(true));
        Ok(1)
    } else {
        state.push(LuaValue::Bool(false));
        if let Some(v) = err_value {
            state.push(v);
        } else {
            state.push(LuaValue::Nil);
        }
        Ok(2)
    }
}

// ── Module entry point ────────────────────────────────────────────────────────

/// Opens the `coroutine` standard library by pushing a new table containing
/// all `coroutine.*` functions.
///
pub fn open_coroutine(state: &mut LuaState) -> Result<usize, LuaError> {
    // `coroutine.close` is a Lua 5.4 addition tied to to-be-closed variables
    // (`specs/research/5.3-upstream-delta.md` delta #9). Under 5.3 it is absent
    // from the roster entirely.
    use lua_types::LuaVersion;
    let version = state.global().lua_version;
    let has_close = !matches!(version, LuaVersion::V51 | LuaVersion::V52 | LuaVersion::V53);
    // `coroutine.isyieldable` is a Lua 5.3 addition; it is absent in 5.1 and 5.2
    // (verified against lua5.1.5 and lua5.2.4: `type(coroutine.isyieldable)` ==
    // "nil"). See specs/followup/5.1-roster-syntax.md §1.
    let has_isyieldable = !matches!(version, LuaVersion::V51 | LuaVersion::V52);
    if has_close && has_isyieldable {
        state.new_lib(CO_FUNCS)?;
    } else {
        let filtered: Vec<(&[u8], lua_CFunction)> = CO_FUNCS
            .iter()
            .filter(|(name, _)| {
                (has_close || *name != b"close".as_slice())
                    && (has_isyieldable || *name != b"isyieldable".as_slice())
            })
            .copied()
            .collect();
        state.new_lib(&filtered)?;
    }
    Ok(1)
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   target_crate:  lua-stdlib
//   unsafe_blocks: 0
//   load-bearing:  this module is the cold shell — arg checking, the COS_*
//                  status mapping, the wrap closure, the cross-thread
//                  argument/result transfer scaffolding, and version-gated
//                  registration. The resume/yield CONTROL TRANSFER (stack save
//                  and restore) lives in lua-vm (lua_vm::do_::lua_resume /
//                  lua_yieldk) and is load-bearing; so are the cross-thread
//                  rooting machinery (RootedThreadBorrow, the resume-pool
//                  buffers, the GC stack snapshots), the LuaThreadClose
//                  panic-unwind that implements 5.5 self-close, and every
//                  version gate.
//   net:           behavior is pinned by tests/coro_strengthen.rs (the version
//                  seams), the official coroutine.lua suite, multiversion
//                  oracle, and check.sh 5.1-5.5. See GRADUATED.md "coroutine".
//   version-gated: get_co/thread_arg_error emit the calling function's name and
//                  the per-version "expected" body (coroutine vs thread, with vs
//                  without ", got <type>"). co_create rejects C-function bodies
//                  on 5.1 only ("Lua function expected").
//   known-gap:     the 5.1 yield-from-outside / yield-across-C-call wording is
//                  "attempt to yield across metamethod/C-call boundary" in the
//                  reference but "attempt to yield from outside a coroutine"
//                  here — the message originates in lua-vm's lua_yieldk (a
//                  cross-cutting yield guard, not this module). NOT fixed here:
//                  the single-source fix is a version gate in lua-vm/src/do_.rs.
//   known-gap:     a 5.1 arg error raised through pcall (no resolvable namewhat)
//                  names '?' in the reference but the qualified function here
//                  ('coroutine.resume', 'coroutine.create', ...). Single-source
//                  fix is to gate lua-vm arg_error_impl's find_func_name_in_loaded
//                  fallback off for V51 (same gap hits base/math arg errors).
// ──────────────────────────────────────────────────────────────────────────────
