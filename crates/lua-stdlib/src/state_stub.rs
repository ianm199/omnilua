//! Phase-A `LuaState` stub shared across all stdlib modules.
//!
//! The real `LuaState` lives in `lua-vm` (not yet compiled). For Phase A/B we
//! provide a single struct here so every module's call sites type-check.
//! Every method body is `todo!("phase-b: <name>")`.
//!
//! TODO(phase-b): replace with `pub use lua_vm::state::LuaState` once lua-vm
//! is compiling.

#![allow(dead_code, unused_variables, clippy::too_many_arguments)]

use lua_types::{
    arith::ArithOp,
    error::LuaError,
    gc::GcRef,
    string::LuaString,
    userdata::LuaUserData,
    value::LuaValue,
    LuaType,
    LuaStatus,
};

/// Per-thread Lua interpreter state. Phase-A shared stub.
pub struct LuaState;

/// Bare function callable from Lua. C: `lua_CFunction`.
#[allow(non_camel_case_types)]
pub type lua_CFunction = fn(&mut LuaState) -> Result<usize, LuaError>;

/// Pseudo-index for the `i`-th upvalue of a C function.
/// C: `#define lua_upvalueindex(i)`
pub fn upvalue_index(i: i32) -> i32 {
    -1_001_000 - i
}

/// Comparison operations (eq, lt, le). C: `LUA_OP*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Lt,
    Le,
}

/// Reader-callback type for `lua_load`. C: `lua_Reader`.
pub type LuaReader<'a> = dyn FnMut() -> Option<Vec<u8>> + 'a;

/// Writer-callback type for `lua_dump`. C: `lua_Writer`.
pub type LuaWriter<'a> = dyn FnMut(&[u8]) -> Result<(), LuaError> + 'a;

/// Debug introspection record. C: `lua_Debug`.
#[derive(Debug, Default, Clone)]
pub struct LuaDebug {
    pub name: Option<Vec<u8>>,
    pub namewhat: Vec<u8>,
    pub what: u8,
    pub source: Vec<u8>,
    pub short_src: Vec<u8>,
    pub linedefined: i32,
    pub lastlinedefined: i32,
    pub currentline: i32,
    pub nups: u8,
    pub nparams: u8,
    pub isvararg: bool,
    pub istailcall: bool,
    pub ftransfer: u16,
    pub ntransfer: u16,
}

impl LuaState {
    // ── Push helpers ────────────────────────────────────────────────────
    pub fn push(&mut self, v: LuaValue) -> Result<(), LuaError> { todo!("phase-b: push") }
    pub fn push_value(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: push_value") }
    pub fn push_copy(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: push_copy") }
    pub fn push_string(&mut self, s: &[u8]) -> Result<(), LuaError> { todo!("phase-b: push_string") }
    pub fn push_bytes(&mut self, s: &[u8]) -> Result<(), LuaError> { todo!("phase-b: push_bytes") }
    pub fn push_fstring(&mut self, args: std::fmt::Arguments<'_>) -> Result<(), LuaError> { todo!("phase-b: push_fstring") }
    pub fn push_c_function(&mut self, f: lua_CFunction) -> Result<(), LuaError> { todo!("phase-b: push_c_function") }
    pub fn push_c_closure(&mut self, f: lua_CFunction, n: i32) -> Result<(), LuaError> { todo!("phase-b: push_c_closure") }
    pub fn push_where(&mut self, level: i32) -> Result<(), LuaError> { todo!("phase-b: push_where") }
    pub fn push_globals(&mut self) -> Result<(), LuaError> { todo!("phase-b: push_globals") }

    // ── Pop helpers ─────────────────────────────────────────────────────
    pub fn pop(&mut self) -> LuaValue { todo!("phase-b: pop") }
    pub fn pop_n(&mut self, n: i32) { todo!("phase-b: pop_n") }
    pub fn pop_bytes(&mut self) -> Vec<u8> { todo!("phase-b: pop_bytes") }

    // ── Top / set_top ───────────────────────────────────────────────────
    pub fn top(&mut self) -> i32 { todo!("phase-b: top") }
    pub fn top_count(&mut self) -> i32 { todo!("phase-b: top_count") }
    pub fn top_idx(&mut self) -> i32 { todo!("phase-b: top_idx") }
    pub fn set_top(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: set_top") }

    // ── Stack manipulation ──────────────────────────────────────────────
    pub fn insert(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: insert") }
    pub fn remove(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: remove") }
    pub fn replace(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: replace") }
    pub fn rotate(&mut self, idx: i32, n: i32) -> Result<(), LuaError> { todo!("phase-b: rotate") }
    pub fn copy_value(&mut self, from: i32, to: i32) -> Result<(), LuaError> { todo!("phase-b: copy_value") }
    pub fn abs_index(&mut self, idx: i32) -> i32 { todo!("phase-b: abs_index") }
    pub fn ensure_stack<S: AsRef<[u8]> + ?Sized>(&mut self, n: i32, msg: &S) -> Result<(), LuaError> { let _ = msg.as_ref(); todo!("phase-b: ensure_stack") }
    pub fn check_stack_space(&mut self, n: i32) -> bool { todo!("phase-b: check_stack_space") }

    // ── Type queries ────────────────────────────────────────────────────
    pub fn type_at(&mut self, idx: i32) -> LuaType { todo!("phase-b: type_at") }
    pub fn type_name(&mut self, t: LuaType) -> &'static [u8] { todo!("phase-b: type_name") }
    pub fn type_name_at(&mut self, idx: i32) -> &'static [u8] { todo!("phase-b: type_name_at") }
    pub fn value_at(&mut self, idx: i32) -> LuaValue { todo!("phase-b: value_at") }
    pub fn get_at(&mut self, idx: i32) -> LuaValue { todo!("phase-b: get_at") }
    pub fn is_none_or_nil(&mut self, idx: i32) -> bool { todo!("phase-b: is_none_or_nil") }
    pub fn is_integer(&mut self, idx: i32) -> bool { todo!("phase-b: is_integer") }
    pub fn is_number(&mut self, idx: i32) -> bool { todo!("phase-b: is_number") }

    // ── Conversions ─────────────────────────────────────────────────────
    pub fn to_lua_string(&mut self, idx: i32) -> Option<GcRef<LuaString>> { todo!("phase-b: to_lua_string") }
    pub fn to_lua_string_bytes(&mut self, idx: i32) -> Option<Vec<u8>> { todo!("phase-b: to_lua_string_bytes") }
    pub fn to_lua_string_len(&mut self, idx: i32) -> Option<usize> { todo!("phase-b: to_lua_string_len") }
    pub fn to_integer_x(&mut self, idx: i32) -> Option<i64> { todo!("phase-b: to_integer_x") }
    pub fn to_number_x(&mut self, idx: i32) -> Option<f64> { todo!("phase-b: to_number_x") }
    pub fn to_boolean(&mut self, idx: i32) -> bool { todo!("phase-b: to_boolean") }
    pub fn to_userdata(&mut self, idx: i32) -> Option<GcRef<LuaUserData>> { todo!("phase-b: to_userdata") }
    pub fn to_display_string(&mut self, idx: i32) -> Result<Vec<u8>, LuaError> { todo!("phase-b: to_display_string") }

    // ── Argument checking ───────────────────────────────────────────────
    pub fn check_arg_any(&mut self, arg: i32) -> Result<(), LuaError> { todo!("phase-b: check_arg_any") }
    pub fn check_arg_integer(&mut self, arg: i32) -> Result<i64, LuaError> { todo!("phase-b: check_arg_integer") }
    pub fn check_arg_string(&mut self, arg: i32) -> Result<Vec<u8>, LuaError> { todo!("phase-b: check_arg_string") }
    pub fn check_arg_type(&mut self, arg: i32, t: LuaType) -> Result<(), LuaError> { todo!("phase-b: check_arg_type") }
    pub fn check_arg_option(&mut self, arg: i32, def: Option<&[u8]>, lst: &[&[u8]]) -> Result<usize, LuaError> { todo!("phase-b: check_arg_option") }

    // ── Optional argument ───────────────────────────────────────────────
    pub fn opt_arg_integer(&mut self, arg: i32, def: i64) -> Result<i64, LuaError> { todo!("phase-b: opt_arg_integer") }
    pub fn opt_arg_string_bytes(&mut self, arg: i32) -> Result<Vec<u8>, LuaError> { todo!("phase-b: opt_arg_string_bytes") }
    pub fn opt_arg_string(&mut self, arg: i32, def: &[u8]) -> Result<Vec<u8>, LuaError> { todo!("phase-b: opt_arg_string") }
    pub fn arg_to_bool(&mut self, arg: i32) -> bool { todo!("phase-b: arg_to_bool") }

    // ── Field / table access ────────────────────────────────────────────
    pub fn get_field(&mut self, idx: i32, k: &[u8]) -> Result<LuaType, LuaError> { todo!("phase-b: get_field") }
    pub fn set_field(&mut self, idx: i32, k: &[u8]) -> Result<(), LuaError> { todo!("phase-b: set_field") }
    pub fn raw_get(&mut self, idx: i32) -> Result<LuaType, LuaError> { todo!("phase-b: raw_get") }
    pub fn raw_set(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: raw_set") }
    pub fn raw_get_i(&mut self, idx: i32, n: i64) -> Result<LuaType, LuaError> { todo!("phase-b: raw_get_i") }
    pub fn raw_set_i(&mut self, idx: i32, n: i64) -> Result<(), LuaError> { todo!("phase-b: raw_set_i") }
    pub fn raw_equal(&mut self, idx1: i32, idx2: i32) -> bool { todo!("phase-b: raw_equal") }
    pub fn raw_len(&mut self, idx: i32) -> i64 { todo!("phase-b: raw_len") }
    pub fn get_i(&mut self, idx: i32, n: i64) -> Result<LuaType, LuaError> { todo!("phase-b: get_i") }
    pub fn get_metafield(&mut self, idx: i32, name: &[u8]) -> Result<LuaType, LuaError> { todo!("phase-b: get_metafield") }
    pub fn get_meta_field(&mut self, idx: i32, name: &[u8]) -> Result<bool, LuaError> { todo!("phase-b: get_meta_field") }
    pub fn get_metatable(&mut self, idx: i32) -> Result<bool, LuaError> { todo!("phase-b: get_metatable") }
    pub fn set_metatable(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: set_metatable") }
    pub fn table_next(&mut self, idx: i32) -> Result<bool, LuaError> { todo!("phase-b: table_next") }
    pub fn new_table(&mut self) -> Result<(), LuaError> { todo!("phase-b: new_table") }
    pub fn create_table(&mut self, narr: i32, nrec: i32) -> Result<(), LuaError> { todo!("phase-b: create_table") }

    // ── GC ──────────────────────────────────────────────────────────────
    pub fn gc_control_simple(&mut self, op: i32) -> Result<i32, LuaError> { todo!("phase-b: gc_control_simple") }
    pub fn gc_count(&mut self) -> Result<i32, LuaError> { todo!("phase-b: gc_count") }
    pub fn gc_count_b(&mut self) -> Result<i32, LuaError> { todo!("phase-b: gc_count_b") }
    pub fn gc_step(&mut self, data: i32) -> Result<i32, LuaError> { todo!("phase-b: gc_step") }
    pub fn gc_set_param(&mut self, op: i32, value: i32) -> Result<i32, LuaError> { todo!("phase-b: gc_set_param") }
    pub fn gc_is_running(&mut self) -> Result<bool, LuaError> { todo!("phase-b: gc_is_running") }
    pub fn gc_gen(&mut self, minor_mul: i32, major_mul: i32) -> Result<i32, LuaError> { todo!("phase-b: gc_gen") }
    pub fn gc_inc(&mut self, pause: i32, step_mul: i32, step_size: i32) -> Result<i32, LuaError> { todo!("phase-b: gc_inc") }

    // ── Calls ───────────────────────────────────────────────────────────
    pub fn call(&mut self, nargs: i32, nresults: i32) -> Result<(), LuaError> { todo!("phase-b: call") }
    pub fn protected_call(&mut self, nargs: i32, nresults: i32, msgh: i32) -> Result<(), LuaError> { todo!("phase-b: protected_call") }
    pub fn len_op(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: len_op") }
    pub fn arith(&mut self, op: ArithOp) -> Result<(), LuaError> { todo!("phase-b: arith") }
    pub fn concat(&mut self, n: i32) -> Result<(), LuaError> { todo!("phase-b: concat") }

    // ── Loading ─────────────────────────────────────────────────────────
    pub fn load(&mut self, chunk: &[u8], name: &[u8], mode: Option<&[u8]>) -> Result<bool, LuaError> { todo!("phase-b: load") }
    pub fn load_buffer_ex<M: ?Sized>(&mut self, buf: &[u8], name: &[u8], mode: &M) -> Result<bool, LuaError> { todo!("phase-b: load_buffer_ex") }
    pub fn load_file(&mut self, path: Option<&[u8]>) -> Result<bool, LuaError> { todo!("phase-b: load_file") }
    pub fn load_file_ex(&mut self, path: Option<&[u8]>, mode: Option<&[u8]>) -> Result<bool, LuaError> { todo!("phase-b: load_file_ex") }
    pub fn load_with_reader<F, M: ?Sized>(&mut self, reader: F, name: &[u8], mode: &M) -> Result<bool, LuaError> { todo!("phase-b: load_with_reader") }
    pub fn dump_function(&mut self, strip: bool) -> Result<Vec<u8>, LuaError> { todo!("phase-b: dump_function") }

    // ── Misc / debug ────────────────────────────────────────────────────
    pub fn warning(&mut self, msg: &[u8], to_cont: bool) -> Result<(), LuaError> { todo!("phase-b: warning") }
    pub fn write_output(&mut self, msg: &[u8]) -> Result<(), LuaError> { todo!("phase-b: write_output") }
    pub fn set_warn_fn(&mut self, f: Option<lua_CFunction>, ud: Option<LuaValue>) { todo!("phase-b: set_warn_fn") }
    pub fn set_funcs<F: Copy>(&mut self, funcs: &[(&[u8], F)], nup: i32) -> Result<(), LuaError> { todo!("phase-b: set_funcs") }
    pub fn set_global(&mut self, name: &[u8]) -> Result<(), LuaError> { todo!("phase-b: set_global") }
    pub fn set_upvalue(&mut self, fidx: i32, n: i32) -> Result<Option<Vec<u8>>, LuaError> { todo!("phase-b: set_upvalue") }
    pub fn get_info(&mut self, what: &[u8], ar: &mut LuaDebug) -> Result<(), LuaError> { todo!("phase-b: get_info") }
    pub fn get_stack(&mut self, level: i32, ar: &mut LuaDebug) -> bool { todo!("phase-b: get_stack") }
    pub fn lua_version(&mut self) -> f64 { todo!("phase-b: lua_version") }
    pub fn string_to_number(&mut self, idx: i32) -> Option<usize> { todo!("phase-b: string_to_number") }
    pub fn string_to_number_push<S: AsRef<[u8]> + ?Sized>(&mut self, s: &S) -> Result<usize, LuaError> { let _ = s.as_ref(); todo!("phase-b: string_to_number_push") }
    pub fn require_lib(&mut self, name: &[u8], openf: lua_CFunction, glb: bool) -> Result<(), LuaError> { todo!("phase-b: require_lib") }
    pub fn peek_bytes(&mut self, idx: i32) -> Option<Vec<u8>> { todo!("phase-b: peek_bytes") }

    // ── Additional methods (batch 2) ───────────────────────────────────
    pub fn intern_str(&mut self, bytes: &[u8]) -> GcRef<LuaString> { todo!("phase-b: intern_str") }
    pub fn check_number(&mut self, arg: i32) -> Result<f64, LuaError> { todo!("phase-b: check_number") }
    pub fn check_integer(&mut self, arg: i32) -> Result<i64, LuaError> { todo!("phase-b: check_integer") }
    pub fn check_any(&mut self, arg: i32) -> Result<(), LuaError> { todo!("phase-b: check_any") }
    pub fn check_arg_number(&mut self, arg: i32) -> Result<f64, LuaError> { todo!("phase-b: check_arg_number") }
    pub fn check_arg_userdata(&mut self, arg: i32, name: &[u8]) -> Result<GcRef<LuaUserData>, LuaError> { todo!("phase-b: check_arg_userdata") }
    pub fn check_stack_growth(&mut self, n: i32) -> bool { todo!("phase-b: check_stack_growth") }
    pub fn opt_integer(&mut self, arg: i32, def: i64) -> Result<i64, LuaError> { todo!("phase-b: opt_integer") }
    pub fn opt_number(&mut self, arg: i32, def: f64) -> Result<f64, LuaError> { todo!("phase-b: opt_number") }
    pub fn opt_arg_lstring(&mut self, arg: i32, def: Option<&[u8]>) -> Result<Option<Vec<u8>>, LuaError> { todo!("phase-b: opt_arg_lstring") }

    pub fn table_get_i(&mut self, idx: i32, n: i64) -> Result<LuaType, LuaError> { todo!("phase-b: table_get_i") }
    pub fn table_set_i(&mut self, idx: i32, n: i64) -> Result<(), LuaError> { todo!("phase-b: table_set_i") }
    pub fn get_table(&mut self, idx: i32) -> Result<LuaType, LuaError> { todo!("phase-b: get_table") }
    pub fn raw_geti(&mut self, idx: i32, n: i64) -> Result<LuaType, LuaError> { todo!("phase-b: raw_geti") }
    pub fn raw_seti(&mut self, idx: i32, n: i64) -> Result<(), LuaError> { todo!("phase-b: raw_seti") }
    pub fn len_at(&mut self, idx: i32) -> i64 { todo!("phase-b: len_at") }
    pub fn length_at(&mut self, idx: i32) -> Result<i64, LuaError> { todo!("phase-b: length_at") }
    pub fn stack_at(&mut self, idx: i32) -> LuaValue { todo!("phase-b: stack_at") }
    pub fn stack_top(&mut self) -> i32 { todo!("phase-b: stack_top") }
    pub fn get_top(&mut self) -> i32 { todo!("phase-b: get_top") }

    pub fn push_value_at(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: push_value_at") }
    pub fn push_fail(&mut self) -> Result<(), LuaError> { todo!("phase-b: push_fail") }
    pub fn push_lstring(&mut self, s: &[u8]) -> Result<(), LuaError> { todo!("phase-b: push_lstring") }
    pub fn push_thread(&mut self) -> Result<bool, LuaError> { todo!("phase-b: push_thread") }
    pub fn push_cclosure(&mut self, f: lua_CFunction, n: i32) -> Result<(), LuaError> { todo!("phase-b: push_cclosure") }
    pub fn push_upvalue(&mut self, idx: i32) -> Result<(), LuaError> { todo!("phase-b: push_upvalue") }
    pub fn push_registry(&mut self) -> Result<(), LuaError> { todo!("phase-b: push_registry") }

    pub fn to_integer(&mut self, idx: i32) -> Option<i64> { todo!("phase-b: to_integer") }
    pub fn to_integer_opt(&mut self, idx: i32) -> Option<i64> { todo!("phase-b: to_integer_opt") }
    pub fn to_number(&mut self, idx: i32) -> Option<f64> { todo!("phase-b: to_number") }
    pub fn to_bytes(&mut self, idx: i32) -> Option<Vec<u8>> { todo!("phase-b: to_bytes") }
    pub fn to_bytes_at(&mut self, idx: i32) -> Option<Vec<u8>> { todo!("phase-b: to_bytes_at") }
    pub fn to_string_coerced(&mut self, idx: i32) -> Option<Vec<u8>> { todo!("phase-b: to_string_coerced") }
    pub fn to_light_userdata(&mut self, idx: i32) -> Option<*mut std::ffi::c_void> { todo!("phase-b: to_light_userdata") }
    pub fn to_thread(&mut self, idx: i32) -> Option<GcRef<lua_types::value::LuaThread>> { todo!("phase-b: to_thread") }
    pub fn to_thread_at(&mut self, idx: i32) -> Option<GcRef<lua_types::value::LuaThread>> { todo!("phase-b: to_thread_at") }
    pub fn type_name_str_at(&mut self, idx: i32) -> &'static [u8] { todo!("phase-b: type_name_str_at") }
    pub fn is_c_function_at(&mut self, idx: i32) -> bool { todo!("phase-b: is_c_function_at") }

    pub fn compare(&mut self, idx1: i32, idx2: i32, op: CompareOp) -> Result<bool, LuaError> { todo!("phase-b: compare") }
    pub fn compare_lt(&mut self, idx1: i32, idx2: i32) -> Result<bool, LuaError> { todo!("phase-b: compare_lt") }

    pub fn get_field_registry(&mut self, name: &[u8]) -> Result<LuaType, LuaError> { todo!("phase-b: get_field_registry") }
    pub fn get_registry_field(&mut self, name: &[u8]) -> Result<LuaType, LuaError> { todo!("phase-b: get_registry_field") }
    pub fn get_subtable_registry(&mut self, name: &[u8]) -> Result<bool, LuaError> { todo!("phase-b: get_subtable_registry") }
    pub fn get_or_create_registry_subtable(&mut self, name: &[u8]) -> Result<bool, LuaError> { todo!("phase-b: get_or_create_registry_subtable") }
    pub fn registry_get(&mut self, key: &[u8]) -> Result<LuaType, LuaError> { todo!("phase-b: registry_get") }
    pub fn registry_set(&mut self, key: &[u8]) -> Result<(), LuaError> { todo!("phase-b: registry_set") }

    pub fn new_lib<F: Copy>(&mut self, funcs: &[(&[u8], F)]) -> Result<(), LuaError> { todo!("phase-b: new_lib") }
    pub fn new_lib_table<F: Copy>(&mut self, funcs: &[(&[u8], F)]) -> Result<(), LuaError> { todo!("phase-b: new_lib_table") }
    pub fn new_metatable(&mut self, name: &[u8]) -> Result<bool, LuaError> { todo!("phase-b: new_metatable") }
    pub fn set_metatable_by_name(&mut self, name: &[u8]) -> Result<(), LuaError> { todo!("phase-b: set_metatable_by_name") }
    pub fn register_funcs<F: Copy>(&mut self, funcs: &[(&[u8], F)]) -> Result<(), LuaError> { todo!("phase-b: register_funcs") }
    pub fn register_lib<F: Copy>(&mut self, name: &[u8], funcs: &[(&[u8], F)]) -> Result<(), LuaError> { todo!("phase-b: register_lib") }
    pub fn set_funcs_with_upvalues<F: Copy>(&mut self, funcs: &[(&[u8], F)], nup: i32) -> Result<(), LuaError> { todo!("phase-b: set_funcs_with_upvalues") }

    pub fn new_userdata_typed(&mut self, name: &[u8], size: usize, nuvalue: i32) -> Result<GcRef<LuaUserData>, LuaError> { todo!("phase-b: new_userdata_typed") }
    pub fn get_iuservalue(&mut self, idx: i32, n: i32) -> Result<LuaType, LuaError> { todo!("phase-b: get_iuservalue") }
    pub fn set_iuservalue(&mut self, idx: i32, n: i32) -> Result<bool, LuaError> { todo!("phase-b: set_iuservalue") }
    pub fn test_arg_userdata(&mut self, arg: i32, name: &[u8]) -> Option<GcRef<LuaUserData>> { todo!("phase-b: test_arg_userdata") }

    pub fn get_upvalue(&mut self, fidx: i32, n: i32) -> Result<Option<Vec<u8>>, LuaError> { todo!("phase-b: get_upvalue") }
    pub fn upvalue_id(&mut self, fidx: i32, n: i32) -> Result<*mut std::ffi::c_void, LuaError> { todo!("phase-b: upvalue_id") }
    pub fn join_upvalues(&mut self, fidx1: i32, n1: i32, fidx2: i32, n2: i32) -> Result<(), LuaError> { todo!("phase-b: join_upvalues") }
    pub fn upvalue_index(&mut self, i: i32) -> i32 { upvalue_index(i) }

    pub fn get_local_at(&mut self, ar: &LuaDebug, n: i32) -> Result<Option<Vec<u8>>, LuaError> { todo!("phase-b: get_local_at") }
    pub fn set_local_at(&mut self, ar: &LuaDebug, n: i32) -> Result<Option<Vec<u8>>, LuaError> { todo!("phase-b: set_local_at") }
    pub fn get_param_name(&mut self, fidx: i32, n: i32) -> Result<Option<Vec<u8>>, LuaError> { todo!("phase-b: get_param_name") }

    pub fn get_debug_info(&mut self, what: &[u8], ar: &mut LuaDebug) -> Result<(), LuaError> { todo!("phase-b: get_debug_info") }
    pub fn get_stack_level(&mut self, level: i32, ar: &mut LuaDebug) -> bool { todo!("phase-b: get_stack_level") }
    pub fn has_frames(&mut self) -> bool { todo!("phase-b: has_frames") }
    pub fn lua_traceback(&mut self, other: &mut LuaState, msg: Option<&[u8]>, level: i32) -> Result<(), LuaError> { todo!("phase-b: lua_traceback") }

    pub fn set_hook(&mut self, f: Option<lua_CFunction>, mask: u32, count: i32) -> Result<(), LuaError> { todo!("phase-b: set_hook") }
    pub fn get_hook_count(&mut self) -> i32 { todo!("phase-b: get_hook_count") }
    pub fn get_hook_mask(&mut self) -> u32 { todo!("phase-b: get_hook_mask") }
    pub fn hook_is_set(&mut self) -> bool { todo!("phase-b: hook_is_set") }
    pub fn hook_is_internal_lua_hook(&mut self) -> bool { todo!("phase-b: hook_is_internal_lua_hook") }
    pub fn set_c_stack_limit(&mut self, limit: i32) -> Result<i32, LuaError> { todo!("phase-b: set_c_stack_limit") }

    pub fn new_thread(&mut self) -> Result<GcRef<lua_types::value::LuaThread>, LuaError> { todo!("phase-b: new_thread") }
    pub fn close_thread(&mut self, from: Option<&mut LuaState>) -> Result<LuaStatus, LuaError> { todo!("phase-b: close_thread") }
    pub fn close(&mut self) { todo!("phase-b: close") }
    pub fn is_yieldable(&mut self) -> bool { todo!("phase-b: is_yieldable") }
    pub fn is_same_thread(&mut self, other: &LuaState) -> bool { todo!("phase-b: is_same_thread") }
    pub fn thread_status(&mut self) -> LuaStatus { todo!("phase-b: thread_status") }

    pub fn load_buffer(&mut self, buf: &[u8], name: &[u8], mode: Option<&[u8]>) -> Result<LuaStatus, LuaError> { todo!("phase-b: load_buffer") }
    pub fn where_error(&mut self, level: i32, msg: &[u8]) -> LuaError { todo!("phase-b: where_error") }
    pub fn arg(&mut self, n: i32) -> LuaValue { todo!("phase-b: arg") }
    pub fn as_bytes_or_coerce(&mut self, idx: i32) -> Option<Vec<u8>> { todo!("phase-b: as_bytes_or_coerce") }
    pub fn as_bytes(&mut self, idx: i32) -> Option<Vec<u8>> { todo!("phase-b: as_bytes") }
}

impl LuaDebug {
    pub fn name_bytes(&self) -> &[u8] { self.name.as_deref().unwrap_or(b"?") }
    pub fn namewhat_bytes(&self) -> &[u8] { &self.namewhat }
    pub fn what_bytes(&self) -> &[u8] { match self.what { b'L' => b"Lua", b'C' => b"C", b'm' => b"main", _ => b"?" } }
    pub fn short_src_bytes(&self) -> &[u8] { &self.short_src }
    pub fn source_bytes(&self) -> &[u8] { &self.source }
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        (Phase-A scaffold; no C source)
//   target_crate:  lua-stdlib
//   confidence:    high
//   todos:         0 (every method body is a deliberate todo!())
//   port_notes:    1
//   unsafe_blocks: 0
//   notes:         Provides a single shared LuaState type with all methods
//                  the stdlib modules call. Bodies all panic; replaced in
//                  Phase B with `pub use lua_vm::state::LuaState`.
// ──────────────────────────────────────────────────────────────────────────
