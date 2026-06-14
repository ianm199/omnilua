//! Behavioral net for the BASE library's **version-gated** surface.
//!
//! `base` is the most VM-adjacent stdlib module: `pcall`/`xpcall`/`error` drive
//! error unwinding, `load` compiles, `next`/`pairs`/`ipairs` iterate, `type`/
//! `tostring`/`raw*` are hot. All of that plumbing is LOAD-BEARING and pinned
//! only thinly by `multiversion_oracle`. This file strengthens the net for the
//! COLD version seams that `multiversion_oracle` + the official suite leave
//! WEAK or UNCOVERED, pinning each against the version-suffixed reference
//! binaries (`/tmp/lua-refs/bin/lua5.x`, captured 2026-06-14 byte-identical
//! across 5.1.5/5.2.4/5.3.6/5.4.7/5.5.0).
//!
//! Three `*_crossversion` tests FAILED at baseline — the net catching real
//! pre-existing bugs the weaker net hid (the Phase-2 discipline, cf.
//! `table.remove` / the string matcher / `os.time`):
//!
//! * **`ipairs` over a non-raw table.** 5.1/5.2 `ipairsaux` uses `lua_rawgeti`
//!   (no `__index`) and `luaL_checktype(1, TABLE)`; 5.3+ switched to `lua_geti`
//!   (honors `__index`) and dropped the type check. Our impl applied the modern
//!   `__index`-consulting, type-check-free path to ALL versions.
//! * **`assert(false, msg)` with a non-string message.** 5.1/5.2 `luaB_assert`
//!   routes the message through `luaL_optstring`, so a present non-string-
//!   coercible 2nd arg raises `bad argument #2 to 'assert' (string expected,
//!   got <type>)`; 5.3+ forward the raw object to `error`. Our impl forwarded
//!   on all versions.
//! * **`rawlen` argument-error wording.** The reference names the function
//!   (`to 'rawlen'`) and, on 5.4/5.5 only (`luaL_argexpected`), appends
//!   `, got <type>`; 5.2/5.3 (`luaL_argcheck`) omit the suffix. Our impl emitted
//!   the nameless `bad argument #1 (table or string expected, got <type>)` on
//!   all versions.
//!
//! The remaining tests are **green at baseline**: they convert correct-but-
//! unguarded paths (the `raw*` no-metamethod contract, `select`'s negative/`#`
//! indexing, `tonumber` base conversion, the `__pairs` 5.1-vs-rest seam, the
//! 5.1-only roster gates, `_VERSION` per version) into tripwires so a future
//! idiomatization or shared-core change cannot silently break them.
//!
//! `omnilua` is a dev-dependency (it depends on `lua-stdlib`, so it can only be
//! a dev-dep — see `Cargo.toml`).

use omnilua::{Lua, LuaVersion, Value};

const ALL: [LuaVersion; 5] = [
    LuaVersion::V51,
    LuaVersion::V52,
    LuaVersion::V53,
    LuaVersion::V54,
    LuaVersion::V55,
];

/// Evaluate `code` under `version`, returning a string return value as bytes.
fn eval_str(version: LuaVersion, code: &str) -> Vec<u8> {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(Value::String(s)) => s
            .as_bytes()
            .unwrap_or_else(|e| panic!("string bytes under {version:?} for `{code}`: {e:?}"))
            .to_vec(),
        Ok(other) => panic!("`{code}` under {version:?} returned {other:?}, expected a string"),
        Err(e) => panic!("eval of `{code}` failed under {version:?}: {e:?}"),
    }
}

/// Evaluate `code` under `version`, returning an integer return value.
fn eval_int(version: LuaVersion, code: &str) -> i64 {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(Value::Integer(i)) => i,
        Ok(Value::Number(n)) if n.fract() == 0.0 => n as i64,
        Ok(other) => panic!("`{code}` under {version:?} returned {other:?}, expected an integer"),
        Err(e) => panic!("eval of `{code}` failed under {version:?}: {e:?}"),
    }
}

/// Evaluate `code`, expecting a boolean-`true` return (an invariant probe).
fn assert_true(version: LuaVersion, code: &str) {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(Value::Boolean(true)) => {}
        Ok(other) => panic!("`{code}` under {version:?} returned {other:?}, expected true"),
        Err(e) => panic!("eval of `{code}` failed under {version:?}: {e:?}"),
    }
}

/// Evaluate `code`, expecting it to raise; return the error message lossily.
fn eval_err(version: LuaVersion, code: &str) -> String {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(v) => panic!("expected error under {version:?} for `{code}`, got {v:?}"),
        Err(e) => e.message_lossy(),
    }
}

// ── ipairs: the 5.1/5.2 raw-access + table-check seam (caught a real bug) ──────

#[test]
fn ipairs_consults_index_only_from_5_3_crossversion() {
    // 5.1/5.2 `ipairsaux` uses `lua_rawgeti` — `__index` is NOT consulted, so a
    // table whose array part is empty iterates zero times even when `__index`
    // would supply values. 5.3+ switched to `lua_geti`, which honors `__index`.
    let probe = "\
        local t = setmetatable({}, {__index = function(_, k) \
            if k <= 3 then return k * 10 end \
        end}) \
        local c = 0 \
        for i, v in ipairs(t) do c = c + 1 end \
        return c";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        assert_eq!(eval_int(v, probe), 0, "{v:?}: ipairs must be raw (no __index)");
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(eval_int(v, probe), 3, "{v:?}: ipairs must honor __index");
    }
}

#[test]
fn ipairs_type_checks_table_only_pre_5_3_crossversion() {
    // 5.1/5.2 `ipairsaux` calls `luaL_checktype(1, LUA_TTABLE)`, so `ipairs` over
    // a non-table raises `bad argument #1 to 'ipairs' (table expected, got …)`.
    // 5.3+ dropped the check (the iterator's `lua_geti` works on any indexable),
    // so `ipairs("hi")` simply iterates zero times instead of raising.
    let probe = "for i, v in ipairs('hi') do end return 'ran'";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        let msg = eval_err(v, probe);
        assert!(
            msg.contains("table expected") && msg.contains("ipairs"),
            "{v:?}: expected a 'table expected' arg error for ipairs, got `{msg}`"
        );
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(eval_str(v, probe), b"ran", "{v:?}: ipairs over a string must not raise");
    }
}

#[test]
fn ipairs_consults_ipairs_metamethod_on_5_2_and_5_3_crossversion() {
    // The `__ipairs` metamethod (LUA_COMPAT_IPAIRS, default ON in 5.2/5.3 via
    // LUA_COMPAT_5_2) makes `ipairs(t)` call `t`'s `__ipairs` for its iterator.
    // 5.1 predates it; 5.4/5.5 removed it. So a `__ipairs` returning an empty
    // iterator suppresses iteration on 5.2/5.3 but not on 5.1/5.4/5.5 (which
    // iterate the array part). Confirmed against lua5.2.4/5.3.6 (honored) and
    // lua5.4.7 (ignored).
    let probe = "\
        local t = setmetatable({'A', 'B'}, {__ipairs = function(x) \
            return function() return nil end, x, 0 \
        end}) \
        local s = '' \
        for i, v in ipairs(t) do s = s .. v end \
        return s";
    for v in [LuaVersion::V52, LuaVersion::V53] {
        assert_eq!(eval_str(v, probe), b"", "{v:?}: __ipairs must suppress iteration");
    }
    for v in [LuaVersion::V51, LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(eval_str(v, probe), b"AB", "{v:?}: __ipairs must be ignored");
    }
}

// ── assert: the 5.1/5.2 string-message seam (caught a real bug) ───────────────

#[test]
fn assert_message_must_be_string_pre_5_3_crossversion() {
    // 5.1/5.2 `luaB_assert` does `luaL_error("%s", luaL_optstring(L, 2, ...))`,
    // so a present non-string-coercible message raises an arg error on the 2nd
    // argument. 5.3+ forward the raw object to `error` unchanged (so a table
    // message becomes the error object itself).
    let probe = "return select(2, pcall(function() assert(false, {code = 7}) end))";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        let msg = eval_err(
            v,
            "assert(false, {code = 7})",
        );
        assert!(
            msg.contains("string expected") && msg.contains("assert"),
            "{v:?}: assert(false, <table>) must raise a string-expected arg error, got `{msg}`"
        );
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        // 5.3+: the error object IS the table — it survives as a table value.
        let lua = Lua::new_versioned(v);
        let got = lua
            .load(probe)
            .eval::<Value>()
            .unwrap_or_else(|e| panic!("{v:?}: {e:?}"));
        assert!(
            matches!(got, Value::Table(_)),
            "{v:?}: assert(false, <table>) must forward the table object, got {got:?}"
        );
    }
}

#[test]
fn assert_number_message_is_stringified_pre_5_3_else_forwarded_crossversion() {
    // A NUMBER second argument diverges. 5.1/5.2 `luaL_optstring` stringifies it
    // and `luaL_error` location-prefixes the result, so the error is the STRING
    // "…: 404". 5.3+ forward the raw value to `error`, which leaves a non-string
    // object verbatim, so the error is the NUMBER 404 itself. Confirmed against
    // lua5.{1.5,2.4,3.6,4.7,5.0}.
    let probe = "return select(2, pcall(function() assert(false, 404) end))";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        let lua = Lua::new_versioned(v);
        match lua.load(probe).eval::<Value>() {
            Ok(Value::String(s)) => {
                let bytes = s.as_bytes().unwrap();
                assert!(bytes.ends_with(b"404"), "{v:?}: `{:?}`", bytes);
            }
            other => panic!("{v:?}: assert(false, 404) must be a prefixed string, got {other:?}"),
        }
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        let lua = Lua::new_versioned(v);
        match lua.load(probe).eval::<Value>() {
            Ok(Value::Integer(404)) | Ok(Value::Number(_)) => {}
            other => panic!("{v:?}: assert(false, 404) must forward the number, got {other:?}"),
        }
    }
}

#[test]
fn assert_string_message_and_default_all_versions() {
    // A string message is location-prefixed on every version (5.1/5.2 via
    // luaL_error, 5.3+ via error's string-prefix path); the absent-message case
    // is the default literal on every version. (Green at baseline.)
    for v in ALL {
        assert!(eval_err(v, "assert(false, 'boom')").ends_with("boom"), "{v:?}");
        assert!(
            eval_err(v, "assert(false)").ends_with("assertion failed!"),
            "{v:?}"
        );
    }
}

// ── rawlen: argument-error wording seam (caught a real bug) ───────────────────

#[test]
fn rawlen_arg_error_names_function_and_gates_got_suffix_crossversion() {
    // rawlen is absent on 5.1 (a 5.2 addition), so this seam starts at 5.2.
    // 5.2/5.3 use `luaL_argcheck(..., "table or string expected")` → NO `, got`
    // suffix. 5.4/5.5 use `luaL_argexpected(..., "table or string")` → the
    // suffix `, got <type>` is appended. Both name the function (`to 'rawlen'`).
    // Assert on the function-named extramsg (the location prefix the wrapper
    // adds is host/chunk-name dependent, so match the stable tail).
    for v in [LuaVersion::V52, LuaVersion::V53] {
        let msg = eval_err(v, "return rawlen(5)");
        assert!(
            msg.ends_with("bad argument #1 to 'rawlen' (table or string expected)"),
            "{v:?}: `{msg}`"
        );
    }
    for v in [LuaVersion::V54, LuaVersion::V55] {
        assert!(
            eval_err(v, "return rawlen(5)")
                .ends_with("bad argument #1 to 'rawlen' (table or string expected, got number)"),
            "{v:?}"
        );
        assert!(
            eval_err(v, "return rawlen(true)")
                .ends_with("bad argument #1 to 'rawlen' (table or string expected, got boolean)"),
            "{v:?}"
        );
        assert!(
            eval_err(v, "return rawlen(nil)")
                .ends_with("bad argument #1 to 'rawlen' (table or string expected, got nil)"),
            "{v:?}"
        );
    }
}

#[test]
fn rawlen_accepts_tables_and_strings_5_2_plus() {
    // The success path: rawlen of a table is its border, of a string its byte
    // length, on every version that has it (5.2+). (Green at baseline.)
    for v in [LuaVersion::V52, LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(eval_int(v, "return rawlen({1, 2, 3})"), 3, "{v:?}");
        assert_eq!(eval_int(v, "return rawlen('hello')"), 5, "{v:?}");
        // rawlen ignores `__len` (it is RAW).
        assert_eq!(
            eval_int(
                v,
                "return rawlen(setmetatable({1, 2}, {__len = function() return 99 end}))"
            ),
            2,
            "{v:?}: rawlen must ignore __len"
        );
    }
}

#[test]
fn rawlen_absent_on_5_1() {
    // rawlen is a 5.2 addition: on 5.1 the global is nil. (Green at baseline;
    // pins the V51 roster gate in base.rs's `open`.)
    assert_eq!(eval_str(LuaVersion::V51, "return type(rawlen)"), b"nil");
}

// ── the raw* no-metamethod contract (green-at-baseline tripwires) ─────────────

#[test]
fn rawget_rawset_rawequal_bypass_metamethods_all_versions() {
    for v in ALL {
        // rawget ignores __index.
        assert_eq!(
            eval_str(
                v,
                "local t = setmetatable({}, {__index = function() return 'META' end}) \
                 t.x = 1 \
                 return tostring(rawget(t, 'x')) .. ',' .. tostring(rawget(t, 'y'))"
            ),
            b"1,nil",
            "{v:?}"
        );
        // rawset ignores __newindex.
        assert_eq!(
            eval_str(
                v,
                "local hit = false \
                 local t = setmetatable({}, {__newindex = function() hit = true end}) \
                 rawset(t, 'k', 'v') \
                 return tostring(t.k) .. ',' .. tostring(hit)"
            ),
            b"v,false",
            "{v:?}"
        );
        // rawequal ignores __eq: two distinct tables sharing an __eq metatable
        // are raw-unequal but ==-equal.
        assert_eq!(
            eval_str(
                v,
                "local m = {__eq = function() return true end} \
                 local a = setmetatable({}, m) \
                 local b = setmetatable({}, m) \
                 return tostring(rawequal(a, b)) .. ',' .. tostring(a == b)"
            ),
            b"false,true",
            "{v:?}"
        );
    }
}

// ── select: negative index, '#', and out-of-range (green at baseline) ─────────

#[test]
fn select_count_and_negative_index_all_versions() {
    for v in ALL {
        assert_eq!(eval_int(v, "return select('#', 'a', 'b', 'c')"), 3, "{v:?}");
        // select(n) returns args from position n; positive index.
        assert_eq!(eval_str(v, "return select(2, 'a', 'b', 'c')"), b"b", "{v:?}");
        // Negative index counts from the end: -1 is the last argument.
        assert_eq!(eval_str(v, "return select(-1, 'a', 'b', 'c')"), b"c", "{v:?}");
        // -2 returns the last two; pin via a join.
        assert_eq!(
            eval_str(
                v,
                "return table.concat({select(-2, 'a', 'b', 'c')}, ',')"
            ),
            b"b,c",
            "{v:?}"
        );
    }
}

#[test]
fn select_out_of_range_index_raises_all_versions() {
    // index 0 and a negative index past the start both raise "index out of
    // range". The function-name resolution on 5.1/5.2 differs (`'?'` / `'_G.…'`)
    // and is a separate VM-layer naming gap, so this pins only the stable
    // `(index out of range)` extramsg, not the function name.
    for v in ALL {
        assert!(
            eval_err(v, "return select(0, 'a', 'b')").contains("index out of range"),
            "{v:?}"
        );
        assert!(
            eval_err(v, "return select(-9, 'a', 'b')").contains("index out of range"),
            "{v:?}"
        );
    }
}

// ── tonumber: base conversion + subtype + base-range error (green at baseline) ─

#[test]
fn tonumber_base_conversion_all_versions() {
    for v in ALL {
        assert_eq!(eval_int(v, "return tonumber('0x10')"), 16, "{v:?}");
        assert_eq!(eval_int(v, "return tonumber('11', 2)"), 3, "{v:?}");
        assert_eq!(eval_int(v, "return tonumber('ff', 16)"), 255, "{v:?}");
        assert_eq!(eval_int(v, "return tonumber('z', 36)"), 35, "{v:?}");
        // A digit out of range for the base → nil (not an error).
        assert_eq!(eval_str(v, "return tostring(tonumber('2', 2))"), b"nil", "{v:?}");
    }
    // base out of range (1 / 37) raises with the function name (resolved on
    // 5.3+; the 5.1/5.2 `'?'`/`'_G.…'` naming is the separate VM gap).
    for v in ALL {
        assert!(
            eval_err(v, "return tonumber('x', 1)").contains("base out of range"),
            "{v:?}"
        );
        assert!(
            eval_err(v, "return tonumber('x', 37)").contains("base out of range"),
            "{v:?}"
        );
    }
}

#[test]
fn tonumber_subtype_is_integer_from_5_3() {
    // The integer/float subtype is a 5.3 addition, OBSERVABLE via `math.type`:
    // `math.type(tonumber('10')) == 'integer'` from 5.3 on. Pinned through the
    // language (not a white-box Value peek) because the reference exposes it.
    // 5.1/5.2 have no `math.type` and `type()` says "number" for both subtypes,
    // so the pre-5.3 subtype is invisible from Lua and NOT reference-pinnable
    // here — deliberately omitted rather than pinned to our own output (the
    // tautology Phase-2 forbids; cf. math's platform-rand honest-negative).
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(eval_str(v, "return math.type(tonumber('10'))"), b"integer", "{v:?}");
        assert_eq!(eval_str(v, "return math.type(tonumber('10.0'))"), b"float", "{v:?}");
        // A based conversion is always an integer subtype on 5.3+.
        assert_eq!(eval_str(v, "return math.type(tonumber('ff', 16))"), b"integer", "{v:?}");
    }
}

// ── pairs / __pairs: honored on 5.2+, ignored on 5.1 (green at baseline) ──────

#[test]
fn pairs_consults_pairs_metamethod_from_5_2_crossversion() {
    // Lua 5.1 has no `__pairs`: `pairs(t)` iterates the raw table even when a
    // `__pairs` is present. 5.2+ honor it (5.4/5.5 did NOT behaviorally remove
    // it — the reference still consults `__pairs` for an explicit iterator,
    // confirmed against lua5.4.7/lua5.5.0). A `__pairs` returning an empty
    // iterator therefore yields zero iterations on 5.2+ but three on 5.1.
    let probe = "\
        local t = setmetatable({1, 2, 3}, {__pairs = function() \
            return function() return nil end, t, nil \
        end}) \
        local c = 0 \
        for k, v in pairs(t) do c = c + 1 end \
        return c";
    assert_eq!(eval_int(LuaVersion::V51, probe), 3, "5.1 ignores __pairs");
    for v in [LuaVersion::V52, LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(eval_int(v, probe), 0, "{v:?} honors __pairs");
    }
}

// ── 5.1-only roster gates (green at baseline) ─────────────────────────────────

#[test]
fn v51_only_globals_present_only_on_5_1() {
    // gcinfo / newproxy / getfenv / setfenv are 5.1-ONLY holdovers, gone in 5.2+.
    for fname in ["gcinfo", "newproxy", "getfenv", "setfenv"] {
        assert_eq!(
            eval_str(LuaVersion::V51, &format!("return type({fname})")),
            b"function",
            "5.1 must expose {fname}"
        );
        for v in [LuaVersion::V52, LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
            assert_eq!(
                eval_str(v, &format!("return type({fname})")),
                b"nil",
                "{v:?} must NOT expose {fname}"
            );
        }
    }
    // `loadstring` and the global `unpack` survive into 5.2 (5.2 keeps
    // `loadstring` as a `load` alias under the default `LUA_COMPAT_*` build, and
    // `unpack` as a `table.unpack` alias), then both are removed in 5.3+.
    // Confirmed against lua5.2.4 (both functions) and lua5.3.6 (both nil).
    for fname in ["loadstring", "unpack"] {
        for v in [LuaVersion::V51, LuaVersion::V52] {
            assert_eq!(
                eval_str(v, &format!("return type({fname})")),
                b"function",
                "{v:?} must expose {fname}"
            );
        }
        for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
            assert_eq!(
                eval_str(v, &format!("return type({fname})")),
                b"nil",
                "{v:?} must NOT expose {fname}"
            );
        }
    }
}

#[test]
fn warn_present_only_from_5_4() {
    // `warn` is a 5.4 addition: a function on 5.4/5.5, nil on 5.1/5.2/5.3.
    for v in [LuaVersion::V51, LuaVersion::V52, LuaVersion::V53] {
        assert_eq!(eval_str(v, "return type(warn)"), b"nil", "{v:?}");
    }
    for v in [LuaVersion::V54, LuaVersion::V55] {
        assert_eq!(eval_str(v, "return type(warn)"), b"function", "{v:?}");
    }
}

// ── _VERSION per version (green at baseline) ──────────────────────────────────

#[test]
fn version_global_string_per_version() {
    assert_eq!(eval_str(LuaVersion::V51, "return _VERSION"), b"Lua 5.1");
    assert_eq!(eval_str(LuaVersion::V52, "return _VERSION"), b"Lua 5.2");
    assert_eq!(eval_str(LuaVersion::V53, "return _VERSION"), b"Lua 5.3");
    assert_eq!(eval_str(LuaVersion::V54, "return _VERSION"), b"Lua 5.4");
    assert_eq!(eval_str(LuaVersion::V55, "return _VERSION"), b"Lua 5.5");
}

// ── error: object preservation + level prefix (green at baseline) ─────────────

#[test]
fn error_object_and_level_all_versions() {
    for v in ALL {
        // A non-string error object survives verbatim (no location prefix).
        assert_true(
            v,
            "local ok, e = pcall(function() error({code = 5}) end) \
             return (not ok) and type(e) == 'table' and e.code == 5",
        );
        // level 0 → no location prefix.
        assert_eq!(
            eval_str(
                v,
                "return select(2, pcall(function() error('boom', 0) end))"
            ),
            b"boom",
            "{v:?}"
        );
        // level 2 → the caller's location, not error's own line.
        assert_true(
            v,
            "local function f() error('x', 2) end \
             local ok, e = pcall(function() f() end) \
             return (not ok) and e:match(': x$') ~= nil",
        );
    }
}

// ── getmetatable: __metatable protection (green at baseline) ──────────────────

#[test]
fn getmetatable_honors_protected_metatable_all_versions() {
    for v in ALL {
        assert_eq!(
            eval_str(
                v,
                "return getmetatable(setmetatable({}, {__metatable = 'LOCKED'}))"
            ),
            b"LOCKED",
            "{v:?}"
        );
    }
}

// ── tostring: __tostring honored on every version (green at baseline) ─────────

#[test]
fn tostring_honors_tostring_metamethod_all_versions() {
    // `__tostring` predates the version range, so it is honored on every
    // version. (`__name` is a SEPARATE 5.3-addition seam left to the VM layer —
    // see GRADUATED.md / the agent report: it lives in `obj_type_name_cow`, used
    // across 23 VM error/display sites, so gating it is a VM-internal change.)
    for v in ALL {
        assert_eq!(
            eval_str(
                v,
                "return tostring(setmetatable({}, {__tostring = function() return 'CUSTOM' end}))"
            ),
            b"CUSTOM",
            "{v:?}"
        );
    }
}
