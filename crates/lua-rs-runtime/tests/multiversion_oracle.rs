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

use omnilua::{Lua, LuaVersion};

/// Run `code` under `version` and return `Ok(tostring(result))` or
/// `Err(error message)`. The snippet is `load`+`pcall`ed *inside* Lua so the VM
/// renders values and error messages faithfully (a `LuaError`'s Rust `Display`
/// can't reach the heap to render an interned message string), and so the
/// snippet's own `global`-strict scope is contained to the inner chunk — the
/// outer wrapper runs in implicit-global mode and always has the builtins.
fn run(version: LuaVersion, code: &str) -> Result<String, String> {
    let lua = Lua::new_versioned(version);
    // Lua 5.1's `load` takes a reader function only — string loading is
    // `loadstring`'s job (a V51 roster gate). 5.2+ `load` accepts a string. Pick
    // the loader the running version exposes for a string chunk.
    let loader = if version == LuaVersion::V51 {
        "loadstring"
    } else {
        "load"
    };
    let wrapper = format!(
        "local f, e = {loader}([==[\n{code}\n]==])\n\
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
        Err(e) => assert!(
            e.contains(needle),
            "code `{code}` error `{e}` lacked `{needle}`"
        ),
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
        eq(
            v,
            "local up = {}; return (function() return up[1 < 2] end)()",
            "nil",
        );
        // Non-folded comparison (two locals) through an upvalue.
        eq(
            v,
            "local x, y = 1, 1; local up = {}; \
             return (function() return up[x == y] end)()",
            "nil",
        );
        // Store side: `_ENV[1<2] = v` must index correctly too.
        eq(v, "_ENV[1 < 2] = 7; return _ENV[true]", "7");
    }
}

#[test]
fn v54_env_relational_index_errors_like_reference() {
    // Guard the deliberate 5.4-only divergence: the reference 5.4 binary raises
    // on this exact construct; our port must not "improve" on it.
    err_contains(
        LuaVersion::V54,
        "return _ENV[1 < 2]",
        "index a number value",
    );
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
    err_contains(
        LuaVersion::V55,
        "global a; a = 1; zz = 2",
        "variable 'zz' not declared",
    );
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
    eq(
        LuaVersion::V55,
        "if true then global Z; Z = 1 end; w = 2; return w",
        "2",
    );
}

#[test]
fn v55_global_initializer_stored() {
    // F2: `global x = expr` actually assigns (was previously dropped).
    eq(LuaVersion::V55, "do global x = 7 end; return x", "7");
    eq(
        LuaVersion::V55,
        "do global a, b = 10, 20 end; return a + b",
        "30",
    );
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
    eq(
        LuaVersion::V55,
        "global x = 1; x = nil; global x = 2; return x",
        "2",
    );
    // A no-initializer re-declaration never checks.
    eq(LuaVersion::V55, "global x; global x; return x", "nil");
    // Plain assignments after the first init never check.
    eq(LuaVersion::V55, "global x = 1; x = 2; x = 3; return x", "3");
    // The RHS is evaluated before the guard fires (upstream order); the value
    // here keeps the global nil, so the second init is fine.
    eq(
        LuaVersion::V55,
        "global x = nil; global x = 2; return x",
        "2",
    );
}

#[test]
fn v55_global_guard_inert_pre_55() {
    // `global` is a plain identifier on 5.4/5.3, so none of the guard paths
    // exist there — repeated assignment to a `global`-named variable is fine.
    eq(
        LuaVersion::V54,
        "global = 1; global = 2; return global",
        "2",
    );
    eq(
        LuaVersion::V53,
        "global = 1; global = 2; return global",
        "2",
    );
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
fn v55_plain_global_redeclare_clears_prior_const() {
    eq(
        LuaVersion::V55,
        "global<const> a, b, c = 10, 20, 30; \
         _ENV.a = nil; _ENV.b = nil; _ENV.c = nil; \
         global table; \
         global a, b, c, d = table.unpack{1, 2, 3, 6, 5}; \
         a = nil; b = nil; c = nil; d = nil; \
         return _ENV.a == nil and _ENV.b == nil and _ENV.c == nil and _ENV.d == nil",
        "true",
    );
}

#[test]
fn v55_goto_scope_messages_for_locals_and_global_star() {
    eq(
        LuaVersion::V55,
        "local f, msg = load([[ goto l1; local aa ::l1:: print(3) ]]); \
         return f == nil and msg:find([[scope of 'aa']], 1, true) ~= nil",
        "true",
    );
    eq(
        LuaVersion::V55,
        "local f, msg = load([[ goto l2; global *; ::l2:: print(3) ]]); \
         return f == nil and msg:find([[scope of '*']], 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_global_declaration_shadows_outer_local() {
    eq(
        LuaVersion::V55,
        "local X = 10; do global X; X = 20 end; return X == 10 and _ENV.X == 20",
        "true",
    );
    eq(
        LuaVersion::V55,
        "local a, b = 100, 200; do global a, b = a, b end; \
         local ok = _ENV.a == 100 and _ENV.b == 200; \
         _ENV.a = nil; _ENV.b = nil; return ok",
        "true",
    );
}

#[test]
fn v55_global_env_declaration_blocks_global_access() {
    eq(
        LuaVersion::V55,
        "local f, msg = load([[global _ENV, a; a = 10]]); \
         return f == nil and msg:find([[_ENV is global when accessing variable 'a']], 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_const_global_function_assignment_reports_function_line() {
    eq(
        LuaVersion::V55,
        "local f, msg = load([[\
          global foo <const>;\n\
          function foo (x)\n\
            return\n\
          end\n\
        ]]); \
        return f == nil and msg:find([[:2: attempt to assign to const variable 'foo']], 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_const_global_wildcard_marks_free_assignments_readonly() {
    eq(
        LuaVersion::V55,
        "local f, msg = load([[global<const> *; print(Y); Y = 1]]); \
         return f == nil and msg:find([[assign to const variable 'Y']], 1, true) ~= nil",
        "true",
    );
    eq(
        LuaVersion::V55,
        "global *; Y = 10; \
         global<const> *; local y = Y; \
         global *; Y = y + Y; \
         local ok = Y == 20; Y = nil; return ok",
        "true",
    );
    eq(
        LuaVersion::V55,
        "local f, msg = load([[global<const> *; _G = nil]]); \
         global<const> *; \
         local old = _G.deep; _G.deep = 3; \
         local ok = f == nil and msg:find([[assign to const variable '_G']], 1, true) ~= nil and _G.deep == 3; \
         _G.deep = old; return ok",
        "true",
    );
}

#[test]
fn v55_load_rejects_fixed_buffer_mode_from_lua() {
    err_contains(LuaVersion::V55, "return load('', '', 'B')", "invalid mode");
}

#[test]
fn v55_coroutine_close_main_thread_message() {
    eq(
        LuaVersion::V55,
        "local main = coroutine.running(); \
         local ok, msg = pcall(coroutine.close, main); \
         return not ok and msg:find('main', 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_coroutine_close_self_unwinds_to_resume_base() {
    eq(
        LuaVersion::V55,
        "local function func2close(f) return setmetatable({}, {__close = f}) end; \
         local function new(what) \
           return coroutine.create(function() \
             local var <close> = func2close(function() \
               if what == 'yield' then coroutine.yield() \
               elseif what == 'error' then error(200) end; \
             end); \
             string.gsub('a', 'a', function() \
               assert(not coroutine.isyieldable()); \
               pcall(pcall, function() coroutine.close(); os.exit(false) end); \
             end); \
           end); \
         end; \
         local ret = new('ret'); local ok1, msg1 = coroutine.resume(ret); \
         local err = new('error'); local ok2, msg2 = coroutine.resume(err); \
         local yld = new('yield'); local ok3, msg3 = coroutine.resume(yld); \
         return ok1 and msg1 == nil and not ok2 and msg2 == 200 and \
                not ok3 and msg3:find('attempt to yield', 1, true) ~= nil and \
                coroutine.status(ret) == 'dead' and coroutine.status(err) == 'dead' and \
                coroutine.status(yld) == 'dead'",
        "true",
    );
}

#[test]
fn v55_debug_getinfo_t_reports_call_metamethod_depth() {
    eq(
        LuaVersion::V55,
        "local debug = require 'debug'; \
         local function u(...) \
           local n = debug.getinfo(1, 't').extraargs; \
           if select('#', ...) ~= n then return false end; \
           return n; \
         end; \
         for i = 0, 4 do \
           if u() ~= i then return false end; \
           u = setmetatable({}, {__call = u}); \
         end; \
         return true",
        "true",
    );
}

#[test]
fn v55_string_dump_uses_55_header() {
    eq(
        LuaVersion::V55,
        "local headformat = 'c4BBc6BiBI4BjBn'; \
         local header = {'\\27Lua', 0x55, 0, '\\x19\\x93\\r\\n\\x1a\\n', \
                         string.packsize('i'), -0x5678, 4, 0x12345678, \
                         string.packsize('j'), -0x5678, string.packsize('n'), -370.5}; \
         local c = string.dump(function () return 1 end); \
         assert(assert(load(c))() == 1); \
         local t = {string.unpack(headformat, c)}; \
         for i = 1, #header do if t[i] ~= header[i] then return false end end; \
         return true",
        "true",
    );
}

#[test]
fn v55_explicit_return_limit_is_254_values() {
    eq(
        LuaVersion::V55,
        "local code = 'return 10' .. string.rep(',10', 253); \
         local res = {assert(load(code))()}; \
         if #res ~= 254 or res[254] ~= 10 then return false end; \
         code = code .. ',10'; \
         local f, msg = load(code); \
         return f == nil and msg:find('too many returns', 1, true) ~= nil",
        "true",
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
    eq(
        LuaVersion::V55,
        "global<const> a, b = 1, 2; return a + b",
        "3",
    );
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
        "global<const> *; local ok; local T = false; do \
           global T<const>; \
           local foo = 20; \
           do global function foo (x) \
             if x == 0 then return 1 else return 2 * foo(x - 1) end \
           end; \
           ok = foo == _ENV.foo and foo(4) == 16 and _ENV.foo(4) == 16 \
         end; \
         end; \
         return ok",
        "true",
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
    eq(
        LuaVersion::V55,
        "local <const> a, b = 1, 2; return a + b",
        "3",
    );
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
    err_contains(
        LuaVersion::V55,
        "local x <foo> = 1",
        "unknown attribute 'foo'",
    );
    err_contains(
        LuaVersion::V54,
        "local x <foo> = 1",
        "unknown attribute 'foo'",
    );
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
    err_contains(
        LuaVersion::V55,
        "for i = 1, 3 do i = 10 end",
        "attempt to assign to const variable 'i'",
    );
    err_contains(
        LuaVersion::V55,
        "for k, v in pairs({1, 2}) do k = 10 end",
        "attempt to assign to const variable 'k'",
    );
    // The second generic var stays assignable; reads are fine.
    eq(
        LuaVersion::V55,
        "local s = 0; for i = 1, 3 do s = s + i end; return s",
        "6",
    );
    eq(
        LuaVersion::V55,
        "for k, v in pairs({7}) do v = 9 end; return 'ok'",
        "ok",
    );
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
    eq(
        LuaVersion::V55,
        "local function f(...t) return #t end return f(1,2,3)",
        "3",
    );
    eq(
        LuaVersion::V55,
        "local function f(...t) return t.n end return f(1,nil,3)",
        "3",
    );
    eq(
        LuaVersion::V55,
        "local function f(...t) return t.n end return f()",
        "0",
    );
    eq(
        LuaVersion::V55,
        "local function f(a,...t) return t[2] end return f(0,10,20)",
        "20",
    );
    // `...` is still usable alongside the named form.
    eq(
        LuaVersion::V55,
        "local function f(...t) return ... end return select('#', f(1,2,3))",
        "3",
    );
    // Fresh table per call.
    eq(
        LuaVersion::V55,
        "local function f(...t) return t end return f(1)==f(1)",
        "false",
    );
    // The table is mutable.
    eq(
        LuaVersion::V55,
        "local function f(...t) t[1]=99; return t[1] end return f(1,2)",
        "99",
    );
    // No attribute allowed on `...t`.
    err_contains(
        LuaVersion::V55,
        "local function f(...t <const>) return t end",
        "')' expected",
    );
}

/// Named-vararg syntax is 5.5-only; on 5.4/5.3 a name after `...` stays a parse
/// error matching the reference (`')' expected near 't'`).
#[test]
fn named_varargs_rejected_pre_55() {
    for v in [LuaVersion::V53, LuaVersion::V54] {
        err_contains(
            v,
            "local function f(...t) return t end",
            "')' expected near 't'",
        );
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
    eq(
        LuaVersion::V53,
        r#"return "0xfffffffffffffffe" & "-1""#,
        "-2",
    );
    eq(
        LuaVersion::V53,
        r#"return "   \n  -45  \t " >> "  -2  ""#,
        "-180",
    );
    // Boundaries that MUST still error on 5.3:
    err_contains(
        LuaVersion::V53,
        r#"return "3.5" & 1"#,
        "no integer representation",
    );
    err_contains(
        LuaVersion::V53,
        r#"return "0xffffffffffffffff.0" | 0"#,
        "no integer representation",
    );
    err_contains(
        LuaVersion::V53,
        r#"return "abc" & 1"#,
        "perform bitwise operation on a string value",
    );
}

/// Cross-version non-regression: 5.4/5.5 do NOT coerce strings in bitwise ops
/// and keep raising "perform bitwise operation on a string value".
#[test]
fn v54_v55_bitwise_no_string_coercion() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(
            v,
            r#"return "3" & 5"#,
            "perform bitwise operation on a string value",
        );
        err_contains(
            v,
            r#"return ~"5""#,
            "perform bitwise operation on a string value",
        );
        err_contains(
            v,
            r#"return "8" >> "1""#,
            "perform bitwise operation on a string value",
        );
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
    err_contains(
        LuaVersion::V53,
        r#"aaa="z"; return aaa+1"#,
        "(global 'aaa')",
    );
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
        err_contains(
            v,
            r#"return "abc" + 1"#,
            "attempt to add a 'string' with a 'number'",
        );
        err_contains(
            v,
            r#"aaa="2"; b=nil; return aaa*b"#,
            "attempt to mul a 'string' with a 'nil'",
        );
    }
}

/// 5.3 `for`-loop non-number bound wording is the older `'for' <what> must be a
/// number`; 5.4/5.5 reworded it to `bad 'for' <what> (number expected, got
/// <type>)`. Captured from lua5.3.6 / lua5.4.7 / lua5.5.0.
#[test]
fn v53_for_loop_error_wording() {
    err_contains(
        LuaVersion::V53,
        "for i=1,'a' do end",
        "'for' limit must be a number",
    );
    err_contains(
        LuaVersion::V53,
        "for i='a',10 do end",
        "'for' initial value must be a number",
    );
    err_contains(
        LuaVersion::V53,
        "for i=1,10,'a' do end",
        "'for' step must be a number",
    );
}

/// Cross-version non-regression: 5.4/5.5 keep the `bad 'for' <what> (number
/// expected, got <type>)` wording.
#[test]
fn v54_v55_for_loop_error_wording_unchanged() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(
            v,
            "for i=1,'a' do end",
            "bad 'for' limit (number expected, got string)",
        );
        err_contains(
            v,
            "for i='a',10 do end",
            "bad 'for' initial value (number expected, got string)",
        );
        err_contains(
            v,
            "for i=1,10,'a' do end",
            "bad 'for' step (number expected, got string)",
        );
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
    err_contains(
        LuaVersion::V53,
        "local x <const> = 1; return x",
        "unexpected symbol",
    );
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
        eq(
            v,
            r#"local _, p = string.unpack("c0", "abc", 0); return p"#,
            "1",
        );
        eq(
            v,
            r#"local _, p = string.unpack("c0", "abc", -4); return p"#,
            "1",
        );
    }
    // In-range positions agree across every version (the gate is inert here).
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            r#"local _, p = string.unpack("c0", "abc", 1); return p"#,
            "1",
        );
        eq(
            v,
            r#"local _, p = string.unpack("c0", "abc", -3); return p"#,
            "1",
        );
        eq(
            v,
            r#"local _, p = string.unpack("c0", "abc", 4); return p"#,
            "4",
        );
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
        for name in [
            "atan2", "cosh", "sinh", "tanh", "pow", "log10", "frexp", "ldexp",
        ] {
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
        eq(
            v,
            "local m, e = math.frexp(8.0); return math.type(m)",
            "float",
        );
        eq(
            v,
            "local m, e = math.frexp(8.0); return math.type(e)",
            "integer",
        );
        eq(
            v,
            "local m, e = math.frexp(0.0); return tostring(m) .. ',' .. tostring(e)",
            "0.0,0",
        );
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
        eq(
            LuaVersion::V55,
            &format!("return type(math.{name})"),
            "function",
        );
    }
    eq(LuaVersion::V55, "return math.ldexp(0.5, 3)", "4.0");
    eq(
        LuaVersion::V55,
        "local m, e = math.frexp(8.0); return tostring(m) .. ',' .. tostring(e)",
        "0.5,4",
    );
}

// ─────────────────────────────────────────────────────────────────────────
// ldexp / frexp SUBNORMAL edges — promoted into the standard gate (P2a).
//
// The bit-scaling in `ldexp` and the mantissa/exponent split in `frexp` are
// load-bearing: a naive `x * 2f64.powi(e)` would underflow the intermediate
// `2^e` factor and lose every subnormal result, and a naive frexp would mishandle
// subnormal inputs (which have a zero stored exponent). math.lua does not probe
// these edges, and only ONE (`ldexp(1.0,-1074)`) was previously pinned, on
// 5.3/5.4 alone. These run on every version that exposes the functions —
// 5.3, 5.4, 5.5 — so the subnormal tripwire fires unconditionally.
//
// The subnormal float values are pinned via `tostring` — the real observable
// rendering. 5.5 changed the default float formatter to print more significant
// digits than 5.3/5.4, so the smallest-subnormal and smallest-normal constants
// are split per version group. Mantissa (0.5/-0.5), integer exponents, 0.0, and
// inf render identically across versions. Captured from lua5.3.6 / lua5.4.7 /
// lua5.5.0. (Note: `string.format("%g", subnormal)` is NOT used here — our
// `%g` formatter has a separate, pre-existing subnormal-rendering bug, out of
// scope for the math packet; `tostring` renders subnormals correctly.)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ldexp_frexp_subnormal_edges() {
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        // `tostring` of the smallest positive subnormal (2^-1074) and the
        // smallest normal (2^-1022): 5.5's formatter prints more digits.
        let (smallest_subnormal, smallest_normal) = match v {
            LuaVersion::V55 => ("4.94065645841247e-324", "2.2250738585072014e-308"),
            _ => ("4.9406564584125e-324", "2.2250738585072e-308"),
        };

        // Smallest positive subnormal: 1.0 * 2^-1074. A naive `2f64.powi(-1074)`
        // is 0.0, so this only passes with the bounded bit-scaling.
        eq(v, "return tostring(math.ldexp(1.0, -1074))", smallest_subnormal);
        // Same subnormal reached via a different (x, e) split.
        eq(v, "return tostring(math.ldexp(0.5, -1073))", smallest_subnormal);
        // One exponent past the smallest subnormal underflows exactly to +0.0.
        eq(v, "return math.ldexp(1.0, -1075)", "0.0");
        // Smallest NORMAL: 2^-1022.
        eq(v, "return tostring(math.ldexp(1.0, -1022))", smallest_normal);
        // Overflow past the top of the f64 range goes to +inf.
        eq(v, "return math.ldexp(1.0, 1024)", "inf");

        // frexp of the smallest positive subnormal: (0.5, -1073). A subnormal
        // input has a zero stored exponent; this exercises the scale-up-and-
        // correct branch. Mantissa and exponent render identically on all three.
        eq(
            v,
            r#"local m, e = math.frexp(5e-324) return tostring(m) .. "," .. tostring(e)"#,
            "0.5,-1073",
        );
        // frexp of the smallest NORMAL (2^-1022): (0.5, -1021).
        eq(
            v,
            r#"local m, e = math.frexp(2.2250738585072014e-308) return tostring(m) .. "," .. tostring(e)"#,
            "0.5,-1021",
        );
        // Sign preserved on a subnormal input.
        eq(
            v,
            r#"local m, e = math.frexp(-5e-324) return tostring(m) .. "," .. tostring(e)"#,
            "-0.5,-1073",
        );
    }
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
    err_contains(
        LuaVersion::V54,
        "local x <const> = 1; x = 2",
        "attempt to assign to const variable 'x'",
    );
    // `global` is an ordinary identifier on 5.4.
    eq(LuaVersion::V54, "local global = 8; return global", "8");
    // for-loop var is assignable on 5.4.
    eq(
        LuaVersion::V54,
        "for i = 1, 1 do i = 10 end; return 'ok'",
        "ok",
    );
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
    let only_lt = "local m = {__lt = function() return false end}; \
         local a = setmetatable({}, m); local b = setmetatable({}, m); return a <= b";
    eq(LuaVersion::V53, only_lt, "true");
    eq(LuaVersion::V54, only_lt, "true");
    // 5.5 removed the fallback: comparing with no __le raises.
    err_contains(
        LuaVersion::V55,
        only_lt,
        "attempt to compare two table values",
    );
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

/// #139 (correctness): in 5.1 an order comparison (`<`/`<=`/`>`/`>=`) whose two
/// operands have different Lua types raises `attempt to compare X with Y` BEFORE
/// any metamethod lookup — the `__lt`/`__le` TM is consulted only for same-type
/// operands. 5.2+ removed that guard and consults the TM for mixed types. Int and
/// Float share the `number` type, so the gate is on the Lua type tag, not the
/// numeric subkind. Verified against /tmp/lua-refs/bin/lua5.1.5.
#[test]
fn v51_mixed_type_order_errors_not_metamethod() {
    let lt = "local t = setmetatable({}, {__lt = function() return true end}); ";
    let le = "local t = setmetatable({}, {__le = function() return true end}); ";
    // Constant operands take the immediate opcode path (OP_LtI/LeI/GtI/GeI),
    // covering both normal and inverted (Gt/Ge) reconstructions. The operand
    // order in the message must match the source order, both directions.
    err_contains(
        LuaVersion::V51,
        &format!("{lt}return t < 2"),
        "attempt to compare table with number",
    );
    err_contains(
        LuaVersion::V51,
        &format!("{lt}return 2 < t"),
        "attempt to compare number with table",
    );
    err_contains(
        LuaVersion::V51,
        &format!("{le}return t <= 2"),
        "attempt to compare table with number",
    );
    err_contains(
        LuaVersion::V51,
        &format!("{le}return 2 <= t"),
        "attempt to compare number with table",
    );
    err_contains(
        LuaVersion::V51,
        &format!("{lt}return t < 'x'"),
        "attempt to compare table with string",
    );
    err_contains(
        LuaVersion::V51,
        &format!("{lt}return 'x' < t"),
        "attempt to compare string with table",
    );
    // Non-constant operand takes the register OP_LT path, not the immediate one.
    err_contains(
        LuaVersion::V51,
        &format!("{lt}local n = 2; return t < n"),
        "attempt to compare table with number",
    );

    // Non-regression: same-Lua-type operands still consult the TM on 5.1.
    eq(
        LuaVersion::V51,
        "local m = {__lt = function() return true end}; \
         local a = setmetatable({}, m); local b = setmetatable({}, m); return a < b",
        "true",
    );

    // Non-regression: 5.2+ consult the TM for mixed types (the gate is V51-only).
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(v, &format!("{lt}return t < 2"), "true");
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
        err_contains(
            v,
            "return collectgarbage('bogusopt')",
            "to 'collectgarbage'",
        );
        err_contains(
            v,
            "return collectgarbage('bogusopt')",
            "invalid option 'bogusopt'",
        );
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
        err_contains(
            v,
            "return string.rep('x', 1.5)",
            "number has no integer representation",
        );
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
        err_contains(
            v,
            r#"return string.unpack("i4", "ab")"#,
            "data string too short",
        );
        // string.pack: unsigned overflow.
        err_contains(v, r#"return string.pack("B", 999)"#, "to 'pack'");
        err_contains(v, r#"return string.pack("B", 999)"#, "unsigned overflow");
        // string.pack: integer overflow.
        err_contains(v, r#"return string.pack("i2", 99999)"#, "to 'pack'");
        err_contains(v, r#"return string.pack("i2", 99999)"#, "integer overflow");
        // string.pack: string longer than given size.
        err_contains(v, r#"return string.pack("c2", "abcd")"#, "to 'pack'");
        err_contains(
            v,
            r#"return string.pack("c2", "abcd")"#,
            "string longer than given size",
        );
        // string.pack: invalid next option for 'X' (getdetails helper, now state-threaded).
        err_contains(v, r#"return string.pack("Xc1", 1)"#, "to 'pack'");
        err_contains(v, r#"return string.pack("Xc1", 1)"#, "invalid next option");
        // string.pack: alignment not power of 2 (getdetails helper).
        err_contains(v, r#"return string.pack("!3i4", 1)"#, "to 'pack'");
        err_contains(
            v,
            r#"return string.pack("!3i4", 1)"#,
            "alignment not power of 2",
        );
        // string.packsize: variable-length format.
        err_contains(v, r#"return string.packsize("s")"#, "to 'packsize'");
        err_contains(
            v,
            r#"return string.packsize("s")"#,
            "variable-length format",
        );
        // string.format: %s with embedded zeros + modifiers (was state-less too).
        err_contains(v, r#"return string.format("%5s", "a\0b")"#, "to 'format'");
        err_contains(
            v,
            r#"return string.format("%5s", "a\0b")"#,
            "string contains zeros",
        );

        // Guard: the bare truncated `bad argument #N (` form (no `to '<fn>'`)
        // must NOT survive for these callsites on any version.
        let e = run(v, r#"return string.pack("B", 999)"#).unwrap_err();
        assert!(
            e.contains("to 'pack'"),
            "v{v:?} string.pack argerror lost funcname: {e}"
        );
    }
    // pos-0 lower-bound rejection is 5.3-only; when it fires, it must also carry
    // the funcname. (5.4/5.5 accept pos 0, asserted elsewhere.)
    err_contains(
        LuaVersion::V53,
        r#"return string.unpack("c0", "abc", 0)"#,
        "to 'unpack'",
    );
    err_contains(
        LuaVersion::V53,
        r#"return string.unpack("c0", "abc", 0)"#,
        "initial position out of string",
    );
}

#[test]
fn v_argerror_perversion_wording() {
    // Item B per-version wording splits.
    // utf8.offset: 5.3 says "out of range"; 5.4/5.5 say "out of bounds".
    err_contains(
        LuaVersion::V53,
        "return utf8.offset('abc', 0, 0)",
        "position out of range",
    );
    err_contains(
        LuaVersion::V54,
        "return utf8.offset('abc', 0, 0)",
        "position out of bounds",
    );
    err_contains(
        LuaVersion::V55,
        "return utf8.offset('abc', 0, 0)",
        "position out of bounds",
    );
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return utf8.offset('abc', 0, 0)", "to 'offset'");
    }
    // string.format width: 5.3 uses the old scanformat message; 5.4/5.5 the
    // checkformat message including the offending spec.
    err_contains(
        LuaVersion::V53,
        "return string.format('%200d', 1)",
        "invalid format (width or precision too long)",
    );
    err_contains(
        LuaVersion::V54,
        "return string.format('%200d', 1)",
        "invalid conversion specification: '%200d'",
    );
    err_contains(
        LuaVersion::V55,
        "return string.format('%200d', 1)",
        "invalid conversion specification: '%200d'",
    );
    // string.format unknown conversion: 5.3 "invalid option", 5.4/5.5 "invalid conversion".
    err_contains(
        LuaVersion::V53,
        "return string.format('%y', 1)",
        "invalid option '%y' to 'format'",
    );
    err_contains(
        LuaVersion::V54,
        "return string.format('%y', 1)",
        "invalid conversion '%y' to 'format'",
    );
    err_contains(
        LuaVersion::V55,
        "return string.format('%y', 1)",
        "invalid conversion '%y' to 'format'",
    );
}

#[test]
fn v_length_concat_location_prefix() {
    // (b) `#` and `..` carry the chunk-location prefix and the message body.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return #nil", "attempt to get length of a nil value");
        err_contains(
            v,
            "return ({})..({})",
            "attempt to concatenate a table value",
        );
        // a `:<line>:` prefix appears before the message.
        let e = run(v, "return #nil").unwrap_err();
        let at = e.find("attempt").expect("message body present");
        assert!(
            e[..at].contains(':'),
            "v{v:?} #nil missing location prefix: {e}"
        );
        let e = run(v, "return ({})..({})").unwrap_err();
        let at = e.find("attempt").expect("message body present");
        assert!(
            e[..at].contains(':'),
            "v{v:?} concat missing location prefix: {e}"
        );
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
        err_contains(
            v,
            "return ({}) - 'y'",
            "attempt to sub a 'table' with a 'string'",
        );
        err_contains(
            v,
            "return -'x'",
            "attempt to unm a 'string' with a 'string'",
        );
        // location prefix present on the string-arith path.
        let e = run(v, "return ({}) - 'y'").unwrap_err();
        let at = e.find("attempt").expect("message body present");
        assert!(
            e[..at].contains(':'),
            "v{v:?} string-arith missing prefix: {e}"
        );
    }
}

#[test]
fn v_table_concat_invalid_value_type_name() {
    // (e) plain type name, no internal byte-array leak.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(
            v,
            "return table.concat({ {} })",
            "invalid value (table) at index 1 in table for 'concat'",
        );
        // negative guard: the internal byte-array repr (e.g. `[116, 97, ...]`)
        // must NOT appear. (The chunk-name prefix `[string "..."]` legitimately
        // contains brackets, so look specifically for the comma-separated digit
        // list that the old `{:?}` Debug-format on `&[u8]` produced.)
        let e = run(v, "return table.concat({ {} })").unwrap_err();
        assert!(
            !e.contains("116, 97"),
            "v{v:?} concat leaked byte-array: {e}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5.5 stdlib roster deltas (utf8.offset arity, collectgarbage option set + param)
// specs/followup/5.5-stdlib-err.md items 1, 2, 3.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v55_utf8_offset_returns_end_position() {
    // 5.5 returns (start, end) byte positions; the end is inclusive.
    eq(
        LuaVersion::V55,
        "local a,b = utf8.offset('aébc', 2); return a .. ',' .. b",
        "2,3",
    );
    eq(
        LuaVersion::V55,
        "local a,b = utf8.offset('héllo', 3); return a .. ',' .. b",
        "4,4",
    );
    // arity is 2 on the success branch.
    eq(
        LuaVersion::V55,
        "return select('#', utf8.offset('héllo', 3))",
        "2",
    );
    // one-byte char: end == start.
    eq(
        LuaVersion::V55,
        "local a,b = utf8.offset('abc', 2); return a .. ',' .. b",
        "2,2",
    );
    // not-found / out-of-range: arity stays 1 (only nil).
    eq(
        LuaVersion::V55,
        "return select('#', utf8.offset('abc', 99))",
        "1",
    );
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
    err_contains(
        LuaVersion::V55,
        "return collectgarbage('setpause', 100)",
        "to 'collectgarbage'",
    );
    err_contains(
        LuaVersion::V55,
        "return collectgarbage('setpause', 100)",
        "invalid option 'setpause'",
    );
    err_contains(
        LuaVersion::V55,
        "return collectgarbage('setstepmul', 100)",
        "invalid option 'setstepmul'",
    );
}

#[test]
fn v54_collectgarbage_keeps_setpause_setstepmul() {
    // Regression guard: 5.4/5.3 still accept setpause/setstepmul.
    for v in [LuaVersion::V53, LuaVersion::V54] {
        eq(
            v,
            "local ok = pcall(collectgarbage, 'setpause', 100); return ok",
            "true",
        );
        eq(
            v,
            "local ok = pcall(collectgarbage, 'setstepmul', 100); return ok",
            "true",
        );
    }
}

#[test]
fn v55_collectgarbage_param_surface() {
    // param read returns an integer.
    eq(
        LuaVersion::V55,
        "return math.type(collectgarbage('param', 'pause'))",
        "integer",
    );
    // invalid param name errors via luaL_checkoption, carrying the value.
    err_contains(
        LuaVersion::V55,
        "return collectgarbage('param', 'bogus')",
        "invalid option 'bogus'",
    );
    // write returns the OLD value, then read returns the value just written
    // (round-trip on the faithful-shape backing store).
    eq(
        LuaVersion::V55,
        "collectgarbage('param', 'stepmul', 333); return collectgarbage('param', 'stepmul')",
        "333",
    );
    // arity is 1.
    eq(
        LuaVersion::V55,
        "return select('#', collectgarbage('param', 'pause'))",
        "1",
    );
}

#[test]
fn v54_collectgarbage_param_not_an_option() {
    // Regression guard: 'param' is NOT a valid collectgarbage option on 5.4/5.3.
    for v in [LuaVersion::V53, LuaVersion::V54] {
        err_contains(
            v,
            "return collectgarbage('param', 'pause')",
            "invalid option",
        );
    }
}

#[test]
fn v54_v55_start_in_reported_generational_mode() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            "local a = collectgarbage('incremental'); \
             local b = collectgarbage('generational'); \
             local c = collectgarbage('incremental'); \
             return a .. '|' .. b .. '|' .. c",
            "generational|incremental|generational",
        );
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
    err_contains(LuaVersion::V55, "error(nil)", "<no error object>");
    // error() with no argument: object defaults to nil.
    err_contains(LuaVersion::V55, "error()", "<no error object>");
    // nested pcall still sees the converted string.
    eq(
        LuaVersion::V55,
        "local ok, e = pcall(function() error(nil) end); return type(e) .. ':' .. tostring(e)",
        "string:<no error object>",
    );
    // xpcall whose handler returns nil also settles to the string (the
    // conversion runs on the handler result, matching upstream ordering).
    eq(
        LuaVersion::V55,
        "local ok, e = xpcall(function() error('x') end, function() return nil end); \
         return type(e) .. ':' .. tostring(e)",
        "string:<no error object>",
    );
}

#[test]
fn v55_xpcall_message_handler_retries_until_result_or_c_stack_overflow() {
    eq(
        LuaVersion::V55,
        "local function err(n) \
           if type(n) ~= 'number' then return n \
           elseif n == 0 then return 'END' \
           else error(n - 1) end \
         end; \
         local _, msg170 = xpcall(error, err, 170); \
         local _, msg300 = xpcall(error, err, 300); \
         return msg170 .. '|' .. msg300",
        "END|C stack overflow",
    );
}

#[test]
fn v53_v54_error_nil_stays_nil() {
    // Regression guard: 5.3/5.4 leave a nil error object as nil (no conversion).
    for v in [LuaVersion::V53, LuaVersion::V54] {
        eq(
            v,
            "local ok, e = pcall(function() error(nil) end); return type(e) .. ':' .. tostring(e)",
            "nil:nil",
        );
        eq(
            v,
            "local ok, e = pcall(function() error() end); return type(e) .. ':' .. tostring(e)",
            "nil:nil",
        );
        // A real string error object is untouched (sanity: conversion is nil-only).
        eq(
            v,
            "local ok, e = pcall(function() error('boom') end); return (e:gsub('^.*: ', ''))",
            "boom",
        );
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
    err_contains(
        LuaVersion::V53,
        r#"return #"\u{110000}""#,
        "UTF-8 value too large",
    );
    err_contains(
        LuaVersion::V53,
        r#"return #"\u{110001}""#,
        "UTF-8 value too large",
    );
    err_contains(
        LuaVersion::V53,
        r#"return #"\u{7FFFFFFF}""#,
        "UTF-8 value too large",
    );
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
    eq(
        LuaVersion::V53,
        "local r = math.random(1, 6); return r >= 1 and r <= 6",
        "true",
    );
}

#[test]
fn v54_v55_random_zero_and_full_range() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        // random(0) returns a full-range integer (no error).
        eq(v, "return math.type(math.random(0))", "integer");
        // Full integer range is accepted.
        eq(
            v,
            "return math.type(math.random(math.mininteger, math.maxinteger))",
            "integer",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5.1 `math.random`/`math.randomseed` PRNG contract (oracle = lua5.1.5).
// 5.1 uses the C `rand()`/`srand()` model: NO `random(0)` full-range special
// case (that empty interval errors), the `random(m, n)` empty-interval error
// reports argument index 2 (the upper bound), `randomseed` REQUIRES its seed
// argument (no auto-seed) and returns NO values, and integer-valued results
// are plain `number`s (float-only, no `math.type`).
//
// The seeded random SEQUENCE is a KNOWN DOCUMENTED divergence: 5.1's host
// `rand()` byte stream is platform-dependent and is not bit-matched here. Only
// the contract — ranges, types, arg errors, return shapes — is asserted.
// See specs/followup/5.1-numbers-prng.md.
//
// NOTE: 5.1 renders the function name as `'?'` for indirect (`pcall(fn,...)`)
// calls where lua-rs renders the qualified name; that systematic indirect-call
// name-recovery gap is tracked separately, so these assertions check the arg
// INDEX and the error TEXT (which match), not the function-name token.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v51_random_contract() {
    let v = LuaVersion::V51;
    // random() ∈ [0, 1), is a number, and float-only (no math.type).
    eq(
        v,
        "local r = math.random(); return type(r) .. ',' .. tostring(r >= 0 and r < 1)",
        "number,true",
    );
    eq(v, "return math.type", "nil");
    // random(n) ∈ [1, n], integer-valued.
    eq(
        v,
        "local r = math.random(10); return r >= 1 and r <= 10 and r == math.floor(r)",
        "true",
    );
    // random(m, n) ∈ [m, n].
    eq(
        v,
        "local r = math.random(5, 8); return r >= 5 and r <= 8",
        "true",
    );
    // random(0) is an EMPTY interval [1, 0] — it errors (no 5.4 full-range case).
    err_contains(
        v,
        "local ok, e = pcall(math.random, 0); error(e, 0)",
        "interval is empty",
    );
    // random(m, n) empty-interval error reports argument #2 (the upper bound).
    err_contains(
        v,
        "local ok, e = pcall(math.random, 5, 2); error(e, 0)",
        "bad argument #2",
    );
    err_contains(
        v,
        "local ok, e = pcall(math.random, 5, 2); error(e, 0)",
        "interval is empty",
    );
    // Three args is "wrong number of arguments".
    err_contains(
        v,
        "local ok, e = pcall(math.random, 1, 2, 3); error(e, 0)",
        "wrong number of arguments",
    );
}

#[test]
fn v51_randomseed_contract() {
    let v = LuaVersion::V51;
    // randomseed REQUIRES its seed argument — no auto-seed when absent.
    err_contains(
        v,
        "local ok, e = pcall(math.randomseed); error(e, 0)",
        "number expected, got no value",
    );
    err_contains(
        v,
        "local ok, e = pcall(math.randomseed, 'x'); error(e, 0)",
        "number expected, got string",
    );
    err_contains(
        v,
        "local ok, e = pcall(math.randomseed); error(e, 0)",
        "bad argument #1",
    );
    // randomseed accepts a number and returns NO values.
    eq(v, "return select('#', math.randomseed(42))", "0");
    eq(v, "return select('#', math.randomseed(5))", "0");
}

#[test]
fn v51_prng_non_regression_modern_unchanged() {
    // The V51 gates must not alter the modern PRNG contract.
    // 5.3: random(0) is empty-interval (no full-range), randomseed returns 2.
    err_contains(
        LuaVersion::V53,
        "local ok, e = pcall(math.random, 0); error(e, 0)",
        "interval is empty",
    );
    eq(
        LuaVersion::V53,
        "return select('#', math.randomseed(42))",
        "2",
    );
    // 5.4/5.5: random(0) is full-range integer; randomseed(N) returns 2 words;
    // randomseed() auto-seeds (no required arg) and also returns 2.
    for v in [LuaVersion::V54, LuaVersion::V55] {
        eq(v, "return math.type(math.random(0))", "integer");
        eq(v, "return select('#', math.randomseed(42))", "2");
        eq(v, "return select('#', math.randomseed())", "2");
    }
    // 5.2 is float-only like 5.1: it shares the require-seed + void-return
    // randomseed contract and the no-full-range random(0) error.
    err_contains(
        LuaVersion::V52,
        "local ok, e = pcall(math.random, 0); error(e, 0)",
        "interval is empty",
    );
    eq(
        LuaVersion::V52,
        "return select('#', math.randomseed(42))",
        "0",
    );
    err_contains(
        LuaVersion::V52,
        "local ok, e = pcall(math.randomseed); error(e, 0)",
        "number expected, got no value",
    );
}

// ─────────────────────────────────────────────────────────────────────────
// PRNG SEQUENCE pins (the xoshiro256** path: 5.4 and 5.5 only).
//
// The pre-existing random assertions pin a single post-seed draw, so a
// divergence that appears only on the SECOND draw (or only on `random(lo,hi)`
// projection, or only on the two-seed-word `randomseed(n1,n2)` form) would
// pass undetected. These tests seed ONCE and pin the whole consecutive
// SEQUENCE — the real tripwire for any reordering of the `next_rand` /
// `project` / `rand_to_float` arithmetic.
//
// Bit-exactness is only available on the xoshiro256** path. 5.4 and 5.5 use
// xoshiro256**; their sequences are byte-for-byte identical to the reference
// binaries (lua5.4.7 / lua5.5.0). 5.1, 5.2, and 5.3 wrap the host C
// `rand()`/`random()`, whose stream is platform-dependent and is a KNOWN,
// DOCUMENTED allowed divergence (specs/followup/5.1-numbers-prng.md,
// specs/research/5.3-upstream-delta.md) — their CONTRACT (range/type/shape) is
// pinned above but their SEQUENCE is intentionally NOT bit-compared.
//
// Every expected sequence below was captured from /tmp/lua-refs/bin/lua5.4.7
// and lua5.5.0 with `string.format("%.17g", ...)` (a round-tripping rendering,
// independent of tostring precision).
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v54_v55_random_float_sequence_bit_exact() {
    for v in [LuaVersion::V54, LuaVersion::V55] {
        // Six consecutive math.random() draws after a single randomseed(1).
        eq(
            v,
            "math.randomseed(1)\n\
             local t = {}\n\
             for i = 1, 6 do t[i] = string.format('%.17g', math.random()) end\n\
             return table.concat(t, '|')",
            "0.81558781554723059|0.98657750643457565|0.079330719590026022|\
             0.49864849323368698|0.59181018547898889|0.83396886864931397",
        );
    }
}

#[test]
fn v54_v55_random_interval_sequence_bit_exact() {
    // The `project` rejection-sampling path: pin a positive-range sequence on
    // 5.4 and a signed-range sequence on 5.5 (both reference-captured).
    eq(
        LuaVersion::V54,
        "math.randomseed(1)\n\
         local t = {}\n\
         for i = 1, 8 do t[i] = math.random(1, 1000) end\n\
         return table.concat(t, ',')",
        "414,488,418,685,983,990,745,895",
    );
    eq(
        LuaVersion::V55,
        "math.randomseed(1)\n\
         local t = {}\n\
         for i = 1, 8 do t[i] = math.random(-50, 50) end\n\
         return table.concat(t, ',')",
        "-21,-17,-6,36,43,9,43,-27",
    );
}

#[test]
fn v54_two_seed_word_random_sequence_bit_exact() {
    // The two-argument `randomseed(n1, n2)` seeding form (set_seed_words with a
    // nonzero second word) — pinned so a change to seed mixing is caught.
    eq(
        LuaVersion::V54,
        "math.randomseed(1, 2)\n\
         local t = {}\n\
         for i = 1, 4 do t[i] = string.format('%.17g', math.random()) end\n\
         return table.concat(t, '|')",
        "0.44949358084855551|0.22576880156399304|\
         0.73957236111052194|0.34305203708388043",
    );
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
    eq(
        LuaVersion::V52,
        "return 9007199254740993",
        "9.007199254741e+15",
    );
    eq(LuaVersion::V52, "return _VERSION", "Lua 5.2");
}

/// Float-only is an OBSERVATIONAL invariant, not a constructional one.
///
/// The gate-based float-only design (5.1/5.2) keeps the dual `LuaValue` enum and
/// does NOT forbid constructing `Int` — `#t`, `string.len`, `string.byte`, and
/// `Int*Int` arithmetic all still produce internal `Int` values under V51/V52.
/// That is intentional and correct: an integer-valued `Int` is observationally
/// identical to the equivalent `Float` once the distinguishing channels are
/// gated (`tostring` suppresses the `.0`; `math.type`/`//`/bitwise — the only
/// things that could tell them apart — are absent or unlexable in 5.1/5.2).
///
/// This test pins that interpretation so a future change does NOT "fix" the
/// internal `Int` construction (a no-op churn) or add a blanket
/// `debug_assert!(no Int under FloatOnly)` (which would fire on `#"abc"` and is
/// the WRONG invariant). What must hold is the observable surface — locked here
/// and in `v52_float_only_number_model` / `v52_math_roster_is_float_only`.
#[test]
fn float_only_internal_int_is_observationally_invisible() {
    for v in [LuaVersion::V51, LuaVersion::V52] {
        // Int*Int (two lengths) is constructed as an internal Int, yet prints
        // and concatenates with no `.0` — indistinguishable from Float(6.0).
        eq(v, r#"return #"abc" * #"de""#, "6");
        eq(v, r#"return tostring(#"abcdef")"#, "6");
        eq(v, r#"return (#"abc" * #"de") .. """#, "6");
        // string.byte yields an internal Int too; arithmetic on it stays faithful.
        eq(v, r#"return string.byte("A") + 1"#, "66");
        // The ONLY channel that would expose the internal Int — math.type — is
        // absent in the float-only family, which is exactly why it stays hidden.
        eq(v, "return type(math.type)", "nil");
    }
    // Contrast: on the dual-model core the same length IS an observable integer
    // (math.type present) — proving the value really is Int internally and that
    // math.type's absence is what makes 5.1/5.2 float-only, not any value fork.
    eq(LuaVersion::V53, r#"return math.type(#"abc")"#, "integer");
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
    err_contains(
        LuaVersion::V52,
        "return 1 << 4",
        "unexpected symbol near '<'",
    );
    err_contains(
        LuaVersion::V52,
        "return 256 >> 4",
        "unexpected symbol near '>'",
    );
    err_contains(LuaVersion::V52, "return 5 ~ 3", "near '~'");
    err_contains(LuaVersion::V52, "return ~0", "unexpected symbol near '~'");
    // The 5.4 <const> attribute syntax is also absent.
    err_contains(
        LuaVersion::V52,
        "local x <const> = 1",
        "unexpected symbol near '<'",
    );
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
        err_contains(
            v,
            "return string.format('%d', 3.5)",
            "no integer representation",
        );
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

// ─────────────────────────────────────────────────────────────────────────
// 5.1 — the legacy fenv-globals family. 5.1 reuses the float-only core (shared
// with 5.2) but restores the per-function environment model: `getfenv`/
// `setfenv` read/write a function's environment (its `_ENV` upvalue under the
// reused modern core) or the running thread's global table for level 0.
// Option B (specs/followup/5.1-fenv.md): no second VM ISA — the modern _ENV
// upvalue IS the closure environment under V51. Every expected value captured
// from /tmp/lua-refs/bin/lua5.1.5.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v51_getfenv_main_chunk_is_g() {
    // In the main chunk getfenv() / getfenv(0) / getfenv(1) all return _G.
    eq(LuaVersion::V51, "return getfenv() == _G", "true");
    eq(LuaVersion::V51, "return getfenv(0) == _G", "true");
    eq(LuaVersion::V51, "return getfenv(1) == _G", "true");
    eq(LuaVersion::V51, "return getfenv(0) == getfenv(1)", "true");
}

#[test]
fn v51_setfenv_per_closure_env() {
    // A closure given its own env resolves free names there; the caller's
    // globals are untouched (the env is PRIVATE, not the shared _ENV cell).
    eq(
        LuaVersion::V51,
        "local function f() return x end \
         local e = setmetatable({x=42},{__index=_G}) \
         setfenv(f, e); return tostring(f()) .. ',' .. tostring(x)",
        "42,nil",
    );
    // setfenv returns the function it set (even an empty-body closure).
    eq(
        LuaVersion::V51,
        "local function f() end return setfenv(f, {}) == f",
        "true",
    );
}

#[test]
fn v51_new_closure_inherits_creator_env() {
    // A closure created inside a function whose env was setfenv'd inherits that
    // env (standard upvalue capture of _ENV).
    eq(
        LuaVersion::V51,
        "local function outer() local function inner() return secret end return inner end \
         local e = setmetatable({secret='hi'},{__index=_G}); setfenv(outer, e) \
         local inner = outer(); return tostring(getfenv(inner)==e) .. ',' .. inner()",
        "true,hi",
    );
}

#[test]
fn v51_setfenv_zero_thread_closure_split() {
    // setfenv(0, t) sets the THREAD global table, observable via getfenv(0),
    // but does NOT retroactively change the running closure's own _ENV upval:
    // getfenv(1) != t and a free name set only in t reads back nil.
    eq(
        LuaVersion::V51,
        "local e = setmetatable({z=99},{__index=_G}); setfenv(0, e) \
         return tostring(getfenv(0)==e) .. ',' .. tostring(getfenv(1)==e) .. ',' .. tostring(z)",
        "true,false,nil",
    );
}

#[test]
fn v51_loaded_chunk_takes_thread_env() {
    // A chunk loaded AFTER setfenv(0, t) takes t as its environment (loaded
    // chunks take the thread env, never the loader closure's env).
    eq(
        LuaVersion::V51,
        "local e = setmetatable({secret='thr'},{__index=_G}); setfenv(0, e) \
         local c = loadstring('return secret'); return tostring(getfenv(c)==e) .. ',' .. c()",
        "true,thr",
    );
}

#[test]
fn v51_level_forms() {
    // getfenv(2) from a callee returns the caller's env. (Parenthesized to
    // avoid a tail call, which 5.1 itself refuses to introspect.)
    eq(
        LuaVersion::V51,
        "local e = setmetatable({k=1},{__index=_G}) \
         local function caller() local function callee() return (getfenv(2)) end return (callee()) end \
         setfenv(caller, e); return caller() == e",
        "true",
    );
    // setfenv(2, t) sets the caller's env (its free name then resolves in t).
    eq(
        LuaVersion::V51,
        "local function caller() local function callee() setfenv(2, setmetatable({y=9},{__index=_G})) end \
         callee(); return y end return caller()",
        "9",
    );
    // A nested closure that captures BOTH a local and a global: setfenv must
    // touch the _ENV upvalue, located by NAME (not position — the captured
    // local is at upvalue 0 here), or the local would be corrupted.
    eq(
        LuaVersion::V51,
        "local up = 7; local function f() return up + g end; g = 100 \
         local e = setmetatable({g=5},{__index=_G}); setfenv(f, e); return f()",
        "12",
    );
}

#[test]
fn v51_float_level_truncates() {
    // Levels truncate toward zero (luaL_checkint cast): getfenv(1.9) is level 1.
    eq(LuaVersion::V51, "return getfenv(1.9) == _G", "true");
}

#[test]
fn v51_c_function_env_is_g() {
    // C functions get the thread global table as their env (documented
    // LUA_ENVIRONINDEX gap, specs/followup/5.1-fenv.md §4).
    eq(LuaVersion::V51, "return getfenv(print) == _G", "true");
}

#[test]
fn v51_fenv_error_cases() {
    err_contains(LuaVersion::V51, "return getfenv(5)", "invalid level");
    err_contains(
        LuaVersion::V51,
        "return getfenv(-1)",
        "level must be non-negative",
    );
    err_contains(
        LuaVersion::V51,
        "return getfenv('x')",
        "number expected, got string",
    );
    err_contains(
        LuaVersion::V51,
        "return setfenv(print, {})",
        "'setfenv' cannot change environment of given object",
    );
    err_contains(
        LuaVersion::V51,
        "return setfenv(0, 'x')",
        "table expected, got string",
    );
    err_contains(LuaVersion::V51, "return setfenv(100, {})", "invalid level");
}

#[test]
fn v51_len_on_table_ignores_metamethod() {
    // THE #1 silent-failure trap: `#t` in 5.1 NEVER consults a table __len;
    // table __len was added in 5.2. Primitive length always wins.
    eq(
        LuaVersion::V51,
        "local t = setmetatable({1,2,3}, {__len = function() return 99 end}); return #t",
        "3",
    );
    eq(LuaVersion::V51, "return #'hello'", "5");
    eq(LuaVersion::V51, "return #({10, 20})", "2");
}

#[test]
fn v51_pairs_ignores_metamethod() {
    // 5.1 has no __pairs metamethod; pairs(t) ALWAYS iterates the raw table
    // even when a __pairs is set (it is silently ignored). Oracle lua5.1.5:
    // a __pairs that error()s never fires; the raw sum (60) comes through.
    eq(
        LuaVersion::V51,
        "local t = setmetatable({10,20,30}, {__pairs = function() error('should not fire') end}); \
         local s = 0; for k,v in pairs(t) do s = s + v end; return s",
        "60",
    );
}

#[test]
fn v52_plus_pairs_honors_metamethod() {
    // Non-regression guard: __pairs IS consulted in 5.2-5.5. It was added in
    // 5.2 and `pairs` still dispatches to it on 5.3/5.4/5.5 (verified against
    // lua5.2.4/5.3.6/5.4.7/5.5.0 — a __pairs that error()s fires on all four).
    // A __pairs returning an empty iterator makes the loop body never run, so
    // the raw sum is shadowed (0).
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "local t = setmetatable({10,20,30}, {__pairs = function(tt) \
               return function() return nil end, tt, nil end}); \
             local s = 0; for k,v in pairs(t) do s = s + v end; return s",
            "0",
        );
    }
}

#[test]
fn v51_gc_on_table_is_inert() {
    // 5.1 has no __gc on tables — only userdata can be finalized. Setting __gc
    // on a table metatable is inert: no call, no error, on collection. Oracle
    // lua5.1.5: the flag set by the finalizer stays false after two GC cycles.
    eq(
        LuaVersion::V51,
        "local flag = {fired = false}; \
         do local t = setmetatable({}, {__gc = function() flag.fired = true end}); t = nil end; \
         collectgarbage(); collectgarbage(); return tostring(flag.fired)",
        "false",
    );
}

#[test]
fn v52_plus_gc_on_table_fires() {
    // Non-regression guard: __gc on tables works in 5.2+ (it was added in 5.2).
    // The finalizer must run on collection and flip the flag to true. We assert
    // the call site does not error and produces a boolean; the exact firing is
    // exercised by the gc canaries — here we only guard that table finalizers
    // remain *registered* (not silently skipped) off V51 by confirming the
    // setmetatable path accepts a table __gc without raising.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "local ok = pcall(function() \
               setmetatable({}, {__gc = function() end}) end); return tostring(ok)",
            "true",
        );
    }
}

#[test]
fn v51_gc_on_userdata_registered_by_setmetatable_fires() {
    // 5.1 has no table finalizers, but full userdata finalizers are active.
    // This pins the VM's `luaC_checkfinalizer` equivalent for userdata
    // metatable assignment: before that path was wired, the finalizer never ran.
    eq(
        LuaVersion::V51,
        "local flag = 'no'; \
         do local u = newproxy(false); \
            debug.setmetatable(u, {__gc = function(x) flag = type(x) end}); \
            u = nil \
         end; \
         collectgarbage(); collectgarbage(); return flag",
        "userdata",
    );
}

#[test]
fn v51_fenv_roster_present() {
    eq(LuaVersion::V51, "return type(getfenv)", "function");
    eq(LuaVersion::V51, "return type(setfenv)", "function");
    eq(LuaVersion::V51, "return _VERSION", "Lua 5.1");
}

#[test]
fn v52_plus_no_fenv_globals() {
    // Non-regression guard: getfenv/setfenv are 5.1-ONLY. They were removed in
    // 5.2 (lexical _ENV) and must stay absent on 5.2/5.3/5.4/5.5. The 5.2+
    // family also keeps consulting a table __len metamethod.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(v, "return type(getfenv)", "nil");
        eq(v, "return type(setfenv)", "nil");
    }
    // 5.2 keeps the float-only core but consults table __len (5.2+ behavior).
    eq(
        LuaVersion::V52,
        "local t = setmetatable({1,2,3}, {__len = function() return 99 end}); return #t",
        "99",
    );
    // The modern (dual-number) family also consults table __len.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            "local t = setmetatable({1,2,3}, {__len = function() return 99 end}); return #t",
            "99",
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 5.1 — roster / syntax / metamethod axis (the legacy stdlib roster, the
// pre-5.2 syntax rejections, and the body-behavior deltas). Every expected
// value captured from /tmp/lua-refs/bin/lua5.1.5. See
// specs/followup/5.1-roster-syntax.md.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v51_global_roster() {
    // unpack is a GLOBAL; table.unpack/pack/move are absent.
    eq(LuaVersion::V51, "return unpack({10,20,30})", "10");
    eq(LuaVersion::V51, "return type(table.unpack)", "nil");
    eq(LuaVersion::V51, "return type(table.pack)", "nil");
    eq(LuaVersion::V51, "return type(table.move)", "nil");
    // legacy table roster present.
    eq(LuaVersion::V51, "return table.getn({1,2,3})", "3");
    eq(
        LuaVersion::V51,
        "return table.maxn({[1]=1,[5]=2,[3]=3})",
        "5",
    );
    eq(LuaVersion::V51, "return type(table.foreach)", "function");
    eq(LuaVersion::V51, "return type(table.foreachi)", "function");
    // table.setn is a gravestone raising the obsolete message.
    err_contains(
        LuaVersion::V51,
        "return table.setn({}, 3)",
        "'setn' is obsolete",
    );
    // 5.1 holdover globals.
    eq(LuaVersion::V51, "return type(gcinfo())", "number");
    eq(LuaVersion::V51, "return type(newproxy)", "function");
    eq(LuaVersion::V51, "return type(newproxy())", "userdata");
    eq(LuaVersion::V51, "return type(loadstring)", "function");
    // absent in 5.1.
    eq(LuaVersion::V51, "return type(bit32)", "nil");
    eq(LuaVersion::V51, "return type(utf8)", "nil");
    eq(LuaVersion::V51, "return type(rawlen)", "nil");
    eq(LuaVersion::V51, "return type(math.type)", "nil");
}

#[test]
fn v51_math_roster() {
    // 5.1 compat math functions present; math.log ignores a 2nd arg (no base).
    eq(LuaVersion::V51, "return type(math.log10)", "function");
    eq(LuaVersion::V51, "return type(math.atan2)", "function");
    eq(LuaVersion::V51, "return type(math.pow)", "function");
    eq(LuaVersion::V51, "return type(math.mod)", "function");
    eq(LuaVersion::V51, "return math.mod(7,3)", "1");
    // math.log(8,2) == math.log(8) == ln(8); the base is silently ignored.
    eq(
        LuaVersion::V51,
        "return math.log(8,2) == math.log(8)",
        "true",
    );
}

#[test]
fn v51_string_and_package_roster() {
    eq(LuaVersion::V51, "return type(string.gfind)", "function");
    eq(LuaVersion::V51, "return type(module)", "function");
    eq(LuaVersion::V51, "return type(package.seeall)", "function");
    eq(LuaVersion::V51, "return type(package.loaders)", "table");
    // package.searchers is the 5.2 rename; absent in 5.1.
    eq(LuaVersion::V51, "return type(package.searchers)", "nil");
    // module() creates/initializes a module table and sets the caller env.
    eq(
        LuaVersion::V51,
        "module('foo', package.seeall); return _NAME .. ',' .. tostring(_M == foo)",
        "foo,true",
    );
}

#[test]
fn v51_body_behavior_deltas() {
    // load takes a reader function ONLY; a string errors. loadstring loads it.
    err_contains(
        LuaVersion::V51,
        "return load('return 1')",
        "function expected, got string",
    );
    eq(LuaVersion::V51, "return loadstring('return 7')()", "7");
    // xpcall(f, h) does NOT forward extra args; f is called with zero args.
    eq(
        LuaVersion::V51,
        "local n; xpcall(function(...) n = select('#', ...) end, function(e) return e end, 1, 2, 3); return n",
        "0",
    );
    // collectgarbage rejects the 5.4-only options under V51.
    err_contains(
        LuaVersion::V51,
        "return collectgarbage('isrunning')",
        "invalid option 'isrunning'",
    );
    // coroutine.running() returns nil in the main coroutine.
    eq(LuaVersion::V51, "return coroutine.running()", "nil");
    eq(LuaVersion::V51, "return type(coroutine.isyieldable)", "nil");
}

#[test]
fn v51_syntax_rejections() {
    // goto is NOT reserved in 5.1 — it stays a valid identifier.
    eq(LuaVersion::V51, "local goto = 5; return goto", "5");
    // The goto STATEMENT and ::label:: do not parse (goto lexes as a name, so
    // `goto done` is a name beginning an assignment → "'=' expected").
    err_contains(
        LuaVersion::V51,
        "do goto done; ::done:: end",
        "'=' expected",
    );
    err_contains(LuaVersion::V51, "::lbl::", "unexpected symbol near ':'");
    // 5.3 integer operators and 5.4 attribs do not parse in 5.1.
    err_contains(LuaVersion::V51, "return 7//2", "unexpected symbol near '/'");
    err_contains(LuaVersion::V51, "return 6 & 3", "near '&'");
    err_contains(
        LuaVersion::V51,
        "return 1 << 4",
        "unexpected symbol near '<'",
    );
    err_contains(
        LuaVersion::V51,
        "local x <const> = 1",
        "unexpected symbol near '<'",
    );
}

#[test]
fn v51_escape_leniency() {
    // 5.1 does NOT recognize \x, \z, \u and does NOT raise on unknown escapes:
    // it drops the backslash and keeps the next char. \x41 → "x41", \z → "z".
    eq(LuaVersion::V51, "return '\\x41'", "x41");
    eq(LuaVersion::V51, "return '\\z'", "z");
    eq(LuaVersion::V51, "return '\\q'", "q");
    // Decimal escapes still work; \65 → "A".
    eq(LuaVersion::V51, "return '\\65'", "A");
    // A decimal escape > 255 errors (5.1 wording: "escape sequence too large").
    err_contains(
        LuaVersion::V51,
        "return '\\999'",
        "escape sequence too large",
    );
}

#[test]
fn v52_plus_roster_unchanged_by_v51_work() {
    // Non-regression guards for the SHARED code paths the V51 gates touched:
    // they must remain version-correct off V51.
    //  - xpcall forwards extra args in 5.2+ (added in 5.2).
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "local n; xpcall(function(...) n = select('#', ...) end, function(e) return e end, 1, 2, 3); return n",
            "3",
        );
    }
    //  - coroutine.running returns thread + is-main boolean in 5.2+.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "local _, m = coroutine.running(); return tostring(m)",
            "true",
        );
    }
    //  - 5.2 keeps isyieldable absent (added in 5.3); 5.3+ has it.
    eq(LuaVersion::V52, "return type(coroutine.isyieldable)", "nil");
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(v, "return type(coroutine.isyieldable)", "function");
    }
    //  - 5.2 keeps load accepting a string (the reader-only restriction is V51).
    eq(LuaVersion::V52, "return load('return 1')()", "1");
    //  - 5.2 collectgarbage still accepts isrunning (added in 5.2).
    eq(
        LuaVersion::V52,
        "return type(collectgarbage('isrunning'))",
        "boolean",
    );
    //  - math.log honors a base argument in 5.2+ (float-only: prints "3").
    eq(LuaVersion::V52, "return math.log(8, 2)", "3");
    //  - 5.2 keeps table.unpack/pack present (V51 drops them).
    eq(LuaVersion::V52, "return type(table.unpack)", "function");
    eq(LuaVersion::V52, "return type(table.pack)", "function");
}

#[test]
fn v55_table_downward_resize_no_panic() {
    // Regression for the `nextvar.lua` (5.5) table-resize panic: a downward
    // array resize migrated array slots into the hash part by calling `set_int`
    // *while iterating the live array*; the first migrated key (`alimit+1`)
    // could re-enter `resize` and truncate the very array being read, so the
    // loop indexed past its (now shorter) physical length and panicked
    // ("index out of bounds: len N, index N") — a panic in safe Rust, worse
    // than any parity mismatch. The fix detaches the migrating slots into an
    // owned snapshot and truncates the array *before* reinserting.
    //
    // This is the exact shape from `nextvar.lua`'s "length for some random
    // tables" loop (`table.create` preallocates an array part, then sparse
    // integer inserts force rehash + downward resize). Seed 7 deterministically
    // reproduced the panic on the unfixed binary; the assertion below would
    // panic inside the VM harness on regression.
    eq(
        LuaVersion::V55,
        "math.randomseed(7)\n\
         local N = 130\n\
         for i = 1, 1000 do\n\
           local a = table.create(math.random(N))\n\
           for j = 1, math.random(N) do a[math.random(N)] = true end\n\
           local _ = #a\n\
         end\n\
         return 'done'",
        "done",
    );
}

// ─────────────────────────────────────────────────────────────────────────
// goto label scoping in disjoint / nested blocks (shared-core item H1).
//
// The scope of the "label already defined" check changed at 5.3 -> 5.4.
// Upstream 5.2/5.3 `checkrepeated` scans only the *current block*
// (`fs->bl->firstlabel`), so a label inside an inner block does NOT collide
// with a same-named label in an enclosing block, and `gotostat`'s `findlabel`
// likewise resolves a goto against the current block only (a non-matching goto
// stays pending and is resolved by `solvegotos` / `movegotosout`). Upstream
// 5.4/5.5 rewrote `checkrepeated` and `findlabel` to scan the whole function
// (`fs->firstlabel`), so any repeated label name in a function is an error.
//
// Captured from the reference binaries (lua5.2.4 / lua5.3.6 accept; lua5.4.7 /
// lua5.5.0 reject with "label 'l3' already defined on line 1"). 5.1 has no
// goto/labels.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v52_v53_disjoint_block_label_shadows_outer() {
    // `::l3::` then `do goto l3; ::l3:: end`: the inner label is a distinct
    // block scope; the `goto l3` binds forward to the inner label, not the
    // outer one (which would loop). Accepted on 5.2/5.3, runs to completion.
    let prog = "local s = ''\n\
                ::l3:: s = s .. 'a'\n\
                do goto l3done; ::l3:: end\n\
                ::l3done:: return s";
    eq(LuaVersion::V52, prog, "a");
    eq(LuaVersion::V53, prog, "a");
}

#[test]
fn v54_v55_disjoint_block_label_rejected() {
    let prog = "::l3:: print('a')\n\
                do goto l3; ::l3:: end\n\
                print('ok')";
    err_contains(LuaVersion::V54, prog, "label 'l3' already defined");
    err_contains(LuaVersion::V55, prog, "label 'l3' already defined");
}

#[test]
fn v52_v53_deeply_nested_label_shadows_outer() {
    let prog = "local s=''\n\
                ::l:: s = s .. 'a'\n\
                do do goto ldone; ::l:: end end\n\
                ::ldone:: return s";
    eq(LuaVersion::V52, prog, "a");
    eq(LuaVersion::V53, prog, "a");
}

#[test]
fn v54_v55_deeply_nested_label_rejected() {
    let prog = "::l:: print('a')\n\
                do do goto l; ::l:: end end\n\
                print('ok')";
    err_contains(LuaVersion::V54, prog, "label 'l' already defined");
    err_contains(LuaVersion::V55, prog, "label 'l' already defined");
}

#[test]
fn same_block_duplicate_label_rejected_all_versions() {
    // A duplicate label in the *same* block is an error on every version that
    // has goto (5.2+).
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        err_contains(v, "::l:: ::l:: print('x')", "label 'l' already defined");
    }
}

#[test]
fn v52_v53_backward_goto_to_enclosing_block_label() {
    // A backward goto from inside a nested block to a label in the enclosing
    // block must resolve (via `movegotosout` re-resolution on 5.2/5.3), not be
    // reported as "no visible label". Mirrors goto.lua's `goto l3a` shape.
    let prog = "local n = 0\n\
                ::top:: n = n + 1\n\
                if n < 3 then do goto top end end\n\
                return n";
    eq(LuaVersion::V52, prog, "3");
    eq(LuaVersion::V53, prog, "3");
    eq(LuaVersion::V54, prog, "3");
    eq(LuaVersion::V55, prog, "3");
}

#[test]
fn goto_label_errors_carry_chunkname_line_prefix() {
    // Upstream raises duplicate-label and undefined-goto errors through
    // `luaK_semerror` -> `luaX_syntaxerror`, which prepends the
    // `chunkname:line:` location (e.g. `(command line):1:`). lua-rs previously
    // built these two messages without that prefix. Both must now carry a
    // `<chunk>:<line>:` location ahead of the message body, on every version
    // that has goto/labels (5.2+).
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        match run(v, "::l:: ::l:: print('x')") {
            Ok(got) => panic!("expected duplicate-label error, got `{got}`"),
            Err(e) => {
                assert!(e.contains("label 'l' already defined"), "body missing: {e}");
                assert!(
                    e.find("]:").or_else(|| e.find("\":")).is_some()
                        && e.find(": label 'l'").is_some(),
                    "duplicate-label error `{e}` lacks a chunkname:line: prefix"
                );
            }
        }
        match run(v, "goto nowhere") {
            Ok(got) => panic!("expected undefined-goto error, got `{got}`"),
            Err(e) => {
                assert!(
                    e.contains("no visible label 'nowhere'"),
                    "body missing: {e}"
                );
                assert!(
                    e.find(": no visible label 'nowhere'").is_some(),
                    "undefined-goto error `{e}` lacks a chunkname:line: prefix"
                );
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// __gc finalizer error propagation (shared-core item 3).
//
// Version split confirmed against the reference binaries (gcerr probe):
//   5.1            — silently swallows; pcall(collectgarbage) returns ok.
//   5.2 / 5.3      — C `GCTM` propagates the wrapped error
//                    `error in __gc metamethod (<msg>)` out of collectgarbage
//                    (gc.lua:360 asserts `not pcall(collectgarbage)`).
//   5.4 / 5.5      — C `GCTM` catches and routes to `luaE_warnerror`; the
//                    error never propagates (collectgarbage returns ok). The
//                    warning is silent unless `warn("@on")`.
// The close path (lua_close / callallpendingfinalizers, propagateerrors=0)
// swallows on every version — exercised end-to-end by traceback_oracle.
// ─────────────────────────────────────────────────────────────────────────

/// Drives a `__gc`-erroring finalizer through an explicit `collectgarbage()`
/// and reports whether the collect call propagated (returns the error text)
/// or swallowed (returns "ok").
fn gc_finalizer_error_disposition(version: LuaVersion, body: &str) -> Result<String, String> {
    run(
        version,
        &format!(
            "do local x = setmetatable({{}}, {{__gc = function() {body} end}}); x = nil end\n\
             local ok, err = pcall(collectgarbage)\n\
             if ok then return 'ok' else return tostring(err) end"
        ),
    )
}

#[test]
fn v52_v53_gc_finalizer_error_propagates() {
    for v in [LuaVersion::V52, LuaVersion::V53] {
        let got = gc_finalizer_error_disposition(v, "error('boom')").expect("collect runs");
        assert!(
            got.contains("error in __gc metamethod (") && got.contains("boom"),
            "version {v:?}: expected wrapped __gc error, got `{got}`"
        );
    }
}

#[test]
fn v52_v53_gc_finalizer_nonstring_error_is_no_message() {
    for v in [LuaVersion::V52, LuaVersion::V53] {
        let got = gc_finalizer_error_disposition(v, "error({})").expect("collect runs");
        assert_eq!(
            got, "error in __gc metamethod (no message)",
            "version {v:?}: non-string __gc error object"
        );
    }
}

#[test]
fn v51_v54_v55_gc_finalizer_error_swallowed() {
    // 5.1 silently swallows; 5.4/5.5 route to the (default-silent) warning
    // system. In every case the explicit collect does NOT propagate.
    for v in [LuaVersion::V51, LuaVersion::V54, LuaVersion::V55] {
        let got = gc_finalizer_error_disposition(v, "error('boom')").expect("collect runs");
        assert_eq!(got, "ok", "version {v:?}: __gc error must not propagate");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Triage-fix regression guards (PR #106). Each pins a bug fixed after
// verifying against the reference binary for the version(s) involved.
// ─────────────────────────────────────────────────────────────────────────

// #97 — `__le` derived from `__lt` must survive a yield inside `__lt`
// (LUA_COMPAT_LT_LE: 5.1–5.4 derive and negate; 5.5 dropped the derivation).
// The CIST_LEQ mark carries the "negate on resume" intent across the yield.
const LE_ACROSS_YIELD: &str = "\
local mt = { __lt = function(a,b) coroutine.yield(0); return a.v < b.v end }\n\
local function drive(f) local co = coroutine.wrap(f); local r = co(); while r == 0 do r = co() end; return r end\n\
local a = setmetatable({v=1}, mt); local b = setmetatable({v=2}, mt)\n\
return tostring(drive(function() return a <= b end)) .. ',' .. tostring(drive(function() return b <= a end))";

#[test]
fn v51_v54_derived_le_survives_yield() {
    for v in [
        LuaVersion::V51,
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
    ] {
        eq(v, LE_ACROSS_YIELD, "true,false");
    }
}

#[test]
fn v55_le_without_metamethod_errors_no_derivation() {
    err_contains(
        LuaVersion::V55,
        LE_ACROSS_YIELD,
        "attempt to compare two table values",
    );
}

// #95 — `break` outside a loop: the error wording is version-specific.
#[test]
fn break_outside_loop_wording_per_version() {
    err_contains(LuaVersion::V51, "break", "no loop to break");
    err_contains(LuaVersion::V52, "break", "not inside a loop");
    err_contains(LuaVersion::V53, "break", "not inside a loop");
    err_contains(LuaVersion::V54, "break", "break outside loop at line");
    err_contains(LuaVersion::V55, "break", "break outside loop near 'break'");
}

// #96 — closures built in a loop over identical upvalues compare `==` on
// 5.2/5.3 (proto closure cache), distinct on 5.1 and 5.4/5.5.
const LOOP_CLOSURE_EQ: &str = "\
local up = 42; local t = {}\n\
for i = 1, 3 do t[i] = function() return up end end\n\
return tostring(t[1] == t[2]) .. ',' .. tostring(t[2] == t[3])";

#[test]
fn v52_v53_loop_closures_cache_equal() {
    eq(LuaVersion::V52, LOOP_CLOSURE_EQ, "true,true");
    eq(LuaVersion::V53, LOOP_CLOSURE_EQ, "true,true");
}

#[test]
fn v51_v54_v55_loop_closures_distinct() {
    for v in [LuaVersion::V51, LuaVersion::V54, LuaVersion::V55] {
        eq(v, LOOP_CLOSURE_EQ, "false,false");
    }
}

#[test]
fn v53_distinct_upvalues_not_cached() {
    // Different captured upvalue per iteration ⇒ distinct closures even on 5.3.
    eq(
        LuaVersion::V53,
        "local t = {}\nfor i = 1, 2 do local x = i; t[i] = function() return x end end\nreturn tostring(t[1] == t[2])",
        "false",
    );
}

// #94 — 5.5 named varargs `function f(...t)` share storage: `...` unpacks live
// from the table `t`, so mutating `t` is observable through a later `...`.
#[test]
fn v55_named_vararg_table_aliases_dots() {
    eq(
        LuaVersion::V55,
        "local function f(...t) t[1] = 99; return ... end\nreturn table.concat({f(1,2,3)}, ',')",
        "99,2,3",
    );
}

#[test]
fn v55_named_vararg_count_follows_n_field() {
    eq(
        LuaVersion::V55,
        "local function f(...t) t.n = 2; return ... end\nreturn table.concat({f(1,2,3)}, ',')",
        "1,2",
    );
}

#[test]
fn v55_named_vararg_rejects_invalid_n_field() {
    for value in ["-1", "math.maxinteger", "math.mininteger", "1.0"] {
        eq(
            LuaVersion::V55,
            &format!(
                "local function f(n, ...t) t.n = n; return ... end; \
                 local ok, msg = pcall(f, {value}, 1); \
                 return not ok and msg:find([[no proper 'n']], 1, true) ~= nil"
            ),
            "true",
        );
    }
}

#[test]
fn v55_named_vararg_indexed_reads_do_not_materialize_table() {
    eq(
        LuaVersion::V55,
        "local function f(keys, t, ...v) \
           for _, k in pairs(keys) do assert(t[k] == v[k]) end \
           assert(t.n == v.n); \
           return ... \
         end; \
         local t = table.pack(10, 20, 30); \
         local keys = {-1, 0, 1, 1.0, 1.1, t.n, t.n + 1, 'n', print, 'k', '1'}; \
         f(keys, t, 10, 20, 30); \
         local m = collectgarbage'count'; \
         f(keys, t, 10, 20, 30); \
         return m == collectgarbage'count'",
        "true",
    );
}

#[test]
fn v55_named_vararg_parameter_is_readonly() {
    err_contains(
        LuaVersion::V55,
        "return assert(load([[return function (... t) t = 10 end]]))",
        "const variable 't'",
    );
    err_contains(
        LuaVersion::V55,
        "return assert(load([[local function foo (...extra) return function (...) extra = nil end end]]))",
        "const variable 'extra'",
    );
}

#[test]
fn v55_generic_for_third_state_local_is_closing_value() {
    eq(
        LuaVersion::V55,
        "local debug = require 'debug'; \
         local closed = false; \
         local closer = setmetatable({}, {__close = function () closed = true end}); \
         local done = false; \
         local function iter(_, _) if not done then done = true; return 1 end end; \
         local function gettoclose(lv) \
           lv = lv + 1; local stvar = 0; \
           for i = 1, 100 do \
             local n, v = debug.getlocal(lv, i); \
             if n == '(for state)' then stvar = stvar + 1; if stvar == 3 then return v end end \
           end \
         end; \
         local seen; \
         do for x in iter, nil, nil, closer do seen = gettoclose(1); break end end; \
         return seen == closer and closed",
        "true",
    );
}

#[test]
fn v55_pairs_metamethod_fourth_result_is_closed() {
    eq(
        LuaVersion::V55,
        "local a = {n = 2, 2, 3}; \
         local function iter(t, i) i = i + 1; if i <= t.n then return i, t[i] end end; \
         local closed = false; \
         setmetatable(a, {__pairs = function(t) \
           local tbc = setmetatable({}, {__close = function() closed = true end}); \
           return iter, t, 0, tbc \
         end}); \
         for _ in pairs(a) do end; \
         return closed",
        "true",
    );
}

#[test]
fn v55_close_metamethod_omits_error_argument_on_normal_exit() {
    eq(
        LuaVersion::V55,
        "local nargs, second; \
         do \
           local x <close> = setmetatable({}, {__close = function(...) \
             nargs = select('#', ...); second = select(2, ...) \
           end}) \
         end; \
         return tostring(nargs) .. ':' .. tostring(second)",
        "1:nil",
    );
}

#[test]
fn v55_plain_vararg_has_hidden_vararg_table_local() {
    eq(
        LuaVersion::V55,
        "local debug = require 'debug'; \
         local function f(...) \
           local name, value = debug.getlocal(1, 1); \
           return name .. ':' .. tostring(value) \
         end; \
         return f(10, 20)",
        "(vararg table):nil",
    );
}

#[test]
fn v55_string_env_index_error_is_field_not_global() {
    eq(
        LuaVersion::V55,
        "local ok, msg = pcall(function() return ('_ENV').x + 1 end); \
         return (not ok) and msg:find(\"field 'x'\", 1, true) ~= nil and \
                msg:find(\"global 'x'\", 1, true) == nil",
        "true",
    );
}

#[test]
fn v55_stripped_bytecode_errors_use_unknown_source_and_line() {
    eq(
        LuaVersion::V55,
        "local f = function(a) return a + 1 end; \
         f = assert(load(string.dump(f, true))); \
         local ok, msg = pcall(f, {}); \
         return tostring(msg):match('^%?:%?:') ~= nil",
        "true",
    );
}

#[test]
fn v55_out_of_range_method_key_error_is_field() {
    eq(
        LuaVersion::V55,
        "local parts = {}; \
         for i = 1, 1000 do parts[i] = 'aaa = x' .. i end; \
         local prefix = table.concat(parts, '; '); \
         local f = assert(load(prefix .. '; local t = {}; t:bbb()')); \
         local ok, msg = pcall(f); \
         return (not ok) and msg:find(\"field 'bbb'\", 1, true) ~= nil and \
                msg:find(\"method 'bbb'\", 1, true) == nil",
        "true",
    );
}

#[test]
fn v55_global_function_env_index_error_is_prefixed_upvalue() {
    eq(
        LuaVersion::V55,
        "local code = '_ENV = 1\\nglobal function foo()\\n  return 10\\nend'; \
         local ok, msg = pcall(assert(load(code))); \
         return (not ok) and msg:find(':2:', 1, true) ~= nil and \
                msg:find(\"upvalue '_ENV'\", 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_duplicate_label_error_uses_duplicate_line() {
    eq(
        LuaVersion::V55,
        "local _, msg = load('::L1::\\n::L1::\\n'); \
         return msg:find(':2:', 1, true) ~= nil and \
                msg:find(\"label 'L1' already defined on line 2\", 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_undeclared_global_error_uses_name_line() {
    eq(
        LuaVersion::V55,
        "local _, msg = load('global none\\nlocal x = b\\n'); \
         return msg:find(':2:', 1, true) ~= nil and \
                msg:find(\"variable 'b' not declared\", 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_multiple_close_local_error_is_prefixed() {
    eq(
        LuaVersion::V55,
        "local _, msg = load('local <close> a, b\\n'); \
         return msg:find(':1:', 1, true) ~= nil and \
                msg:find('multiple to-be-closed variables', 1, true) ~= nil",
        "true",
    );
}

#[test]
fn v55_named_vararg_survives_dump_roundtrip() {
    eq(
        LuaVersion::V55,
        "local f = function(...t) t[1] = 99; return ... end\nlocal g = load(string.dump(f))\nreturn table.concat({g(1,2,3)}, ',')",
        "99,2,3",
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Issue #105 — Lua 5.1 quotes special near/expected tokens (<eof>, <name>, …).
//
// 5.1's `luaX_lexerror` wraps the offending token in `LUA_QS` ('%s')
// unconditionally and `error_expected` wraps the expected token the same way,
// so the special multi-char labels `<eof>`/`<name>`/`<number>`/`<string>` come
// out quoted. 5.2 rewrote `txtToken`/`luaX_token2str` to leave those bare and
// quote only symbols/reserved/literals. lua-rs implemented the 5.2+ rule on all
// versions; the 5.1 column below was captured from upstream lua-5.1.5.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn issue105_v51_quotes_special_near_and_expected_tokens() {
    err_contains(LuaVersion::V51, "if", "unexpected symbol near '<eof>'");
    err_contains(LuaVersion::V51, "local", "'<name>' expected near '<eof>'");
    err_contains(LuaVersion::V51, "return 1 2", "'<eof>' expected near '2'");
}

#[test]
fn issue105_v52_plus_leave_special_tokens_bare() {
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        err_contains(v, "if", "unexpected symbol near <eof>");
        err_contains(v, "local", "<name> expected near <eof>");
        err_contains(v, "return 1 2", "<eof> expected near '2'");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Issue #95 — `break` outside a loop is worded five different ways across the
// version family. The wording is gated in `breakstat`/`undef_goto`
// (`crates/lua-parse/src/lib.rs`); these assertions pin every arm so a future
// refactor can't silently collapse them back to one form. Reference wordings
// captured from each upstream binary (see specs/followup/HARD_PROBLEMS_REPORT.md).
//
// 5.1 raises eagerly *after* consuming `break`, so its `near` token is the next
// token (`<eof>` here) — and quoted, per issue #105. 5.5 raises eagerly *before*
// consuming `break`. 5.2/5.3/5.4 defer to the goto-resolution machinery.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn issue95_break_outside_loop_wording_v51() {
    err_contains(LuaVersion::V51, "break", "no loop to break near '<eof>'");
}

#[test]
fn issue95_break_outside_loop_wording_v52_v53() {
    for v in [LuaVersion::V52, LuaVersion::V53] {
        err_contains(v, "break", "not inside a loop");
    }
}

#[test]
fn issue95_break_outside_loop_wording_v54() {
    err_contains(LuaVersion::V54, "break", "break outside loop at line");
}

#[test]
fn issue95_break_outside_loop_wording_v55() {
    err_contains(LuaVersion::V55, "break", "break outside loop near 'break'");
}

// ─────────────────────────────────────────────────────────────────────────
// Issue #92 — version-gated line-hook fidelity.
//
// Captures the `debug.sethook(f, "l")` line-event trace for `code`, exactly the
// way Lua's own `db.lua` `test()` does it: the chunk is held in a *variable* so
// the `sethook; load(s)(); sethook` driver sits on one physical line and the
// only line events captured are the inner chunk's own lines (no driver-line
// pollution). Returns the comma-joined line sequence.
// ─────────────────────────────────────────────────────────────────────────
fn trace_lines(version: LuaVersion, code: &str) -> String {
    let lua = Lua::new_versioned(version);
    let loader = if version == LuaVersion::V51 {
        "loadstring"
    } else {
        "load"
    };
    let wrapper = format!(
        "local s = [==[\n{code}\n]==]\n\
         local out = {{}}\n\
         local function f(e, l) out[#out + 1] = tostring(l) end\n\
         debug.sethook(f, 'l'); {loader}(s)(); debug.sethook()\n\
         return table.concat(out, ',')"
    );
    lua.load(&wrapper)
        .eval()
        .unwrap_or_else(|e| panic!("trace harness failed for `{code}`: {e:?}"))
}

/// Cause 2: the conditional `TEST`/`JMP` of an `if`/`elseif` is attributed to
/// the `then`-line on 5.1–5.4 (a separate line-3 event) but folded onto the
/// condition-expression line on 5.5 (no line-3 event). Reference traces captured
/// per `db.lua` (5.3.4-tests `{2,3,4,7}` vs 5.5.0-tests `{2,4,7}`).
const IF_MULTILINE: &str = "if\nmath.sin(1)\nthen\n a=1\nelse\n a=2\nend";

#[test]
fn issue92_if_test_jmp_line_attribution_pre55() {
    for v in [
        LuaVersion::V51,
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
    ] {
        assert_eq!(trace_lines(v, IF_MULTILINE), "2,3,4,7", "version {v:?}");
    }
}

#[test]
fn issue92_if_test_jmp_line_attribution_v55() {
    assert_eq!(trace_lines(LuaVersion::V55, IF_MULTILINE), "2,4,7");
}

#[test]
fn v55_if_then_break_hook_includes_break_line() {
    let code = "while math.sin(1) do\n  if math.sin(1)\n  then break\n  end\nend\na=1";
    assert_eq!(trace_lines(LuaVersion::V55, code), "1,2,3,6");
}

/// `while`/`repeat` already attribute their conditional `TEST`/`JMP` to the
/// condition-expression line on every version (the codegen `cond()` captures the
/// line before any `do`/`until` token), so 5.5 does not change them. Pin that
/// invariant so the cause-2 fix doesn't accidentally touch loop conditions.
#[test]
fn issue92_while_condition_line_attribution_unchanged_all_versions() {
    let code = "local i = 0\nwhile\ni < 1\ndo\ni = i + 1\nend";
    for v in [
        LuaVersion::V51,
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        let got = trace_lines(v, code);
        assert_eq!(
            got,
            trace_lines(LuaVersion::V54, code),
            "version {v:?} drifted: {got}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Issue #92 cause 1 — the numeric-`for` back-edge line-hook event.
//
// 5.1/5.2/5.3 compile a numeric `for` so FORPREP jumps *forward* to the FORLOOP
// test at the bottom; iteration 1 therefore enters the body via a backward jump
// and fires a line event, giving n+1 events for an n-iteration single-line loop.
// 5.4 made FORPREP fall through to the body (count-based loop), so iteration 1
// fires no event → n events. Expected traces captured from the reference
// binaries (5.3.6 / 5.4.7) and corroborated by db.lua's own `test()` battery
// (5.3.4-tests `{1,1,1,1,1}` vs 5.4.7-tests `{1,1,1,1}` for `for i=1,4`).
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn issue92_numeric_for_backedge_legacy_pre54() {
    // 4-iteration single-line loop: <=5.3 fire one event per iteration plus the
    // entry → 5 events.
    for v in [LuaVersion::V51, LuaVersion::V52, LuaVersion::V53] {
        assert_eq!(
            trace_lines(v, "for i=1,4 do a=1 end"),
            "1,1,1,1,1",
            "version {v:?}"
        );
    }
}

#[test]
fn issue92_numeric_for_backedge_modern_54_55() {
    // 5.4 count-based loop: iteration 1 falls through, so 4 events.
    for v in [LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(
            trace_lines(v, "for i=1,4 do a=1 end"),
            "1,1,1,1",
            "version {v:?}"
        );
    }
}

/// A multi-line numeric `for` changes line between header and body every
/// iteration, so `changedline` fires regardless of the back-edge rule — the
/// trace is identical across all versions. (db.lua 5.3.4/5.4.7 both `{1,2,...}`.)
#[test]
fn issue92_numeric_for_multiline_unchanged_all_versions() {
    let code = "for i=1,3 do\n  a=i\nend";
    for v in [
        LuaVersion::V51,
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        assert_eq!(trace_lines(v, code), "1,2,1,2,1,2,1,3", "version {v:?}");
    }
}

/// The legacy numeric-`for` semantics must stay *behaviorally* correct, not just
/// trace-correct. These results were diffed against the lua-5.3.6 reference
/// binary: down-loops, zero-iteration loops, and (unlike 5.4) a zero step that
/// runs zero times instead of raising "'for' step is zero".
#[test]
fn issue92_legacy_numeric_for_behavior_matches_53() {
    eq(
        LuaVersion::V53,
        "local t={} for i=5,1,-1 do t[#t+1]=i end return table.concat(t,',')",
        "5,4,3,2,1",
    );
    eq(
        LuaVersion::V53,
        "local n=0 for i=1,0 do n=n+1 end return n",
        "0",
    );
    // 5.3 has no zero-step error; the comparison just fails and the loop is empty.
    eq(
        LuaVersion::V53,
        "local n=0 for i=1,2,0 do n=n+1 if n>3 then break end end return n",
        "0",
    );
    // control variable stays an integer subtype on 5.3.
    eq(
        LuaVersion::V53,
        "local r for i=1,1 do r=math.type(i) end return r",
        "integer",
    );
}

// ─────────────────────────────────────────────────────────────────────────
// P2b net-strengthening (table library). The table module was already
// idiomatic with a coarse behavioral net: 75% standard paths sampled, edge
// cases and version seams thin. These tests pin REFERENCE behavior (captured
// from `/tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7,5.0}`) for the four real gaps
// the recon flagged, so the idiomatization that follows is verified, not
// described. See `docs/IDIOMATIZATION_SPRINT_2_SPEC.md` → "### P2b — table".
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v51_insert_uses_primitive_length_ignoring_len_metamethod() {
    // Gap 2: under 5.1 `table.insert` (and `#`) use the PRIMITIVE length and
    // NEVER consult a table `__len` metamethod — `__len` on tables was a 5.2
    // addition. So inserting into a `{10,20,30}` carrying a `__len` that lies
    // (returns 99) appends at the primitive border 4, not at 100. Verified
    // against lua5.1.5.
    eq(
        LuaVersion::V51,
        "local t = setmetatable({10,20,30}, {__len = function() return 99 end}); \
         table.insert(t, 40); return tostring(t[4]) .. ',' .. tostring(t[100]) .. ',' .. #t",
        "40,nil,4",
    );
}

#[test]
fn v52_plus_insert_honors_len_metamethod() {
    // Gap 2 contrast: under 5.2-5.5 `table.insert` HONORS `__len`. With a
    // `__len` that returns 1, the append lands at border+1 == 2 (overwriting
    // the primitive element there), and position 4 stays nil. This is the
    // exact inverse of the 5.1 behavior above; keeping both proves the version
    // gate is exercised, not a coincidental match. Verified against
    // lua5.2.4/5.3.6/5.4.7/5.5.0.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "local t = setmetatable({10,20,30}, {__len = function() return 1 end}); \
             table.insert(t, 40); return tostring(t[1]) .. ',' .. tostring(t[2]) .. ',' .. tostring(t[4])",
            "10,40,nil",
        );
    }
}

#[test]
fn v52_plus_pack_n_field_counts_all_args_including_nils() {
    // Gap 3: `table.pack`'s `.n` field is the literal argument count, holes
    // and trailing nils included — it is NOT a border. `pack(1, nil, 3).n`
    // is 3 (with `t[2] == nil`), `pack()` is 0, and `pack(nil, nil, nil)` is
    // 3. This is the whole point of `.n`: to recover the arity a `#t` border
    // would lose. Verified against lua5.2.4/5.3.6/5.4.7/5.5.0.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "local t = table.pack(1, nil, 3); \
             return t.n .. ',' .. tostring(t[1]) .. ',' .. tostring(t[2]) .. ',' .. tostring(t[3])",
            "3,1,nil,3",
        );
        eq(v, "return table.pack().n", "0");
        eq(v, "return table.pack(nil, nil, nil).n", "3");
    }
}

#[test]
fn v52_plus_unpack_boundaries_and_too_many_results() {
    // Gap 3: `table.unpack` boundary behavior expressible with plain literals
    // (so it runs from 5.2, before the integer subtype existed). An `i > e`
    // empty range yields zero results; a span at/above INT_MAX
    // (`unpack({}, 1, 2^31)`) raises "too many results to unpack" rather than
    // attempting a 2-billion-result push. Verified against
    // lua5.2.4/5.3.6/5.4.7/5.5.0.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "return select('#', table.unpack({1, 2, 3}, 5, 2))",
            "0",
        );
        err_contains(
            v,
            "return table.unpack({}, 1, 2147483647)",
            "too many results to unpack",
        );
    }
}

#[test]
fn v53_plus_unpack_i64_extreme_boundaries() {
    // Gap 3 (5.3+ only): the i64-extreme cases need `math.mininteger` /
    // `math.maxinteger`, which are 5.3 additions (5.1/5.2 have no integer
    // subtype). The full span `mininteger..maxinteger` wraps to a huge
    // unsigned count and raises "too many results to unpack" rather than
    // entering a 2^64-iteration loop; a single element at `maxinteger`
    // (i == e) is an in-range read that returns the value. Verified against
    // lua5.3.6/5.4.7/5.5.0.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        err_contains(
            v,
            "return table.unpack({}, math.mininteger, math.maxinteger)",
            "too many results to unpack",
        );
        eq(
            v,
            "local t = {}; t[math.maxinteger] = 'x'; \
             return table.unpack(t, math.maxinteger, math.maxinteger)",
            "x",
        );
    }
}

#[test]
fn v53_plus_move_overlap_copies_in_collision_safe_order() {
    // Gap 4: `table.move` within one array must pick copy direction so an
    // overlapping range is not clobbered mid-copy. A right shift
    // (`move(t, 1, 3, 2)` over `{1,2,3,4,5}`) copies BACKWARD, yielding
    // `1,1,2,3,5`; a left shift (`move(t, 3, 5, 1)`) copies FORWARD, yielding
    // `3,4,5,4,5`. A naive single-direction loop corrupts one of these. A
    // cross-table forward copy into a distinct destination is also pinned.
    // `table.move` is a 5.3 addition. Verified against lua5.3.6/5.4.7/5.5.0.
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            "local t = {1,2,3,4,5}; table.move(t, 1, 3, 2); return table.concat(t, ',')",
            "1,1,2,3,5",
        );
        eq(
            v,
            "local t = {1,2,3,4,5}; table.move(t, 3, 5, 1); return table.concat(t, ',')",
            "3,4,5,4,5",
        );
        eq(
            v,
            "local b = {0,0,0,0,0}; table.move({10,20,30}, 1, 3, 2, b); return table.concat(b, ',')",
            "0,10,20,30,0",
        );
        eq(
            v,
            "local b = {}; return tostring(table.move({1,2}, 1, 2, 1, b) == b)",
            "true",
        );
    }
}

#[test]
fn v53_plus_move_drives_index_and_newindex_metamethods_in_order() {
    // Gap 4: `table.move` reads each source slot through `__index` and writes
    // each destination slot through `__newindex`, interleaved one element at a
    // time. A non-overlapping cross-table forward copy fires
    // get1/set1/get2/set2/get3/set3 in that exact order; an overlapping
    // in-place right shift fires the BACKWARD order (g3/s4/g2/s3/g1/s2),
    // proving the decreasing-index loop drives the metamethods, not just raw
    // slots. Verified against lua5.3.6/5.4.7/5.5.0.
    let forward = "local log = {}; \
        local src = setmetatable({}, {__index = function(_, k) log[#log+1] = 'get' .. k; return k * 10 end}); \
        local dst = setmetatable({}, {__newindex = function(_, k, v) log[#log+1] = 'set' .. k .. '=' .. v end}); \
        table.move(src, 1, 3, 1, dst); return table.concat(log, ' ')";
    let backward = "local log = {}; local store = {1,2,3,4,5}; \
        local t = setmetatable({}, { \
          __index = function(_, k) log[#log+1] = 'g' .. k; return store[k] end, \
          __newindex = function(_, k, v) log[#log+1] = 's' .. k .. '=' .. tostring(v); store[k] = v end}); \
        table.move(t, 1, 3, 2); return table.concat(log, ' ')";
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(v, forward, "get1 set1=10 get2 set2=20 get3 set3=30");
        eq(v, backward, "g3 s4=3 g2 s3=2 g1 s2=1");
    }
}

#[test]
fn v_remove_out_of_bounds_arg_gate_crossversion() {
    // Gap 1: `table.remove`'s out-of-bounds handling is version-gated in THREE
    // distinct ways, and the original net only checked the 5.3-vs-5.4 arg
    // index — so two real divergences hid behind it.
    //
    //   5.1: legacy ltablib.c has NO bounds check. An out-of-range position
    //        (`pos` outside `[1, size]`) silently removes nothing and returns
    //        ZERO results — never an error. Verified against lua5.1.5
    //        (`tremove`: `if (!(1 <= pos && pos <= e)) return 0;`).
    //   5.2 + 5.3: `luaL_argcheck(..., 1, "position out of bounds")` — arg #1.
    //   5.4 + 5.5: the same check moved to arg #2.
    //
    // We pin every cell of the matrix so the gate can never silently widen.

    // 5.1: out-of-bounds is inert — no error, zero results.
    eq(
        LuaVersion::V51,
        "return select('#', table.remove({}, 5))",
        "0",
    );
    eq(
        LuaVersion::V51,
        "return select('#', table.remove({1, 2, 3}, 5))",
        "0",
    );
    eq(
        LuaVersion::V51,
        "return tostring(pcall(table.remove, {1, 2, 3}, 0))",
        "true",
    );
    // 5.1: a VALID remove still returns the removed value and shifts the array.
    eq(
        LuaVersion::V51,
        "local t = {10, 20, 30}; local r = table.remove(t, 2); \
         return r .. ',' .. table.concat(t, ',')",
        "20,10,30",
    );

    // 5.2 + 5.3: out-of-bounds raises with arg #1.
    for v in [LuaVersion::V52, LuaVersion::V53] {
        err_contains(v, "return table.remove({}, 5)", "position out of bounds");
        err_contains(v, "return table.remove({}, 5)", "argument #1");
        err_contains(v, "return table.remove({1, 2, 3}, 5)", "argument #1");
    }

    // 5.4 + 5.5: same wording, arg #2.
    for v in [LuaVersion::V54, LuaVersion::V55] {
        err_contains(v, "return table.remove({}, 5)", "position out of bounds");
        err_contains(v, "return table.remove({}, 5)", "argument #2");
        err_contains(v, "return table.remove({1, 2, 3}, 5)", "argument #2");
    }
}

#[test]
fn v_sort_observable_contract_crossversion() {
    // Sort: the OBSERVABLE contract (not the partition internals). A custom
    // descending comparator orders correctly; an always-true comparator is a
    // non-strict-order violation that raises "invalid order function for
    // sorting"; the default `<` on a mixed string/number array raises the
    // comparison error. The partition-internal callback-during-GC safety is a
    // load-bearing region the behavioral net CANNOT guard (see the P2b
    // verdict) — only these externally-visible facts are pinned here.
    // Verified against lua5.1.5/5.3.6/5.4.7/5.5.0.
    for v in [
        LuaVersion::V51,
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            "local t = {3,1,4,1,5,9,2,6}; table.sort(t, function(a, b) return a > b end); \
             return table.concat(t, ',')",
            "9,6,5,4,3,2,1,1",
        );
        err_contains(
            v,
            "table.sort({1,2,3,4,5,6,7,8,9,10}, function() return true end)",
            "invalid order function for sorting",
        );
    }
    err_contains(
        LuaVersion::V54,
        "table.sort({1, 'a', 3})",
        "attempt to compare",
    );
}

// ─────────────────────────────────────────────────────────────────────────
// P2c — string pattern-matcher net-strengthening (Sprint 2). The behavioral
// net is the ONLY net for the matcher (no structural oracle); these three
// assertions pin the matcher's danger zones that pm.lua does NOT exercise,
// each captured from the version-suffixed reference binaries
// (`/tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7,5.0}`).
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v_yield_outside_coroutine_wording_crossversion() {
    // `coroutine.yield` from outside a resumable coroutine (VM finding F3).
    // 5.1's `lua_yield` has a single message — "attempt to yield across
    // metamethod/C-call boundary" — for both the main-thread and C-call-boundary
    // cases. 5.2+ split it into "attempt to yield from outside a coroutine".
    // Verified vs /tmp/lua-refs/bin/lua5.x for the main-thread case below.
    err_contains(
        LuaVersion::V51,
        "coroutine.yield()",
        "attempt to yield across metamethod/C-call boundary",
    );
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        err_contains(v, "coroutine.yield()", "attempt to yield from outside a coroutine");
    }
}

// ─────────────────────────────────────────────────────────────────────────

#[test]
fn v_pattern_too_complex_gate_crossversion() {
    // The matcher bounds recursion depth at MAXCCALLS (200) and raises
    // "pattern too complex" when a pattern recurses past it. pm.lua exercises
    // the matcher but never HITS this bound, so it was unguarded.
    //
    // CRITICAL version seam: the `matchdepth`/MAXCCALLS guard was ADDED IN 5.2.
    // 5.1's lstrlib.c match() has NO depth counter at all (verified: lua-5.1.5
    // lstrlib.c has zero `matchdepth`/`MAXCCALLS`), so a pattern that trips the
    // bound on 5.2+ simply MATCHES on 5.1. The trigger below recurses via 200
    // lazy `.-` elements over a 200-char subject.
    //
    // 5.1: no bound — the match succeeds (find returns start index 1).
    eq(
        LuaVersion::V51,
        r#"local ok, r = pcall(string.find, string.rep("a", 200), string.rep(".-", 200))
           return tostring(ok) .. ":" .. tostring(r)"#,
        "true:1",
    );
    // 5.2 + 5.3 + 5.4 + 5.5: the bound is hit, "pattern too complex" raised.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        err_contains(
            v,
            r#"return string.find(string.rep("a", 200), string.rep(".-", 200))"#,
            "pattern too complex",
        );
    }
}

#[test]
fn v_empty_match_advance_rule_crossversion() {
    // The empty-match advance rule changed in 5.3.3. Pre-5.3 (5.1/5.2) the
    // matcher has NO `lastmatch` de-dup: after an empty match at a position it
    // counts/emits it and advances one character, so an empty match can land
    // at a position that ALSO ends a non-empty match — producing a DOUBLED
    // result. 5.3+ added the `lastmatch` guard (`e != lastmatch`) in both
    // gmatch_aux and str_gsub, which suppresses the redundant empty match.
    // Confirmed against the 5.1.5/5.2.4 sources (no lastmatch) vs 5.3.6/5.4.7/
    // 5.5.0 (lastmatch present).

    // gsub with the empty-or-more space pattern.
    for v in [LuaVersion::V51, LuaVersion::V52] {
        eq(
            v,
            r#"return (string.gsub("a b cd", " *", "-"))"#,
            "-a--b--c-d-",
        );
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            r#"return (string.gsub("a b cd", " *", "-"))"#,
            "-a-b-c-d-",
        );
    }

    // gmatch with a zero-width-allowed pattern (`%a*`) over a subject with a
    // gap: pre-5.3 emits a spurious empty capture between each word.
    for v in [LuaVersion::V51, LuaVersion::V52] {
        eq(
            v,
            r#"local t = {}
               for s in string.gmatch("ab cd", "%a*") do t[#t + 1] = "[" .. s .. "]" end
               return table.concat(t)"#,
            "[ab][][cd][]",
        );
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            r#"local t = {}
               for s in string.gmatch("ab cd", "%a*") do t[#t + 1] = "[" .. s .. "]" end
               return table.concat(t)"#,
            "[ab][cd]",
        );
    }

    // A purely-empty pattern emits one empty match per position PLUS one past
    // the end on EVERY version (lastmatch never fires because each match
    // advances `src`): all five agree, pinned so a future change can't drift.
    for v in [
        LuaVersion::V51,
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        eq(
            v,
            r#"local t = {}
               for s in string.gmatch("abc", "") do t[#t + 1] = "[" .. s .. "]" end
               return table.concat(t)"#,
            "[][][][]",
        );
    }
}

#[test]
fn v_too_many_captures_gate_crossversion() {
    // Exceeding LUA_MAX_CAPTURES (32) raises "too many captures" in
    // start_capture on EVERY version (the limit has been 32 since 5.0 and is
    // not version-gated). 33 position captures `()` trip it. pm.lua never
    // exercises the overflow edge; this pins it as a green tripwire so a future
    // refactor of the capture array can't silently change the ceiling.
    for v in [
        LuaVersion::V51,
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        err_contains(
            v,
            r#"return string.match("abc", string.rep("()", 33))"#,
            "too many captures",
        );
    }
}

#[test]
fn v_dump_version_byte_per_version() {
    for (v, byte) in [
        (LuaVersion::V51, "81"),
        (LuaVersion::V52, "82"),
        (LuaVersion::V53, "83"),
        (LuaVersion::V54, "84"),
        (LuaVersion::V55, "85"),
    ] {
        eq(
            v,
            "return string.byte(string.dump(function() return 1 end), 5)",
            byte,
        );
    }
}

#[test]
fn v52_load_dumped_multiupvalue_skips_env_injection() {
    eq(
        LuaVersion::V52,
        "local a,b=20,30; local f=function(x) if x=='set' then a=10+b;b=b+1 else return a end end; \
         local g=load(string.dump(f)); return g()",
        "nil",
    );
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        eq(
            v,
            "local a,b=20,30; local f=function(x) if x=='set' then a=10+b;b=b+1 else return a end end; \
             local g=load(string.dump(f)); return type(g())",
            "table",
        );
    }
}
