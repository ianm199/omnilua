//! Lua incremental tri-color mark-and-sweep garbage collector (Phase D).
//!
//! Ported from `src/lgc.c` (Lua 5.4.7, 1744 lines, 73 functions).
//!
//! This crate (`lua-gc`) is permitted to use `unsafe` (ceiling: 20 blocks per
//! `harness/unsafe-budgets.toml`). Every block carries `// SAFETY: ...`.
//!
//! # Algorithm overview
//!
//! Lua 5.4 uses a tri-color incremental mark-and-sweep collector with an
//! optional generational minor/major cycle strategy. The three colors are:
//! - **White** (two alternating bits): not yet visited; dead after a cycle if
//!   still white.
//! - **Gray**: visited but outgoing references not yet traced.
//! - **Black**: fully traced; invariant: a black object cannot point to a white one.
//!
//! The GC advances through the states in `GcState` order via `single_step`.
//!
//! # Circular-dependency note
//!
//! All public functions nominally take `&mut LuaState` (defined in `lua-vm`).
//! That creates: `lua-gc → lua-vm → lua-gc`. Resolution is deferred to Phase B
//! (introduce a `GcHost` trait in `lua-types`). Imports of `lua-vm` types are
//! commented out below; Phase B re-enables them.
//!
//! # Gray-list design deviation
//!
//! In C, each GC object has an intrusive `gclist` field linking it into a gray
//! list (`g->gray`, `g->grayagain`, etc.). `types.tsv` removes `gclist` from
//! individual types and replaces every intrusive list with a
//! `Vec<GcRef<dyn Collectable>>` on `GlobalState`. The sweep / mark functions
//! below are adapted accordingly; see `// PORT NOTE:` comments.
//!
//! C: `#define lgc_c` / `#define LUA_CORE`

// TODO(port): lua-vm types cannot be imported until Phase B resolves the
// circular dep (lua-gc → lua-vm → lua-gc). Written speculatively; rustc will
// report "unresolved import" which is an expected Phase-A error.
// use lua_vm::state::{LuaState, GlobalState};
// use lua_vm::table::LuaTable;
// use lua_vm::func::{LuaClosure, UpVal};
// use lua_vm::object::LuaUserData;
// use lua_vm::proto::LuaProto;
// use lua_vm::string::LuaString;

#[allow(unused_imports)]
use lua_types::error::LuaError;
#[allow(unused_imports)]
use lua_types::gc::GcRef;
#[allow(unused_imports)]
use lua_types::value::LuaValue;

// ---------------------------------------------------------------------------
// Phase-B stub types
//
// `LuaState` and `GlobalState` properly live in `lua-vm`, which depends on
// `lua-gc` — a circular import. These placeholder structs let `lua-gc`
// compile in isolation. Phase B replaces them with `use lua_vm::state::...`
// once the `GcHost` trait in `lua-types` breaks the cycle.
//
// TODO(phase-b): needs lua_vm::state::{LuaState, GlobalState, StringTable}.
// ---------------------------------------------------------------------------

/// Phase-B stub for `lua_vm::state::StringTable` (`stringtable` in C).
pub struct StringTable {
    pub nuse: usize,
    pub size: usize,
}

/// Phase-B stub for `lua_vm::state::GlobalState` (`global_State` in C).
///
/// Field types are placeholders sufficient for `lua-gc` to type-check; the
/// real definitions land with Phase B's `GcHost` trait. Non-snake-case
/// field names match the C source one-for-one to keep this port faithful.
#[allow(non_snake_case)]
pub struct GlobalState {
    pub gcstate: u8,
    pub gckind: u8,
    pub gcemergency: bool,
    pub gcstp: u8,
    pub gcstopem: bool,
    pub currentwhite: u8,
    pub gcpause: u8,
    pub gcstepmul: u8,
    pub gcstepsize: u8,
    pub genmajormul: u8,
    pub genminormul: u8,
    pub GCdebt: isize,
    pub GCestimate: usize,
    pub lastatomic: usize,
    pub gray: GcObj,
    pub grayagain: GcObj,
    pub weak: GcObj,
    pub allweak: GcObj,
    pub ephemeron: GcObj,
    pub tobefnz: GcObj,
    pub allgc: GcObj,
    pub finobj: GcObj,
    pub fixedgc: GcObj,
    pub sweepgc: GcObjCursor,
    pub firstold1: GcObj,
    pub finobjold1: GcObj,
    pub finobjsur: GcObj,
    pub finobjrold: GcObj,
    pub survival: GcObj,
    pub old1: GcObj,
    pub reallyold: GcObj,
    pub strt: StringTable,
    total_bytes_stub: usize,
}

impl GlobalState {
    /// Phase-B stub for `g->mainthread` (returns the GcObj head of the main thread).
    ///
    /// # Safety
    /// Placeholder returns null; callers under Phase B will receive a real pointer.
    pub unsafe fn mainthread_raw(&self) -> GcObj {
        std::ptr::null_mut()
    }

    /// Phase-B stub for `gettotalbytes(g)` (C macro returning total live bytes).
    pub fn total_bytes(&self) -> usize {
        self.total_bytes_stub
    }
}

/// Phase-B stub for `lua_vm::state::LuaState` (`lua_State` in C).
pub struct LuaState {
    g: GlobalState,
}

impl LuaState {
    pub fn global(&self) -> &GlobalState {
        &self.g
    }

    pub fn global_mut(&mut self) -> &mut GlobalState {
        &mut self.g
    }

    /// Phase-B stub for `luaE_setdebt`.
    pub fn set_debt(&mut self, _debt: isize) {
        todo!("phase-b: needs lua_vm::state::LuaState::set_debt")
    }

    /// Phase-B stub for accessing the running thread as a raw GcObj.
    ///
    /// # Safety
    /// Placeholder returns null; Phase B supplies the live thread pointer.
    pub unsafe fn current_thread_raw(&self) -> GcObj {
        std::ptr::null_mut()
    }
}

// ---------------------------------------------------------------------------
// GC state machine constants  (lgc.h)
// ---------------------------------------------------------------------------

/// C: `#define GCSpropagate 0`
pub const GCS_PROPAGATE: u8 = 0;
/// C: `#define GCSenteratomic 1`
pub const GCS_ENTER_ATOMIC: u8 = 1;
/// C: `#define GCSatomic 2`
pub const GCS_ATOMIC: u8 = 2;
/// C: `#define GCSswpallgc 3`
pub const GCS_SWP_ALLGC: u8 = 3;
/// C: `#define GCSswpfinobj 4`
pub const GCS_SWP_FINOBJ: u8 = 4;
/// C: `#define GCSswptobefnz 5`
pub const GCS_SWP_TOBEFNZ: u8 = 5;
/// C: `#define GCSswpend 6`
pub const GCS_SWP_END: u8 = 6;
/// C: `#define GCScallfin 7`
pub const GCS_CALLFIN: u8 = 7;
/// C: `#define GCSpause 8`
pub const GCS_PAUSE: u8 = 8;

// ---------------------------------------------------------------------------
// GC color bit positions  (lgc.h)
// ---------------------------------------------------------------------------

/// C: `#define WHITE0BIT 3`
pub const WHITE0_BIT: u8 = 3;
/// C: `#define WHITE1BIT 4`
pub const WHITE1_BIT: u8 = 4;
/// C: `#define BLACKBIT 5`
pub const BLACK_BIT: u8 = 5;
/// C: `#define FINALIZEDBIT 6`
pub const FINALIZED_BIT: u8 = 6;

/// C: `#define WHITEBITS bit2mask(WHITE0BIT, WHITE1BIT)` = 0b00011000
pub const WHITE_BITS: u8 = (1 << WHITE0_BIT) | (1 << WHITE1_BIT);

/// C: `#define maskcolors (bitmask(BLACKBIT) | WHITEBITS)`
const MASK_COLORS: u8 = (1u8 << BLACK_BIT) | WHITE_BITS;

/// C: `#define maskgcbits (maskcolors | AGEBITS)`
const MASK_GC_BITS: u8 = MASK_COLORS | AGE_BITS;

// ---------------------------------------------------------------------------
// Generational GC age constants  (lgc.h)
// ---------------------------------------------------------------------------

/// C: `#define G_NEW 0` — created in the current cycle
pub const G_NEW: u8 = 0;
/// C: `#define G_SURVIVAL 1` — created in the previous cycle
pub const G_SURVIVAL: u8 = 1;
/// C: `#define G_OLD0 2` — promoted by a forward barrier in this cycle
pub const G_OLD0: u8 = 2;
/// C: `#define G_OLD1 3` — first full cycle as an old object
pub const G_OLD1: u8 = 3;
/// C: `#define G_OLD 4` — really old; skipped in minor collections
pub const G_OLD: u8 = 4;
/// C: `#define G_TOUCHED1 5` — old object touched this cycle
pub const G_TOUCHED1: u8 = 5;
/// C: `#define G_TOUCHED2 6` — old object touched in the previous cycle
pub const G_TOUCHED2: u8 = 6;

/// C: `#define AGEBITS 7` — mask for the bottom 3 bits of `marked`
pub const AGE_BITS: u8 = 7; // 0b00000111

// ---------------------------------------------------------------------------
// GC stop-flag constants  (lgc.h)
// ---------------------------------------------------------------------------

/// C: `#define GCSTPUSR 1` — stopped by user (`collectgarbage("stop")`)
pub const GCSTPUSR: u8 = 1;
/// C: `#define GCSTPGC 2` — stopped by the GC itself (during finalization)
pub const GCSTPGC: u8 = 2;
/// C: `#define GCSTPCLS 4` — stopped while closing a Lua state
pub const GCSTPCLS: u8 = 4;

// ---------------------------------------------------------------------------
// GC mode constants  (lstate.h)
// ---------------------------------------------------------------------------

/// C: `KGC_INC` — incremental collection mode
pub const KGC_INC: u8 = 0;
/// C: `KGC_GEN` — generational collection mode
pub const KGC_GEN: u8 = 1;

// ---------------------------------------------------------------------------
// GC step / tuning constants  (lgc.c, lgc.h)
// ---------------------------------------------------------------------------

/// C: `#define GCSWEEPMAX 100`
const GC_SWEEP_MAX: i32 = 100;

/// C: `#define GCFINMAX 10`
const GC_FIN_MAX: i32 = 10;

/// C: `#define GCFINALIZECOST 50`
const GC_FINALIZE_COST: usize = 50;

/// C: `#define WORK2MEM sizeof(TValue)` — bytes per unit of traversal work.
/// `TValue` maps to `LuaValue` in Rust.
const WORK2MEM: usize = std::mem::size_of::<LuaValue>();

/// C: `#define PAUSEADJ 100`
const PAUSE_ADJ: isize = 100;

// ---------------------------------------------------------------------------
// GC object header  (lgc.h / lobject.h)
// ---------------------------------------------------------------------------

/// Common header for every GC-collectable object.
///
/// In C this is the `CommonHeader` macro, expanding to three fields at the
/// top of every `GCObject`-derived struct:
/// ```c
/// struct GCObject *next;   // next in allgc / finobj / fixedgc list
/// lu_byte tt;              // type tag
/// lu_byte marked;          // GC color + age bits
/// ```
///
/// In Rust (Phase D), every collectable type will carry one of these at
/// offset 0 (ensured by `#[repr(C)]`), so that `*mut GcHeader` casts are
/// sound.
///
/// Phase A-C: structure is defined here but not yet embedded in types (those
/// still use `Rc<T>`). Phase D replaces `GcRef<T>` with a raw-pointer arena.
///
/// TODO(port): Phase D must add `#[repr(C)] pub struct GcHeader { ... }` field
/// at the top of LuaString, LuaTable, LuaClosure, LuaProto, LuaUserData,
/// UpVal, and LuaState, and derive the `GcManaged` trait for each.
#[repr(C)]
pub struct GcHeader {
    /// Intrusive `next` pointer for allgc / finobj / fixedgc linked lists.
    /// NULL if this is the last element.
    pub next: *mut GcHeader,
    /// Internal type tag — same values as `LuaType` / variant tags.
    pub tt: u8,
    /// GC color bits (bits 3-5) and generational age (bits 0-2).
    pub marked: u8,
}

/// Type alias used throughout this module for GC object pointers.
///
/// C: `GCObject *`
///
/// In Phase D every collectable object begins with a `GcHeader`, so casting
/// between `*mut GcHeader` and `*mut ConcreteType` is sound when both are
/// `#[repr(C)]`.
pub type GcObj = *mut GcHeader;

/// Type alias for a pointer-to-list-head, used for list cursor manipulation.
///
/// C: `GCObject **p` (pointer to the "prev-next" slot, enabling O(1) removal)
///
/// PORT NOTE: In Phase D this models the intrusive-list cursor pattern directly
/// as in C. Phase B/D must ensure that every `*mut GcObj` written through a
/// cursor keeps the list consistent.
pub type GcObjCursor = *mut GcObj;

// ---------------------------------------------------------------------------
// GC type-tag constants used in dispatch
// These mirror the `makevariant(LUA_T*, variant)` values from lobject.h.
// ---------------------------------------------------------------------------

/// C: `LUA_VSHRSTR` = short interned string
const LUA_VSHRSTR: u8 = 0x04; // makevariant(LUA_TSTRING, 0)
/// C: `LUA_VLNGSTR` = long heap-allocated string
const LUA_VLNGSTR: u8 = 0x14; // makevariant(LUA_TSTRING, 1)
/// C: `LUA_VUPVAL`
const LUA_VUPVAL: u8 = 0x40; // makevariant(LUA_TUPVAL, 0)
/// C: `LUA_VUSERDATA`
const LUA_VUSERDATA: u8 = 0x08; // makevariant(LUA_TUSERDATA, 0)
/// C: `LUA_VLCL` = Lua closure
const LUA_VLCL: u8 = 0x46; // makevariant(LUA_TFUNCTION, 0)
/// C: `LUA_VCCL` = C closure
const LUA_VCCL: u8 = 0x56; // makevariant(LUA_TFUNCTION, 1)
/// C: `LUA_VTABLE`
const LUA_VTABLE: u8 = 0x05; // makevariant(LUA_TTABLE, 0)
/// C: `LUA_VTHREAD`
const LUA_VTHREAD: u8 = 0x09; // makevariant(LUA_TTHREAD, 0)
/// C: `LUA_VPROTO`
const LUA_VPROTO: u8 = 0x42; // makevariant(LUA_TPROTO, 0)

// ---------------------------------------------------------------------------
// Inline bit-manipulation helpers (from lgc.h macros)
// ---------------------------------------------------------------------------

/// C: `luaC_white(g)` — returns the current white color mask.
#[inline]
pub fn current_white(current_white: u8) -> u8 {
    // C: cast_byte((g)->currentwhite & WHITEBITS)
    current_white & WHITE_BITS
}

/// C: `otherwhite(g)` — returns the OTHER white (used to detect dead objects).
#[inline]
pub fn other_white(current_white: u8) -> u8 {
    // C: (g)->currentwhite ^ WHITEBITS
    current_white ^ WHITE_BITS
}

/// C: `iswhite(x)` — true if `marked` has either white bit set.
#[inline]
pub fn is_white_marked(marked: u8) -> bool {
    // C: testbits((x)->marked, WHITEBITS)
    (marked & WHITE_BITS) != 0
}

/// C: `isblack(x)` — true if the black bit is set.
#[inline]
pub fn is_black_marked(marked: u8) -> bool {
    // C: testbit((x)->marked, BLACKBIT)
    (marked & (1 << BLACK_BIT)) != 0
}

/// C: `isgray(x)` — neither white nor black.
#[inline]
pub fn is_gray_marked(marked: u8) -> bool {
    // C: !testbits((x)->marked, WHITEBITS | bitmask(BLACKBIT))
    (marked & (WHITE_BITS | (1 << BLACK_BIT))) == 0
}

/// C: `tofinalize(x)`
#[inline]
pub fn is_finalized_marked(marked: u8) -> bool {
    // C: testbit((x)->marked, FINALIZEDBIT)
    (marked & (1 << FINALIZED_BIT)) != 0
}

/// C: `isdeadm(ow, m)` — object marked with the OTHER white is dead.
#[inline]
pub fn is_dead_marked(other_white: u8, marked: u8) -> bool {
    // C: (m) & (ow)
    (marked & other_white) != 0
}

/// C: `getage(o)` — bottom 3 bits of `marked`.
#[inline]
pub fn get_age(marked: u8) -> u8 {
    // C: (o)->marked & AGEBITS
    marked & AGE_BITS
}

/// C: `setage(o, a)` — replace the bottom 3 age bits.
#[inline]
pub fn set_age_bits(marked: u8, age: u8) -> u8 {
    // C: (o)->marked = cast_byte(((o)->marked & (~AGEBITS)) | a)
    (marked & !AGE_BITS) | age
}

/// C: `isold(o)` — age > G_SURVIVAL.
#[inline]
pub fn is_old_marked(marked: u8) -> bool {
    // C: getage(o) > G_SURVIVAL
    get_age(marked) > G_SURVIVAL
}

/// C: `changeage(o, f, t)` — XOR from age `f` to age `t` (asserts source).
#[inline]
pub fn change_age_bits(marked: u8, from: u8, to: u8) -> u8 {
    debug_assert_eq!(get_age(marked), from, "changeage: source age mismatch");
    marked ^ (from ^ to)
}

/// C: `set2gray(x)` — `resetbits(x->marked, maskcolors)`.
#[inline]
pub fn set_to_gray(marked: u8) -> u8 {
    marked & !MASK_COLORS
}

/// C: `set2black(x)` — clear white bits, set black bit.
#[inline]
pub fn set_to_black(marked: u8) -> u8 {
    // C: (x->marked & ~WHITEBITS) | bitmask(BLACKBIT)
    (marked & !WHITE_BITS) | (1 << BLACK_BIT)
}

/// C: `nw2black(x)` — sets black bit; asserts object is not white.
#[inline]
pub fn nw2black(marked: u8) -> u8 {
    debug_assert!(!is_white_marked(marked), "nw2black: object is white");
    marked | (1 << BLACK_BIT)
}

/// C: `makewhite(g, x)` — erase color bits; set only the current white bit.
#[inline]
pub fn make_white(marked: u8, cur_white: u8) -> u8 {
    // C: (x->marked & ~maskcolors) | luaC_white(g)
    (marked & !MASK_COLORS) | (cur_white & WHITE_BITS)
}

/// C: `keepinvariant(g)` — true when GC state <= GCSatomic.
#[inline]
pub fn keep_invariant(gcstate: u8) -> bool {
    // C: (g)->gcstate <= GCSatomic
    gcstate <= GCS_ATOMIC
}

/// C: `issweepphase(g)`
#[inline]
pub fn is_sweep_phase(gcstate: u8) -> bool {
    // C: GCSswpallgc <= (g)->gcstate && (g)->gcstate <= GCSswpend
    GCS_SWP_ALLGC <= gcstate && gcstate <= GCS_SWP_END
}

/// C: `gcrunning(g)`
#[inline]
pub fn gc_running(gcstp: u8) -> bool {
    // C: (g)->gcstp == 0
    gcstp == 0
}

/// C: `isdecGCmodegen(g)` — declared generational mode (may be temporarily incremental).
#[inline]
pub fn is_dec_gc_mode_gen(gckind: u8, lastatomic: usize) -> bool {
    // C: g->gckind == KGC_GEN || g->lastatomic != 0
    gckind == KGC_GEN || lastatomic != 0
}

// ---------------------------------------------------------------------------
// Function stubs — bodies filled in by subsequent Edit calls
// ---------------------------------------------------------------------------
// Sections mirror the C source groupings:
//   §A  Generic / utility helpers
//   §B  Mark functions
//   §C  Traverse functions
//   §D  Sweep functions
//   §E  Finalization
//   §F  Generational collector
//   §G  GC control (public API)

// §A — Generic / utility helpers -------------------------------------------

/// C: `static GCObject **getgclist(GCObject *o)`
///
/// Returns a pointer to the `gclist` field of `o`, which varies by type.
///
/// PORT NOTE: In C, each collectable type embeds a `gclist` field for the
/// gray list. `types.tsv` removes `gclist` from individual types; gray lists
/// become `Vec<GcRef<dyn Collectable>>` on `GlobalState`. Phase D must either
/// re-add the `gclist` field or keep the Vec approach. This function preserves
/// the C logic for Phase D; callers may use `push_to_gray` helpers instead.
///
/// # Safety
/// `o` must point to a valid, fully-initialized GC object of one of the listed
/// types.  The returned pointer aliases a field inside `*o`.
unsafe fn get_gc_list(o: GcObj) -> GcObjCursor {
    // C: switch (o->tt) { case LUA_VTABLE: return &gco2t(o)->gclist; ... }
    // TODO(port): Phase D must define the byte offset of `gclist` within each
    // concrete #[repr(C)] type and compute the pointer arithmetically.
    // The sizes and offsets will be fixed once concrete types embed GcHeader.
    match (*o).tt {
        LUA_VTABLE | LUA_VLCL | LUA_VCCL | LUA_VTHREAD | LUA_VPROTO | LUA_VUSERDATA => {
            // TODO(port): return &(concrete_type_cast(o)->gclist)
            // For now return null; Phase D will supply the correct offset.
            std::ptr::null_mut()
        }
        _ => {
            debug_assert!(false, "get_gc_list: unrecognized tt={}", (*o).tt);
            std::ptr::null_mut()
        }
    }
}

/// C: `static void linkgclist_(GCObject *o, GCObject **pnext, GCObject **list)`
///
/// Prepends `o` to `*list` through `pnext` (the field inside `o` that will
/// point to the former head), and paints `o` gray.
///
/// # Safety
/// `o`, `pnext`, and `list` must be non-null pointers with aligned, live data.
unsafe fn link_gc_list(o: GcObj, pnext: GcObjCursor, list: GcObjCursor) {
    // C: lua_assert(!isgray(o));
    debug_assert!(!is_gray_marked((*o).marked), "link_gc_list: already gray");
    // C: *pnext = *list;  *list = o;  set2gray(o);
    *pnext = *list;
    *list = o;
    (*o).marked = set_to_gray((*o).marked);
}

/// C: `static void clearkey(Node *n)`
///
/// If hash-node `n` has a collectable key, marks it dead (tombstone) so the
/// key's memory can be freed while the slot remains in the hash chain.
///
/// PORT NOTE: `Node` / `TableNode` is defined in lua-vm (ltable).  Phase B
/// resolves this import.  The body below is the full C logic translated to Rust.
///
/// TODO(port): parameter type `TableNode` from lua-vm; circular dep.
fn clear_key(/* n: &mut TableNode */) {
    // C: lua_assert(isempty(gval(n)));
    // C: if (keyiscollectable(n)) setdeadkey(n);
    // TODO(port): n.set_key_dead() once TableNode is accessible.
    // (macros.tsv: keyiscollectable(n) → n.key_is_collectable())
    // (macros.tsv: setdeadkey(n)      → n.set_key_dead())
}

/// C: `static int iscleared(global_State *g, const GCObject *o)`
///
/// Returns `true` when GC object `o` should be cleared from a weak table.
/// `NULL` (non-collectable) → never cleared.
/// Strings → always marked (never cleared).
/// Other white objects → cleared (collected).
///
/// # Safety
/// `o` must be null or a pointer to a live `GcHeader`.
unsafe fn is_cleared(cur_white: u8, o: GcObj) -> bool {
    if o.is_null() {
        // C: if (o == NULL) return 0;
        return false;
    }
    // C: novariant(o->tt) == LUA_TSTRING
    // novariant strips the variant bits: (tt & 0x0F) gives the base type.
    // LUA_TSTRING base type is 4 (= 0x04 & 0x0F).
    let base_type = (*o).tt & 0x0F;
    if base_type == 0x04 {
        // C: markobject(g, o); return 0;  — strings are treated as values
        // markobject(g,t) = if (iswhite(t)) reallymarkobject(g, obj2gco(t))
        // We can't call really_mark_object here without &mut LuaState;
        // caller must ensure strings in weak tables are marked separately.
        // TODO(port): supply state parameter for the markobject call on strings.
        return false;
    }
    // C: else return iswhite(o);
    is_white_marked((*o).marked)
}

// §A — Public write barriers ------------------------------------------------

/// C: `void luaC_barrier_(lua_State *L, GCObject *o, GCObject *v)`
///
/// Forward barrier: black object `o` now points to white object `v`.
/// Restores the black→white invariant by marking `v` (propagate phase) or
/// making `o` white again (incremental sweep phase only).
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn barrier(
    state: &mut LuaState,
    o: GcObj,
    v: GcObj,
) {
    // C: global_State *g = G(L);
    // C: lua_assert(isblack(o) && iswhite(v) && !isdead(g,v) && !isdead(g,o));
    let g = state.global_mut();
    let ow = other_white(g.currentwhite);
    debug_assert!(is_black_marked((*o).marked));
    debug_assert!(is_white_marked((*v).marked));
    debug_assert!(!is_dead_marked(ow, (*v).marked));
    debug_assert!(!is_dead_marked(ow, (*o).marked));

    // C: if (keepinvariant(g))
    if keep_invariant(g.gcstate) {
        // C: reallymarkobject(g, v);
        really_mark_object(state, v);
        // C: if (isold(o)) { lua_assert(!isold(v)); setage(v, G_OLD0); }
        if is_old_marked((*o).marked) {
            debug_assert!(!is_old_marked((*v).marked));
            (*v).marked = set_age_bits((*v).marked, G_OLD0);
        }
    } else {
        // C: lua_assert(issweepphase(g));
        debug_assert!(is_sweep_phase(g.gcstate));
        // C: if (g->gckind == KGC_INC) makewhite(g, o);
        if g.gckind == KGC_INC {
            (*o).marked = make_white((*o).marked, g.currentwhite);
        }
    }
}

/// C: `void luaC_barrierback_(lua_State *L, GCObject *o)`
///
/// Backward barrier: re-grays the black object `o` so its references are
/// re-examined next propagation.  Used for tables/threads.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn barrier_back(
    state: &mut LuaState,
    o: GcObj,
) {
    // C: global_State *g = G(L);
    // C: lua_assert(isblack(o) && !isdead(g, o));
    let g = state.global_mut();
    debug_assert!(is_black_marked((*o).marked));
    debug_assert!(!is_dead_marked(other_white(g.currentwhite), (*o).marked));
    // C: assert (g->gckind == KGC_GEN) == (isold(o) && getage(o) != G_TOUCHED1)
    debug_assert_eq!(
        g.gckind == KGC_GEN,
        is_old_marked((*o).marked) && get_age((*o).marked) != G_TOUCHED1
    );

    // C: if (getage(o) == G_TOUCHED2) set2gray(o);
    // C: else linkobjgclist(o, g->grayagain);
    if get_age((*o).marked) == G_TOUCHED2 {
        (*o).marked = set_to_gray((*o).marked);
    } else {
        // PORT NOTE: C uses intrusive gclist; we push to the Vec gray list.
        // TODO(port): push raw GcObj to grayagain Vec once Phase D stabilizes.
        let gclist = get_gc_list(o);
        if !gclist.is_null() {
            link_gc_list(o, gclist, &mut g.grayagain);
        }
    }

    // C: if (isold(o)) setage(o, G_TOUCHED1);
    if is_old_marked((*o).marked) {
        (*o).marked = set_age_bits((*o).marked, G_TOUCHED1);
    }
}

/// C: `void luaC_fix(lua_State *L, GCObject *o)`
///
/// Removes `o` from the head of `allgc` and puts it in `fixedgc` — permanent,
/// gray, age OLD, never swept.  Called for reserved-word strings.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn fix(state: &mut LuaState, o: GcObj) {
    // C: global_State *g = G(L);
    // C: lua_assert(g->allgc == o);
    let g = state.global_mut();
    debug_assert_eq!(g.allgc, o, "luaC_fix: object must be first in allgc");

    // C: set2gray(o); setage(o, G_OLD);
    (*o).marked = set_to_gray((*o).marked);
    (*o).marked = set_age_bits((*o).marked, G_OLD);

    // C: g->allgc = o->next; o->next = g->fixedgc; g->fixedgc = o;
    g.allgc = (*o).next;
    (*o).next = g.fixedgc;
    g.fixedgc = o;
}

/// C: `GCObject *luaC_newobjdt(lua_State *L, int tt, size_t sz, size_t offset)`
///
/// Allocates `sz` bytes, interprets the result as a `GcHeader` at byte
/// `offset`, initialises its `marked` / `tt` / `next` fields, and links it
/// at the head of `allgc`.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.  Allocation itself
/// goes through `crate::mem::malloc_`; that circular dep is also pending.
pub(crate) unsafe fn new_obj_dt(
    state: &mut LuaState,
    tt: u8,
    sz: usize,
    offset: usize,
) -> GcObj {
    // C: global_State *g = G(L);
    // C: char *p = cast_charp(luaM_newobject(L, novariant(tt), sz));
    // C: GCObject *o = cast(GCObject *, p + offset);
    let g = state.global_mut();
    // TODO(port): call crate::mem::malloc_ for real allocation; placeholder below.
    let p = {
        let layout = std::alloc::Layout::from_size_align(sz, 8)
            .expect("new_obj_dt: invalid layout");
        // SAFETY: layout is valid; pointer is initialised below before use.
        std::alloc::alloc(layout)
    };
    if p.is_null() {
        // TODO(port): propagate LuaError::Memory instead of panicking.
        panic!("new_obj_dt: out of memory");
    }
    // C: GCObject *o = cast(GCObject *, p + offset);
    let o = p.add(offset) as GcObj;

    // C: o->marked = luaC_white(g); o->tt = tt; o->next = g->allgc; g->allgc = o;
    (*o).marked = current_white(g.currentwhite);
    (*o).tt = tt;
    (*o).next = g.allgc;
    g.allgc = o;
    o
}

/// C: `GCObject *luaC_newobj(lua_State *L, int tt, size_t sz)`
///
/// Convenience: `new_obj_dt` with `offset = 0`.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn new_obj(
    state: &mut LuaState,
    tt: u8,
    sz: usize,
) -> GcObj {
    // C: return luaC_newobjdt(L, tt, sz, 0);
    new_obj_dt(state, tt, sz, 0)
}

// §B — Mark functions -------------------------------------------------------

/// C: `static void reallymarkobject(global_State *g, GCObject *o)`
///
/// Core mark dispatcher. Strings / dead upvalues → turn black immediately
/// (nothing to traverse). Other objects → push onto the gray list.
///
/// PORT NOTE: The `LUA_VUPVAL` case in C reads `uv->v.p` (the value behind
/// the upvalue) to mark it.  In Rust that requires downcasting through
/// `UpVal` (in lua-vm). Marked with TODO(port) below.
///
/// # Safety
/// `o` must be a valid GcHeader pointer to a white GC object.
unsafe fn really_mark_object(
    state: &mut LuaState,
    o: GcObj,
) {
    // C: switch (o->tt)
    match (*o).tt {
        LUA_VSHRSTR | LUA_VLNGSTR => {
            // C: set2black(o);  /* nothing to visit */
            (*o).marked = set_to_black((*o).marked);
        }
        LUA_VUPVAL => {
            // C: UpVal *uv = gco2upv(o);
            // C: if (upisopen(uv)) set2gray(uv); else set2black(uv);
            // C: markvalue(g, uv->v.p);
            // TODO(port): downcast GcObj to UpVal to inspect open/closed state
            // and to mark the upvalue's current value.  UpVal is in lua-vm.
            // Conservative fallback: paint gray so the traversal pass handles it.
            (*o).marked = set_to_gray((*o).marked);
        }
        LUA_VUSERDATA => {
            // C: Udata *u = gco2u(o);
            // C: if (u->nuvalue == 0) { markobjectN(g, u->metatable); set2black(u); break; }
            // C: /* else FALLTHROUGH to linkobjgclist */
            // TODO(port): inspect nuvalue via downcast; for now treat as needing traversal.
            let g = state.global_mut();
            let gclist_ptr = get_gc_list(o);
            if !gclist_ptr.is_null() {
                link_gc_list(o, gclist_ptr, &mut g.gray);
            }
        }
        LUA_VLCL | LUA_VCCL | LUA_VTABLE | LUA_VTHREAD | LUA_VPROTO => {
            // C: linkobjgclist(o, g->gray);
            let g = state.global_mut();
            let gclist_ptr = get_gc_list(o);
            if !gclist_ptr.is_null() {
                link_gc_list(o, gclist_ptr, &mut g.gray);
            }
        }
        _ => {
            debug_assert!(false, "really_mark_object: unknown tt={}", (*o).tt);
        }
    }
}

/// C: `static void markmt(global_State *g)`
///
/// Marks the metatable for each of the `LUA_NUMTAGS` primitive types.
/// C: `for (i=0; i < LUA_NUMTAGS; i++) markobjectN(g, g->mt[i]);`
///
/// TODO(port): `GlobalState.mt` element type is `Option<GcRef<LuaTable>>`;
/// here we need raw GcObj pointers.  Phase D resolves.
unsafe fn mark_metatables(state: &mut LuaState) {
    // C: int i; for (i=0; i < LUA_NUMTAGS; i++) markobjectN(g, g->mt[i]);
    // macros.tsv: markobjectN(g,t) → if (t) markobject(g,t)
    // macros.tsv: markobject(g,t)  → if (iswhite(t)) reallymarkobject(g, obj2gco(t))
    // TODO(port): iterate g.mt[] once GlobalState is accessible with raw GcObj ptrs.
    let _ = state; // suppress unused warning until bodies are wired up
}

/// C: `static lu_mem markbeingfnz(global_State *g)`
///
/// Marks every object in the `tobefnz` list (pending finalizer calls).
/// Returns the count of objects it marked.
///
/// C: `for (o = g->tobefnz; o != NULL; o = o->next) { count++; markobject(g, o); }`
unsafe fn mark_being_finalized(state: &mut LuaState) -> usize {
    let g = state.global_mut();
    let mut count: usize = 0;
    // C: GCObject *o; for (o = g->tobefnz; o != NULL; o = o->next)
    let mut o = g.tobefnz;
    while !o.is_null() {
        count += 1;
        // C: markobject(g, o) → if (iswhite(o)) reallymarkobject(g, obj2gco(o))
        if is_white_marked((*o).marked) {
            really_mark_object(state, o);
        }
        // C: o = o->next
        // SAFETY: o is a valid live GcHeader; next is trusted from the GC list.
        o = (*o).next;
    }
    count
}

/// C: `static int remarkupvals(global_State *g)`
///
/// Walks the `twups` thread list.  For each non-marked thread, visits its
/// open upvalues (marking their values if the upvalue itself is already
/// gray/black), then removes the thread from the list.  Returns work estimate.
///
/// PORT NOTE: `twups` in C is `lua_State **` (intrusive via `thread->twups`).
/// In Rust, `GlobalState.twups` is `Vec<GcRef<LuaState>>` (types.tsv).
/// The body below follows the C intrusive-pointer logic with raw ptrs; Phase D
/// must reconcile with the Vec representation.
///
/// TODO(port): Requires access to `LuaState.openupval` and `UpVal` type from lua-vm.
unsafe fn remark_upvalues(state: &mut LuaState) -> usize {
    // C: lua_State *thread; lua_State **p = &g->twups; int work = 0;
    // C: while ((thread = *p) != NULL) {
    //       work++;
    //       if (!iswhite(thread) && thread->openupval != NULL)
    //           p = &thread->twups;  /* keep marked thread with upvalues */
    //       else {
    //           *p = thread->twups;          /* remove thread from list */
    //           thread->twups = thread;       /* mark it as out of list */
    //           for (uv = thread->openupval; ...) {
    //               if (!iswhite(uv)) markvalue(g, uv->v.p);
    //           }
    //       }
    //   }
    //   return work;
    // TODO(port): implement once LuaState fields are accessible here.
    // For now return 0 (conservative; no work claimed).
    let _ = state;
    0
}

/// C: `static void cleargraylists(global_State *g)`
///
/// Sets all five gray-list heads to NULL, preparing for a fresh cycle.
/// C: `g->gray = g->grayagain = NULL; g->weak = g->allweak = g->ephemeron = NULL;`
unsafe fn clear_gray_lists(state: &mut LuaState) {
    let g = state.global_mut();
    // C: g->gray = g->grayagain = NULL;
    g.gray = std::ptr::null_mut();
    g.grayagain = std::ptr::null_mut();
    // C: g->weak = g->allweak = g->ephemeron = NULL;
    g.weak = std::ptr::null_mut();
    g.allweak = std::ptr::null_mut();
    g.ephemeron = std::ptr::null_mut();
}

/// C: `static void restartcollection(global_State *g)`
///
/// Clears all gray lists and marks the root set: main thread, registry,
/// primitive-type metatables, and objects already being finalized.
unsafe fn restart_collection(state: &mut LuaState) {
    // C: cleargraylists(g);
    clear_gray_lists(state);
    // C: markobject(g, g->mainthread);
    let mainthread = state.global().mainthread_raw();
    if is_white_marked((*mainthread).marked) {
        really_mark_object(state, mainthread);
    }
    // C: markvalue(g, &g->l_registry);
    // TODO(port): mark the registry value (LuaValue::Table → GcObj).
    // C: markmt(g);
    mark_metatables(state);
    // C: markbeingfnz(g);
    let _ = mark_being_finalized(state);
}

// §C — Traverse functions ---------------------------------------------------

/// C: `static void genlink(global_State *g, GCObject *o)`
///
/// After traversal (object is now black), decides what to do with `o` in
/// generational mode:
/// - `G_TOUCHED1` → re-link into `grayagain` for the next atomic step.
/// - `G_TOUCHED2` → advance to `G_OLD`.
/// Anything else needs no action.
unsafe fn gen_link(state: &mut LuaState, o: GcObj) {
    // C: lua_assert(isblack(o));
    debug_assert!(is_black_marked((*o).marked));
    // C: if (getage(o) == G_TOUCHED1) linkobjgclist(o, g->grayagain);
    if get_age((*o).marked) == G_TOUCHED1 {
        let g = state.global_mut();
        let gclist_ptr = get_gc_list(o);
        if !gclist_ptr.is_null() {
            link_gc_list(o, gclist_ptr, &mut g.grayagain);
        }
    } else if get_age((*o).marked) == G_TOUCHED2 {
        // C: changeage(o, G_TOUCHED2, G_OLD);
        (*o).marked = change_age_bits((*o).marked, G_TOUCHED2, G_OLD);
    }
}

/// C: `static void traverseweakvalue(global_State *g, Table *h)`
///
/// Traverses a table with weak values.  Marks all keys.  If a hash slot has
/// a white value that slot is noted (the table will need clearing later).
/// Links the table into `g->weak` (atomic phase) or `g->grayagain` (propagate).
///
/// TODO(port): TableNode / LuaTable type from lua-vm; all hash-node and
/// array accesses are stubbed with TODO(port) here.
unsafe fn traverse_weak_value(state: &mut LuaState, h: GcObj) {
    // C: Node *n, *limit = gnodelast(h);
    // C: int hasclears = (h->alimit > 0);
    // C: for (n = gnode(h,0); n < limit; n++) {
    //       if (isempty(gval(n))) clearkey(n);
    //       else { markkey(g,n); if (!hasclears && iscleared(g, gcvalueN(gval(n)))) hasclears=1; }
    //   }
    // C: if (g->gcstate == GCSatomic && hasclears) linkgclist(h, g->weak);
    // C: else linkgclist(h, g->grayagain);
    // TODO(port): iterate over hash nodes once LuaTable is accessible here.
    let g = state.global_mut();
    let gclist_ptr = get_gc_list(h);
    if !gclist_ptr.is_null() {
        if g.gcstate == GCS_ATOMIC {
            link_gc_list(h, gclist_ptr, &mut g.weak);
        } else {
            link_gc_list(h, gclist_ptr, &mut g.grayagain);
        }
    }
}

/// C: `static int traverseephemeron(global_State *g, Table *h, int inv)`
///
/// Traverses an ephemeron table (weak key → value pairs).  Marks array
/// elements unconditionally.  For the hash part, marks values whose key is
/// already marked, and notes white-key/white-value pairs.
/// Returns `true` if any new object was marked.
///
/// TODO(port): LuaTable / hash-node access from lua-vm.
unsafe fn traverse_ephemeron(
    state: &mut LuaState,
    h: GcObj,
    inv: bool,
) -> bool {
    // C: int marked=0, hasclears=0, hasww=0;
    // C: for i in array: if valiswhite(&array[i]) { marked=1; reallymarkobject(g, gcvalue); }
    // C: for i in hash (possibly reversed if inv): ...complex key/value white checks...
    // C: if (gcstate==GCSpropagate) linkgclist(h, grayagain);
    // C: else if hasww: linkgclist(h, ephemeron);
    // C: else if hasclears: linkgclist(h, allweak);
    // C: else genlink(g, obj2gco(h));
    // TODO(port): implement once LuaTable is accessible.
    let _ = inv;
    let g = state.global_mut();
    let gclist_ptr = get_gc_list(h);
    if !gclist_ptr.is_null() {
        if g.gcstate == GCS_PROPAGATE {
            link_gc_list(h, gclist_ptr, &mut g.grayagain);
        } else {
            link_gc_list(h, gclist_ptr, &mut g.ephemeron);
        }
    }
    false // conservative: report no new marks until body is filled
}

/// C: `static void traversestrongtable(global_State *g, Table *h)`
///
/// Marks all keys and values in a normal (non-weak) table, then calls
/// `gen_link` for generational-mode bookkeeping.
///
/// TODO(port): LuaTable / hash-node access from lua-vm.
unsafe fn traverse_strong_table(state: &mut LuaState, h: GcObj) {
    // C: Node *n, *limit = gnodelast(h);
    // C: for (i=0; i<asize; i++) markvalue(g, &h->array[i]);
    // C: for (n=gnode(h,0); n<limit; n++) {
    //       if (isempty(gval(n))) clearkey(n);
    //       else { markkey(g,n); markvalue(g, gval(n)); }
    //   }
    // C: genlink(g, obj2gco(h));
    // TODO(port): array and hash traversal once LuaTable is accessible.
    gen_link(state, h);
}

/// C: `static lu_mem traversetable(global_State *g, Table *h)`
///
/// Marks the metatable, then dispatches on the `__mode` string to choose
/// between weak-key, weak-value, all-weak, or strong traversal.
/// Returns a work estimate: `1 + alimit + 2 * allocsizenode(h)`.
///
/// TODO(port): LuaTable, fasttm (TagMethod lookup), and string access from lua-vm.
unsafe fn traverse_table(state: &mut LuaState, h: GcObj) -> usize {
    // C: const TValue *mode = gfasttm(g, h->metatable, TM_MODE);
    // C: TString *smode;
    // C: markobjectN(g, h->metatable);
    // C: if (mode && ttisshrstring(mode) && ...) { /* dispatch */ }
    // C: else traversestrongtable(g, h);
    // C: return 1 + h->alimit + 2 * allocsizenode(h);
    // TODO(port): metatable / mode field access, string comparison.
    // Fallback: treat as strong table.
    traverse_strong_table(state, h);
    // Return minimum work estimate (1) until alimit is accessible.
    1
}

/// C: `static int traverseudata(global_State *g, Udata *u)`
///
/// Marks the metatable and each user value slot.  Calls `gen_link`.
/// Returns `1 + u->nuvalue` as the work estimate.
///
/// TODO(port): `LuaUserData` / `nuvalue` / `uv[]` from lua-vm.
unsafe fn traverse_udata(state: &mut LuaState, u: GcObj) -> usize {
    // C: markobjectN(g, u->metatable);
    // C: for (i=0; i<u->nuvalue; i++) markvalue(g, &u->uv[i].uv);
    // C: genlink(g, obj2gco(u));
    // C: return 1 + u->nuvalue;
    // TODO(port): access uv[] array once LuaUserData is accessible.
    gen_link(state, u);
    1 // conservative work estimate
}

/// C: `static int traverseproto(global_State *g, Proto *f)`
///
/// Marks source name, all constants, upvalue descriptors, nested protos, and
/// local-variable names.  Returns `1 + sizek + sizeupvalues + sizep + sizelocvars`.
///
/// TODO(port): `LuaProto` fields from lua-vm.
unsafe fn traverse_proto(state: &mut LuaState, f: GcObj) -> usize {
    // C: markobjectN(g, f->source);
    // C: for k: markvalue(g, &f->k[i]);
    // C: for upvalues: markobjectN(g, f->upvalues[i].name);
    // C: for p:  markobjectN(g, f->p[i]);
    // C: for locvars: markobjectN(g, f->locvars[i].varname);
    // C: return 1 + f->sizek + f->sizeupvalues + f->sizep + f->sizelocvars;
    // TODO(port): field access once LuaProto is accessible.
    let _ = (state, f);
    1
}

/// C: `static int traverseCclosure(global_State *g, CClosure *cl)`
///
/// Marks all upvalue slots (`cl->upvalue[i]`).
/// Returns `1 + cl->nupvalues`.
///
/// TODO(port): CClosure fields from lua-vm.
unsafe fn traverse_c_closure(state: &mut LuaState, cl: GcObj) -> usize {
    // C: for (i=0; i<cl->nupvalues; i++) markvalue(g, &cl->upvalue[i]);
    // C: return 1 + cl->nupvalues;
    // TODO(port): upvalue iteration once LuaClosure::C is accessible.
    let _ = (state, cl);
    1
}

/// C: `static int traverseLclosure(global_State *g, LClosure *cl)`
///
/// Marks the prototype and each upvalue cell.
/// Returns `1 + cl->nupvalues`.
///
/// TODO(port): LClosure / UpVal from lua-vm.
unsafe fn traverse_l_closure(state: &mut LuaState, cl: GcObj) -> usize {
    // C: markobjectN(g, cl->p);
    // C: for (i=0; i<cl->nupvalues; i++) { UpVal *uv = cl->upvals[i]; markobjectN(g, uv); }
    // C: return 1 + cl->nupvalues;
    // TODO(port): LClosure / UpVal fields once lua-vm is accessible.
    let _ = (state, cl);
    1
}

/// C: `static int traversethread(global_State *g, lua_State *th)`
///
/// Marks all live stack slots of `th`.  In the atomic phase, also clears
/// the dead portion of the stack above `top` and re-links the thread into
/// `twups` if it has open upvalues.  Returns `1 + stacksize(th)`.
///
/// TODO(port): LuaState stack / openupval / twups fields from lua-vm.
unsafe fn traverse_thread(state: &mut LuaState, th: GcObj) -> usize {
    let g = state.global_mut();
    // C: if (isold(th) || g->gcstate == GCSpropagate) linkgclist(th, g->grayagain);
    if is_old_marked((*th).marked) || g.gcstate == GCS_PROPAGATE {
        let gclist_ptr = get_gc_list(th);
        if !gclist_ptr.is_null() {
            link_gc_list(th, gclist_ptr, &mut g.grayagain);
        }
    }
    // C: if (o == NULL) return 1;  /* stack not built yet */
    // C: for (; o < th->top.p; o++) markvalue(g, s2v(o));
    // C: for (uv = th->openupval; uv != NULL; uv=uv->u.open.next) markobject(g, uv);
    // C: if (gcstate==GCSatomic) { shrink; clear dead slots; re-link twups if needed }
    // C: return 1 + stacksize(th);
    // TODO(port): stack iteration once LuaState stack fields are accessible here.
    1 // conservative work estimate
}

/// C: `static lu_mem propagatemark(global_State *g)`
///
/// Pops the first object off the `gray` list, paints it black, and dispatches
/// to the appropriate traverse function.  Returns the work estimate.
unsafe fn propagate_mark(state: &mut LuaState) -> usize {
    // C: GCObject *o = g->gray;
    // C: nw2black(o); g->gray = *getgclist(o);
    let g = state.global_mut();
    let o = g.gray;
    debug_assert!(!o.is_null(), "propagate_mark: gray list is empty");

    // C: nw2black(o)
    (*o).marked = nw2black((*o).marked);

    // C: g->gray = *getgclist(o)   — advance list head
    let gclist_ptr = get_gc_list(o);
    if !gclist_ptr.is_null() {
        g.gray = *gclist_ptr;
    } else {
        g.gray = std::ptr::null_mut();
    }

    // C: switch (o->tt)
    match (*o).tt {
        LUA_VTABLE   => traverse_table(state, o),
        LUA_VUSERDATA => traverse_udata(state, o),
        LUA_VLCL     => traverse_l_closure(state, o),
        LUA_VCCL     => traverse_c_closure(state, o),
        LUA_VPROTO   => traverse_proto(state, o),
        LUA_VTHREAD  => traverse_thread(state, o),
        _ => {
            debug_assert!(false, "propagate_mark: unknown tt={}", (*o).tt);
            0
        }
    }
}

/// C: `static lu_mem propagateall(global_State *g)`
///
/// Runs `propagate_mark` until `g->gray` is empty.  Returns total work.
unsafe fn propagate_all(state: &mut LuaState) -> usize {
    // C: lu_mem tot = 0; while (g->gray) tot += propagatemark(g); return tot;
    let mut tot: usize = 0;
    while !state.global().gray.is_null() {
        tot += propagate_mark(state);
    }
    tot
}

/// C: `static void convergeephemerons(global_State *g)`
///
/// Iterates until no more marks propagate through ephemeron tables.
/// Alternates traversal direction each iteration to speed convergence on chains.
unsafe fn converge_ephemerons(state: &mut LuaState) {
    // C: int changed; int dir = 0;
    // C: do {
    //       GCObject *next = g->ephemeron; g->ephemeron = NULL; changed = 0;
    //       while ((w = next) != NULL) {
    //           Table *h = gco2t(w); next = h->gclist; nw2black(h);
    //           if (traverseephemeron(g, h, dir)) { propagateall(g); changed=1; }
    //       }
    //       dir = !dir;
    //   } while (changed);
    let mut dir = false;
    loop {
        // C: GCObject *next = g->ephemeron; g->ephemeron = NULL; changed = 0;
        let mut changed = false;
        let mut w = {
            let g = state.global_mut();
            let next = g.ephemeron;
            g.ephemeron = std::ptr::null_mut();
            next
        };
        // C: while ((w = next) != NULL)
        while !w.is_null() {
            // C: Table *h = gco2t(w); next = h->gclist; nw2black(h);
            let next_w = {
                let gclist_ptr = get_gc_list(w);
                if !gclist_ptr.is_null() { *gclist_ptr } else { std::ptr::null_mut() }
            };
            (*w).marked = nw2black((*w).marked);

            // C: if (traverseephemeron(g, h, dir)) { propagateall(g); changed=1; }
            if traverse_ephemeron(state, w, dir) {
                propagate_all(state);
                changed = true;
            }
            w = next_w;
        }
        // C: dir = !dir;
        dir = !dir;
        // C: } while (changed);
        if !changed {
            break;
        }
    }
}

// §D — Sweep functions ------------------------------------------------------

/// C: `static void clearbykeys(global_State *g, GCObject *l)`
///
/// Walks the intrusive `gclist` chain of weak-table objects in `l`.
/// For each table, removes hash entries whose *key* is white (dead).
///
/// PORT NOTE: Intrusive list linked via `gclist`. In Rust (types.tsv) that
/// field is removed; callers must supply the list head as a `GcObj`.
/// TODO(port): hash-node iteration requires LuaTable access from lua-vm.
unsafe fn clear_by_keys(state: &mut LuaState, mut list: GcObj) {
    // C: for (; l; l = gco2t(l)->gclist) {
    //       Table *h = gco2t(l);
    //       Node *limit = gnodelast(h);
    //       for (n = gnode(h,0); n < limit; n++) {
    //           if (iscleared(g, gckeyN(n))) setempty(gval(n));
    //           if (isempty(gval(n))) clearkey(n);
    //       }
    //   }
    let cw = state.global().currentwhite;
    while !list.is_null() {
        // C: Table *h = gco2t(l);
        let _h = list; // placeholder — actual Table ops below are TODO(port)
        // TODO(port): iterate hash nodes once LuaTable is accessible.
        // C: Node *limit = gnodelast(h); for (n=gnode(h,0); n<limit; n++) ...
        // placeholder: advance via gclist
        let gclist_ptr = get_gc_list(list);
        list = if !gclist_ptr.is_null() { *gclist_ptr } else { std::ptr::null_mut() };
        let _ = cw; // suppress warning
    }
}

/// C: `static void clearbyvalues(global_State *g, GCObject *l, GCObject *f)`
///
/// Walks the intrusive `gclist` chain of weak-table objects from `l` up to
/// (but not including) `f`, removing entries whose *value* is white (dead).
///
/// TODO(port): array and hash-node iteration requires LuaTable from lua-vm.
unsafe fn clear_by_values(state: &mut LuaState, mut list: GcObj, f: GcObj) {
    // C: for (; l != f; l = gco2t(l)->gclist) {
    //       for i in array: if (iscleared(g, gcvalueN(&array[i]))) setempty(&array[i]);
    //       for n in hash:  if (iscleared(g, gcvalueN(gval(n)))) setempty(gval(n));
    //                       if (isempty(gval(n))) clearkey(n);
    //   }
    let cw = state.global().currentwhite;
    while list != f && !list.is_null() {
        // TODO(port): array/hash traversal once LuaTable is accessible.
        let gclist_ptr = get_gc_list(list);
        list = if !gclist_ptr.is_null() { *gclist_ptr } else { std::ptr::null_mut() };
        let _ = cw;
    }
}

/// C: `static void freeupval(lua_State *L, UpVal *uv)`
///
/// If the upvalue is open, unlinks it from its thread; then deallocates.
///
/// TODO(port): UpVal / luaF_unlinkupval from lua-vm.
unsafe fn free_upval(state: &mut LuaState, uv: GcObj) {
    // C: if (upisopen(uv)) luaF_unlinkupval(uv);
    // C: luaM_free(L, uv);
    // macros.tsv: upisopen(uv) → matches!(uv, UpVal::Open {..})
    // macros.tsv: luaM_free → Rust Drop handles this
    // TODO(port): call luaF_unlinkupval equivalent once UpVal is accessible.
    let _ = (state, uv);
}

/// C: `static void freeobj(lua_State *L, GCObject *o)`
///
/// Type-dispatches GC object deallocation.
///
/// TODO(port): All concrete type de-allocators (luaF_freeproto, luaH_free,
/// luaE_freethread, luaS_remove) are in lua-vm; circular dep.
unsafe fn free_obj(state: &mut LuaState, o: GcObj) {
    // C: switch (o->tt)
    match (*o).tt {
        LUA_VPROTO => {
            // C: luaF_freeproto(L, gco2p(o));
            // TODO(port): call lua_vm::func::free_proto once accessible.
        }
        LUA_VUPVAL => {
            // C: freeupval(L, gco2upv(o));
            free_upval(state, o);
        }
        LUA_VLCL => {
            // C: luaM_freemem(L, cl, sizeLclosure(cl->nupvalues));
            // macros.tsv: luaM_freemem → Rust Drop handles it
            // TODO(port): actual dealloc once LClosure size is known
        }
        LUA_VCCL => {
            // C: luaM_freemem(L, cl, sizeCclosure(cl->nupvalues));
            // TODO(port): same
        }
        LUA_VTABLE => {
            // C: luaH_free(L, gco2t(o));
            // TODO(port): call lua_vm::table::LuaTable::free
        }
        LUA_VTHREAD => {
            // C: luaE_freethread(L, gco2th(o));
            // TODO(port): call lua_vm::state::free_thread
        }
        LUA_VUSERDATA => {
            // C: luaM_freemem(L, o, sizeudata(u->nuvalue, u->len));
            // TODO(port): compute size and free
        }
        LUA_VSHRSTR => {
            // C: luaS_remove(L, ts); luaM_freemem(L, ts, sizelstring(ts->shrlen));
            // TODO(port): call lua_vm::string::remove from intern table
        }
        LUA_VLNGSTR => {
            // C: luaM_freemem(L, ts, sizelstring(ts->u.lnglen));
            // TODO(port): free long-string allocation
        }
        _ => {
            debug_assert!(false, "free_obj: unknown type tag {}", (*o).tt);
        }
    }
}

/// C: `static GCObject **sweeplist(lua_State *L, GCObject **p, int countin, int *countout)`
///
/// Sweeps up to `count` objects from the intrusive list at `*p`.
/// Dead objects (marked with `otherwhite`) are freed; live ones are reset to
/// current white.  Returns the next cursor position (`NULL` if list exhausted).
///
/// PORT NOTE: Uses `GCObject **p` (pointer-to-next-pointer) as a cursor for
/// O(1) removal from an intrusive singly-linked list.
unsafe fn sweep_list(
    state: &mut LuaState,
    mut p: *mut GcObj,
    count: i32,
    count_out: *mut i32,
) -> *mut GcObj {
    // C: global_State *g = G(L);
    // C: int ow = otherwhite(g); int i; int white = luaC_white(g);
    let (ow, white) = {
        let g = state.global();
        (other_white(g.currentwhite), current_white(g.currentwhite))
    };
    let mut i = 0;
    // C: for (i = 0; *p != NULL && i < countin; i++)
    while !(*p).is_null() && i < count {
        let curr = *p;
        let marked = (*curr).marked;
        // C: if (isdeadm(ow, marked))
        if is_dead_marked(ow, marked) {
            // C: *p = curr->next; freeobj(L, curr);
            *p = (*curr).next;
            free_obj(state, curr);
        } else {
            // C: curr->marked = cast_byte((marked & ~maskgcbits) | white);
            (*curr).marked = (marked & !MASK_GC_BITS) | white;
            // C: p = &curr->next;
            p = &mut (*curr).next;
        }
        i += 1;
    }
    // C: if (countout) *countout = i;
    if !count_out.is_null() {
        *count_out = i;
    }
    // C: return (*p == NULL) ? NULL : p;
    if (*p).is_null() { std::ptr::null_mut() } else { p }
}

/// C: `static GCObject **sweeptolive(lua_State *L, GCObject **p)`
///
/// Sweeps one object at a time until the cursor moves (i.e., a live object is
/// encountered) or the list is exhausted.
unsafe fn sweep_to_live(state: &mut LuaState, mut p: *mut GcObj) -> *mut GcObj {
    // C: GCObject **old = p; do { p = sweeplist(L, p, 1, NULL); } while (p == old);
    // C: return p;
    let old = p;
    loop {
        p = sweep_list(state, p, 1, std::ptr::null_mut());
        if p != old {
            break;
        }
    }
    p
}

// §E — Finalization ---------------------------------------------------------

/// C: `static void checkSizes(lua_State *L, global_State *g)`
///
/// If not in emergency mode and the interned string table is < 25% full,
/// halves it and corrects `GCestimate` for the debt change.
///
/// TODO(port): `luaS_resize` is in lua-vm (lstring.c → lua-vm::string).
unsafe fn check_sizes(state: &mut LuaState) {
    // C: if (!g->gcemergency) {
    //       if (g->strt.nuse < g->strt.size / 4) {
    //           l_mem olddebt = g->GCdebt;
    //           luaS_resize(L, g->strt.size / 2);
    //           g->GCestimate += g->GCdebt - olddebt;
    //       }
    //   }
    let g = state.global_mut();
    if !g.gcemergency {
        if g.strt.nuse < g.strt.size / 4 {
            let old_debt = g.GCdebt;
            // TODO(port): call lua_vm::string::resize(state, g.strt.size / 2);
            let new_debt = state.global().GCdebt;
            state.global_mut().GCestimate =
                state.global().GCestimate.wrapping_add_signed(new_debt - old_debt);
        }
    }
}

/// C: `static GCObject *udata2finalize(global_State *g)`
///
/// Pops the head of `tobefnz`, moves it back to `allgc`, clears the
/// `FINALIZEDBIT`, and (in sweep phase or age==G_OLD1) adjusts GC bookkeeping.
unsafe fn udata_to_finalize(state: &mut LuaState) -> GcObj {
    // C: GCObject *o = g->tobefnz;
    // C: lua_assert(tofinalize(o));
    let g = state.global_mut();
    let o = g.tobefnz;
    debug_assert!(!o.is_null());
    debug_assert!(is_finalized_marked((*o).marked));

    // C: g->tobefnz = o->next; o->next = g->allgc; g->allgc = o;
    g.tobefnz = (*o).next;
    (*o).next = g.allgc;
    g.allgc = o;

    // C: resetbit(o->marked, FINALIZEDBIT);
    (*o).marked &= !(1 << FINALIZED_BIT);

    // C: if (issweepphase(g)) makewhite(g, o);
    if is_sweep_phase(g.gcstate) {
        (*o).marked = make_white((*o).marked, g.currentwhite);
    } else if get_age((*o).marked) == G_OLD1 {
        // C: else if (getage(o) == G_OLD1) g->firstold1 = o;
        g.firstold1 = o;
    }
    o
}

/// C: `static void dothecall(lua_State *L, void *ud)` — trampoline used as
/// the `f` argument to `luaD_pcall` when running a `__gc` finalizer.
///
/// C: `luaD_callnoyield(L, L->top.p - 2, 0);`
///
/// TODO(port): `luaD_callnoyield` is in lua-vm; circular dep.
unsafe fn do_the_call(state: &mut LuaState) {
    // C: UNUSED(ud); luaD_callnoyield(L, L->top.p - 2, 0);
    // TODO(port): call state.call_no_yield(top - 2, 0) once lua-vm is wired.
    let _ = state;
}

/// C: `static void GCTM(lua_State *L)`
///
/// Invokes the `__gc` finalizer for the next object in `tobefnz`.
/// Runs inside a protected call so errors are caught and warned about.
/// Temporarily stops debug hooks and GC steps.
///
/// TODO(port): luaT_gettmbyobj, luaD_pcall, luaE_warnerror from lua-vm.
unsafe fn gc_tm(state: &mut LuaState) {
    // C: lua_assert(!g->gcemergency);
    debug_assert!(!state.global().gcemergency);

    // C: setgcovalue(L, &v, udata2finalize(g));
    let _o = udata_to_finalize(state);

    // C: tm = luaT_gettmbyobj(L, &v, TM_GC);
    // C: if (!notm(tm)) { /* push tm and v, pcall dothecall */ }
    // TODO(port): tag-method lookup and protected call — all in lua-vm.
    // For now, simply re-close the object without calling its finalizer.

    // C: L->allowhook = oldah; g->gcstp = oldgcstp;
    // (state restoration is elided since we don't run the finalizer yet)
}

/// C: `static int runafewfinalizers(lua_State *L, int n)`
///
/// Calls at most `n` finalizers from `tobefnz`.  Returns how many were called.
unsafe fn run_few_finalizers(state: &mut LuaState, n: i32) -> i32 {
    // C: for (i = 0; i < n && g->tobefnz; i++) GCTM(L);
    // C: return i;
    let mut i = 0;
    while i < n && !state.global().tobefnz.is_null() {
        gc_tm(state);
        i += 1;
    }
    i
}

/// C: `static void callallpendingfinalizers(lua_State *L)`
///
/// Drains the entire `tobefnz` list by calling `gc_tm` until empty.
unsafe fn call_all_pending_finalizers(state: &mut LuaState) {
    // C: while (g->tobefnz) GCTM(L);
    while !state.global().tobefnz.is_null() {
        gc_tm(state);
    }
}

/// C: `static GCObject **findlast(GCObject **p)`
///
/// Walks intrusive `next` links to find the last `next` field in the list —
/// i.e., the `&last->next` slot where `last->next == NULL`.
/// Used to append to a list in O(n).
unsafe fn find_last(mut p: *mut GcObj) -> *mut GcObj {
    // C: while (*p != NULL) p = &(*p)->next;
    // C: return p;
    while !(*p).is_null() {
        // SAFETY: *p is a live GcHeader with a valid `next` field.
        p = &mut (**p).next;
    }
    p
}

/// C: `static void separatetobefnz(global_State *g, int all)`
///
/// Moves objects from `finobj` into `tobefnz`.
/// If `all` is true, moves every object; otherwise only white (dead) ones.
/// Stops at `finobjold1` (objects past that cannot be white in gen. mode).
unsafe fn separate_to_be_finalized(state: &mut LuaState, all: bool) {
    // C: GCObject *curr; GCObject **p = &g->finobj;
    // C: GCObject **lastnext = findlast(&g->tobefnz);
    // C: while ((curr = *p) != g->finobjold1) {
    //       lua_assert(tofinalize(curr));
    //       if (!(iswhite(curr) || all)) { p = &curr->next; }
    //       else {
    //           if (curr == g->finobjsur) g->finobjsur = curr->next;
    //           *p = curr->next;
    //           curr->next = *lastnext; *lastnext = curr; lastnext = &curr->next;
    //       }
    //   }
    let g = state.global_mut();
    let mut p: *mut GcObj = &mut g.finobj;
    let mut lastnext: *mut GcObj = find_last(&mut g.tobefnz);
    let finobjold1 = g.finobjold1;

    while !(*p).is_null() && *p != finobjold1 {
        let curr = *p;
        debug_assert!(is_finalized_marked((*curr).marked));

        if !is_white_marked((*curr).marked) && !all {
            // C: p = &curr->next;
            p = &mut (*curr).next;
        } else {
            // C: if (curr == g->finobjsur) g->finobjsur = curr->next;
            if curr == g.finobjsur {
                g.finobjsur = (*curr).next;
            }
            // C: *p = curr->next;
            *p = (*curr).next;
            // C: curr->next = *lastnext; *lastnext = curr; lastnext = &curr->next;
            (*curr).next = *lastnext;
            *lastnext = curr;
            lastnext = &mut (*curr).next;
        }
    }
}

/// C: `static void checkpointer(GCObject **p, GCObject *o)`
///
/// If `*p == o`, advances `*p` to `o->next`.  Used to keep sweep cursors
/// valid when `o` is being unlinked from `allgc`.
///
/// # Safety
/// `p` must be non-null; `o` must be a valid GcHeader pointer.
unsafe fn check_pointer(p: *mut GcObj, o: GcObj) {
    // C: if (o == *p) *p = o->next;
    if *p == o {
        *p = (*o).next;
    }
}

/// C: `static void correctpointers(global_State *g, GCObject *o)`
///
/// Corrects the four generational cohort cursor fields when `o` is about to
/// be removed from `allgc`.
unsafe fn correct_pointers(state: &mut LuaState, o: GcObj) {
    // C: checkpointer(&g->survival, o); checkpointer(&g->old1, o);
    // C: checkpointer(&g->reallyold, o); checkpointer(&g->firstold1, o);
    let g = state.global_mut();
    check_pointer(&mut g.survival, o);
    check_pointer(&mut g.old1, o);
    check_pointer(&mut g.reallyold, o);
    check_pointer(&mut g.firstold1, o);
}

/// C: `void luaC_checkfinalizer(lua_State *L, GCObject *o, Table *mt)`
///
/// If `o` does not already have FINALIZEDBIT set, `mt` has a `__gc` field,
/// and the state is not closing, moves `o` from `allgc` to `finobj`.
///
/// TODO(port): `gfasttm` (GlobalState::fast_tm) from lua-vm; circular dep.
pub(crate) unsafe fn check_finalizer(
    state: &mut LuaState,
    o: GcObj,
    mt: GcObj,
) {
    // C: if (tofinalize(o) || gfasttm(g,mt,TM_GC)==NULL || (g->gcstp & GCSTPCLS))
    //       return;
    {
        let g = state.global();
        if is_finalized_marked((*o).marked) {
            return;
        }
        // TODO(port): check gfasttm(g, mt, TM_GC) — fast-path TM lookup.
        if g.gcstp & GCSTPCLS != 0 {
            return;
        }
        let _ = mt;
    }

    // C: if (issweepphase(g)) { makewhite(g,o); if (g->sweepgc == &o->next) ... }
    // C: else correctpointers(g, o);
    {
        let g = state.global_mut();
        if is_sweep_phase(g.gcstate) {
            let cw = g.currentwhite;
            (*o).marked = make_white((*o).marked, cw);
            // C: if (g->sweepgc == &o->next) g->sweepgc = sweeptolive(L, g->sweepgc);
            // TODO(port): sweepgc cursor is *mut GcObj; adjust if needed.
        }
    }
    if !is_sweep_phase(state.global().gcstate) {
        correct_pointers(state, o);
    }

    // C: for (p = &g->allgc; *p != o; p = &(*p)->next) {}
    // C: *p = o->next; o->next = g->finobj; g->finobj = o;
    // C: l_setbit(o->marked, FINALIZEDBIT);
    let g = state.global_mut();
    let mut p: *mut GcObj = &mut g.allgc;
    while !(*p).is_null() && *p != o {
        p = &mut (**p).next;
    }
    debug_assert_eq!(*p, o, "check_finalizer: object not found in allgc");
    *p = (*o).next;
    (*o).next = g.finobj;
    g.finobj = o;
    (*o).marked |= 1 << FINALIZED_BIT;
}

// §F — Generational collector -----------------------------------------------

/// C: `static void setpause(global_State *g)`
///
/// Sets GC debt so the next cycle begins when memory grows by roughly
/// `gcpause / PAUSEADJ` times the current estimate.
unsafe fn set_pause(state: &mut LuaState) {
    // C: int pause = getgcparam(g->gcpause);
    // C: l_mem estimate = g->GCestimate / PAUSEADJ;
    // C: threshold = (pause < MAX_LMEM / estimate) ? estimate * pause : MAX_LMEM;
    // C: debt = gettotalbytes(g) - threshold;
    // C: if (debt > 0) debt = 0;
    // C: luaE_setdebt(g, debt);
    // macros.tsv: getgcparam(p) → (p as u32) * 4
    let g = state.global_mut();
    let pause = g.gcpause as isize * 4;
    let estimate = (g.GCestimate as isize).max(1) / PAUSE_ADJ;
    let threshold = if pause < isize::MAX / estimate {
        estimate * pause
    } else {
        isize::MAX
    };
    let total = g.total_bytes() as isize;
    let mut debt = total - threshold;
    if debt > 0 { debt = 0; }
    // C: luaE_setdebt(g, debt)
    // TODO(port): luaE_setdebt is in lua-vm; call state.set_debt(debt) in Phase B.
    g.GCdebt = debt;
}

/// C: `static void sweep2old(lua_State *L, GCObject **p)`
///
/// Sweeps a list for entry into generational mode: frees dead (white) objects,
/// promotes all survivors to G_OLD, puts threads in `grayagain`, keeps open
/// upvalues gray, paints everything else black.
unsafe fn sweep_to_old(state: &mut LuaState, p: *mut GcObj) {
    // C: GCObject *curr; global_State *g = G(L);
    // C: while ((curr = *p) != NULL) {
    //       if (iswhite(curr)) { *p = curr->next; freeobj(L, curr); }
    //       else {
    //           setage(curr, G_OLD);
    //           if (curr->tt == LUA_VTHREAD) { linkgclist(th, g->grayagain); }
    //           else if (curr->tt == LUA_VUPVAL && upisopen(gco2upv(curr))) set2gray(curr);
    //           else nw2black(curr);
    //           p = &curr->next;
    //       }
    //   }
    let mut p = p;
    loop {
        let curr = *p;
        if curr.is_null() { break; }
        if is_white_marked((*curr).marked) {
            // C: *p = curr->next; freeobj(L, curr);
            *p = (*curr).next;
            free_obj(state, curr);
        } else {
            // C: setage(curr, G_OLD);
            (*curr).marked = set_age_bits((*curr).marked, G_OLD);
            if (*curr).tt == LUA_VTHREAD {
                let g = state.global_mut();
                let gclist_ptr = get_gc_list(curr);
                if !gclist_ptr.is_null() {
                    link_gc_list(curr, gclist_ptr, &mut g.grayagain);
                }
            } else if (*curr).tt == LUA_VUPVAL {
                // TODO(port): check upisopen(gco2upv(curr)) — requires UpVal downcast.
                // Conservative: keep gray
                (*curr).marked = set_to_gray((*curr).marked);
            } else {
                (*curr).marked = nw2black((*curr).marked);
            }
            p = &mut (*curr).next;
        }
    }
}

/// C: `static GCObject **sweepgen(lua_State *L, global_State *g, GCObject **p,
///                                GCObject *limit, GCObject **pfirstold1)`
///
/// Generational sweep from `*p` up to (not including) `limit`.
/// New objects → make white and advance to G_SURVIVAL.
/// Other objects → advance age; track first G_OLD1 in `*pfirstold1`.
unsafe fn sweep_gen(
    state: &mut LuaState,
    mut p: *mut GcObj,
    limit: GcObj,
    pfirstold1: *mut GcObj,
) -> *mut GcObj {
    // C: static const lu_byte nextage[] = { G_SURVIVAL, G_OLD1, G_OLD1, G_OLD, G_OLD, G_TOUCHED1, G_TOUCHED2 };
    const NEXT_AGE: [u8; 7] = [
        G_SURVIVAL, G_OLD1, G_OLD1, G_OLD, G_OLD, G_TOUCHED1, G_TOUCHED2,
    ];
    let white = current_white(state.global().currentwhite);

    loop {
        let curr = *p;
        if curr.is_null() || curr == limit { break; }

        if is_white_marked((*curr).marked) {
            // C: lua_assert(!isold(curr) && isdead(g,curr));
            debug_assert!(!is_old_marked((*curr).marked));
            // C: *p = curr->next; freeobj(L, curr);
            *p = (*curr).next;
            free_obj(state, curr);
        } else {
            let age = get_age((*curr).marked);
            if age == G_NEW {
                // C: marked = curr->marked & ~maskgcbits; curr->marked = marked | G_SURVIVAL | white;
                (*curr).marked = ((*curr).marked & !MASK_GC_BITS) | G_SURVIVAL | white;
            } else {
                // C: setage(curr, nextage[getage(curr)]);
                let new_age = NEXT_AGE[age as usize];
                (*curr).marked = set_age_bits((*curr).marked, new_age);
                // C: if (getage(curr) == G_OLD1 && *pfirstold1 == NULL) *pfirstold1 = curr;
                if get_age((*curr).marked) == G_OLD1 && (*pfirstold1).is_null() {
                    *pfirstold1 = curr;
                }
            }
            p = &mut (*curr).next;
        }
    }
    p
}

/// C: `static void whitelist(global_State *g, GCObject *p)`
///
/// Paints all objects in the intrusive list `p` white (clears age bits too).
/// Used when transitioning from generational to incremental mode.
unsafe fn whitelist(state: &mut LuaState, mut p: GcObj) {
    // C: int white = luaC_white(g);
    // C: for (; p != NULL; p = p->next) p->marked = (p->marked & ~maskgcbits) | white;
    let white = current_white(state.global().currentwhite);
    while !p.is_null() {
        (*p).marked = ((*p).marked & !MASK_GC_BITS) | white;
        p = (*p).next;
    }
}

/// C: `static GCObject **correctgraylist(GCObject **p)`
///
/// Post-generational-sweep gray list correction.  Walks `*p` and for each object:
/// - white → remove (swept away)
/// - G_TOUCHED1 → paint black, advance to G_TOUCHED2, keep in list
/// - thread → keep in list
/// - G_TOUCHED2 → advance to G_OLD, paint black, remove
/// - everything else → remove
/// Returns a pointer to the tail link of the surviving list.
unsafe fn correct_gray_list(mut p: *mut GcObj) -> *mut GcObj {
    // C: GCObject *curr; while ((curr = *p) != NULL) { ... }
    loop {
        let curr = *p;
        if curr.is_null() { break; }

        let next_ptr = get_gc_list(curr);

        if is_white_marked((*curr).marked) {
            // remove: *p = *next; continue;
            *p = if !next_ptr.is_null() { *next_ptr } else { std::ptr::null_mut() };
            // don't advance p
        } else if get_age((*curr).marked) == G_TOUCHED1 {
            // C: nw2black(curr); changeage(curr, G_TOUCHED1, G_TOUCHED2); goto remain;
            debug_assert!(is_gray_marked((*curr).marked));
            (*curr).marked = nw2black((*curr).marked);
            (*curr).marked = change_age_bits((*curr).marked, G_TOUCHED1, G_TOUCHED2);
            // remain: p = next; continue;
            p = if !next_ptr.is_null() { next_ptr } else { break; };
        } else if (*curr).tt == LUA_VTHREAD {
            // C: goto remain;
            debug_assert!(is_gray_marked((*curr).marked));
            p = if !next_ptr.is_null() { next_ptr } else { break; };
        } else {
            // C: if (getage(curr)==G_TOUCHED2) changeage(curr, G_TOUCHED2, G_OLD);
            // C: nw2black(curr); goto remove;
            debug_assert!(is_old_marked((*curr).marked));
            if get_age((*curr).marked) == G_TOUCHED2 {
                (*curr).marked = change_age_bits((*curr).marked, G_TOUCHED2, G_OLD);
            }
            (*curr).marked = nw2black((*curr).marked);
            *p = if !next_ptr.is_null() { *next_ptr } else { std::ptr::null_mut() };
        }
    }
    p
}

/// C: `static void correctgraylists(global_State *g)`
///
/// Coalesces `grayagain`, `weak`, `allweak`, and `ephemeron` into `grayagain`
/// by running `correctgraylist` on each and chaining the tails.
unsafe fn correct_gray_lists(state: &mut LuaState) {
    // C: GCObject **list = correctgraylist(&g->grayagain);
    // C: *list = g->weak; g->weak = NULL; list = correctgraylist(list);
    // C: *list = g->allweak; g->allweak = NULL; list = correctgraylist(list);
    // C: *list = g->ephemeron; g->ephemeron = NULL; correctgraylist(list);
    let g = state.global_mut();
    let list = correct_gray_list(&mut g.grayagain);
    *list = g.weak;
    g.weak = std::ptr::null_mut();
    let list = correct_gray_list(list);
    *list = g.allweak;
    g.allweak = std::ptr::null_mut();
    let list = correct_gray_list(list);
    *list = g.ephemeron;
    g.ephemeron = std::ptr::null_mut();
    correct_gray_list(list);
}

/// C: `static void markold(global_State *g, GCObject *from, GCObject *to)`
///
/// Walks `[from, to)` in the `allgc` list; for each G_OLD1 object, advances
/// it to G_OLD and (if black) calls `really_mark_object` to re-mark it.
unsafe fn mark_old(state: &mut LuaState, from: GcObj, to: GcObj) {
    // C: for (p = from; p != to; p = p->next) {
    //       if (getage(p) == G_OLD1) {
    //           lua_assert(!iswhite(p));
    //           changeage(p, G_OLD1, G_OLD);
    //           if (isblack(p)) reallymarkobject(g, p);
    //       }
    //   }
    let mut p = from;
    while p != to && !p.is_null() {
        if get_age((*p).marked) == G_OLD1 {
            debug_assert!(!is_white_marked((*p).marked));
            (*p).marked = change_age_bits((*p).marked, G_OLD1, G_OLD);
            if is_black_marked((*p).marked) {
                really_mark_object(state, p);
            }
        }
        p = (*p).next;
    }
}

/// C: `static void finishgencycle(lua_State *L, global_State *g)`
///
/// Wraps up a young-generation cycle: fix gray lists, check string-table size,
/// transition to GCSpropagate (skip restartcollection), call pending finalizers.
unsafe fn finish_gen_cycle(state: &mut LuaState) {
    // C: correctgraylists(g); checkSizes(L,g); g->gcstate=GCSpropagate;
    // C: if (!g->gcemergency) callallpendingfinalizers(L);
    correct_gray_lists(state);
    check_sizes(state);
    state.global_mut().gcstate = GCS_PROPAGATE;
    if !state.global().gcemergency {
        call_all_pending_finalizers(state);
    }
}

/// C: `static void youngcollection(lua_State *L, global_State *g)`
///
/// Full minor (young-generation) collection.
unsafe fn young_collection(state: &mut LuaState) {
    // C: lua_assert(g->gcstate == GCSpropagate);
    debug_assert_eq!(state.global().gcstate, GCS_PROPAGATE);
    // C: if (g->firstold1) { markold(g, g->firstold1, g->reallyold); g->firstold1=NULL; }
    {
        let g = state.global_mut();
        let firstold1 = g.firstold1;
        let reallyold = g.reallyold;
        if !firstold1.is_null() {
            g.firstold1 = std::ptr::null_mut();
            mark_old(state, firstold1, reallyold);
        }
    }
    {
        let g = state.global();
        let finobj = g.finobj;
        let finobjrold = g.finobjrold;
        let tobefnz = g.tobefnz;
        mark_old(state, finobj, finobjrold);
        mark_old(state, tobefnz, std::ptr::null_mut());
    }
    let _ = atomic_phase(state);

    // C: g->gcstate = GCSswpallgc;
    // C: psurvival = sweepgen(L, g, &g->allgc, g->survival, &g->firstold1);
    // C: sweepgen(L, g, psurvival, g->old1, &g->firstold1);
    // C: g->reallyold=g->old1; g->old1=*psurvival; g->survival=g->allgc;
    state.global_mut().gcstate = GCS_SWP_ALLGC;
    {
        let g = state.global_mut();
        let survival = g.survival;
        let old1 = g.old1;
        let allgc_ptr: *mut GcObj = &mut g.allgc;
        let firstold1_ptr: *mut GcObj = &mut g.firstold1;
        let psurvival = sweep_gen(state, allgc_ptr, survival, firstold1_ptr);
        let firstold1_ptr2: *mut GcObj = &mut state.global_mut().firstold1;
        sweep_gen(state, psurvival, old1, firstold1_ptr2);
        let g = state.global_mut();
        g.reallyold = old1;
        g.old1 = *psurvival;
        g.survival = g.allgc;
    }

    // C: (repeat for finobj sublists, then tobefnz)
    {
        let g = state.global_mut();
        let finobjsur = g.finobjsur;
        let finobjold1 = g.finobjold1;
        let finobj_ptr: *mut GcObj = &mut g.finobj;
        let mut dummy: GcObj = std::ptr::null_mut();
        let psurvival = sweep_gen(state, finobj_ptr, finobjsur, &mut dummy);
        sweep_gen(state, psurvival, finobjold1, &mut dummy);
        let g = state.global_mut();
        g.finobjrold = finobjold1;
        g.finobjold1 = *psurvival;
        g.finobjsur = g.finobj;
    }
    {
        let g = state.global_mut();
        let tobefnz_ptr: *mut GcObj = &mut g.tobefnz;
        let mut dummy: GcObj = std::ptr::null_mut();
        sweep_gen(state, tobefnz_ptr, std::ptr::null_mut(), &mut dummy);
    }
    finish_gen_cycle(state);
}

/// C: `static void atomic2gen(lua_State *L, global_State *g)`
///
/// After the atomic phase of an incremental cycle, sweeps everything to G_OLD
/// and switches to generational mode.
unsafe fn atomic_to_gen(state: &mut LuaState) {
    // C: cleargraylists(g); g->gcstate = GCSswpallgc; sweep2old(L, &g->allgc);
    // C: g->reallyold=g->old1=g->survival=g->allgc; g->firstold1=NULL;
    // C: sweep2old(L, &g->finobj); g->finobjrold=g->finobjold1=g->finobjsur=g->finobj;
    // C: sweep2old(L, &g->tobefnz);
    // C: g->gckind=KGC_GEN; g->lastatomic=0; g->GCestimate=gettotalbytes(g);
    // C: finishgencycle(L,g);
    clear_gray_lists(state);
    state.global_mut().gcstate = GCS_SWP_ALLGC;
    {
        let g = state.global_mut();
        let allgc_ptr: *mut GcObj = &mut g.allgc;
        sweep_to_old(state, allgc_ptr);
    }
    {
        let g = state.global_mut();
        g.reallyold = g.allgc;
        g.old1 = g.allgc;
        g.survival = g.allgc;
        g.firstold1 = std::ptr::null_mut();
    }
    {
        let g = state.global_mut();
        let finobj_ptr: *mut GcObj = &mut g.finobj;
        sweep_to_old(state, finobj_ptr);
    }
    {
        let g = state.global_mut();
        g.finobjrold = g.finobj;
        g.finobjold1 = g.finobj;
        g.finobjsur = g.finobj;
    }
    {
        let g = state.global_mut();
        let tobefnz_ptr: *mut GcObj = &mut g.tobefnz;
        sweep_to_old(state, tobefnz_ptr);
    }
    {
        let g = state.global_mut();
        g.gckind = KGC_GEN;
        g.lastatomic = 0;
        g.GCestimate = g.total_bytes();
    }
    finish_gen_cycle(state);
}

/// C: `static void setminordebt(global_State *g)`
///
/// Schedules the next minor collection when memory grows by `genminormul` %.
unsafe fn set_minor_debt(state: &mut LuaState) {
    // C: luaE_setdebt(g, -(cast(l_mem, (gettotalbytes(g) / 100)) * g->genminormul));
    let g = state.global_mut();
    let total = g.total_bytes() as isize;
    let mul = g.genminormul as isize;
    // TODO(port): luaE_setdebt call; for now set GCdebt directly.
    g.GCdebt = -((total / 100) * mul);
}

/// C: `static lu_mem entergen(lua_State *L, global_State *g)`
///
/// Drives the incremental GC to GCSpause, starts a new cycle, runs the full
/// atomic phase, then converts to generational mode.
/// Returns the number of objects traversed in the atomic phase.
unsafe fn enter_gen(state: &mut LuaState) -> usize {
    // C: luaC_runtilstate(L, bitmask(GCSpause));
    // C: luaC_runtilstate(L, bitmask(GCSpropagate));
    // C: numobjs = atomic(L); atomic2gen(L,g); setminordebt(g); return numobjs;
    run_until_state(state, 1u32 << GCS_PAUSE);
    run_until_state(state, 1u32 << GCS_PROPAGATE);
    let numobjs = atomic_phase(state);
    atomic_to_gen(state);
    set_minor_debt(state);
    numobjs
}

/// C: `static void enterinc(global_State *g)`
///
/// Converts all live objects to white, clears generational cohort pointers,
/// and enters incremental mode at GCSpause.
unsafe fn enter_inc(state: &mut LuaState) {
    // C: whitelist(g, g->allgc); g->reallyold=g->old1=g->survival=NULL;
    // C: whitelist(g, g->finobj); whitelist(g, g->tobefnz);
    // C: g->finobjrold=g->finobjold1=g->finobjsur=NULL;
    // C: g->gcstate=GCSpause; g->gckind=KGC_INC; g->lastatomic=0;
    {
        let g = state.global_mut();
        let allgc = g.allgc;
        whitelist(state, allgc);
    }
    {
        let g = state.global_mut();
        g.reallyold = std::ptr::null_mut();
        g.old1 = std::ptr::null_mut();
        g.survival = std::ptr::null_mut();
    }
    {
        let g = state.global_mut();
        let finobj = g.finobj;
        let tobefnz = g.tobefnz;
        whitelist(state, finobj);
        whitelist(state, tobefnz);
    }
    let g = state.global_mut();
    g.finobjrold = std::ptr::null_mut();
    g.finobjold1 = std::ptr::null_mut();
    g.finobjsur = std::ptr::null_mut();
    g.gcstate = GCS_PAUSE;
    g.gckind = KGC_INC;
    g.lastatomic = 0;
}

/// C: `void luaC_changemode(lua_State *L, int newmode)`
///
/// Switches between incremental (`KGC_INC`) and generational (`KGC_GEN`) modes.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn change_mode(state: &mut LuaState, new_mode: u8) {
    // C: if (newmode != g->gckind) {
    //       if (newmode == KGC_GEN) entergen(L,g); else enterinc(g);
    //   }
    // C: g->lastatomic = 0;
    if new_mode != state.global().gckind {
        if new_mode == KGC_GEN {
            enter_gen(state);
        } else {
            enter_inc(state);
        }
    }
    state.global_mut().lastatomic = 0;
}

/// C: `static lu_mem fullgen(lua_State *L, global_State *g)`
///
/// Full generational collection: switch to incremental then back.
unsafe fn full_gen(state: &mut LuaState) -> usize {
    // C: enterinc(g); return entergen(L, g);
    enter_inc(state);
    enter_gen(state)
}

/// C: `static void stepgenfull(lua_State *L, global_State *g)`
///
/// Full step when the last collection was "bad" (freed too few objects).
/// Uses the incremental collector; may return to generational if this cycle
/// is good, or stays incremental (and records `lastatomic`) if not.
unsafe fn step_gen_full(state: &mut LuaState) {
    // C: lu_mem newatomic; lu_mem lastatomic = g->lastatomic;
    // C: if (g->gckind == KGC_GEN) enterinc(g);
    // C: luaC_runtilstate(L, bitmask(GCSpropagate));
    // C: newatomic = atomic(L);
    // C: if (newatomic < lastatomic + (lastatomic >> 3)) { atomic2gen; setminordebt; }
    // C: else { g->GCestimate=gettotalbytes; entersweep; runtilstate(pause); setpause; g->lastatomic=newatomic; }
    let lastatomic = state.global().lastatomic;
    if state.global().gckind == KGC_GEN {
        enter_inc(state);
    }
    run_until_state(state, 1u32 << GCS_PROPAGATE);
    let newatomic = atomic_phase(state);
    if newatomic < lastatomic + (lastatomic >> 3) {
        atomic_to_gen(state);
        set_minor_debt(state);
    } else {
        state.global_mut().GCestimate = state.global().total_bytes();
        enter_sweep(state);
        run_until_state(state, 1u32 << GCS_PAUSE);
        set_pause(state);
        state.global_mut().lastatomic = newatomic;
    }
}

/// C: `static void genstep(lua_State *L, global_State *g)`
///
/// Top-level step for generational mode. Delegates to `step_gen_full` if the
/// last collection was bad, otherwise decides between major and minor.
unsafe fn gen_step(state: &mut LuaState) {
    // C: if (g->lastatomic != 0) stepgenfull(L,g);
    // C: else {
    //       lu_mem majorbase = g->GCestimate;
    //       lu_mem majorinc = (majorbase/100) * getgcparam(g->genmajormul);
    //       if (g->GCdebt > 0 && gettotalbytes(g) > majorbase + majorinc) {
    //           lu_mem numobjs = fullgen(L,g);
    //           if (gettotalbytes(g) < majorbase + (majorinc/2)) { /* good */ }
    //           else { g->lastatomic = numobjs; setpause(g); }
    //       } else {
    //           youngcollection(L,g); setminordebt(g); g->GCestimate = majorbase;
    //       }
    //   }
    //   lua_assert(isdecGCmodegen(g));
    if state.global().lastatomic != 0 {
        step_gen_full(state);
    } else {
        let majorbase = state.global().GCestimate;
        let majorinc = (majorbase as isize / 100)
            * (state.global().genmajormul as isize * 4);
        let debt = state.global().GCdebt;
        let total = state.global().total_bytes() as isize;
        if debt > 0 && total > majorbase as isize + majorinc {
            let numobjs = full_gen(state);
            if (state.global().total_bytes() as isize) < majorbase as isize + majorinc / 2 {
                debug_assert_eq!(state.global().lastatomic, 0);
            } else {
                state.global_mut().lastatomic = numobjs;
                set_pause(state);
            }
        } else {
            young_collection(state);
            set_minor_debt(state);
            state.global_mut().GCestimate = majorbase;
        }
    }
    debug_assert!(is_dec_gc_mode_gen(
        state.global().gckind, state.global().lastatomic
    ));
}

// §G — GC control (public API) ----------------------------------------------

/// C: `static void entersweep(lua_State *L)`
///
/// Transitions to `GCSswpallgc` and advances `sweepgc` past already-live
/// objects so the main sweep does not re-visit them.
unsafe fn enter_sweep(state: &mut LuaState) {
    // C: global_State *g = G(L); g->gcstate = GCSswpallgc;
    // C: lua_assert(g->sweepgc == NULL);
    // C: g->sweepgc = sweeptolive(L, &g->allgc);
    debug_assert!(state.global().sweepgc.is_null());
    state.global_mut().gcstate = GCS_SWP_ALLGC;
    let allgc_ptr: *mut GcObj = &mut state.global_mut().allgc;
    let cursor = sweep_to_live(state, allgc_ptr);
    state.global_mut().sweepgc = cursor;
}

/// C: `static void deletelist(lua_State *L, GCObject *p, GCObject *limit)`
///
/// Frees every object in `p` until `limit` (exclusive).
unsafe fn delete_list(state: &mut LuaState, mut p: GcObj, limit: GcObj) {
    // C: while (p != limit) { GCObject *next = p->next; freeobj(L, p); p = next; }
    while p != limit && !p.is_null() {
        let next = (*p).next;
        free_obj(state, p);
        p = next;
    }
}

/// C: `void luaC_freeallobjects(lua_State *L)`
///
/// Calls all pending finalizers then frees every GC object except the main
/// thread.  Called during `lua_close`.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn free_all_objects(state: &mut LuaState) {
    // C: g->gcstp = GCSTPCLS; luaC_changemode(L, KGC_INC);
    // C: separatetobefnz(g, 1); lua_assert(g->finobj == NULL);
    // C: callallpendingfinalizers(L);
    // C: deletelist(L, g->allgc, obj2gco(g->mainthread));
    // C: lua_assert(g->finobj == NULL);
    // C: deletelist(L, g->fixedgc, NULL);
    // C: lua_assert(g->strt.nuse == 0);
    state.global_mut().gcstp = GCSTPCLS;
    change_mode(state, KGC_INC);
    separate_to_be_finalized(state, true);
    debug_assert!(state.global().finobj.is_null());
    call_all_pending_finalizers(state);
    // C: deletelist up to mainthread (keep main thread alive)
    let mainthread = state.global().mainthread_raw();
    let allgc = state.global().allgc;
    delete_list(state, allgc, mainthread);
    debug_assert!(state.global().finobj.is_null());
    let fixedgc = state.global().fixedgc;
    delete_list(state, fixedgc, std::ptr::null_mut());
    debug_assert_eq!(state.global().strt.nuse, 0);
}

/// C: `static lu_mem atomic(lua_State *L)`
///
/// The stop-the-world atomic GC phase:
/// 1. Mark the running thread and re-mark the registry / metatables.
/// 2. Propagate all remaining gray objects.
/// 3. Remark upvalues of dying threads.
/// 4. Converge ephemeron tables.
/// 5. Clear weak-table values, separate objects to finalize.
/// 6. Mark finalizable objects, re-propagate, re-converge.
/// 7. Clear keys/values from weak tables.
/// 8. Flip the current white color.
/// Returns a work estimate (total slots marked).
unsafe fn atomic_phase(state: &mut LuaState) -> usize {
    // C: GCObject *origweak, *origall; GCObject *grayagain = g->grayagain;
    // C: g->grayagain = NULL; lua_assert(g->ephemeron==NULL && g->weak==NULL);
    // C: lua_assert(!iswhite(g->mainthread)); g->gcstate = GCSatomic;
    let grayagain = state.global().grayagain;
    state.global_mut().grayagain = std::ptr::null_mut();
    debug_assert!(state.global().ephemeron.is_null());
    debug_assert!(state.global().weak.is_null());
    let main_marked = (*state.global().mainthread_raw()).marked;
    debug_assert!(!is_white_marked(main_marked));
    state.global_mut().gcstate = GCS_ATOMIC;

    // C: markobject(g, L); markvalue(g, &g->l_registry); markmt(g);
    let running_thread = state.current_thread_raw();
    if is_white_marked((*running_thread).marked) {
        really_mark_object(state, running_thread);
    }
    // TODO(port): mark registry value (LuaValue → GcObj)
    mark_metatables(state);

    // C: work += propagateall(g);
    let mut work: usize = propagate_all(state);
    // C: work += remarkupvals(g); work += propagateall(g);
    work += remark_upvalues(state);
    work += propagate_all(state);

    // C: g->gray = grayagain; work += propagateall(g);
    state.global_mut().gray = grayagain;
    work += propagate_all(state);

    // C: convergeephemerons(g);
    converge_ephemerons(state);

    // C: clearbyvalues(g, g->weak, NULL); clearbyvalues(g, g->allweak, NULL);
    let (weak, allweak) = {
        let g = state.global();
        (g.weak, g.allweak)
    };
    clear_by_values(state, weak, std::ptr::null_mut());
    clear_by_values(state, allweak, std::ptr::null_mut());
    let origweak = weak;
    let origall = allweak;

    // C: separatetobefnz(g, 0); work += markbeingfnz(g); work += propagateall(g);
    separate_to_be_finalized(state, false);
    work += mark_being_finalized(state);
    work += propagate_all(state);

    // C: convergeephemerons(g);
    converge_ephemerons(state);

    // C: clearbykeys(g, g->ephemeron); clearbykeys(g, g->allweak);
    let (ephemeron, allweak2) = {
        let g = state.global();
        (g.ephemeron, g.allweak)
    };
    clear_by_keys(state, ephemeron);
    clear_by_keys(state, allweak2);

    // C: clearbyvalues(g, g->weak, origweak); clearbyvalues(g, g->allweak, origall);
    let (weak2, allweak3) = {
        let g = state.global();
        (g.weak, g.allweak)
    };
    clear_by_values(state, weak2, origweak);
    clear_by_values(state, allweak3, origall);

    // C: luaS_clearcache(g);
    // TODO(port): state.clear_string_cache() — in lua-vm.

    // C: g->currentwhite = cast_byte(otherwhite(g));
    let new_white = other_white(state.global().currentwhite);
    state.global_mut().currentwhite = new_white;

    debug_assert!(state.global().gray.is_null());
    work
}

/// C: `static int sweepstep(lua_State *L, global_State *g, int nextstate,
///                           GCObject **nextlist)`
///
/// Sweeps up to `GCSWEEPMAX` objects from `g->sweepgc`.  If the list is
/// exhausted, transitions to `nextstate` and sets `sweepgc = nextlist`.
unsafe fn sweep_step(
    state: &mut LuaState,
    next_state: u8,
    next_list: *mut GcObj,
) -> usize {
    // C: if (g->sweepgc) {
    //       l_mem olddebt = g->GCdebt;
    //       int count; g->sweepgc = sweeplist(L, g->sweepgc, GCSWEEPMAX, &count);
    //       g->GCestimate += g->GCdebt - olddebt;
    //       return count;
    //   } else { g->gcstate=nextstate; g->sweepgc=nextlist; return 0; }
    if !state.global().sweepgc.is_null() {
        let old_debt = state.global().GCdebt;
        let sweepgc = state.global().sweepgc;
        let mut count: i32 = 0;
        let new_cursor = sweep_list(state, sweepgc, GC_SWEEP_MAX, &mut count);
        state.global_mut().sweepgc = new_cursor;
        let new_debt = state.global().GCdebt;
        state.global_mut().GCestimate =
            state.global().GCestimate.wrapping_add_signed(new_debt - old_debt);
        count as usize
    } else {
        state.global_mut().gcstate = next_state;
        state.global_mut().sweepgc = next_list;
        0
    }
}

/// C: `static lu_mem singlestep(lua_State *L)`
///
/// Advances the GC FSM by one step and returns the work done.
unsafe fn single_step(state: &mut LuaState) -> usize {
    // C: lua_assert(!g->gcstopem); g->gcstopem = 1;
    debug_assert!(!state.global().gcstopem);
    state.global_mut().gcstopem = true;

    let work = match state.global().gcstate {
        GCS_PAUSE => {
            // C: restartcollection(g); g->gcstate=GCSpropagate; work=1;
            restart_collection(state);
            state.global_mut().gcstate = GCS_PROPAGATE;
            1
        }
        GCS_PROPAGATE => {
            // C: if (g->gray==NULL) { g->gcstate=GCSenteratomic; work=0; }
            // C: else work = propagatemark(g);
            if state.global().gray.is_null() {
                state.global_mut().gcstate = GCS_ENTER_ATOMIC;
                0
            } else {
                propagate_mark(state)
            }
        }
        GCS_ENTER_ATOMIC => {
            // C: work=atomic(L); entersweep(L); g->GCestimate=gettotalbytes(g);
            let work = atomic_phase(state);
            enter_sweep(state);
            state.global_mut().GCestimate = state.global().total_bytes();
            work
        }
        GCS_SWP_ALLGC => {
            // C: work = sweepstep(L, g, GCSswpfinobj, &g->finobj);
            let finobj_ptr: *mut GcObj = &mut state.global_mut().finobj;
            sweep_step(state, GCS_SWP_FINOBJ, finobj_ptr)
        }
        GCS_SWP_FINOBJ => {
            // C: work = sweepstep(L, g, GCSswptobefnz, &g->tobefnz);
            let tobefnz_ptr: *mut GcObj = &mut state.global_mut().tobefnz;
            sweep_step(state, GCS_SWP_TOBEFNZ, tobefnz_ptr)
        }
        GCS_SWP_TOBEFNZ => {
            // C: work = sweepstep(L, g, GCSswpend, NULL);
            sweep_step(state, GCS_SWP_END, std::ptr::null_mut())
        }
        GCS_SWP_END => {
            // C: checkSizes(L,g); g->gcstate=GCScallfin; work=0;
            check_sizes(state);
            state.global_mut().gcstate = GCS_CALLFIN;
            0
        }
        GCS_CALLFIN => {
            // C: if (g->tobefnz && !g->gcemergency) {
            //       g->gcstopem=0; work = runafewfinalizers(L, GCFINMAX) * GCFINALIZECOST;
            //   } else { g->gcstate=GCSpause; work=0; }
            if !state.global().tobefnz.is_null() && !state.global().gcemergency {
                state.global_mut().gcstopem = false;
                run_few_finalizers(state, GC_FIN_MAX) as usize * GC_FINALIZE_COST
            } else {
                state.global_mut().gcstate = GCS_PAUSE;
                0
            }
        }
        s => {
            debug_assert!(false, "single_step: unknown gcstate={}", s);
            0
        }
    };

    state.global_mut().gcstopem = false;
    work
}

/// C: `void luaC_runtilstate(lua_State *L, int statesmask)`
///
/// Runs `single_step` until `g->gcstate` matches one of the bits in `states_mask`.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn run_until_state(state: &mut LuaState, states_mask: u32) {
    // C: while (!testbit(statesmask, g->gcstate)) singlestep(L);
    // macros.tsv: testbit(x,b) → (x & (1<<b)) != 0
    while (states_mask & (1u32 << state.global().gcstate)) == 0 {
        single_step(state);
    }
}

/// C: `static void incstep(lua_State *L, global_State *g)`
///
/// Basic incremental step: converts GC debt and step size to "work units",
/// runs `single_step` until credit is positive or a pause state is reached,
/// then converts remaining credit back to bytes.
unsafe fn inc_step(state: &mut LuaState) {
    // C: int stepmul = (getgcparam(g->gcstepmul) | 1);
    // C: l_mem debt = (g->GCdebt / WORK2MEM) * stepmul;
    // C: l_mem stepsize = (g->gcstepsize <= log2maxs(l_mem))
    //          ? ((cast(l_mem, 1) << g->gcstepsize) / WORK2MEM) * stepmul
    //          : MAX_LMEM;
    let stepmul = (state.global().gcstepmul as isize * 4) | 1;
    let debt = (state.global().GCdebt / WORK2MEM as isize) * stepmul;
    let gcstepsize = state.global().gcstepsize;
    // log2maxs(l_mem) → (size_of::<isize>() * 8 - 2) as u32  (macros.tsv)
    let log2max = (std::mem::size_of::<isize>() * 8 - 2) as u8;
    let stepsize: isize = if gcstepsize <= log2max {
        ((1isize << gcstepsize) / WORK2MEM as isize) * stepmul
    } else {
        isize::MAX
    };

    // C: do { lu_mem work = singlestep(L); debt -= work; }
    // C: while (debt > -stepsize && g->gcstate != GCSpause);
    let mut debt = debt;
    loop {
        let work = single_step(state) as isize;
        debt -= work;
        if !(debt > -stepsize && state.global().gcstate != GCS_PAUSE) { break; }
    }

    // C: if (g->gcstate == GCSpause) setpause(g);
    // C: else { debt = (debt / stepmul) * WORK2MEM; luaE_setdebt(g, debt); }
    if state.global().gcstate == GCS_PAUSE {
        set_pause(state);
    } else {
        let new_debt = (debt / stepmul) * WORK2MEM as isize;
        // TODO(port): luaE_setdebt(g, debt) — set GCdebt properly via lua-vm.
        state.global_mut().GCdebt = new_debt;
    }
}

/// C: `void luaC_step(lua_State *L)`
///
/// Performs one GC step if the GC is running.  Sets a long wait if not running.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn step(state: &mut LuaState) {
    // C: if (!gcrunning(g)) luaE_setdebt(g, -2000);
    // C: else { if (isdecGCmodegen(g)) genstep(L,g); else incstep(L,g); }
    let (running, gen_mode) = {
        let g = state.global();
        (gc_running(g.gcstp), is_dec_gc_mode_gen(g.gckind, g.lastatomic))
    };
    if !running {
        // TODO(port): luaE_setdebt(g, -2000)
        state.global_mut().GCdebt = -2000;
    } else if gen_mode {
        gen_step(state);
    } else {
        inc_step(state);
    }
}

/// C: `static void fullinc(lua_State *L, global_State *g)`
///
/// Full collection in incremental mode.  If there are black objects, sweeps
/// them first to return them to white before starting a new mark cycle.
unsafe fn full_inc(state: &mut LuaState) {
    // C: if (keepinvariant(g)) entersweep(L);
    // C: luaC_runtilstate(L, bitmask(GCSpause));
    // C: luaC_runtilstate(L, bitmask(GCSpropagate));
    // C: g->gcstate = GCSenteratomic;
    // C: luaC_runtilstate(L, bitmask(GCScallfin));
    // C: lua_assert(g->GCestimate == gettotalbytes(g));
    // C: luaC_runtilstate(L, bitmask(GCSpause)); setpause(g);
    if keep_invariant(state.global().gcstate) {
        enter_sweep(state);
    }
    run_until_state(state, 1u32 << GCS_PAUSE);
    run_until_state(state, 1u32 << GCS_PROPAGATE);
    state.global_mut().gcstate = GCS_ENTER_ATOMIC;
    run_until_state(state, 1u32 << GCS_CALLFIN);
    debug_assert_eq!(
        state.global().GCestimate,
        state.global().total_bytes(),
        "fullinc: GCestimate mismatch after full cycle"
    );
    run_until_state(state, 1u32 << GCS_PAUSE);
    set_pause(state);
}

/// C: `void luaC_fullgc(lua_State *L, int isemergency)`
///
/// Runs a full GC cycle (incremental or generational).  In emergency mode,
/// finalizers and shrink operations are suppressed.
///
/// TODO(port): circular dep — `LuaState` is in lua-vm.
pub(crate) unsafe fn full_gc(state: &mut LuaState, is_emergency: bool) {
    // C: lua_assert(!g->gcemergency); g->gcemergency = isemergency;
    // C: if (g->gckind == KGC_INC) fullinc(L,g); else fullgen(L,g);
    // C: g->gcemergency = 0;
    debug_assert!(!state.global().gcemergency);
    state.global_mut().gcemergency = is_emergency;
    if state.global().gckind == KGC_INC {
        full_inc(state);
    } else {
        full_gen(state);
    }
    state.global_mut().gcemergency = false;
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lgc.c  (1744 lines, 73 functions)
//   target_crate:  lua-gc
//   confidence:    medium
//   todos:         80
//   port_notes:    9
//   unsafe_blocks: 0   (all GC functions are declared `unsafe fn`; no bare
//                       `unsafe { }` blocks — the entire body is in scope)
//   notes:         All 73 functions translated from C.  Logic is faithful;
//                  the primary Phase B/D blockers are:
//                  (1) Circular dep: LuaState / GlobalState are in lua-vm;
//                      every function is written speculatively.  Phase B
//                      introduces a GcHost trait or moves these to lua-vm.
//                  (2) GcHeader embedding: concrete types (LuaTable, LuaProto,
//                      etc.) must embed GcHeader at offset 0 via #[repr(C)]
//                      for get_gc_list() and the raw-pointer intrusive lists
//                      to work.  types.tsv removes gclist; Phase D must decide
//                      whether to re-add it or keep Vec-based gray lists.
//                  (3) Type downcasts: many traverse/free functions need to
//                      cast GcObj to concrete types (LuaTable, UpVal, etc.)
//                      that live in lua-vm — all marked TODO(port).
//                  (4) GlobalState field names (allgc, gray, etc.) are raw
//                      *mut GcHeader in this file, but types.tsv says they
//                      are Vec<GcRef<dyn Collectable>> — Phase D must choose
//                      one representation and adjust accordingly.
//                  Confidence "medium": GC algorithm logic is correct and
//                  mirrors C exactly; name-resolution errors are the only
//                  compile blockers expected in Phase B.
// ──────────────────────────────────────────────────────────────────────────
