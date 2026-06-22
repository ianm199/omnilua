//! A drop-in subset of the `hlua` 0.4 embedding API, backed by the pure-Rust
//! lua-rs VM instead of the C library.
//!
//! The goal is source compatibility: a consumer that does
//! `use lua_rs_hlua_shim as hlua;` should compile and run against lua-rs without
//! touching its own code. The implemented surface is the part exercised by
//! `authoscope` — `Lua`, `functionN`, `AnyLuaValue` and friends, `LuaFunction`,
//! and `StringInLua`. It is intentionally not the whole hlua API.
//!
//! Sandboxing note: lua-rs's runtime opens the full standard library, whereas
//! hlua consumers typically open only `string`. This is acceptable for a
//! compatibility spike but widens the sandbox; restricting the opened libraries
//! is tracked as future work.

mod any;
mod func;

use std::cell::Cell;
use std::marker::PhantomData;

use lua_stdlib::auxlib::load_buffer;
use lua_stdlib::init::open_libs;
use lua_types::closure::LuaLClosure;
use lua_types::gc::GcRef;
use lua_types::upval::UpVal;
use lua_types::value::LuaValue;
use lua_vm::api;
use lua_vm::state::{new_state, LuaState};

pub use any::{AnyHashableLuaValue, AnyLuaString, AnyLuaValue};
pub use func::{
    function0, function1, function2, function3, function4, function5, function6, FnHandle,
    LuaFunction, StringInLua,
};

use func::{registry_remove, Adapter, FromLuaGlobal, SetValue};

/// Error returned by `Lua::execute` and `LuaFunction::call_with_args`.
///
/// Carries only an owned message so the type is `Send + Sync + 'static` and
/// flows through `anyhow`'s `?` the way `hlua::LuaError` does for consumers.
#[derive(Debug, Clone)]
pub struct LuaError {
    message: String,
}

impl LuaError {
    pub(crate) fn from_vm(err: lua_types::LuaError) -> Self {
        LuaError {
            message: format!("{err}"),
        }
    }
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LuaError {}

/// A Lua state with the standard libraries installed.
///
/// The `'lua` parameter mirrors hlua's signature so consumer code that writes
/// `Lua<'a>` keeps compiling; the shim itself owns its state outright.
pub struct Lua<'lua> {
    state: LuaState,
    owned_closures: Vec<usize>,
    _marker: PhantomData<&'lua ()>,
}

impl Default for Lua<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'lua> Lua<'lua> {
    /// Create a new Lua state with the standard libraries installed. Panics
    /// only on allocation failure, matching the effective behaviour of
    /// `hlua::Lua::new()`.
    pub fn new() -> Lua<'lua> {
        let mut state = new_state().expect("lua-rs state allocation failed");
        state.global_mut().parser_hook = Some(parser_hook);
        open_libs(&mut state).expect("opening lua-rs standard libraries failed");
        Lua {
            state,
            owned_closures: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Open the string library. lua-rs already opens the full standard library
    /// at construction, so this is a no-op kept for API compatibility.
    pub fn open_string(&mut self) {}

    /// Load and run a chunk of Lua source.
    pub fn execute<T: FromExec>(&mut self, code: &str) -> Result<T, LuaError> {
        let status =
            load_buffer(&mut self.state, code.as_bytes(), b"=(load)").map_err(LuaError::from_vm)?;
        if status != 0 {
            let err = self.state.pop();
            return Err(LuaError::from_vm(lua_types::LuaError::from_value(err)));
        }
        api::pcall_k(&mut self.state, 0, 0, 0, 0, None).map_err(LuaError::from_vm)?;
        T::from_exec(self)
    }

    /// Set a global. Accepts the `functionN` wrappers from this crate.
    pub fn set<V: SetValue>(&mut self, name: &str, value: V) {
        value.set_into(self, name);
    }

    /// Read a global by name. The value type is chosen by the caller, e.g.
    /// `lua.get::<StringInLua<_>, _>("descr")`.
    pub fn get<'l, V>(&'l mut self, name: &str) -> Option<V>
    where
        V: FromLuaGlobal<'l>,
    {
        V::from_lua_global(self, name)
    }

    pub(crate) fn state_mut(&mut self) -> &mut LuaState {
        &mut self.state
    }

    /// Register a hosted closure as a global named `name`. The adapter index is
    /// recorded so it is freed when this `Lua` is dropped.
    pub(crate) fn install_closure(&mut self, name: &str, adapter: Adapter) {
        let index = func::registry_insert(adapter);
        self.owned_closures.push(index);
        let state = &mut self.state;
        api::push_integer(state, index as i64);
        api::push_cclosure(state, func::trampoline, 1).expect("push_cclosure failed");
        api::set_global(state, name.as_bytes()).expect("set_global failed");
    }
}

impl Drop for Lua<'_> {
    fn drop(&mut self) {
        for &index in &self.owned_closures {
            registry_remove(index);
        }
    }
}

/// The result type produced by [`Lua::execute`]. Only `()` is supported, which
/// is all `authoscope` requires.
pub trait FromExec: Sized {
    fn from_exec(lua: &mut Lua<'_>) -> Result<Self, LuaError>;
}

impl FromExec for () {
    fn from_exec(_lua: &mut Lua<'_>) -> Result<Self, LuaError> {
        Ok(())
    }
}

/// Compile a chunk to a Lua closure. Installed on the state so `load_buffer`
/// has a front end; this is the same bootstrap `lua-rs-runtime` performs,
/// inlined here so the shim depends only on published crates.
fn parser_hook(
    state: &mut LuaState,
    z: &mut lua_vm::zio::ZIO,
    name: &[u8],
    firstchar: i32,
) -> Result<GcRef<LuaLClosure>, lua_types::LuaError> {
    let proto = lua_parse::parse(
        state,
        lua_parse::DynData::default(),
        z,
        name,
        firstchar,
    )?;
    let nupvals = proto.upvalues.len();
    let mut upvals = Vec::with_capacity(nupvals);
    for _ in 0..nupvals {
        upvals.push(Cell::new(GcRef::new(UpVal::closed(LuaValue::Nil))));
    }
    Ok(GcRef::new(LuaLClosure {
        proto: GcRef::new(*proto),
        upvals: upvals.into_boxed_slice(),
    }))
}
