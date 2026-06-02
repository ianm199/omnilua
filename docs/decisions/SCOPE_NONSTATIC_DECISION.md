# Non-`'static` userdata for `Lua::scope`: design background and decision

Design doc for a feature decision on **ianm199/lua-rs**. Written so a reader
who doesn't know the project, the players, or the prior discussion can weigh
in. Lots of links; everything claimed about other codebases is quoted from
source.

---

## 1. The cast

There are several "lua-rs"-named things floating around. Confusing on
purpose; clarifying first.

- **ianm199/lua-rs** — the project this doc is about. A pure-Rust Lua 5.4
  implementation (no C bindings). On `main` it's v0.0.9. Repo:
  https://github.com/ianm199/lua-rs.
- **CppCXY/lua-rs** — a *different* pure-Rust Lua implementation, by an
  unrelated author. Confusingly close name. Often referred to in their own
  docs as `luars`. Repo: https://github.com/CppCXY/lua-rs. Active.
- **mlua** — the long-standing Rust binding to C Lua / LuaJIT. Not
  pure-Rust; wraps the C runtime. Repo: https://github.com/mlua-rs/mlua.
  Treated here as the reference API shape because that's what Rust
  ecosystem users are used to.
- **bevy** — Rust ECS game engine. https://bevyengine.org/. Relevant
  because the user driving this request wants to script Bevy games in Lua.

The two pure-Rust Lua projects (ianm199/lua-rs and CppCXY/lua-rs) are *not*
forks of each other. They are independent implementations competing for the
same ecological niche: "what mlua gives you, but without the C dependency."

### The user driving this discussion: "Shatur"

GitHub: https://github.com/Shatur. Long-time Bevy ecosystem author. Ships
three popular Bevy crates:

- `bevy_replicon` (~609 stars) — Bevy networking / replication.
- `bevy_enhanced_input` (~274 stars) — Bevy input handling.
- `simgine` (~114 stars) — Bevy game framework.

He's "exploring options" for a generic crate that integrates a scripting
language with Bevy systems. He's a strong adoption signal for whichever
Lua runtime serves his needs cleanly — pulling him in unlocks the ~1,000
combined stars his crates touch.

His public messages, quoted further below, lay out the comparison he's
making. He's currently using mlua and wants to move to a pure-Rust option,
but neither pure-Rust option fully solves his case.

---

## 2. What "scope" means in this context

When Lua holds a Rust value as a "userdata" (Lua's name for opaque
host-language objects with methods), the binding usually wants to own that
value: `Lua::create_userdata(my_thing)` takes `T` by value and keeps it
alive for as long as Lua references it. That implies `T: 'static` — no
borrows from the Rust caller's stack frame can leak into Lua, because Lua
might use them after the caller returns.

That's restrictive when the Rust caller has a stack-local mutable borrow
it wants Lua to mutate *during* a call. Example: a Bevy "system" gets
`&mut World` for one frame. The caller doesn't own the World; the
scheduler does. If you want to run Lua scripts that mutate that World, you
have nowhere to put it.

mlua solved this in 2018 with the `Lua::scope` API. Shape:

```rust
lua.scope(|s| {
    // Hand Lua a borrow that is only valid inside this closure.
    let ud = s.create_userdata_ref_mut(&mut world)?;
    lua.globals().set("world", &ud)?;
    lua.load("world:spawn('player')").exec()
})?;
// scope ended; the borrow is back with the caller.
// Any Lua reference to `ud` that escaped the scope (via a global, etc.)
// fails on next use with a runtime error.
```

Two safety properties matter:

1. The Rust borrow lifetime is constrained by the closure body (HRTB
   `for<'scope>` machinery). Borrowed data can't be longer-lived than the
   scope.
2. If a Lua script squirrels the userdata away on a global and tries to
   use it on a later call, the userdata's metatable methods check an
   "invalidated" flag and return a clean Lua runtime error instead of
   reading freed memory.

It's the same shape `RefCell` gives you, but spanning the Rust/Lua FFI
boundary. mlua's docs page that introduces this:
https://docs.rs/mlua/latest/mlua/struct.Scope.html

---

## 3. The PR this doc is about

**lua-rs#27**: https://github.com/ianm199/lua-rs/pull/27 — "Scope API:
hand Lua a `&mut T` for a closure body (closes #26)."

Adds `Lua::scope` to ianm199/lua-rs in roughly mlua's shape. Diff
summary:

```rust
impl Lua {
    pub fn scope<F, R>(&self, f: F) -> Result<R>
    where F: for<'scope> FnOnce(&Scope<'scope>) -> Result<R>;
}

impl<'scope> Scope<'scope> {
    pub fn create_userdata_ref_mut<T: UserData>(&self, lua: &Lua, data: &'scope mut T)
        -> Result<AnyUserData>;
    pub fn create_function<A, R, F>(&self, lua: &Lua, func: F) -> Result<Function>
    where F: Fn(&Lua, A) -> Result<R> + 'scope;
    pub fn create_function_mut<A, R, F>(&self, lua: &Lua, func: F) -> Result<Function>
    where F: FnMut(&Lua, A) -> Result<R> + 'scope;
}

impl AnyUserData {
    pub fn scoped_borrow<T, R>(&self, f: impl FnOnce(&T) -> R) -> Result<R>;
    pub fn scoped_borrow_mut<T, R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R>;
}
```

61 passing tests; 4 unsafe blocks (NonNull deref + one lifetime transmute);
miri-clean on the unsafe surface. Working demo:
https://ianm199.github.io/bevy-lua-rs-starter/ — Bevy + the new scope API,
~135 LoC of game code.

The `UserData` trait in ianm199/lua-rs requires `T: 'static` (via
`TypeId`-keyed metatable cache and `Rc<dyn Any>` for the host payload).
That bound carries through to `Scope::create_userdata_ref_mut`, even
though the *borrow* of T can be non-`'static`.

---

## 4. The triggering request

The original issue: https://github.com/ianm199/lua-rs/issues/26.

Shatur's message in full:

> I'm a long-time Bevy user and maintain a few quite popular crates for
> it. And I'm planning to create a crate that provides a generic
> integration with a scripting language (that I'll also use for my game).
> I'm still exploring options. For the integration I planning I'll need
> #26. CppCXY/lua-rs (these names are getting confusing) supports the
> mentioned API, but still requires `'static` (CppCXY/lua-rs#44), which
> prevents me passing things like `Commands`, `Res`, etc that has
> lifetimes. And finally mlua supports the scope API and doesn't require
> `'static`, but it's bindings 😅

Three claims to verify against source:

1. CppCXY/lua-rs has `Scope` but requires `'static` on userdata.
2. mlua has `Scope` and *doesn't* require `'static`.
3. Therefore Bevy types like `Commands<'w, 's>` and `Res<'w, T>` (which
   carry lifetime parameters and are not `'static`) can be passed to
   mlua's scope but not to CppCXY's.

What he means by "passing `Commands`": in Bevy a "system" is a function
that gets dependencies injected by the scheduler. Standard signatures
look like:

```rust
fn my_system(mut commands: Commands, time: Res<Time>, query: Query<&mut Position>) {
    /* ... */
}
```

`Commands<'w, 's>`, `Res<'w, T>`, and `Query<'w, 's, ...>` all carry
lifetime parameters tying them to the scheduler's borrow on the World.
None of them are `'static`. If `create_userdata` requires `T: 'static`,
none of them can be passed to a Lua script as userdata. He'd have to
flatten each one into hand-written closure glue per type — fatal for a
*generic* crate that wants one trait impl per `SystemParam`.

What he wants the script side to look like:

```lua
cmd:spawn("player")
print(time:delta())
for entity, pos in query:iter() do ... end
```

i.e. handle-grouped method dispatch on Bevy params. Not a flat function
table.

---

## 5. Source-level state of the art

### 5a. CppCXY/lua-rs

Issue Shatur cited: https://github.com/CppCXY/lua-rs/issues/44 — "Don't
require `'static` for `TraitUserData`." **Closed, not implemented.**

CppCXY's actual trait, on `main` today
(`crates/luars/src/lua_value/userdata_trait.rs:154`):

```rust
pub trait UserDataTrait: 'static {
```

Scope's userdata method (`crates/luars/src/lua_api/scope.rs:448`):

```rust
pub fn create_userdata_ref<T: UserDataTrait>(
    &mut self,
    reference: &mut T,
) -> LuaResult<ScopedUserData<'scope, T>> {
```

`T: UserDataTrait` ⇒ `T: 'static`. So `Commands<'w, 's>` can't be passed.

The CppCXY maintainer's reasoning in the issue thread:

> Through a simple attempt, after losing `'static`, the compiler cannot
> store `data: Box<dyn UserDataTrait>` because it cannot prove its
> lifetime. If we really want to achieve this, `LuaUserdata` can only
> store pointers. […]
>
> The current `luauserdata` does not actually go through metatables,
> which allows for better performance. To implement this mechanism, I
> would basically have to use metatables just like mlua does.

So they deliberately chose perf (avoiding metatables) over the
non-`'static` case. Shatur's resulting workaround in his own code (from
the issue comments):

```rust
struct LifeTime<'a> { value: &'a str }

struct LifeTimeUserdata {
    ptr: *mut LifeTime<'static>,   // lifetime-erased raw pointer
}

#[lua_methods]
impl LifeTimeUserdata {
    pub fn get_str(&self) -> String {
        unsafe { (*self.ptr).value.to_string() }
    }
}
```

His own comment on this: "users will be able to create dangling references
with safe code. However, I can just make the construction unsafe with a
precondition that the reference shouldn't go out of scope." That is the
pattern ianm199/bevy-lua-rs-starter currently uses on top of #27 too
(`LuaWorld { ptr: NonNull<World> }`). It works but the safety story leaks
into every caller.

### 5b. mlua

mlua's `UserData` trait (`src/userdata.rs:686` on `main`):

```rust
pub trait UserData: Sized {
```

`Sized` only. No `'static`. This is the exact line Shatur is comparing
against when he says "mlua doesn't require `'static`."

mlua's `Scope` has **four** userdata constructors. From `src/scope.rs`:

```rust
// (1) by &T, T must be 'static
pub fn create_userdata_ref<T>(&'scope self, data: &'env T) -> Result<AnyUserData>
where T: UserData + 'static;

// (2) by &mut T, T must be 'static
pub fn create_userdata_ref_mut<T>(&'scope self, data: &'env mut T) -> Result<AnyUserData>
where T: UserData + 'static;

// (3) by VALUE, T can be non-'static
pub fn create_userdata<T>(&'scope self, data: T) -> Result<AnyUserData>
where T: UserData + 'env;

// (4) by VALUE, no UserData impl needed; pass a register function
pub fn create_any_userdata<T>(
    &'scope self,
    data: T,
    register: impl FnOnce(&mut UserDataRegistry<T>),
) -> Result<AnyUserData>
where T: 'env;
```

The doc comment on (3) names the tradeoff verbatim:

> The main limitation that comes from using non-`'static` userdata is
> that the produced userdata will no longer have a `TypeId` associated
> with it, because `TypeId` can only work for `'static` types. This means
> that it is impossible, once the userdata is created, to get a reference
> to it back *out* of an `AnyUserData` handle. […] Also, there is no way
> to re-use a single metatable for multiple non-`'static` types, so there
> is a higher cost associated with creating the userdata metatable each
> time a new userdata is created.

Two important properties of mlua's design:

- mlua's non-`'static` constructor (3) takes `T` **by value**, not by
  `&mut`. The scope owns T for its duration and drops it on exit. For
  Bevy this works: `Commands<'w, 's>` is a `SystemParam` that can be
  moved into the scope; deferred commands are stored in the World's
  `CommandQueue`, not in the `Commands` handle.
- (4) `create_any_userdata` takes a register function instead of
  requiring `impl UserData for T`. This is the **orphan-rule escape
  hatch**: in Rust you can only `impl Trait for Type` if you own Trait or
  Type. Shatur's generic Bevy-scripting crate owns neither `UserData`
  (it'd be ours) nor `bevy::Commands` (Bevy's), so without (4) he
  literally cannot expose `Commands` as a userdata even if our `T:
  'static` bound is relaxed. The register-fn pattern sidesteps the
  orphan rule by not requiring a trait impl at all.

---

## 6. The gap in lua-rs#27 today

| What you want to pass | mlua path | ianm199/lua-rs#27 | Workaround |
|---|---|---|---|
| `&mut World` (`'static` type, borrowed) | (2) `create_userdata_ref_mut` | ✓ `create_userdata_ref_mut` | none needed |
| `Commands<'w, 's>` (move in) | (3) `create_userdata` | ✗ | `create_function_mut` capturing `&mut commands` |
| `Res<'w, T>` (move in) | (3) `create_userdata` | ✗ | same |
| `Query<'w, 's, ...>` (move in) | (3) `create_userdata` | ✗ | same |
| Any foreign type with methods | (4) `create_any_userdata` | ✗ | newtype wrapper + impl UserData |

For one-off integrations the workaround column is fine — it's what mlua
users do too when they don't reach for the userdata path. For a *generic*
integration crate that wants one mechanism per `SystemParam`, it isn't:
every param has to be flattened into hand-written closure glue per type
instead of one trait impl per param.

Shatur's specific use case (the generic Bevy-scripting crate) needs (3)
*and* (4). Anything less than both pushes him back to mlua's C bindings
for the API ergonomics.

---

## 7. Three options

### A. Ship lua-rs#27 as-is

What lands: `Lua::scope` + `Scope::create_userdata_ref_mut` (with `T: 'static`)
+ `Scope::create_function*`. Closure workaround documented for Bevy
params.

- **Pros**
  - Shippable today; tested; miri-clean on the new unsafe surface.
  - Closes the immediate gap for "hand Lua a `&mut World`."
  - Self-contained, reviewable, ~1.5k LoC including tests.
- **Cons**
  - For a generic Bevy-scripting crate, weaker than mlua. The author has
    to hand-write closure glue per `SystemParam`.
  - The pitch "pure-Rust mlua" carries a real asterisk until the
    non-`'static` path exists.

### B. Add non-`'static` userdata + register-fn to lua-rs#27 before merging

What lands: A, plus mlua's (3) `create_userdata<T: UserData + 'env>` and
(4) `create_any_userdata<T: 'env>(data, register)`. Requires:

- Relax `UserData: 'static` to `UserData: Sized` (mlua's bound).
- Fork dispatch path: per-instance metatables for non-`'static` userdata
  (no `TypeId` cache); this is exactly the cost CppCXY refused to pay.
- Replace `host_value: Rc<dyn Any>` with a 3-variant enum (static-owned,
  scoped-borrow, scoped-owned).
- New `UserDataRegistry`-shape parameter for `create_any_userdata` so
  callers can register methods on foreign types without an impl.
- New tests including miri exercises for the additional unsafe.

- **Pros**
  - Full mlua-equivalent surface in one PR. "Pure-Rust mlua" without an
    asterisk.
  - Shatur's generic Bevy-scripting crate works on lua-rs first try.
- **Cons**
  - ~400-800 LoC on top of the current PR, plus tests. 1-2 more days.
  - **Designed without Shatur's input.** The trait split / register-fn
    shape is exactly the kind of API surface that wants real-user review
    before being locked in. mlua arrived at theirs after years of user
    iteration.
  - Delays the existing value of #27.
  - More API surface to maintain forever.

### C. Ship lua-rs#27, open a follow-up issue with this doc as the body

What lands now: A. Immediately after, open `lua-rs#28` titled "Non-
`'static` userdata in scope (mlua's `create_userdata` /
`create_any_userdata`)" with this doc inline and four explicit questions
for Shatur:

1. Is the closure-capture pattern enough for the first version of your
   generic crate, or is it a blocker?
2. If non-`'static` userdata is needed, do you want the UserData-trait
   path (your own types), the register-fn path (foreign types like
   `bevy::Commands`), or both? Which matters more first?
3. By value or by `&mut` borrow? mlua does by-value for the non-static
   case. Bevy `SystemParam`s can be moved. Is there a Bevy thing you'd
   want by `&mut` that isn't `'static`?
4. Are you OK with no `TypeId` downcast on the Rust side for non-static
   userdata, in exchange for per-instance metatable cost? (This is the
   mlua tradeoff and the one CppCXY refused.)

- **Pros**
  - Scope ships today. Concrete demo Shatur can run.
  - Follow-up is designed *with* the only customer for this feature, not
    for him.
  - Two reviewable PRs beat one big one.
  - Doesn't lock in a surface (trait split vs trait relax, register-fn
    signature, by-value vs by-ref) that might be wrong.
- **Cons**
  - Two PRs to land instead of one.
  - Follow-up might slip if priorities shift. (Mitigation: doc lives in
    the repo; issue stays open; the path is documented.)

---

## 8. Recommendation: **C**

lua-rs#27 as it stands is a real, self-contained win — `&mut World`
round-trips through Lua, the safety story is mechanical, miri's happy
with the new unsafe. Shipping today gets:

- A concrete artifact for the issue thread.
- Working integration on bevy-lua-rs-starter demonstrating the half of
  the API that *is* solved.
- A live demo Shatur can poke at:
  https://ianm199.github.io/bevy-lua-rs-starter/.

Bundling non-`'static` userdata into the same PR has three real problems:

1. **Design risk without input.** mlua has *two* non-`'static` paths
   (UserData-trait + register-fn) and they were arrived at after years
   of users telling them what they needed. Picking the shape for our
   variant without talking to Shatur is asking to redo the API later.
2. **Scope creep on a clean PR.** #27 is reviewable as-is; doubling its
   surface isn't.
3. **Time.** ~1-2 days of careful work to do B properly, vs. shipping
   today.

The follow-up issue should be opened on the same day #27 merges, so the
conversation with Shatur starts before he disengages. Link this doc as
the design background.

---

## 9. What the future implementation looks like (sketch for B)

For when B happens, regardless of timing. The shape, based on mlua's
structure:

### Trait change

```rust
// Before (today, ianm199/lua-rs):
pub trait UserData: 'static { /* ... */ }

// After (mirroring mlua):
pub trait UserData: Sized { /* ... */ }
```

The `'static` bound moves from the trait onto the static-path call sites
(`Lua::create_userdata`, the TypeId-keyed cache). The trait itself
becomes lifetime-neutral.

### New scope constructors

```rust
impl<'scope> Scope<'scope> {
    // Existing (unchanged):
    pub fn create_userdata_ref_mut<T: UserData>(&self, lua: &Lua, data: &'scope mut T)
        -> Result<AnyUserData>;

    // New (mlua's (3)):
    pub fn create_userdata_value<T>(&self, lua: &Lua, data: T) -> Result<AnyUserData>
    where T: UserData + 'scope;

    // New (mlua's (4)):
    pub fn create_any_userdata<T>(
        &self,
        lua: &Lua,
        data: T,
        register: impl FnOnce(&mut UserDataMethodRegistry<T>),
    ) -> Result<AnyUserData>
    where T: 'scope;
}
```

### Storage change

`host_value: Option<Rc<dyn Any>>` becomes a 3-variant enum:

```rust
enum HostValue {
    Static(Rc<dyn Any>),              // existing path: owned, TypeId-keyed
    ScopedBorrow(Rc<dyn ScopeInvalidate>),  // existing scope-borrow path
    ScopedOwned(Rc<dyn ScopeInvalidate>),   // new: by-value, drops T on invalidate
}
```

The new variant owns the T inside its cell; `invalidate()` takes the
`Option<T>` (drops the value, releases captured borrows) before any
caller can reach it.

### Metatable building

Currently keyed by `TypeId` for cache reuse. For `'static` types that
stays. For non-`'static` types we build a fresh metatable per userdata
instance, with closures bound `+ 'scope` instead of `+ 'static`.
Forked path lives in two new helpers on `Lua`.

### Additional unsafe

- One more lifetime transmute (same pattern as the existing
  `ScopedFnCell`) to launder a `+ 'scope` closure-bound to `+ 'static`
  so the resulting `Function` satisfies the existing `'static` callback
  type. Same safety story (invalidate before captured T drops).
- No additional raw deref beyond what `ScopedCell` and `ScopedFnCell`
  already do.

### LoC / test estimate

- ~400-600 LoC of impl on `crates/lua-rs-runtime/src/lib.rs`.
- ~200-300 LoC of tests (`scope_value_userdata_*` and
  `scope_any_userdata_*` clusters; more miri-clean cell tests).
- ~50 LoC integration test extending the existing
  `tests/scope_world_smoke.rs` with a non-static case (a `Commands`-
  shaped mock).

Total: ~700-900 LoC for a complete B follow-up. Well-scoped, lots of
prior art to copy from mlua's source.

---

## 10. Pointers and reading list

For a reviewer who wants to verify everything claimed above:

- The PR under discussion: https://github.com/ianm199/lua-rs/pull/27
- The triggering issue (Shatur's request): https://github.com/ianm199/lua-rs/issues/26
- mlua's `Scope` source: https://github.com/mlua-rs/mlua/blob/main/src/scope.rs
- mlua's `UserData` trait: https://github.com/mlua-rs/mlua/blob/main/src/userdata.rs#L686
- mlua's `Scope` rustdoc (best high-level intro): https://docs.rs/mlua/latest/mlua/struct.Scope.html
- CppCXY/lua-rs `UserDataTrait`: https://github.com/CppCXY/lua-rs/blob/main/crates/luars/src/lua_value/userdata_trait.rs#L154
- CppCXY/lua-rs scope: https://github.com/CppCXY/lua-rs/blob/main/crates/luars/src/lua_api/scope.rs
- CppCXY/lua-rs#44 (the closed `'static` issue with the same complaint): https://github.com/CppCXY/lua-rs/issues/44
- bevy-lua-rs-starter (working demo on top of #27): https://github.com/ianm199/bevy-lua-rs-starter, live at https://ianm199.github.io/bevy-lua-rs-starter/
- Bevy `SystemParam` (what `Commands`, `Res`, `Query` are): https://docs.rs/bevy/latest/bevy/ecs/system/trait.SystemParam.html

---

## 11. TL;DR

- Shatur's "passing `Commands`, `Res`" complaint is about two specific
  mlua methods that neither pure-Rust Lua implementation currently
  provides: `Scope::create_userdata<T: UserData + 'env>` (by-value, non-
  `'static`) and `Scope::create_any_userdata` (register-fn for foreign
  types).
- CppCXY/lua-rs closed their version of this issue as "won't fix"
  because adding it would force them to switch to metatables and tank
  the perf they're optimizing for.
- ianm199/lua-rs#27 ships the `'static`-only half of mlua's scope API.
  Closure workarounds exist for the rest but don't compose for a
  generic integration crate.
- Recommendation: ship #27 today, open a follow-up issue with this doc
  as the body, tag Shatur with four explicit design questions before
  implementing the rest.
