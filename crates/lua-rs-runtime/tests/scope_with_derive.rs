//! Proves that a type whose methods come from the `#[lua_methods]` macro works
//! unchanged as (a) an owned userdata, (b) a scope-borrowed userdata, and
//! (c) a delegated sub-userdata. The macro emits ordinary
//! `UserDataMethods::add_method{,_mut}` calls, and all three usages drive the
//! same `T::add_methods`, so none of them need macro-specific handling.
//!
//! Requires the `derive` feature:
//!   cargo test -p lua-rs-runtime --features derive --test scope_with_derive
#![cfg(feature = "derive")]

use lua_rs_runtime::{lua_methods, AnyUserData, Lua, LuaUserData, UserData, UserDataMethods};

/// Child type: all of its Lua methods are macro-generated.
#[derive(LuaUserData)]
#[lua(methods)]
struct Knob {
    setting: i64,
}

#[lua_methods]
impl Knob {
    pub fn turn(&mut self, by: i64) -> i64 {
        self.setting += by;
        self.setting
    }
    pub fn read(&self) -> i64 {
        self.setting
    }
}

/// Parent type, hand-written, exposing `panel:knob()` as a delegate. The
/// accessor must use `add_function` (it needs the receiver handle to build
/// the delegate); everything the delegate then dispatches is `Knob`'s
/// macro-generated methods.
struct Panel {
    knob: Knob,
}

impl UserData for Panel {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_function("knob", |lua, this: AnyUserData| {
            this.delegate::<Panel, Knob, _>(lua, |p| &mut p.knob)
        });
    }
}

/// (a) Owned: `Lua::create_userdata(Knob { .. })`.
#[test]
fn lua_methods_type_works_as_owned_userdata() {
    let lua = Lua::new();
    let knob = lua.create_userdata(Knob { setting: 10 }).unwrap();
    lua.globals().set("k", &knob).unwrap();
    let out: i64 = lua.load("k:turn(5); return k:read()").eval().unwrap();
    assert_eq!(out, 15);
}

/// (b) Borrowed: `Scope::create_userdata_ref_mut(&mut knob)`. Same macro
/// methods, and the mutation lands on the Rust-side value after the scope.
#[test]
fn lua_methods_type_works_as_scoped_userdata() {
    let lua = Lua::new();
    let mut knob = Knob { setting: 100 };
    let out: i64 = lua
        .scope(|s| {
            let ud = s.create_userdata_ref_mut(&lua, &mut knob)?;
            lua.globals().set("k", &ud)?;
            lua.load("k:turn(-30); return k:read()").eval()
        })
        .unwrap();
    assert_eq!(out, 70);
    assert_eq!(knob.setting, 70);
}

/// (c) Delegated: `panel:knob()` returns a sub-userdata whose methods are
/// `Knob`'s macro-generated ones, re-borrowed from the parent per call.
#[test]
fn lua_methods_type_works_as_delegate() {
    let lua = Lua::new();
    let mut panel = Panel {
        knob: Knob { setting: 0 },
    };
    let out: i64 = lua
        .scope(|s| {
            let ud = s.create_userdata_ref_mut(&lua, &mut panel)?;
            lua.globals().set("panel", &ud)?;
            lua.load(
                r#"
                local kn = panel:knob()
                kn:turn(3)
                kn:turn(4)
                return kn:read()
            "#,
            )
            .eval()
        })
        .unwrap();
    assert_eq!(out, 7);
    assert_eq!(panel.knob.setting, 7);
}

// ---------------------------------------------------------------------------
// Macro-generated delegates: a `#[lua_methods]` method that returns a
// reference auto-registers as an `add_function` + delegate, so the accessor
// no longer has to be hand-written.
// ---------------------------------------------------------------------------

/// A rig whose accessors are all macro-generated. `knob`/`nth` return
/// `&mut Knob` (mutable delegates); `knob_ro` returns `&Knob` (shared
/// delegate). The container fields are skipped so the derive doesn't try to
/// clone them as Lua fields.
#[derive(LuaUserData)]
#[lua(methods)]
struct Rig {
    #[lua(skip)]
    knob: Knob,
    #[lua(skip)]
    bank: Vec<Knob>,
}

#[lua_methods]
impl Rig {
    pub fn knob(&mut self) -> &mut Knob {
        &mut self.knob
    }
    pub fn knob_ro(&self) -> &Knob {
        &self.knob
    }
    pub fn nth(&mut self, i: i64) -> &mut Knob {
        &mut self.bank[i as usize]
    }
}

fn lua_err(lua: &Lua, body: &str) -> String {
    let (ok, msg): (bool, String) = lua
        .load(&format!(
            "local ok, e = pcall(function() {body} end); return ok, tostring(e)"
        ))
        .eval()
        .expect("pcall wrapper evaluates");
    assert!(!ok, "expected `{body}` to fail");
    msg
}

/// `fn knob(&mut self) -> &mut Knob` becomes a delegate: `rig:knob()` is a
/// live sub-reference, mutations persist to the Rust value.
#[test]
fn macro_mut_ref_method_becomes_delegate() {
    let lua = Lua::new();
    let mut rig = Rig {
        knob: Knob { setting: 0 },
        bank: vec![],
    };
    lua.scope(|s| {
        let ud = s.create_userdata_ref_mut(&lua, &mut rig)?;
        lua.globals().set("rig", &ud)?;
        lua.load("local k = rig:knob(); k:turn(5); k:turn(2)").exec()
    })
    .unwrap();
    assert_eq!(rig.knob.setting, 7);
}

/// `fn knob_ro(&self) -> &Knob` becomes a shared delegate: read methods work.
#[test]
fn macro_shared_ref_method_becomes_delegate() {
    let lua = Lua::new();
    let mut rig = Rig {
        knob: Knob { setting: 42 },
        bank: vec![],
    };
    let out: i64 = lua
        .scope(|s| {
            let ud = s.create_userdata_ref_mut(&lua, &mut rig)?;
            lua.globals().set("rig", &ud)?;
            lua.load("return rig:knob_ro():read()").eval()
        })
        .unwrap();
    assert_eq!(out, 42);
}

/// A delegate accessor that takes arguments: `rig:nth(2):turn(5)`.
#[test]
fn macro_delegate_method_with_args() {
    let lua = Lua::new();
    let mut rig = Rig {
        knob: Knob { setting: 0 },
        bank: vec![
            Knob { setting: 0 },
            Knob { setting: 0 },
            Knob { setting: 10 },
        ],
    };
    lua.scope(|s| {
        let ud = s.create_userdata_ref_mut(&lua, &mut rig)?;
        lua.globals().set("rig", &ud)?;
        lua.load("rig:nth(2):turn(5)").exec()
    })
    .unwrap();
    assert_eq!(rig.bank[2].setting, 15);
}

/// A shared (`&Knob`) delegate must reject a mutating child method.
#[test]
fn macro_shared_delegate_rejects_mut_method() {
    let lua = Lua::new();
    let mut rig = Rig {
        knob: Knob { setting: 1 },
        bank: vec![],
    };
    let msg = lua
        .scope(|s| {
            let ud = s.create_userdata_ref_mut(&lua, &mut rig)?;
            lua.globals().set("rig", &ud)?;
            Ok(lua_err(&lua, "rig:knob_ro():turn(1)"))
        })
        .unwrap();
    assert!(
        msg.contains("read-only") || msg.contains("mutating"),
        "expected read-only rejection, got: {msg}"
    );
    assert_eq!(rig.knob.setting, 1, "the value must be untouched");
}
