//! Multi-version behavior tests — the differential oracle, baked into CI.
//!
//! Every expected value here was captured from the unmodified upstream
//! reference binary for that version (`make macosx` build of lua-5.3.6 /
//! lua-5.4.7 / lua-5.5.0; see `specs/oracle/CONTRACT.md`) via
//! `specs/oracle/diff_one.sh`. These assertions let `cargo test` catch a
//! regression in any version's behavior without needing the C binaries present
//! — they encode "what the reference does" as constants. When a case here was
//! found by the adversarial sweep (`specs/MULTIVERSION_ADVERSARIAL_FINDINGS.md`)
//! it is noted.

use lua_rs_runtime::{Lua, LuaVersion};

/// Run `code` under `version` and return `Ok(tostring(result))` or
/// `Err(error message)`. The snippet is `load`+`pcall`ed *inside* Lua so the VM
/// renders values and error messages faithfully (a `LuaError`'s Rust `Display`
/// can't reach the heap to render an interned message string), and so the
/// snippet's own `global`-strict scope is contained to the inner chunk — the
/// outer wrapper runs in implicit-global mode and always has the builtins.
fn run(version: LuaVersion, code: &str) -> Result<String, String> {
    let lua = Lua::new_versioned(version);
    let wrapper = format!(
        "local f, e = load([==[\n{code}\n]==])\n\
         if not f then return 'E\\0' .. e end\n\
         local ok, r = pcall(f)\n\
         if not ok then return 'E\\0' .. tostring(r) end\n\
         return 'V\\0' .. tostring(r)"
    );
    let out: String = lua
        .load(&wrapper)
        .eval()
        .unwrap_or_else(|e| panic!("harness failure for `{code}`: {e:?}"));
    if let Some(v) = out.strip_prefix("V\0") {
        Ok(v.to_string())
    } else if let Some(e) = out.strip_prefix("E\0") {
        Err(e.to_string())
    } else {
        panic!("harness: unexpected output `{out}` for `{code}`")
    }
}

/// Assert `code` produces exactly `expected` under `version`.
fn eq(version: LuaVersion, code: &str, expected: &str) {
    match run(version, code) {
        Ok(got) => assert_eq!(got, expected, "code: {code}"),
        Err(e) => panic!("code `{code}` errored (`{e}`), expected `{expected}`"),
    }
}

/// Assert `code` fails to compile/run under `version` with a message containing
/// `needle`.
fn err_contains(version: LuaVersion, code: &str, needle: &str) {
    match run(version, code) {
        Ok(got) => panic!("code `{code}` returned `{got}`, expected error containing `{needle}`"),
        Err(e) => assert!(e.contains(needle), "code `{code}` error `{e}` lacked `{needle}`"),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5.5 global declarations (F1/F2/F8 + enforcement) and language changes (F3/F4)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v55_global_enforcement() {
    // Implicit `global *` until the first explicit decl.
    eq(LuaVersion::V55, "y = 3; return y", "3");
    // Declared globals read/write.
    eq(LuaVersion::V55, "global a; a = 5; return a", "5");
    // After an explicit decl, an undeclared free name is a compile error.
    err_contains(LuaVersion::V55, "global a; a = 1; zz = 2", "variable 'zz' not declared");
    err_contains(
        LuaVersion::V55,
        "global f; local function g() return nope end return g()",
        "variable 'nope' not declared",
    );
}

#[test]
fn v55_global_block_scoped() {
    // F1: a `global` decl is confined to its block; strict mode ends with it
    // (using builtins / free names after the block would error if it leaked).
    eq(LuaVersion::V55, "do global Y; Y = 1 end; return Y", "1");
    eq(LuaVersion::V55, "if true then global Z; Z = 1 end; w = 2; return w", "2");
}

#[test]
fn v55_global_initializer_stored() {
    // F2: `global x = expr` actually assigns (was previously dropped).
    eq(LuaVersion::V55, "do global x = 7 end; return x", "7");
    eq(LuaVersion::V55, "do global a, b = 10, 20 end; return a + b", "30");
}

#[test]
fn v55_const_global_rejects_assignment() {
    err_contains(
        LuaVersion::V55,
        "global x <const> = 1; x = 2",
        "attempt to assign to const variable 'x'",
    );
}

#[test]
fn v55_global_is_a_valid_identifier() {
    // F8: `global` is contextual, not reserved (LUA_COMPAT_GLOBAL). No panic.
    eq(LuaVersion::V55, "local global = 5; return global", "5");
    eq(LuaVersion::V55, "global = 7; return global", "7");
}

#[test]
fn v55_for_control_var_readonly() {
    // F3: numeric and first-generic for vars are read-only.
    err_contains(LuaVersion::V55, "for i = 1, 3 do i = 10 end", "attempt to assign to const variable 'i'");
    err_contains(
        LuaVersion::V55,
        "for k, v in pairs({1, 2}) do k = 10 end",
        "attempt to assign to const variable 'k'",
    );
    // The second generic var stays assignable; reads are fine.
    eq(LuaVersion::V55, "local s = 0; for i = 1, 3 do s = s + i end; return s", "6");
    eq(LuaVersion::V55, "for k, v in pairs({7}) do v = 9 end; return 'ok'", "ok");
}

#[test]
fn v55_float_tostring_round_trips() {
    // F4: %.15g-then-%.17g shortest round-trip form (wrapper's tostring runs
    // under V55).
    eq(LuaVersion::V55, "return 1/3", "0.33333333333333331");
    eq(LuaVersion::V55, "return 3.14", "3.14");
    eq(LuaVersion::V55, "return 0.1 + 0.2", "0.30000000000000004");
    eq(LuaVersion::V55, "return 2^53", "9007199254740992.0");
    eq(LuaVersion::V55, "return 1e16", "1e+16");
    eq(LuaVersion::V55, "return 1.0", "1.0");
}

#[test]
fn v55_table_create_present() {
    eq(LuaVersion::V55, "return type(table.create)", "function");
}

// ─────────────────────────────────────────────────────────────────────────
// 5.3 behavioral deltas
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_bit32_surface() {
    eq(LuaVersion::V53, "return bit32.band(6, 3)", "2");
    eq(LuaVersion::V53, "return bit32.btest(6, 3)", "true");
    eq(LuaVersion::V53, "return bit32.extract(0xF0, 4, 4)", "15");
    eq(LuaVersion::V53, "return bit32.replace(0, 5, 0, 4)", "5");
    eq(LuaVersion::V53, "return bit32.arshift(-8, 1)", "4294967292");
    eq(LuaVersion::V53, "return bit32.lrotate(1, 1)", "2");
    eq(LuaVersion::V53, "return bit32.rrotate(1, 1)", "2147483648");
}

#[test]
fn v53_string_coercion_is_float() {
    // 5.3: a string coerced in arithmetic yields a float (integer in 5.4).
    eq(LuaVersion::V53, "return math.type('0x10' + 0)", "float");
    eq(LuaVersion::V54, "return math.type('0x10' + 0)", "integer");
}

#[test]
fn v53_removed_builtins_absent() {
    eq(LuaVersion::V53, "return type(warn)", "nil");
    eq(LuaVersion::V53, "return type(coroutine.close)", "nil");
    eq(LuaVersion::V53, "return type(bit32)", "table");
    eq(LuaVersion::V53, "return type(table.create)", "nil");
    eq(LuaVersion::V53, "return type(math.type)", "function");
}

#[test]
fn v53_rejects_attribute_syntax() {
    err_contains(LuaVersion::V53, "local x <const> = 1; return x", "unexpected symbol");
}

// ─────────────────────────────────────────────────────────────────────────
// 5.4 regression guard — these must NOT drift (the multiversion work is
// required to leave 5.4 byte-identical to lua5.4.7 on these).
// ─────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────
// 5.1 — PARTIAL spike (lua-5.1-spike branch only). Covers the float-only
// observable number behavior and the math roster; the fenv globals subsystem
// and syntax gates are not done yet (specs/LUA_5_1_PLAN.md).
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v51_float_only_observable_behavior() {
    // Float-only: integer-valued floats print without ".0" (vs 5.3+).
    eq(LuaVersion::V51, "return 10/2", "5");
    eq(LuaVersion::V51, "return 2^2", "4");
    eq(LuaVersion::V51, "return 1.5", "1.5");
    eq(LuaVersion::V51, "return math.floor(3.7)", "3");
    // The 5.3+ integer-subtype math members are absent.
    eq(LuaVersion::V51, "return type(math.type)", "nil");
    eq(LuaVersion::V51, "return type(math.tointeger)", "nil");
    eq(LuaVersion::V51, "return type(math.maxinteger)", "nil");
    eq(LuaVersion::V51, "return _VERSION", "Lua 5.1");
}

#[test]
fn v54_unchanged() {
    eq(LuaVersion::V54, "return 1/3", "0.33333333333333"); // %.14g
    eq(LuaVersion::V54, "return 2^53", "9.007199254741e+15");
    eq(LuaVersion::V54, "return 3.14", "3.14");
    eq(LuaVersion::V54, "return type(warn)", "function");
    eq(LuaVersion::V54, "return type(coroutine.close)", "function");
    eq(LuaVersion::V54, "return type(bit32)", "nil");
    eq(LuaVersion::V54, "local x <const> = 42; return x", "42");
    err_contains(LuaVersion::V54, "local x <const> = 1; x = 2", "attempt to assign to const variable 'x'");
    // `global` is an ordinary identifier on 5.4.
    eq(LuaVersion::V54, "local global = 8; return global", "8");
    // for-loop var is assignable on 5.4.
    eq(LuaVersion::V54, "for i = 1, 1 do i = 10 end; return 'ok'", "ok");
}
