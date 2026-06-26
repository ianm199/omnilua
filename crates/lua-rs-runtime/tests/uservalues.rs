//! Userdata uservalues — issue #233.
//!
//! Slots are fixed at creation (`create_userdata_with_uservalues`); set/get drive
//! the existing GC-write-barriered `set_i_uservalue`/`get_i_uservalue`. Barrier
//! CORRECTNESS is gated by the GC canaries (`harness/canaries/gc/run_canaries.sh`,
//! incl. `canary_j_testc_sweep_uservalue_barrier.lua`), NOT by a full-collect test
//! here — a full collect marks `uv` via `Trace` regardless of the barrier, so it
//! would pass even if the barrier were removed. These tests cover functional
//! behavior + the bounds/index-safety the review flagged.

use omnilua::{Lua, LuaVersion, Table, UserData, Value};

struct Holder;
impl UserData for Holder {}

#[test]
fn set_get_round_trip_across_slots() {
    let lua = Lua::new();
    let ud = lua.create_userdata_with_uservalues(Holder, 2).unwrap();
    ud.set_user_value(1, "hello").unwrap();
    ud.set_user_value(2, 42i64).unwrap();

    let a: String = ud.user_value(1).unwrap();
    let b: i64 = ud.user_value(2).unwrap();
    assert_eq!(a, "hello");
    assert_eq!(b, 42);
}

#[test]
fn unset_slot_reads_nil() {
    let lua = Lua::new();
    let ud = lua.create_userdata_with_uservalues(Holder, 1).unwrap();
    let v: Value = ud.user_value(1).unwrap();
    assert!(matches!(v, Value::Nil));
}

#[test]
fn out_of_range_and_zero_index_error() {
    let lua = Lua::new();
    let ud = lua.create_userdata_with_uservalues(Holder, 1).unwrap();
    assert!(ud.set_user_value(2, 1i64).is_err(), "only 1 slot");
    assert!(ud.set_user_value(0, 1i64).is_err(), "1-based");
    assert!(ud.set_user_value(usize::MAX, 1i64).is_err(), "no i32 wrap");
}

#[test]
fn zero_slot_userdata_rejects_set() {
    let lua = Lua::new();
    let ud = lua.create_userdata(Holder).unwrap();
    assert!(ud.set_user_value(1, 1i64).is_err());
}

#[test]
fn oversized_slot_counts_error_not_oom() {
    let lua = Lua::new();
    // usize::MAX (unaddressable) AND a large-but-in-i32-range count (e.g. 1e9,
    // which would be tens of GB) must both Err — never attempt the allocation.
    assert!(lua.create_userdata_with_uservalues(Holder, usize::MAX).is_err());
    assert!(lua
        .create_userdata_with_uservalues(Holder, 1_000_000_000)
        .is_err());
    // a sane count just above the cap also errors cleanly.
    assert!(lua
        .create_userdata_with_uservalues(Holder, (u16::MAX as usize) + 1)
        .is_err());
}

#[test]
fn table_uservalue_retrievable_after_collection() {
    let lua = Lua::new();
    let ud = lua.create_userdata_with_uservalues(Holder, 1).unwrap();
    let t = lua.create_table().unwrap();
    t.raw_set("k", 7i64).unwrap();
    ud.set_user_value(1, t).unwrap();

    lua.gc_collect();

    let back: Table = ud.user_value(1).unwrap();
    let k: i64 = back.raw_get("k").unwrap();
    assert_eq!(k, 7);
}

#[test]
fn version_invariant() {
    for v in [LuaVersion::V51, LuaVersion::V54] {
        let lua = Lua::new_versioned(v);
        let ud = lua.create_userdata_with_uservalues(Holder, 1).unwrap();
        ud.set_user_value(1, "x").unwrap();
        let s: String = ud.user_value(1).unwrap();
        assert_eq!(s, "x", "{v:?}");
    }
}
