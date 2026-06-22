//! `error_wording_kit` — golden kit for per-version error/traceback funcname
//! wording (the deferred F1 "funcname-per-version resolver" architectural item).
//!
//! Flavor: golden constants — the EXACT error strings the real PUC-Rio reference
//! binaries emit for a battery of error-producing snippets, captured once from
//! `/tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7,5.5.0}` and frozen into `GOLDEN`
//! below. The kit then runs purely in-process against the frozen golden: NO
//! reference binary, NO subprocess, NO `/tmp` dependency — the rung-2 inner loop
//! the funcname fix develops against, in milliseconds, and it works on wasm/CI.
//!
//! WHY THIS KIT EXISTS: the C function-name shown in `bad argument #N to '<name>'`
//! and in `stack traceback` frames is resolved differently per version, and the
//! resolver (`luaL_argerror` -> `pushglobalfuncname` -> `findfield` in
//! `lauxlib.c`) is the residual cross-version dependency three agents flagged.
//! The per-version contract the golden pins:
//!   - 5.1 recorded NO names for C functions: every C-function arg error and
//!     traceback frame is `'?'` (PUC-Rio 5.1 has no `pushglobalfuncname`).
//!   - 5.2 resolves names against the *global table* and does NOT strip a `_G.`
//!     prefix (PUC-Rio 5.2's `pushglobalfuncname` has no strip step — that was
//!     added in 5.3): a loaded-module member is `'<module>.<field>'` (e.g.
//!     `'coroutine.resume'`); a bare global resolved through the `_G` module is
//!     `'_G.<name>'`.
//!   - 5.3+ resolve against `package.loaded` and DO strip a leading `_G.`
//!     (C: `strncmp(name, LUA_GNAME ".", 3)`), so a bare global is `'<name>'`.
//!   - 5.3+ also change some argument SEMANTICS (e.g. `math.max("hello")` coerces
//!     and succeeds; `coroutine.resume` says `thread expected`, 5.1/5.2 say
//!     `coroutine expected`) — captured here so the wording AND the
//!     name-resolution stay pinned together.
//!
//! ## 5.2 `_G.`-prefix non-determinism (why the V52 globals are pinned to `_G.`)
//!
//! PUC-Rio 5.2's name resolution for a *bare global* is genuinely
//! non-deterministic across runs of the reference binary itself. `findfield`
//! walks the global table, which contains `_G._G` (a self-reference), so a global
//! like `next` is reachable both directly (`'next'`) and one level deeper through
//! the self-reference (`'_G.next'`); which path `findfield` hits first depends on
//! hash-iteration order, so the same `pcall(next, 1)` prints `'next'` on one run
//! and `'_G.next'` on the next (verified by running `lua5.2 -e 'print(pcall(next,
//! 1))'` repeatedly — it flips). The module-member form (`'coroutine.resume'`)
//! is stable; only the bare-global `_G.`-vs-not choice flips.
//!
//! Because there is no single byte-exact string a deterministic port can emit
//! that MATCHes every reference run, this kit pins the deterministic `'_G.<name>'`
//! form for ALL V52 globals (our resolver searches `package.loaded`, where every
//! global is reachable only through the `_G` module, so `'_G.<name>'` falls out
//! deterministically — and it is one of the two valid reference outputs). The
//! `setmetatable` V52 row therefore reads `'_G.setmetatable'`, NOT the bare
//! `'setmetatable'` a given reference run may have printed; both are correct, and
//! the `diff_one.sh` oracle will appear to flip on these V52 globals for the same
//! reason. This is the ONE narrowed golden entry; all other coverage is retained.
//!
//! COVERED: the `'<name>'` token chosen by the resolver for C-function arg errors
//! across all five versions, plus the surrounding message text (so a wording
//! regression is also caught). NOT COVERED: full multi-frame traceback layout
//! (the `diff_one.sh` oracle and official suite cover that end-to-end); this kit
//! pins the one token that is the version seam.

use omnilua::{Lua, LuaVersion};

/// A frozen golden row: the version under test, the snippet to evaluate, and the
/// exact byte-for-byte string the reference binary printed for it.
struct Golden {
    version: LuaVersion,
    snippet: &'static str,
    expected: &'static str,
}

const fn g(version: LuaVersion, snippet: &'static str, expected: &'static str) -> Golden {
    Golden {
        version,
        snippet,
        expected,
    }
}

/// The battery. Each `snippet` is a single error-producing expression whose
/// `pcall` message we compare. `expected` is the reference's exact output for
/// `print(pcall(<the C function>, <bad args>))`, i.e. `"false\t<message>"`, with
/// the leading `false\t` stripped to `<message>` here (we drive `pcall`
/// ourselves and return only the message).
///
/// Captured 2026-06-21 from `/tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7,5.5.0}`.
const GOLDEN: &[Golden] = &[
    // ── 5.1: every C-function arg error names the function '?' ──────────────
    g(
        LuaVersion::V51,
        "coroutine.resume, \"x\"",
        "bad argument #1 to '?' (coroutine expected)",
    ),
    g(
        LuaVersion::V51,
        "math.max, \"hello\"",
        "bad argument #1 to '?' (number expected, got string)",
    ),
    g(
        LuaVersion::V51,
        "string.format, \"%d\", \"x\"",
        "bad argument #2 to '?' (number expected, got string)",
    ),
    g(
        LuaVersion::V51,
        "string.rep",
        "bad argument #1 to '?' (string expected, got no value)",
    ),
    g(
        LuaVersion::V51,
        "table.insert, 1, 2",
        "bad argument #1 to '?' (table expected, got number)",
    ),
    g(
        LuaVersion::V51,
        "setmetatable, 1, 2",
        "bad argument #1 to '?' (table expected, got number)",
    ),
    g(
        LuaVersion::V51,
        "ipairs",
        "bad argument #1 to '?' (table expected, got no value)",
    ),
    g(
        LuaVersion::V51,
        "next, 1",
        "bad argument #1 to '?' (table expected, got number)",
    ),
    g(
        LuaVersion::V51,
        "select, \"x\"",
        "bad argument #1 to '?' (number expected, got string)",
    ),
    // ── 5.2: module members qualified; bare globals via '_G.' (per C) ───────
    g(
        LuaVersion::V52,
        "coroutine.resume, \"x\"",
        "bad argument #1 to 'coroutine.resume' (coroutine expected)",
    ),
    g(
        LuaVersion::V52,
        "math.max, \"hello\"",
        "bad argument #1 to 'math.max' (number expected, got string)",
    ),
    g(
        LuaVersion::V52,
        "string.format, \"%d\", \"x\"",
        "bad argument #2 to 'string.format' (number expected, got string)",
    ),
    g(
        LuaVersion::V52,
        "table.insert, 1, 2",
        "bad argument #1 to 'table.insert' (table expected, got number)",
    ),
    g(
        LuaVersion::V52,
        "setmetatable, 1, 2",
        "bad argument #1 to '_G.setmetatable' (table expected, got number)",
    ),
    g(
        LuaVersion::V52,
        "ipairs",
        "bad argument #1 to '_G.ipairs' (table expected, got no value)",
    ),
    g(
        LuaVersion::V52,
        "next, 1",
        "bad argument #1 to '_G.next' (table expected, got number)",
    ),
    g(
        LuaVersion::V52,
        "tonumber, \"x\", 99",
        "bad argument #2 to '_G.tonumber' (base out of range)",
    ),
    g(
        LuaVersion::V52,
        "select, \"x\"",
        "bad argument #1 to '_G.select' (number expected, got string)",
    ),
    // ── 5.3: qualified names; '_G.' prefix no longer emitted; coroutine
    //         expects 'thread'; numeric coercion changed (max succeeds) ──────
    g(
        LuaVersion::V53,
        "coroutine.resume, \"x\"",
        "bad argument #1 to 'coroutine.resume' (thread expected)",
    ),
    g(
        LuaVersion::V53,
        "string.format, \"%d\", \"x\"",
        "bad argument #2 to 'string.format' (number expected, got string)",
    ),
    g(
        LuaVersion::V53,
        "table.insert, 1, 2",
        "bad argument #1 to 'table.insert' (table expected, got number)",
    ),
    g(
        LuaVersion::V53,
        "setmetatable, 1, 2",
        "bad argument #1 to 'setmetatable' (table expected, got number)",
    ),
    g(
        LuaVersion::V53,
        "ipairs",
        "bad argument #1 to 'ipairs' (value expected)",
    ),
    g(
        LuaVersion::V53,
        "next, 1",
        "bad argument #1 to 'next' (table expected, got number)",
    ),
    g(
        LuaVersion::V53,
        "select, \"x\"",
        "bad argument #1 to 'select' (number expected, got string)",
    ),
    // ── 5.4: BASELINE — must stay byte-identical ────────────────────────────
    g(
        LuaVersion::V54,
        "coroutine.resume, \"x\"",
        "bad argument #1 to 'coroutine.resume' (thread expected, got string)",
    ),
    g(
        LuaVersion::V54,
        "string.format, \"%d\", \"x\"",
        "bad argument #2 to 'string.format' (number expected, got string)",
    ),
    g(
        LuaVersion::V54,
        "table.insert, 1, 2",
        "bad argument #1 to 'table.insert' (table expected, got number)",
    ),
    g(
        LuaVersion::V54,
        "ipairs",
        "bad argument #1 to 'ipairs' (value expected)",
    ),
    g(
        LuaVersion::V54,
        "next, 1",
        "bad argument #1 to 'next' (table expected, got number)",
    ),
    g(
        LuaVersion::V54,
        "select, \"x\"",
        "bad argument #1 to 'select' (number expected, got string)",
    ),
    // ── 5.5: BASELINE — must stay byte-identical ────────────────────────────
    g(
        LuaVersion::V55,
        "coroutine.resume, \"x\"",
        "bad argument #1 to 'coroutine.resume' (thread expected, got string)",
    ),
    g(
        LuaVersion::V55,
        "string.format, \"%d\", \"x\"",
        "bad argument #2 to 'string.format' (number expected, got string)",
    ),
    g(
        LuaVersion::V55,
        "table.insert, 1, 2",
        "bad argument #1 to 'table.insert' (table expected, got number)",
    ),
    g(
        LuaVersion::V55,
        "ipairs",
        "bad argument #1 to 'ipairs' (value expected)",
    ),
    g(
        LuaVersion::V55,
        "next, 1",
        "bad argument #1 to 'next' (table expected, got number)",
    ),
    g(
        LuaVersion::V55,
        "select, \"x\"",
        "bad argument #1 to 'select' (number expected, got string)",
    ),
];

/// Evaluate `pcall(<snippet>)` under `version` and return the error message the
/// running VM produces (the second `pcall` return). The snippet is the
/// comma-separated `function, args...` list passed straight to `pcall`. Driving
/// `pcall` ourselves (rather than `print(pcall(...))`) avoids stdout capture and
/// returns the message string directly for comparison.
fn pcall_message(version: LuaVersion, snippet: &str) -> String {
    let lua = Lua::new_versioned(version);
    let wrapper = format!(
        "local ok, msg = pcall({snippet})\n\
         if ok then error('expected an error, got success') end\n\
         return tostring(msg)"
    );
    lua.load(&wrapper)
        .eval::<String>()
        .unwrap_or_else(|e| panic!("error_wording_kit harness failure ({version:?}): {e:?}"))
}

#[test]
fn arg_error_funcname_matches_reference_golden() {
    let mut failures = Vec::new();
    for row in GOLDEN {
        let got = pcall_message(row.version, row.snippet);
        if got != row.expected {
            failures.push(format!(
                "  {:?}  pcall({})\n      ours: {got}\n      ref : {}",
                row.version, row.snippet, row.expected
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "C-function arg-error funcname diverges from reference:\n{}",
        failures.join("\n")
    );
}

/// Focused regression: 5.1 must name EVERY C function `'?'` (no fallback to the
/// 5.2+ `pushglobalfuncname` qualified name). This is the specific seam the F1
/// fix gates — kept as its own assertion so a regression points straight at it.
#[test]
fn v51_c_functions_are_question_mark() {
    for snippet in &[
        "coroutine.resume, \"x\"",
        "math.max, \"hello\"",
        "string.format, \"%d\", \"x\"",
        "string.rep",
        "table.insert, 1, 2",
        "setmetatable, 1, 2",
        "ipairs",
        "next, 1",
        "select, \"x\"",
    ] {
        let got = pcall_message(LuaVersion::V51, snippet);
        assert!(
            got.contains("to '?'"),
            "5.1 pcall({snippet}) should name the C function '?', got: {got}"
        );
    }
}

/// Focused regression: 5.4 and 5.5 (the unchangeable baselines) must keep their
/// qualified names — the F1 5.1 gate must not leak into the modern core.
#[test]
fn modern_baselines_keep_qualified_names() {
    for version in &[LuaVersion::V54, LuaVersion::V55] {
        let got = pcall_message(*version, "coroutine.resume, \"x\"");
        assert_eq!(
            got, "bad argument #1 to 'coroutine.resume' (thread expected, got string)",
            "{version:?} must keep the qualified C-function name"
        );
    }
}
