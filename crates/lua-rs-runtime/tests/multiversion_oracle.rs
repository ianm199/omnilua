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
// Shared-core item A: an upvalue (e.g. `_ENV`) indexed by a relational/jump
// key. `luaK_exp2val` must force the jump result into a register *before*
// `luaK_indexed` discharges the table upvalue, or the boolean materialization
// and the GETUPVAL collide and the table operand ends up holding a number.
//
// Version split confirmed against the reference binaries: 5.3 and 5.5 return
// `nil` (our shared register-based GETTABUP needs the `VJMP` clause to match),
// while 5.4's reference *genuinely* raises "attempt to index a number value"
// (an upstream 5.4 bug 5.5 later fixed by adding `e->k == VJMP` to
// `luaK_exp2val`). The fix reproduces all three faithfully.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_v55_env_relational_index_returns_nil() {
    for v in [LuaVersion::V53, LuaVersion::V55] {
        // _ENV (upvalue 0) indexed by a folded relational constant.
        eq(v, "return _ENV[1 < 2]", "nil");
        // A captured local upvalue indexed by the same.
        eq(v, "local up = {}; return (function() return up[1 < 2] end)()", "nil");
        // Non-folded comparison (two locals) through an upvalue.
        eq(v,
            "local x, y = 1, 1; local up = {}; \
             return (function() return up[x == y] end)()",
            "nil");
        // Store side: `_ENV[1<2] = v` must index correctly too.
        eq(v, "_ENV[1 < 2] = 7; return _ENV[true]", "7");
    }
}

#[test]
fn v54_env_relational_index_errors_like_reference() {
    // Guard the deliberate 5.4-only divergence: the reference 5.4 binary raises
    // on this exact construct; our port must not "improve" on it.
    err_contains(LuaVersion::V54, "return _ENV[1 < 2]", "index a number value");
    err_contains(
        LuaVersion::V54,
        "local up = {}; return (function() return up[1 < 2] end)()",
        "index a number value",
    );
}

#[test]
fn all_versions_register_table_relational_index_unaffected() {
    // Regression guard: a *register* table (GETTABLE, not GETTABUP) was always
    // correct on every version. The fix must leave it untouched.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(v, "local t = { [true] = 99 }; return t[1 < 2]", "99");
        // Literal boolean key through an upvalue keeps working (no jump list).
        eq(v, "_ENV[true] = 42; return _ENV[true]", "42");
        // String-key upvalue index (the VKStr fast path) is a no-op for the fix.
        eq(v, "xyz = 5; return _ENV.xyz", "5");
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
fn v55_global_prefixed_const_namelist() {
    // 5.5 `global <const> a, b` — a leading attribute applies to the whole name
    // list (it is NOT tied to `*`). Each name may still carry its own attribute.
    // Captured from lua5.5.0.
    eq(LuaVersion::V55, "global<const> a, b = 1, 2; return a + b", "3");
    eq(LuaVersion::V55, "global <const> a = 5; return a", "5");
    err_contains(
        LuaVersion::V55,
        "global <const> a = 1; a = 2",
        "attempt to assign to const variable 'a'",
    );
}

#[test]
fn v55_global_function_form() {
    // 5.5 `global function NAME body` (lparser.c globalfunc). Captured from
    // lua5.5.0.
    eq(
        LuaVersion::V55,
        "global function f() return 7 end; return f()",
        "7",
    );
    eq(
        LuaVersion::V55,
        "global function fact(x) if x==0 then return 1 else return x*fact(x-1) end end; return fact(5)",
        "120",
    );
    err_contains(
        LuaVersion::V55,
        "global function f() end; global function f() end",
        "global 'f' already defined",
    );
}

#[test]
fn v55_global_wildcard_coexists_with_named_decl() {
    // 5.5 `global *` enables global-by-default for the scope; a later
    // `global name` does NOT void it (the `*` declaration coexists). Without
    // this, `assert` below would be "not declared". Captured from lua5.5.0.
    eq(
        LuaVersion::V55,
        "global *\nglobal fact = false\nfact = 3\nreturn assert(fact)",
        "3",
    );
}

#[test]
fn v55_local_prefixed_attribute() {
    // 5.5 allows a PREFIXED attribute on a local: `local <const> a, b`.
    // 5.4 rejects the prefix form (attribute only postfix). Captured from
    // lua5.5.0 / lua5.4.7.
    eq(LuaVersion::V55, "local <const> a, b = 1, 2; return a + b", "3");
    eq(LuaVersion::V55, "local<const> x = 5; return x", "5");
    err_contains(
        LuaVersion::V55,
        "local <const> x = 5; x = 6",
        "attempt to assign to const variable 'x'",
    );
    // 5.4: prefixed attribute is a syntax error (postfix only).
    err_contains(LuaVersion::V54, "local <const> x = 5", "<name> expected");
}

#[test]
fn v55_attribute_message_text() {
    // Div.3 / Div.4 message text, captured from lua5.5.0 (and the local form is
    // shared with 5.4). Location prefix present, no spurious `near`.
    err_contains(LuaVersion::V55, "local x <foo> = 1", "unknown attribute 'foo'");
    err_contains(LuaVersion::V54, "local x <foo> = 1", "unknown attribute 'foo'");
    err_contains(
        LuaVersion::V55,
        "global x <foo> = 1",
        "unknown attribute 'foo'",
    );
    err_contains(
        LuaVersion::V55,
        "global x <close> = setmetatable({},{})",
        "global variables cannot be to-be-closed",
    );
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

/// Shared-core item F: `string.unpack` initial-position lower bound.
///
/// Lua 5.3's `posrelat` returns `0` for `pos == 0` (and for negatives whose
/// magnitude exceeds the string length), after which `string.unpack`'s
/// `pos = posrelat - 1` underflows and trips the "initial position out of
/// string" guard. 5.4/5.5 switched to `posrelatI`, which maps `0` to `1`, so
/// `pos == 0` is intentionally a valid start there. Confirmed against the
/// lua5.3.6 / lua5.4.7 / lua5.5.0 reference binaries; matches tpack.lua's own
/// version split (5.3 `checkerror("out of string", unpack, "c0", x, 0)`).
#[test]
fn v53_string_unpack_c0_initial_position_lower_bound() {
    // 5.3: pos=0 and out-of-range-negative pos both reject.
    err_contains(
        LuaVersion::V53,
        r#"return string.unpack("c0", "abc", 0)"#,
        "initial position out of string",
    );
    err_contains(
        LuaVersion::V53,
        r#"return string.unpack("c0", "abc", -4)"#,
        "initial position out of string",
    );
}

/// Guard that the item-F gate is 5.3-only: 5.4/5.5 accept `pos == 0` (and the
/// just-out-of-range negative) exactly as their references do, and every
/// version still agrees on the in-range positions. A regression here would
/// mean the `V53` branch leaked into the newer collectors.
#[test]
fn v54_v55_string_unpack_c0_pos_zero_accepted() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        // pos=0 is a valid start on 5.4/5.5 (posrelatI maps 0 -> 1). `c0`
        // unpacks an empty string and returns the next position as 2nd result.
        eq(v, r#"local _, p = string.unpack("c0", "abc", 0); return p"#, "1");
        eq(v, r#"local _, p = string.unpack("c0", "abc", -4); return p"#, "1");
    }
    // In-range positions agree across every version (the gate is inert here).
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(v, r#"local _, p = string.unpack("c0", "abc", 1); return p"#, "1");
        eq(v, r#"local _, p = string.unpack("c0", "abc", -3); return p"#, "1");
        eq(v, r#"local _, p = string.unpack("c0", "abc", 4); return p"#, "4");
    }
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
fn v_argerror_funcname_value_crossversion() {
    // Item B (shared-core): luaL_argerror / luaL_checkoption callsites that used
    // the state-less constructor lost the `to '<fn>'` qualifier and the offending
    // value. They must now carry both on every affected version.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        // collectgarbage invalid option: funcname + offending value.
        err_contains(v, "return collectgarbage('bogusopt')", "to 'collectgarbage'");
        err_contains(v, "return collectgarbage('bogusopt')", "invalid option 'bogusopt'");
        // tonumber base out of range: funcname.
        err_contains(v, "return tonumber('x', 1)", "to 'tonumber'");
        err_contains(v, "return tonumber('x', 1)", "base out of range");
        // table.insert position out of bounds.
        err_contains(v, "return table.insert({}, 5, 5)", "to 'insert'");
        err_contains(v, "return table.insert({}, 5, 5)", "position out of bounds");
        // math.random empty interval.
        err_contains(v, "return math.random(5, 2)", "to 'random'");
        err_contains(v, "return math.random(5, 2)", "interval is empty");
        // no-integer-representation now routes through the faithful path.
        err_contains(v, "return string.rep('x', 1.5)", "to 'rep'");
        err_contains(v, "return string.rep('x', 1.5)", "number has no integer representation");
    }
}

#[test]
fn v_argerror_pack_unpack_funcname_crossversion() {
    // Item B remainder (shared-core): the string.pack / string.unpack /
    // string.packsize argument-error family used the state-less
    // `LuaError::arg_error` constructor, dropping the `to '<fn>'` qualifier on
    // every version. They must now resolve the function name like the rest of
    // the faithful arg_error path. Cases below error identically on 5.3/5.4/5.5
    // (pos-0 acceptance differs by version, so it is asserted separately).
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        // string.unpack: data string too short.
        err_contains(v, r#"return string.unpack("i4", "ab")"#, "to 'unpack'");
        err_contains(v, r#"return string.unpack("i4", "ab")"#, "data string too short");
        // string.pack: unsigned overflow.
        err_contains(v, r#"return string.pack("B", 999)"#, "to 'pack'");
        err_contains(v, r#"return string.pack("B", 999)"#, "unsigned overflow");
        // string.pack: integer overflow.
        err_contains(v, r#"return string.pack("i2", 99999)"#, "to 'pack'");
        err_contains(v, r#"return string.pack("i2", 99999)"#, "integer overflow");
        // string.pack: string longer than given size.
        err_contains(v, r#"return string.pack("c2", "abcd")"#, "to 'pack'");
        err_contains(v, r#"return string.pack("c2", "abcd")"#, "string longer than given size");
        // string.pack: invalid next option for 'X' (getdetails helper, now state-threaded).
        err_contains(v, r#"return string.pack("Xc1", 1)"#, "to 'pack'");
        err_contains(v, r#"return string.pack("Xc1", 1)"#, "invalid next option");
        // string.pack: alignment not power of 2 (getdetails helper).
        err_contains(v, r#"return string.pack("!3i4", 1)"#, "to 'pack'");
        err_contains(v, r#"return string.pack("!3i4", 1)"#, "alignment not power of 2");
        // string.packsize: variable-length format.
        err_contains(v, r#"return string.packsize("s")"#, "to 'packsize'");
        err_contains(v, r#"return string.packsize("s")"#, "variable-length format");
        // string.format: %s with embedded zeros + modifiers (was state-less too).
        err_contains(v, r#"return string.format("%5s", "a\0b")"#, "to 'format'");
        err_contains(v, r#"return string.format("%5s", "a\0b")"#, "string contains zeros");

        // Guard: the bare truncated `bad argument #N (` form (no `to '<fn>'`)
        // must NOT survive for these callsites on any version.
        let e = run(v, r#"return string.pack("B", 999)"#).unwrap_err();
        assert!(e.contains("to 'pack'"),
            "v{v:?} string.pack argerror lost funcname: {e}");
    }
    // pos-0 lower-bound rejection is 5.3-only; when it fires, it must also carry
    // the funcname. (5.4/5.5 accept pos 0, asserted elsewhere.)
    err_contains(LuaVersion::V53, r#"return string.unpack("c0", "abc", 0)"#, "to 'unpack'");
    err_contains(LuaVersion::V53, r#"return string.unpack("c0", "abc", 0)"#,
        "initial position out of string");
}

#[test]
fn v_argerror_perversion_wording() {
    // Item B per-version wording splits.
    // utf8.offset: 5.3 says "out of range"; 5.4/5.5 say "out of bounds".
    err_contains(LuaVersion::V53, "return utf8.offset('abc', 0, 0)", "position out of range");
    err_contains(LuaVersion::V54, "return utf8.offset('abc', 0, 0)", "position out of bounds");
    err_contains(LuaVersion::V55, "return utf8.offset('abc', 0, 0)", "position out of bounds");
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return utf8.offset('abc', 0, 0)", "to 'offset'");
    }
    // string.format width: 5.3 uses the old scanformat message; 5.4/5.5 the
    // checkformat message including the offending spec.
    err_contains(LuaVersion::V53, "return string.format('%200d', 1)",
        "invalid format (width or precision too long)");
    err_contains(LuaVersion::V54, "return string.format('%200d', 1)",
        "invalid conversion specification: '%200d'");
    err_contains(LuaVersion::V55, "return string.format('%200d', 1)",
        "invalid conversion specification: '%200d'");
    // string.format unknown conversion: 5.3 "invalid option", 5.4/5.5 "invalid conversion".
    err_contains(LuaVersion::V53, "return string.format('%y', 1)", "invalid option '%y' to 'format'");
    err_contains(LuaVersion::V54, "return string.format('%y', 1)", "invalid conversion '%y' to 'format'");
    err_contains(LuaVersion::V55, "return string.format('%y', 1)", "invalid conversion '%y' to 'format'");
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
    // Item B: the message carries the function name and the offending value,
    // not the bare truncated `invalid option`.
    err_contains(LuaVersion::V55,
        "return collectgarbage('setpause', 100)", "to 'collectgarbage'");
    err_contains(LuaVersion::V55,
        "return collectgarbage('setpause', 100)", "invalid option 'setpause'");
    err_contains(LuaVersion::V55,
        "return collectgarbage('setstepmul', 100)", "invalid option 'setstepmul'");
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
    // invalid param name errors via luaL_checkoption, carrying the value.
    err_contains(LuaVersion::V55,
        "return collectgarbage('param', 'bogus')", "invalid option 'bogus'");
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

#[test]
fn v55_error_nil_becomes_no_error_object() {
    // 5.5's luaG_errormsg converts a nil error object to the literal string
    // "<no error object>" after the message handler runs (ldebug.c). The `run`
    // wrapper pcalls the chunk and returns `tostring(error_object)`, so on 5.5
    // the propagated object is the string and on 5.3/5.4 it stays nil.
    // error(nil): explicit nil object.
    err_contains(LuaVersion::V55,
        "error(nil)", "<no error object>");
    // error() with no argument: object defaults to nil.
    err_contains(LuaVersion::V55,
        "error()", "<no error object>");
    // nested pcall still sees the converted string.
    eq(LuaVersion::V55,
        "local ok, e = pcall(function() error(nil) end); return type(e) .. ':' .. tostring(e)",
        "string:<no error object>");
    // xpcall whose handler returns nil also settles to the string (the
    // conversion runs on the handler result, matching upstream ordering).
    eq(LuaVersion::V55,
        "local ok, e = xpcall(function() error('x') end, function() return nil end); \
         return type(e) .. ':' .. tostring(e)",
        "string:<no error object>");
}

#[test]
fn v53_v54_error_nil_stays_nil() {
    // Regression guard: 5.3/5.4 leave a nil error object as nil (no conversion).
    for v in [LuaVersion::V53, LuaVersion::V54] {
        eq(v,
            "local ok, e = pcall(function() error(nil) end); return type(e) .. ':' .. tostring(e)",
            "nil:nil");
        eq(v,
            "local ok, e = pcall(function() error() end); return type(e) .. ':' .. tostring(e)",
            "nil:nil");
        // A real string error object is untouched (sanity: conversion is nil-only).
        eq(v,
            "local ok, e = pcall(function() error('boom') end); return (e:gsub('^.*: ', ''))",
            "boom");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Shared-core item D: the `\u{...}` codepoint upper bound in the lexer.
// `llex.c readutf8esc` caps the value differently by family: 5.3 bounds the
// running value at 0x10FFFF (per-digit, *after* the shift), while 5.4/5.5
// bound it at 0x7FFFFFFF (per-digit, *before* the shift). The fix version-gates
// only the 5.3 path; 5.4/5.5 are unchanged. (5.1/5.2 have no `\u{}` escape.)
// Reproduced against the reference binaries via `specs/oracle/diff_one.sh`.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_utf8_escape_caps_at_10ffff() {
    // 0x10FFFF is the largest accepted codepoint on 5.3.
    eq(LuaVersion::V53, r#"return #"\u{10FFFF}""#, "4");
    // One past the cap, and the legacy 5.4/5.5 ceiling, both reject on 5.3.
    err_contains(LuaVersion::V53, r#"return #"\u{110000}""#, "UTF-8 value too large");
    err_contains(LuaVersion::V53, r#"return #"\u{110001}""#, "UTF-8 value too large");
    err_contains(LuaVersion::V53, r#"return #"\u{7FFFFFFF}""#, "UTF-8 value too large");
}

#[test]
fn v54_v55_utf8_escape_caps_at_7fffffff() {
    // Guard that the unaffected versions keep the wider 0x7FFFFFFF ceiling:
    // values 5.3 rejects (>0x10FFFF up to 0x7FFFFFFF) are still accepted here,
    // and only values above 0x7FFFFFFF are rejected.
    for v in [LuaVersion::V54, LuaVersion::V55] {
        eq(v, r#"return #"\u{10FFFF}""#, "4");
        eq(v, r#"return #"\u{110000}""#, "4");
        eq(v, r#"return #"\u{7FFFFFFF}""#, "6");
        err_contains(v, r#"return #"\u{80000000}""#, "UTF-8 value too large");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Shared-core item E: `print` and the global `tostring`.
//
// 5.1/5.2/5.3 `luaB_print` fetch the *global* `tostring` and call it on each
// argument, so a `nil` global makes `print` raise "attempt to call a nil
// value", a custom global `tostring` is honored, and a result that is neither
// a string nor a coercible number raises "'tostring' must return a string to
// 'print'". 5.4/5.5 `luaB_print` use `luaL_tolstring` directly and ignore the
// global `tostring` entirely. Reproduced against the reference binaries via
// `specs/oracle/diff_one.sh`.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_print_calls_global_tostring() {
    // A nil global `tostring` makes print raise when it has an argument. The
    // mutation is local to the inner chunk's `pcall`; restoring it keeps the
    // wrapper's own `tostring(result)` (and other tests) unaffected.
    err_contains(
        LuaVersion::V53,
        "local s = tostring; tostring = nil; local ok, e = pcall(print, 1); tostring = s; error(e, 0)",
        "attempt to call a nil value",
    );
    // A custom global `tostring` is honored (no error, runs to completion).
    eq(
        LuaVersion::V53,
        "local s = tostring; tostring = function(x) return 'X' .. x end; print(7); tostring = s; return 'ok'",
        "ok",
    );
    // A result that is neither a string nor a coercible number raises.
    err_contains(
        LuaVersion::V53,
        "local s = tostring; tostring = function(x) return {} end; local ok, e = pcall(print, 1); tostring = s; error(e, 0)",
        "'tostring' must return a string to 'print'",
    );
    // A number return is coercible and accepted (mirrors C `lua_tolstring`).
    eq(
        LuaVersion::V53,
        "local s = tostring; tostring = function(x) return 42 end; print(1); tostring = s; return 'ok'",
        "ok",
    );
    // No arguments: the nil global is never called, so no error.
    eq(
        LuaVersion::V53,
        "local s = tostring; tostring = nil; print(); tostring = s; return 'ok'",
        "ok",
    );
}

#[test]
fn v54_v55_print_ignores_global_tostring() {
    // Guard the unaffected versions: print uses luaL_tolstring directly, so a
    // nil or non-string-returning global `tostring` does NOT affect print.
    for v in [LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            "local s = tostring; tostring = nil; print(1); tostring = s; return 'ok'",
            "ok",
        );
        eq(
            v,
            "local s = tostring; tostring = function(x) return {} end; print(1); tostring = s; return 'ok'",
            "ok",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Shared-core: `utf8.char` codepoint ceiling is version-split. 5.3 rejects
// codepoints above 0x10FFFF ("value out of range"); 5.4/5.5 widened the encoder
// to accept up to 0x7FFFFFFF (the lax extended-UTF-8 range), rejecting only
// above that. Distinct from the *lexer* `\u{}` ceiling (item D). Reproduced
// against the reference binaries via `specs/oracle/diff_one.sh`; blocks the
// `utf8.lua:151` `checkerror("value out of range", ...)` on 5.3.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_utf8_char_caps_at_10ffff() {
    eq(LuaVersion::V53, "return #utf8.char(0x10FFFF)", "4");
    err_contains(
        LuaVersion::V53,
        "local ok, e = pcall(utf8.char, 0x110000); error(e, 0)",
        "value out of range",
    );
}

#[test]
fn v54_v55_utf8_char_caps_at_7fffffff() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        eq(v, "return #utf8.char(0x110000)", "4");
        eq(v, "return #utf8.char(0x7FFFFFFF)", "6");
        err_contains(
            v,
            "local ok, e = pcall(utf8.char, 0x80000000); error(e, 0)",
            "value out of range",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Shared-core: `string.format` spec-scanner error wording is version-split.
// 5.3 `scanformat` raises "invalid format (repeated flags)" when the flag run
// reaches `sizeof(FLAGS) == 6` characters; 5.4/5.5 `getformat` fold this into a
// single "invalid format (too long)". Blocks `strings.lua:303`. Captured from
// the reference binaries.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_format_repeated_flags() {
    err_contains(
        LuaVersion::V53,
        "local aux = string.rep('0', 600); local ok, e = pcall(string.format, '%'..aux..'d', 10); error(e, 0)",
        "invalid format (repeated flags)",
    );
}

#[test]
fn v54_v55_format_too_long() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(
            v,
            "local aux = string.rep('0', 600); local ok, e = pcall(string.format, '%'..aux..'d', 10); error(e, 0)",
            "invalid format (too long)",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Shared-core: `string.pack`/`packsize` `c<n>` size parsing is version-split.
// 5.3/5.4 read the size into a C `int`; a huge numeral overflows it and the
// trailing digit is mis-read as a new option ("invalid format option '<d>'").
// 5.5 widened `getnum` to `size_t`, so `c<near-maxinteger>` parses cleanly and
// is bounded by the running-total checks ("format result too large" for
// `packsize`, "result too long" for `pack`). Blocks `tpack.lua` on 5.5.
// Captured from the reference binaries.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_v54_pack_csize_overflows_int() {
    for v in [LuaVersion::V53, LuaVersion::V54] {
        err_contains(
            v,
            "local f = string.format('c%d', math.maxinteger - 9); local ok, e = pcall(string.packsize, f); error(e, 0)",
            "invalid format option '6'",
        );
    }
}

#[test]
fn v55_pack_csize_wide() {
    // The near-maxinteger size parses and packsize returns it.
    eq(
        LuaVersion::V55,
        "return string.packsize(string.format('c%d', math.maxinteger - 9))",
        "9223372036854775798",
    );
    // packsize running total overflow.
    err_contains(
        LuaVersion::V55,
        "local f = string.format('c%dc10', math.maxinteger - 9); local ok, e = pcall(string.packsize, f); error(e, 0)",
        "format result too large",
    );
    // pack running total overflow ("result too long"), reported on arg #1.
    err_contains(
        LuaVersion::V55,
        "local f = string.format('xxxxxxxxxx c%d', math.maxinteger - 9); local ok, e = pcall(string.pack, f); error(e, 0)",
        "result too long",
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Shared-core: `math.random` argument handling is version-split. 5.3 treats
// `random(N)` as `[1, N]` (so `random(0)` is the empty `[1, 0]`) and rejects
// intervals whose width overflows a signed integer
// (`low >= 0 || up <= LUA_MAXINTEGER + low` else "interval too large"). 5.4/5.5
// rewrote the generator around a `project` bit-mask: `random(0)` returns a
// full-range integer and any interval is accepted. Blocks `math.lua` on 5.3.
// Captured from the reference binaries.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v53_random_interval_guards() {
    err_contains(
        LuaVersion::V53,
        "local ok, e = pcall(math.random, 0); error(e, 0)",
        "interval is empty",
    );
    err_contains(
        LuaVersion::V53,
        "local ok, e = pcall(math.random, math.mininteger, 0); error(e, 0)",
        "interval too large",
    );
    err_contains(
        LuaVersion::V53,
        "local ok, e = pcall(math.random, -1, math.maxinteger); error(e, 0)",
        "interval too large",
    );
    // A normal interval still works.
    eq(LuaVersion::V53, "local r = math.random(1, 6); return r >= 1 and r <= 6", "true");
}

#[test]
fn v54_v55_random_zero_and_full_range() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        // random(0) returns a full-range integer (no error).
        eq(v, "return math.type(math.random(0))", "integer");
        // Full integer range is accepted.
        eq(v, "return math.type(math.random(math.mininteger, math.maxinteger))", "integer");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5.2 — the float-only + _ENV bridge to the legacy family. 5.2 reuses the
// modern _ENV core but is FLOAT-ONLY (no integer subtype, no //, no bitwise
// ops, no int-specific stdlib) and carries the 5.2 roster (bit32 present,
// utf8 absent, getfenv/setfenv removed, unpack/loadstring globals retained).
// Every expected value captured from /tmp/lua-refs/bin/lua5.2.4.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v52_float_only_number_model() {
    // Integer-valued floats print without ".0" (float-only: no int/float split).
    eq(LuaVersion::V52, "return 10/2", "5");
    eq(LuaVersion::V52, "return 2^2", "4");
    eq(LuaVersion::V52, "return tostring(1.0)", "1");
    eq(LuaVersion::V52, "return 1 .. 2", "12");
    eq(LuaVersion::V52, "return 1.5", "1.5");
    eq(LuaVersion::V52, "return math.floor(3.7)", "3");
    eq(LuaVersion::V52, "return string.format('%d', 42)", "42");
    // All numeric literals lex as float, so a 2^53+1 literal loses precision
    // exactly as lua5.2.4 does (it has no i64 to preserve it).
    eq(LuaVersion::V52, "return 9007199254740993", "9.007199254741e+15");
    eq(LuaVersion::V52, "return _VERSION", "Lua 5.2");
}

#[test]
fn v52_math_roster_is_float_only() {
    // The 5.3 integer-subtype members are absent.
    eq(LuaVersion::V52, "return type(math.type)", "nil");
    eq(LuaVersion::V52, "return type(math.tointeger)", "nil");
    eq(LuaVersion::V52, "return type(math.maxinteger)", "nil");
    eq(LuaVersion::V52, "return type(math.mininteger)", "nil");
    eq(LuaVersion::V52, "return type(math.ult)", "nil");
    // The LUA_COMPAT_MATHLIB deprecated roster IS present in the default build.
    eq(LuaVersion::V52, "return type(math.atan2)", "function");
    eq(LuaVersion::V52, "return type(math.cosh)", "function");
    eq(LuaVersion::V52, "return type(math.pow)", "function");
    eq(LuaVersion::V52, "return type(math.log10)", "function");
    eq(LuaVersion::V52, "return type(math.frexp)", "function");
}

#[test]
fn v52_stdlib_roster_delta() {
    eq(LuaVersion::V52, "return type(bit32)", "table");
    eq(LuaVersion::V52, "return bit32.band(6,3)", "2");
    eq(LuaVersion::V52, "return type(utf8)", "nil");
    eq(LuaVersion::V52, "return type(table.pack)", "function");
    eq(LuaVersion::V52, "return type(table.unpack)", "function");
    eq(LuaVersion::V52, "return type(table.move)", "nil");
    eq(LuaVersion::V52, "return type(unpack)", "function");
    eq(LuaVersion::V52, "return type(loadstring)", "function");
    eq(LuaVersion::V52, "return type(coroutine.close)", "nil");
    eq(LuaVersion::V52, "return type(warn)", "nil");
    eq(LuaVersion::V52, "return type(table.create)", "nil");
    // _ENV present; fenv globals removed in 5.2.
    eq(LuaVersion::V52, "return type(_ENV)", "table");
    eq(LuaVersion::V52, "return type(getfenv)", "nil");
    eq(LuaVersion::V52, "return type(setfenv)", "nil");
    // package.loaders kept as alias of searchers in 5.2 (dropped in 5.3).
    eq(LuaVersion::V52, "return type(package.loaders)", "table");
    eq(LuaVersion::V52, "return type(package.searchers)", "table");
}

#[test]
fn v52_rejects_53_integer_operators() {
    // The 5.3 integer operators do not exist in 5.2; messages match lua5.2.4.
    err_contains(LuaVersion::V52, "return 7//2", "unexpected symbol near '/'");
    err_contains(LuaVersion::V52, "return 6 & 3", "near '&'");
    err_contains(LuaVersion::V52, "return 6 | 3", "near '|'");
    err_contains(LuaVersion::V52, "return 1 << 4", "unexpected symbol near '<'");
    err_contains(LuaVersion::V52, "return 256 >> 4", "unexpected symbol near '>'");
    err_contains(LuaVersion::V52, "return 5 ~ 3", "near '~'");
    err_contains(LuaVersion::V52, "return ~0", "unexpected symbol near '~'");
    // The 5.4 <const> attribute syntax is also absent.
    err_contains(LuaVersion::V52, "local x <const> = 1", "unexpected symbol near '<'");
}

#[test]
fn v52_format_int_family_truncates() {
    // Float-only: string.format's %d/%i/%u/%o/%x/%X truncate a non-integral
    // number toward zero (lua5.2.4), where 5.3+ raises "no integer
    // representation". A string operand is coerced to a number first.
    eq(LuaVersion::V52, "return string.format('%d', 3.5)", "3");
    eq(LuaVersion::V52, "return string.format('%d', -3.5)", "-3");
    eq(LuaVersion::V52, "return string.format('%d', 2.9)", "2");
    eq(LuaVersion::V52, "return string.format('%x', 255.9)", "ff");
    eq(LuaVersion::V52, "return string.format('%d', '3.5')", "3");
    // An integer operand still formats exactly.
    eq(LuaVersion::V52, "return string.format('%d', 42)", "42");
}

#[test]
fn v53_plus_format_int_family_still_strict() {
    // The truncation behavior is V52-gated: the dual-number versions keep the
    // strict integer-representation check for the same %d-family conversions.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return string.format('%d', 3.5)", "no integer representation");
        eq(v, "return string.format('%d', 42)", "42");
    }
}

#[test]
fn v52_goto_and_basics() {
    eq(LuaVersion::V52, "do goto x ::x:: end return 'ok'", "ok");
    eq(LuaVersion::V52, "return unpack({10,20,30})", "10");
    eq(LuaVersion::V52, "return loadstring('return 7')()", "7");
    eq(LuaVersion::V52, "return ('10'+5)", "15");
}
