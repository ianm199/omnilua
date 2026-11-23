//! Host callbacks that allocate while running inside a coroutine.
//!
//! Each test keeps live objects in a coroutine's locals while a Rust callback
//! allocates enough to trigger collection. Every object must survive.
//! Run under `LUA_RS_GC_STRESS=1 LUA_RS_GC_QUARANTINE=1` for the strictest
//! check.

use omnilua::{Function, Lua, Thread};

const WORK: &str = r#"
    local function work()
        local keep = {}
        for i = 1, 200 do
            keep[i] = { tag = "k" .. i }
            alloc(200)
        end
        local n = 0
        for i = 1, 200 do
            assert(keep[i].tag == "k" .. i)
            n = n + 1
        end
        return n
    end
"#;

fn setup() -> Lua {
    let lua = Lua::new();
    let alloc = lua
        .create_function(|lua, n: i64| {
            for _ in 0..n {
                lua.create_table()?;
            }
            Ok(())
        })
        .unwrap();
    lua.globals().set("alloc", alloc).unwrap();
    lua
}

#[test]
fn main_thread() {
    let lua = setup();
    let n: i64 = lua.load(format!("{WORK} return work()")).eval().unwrap();
    assert_eq!(n, 200);
}

#[test]
fn lua_coroutine() {
    let lua = setup();
    let n: i64 = lua
        .load(format!("{WORK} return coroutine.wrap(work)()"))
        .eval()
        .unwrap();
    assert_eq!(n, 200);
}

#[test]
fn nested_lua_coroutines() {
    let lua = setup();
    let n: i64 = lua
        .load(format!(
            "{WORK} return coroutine.wrap(function() return coroutine.wrap(work)() end)()"
        ))
        .eval()
        .unwrap();
    assert_eq!(n, 200);
}

#[test]
fn host_thread_resume() {
    let lua = setup();
    let f: Function = lua.load(format!("{WORK} return work")).eval().unwrap();
    let thread: Thread = lua.create_thread(f).unwrap();
    let n: i64 = thread.resume(()).unwrap();
    assert_eq!(n, 200);
}

#[test]
fn yield_between_callbacks() {
    let lua = setup();
    let n: i64 = lua
        .load(
            r#"
            local co = coroutine.wrap(function()
                local keep = {}
                for i = 1, 100 do
                    keep[i] = { tag = "k" .. i }
                    alloc(100)
                    coroutine.yield()
                end
                for i = 1, 100 do assert(keep[i].tag == "k" .. i) end
                return 100
            end)
            local r
            repeat r = co(); alloc(50) until r
            return r
            "#,
        )
        .eval()
        .unwrap();
    assert_eq!(n, 100);
}
