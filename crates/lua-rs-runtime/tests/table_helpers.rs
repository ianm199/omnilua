//! Table sequence helpers + lazy iteration — issue #232.
//!
//! `push`/`insert`/`remove`/`pop`/`clear` mirror `table.insert`/`table.remove`;
//! the insert/remove cases assert parity against the stdlib functions.
//! `pairs`/`raw_pairs_iter` are the lazy iterators (no eager `Vec`), with
//! `__pairs` honored per version.

use omnilua::{Lua, LuaVersion, Table, Value};

const META_PAIRS_TABLE: &str = "return setmetatable({ a = 1, b = 2, c = 3 }, {
    __pairs = function(tbl) return function() return nil end, tbl, nil end
})";

fn int(v: Value) -> i64 {
    match v {
        Value::Integer(i) => i,
        other => panic!("expected integer, got {other:?}"),
    }
}

#[test]
fn push_appends() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    t.push(10i64).unwrap();
    t.push(20i64).unwrap();
    t.push(30i64).unwrap();

    assert_eq!(t.len().unwrap(), 3);
    let mid: i64 = t.raw_get(2).unwrap();
    assert_eq!(mid, 20);
}

#[test]
fn insert_matches_stdlib_table_insert() {
    let lua = Lua::new();
    let mine = lua.create_table().unwrap();
    for v in [1i64, 2, 3, 4] {
        mine.push(v).unwrap();
    }
    mine.insert(3, 100i64).unwrap();
    lua.globals().set("mine", mine).unwrap();

    lua.load(
        r#"
        local ref = {1, 2, 3, 4}
        table.insert(ref, 3, 100)
        assert(#mine == #ref, "length mismatch")
        for i = 1, #ref do assert(mine[i] == ref[i], "mismatch at "..i) end
    "#,
    )
    .exec()
    .unwrap();
}

#[test]
fn remove_matches_stdlib_table_remove() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    for v in [1i64, 2, 3, 4] {
        t.push(v).unwrap();
    }
    assert_eq!(int(t.remove(2).unwrap()), 2);
    lua.globals().set("t", t).unwrap();

    lua.load(
        r#"
        local ref = {1, 2, 3, 4}
        table.remove(ref, 2)
        assert(#t == #ref)
        for i = 1, #ref do assert(t[i] == ref[i], "mismatch at "..i) end
    "#,
    )
    .exec()
    .unwrap();
}

#[test]
fn pop_removes_last_then_empties() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    for v in [1i64, 2, 3] {
        t.push(v).unwrap();
    }
    assert_eq!(int(t.pop().unwrap()), 3);
    assert_eq!(int(t.pop().unwrap()), 2);
    assert_eq!(int(t.pop().unwrap()), 1);
    assert!(matches!(t.pop().unwrap(), Value::Nil));
    assert_eq!(t.len().unwrap(), 0);
}

#[test]
fn clear_empties_array_and_hash() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    for v in [1i64, 2, 3] {
        t.push(v).unwrap();
    }
    t.raw_set("k", "v").unwrap();

    t.clear().unwrap();

    assert_eq!(t.len().unwrap(), 0);
    assert!(matches!(t.raw_get::<_, Value>("k").unwrap(), Value::Nil));
}

#[test]
fn insert_out_of_bounds_errors() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    t.push(1i64).unwrap();
    assert!(t.insert(5, 9i64).is_err());
}

#[test]
fn lazy_pairs_iterates_every_entry() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    for i in 1..=1000i64 {
        t.push(i).unwrap();
    }

    let mut count = 0i64;
    let mut sum = 0i64;
    for pair in t.pairs().unwrap() {
        let (_k, v) = pair.unwrap();
        if let Value::Integer(n) = v {
            sum += n;
        }
        count += 1;
    }
    assert_eq!(count, 1000);
    assert_eq!(sum, 1000 * 1001 / 2);
}

#[test]
fn lazy_pairs_can_stop_early_without_materializing_all() {
    let lua = Lua::new();
    let t = lua.create_table().unwrap();
    for i in 1..=10_000i64 {
        t.push(i).unwrap();
    }

    let first_three: Vec<_> = t
        .pairs()
        .unwrap()
        .take(3)
        .map(|pair| pair.unwrap())
        .collect();
    assert_eq!(first_three.len(), 3);
}

#[test]
fn pairs_honors_metamethod_on_5_2_plus() {
    // __pairs was added in 5.2 and is still consulted by luaB_pairs through 5.5
    // (it was dropped from the manual, not the implementation). Verified
    // against every reference: pairs(t) with this immediate-nil custom iterator
    // yields zero pairs.
    for v in [
        LuaVersion::V52,
        LuaVersion::V53,
        LuaVersion::V54,
        LuaVersion::V55,
    ] {
        let lua = Lua::new_versioned(v);
        let t: Table = lua.load(META_PAIRS_TABLE).eval().unwrap();

        assert_eq!(
            t.pairs().unwrap().count(),
            0,
            "__pairs custom iterator should be honored on {v:?}"
        );
        assert_eq!(
            t.raw_pairs_iter().unwrap().count(),
            3,
            "raw_pairs_iter must ignore __pairs on {v:?}"
        );
    }
}

#[test]
fn pairs_ignores_metamethod_on_5_1() {
    // 5.1 has no __pairs metamethod, so pairs() iterates the real contents.
    let lua = Lua::new_versioned(LuaVersion::V51);
    let t: Table = lua.load(META_PAIRS_TABLE).eval().unwrap();
    assert_eq!(t.pairs().unwrap().count(), 3);
    assert_eq!(t.raw_pairs_iter().unwrap().count(), 3);
}
