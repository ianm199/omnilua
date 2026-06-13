//! `LuaError` and its canonical constructors. PORT_STRATEGY §3.7, PORTING.md §6.

use crate::status::LuaStatus;
use crate::value::LuaValue;
use std::fmt;

/// Internal control-flow payload used by the standalone CLI to implement
/// `os.exit` without making Lua protected calls catch it as an ordinary error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuaExit(pub i32);

/// Internal control-flow payload for Lua 5.5 `coroutine.close()` self-close.
/// It is caught at the coroutine resume boundary; panic hooks should suppress it
/// like [`LuaExit`] because it is not a Rust runtime panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuaThreadClose(pub LuaStatus);

/// Unwind marker for the T5b table-set error prototype (branch
/// `proto/unwind-errors-tableset`, never merged).
///
/// The hot table set/get path raises errors by `raise_set_error(e)` — which
/// stashes the `LuaError` in [`take_pending_set_error`]'s thread-local and then
/// `panic_any(LuaSetError)` — instead of returning `Err(e)`, so the success
/// path threads no `Result`. It is caught and converted back to
/// `Err(LuaError)` at the single protected-call boundary
/// (`lua_vm::do_::raw_run_protected`), making the conversion invisible to every
/// pcall / coroutine-resume / close caller.
///
/// **The payload is a zero-size marker, not the `LuaError` itself.**
/// `std::panic::panic_any` requires `Any + Send`, but a `LuaError::Runtime`
/// carries a `LuaValue` of GC pointers (`GcRef` / `Rc<[u8]>`) that are
/// deliberately **not `Send`** in this single-threaded VM. So the error value
/// rides a thread-local (the Rust analog of C's `longjmp` leaving the error
/// object on `L`'s stack) and only control transfer rides the panic. This is a
/// primary T5b finding: a full conversion cannot put the error value in the
/// panic payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuaSetError;

thread_local! {
    static PENDING_SET_ERROR: std::cell::RefCell<Option<LuaError>> =
        const { std::cell::RefCell::new(None) };
}

/// Stash `err` in the thread-local and unwind with the [`LuaSetError`] marker.
/// The matching `take_pending_set_error` at the catch boundary recovers it.
/// T5b prototype, never merged.
pub fn raise_set_error(err: LuaError) -> ! {
    PENDING_SET_ERROR.with(|slot| *slot.borrow_mut() = Some(err));
    std::panic::panic_any(LuaSetError)
}

/// Recover the `LuaError` stashed by the most recent [`raise_set_error`] on
/// this thread. Returns [`LuaError::Error`] if somehow absent (a `LuaSetError`
/// unwind with no stash is a prototype bug, surfaced as a generic error rather
/// than a second panic). T5b prototype, never merged.
pub fn take_pending_set_error() -> LuaError {
    PENDING_SET_ERROR.with(|slot| slot.borrow_mut().take().unwrap_or(LuaError::Error))
}

/// The Lua error type. Carries a `LuaValue` payload because Lua errors can
/// be any value (typically a string).
#[derive(Debug, Clone)]
pub enum LuaError {
    Runtime(LuaValue),
    Syntax(LuaValue),
    Memory,
    Error,
    Yield,
    File,
    Gc,
}

impl LuaError {
    // ── Generic message constructors ─────────────────────────────────────
    pub fn runtime(args: fmt::Arguments<'_>) -> Self {
        LuaError::Runtime(LuaValue::Str(crate::gc::GcRef::new(
            crate::string::LuaString::from_bytes(format!("{}", args).into_bytes()),
        )))
    }
    pub fn syntax(args: fmt::Arguments<'_>) -> Self {
        LuaError::Syntax(LuaValue::Str(crate::gc::GcRef::new(
            crate::string::LuaString::from_bytes(format!("{}", args).into_bytes()),
        )))
    }
    pub fn syntax_at(args: fmt::Arguments<'_>, source: &[u8], line: i32) -> Self {
        LuaError::Syntax(LuaValue::Str(crate::gc::GcRef::new(
            crate::string::LuaString::from_bytes(
                format!("{}:{}: {}", String::from_utf8_lossy(source), line, args).into_bytes(),
            ),
        )))
    }
    pub fn syntax_raw(msg: &[u8]) -> Self {
        LuaError::Syntax(LuaValue::Str(crate::gc::GcRef::new(
            crate::string::LuaString::from_bytes(msg.to_vec()),
        )))
    }

    // ── Standard-shape constructors ──────────────────────────────────────
    pub fn type_error(v: &LuaValue, op: &str) -> Self {
        LuaError::runtime(format_args!("attempt to {} a {} value", op, v.type_name()))
    }
    pub fn call_error(v: &LuaValue) -> Self {
        Self::type_error(v, "call")
    }
    pub fn concat_error(p1: &LuaValue, p2: &LuaValue) -> Self {
        let bad = if matches!(p1, LuaValue::Str(_) | LuaValue::Int(_) | LuaValue::Float(_)) {
            p2
        } else {
            p1
        };
        LuaError::runtime(format_args!(
            "attempt to concatenate a {} value",
            bad.type_name()
        ))
    }
    pub fn arith_error(p1: &LuaValue, p2: &LuaValue, _msg: &str) -> Self {
        let bad = if matches!(p1, LuaValue::Int(_) | LuaValue::Float(_)) {
            p2
        } else {
            p1
        };
        LuaError::runtime(format_args!(
            "attempt to perform arithmetic on a {} value",
            bad.type_name()
        ))
    }
    pub fn int_overflow(_p1: &LuaValue, _p2: &LuaValue) -> Self {
        LuaError::runtime(format_args!("number has no integer representation"))
    }
    pub fn order_error(p1: &LuaValue, p2: &LuaValue) -> Self {
        LuaError::runtime(format_args!(
            "attempt to compare {} with {}",
            p1.type_name(),
            p2.type_name()
        ))
    }
    pub fn for_error(v: &LuaValue, what: &str) -> Self {
        LuaError::runtime(format_args!(
            "bad 'for' {} (number expected, got {})",
            what,
            v.type_name()
        ))
    }
    pub fn arg_error(narg: i32, msg: &str) -> Self {
        LuaError::runtime(format_args!("bad argument #{} ({})", narg, msg))
    }
    pub fn type_arg_error(narg: i32, expected: &str, got: &LuaValue) -> Self {
        LuaError::runtime(format_args!(
            "bad argument #{} ({} expected, got {})",
            narg,
            expected,
            got.type_name()
        ))
    }

    // ── Pass-through constructors ────────────────────────────────────────
    pub fn from_value(v: LuaValue) -> Self {
        // Special-case: the global "not enough memory" string becomes Memory.
        // The real impl checks ptr equality against G(L)->memerrmsg.
        LuaError::Runtime(v)
    }
    pub fn with_status(status: LuaStatus) -> Self {
        match status {
            LuaStatus::Ok => LuaError::Error,
            LuaStatus::Yield => LuaError::Yield,
            LuaStatus::ErrRun => LuaError::Runtime(LuaValue::Nil),
            LuaStatus::ErrSyntax => LuaError::Syntax(LuaValue::Nil),
            LuaStatus::ErrMem => LuaError::Memory,
            LuaStatus::ErrErr => LuaError::Error,
            LuaStatus::ErrFile => LuaError::File,
            LuaStatus::ErrGc => LuaError::Gc,
        }
    }

    pub fn to_status(&self) -> LuaStatus {
        match self {
            LuaError::Runtime(_) => LuaStatus::ErrRun,
            LuaError::Syntax(_) => LuaStatus::ErrSyntax,
            LuaError::Memory => LuaStatus::ErrMem,
            LuaError::Error => LuaStatus::ErrErr,
            LuaError::Yield => LuaStatus::Yield,
            LuaError::File => LuaStatus::ErrFile,
            LuaError::Gc => LuaStatus::ErrGc,
        }
    }

    pub fn into_value(self) -> LuaValue {
        match self {
            LuaError::Runtime(v) | LuaError::Syntax(v) => v,
            _ => LuaValue::Nil,
        }
    }

    /// Human-readable error payload for embedders.
    ///
    /// Lua errors can carry any Lua value. When the payload is a byte string,
    /// this returns it using lossy UTF-8 conversion; other payloads fall back to
    /// the Lua type name so host integrations do not have to parse `Debug`.
    pub fn message_lossy(&self) -> String {
        match self {
            LuaError::Runtime(v) | LuaError::Syntax(v) => lua_value_message_lossy(v),
            LuaError::Memory => "not enough memory".to_string(),
            LuaError::Error => "error in error handling".to_string(),
            LuaError::Yield => "attempt to yield across a C-call boundary".to_string(),
            LuaError::File => "file error".to_string(),
            LuaError::Gc => "garbage collector error".to_string(),
        }
    }
}

fn lua_value_message_lossy(value: &LuaValue) -> String {
    match value {
        LuaValue::Str(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
        LuaValue::Nil => "nil".to_string(),
        LuaValue::Bool(v) => v.to_string(),
        LuaValue::Int(v) => v.to_string(),
        LuaValue::Float(v) => v.to_string(),
        other => format!("{} error", other.type_name()),
    }
}

impl fmt::Display for LuaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl std::error::Error for LuaError {}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        n/a (Rust-native error enum; no C analogue)
//   target_crate:  lua-types
//   confidence:    high
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 0
//   notes:         LuaError + supporting variants. Pure Rust idiom (Result<T, LuaError>) replacing
//                  C's setjmp/longjmp error propagation. No direct C-source mapping.
// ──────────────────────────────────────────────────────────────────────────────
