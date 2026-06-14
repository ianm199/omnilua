//! White-box behavioral net for the `math.random` FloatOnly type invariant.
//!
//! Under the float-only number model (Lua 5.1 and 5.2), `math.random` must
//! never construct an integer subtype: every result — `random()`, `random(n)`,
//! and `random(m, n)` — is a `Float`. This is a real reference invariant
//! (`lmathlib.c` 5.1/5.2 push `lua_Number`, and the modern body honours it via
//! the `FloatOnly` gate), but it is **invisible to a behavioral oracle**: those
//! versions have no `math.type`, and `type()` reports `"number"` for both
//! subtypes, so the distinction cannot be seen from Lua.
//!
//! These tests look through the `omnilua` embedding API at the raw returned
//! [`Value`]: `Value::Integer` is an `Int`, `Value::Number` is a `Float`.
//! On 5.1/5.2 every draw must be `Number`; the 5.4 contrast (`Integer`) pins
//! that the gate is the thing being exercised, not a missing-integer accident.
//!
//! `omnilua` is a dev-dependency here (it depends on `lua-stdlib`, so it can
//! only be a dev-dep — see `Cargo.toml`).

use omnilua::{Lua, LuaVersion, Value};

/// Evaluate `code` under `version` and return the raw [`Value`] (subtype
/// preserved), so a test can tell `Int` from `Float`.
fn eval_value(version: LuaVersion, code: &str) -> Value {
    let lua = Lua::new_versioned(version);
    lua.load(code)
        .eval()
        .unwrap_or_else(|e| panic!("eval of `{code}` failed under {version:?}: {e:?}"))
}

/// Assert the result is a `Float` (`Value::Number`), the FloatOnly invariant.
fn assert_float(version: LuaVersion, code: &str) {
    match eval_value(version, code) {
        Value::Number(_) => {}
        other => panic!(
            "FloatOnly invariant violated under {version:?}: `{code}` returned \
             {other:?}, expected a Float (Value::Number)"
        ),
    }
}

#[test]
fn v51_v52_random_results_are_always_float() {
    for v in [LuaVersion::V51, LuaVersion::V52] {
        // No-arg float draw — trivially a Float, but pinned for completeness.
        assert_float(v, "math.randomseed(1); return math.random()");
        // One-arg [1, n] draw: the integer-valued projection result MUST still
        // be pushed as Float under FloatOnly (never Int).
        assert_float(v, "math.randomseed(1); return math.random(10)");
        assert_float(v, "math.randomseed(1); return math.random(1)");
        // Two-arg [m, n] draw: likewise Float.
        assert_float(v, "math.randomseed(1); return math.random(5, 8)");
        assert_float(v, "math.randomseed(1); return math.random(-3, 3)");
    }
}

/// The contrast: under 5.4 (integer number model) `random(m, n)` IS an `Int`.
///
/// This pins that the FloatOnly tests above are exercising the version GATE —
/// not silently passing because `math.random` happens never to build an `Int`
/// anywhere. If a refactor made 5.4's interval draw a Float, this fails.
#[test]
fn v54_random_interval_is_integer_subtype() {
    match eval_value(LuaVersion::V54, "math.randomseed(1); return math.random(5, 8)") {
        Value::Integer(_) => {}
        other => panic!(
            "5.4 random(5,8) should be an Int (Value::Integer), got {other:?}"
        ),
    }
}
