//! Host→Lua number-model seam — issue #234 slice 1 (codex-reviewed, descoped).
//!
//! `lower_host_int` at the `to_raw_for_lua` choke point lowers a host i64 into the
//! active number model: integer on 5.3+, float on 5.1/5.2 (exact via the VM's
//! `int_fits_float`, else per `LossyIntPolicy`). Egress (`FromLua<i64>`) is
//! deliberately unchanged.

use omnilua::{LossyIntPolicy, Lua, LuaVersion, Value};

#[test]
fn host_int_lowers_to_float_in_5_1() {
    let lua = Lua::new_versioned(LuaVersion::V51);
    lua.globals().set("x", 42i64).unwrap();
    let v: Value = lua.globals().get("x").unwrap();
    assert!(
        matches!(v, Value::Number(n) if n == 42.0),
        "5.1 is float-only: a host i64 must become a float, got {v:?}"
    );
}

#[test]
fn host_int_stays_integer_in_5_4() {
    let lua = Lua::new();
    lua.globals().set("x", 42i64).unwrap();
    let v: Value = lua.globals().get("x").unwrap();
    assert!(matches!(v, Value::Integer(42)), "got {v:?}");
}

#[test]
fn exactness_under_error_policy_is_true_round_trip() {
    let lua = Lua::new_versioned(LuaVersion::V51);
    lua.set_lossy_int_policy(LossyIntPolicy::ErrorOnInexact);

    // Exactly representable — must be accepted (including powers of 2 above 2^53
    // and i64::MIN, which `int_fits_float`'s |i|<=2^53 range would wrongly reject).
    for exact in [1i64 << 53, 1i64 << 60, i64::MIN, 0, -1, 1_000_000] {
        lua.globals()
            .set("x", exact)
            .unwrap_or_else(|e| panic!("{exact} is exact but errored: {e}"));
    }

    // Not representable — must error, not saturate.
    for inexact in [(1i64 << 53) + 1, i64::MAX, i64::MAX - 1] {
        assert!(
            lua.globals().set("y", inexact).is_err(),
            "{inexact} is inexact and must error under ErrorOnInexact"
        );
    }
}

#[test]
fn widen_lossy_default_never_errors() {
    let lua = Lua::new_versioned(LuaVersion::V51);
    let inexact = (1i64 << 53) + 1;
    lua.globals().set("y", inexact).unwrap();
    let v: Value = lua.globals().get("y").unwrap();
    assert!(matches!(v, Value::Number(_)));
}

#[test]
fn marshal_from_respects_policy() {
    let v54 = Lua::new();
    let big = (1i64 << 53) + 1;

    let v51 = Lua::new_versioned(LuaVersion::V51);
    let w = v51.marshal_from(&v54, &Value::Integer(big)).unwrap();
    assert!(matches!(w, Value::Number(_)), "default WidenLossy preserves #235 widening");

    v51.set_lossy_int_policy(LossyIntPolicy::ErrorOnInexact);
    assert!(v51.marshal_from(&v54, &Value::Integer(big)).is_err());
}

#[test]
fn from_lua_i64_egress_unchanged() {
    let lua = Lua::new();
    let ok: i64 = lua.load("return 3.0").eval().unwrap();
    assert_eq!(ok, 3);
    assert!(lua.load("return 3.5").eval::<i64>().is_err());
}
