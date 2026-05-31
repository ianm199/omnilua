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
fn v55_global_already_defined_guard() {
    // Div.1: `global name = expr` raises `global '<name>' already defined` at
    // runtime when the global currently holds a non-nil value. Pinned against
    // lua5.5.0 (specs/followup/5.5-lang.md).

    // Re-declare-with-initializer when already non-nil → error.
    err_contains(
        LuaVersion::V55,
        "global x = 1; global x = 2",
        "global 'x' already defined",
    );
    // The non-nil value can arrive via a plain assignment, not just an init.
    err_contains(
        LuaVersion::V55,
        "global x; x = 5; global x = 9",
        "global 'x' already defined",
    );
    // `false` is non-nil, so it also triggers the guard (strict nil check).
    err_contains(
        LuaVersion::V55,
        "global x = false; global x = 1",
        "global 'x' already defined",
    );
    // A nested-block re-init still checks the live value.
    err_contains(
        LuaVersion::V55,
        "global x = 1; do global x = 2 end",
        "global 'x' already defined",
    );
    // In a multi-name decl, the guard fires for whichever name is already
    // defined (checked top-down, matching upstream `initglobal`).
    err_contains(
        LuaVersion::V55,
        "global a = 1; global b = 2; global a, b = 3, 4",
        "global 'b' already defined",
    );

    // Nil'd out first → the re-init is allowed (proves it is a live-value
    // check, not compile-time redeclaration tracking).
    eq(LuaVersion::V55, "global x = 1; x = nil; global x = 2; return x", "2");
    // A no-initializer re-declaration never checks.
    eq(LuaVersion::V55, "global x; global x; return x", "nil");
    // Plain assignments after the first init never check.
    eq(LuaVersion::V55, "global x = 1; x = 2; x = 3; return x", "3");
    // The RHS is evaluated before the guard fires (upstream order); the value
    // here keeps the global nil, so the second init is fine.
    eq(LuaVersion::V55, "global x = nil; global x = 2; return x", "2");
}

#[test]
fn v55_global_guard_inert_pre_55() {
    // `global` is a plain identifier on 5.4/5.3, so none of the guard paths
    // exist there — repeated assignment to a `global`-named variable is fine.
    eq(LuaVersion::V54, "global = 1; global = 2; return global", "2");
    eq(LuaVersion::V53, "global = 1; global = 2; return global", "2");
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

/// 5.5 named varargs `function f(...t)` bind the trailing varargs into a fresh
/// packed table (`table.pack` semantics: 1-based sequence plus an integer `.n`
/// counting all args incl. nil holes). `...` keeps working inside the body.
/// Every expected value captured from lua5.5.0 (`specs/followup/5.5-lang.md`,
/// Div.2a).
#[test]
fn v55_named_varargs() {
    eq(LuaVersion::V55, "local function f(...t) return #t end return f(1,2,3)", "3");
    eq(LuaVersion::V55, "local function f(...t) return t.n end return f(1,nil,3)", "3");
    eq(LuaVersion::V55, "local function f(...t) return t.n end return f()", "0");
    eq(LuaVersion::V55, "local function f(a,...t) return t[2] end return f(0,10,20)", "20");
    // `...` is still usable alongside the named form.
    eq(LuaVersion::V55, "local function f(...t) return ... end return select('#', f(1,2,3))", "3");
    // Fresh table per call.
    eq(LuaVersion::V55, "local function f(...t) return t end return f(1)==f(1)", "false");
    // The table is mutable.
    eq(LuaVersion::V55, "local function f(...t) t[1]=99; return t[1] end return f(1,2)", "99");
    // No attribute allowed on `...t`.
    err_contains(LuaVersion::V55, "local function f(...t <const>) return t end", "')' expected");
}

/// Named-vararg syntax is 5.5-only; on 5.4/5.3 a name after `...` stays a parse
/// error matching the reference (`')' expected near 't'`).
#[test]
fn named_varargs_rejected_pre_55() {
    for v in [LuaVersion::V53, LuaVersion::V54] {
        err_contains(v, "local function f(...t) return t end", "')' expected near 't'");
    }
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

/// 5.3 coerces numeric strings to integers in the *core* bitwise ops
/// (`& | ~ << >>` and unary `~`), where 5.4/5.5 require number operands and
/// raise. Boundary cases: a numeric-but-non-integral string yields "no integer
/// representation"; a non-numeric string yields "perform bitwise operation".
/// All values captured from lua5.3.6 (see `specs/followup/5.3-coerce-err.md`).
#[test]
fn v53_bitwise_string_coercion() {
    eq(LuaVersion::V53, r#"return "3" & 5"#, "1");
    eq(LuaVersion::V53, r#"return "0xff" | 0"#, "255");
    eq(LuaVersion::V53, r#"return ~"5""#, "-6");
    eq(LuaVersion::V53, r#"return "8" >> "1""#, "4");
    eq(LuaVersion::V53, r#"return "8" << 1"#, "16");
    eq(LuaVersion::V53, r#"return 5 & "3""#, "1");
    eq(LuaVersion::V53, r#"return " 0x10 " & 255"#, "16");
    eq(LuaVersion::V53, r#"return "3.0" & 1"#, "1");
    eq(LuaVersion::V53, r#"return "0xffffffffffffffff" | 0"#, "-1");
    eq(LuaVersion::V53, r#"return "0xfffffffffffffffe" & "-1""#, "-2");
    eq(LuaVersion::V53, r#"return "   \n  -45  \t " >> "  -2  ""#, "-180");
    // Boundaries that MUST still error on 5.3:
    err_contains(LuaVersion::V53, r#"return "3.5" & 1"#, "no integer representation");
    err_contains(LuaVersion::V53, r#"return "0xffffffffffffffff.0" | 0"#, "no integer representation");
    err_contains(LuaVersion::V53, r#"return "abc" & 1"#, "perform bitwise operation on a string value");
}

/// Cross-version non-regression: 5.4/5.5 do NOT coerce strings in bitwise ops
/// and keep raising "perform bitwise operation on a string value".
#[test]
fn v54_v55_bitwise_no_string_coercion() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, r#"return "3" & 5"#, "perform bitwise operation on a string value");
        err_contains(v, r#"return ~"5""#, "perform bitwise operation on a string value");
        err_contains(v, r#"return "8" >> "1""#, "perform bitwise operation on a string value");
    }
}

/// 5.3 arith-on-non-coercible-string error wording. In the shared (5.4) model
/// arithmetic metamethods live on the string metatable and the failure message
/// is `attempt to <op> a '<t>' with a '<t>'` (no operand varinfo). 5.3 owns the
/// coercion in the core and raises `attempt to perform arithmetic on a <type>
/// value (<varinfo>)`, blaming the operand that is not a number. All wordings
/// captured from lua5.3.6 (see `specs/followup/5.3-coerce-err.md`).
#[test]
fn v53_arith_string_error_wording() {
    err_contains(
        LuaVersion::V53,
        r#"return "abc" + 1"#,
        "attempt to perform arithmetic on a string value",
    );
    err_contains(
        LuaVersion::V53,
        r#"return "abc" * 2"#,
        "attempt to perform arithmetic on a string value",
    );
    err_contains(
        LuaVersion::V53,
        r#"return -"x""#,
        "attempt to perform arithmetic on a string value",
    );
    // Varinfo comes from the VM call site (local/global/constant).
    err_contains(LuaVersion::V53, r#"local x="a"; return x+1"#, "(local 'x')");
    err_contains(LuaVersion::V53, r#"aaa="z"; return aaa+1"#, "(global 'aaa')");
    // A coercible string paired with a genuine non-number blames the
    // non-number operand, matching `luaG_opinterror` (errors.lua:102).
    err_contains(
        LuaVersion::V53,
        r#"aaa="2"; b=nil; return aaa*b"#,
        "attempt to perform arithmetic on a nil value (global 'b')",
    );
    // 5.3 successful string→number arith coercion still works (guard against
    // the fix accidentally stealing the success path). 5.1–5.3 always promote a
    // string operand to float in arithmetic, so the result type is `float` even
    // for integer-looking strings (verified vs lua5.3.6).
    eq(LuaVersion::V53, r#"return math.type("1"+"2")"#, "float");
    eq(LuaVersion::V53, r#"return math.type("1.0"+"2")"#, "float");
    eq(LuaVersion::V53, r#"return "3" + 2"#, "5.0");
    // Regression: a non-coercible string paired with a value that carries a
    // genuine arith metamethod must dispatch to that metamethod, NOT raise the
    // 5.3 core error. The 5.3 intercept fires only when the other operand has
    // no real metamethod. Both operand orders. (events.lua:139 — `b+'5'` where
    // `b` has `__add` — regressed before this guard was made metamethod-aware.)
    eq(
        LuaVersion::V53,
        r#"local t=setmetatable({},{__add=function() return 42 end}); return t+"5""#,
        "42",
    );
    eq(
        LuaVersion::V53,
        r#"local t=setmetatable({},{__add=function() return 42 end}); return "5"+t"#,
        "42",
    );
    // The same dispatch holds on 5.4/5.5 (the string-metatable path is never
    // consulted when the other operand has its own metamethod).
    for v in [LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            r#"local t=setmetatable({},{__add=function() return 42 end}); return "5"+t"#,
            "42",
        );
    }
}

/// Cross-version non-regression: 5.4/5.5 keep the string-metamethod arith
/// wording (`attempt to add a 'string' with a 'number'`) and must NOT switch to
/// the 5.3 core wording.
#[test]
fn v54_v55_arith_string_wording_unchanged() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, r#"return "abc" + 1"#, "attempt to add a 'string' with a 'number'");
        err_contains(v, r#"aaa="2"; b=nil; return aaa*b"#, "attempt to mul a 'string' with a 'nil'");
    }
}

/// 5.3 `for`-loop non-number bound wording is the older `'for' <what> must be a
/// number`; 5.4/5.5 reworded it to `bad 'for' <what> (number expected, got
/// <type>)`. Captured from lua5.3.6 / lua5.4.7 / lua5.5.0.
#[test]
fn v53_for_loop_error_wording() {
    err_contains(LuaVersion::V53, "for i=1,'a' do end", "'for' limit must be a number");
    err_contains(LuaVersion::V53, "for i='a',10 do end", "'for' initial value must be a number");
    err_contains(LuaVersion::V53, "for i=1,10,'a' do end", "'for' step must be a number");
}

/// Cross-version non-regression: 5.4/5.5 keep the `bad 'for' <what> (number
/// expected, got <type>)` wording.
#[test]
fn v54_v55_for_loop_error_wording_unchanged() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "for i=1,'a' do end", "bad 'for' limit (number expected, got string)");
        err_contains(v, "for i='a',10 do end", "bad 'for' initial value (number expected, got string)");
        err_contains(v, "for i=1,10,'a' do end", "bad 'for' step (number expected, got string)");
    }
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

/// `LUA_COMPAT_MATHLIB` roster (issue #19; `specs/followup/5.3-math.md`).
///
/// Per-version presence verified directly against the reference binaries:
/// `atan2/cosh/sinh/tanh/pow/log10` are in the default lua5.3.6 AND lua5.4.7
/// builds (5.4's `LUA_COMPAT_5_3` umbrella enables `LUA_COMPAT_MATHLIB`) but
/// gone in lua5.5.0; `frexp`/`ldexp` survive into 5.5 (registered outside the
/// compat `#if` in lua5.5.0's `lmathlib.c`). Values are `%.14g` tostring,
/// captured from the oracle.
#[test]
fn v53_compat_math_present_and_correct() {
    for v in [LuaVersion::V53, LuaVersion::V54] {
        for name in ["atan2", "cosh", "sinh", "tanh", "pow", "log10", "frexp", "ldexp"] {
            eq(v, &format!("return type(math.{name})"), "function");
        }
    }
    // Exact values (5.3; identical on 5.4 — same C wrappers).
    for v in [LuaVersion::V53, LuaVersion::V54] {
        eq(v, "return math.cosh(1)", "1.5430806348152");
        eq(v, "return math.sinh(1)", "1.1752011936438");
        eq(v, "return math.tanh(1)", "0.76159415595576");
        eq(v, "return math.pow(2, 0.5)", "1.4142135623731");
        // pow always returns a float, even with integer-valued result.
        eq(v, "return math.pow(2, 3)", "8.0");
        eq(v, "return math.type(math.pow(2, 3))", "float");
        eq(v, "return math.log10(1000)", "3.0");
        eq(v, "return math.ldexp(0.5, 3)", "4.0");
        eq(v, "return math.ldexp(1.0, -1)", "0.5");
        // ldexp must reach subnormals (naive x*2^e underflows the factor).
        eq(v, "return math.ldexp(1.0, -1074)", "4.9406564584125e-324");
        // frexp returns (float mantissa, integer exponent).
        eq(v, "local m, e = math.frexp(8.0); return m", "0.5");
        eq(v, "local m, e = math.frexp(8.0); return e", "4");
        eq(v, "local m, e = math.frexp(8.0); return math.type(m)", "float");
        eq(v, "local m, e = math.frexp(8.0); return math.type(e)", "integer");
        eq(v, "local m, e = math.frexp(0.0); return tostring(m) .. ',' .. tostring(e)", "0.0,0");
        // atan2 is the math_atan alias (two-arg form).
        eq(v, "return math.atan2(1, 1) == math.atan(1, 1)", "true");
        eq(v, "return math.atan2(1, 0) == math.atan(1, 0)", "true");
        // Arg errors name the function.
        err_contains(v, "return math.cosh('x')", "bad argument #1 to 'cosh'");
        err_contains(v, "return math.pow(2)", "bad argument #2 to 'pow'");
    }
}

/// No-cross-version-regression guard for the compat-math roster.
///
/// In lua5.5.0 the six `LUA_COMPAT_MATHLIB` functions are gone, but `frexp`
/// and `ldexp` remain (moved outside the compat `#if`). All values verified
/// against lua5.5.0.
#[test]
fn v55_compat_math_partition() {
    for name in ["atan2", "cosh", "sinh", "tanh", "pow", "log10"] {
        eq(LuaVersion::V55, &format!("return type(math.{name})"), "nil");
    }
    for name in ["frexp", "ldexp"] {
        eq(LuaVersion::V55, &format!("return type(math.{name})"), "function");
    }
    eq(LuaVersion::V55, "return math.ldexp(0.5, 3)", "4.0");
    eq(LuaVersion::V55, "local m, e = math.frexp(8.0); return tostring(m) .. ',' .. tostring(e)", "0.5,4");
}

// ─────────────────────────────────────────────────────────────────────────
// 5.4 regression guard — these must NOT drift (the multiversion work is
// required to leave 5.4 byte-identical to lua5.4.7 on these).
// ─────────────────────────────────────────────────────────────────────────

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

/// #76: math.type / math.tointeger return `nil` (not `false`) on failure.
/// luaL_pushfail = lua_pushnil in the default 5.3/5.4/5.5 builds (oracle
/// contract pins LUA_FAILISFALSE off). Pre-existing 5.4 port bug.
#[test]
fn issue76_math_fail_returns_nil() {
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(v, "return math.type('x')", "nil");
        eq(v, "return math.type(true)", "nil");
        eq(v, "return math.tointeger(3.5)", "nil");
        eq(v, "return math.tointeger(2^63)", "nil");
        // guard the success paths still work (regression fence):
        eq(v, "return math.tointeger('7')", "7");
        eq(v, "return math.type(1)", "integer");
        eq(v, "return math.type(1.0)", "float");
        // truthiness fence — lock the semantic intent, not just tostring:
        eq(v, "return math.type('x') == nil", "true");
        eq(
            v,
            "if math.tointeger(3.5) then return 'truthy' else return 'falsey' end",
            "falsey",
        );
    }
}

/// #77 (R-A): `string.find` on the pattern-matching path with zero explicit
/// captures must return exactly `start, end` — no spurious trailing empty
/// string. Upstream's `push_captures` uses `nlevels = (ms->level==0 && s) ? 1
/// : ms->level`; the `&& s` guard means *find* (s == NULL) pushes nothing when
/// there are no captures, while *match*/*gmatch*/*gsub* (s != NULL) still push
/// the whole match. Pre-existing 5.4 port bug, cross-version.
#[test]
fn issue77_string_find_no_spurious_capture() {
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        // bug: find with magic-char pattern, no captures → arity 2 (was 3).
        eq(v, "return select('#', string.find('hello','l+'))", "2");
        eq(
            v,
            "local a,b,c = string.find('hello','l+'); \
             return tostring(a)..','..tostring(b)..','..tostring(c)",
            "3,4,nil",
        );
        // anchored magic pattern, no captures → still arity 2.
        eq(v, "return select('#', string.find('hello','^h+'))", "2");

        // regression fences — these were already correct, lock them in:
        // explicit capture → arity 3, capture present.
        eq(v, "return select('#', string.find('hello','(l+)'))", "3");
        eq(
            v,
            "local a,b,c = string.find('hello','(l+)'); \
             return tostring(a)..','..tostring(b)..','..tostring(c)",
            "3,4,ll",
        );
        // match still returns the whole match (s != NULL path).
        eq(v, "return string.match('hello','l+')", "ll");
        // plain/literal path is unaffected → arity 2.
        eq(v, "return select('#', string.find('hello','ll'))", "2");
        // gsub count unaffected.
        eq(v, "return ({string.gsub('hello','l+','L')})[2]", "1");
        // gsub function-replacement: whole match still passed (s != NULL path).
        eq(
            v,
            "return (string.gsub('hello','l+',function(w) return '['..w..']' end))",
            "he[ll]o",
        );
        // gmatch with no captures still yields the whole match each step.
        eq(
            v,
            "local t={}; for w in string.gmatch('a,b,c','%a+') do t[#t+1]=w end; \
             return table.concat(t,'|')",
            "a|b|c",
        );
    }
}

/// #78 (R-C): `a <= b` with only `__lt` defined derives `not (b < a)` in the
/// default 5.1–5.4 reference builds (LUA_COMPAT_LT_LE, on by default) and is
/// removed in 5.5 (raises). Version-gated to match each reference exactly.
#[test]
fn issue78_le_derived_from_lt() {
    // __lt returns false → a<=b == not(b<a) == not(false) == true (5.3/5.4).
    let only_lt =
        "local m = {__lt = function() return false end}; \
         local a = setmetatable({}, m); local b = setmetatable({}, m); return a <= b";
    eq(LuaVersion::V53, only_lt, "true");
    eq(LuaVersion::V54, only_lt, "true");
    // 5.5 removed the fallback: comparing with no __le raises.
    err_contains(LuaVersion::V55, only_lt, "attempt to compare two table values");
    // >= also routes through __le (with swap) and derives on 5.4.
    eq(
        LuaVersion::V54,
        "local m = {__lt = function() return false end}; \
         local a = setmetatable({}, m); local b = setmetatable({}, m); return a >= b",
        "true",
    );
    // explicit __le is unaffected by the fallback on every version.
    let with_le =
        "local m = {__le = function() return true end, __lt = function() return false end}; \
         local a = setmetatable({}, m); return a <= a";
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(v, with_le, "true");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// #79 error-message fidelity (R-D/E/F/G). Shared-core: must match every
// version reference (5.3/5.4/5.5). Sub-item (d) — the `[C]: in ?` traceback
// tail — is deferred (architectural) and not asserted here.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v_argerror_to_fnname() {
    // (a1) bad-argument carries the resolved function name `to '<fn>'`.
    // The harness invokes these as inline field-access calls
    // (`string.char(...)`), so the name resolves from the call instruction to
    // the bare field `'char'` (exactly like the C reference for the inline
    // form); the `pcall(string.char, ...)` global-lookup form resolves to the
    // dotted `'string.char'`. Either way the `to '<fn>'` qualifier — the #79
    // defect — must be present.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return string.char(256)", "to 'char'");
        err_contains(v, "return string.char(256)", "value out of range");
        err_contains(v, "return utf8.char(0x80000000)", "to 'char'");
        err_contains(v, "return utf8.char(0x80000000)", "value out of range");
    }
}

#[test]
fn v_argerror_no_value() {
    // (a2) absent argument => `got no value`, not `got nil`.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return string.sub()", "got no value");
        err_contains(v, "return string.rep('x')", "got no value");
    }
}

#[test]
fn v_length_concat_location_prefix() {
    // (b) `#` and `..` carry the chunk-location prefix and the message body.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return #nil", "attempt to get length of a nil value");
        err_contains(v, "return ({})..({})", "attempt to concatenate a table value");
        // a `:<line>:` prefix appears before the message.
        let e = run(v, "return #nil").unwrap_err();
        let at = e.find("attempt").expect("message body present");
        assert!(e[..at].contains(':'), "v{v:?} #nil missing location prefix: {e}");
        let e = run(v, "return ({})..({})").unwrap_err();
        let at = e.find("attempt").expect("message body present");
        assert!(e[..at].contains(':'), "v{v:?} concat missing location prefix: {e}");
    }
}

#[test]
fn v54_v55_string_arith_coercion_failure() {
    // (b)+(c) string-arith failure: prefix present, operands labeled correctly.
    // 5.4/5.5 share the string arithmetic metamethods and the
    // `<op> a 'X' with a 'Y'` wording. (5.3 has no string-arith metamethods and
    // uses the legacy `perform arithmetic on a <type> value` wording from a
    // different VM path — version-gating that registration is out of #79 scope.)
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return ({}) - 'y'", "attempt to sub a 'table' with a 'string'");
        err_contains(v, "return -'x'", "attempt to unm a 'string' with a 'string'");
        // location prefix present on the string-arith path.
        let e = run(v, "return ({}) - 'y'").unwrap_err();
        let at = e.find("attempt").expect("message body present");
        assert!(e[..at].contains(':'), "v{v:?} string-arith missing prefix: {e}");
    }
}

#[test]
fn v_table_concat_invalid_value_type_name() {
    // (e) plain type name, no internal byte-array leak.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return table.concat({ {} })",
            "invalid value (table) at index 1 in table for 'concat'");
        // negative guard: the internal byte-array repr (e.g. `[116, 97, ...]`)
        // must NOT appear. (The chunk-name prefix `[string "..."]` legitimately
        // contains brackets, so look specifically for the comma-separated digit
        // list that the old `{:?}` Debug-format on `&[u8]` produced.)
        let e = run(v, "return table.concat({ {} })").unwrap_err();
        assert!(!e.contains("116, 97"), "v{v:?} concat leaked byte-array: {e}");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5.5 stdlib roster deltas (utf8.offset arity, collectgarbage option set + param)
// specs/followup/5.5-stdlib-err.md items 1, 2, 3.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v55_utf8_offset_returns_end_position() {
    // 5.5 returns (start, end) byte positions; the end is inclusive.
    eq(LuaVersion::V55,
        "local a,b = utf8.offset('aébc', 2); return a .. ',' .. b", "2,3");
    eq(LuaVersion::V55,
        "local a,b = utf8.offset('héllo', 3); return a .. ',' .. b", "4,4");
    // arity is 2 on the success branch.
    eq(LuaVersion::V55,
        "return select('#', utf8.offset('héllo', 3))", "2");
    // one-byte char: end == start.
    eq(LuaVersion::V55,
        "local a,b = utf8.offset('abc', 2); return a .. ',' .. b", "2,2");
    // not-found / out-of-range: arity stays 1 (only nil).
    eq(LuaVersion::V55,
        "return select('#', utf8.offset('abc', 99))", "1");
}

#[test]
fn v54_utf8_offset_arity_unchanged() {
    // Regression guard: 5.4/5.3 return only the start position (arity 1).
    for v in [LuaVersion::V53, LuaVersion::V54] {
        eq(v, "return select('#', utf8.offset('aébc', 2))", "1");
        eq(v, "return utf8.offset('aébc', 2)", "2");
    }
}

#[test]
fn v55_collectgarbage_drops_setpause_setstepmul() {
    // 5.5 removed setpause/setstepmul; they are now invalid options.
    err_contains(LuaVersion::V55,
        "return collectgarbage('setpause', 100)", "invalid option");
    err_contains(LuaVersion::V55,
        "return collectgarbage('setstepmul', 100)", "invalid option");
}

#[test]
fn v54_collectgarbage_keeps_setpause_setstepmul() {
    // Regression guard: 5.4/5.3 still accept setpause/setstepmul.
    for v in [LuaVersion::V53, LuaVersion::V54] {
        eq(v, "local ok = pcall(collectgarbage, 'setpause', 100); return ok", "true");
        eq(v, "local ok = pcall(collectgarbage, 'setstepmul', 100); return ok", "true");
    }
}

#[test]
fn v55_collectgarbage_param_surface() {
    // param read returns an integer.
    eq(LuaVersion::V55,
        "return math.type(collectgarbage('param', 'pause'))", "integer");
    // invalid param name errors via luaL_checkoption.
    err_contains(LuaVersion::V55,
        "return collectgarbage('param', 'bogus')", "invalid option");
    // write returns the OLD value, then read returns the value just written
    // (round-trip on the faithful-shape backing store).
    eq(LuaVersion::V55,
        "collectgarbage('param', 'stepmul', 333); return collectgarbage('param', 'stepmul')",
        "333");
    // arity is 1.
    eq(LuaVersion::V55,
        "return select('#', collectgarbage('param', 'pause'))", "1");
}

#[test]
fn v54_collectgarbage_param_not_an_option() {
    // Regression guard: 'param' is NOT a valid collectgarbage option on 5.4/5.3.
    for v in [LuaVersion::V53, LuaVersion::V54] {
        err_contains(v,
            "return collectgarbage('param', 'pause')", "invalid option");
    }
}

#[test]
fn v55_version_string() {
    eq(LuaVersion::V55, "return _VERSION", "Lua 5.5");
    eq(LuaVersion::V54, "return _VERSION", "Lua 5.4");
    eq(LuaVersion::V53, "return _VERSION", "Lua 5.3");
}
