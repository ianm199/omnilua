//! Behavioral net for the `debug` library's deterministic introspection
//! surface (the Phase-2 net-strengthening packet, P2-debug).
//!
//! `debug.*` reaches deep into the VM (call stack, activation records, upvalue
//! cells, the registry). Much of its *output* is non-deterministic — addresses,
//! `function: 0x...` reprs, exact short_src truncation of long temp paths — and
//! is deliberately NOT pinned here. What IS pinned is the deterministic,
//! address-free contract that the official `db.lua` suite exercises only on 5.4:
//!
//! - `getinfo` field correctness on a known function (`what`, `source`,
//!   `short_src`, `linedefined`, `lastlinedefined`, `nparams`, `isvararg`,
//!   `nups`, `name`, `namewhat`, `currentline`) — pinned per version, because
//!   the `u` option's `nparams`/`isvararg` is a 5.2 addition (5.1 emits only
//!   `nups`).
//! - `getlocal`/`setlocal` by index and the function-argument parameter-name
//!   form (a 5.2 addition — 5.1 rejects a function argument).
//! - `getupvalue`/`setupvalue` name+value round-trip.
//! - `traceback` frame STRUCTURE (the message line, the `stack traceback:`
//!   header, and the per-frame line shape) — never addresses.
//! - `sethook` count-hook firing and `gethook` mask round-trip.
//!
//! These pin REFERENCE behavior captured from `/tmp/lua-refs/bin/lua5.x`; the
//! per-version `getinfo`/`getlocal` cases caught two real 5.1 divergences (the
//! impl emitted 5.2+ `nparams`/`isvararg` on 5.1 and accepted the 5.2+
//! function-argument `getlocal` form on 5.1) — both fixed in `debug_lib.rs`.
//!
//! `omnilua` is a dev-dependency here (it depends on `lua-stdlib`, so it can
//! only be a dev-dep — see `Cargo.toml`).

use omnilua::{Lua, LuaVersion};

/// All versions the multi-version core serves.
const ALL_VERSIONS: [LuaVersion; 5] = [
    LuaVersion::V51,
    LuaVersion::V52,
    LuaVersion::V53,
    LuaVersion::V54,
    LuaVersion::V55,
];

/// Versions with the modern (5.2+) `debug` surface: `getinfo`'s `u` option
/// reports `nparams`/`isvararg`, and `getlocal`/`setlocal` accept a function
/// argument for parameter-name introspection.
const V52_PLUS: [LuaVersion; 4] = [
    LuaVersion::V52,
    LuaVersion::V53,
    LuaVersion::V54,
    LuaVersion::V55,
];

/// Evaluate `code` under `version` and return the single string it produces.
///
/// Every probe program ends in `return <joined string>` so the assertion is an
/// exact, deterministic string comparison against the reference-captured value
/// (no tuple/multi-value or address surface enters the test).
fn eval_str(version: LuaVersion, code: &str) -> String {
    let lua = Lua::new_versioned(version);
    lua.load(code)
        .eval::<String>()
        .unwrap_or_else(|e| panic!("eval under {version:?} failed: {e:?}\ncode:\n{code}"))
}

/// A function defined on a fixed two-line body via `load(src, "=known")`, so its
/// `source`/`short_src` are the deterministic chunk name `known` (not a
/// temp-file path) on every version and every host.
const KNOWN_FN: &str = "local function known(a, b)\n  local x = a + b\n  return x\nend\nreturn known";

// ── getinfo: deterministic field correctness ────────────────────────────────

/// `getinfo(known, "Sln")` — the address-free fields are identical on every
/// version: a Lua function, chunk-named source, the two def lines, no name.
#[test]
fn getinfo_known_function_core_fields_all_versions() {
    let probe = format!(
        "local known = (loadstring or load)([[{KNOWN_FN}]], '=known')()\n\
         local i = debug.getinfo(known, 'Sln')\n\
         return tostring(i.what)..'|'..tostring(i.source)..'|'..tostring(i.short_src)\n\
           ..'|'..tostring(i.linedefined)..'|'..tostring(i.lastlinedefined)\n\
           ..'|'..tostring(i.name)..'|'..tostring(i.namewhat)"
    );
    for v in ALL_VERSIONS {
        assert_eq!(
            eval_str(v, &probe),
            "Lua|=known|known|1|4|nil|",
            "getinfo core fields diverged under {v:?}"
        );
    }
}

/// `getinfo(known, "u")` — the `u` option's `nparams`/`isvararg` is a **5.2
/// addition**. On 5.1 it reports only `nups`; `nparams`/`isvararg` are absent
/// (`nil`). This case caught a real divergence: the impl emitted the 5.2+
/// fields on 5.1.
#[test]
fn getinfo_u_option_nparams_isvararg_is_v52_plus() {
    let probe = format!(
        "local known = (loadstring or load)([[{KNOWN_FN}]], '=known')()\n\
         local i = debug.getinfo(known, 'u')\n\
         return tostring(i.nups)..'|'..tostring(i.nparams)..'|'..tostring(i.isvararg)"
    );
    // 5.1: only nups; nparams/isvararg absent.
    assert_eq!(
        eval_str(LuaVersion::V51, &probe),
        "0|nil|nil",
        "5.1 getinfo 'u' must report only nups (no nparams/isvararg)"
    );
    // 5.2+: nups + nparams + isvararg.
    for v in V52_PLUS {
        assert_eq!(
            eval_str(v, &probe),
            "0|2|false",
            "{v:?} getinfo 'u' must report nups, nparams, isvararg"
        );
    }
}

/// `getinfo(level, "l").currentline` — the line of the *call site* of getinfo,
/// deterministic for a fixed program layout, on every version.
#[test]
fn getinfo_currentline_at_known_line_all_versions() {
    // getinfo is called on line 2 of this chunk (1-based, after the leading \n
    // stripped by load); pin the value the reference reports.
    let probe = "\nreturn tostring(debug.getinfo(1, 'l').currentline)";
    for v in ALL_VERSIONS {
        assert_eq!(
            eval_str(v, probe),
            "2",
            "getinfo currentline diverged under {v:?}"
        );
    }
}

/// `getinfo` of a C function — `what == "C"`, `short_src == "[C]"`, on every
/// version (db.lua line 38 pins this only on 5.4).
#[test]
fn getinfo_c_function_what_and_short_src_all_versions() {
    let probe = "local i = debug.getinfo(print)\n\
                 return tostring(i.what)..'|'..tostring(i.short_src)";
    for v in ALL_VERSIONS {
        assert_eq!(eval_str(v, probe), "C|[C]", "getinfo(C fn) diverged under {v:?}");
    }
}

// ── getlocal / setlocal ──────────────────────────────────────────────────────

/// `getlocal(level, index)` on a running frame — names and values of the active
/// locals in declaration order, deterministic on every version.
#[test]
fn getlocal_by_index_on_running_frame_all_versions() {
    let probe = "local function withlocals(p, q)\n\
                   local aa = 10\n\
                   local n1, v1 = debug.getlocal(1, 1)\n\
                   local n2, v2 = debug.getlocal(1, 2)\n\
                   local n3, v3 = debug.getlocal(1, 3)\n\
                   return tostring(n1)..'='..tostring(v1)..'|'\n\
                     ..tostring(n2)..'='..tostring(v2)..'|'\n\
                     ..tostring(n3)..'='..tostring(v3)\n\
                 end\n\
                 return withlocals(7, 8)";
    for v in ALL_VERSIONS {
        assert_eq!(
            eval_str(v, probe),
            "p=7|q=8|aa=10",
            "getlocal by index diverged under {v:?}"
        );
    }
}

/// `getlocal(func, index)` — the parameter-name form is a **5.2 addition**.
/// On 5.2+ it returns the parameter names (no value); on 5.1 a function
/// argument is rejected (`number expected, got function`). This case caught a
/// real divergence: the impl accepted the function form on 5.1.
#[test]
fn getlocal_function_param_name_form_is_v52_plus() {
    // 5.2+: function argument yields parameter names; 4th index is absent.
    let probe_ok = format!(
        "local known = (loadstring or load)([[{KNOWN_FN}]], '=known')()\n\
         return tostring(debug.getlocal(known, 1))..'|'\n\
           ..tostring(debug.getlocal(known, 2))..'|'\n\
           ..tostring(debug.getlocal(known, 3))"
    );
    for v in V52_PLUS {
        assert_eq!(
            eval_str(v, &probe_ok),
            "a|b|nil",
            "{v:?} getlocal(func, n) must return parameter names"
        );
    }
    // 5.1: a function argument is rejected (the func form does not exist).
    let probe_err = format!(
        "local known = (loadstring or load)([[{KNOWN_FN}]], '=known')()\n\
         local ok = pcall(debug.getlocal, known, 1)\n\
         return tostring(ok)"
    );
    assert_eq!(
        eval_str(LuaVersion::V51, &probe_err),
        "false",
        "5.1 getlocal(func, n) must error (function form is 5.2+)"
    );
}

/// `setlocal(level, index, value)` mutates the named local and returns its
/// name; deterministic on every version.
#[test]
fn setlocal_by_index_mutates_and_returns_name_all_versions() {
    let probe = "local function setl()\n\
                   local z = 1\n\
                   local name = debug.setlocal(1, 1, 42)\n\
                   return tostring(name)..'|'..tostring(z)\n\
                 end\n\
                 return setl()";
    for v in ALL_VERSIONS {
        assert_eq!(eval_str(v, probe), "z|42", "setlocal diverged under {v:?}");
    }
}

// ── getupvalue / setupvalue ──────────────────────────────────────────────────

/// `getupvalue`/`setupvalue` name+value round-trip on every version.
#[test]
fn upvalue_get_set_roundtrip_all_versions() {
    let probe = "local up = 100\n\
                 local function usesup() up = up + 1; return up end\n\
                 local gn, gv = debug.getupvalue(usesup, 1)\n\
                 local sn = debug.setupvalue(usesup, 1, 555)\n\
                 return tostring(gn)..'='..tostring(gv)..'|'\n\
                   ..tostring(sn)..'|'..tostring(usesup())";
    for v in ALL_VERSIONS {
        assert_eq!(
            eval_str(v, probe),
            "up=100|up|556",
            "upvalue get/set round-trip diverged under {v:?}"
        );
    }
}

// ── traceback: frame STRUCTURE (no addresses) ────────────────────────────────

/// `debug.traceback(msg, level)` — pin the address-free structure: the message
/// line first, the `stack traceback:` header second, and the presence of a
/// chunk-named frame line. Addresses and the exact tail-call wording (which
/// differs 5.1 vs 5.2+) are NOT pinned.
#[test]
fn traceback_structure_message_then_header_then_frames_all_versions() {
    let probe = "local src = 'local function l3() return debug.traceback(\\'MYMSG\\', 1) end\\n'\n\
                   ..'local function l2() return l3() end\\n'\n\
                   ..'return l2()'\n\
                 local tb = (loadstring or load)(src, '=t')()\n\
                 local lines = {}\n\
                 for line in tb:gmatch('[^\\n]+') do lines[#lines + 1] = line end\n\
                 -- first line is the message; second is the header; pin a frame shape\n\
                 local has_chunk_frame = false\n\
                 for _, l in ipairs(lines) do\n\
                   if l:find('t:1:', 1, true) then has_chunk_frame = true end\n\
                 end\n\
                 return lines[1]..'|'..lines[2]..'|'..tostring(has_chunk_frame)";
    for v in ALL_VERSIONS {
        assert_eq!(
            eval_str(v, probe),
            "MYMSG|stack traceback:|true",
            "traceback structure diverged under {v:?}"
        );
    }
}

/// `debug.traceback` with a non-string message returns it unchanged (the
/// message is returned as-is when it is not a string), on every version.
#[test]
fn traceback_nonstring_message_returned_unchanged_all_versions() {
    let probe = "local t = {}\n\
                 return tostring(debug.traceback(t) == t)";
    for v in ALL_VERSIONS {
        assert_eq!(
            eval_str(v, probe),
            "true",
            "traceback(non-string) must return the message unchanged under {v:?}"
        );
    }
}

// ── sethook / gethook ────────────────────────────────────────────────────────

/// A count hook (mask `""`, count `1`) fires while a loop runs, and `gethook`
/// reports the active mask; deterministic shape on every version.
#[test]
fn sethook_count_hook_fires_and_gethook_reports_mask_all_versions() {
    let probe = "local fires = 0\n\
                 debug.sethook(function() fires = fires + 1 end, '', 1)\n\
                 local x = 0\n\
                 for i = 1, 50 do x = x + i end\n\
                 debug.sethook()\n\
                 debug.sethook(function() end, 'l', 0)\n\
                 local _, mask = debug.gethook()\n\
                 debug.sethook()\n\
                 return tostring(fires > 0)..'|'..tostring(mask)";
    for v in ALL_VERSIONS {
        assert_eq!(
            eval_str(v, probe),
            "true|l",
            "count-hook firing / gethook mask diverged under {v:?}"
        );
    }
}

/// With no hook installed, `gethook` reports the fail value (nil) — db.lua line
/// 16 pins this only on 5.4.
#[test]
fn gethook_with_no_hook_is_nil_all_versions() {
    let probe = "return tostring(debug.gethook())";
    for v in ALL_VERSIONS {
        assert_eq!(eval_str(v, probe), "nil", "gethook (no hook) diverged under {v:?}");
    }
}
