//! Table sequence helpers — issue #232.
//!
//! `push`/`insert`/`remove`/`pop`/`clear` mirror `table.insert`/`table.remove`;
//! the insert/remove cases assert parity against the stdlib functions.

use omnilua::{Lua, Value};

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
