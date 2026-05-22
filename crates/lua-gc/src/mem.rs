//! Memory manager interface for the Lua GC.
//!
//! Ported from `src/lmem.c` (Lua 5.4.7, 216 lines, 8 functions).
//!
//! Owns raw allocation / deallocation and GC-debt accounting. Because every
//! function touches the system allocator via raw pointers, all actual memory
//! operations live inside `unsafe` blocks with `// SAFETY:` comments. This
//! is permitted by `harness/unsafe-budgets.toml` for the `lua-gc` crate.
//!
//! # Design note (Phase A → B)
//!
//! In C, every function in this file receives `lua_State *L` and accesses
//! `global_State` via `G(L)`. In Rust, `LuaState` is defined in `lua-vm`
//! while `lmem.c` is mapped to `lua-gc`. This creates a circular dependency:
//! `lua-gc → lua-vm → lua-gc`. Phase B must resolve this (see TODO(port) at
//! each function). For Phase A, the function bodies are written as if
//! `LuaState` is in scope; the compiler will reject the import until Phase B
//! fixes the graph.

// C: #define lmem_c
// C: #define LUA_CORE
// C: #include "lprefix.h"
// C: #include "lua.h"
// C: #include "ldebug.h"
// C: #include "ldo.h"
// C: #include "lgc.h"
// C: #include "lmem.h"
// C: #include "lobject.h"
// C: #include "lstate.h"

use std::alloc::{self, Layout};

// TODO(port): LuaState is defined in lua-vm; bringing it in here creates a
// circular dependency (lua-gc → lua-vm → lua-gc). Phase B must either:
//   (a) introduce a `GcHost` trait in lua-types that LuaState implements, or
//   (b) move lmem.c functions into lua-vm and keep lua-gc for Phase-D GC only.
// For Phase A the import is written speculatively; do not attempt to compile.
// use lua_vm::state::LuaState;

use lua_types::error::LuaError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// C: `#define MINSIZEARRAY 4`
///
/// Minimum initial capacity for parser dynamic arrays. Prevents redundant
/// reallocations at sizes 1 → 2 → 4 during parsing. Arrays are shrunk to
/// their exact final sizes (or dropped) once parsing completes.
const MINSIZEARRAY: i32 = 4;

/// Byte alignment used for all GC object allocations via `call_realloc`.
///
/// The C code used `malloc`/`realloc`/`free`, which return maximally-aligned
/// memory. Rust's `std::alloc` requires an explicit `Layout`. We conservatively
/// use 8 bytes (the maximum primitive alignment on 64-bit targets).
///
/// TODO(port): Phase D must thread per-object `Layout` (including exact
/// alignment) through GC object headers so that `dealloc` uses the same
/// layout as `alloc`. Until then, every GC allocation is padded to 8-byte
/// alignment.
const GC_ALIGN: usize = 8;

// ---------------------------------------------------------------------------
// Internal raw-allocator shim
// ---------------------------------------------------------------------------

/// Invoke the system allocator using Lua's `frealloc` calling convention.
///
/// C: `#define callfrealloc(g,block,os,ns)  ((*g->frealloc)(g->ud, block, os, ns))`
///
/// Lua's hook semantics (documented at the top of `lmem.c`):
/// - `(null, _, 0)` → no-op, returns null.
/// - `(null, tag, ns > 0)` → allocate `ns` bytes. `os` is a GC type tag in
///   the new-object case; the default allocator ignores it.
/// - `(p, os, 0)` → free `p`, returns null.
/// - `(p, os, ns > 0)` → reallocate `p` from `os` to `ns` bytes.
///
/// `global_State.frealloc` and `global_State.ud` are removed in the Rust
/// port (types.tsv); we call `std::alloc` directly.
///
/// # Safety
/// - `ptr` is either null or a pointer previously returned by this function
///   with the same `GC_ALIGN` and the stated `os` as the byte count.
/// - If `ptr` is non-null, `os > 0`.
unsafe fn call_realloc(ptr: *mut u8, os: usize, ns: usize) -> *mut u8 {
    if ns == 0 {
        // C: frealloc(p, os, 0) → free p.
        if !ptr.is_null() {
            debug_assert!(os > 0, "freeing non-null block with osize == 0");
            // SAFETY: `ptr` was allocated by this function with `GC_ALIGN` and
            // size `os` — guaranteed by caller.
            let layout = Layout::from_size_align_unchecked(os, GC_ALIGN);
            alloc::dealloc(ptr, layout);
        }
        std::ptr::null_mut()
    } else if ptr.is_null() {
        // C: frealloc(null, tag, ns) → new allocation of ns bytes.
        // `os` is a GC type tag in this path; ignored by the default allocator.
        let layout = match Layout::from_size_align(ns, GC_ALIGN) {
            Ok(l) => l,
            // Layout overflow → treat as allocation failure; caller handles NULL.
            Err(_) => return std::ptr::null_mut(),
        };
        // SAFETY: `layout` is well-formed (just validated by `from_size_align`).
        alloc::alloc(layout)
    } else {
        // C: frealloc(p, os, ns) → resize.
        debug_assert!(os > 0, "reallocating non-null block with osize == 0");
        // SAFETY: `ptr` was allocated with `GC_ALIGN` and size `os` — caller guarantee.
        let old_layout = Layout::from_size_align_unchecked(os, GC_ALIGN);
        // SAFETY: `ns > 0` (checked above); `GC_ALIGN` is a valid power-of-two.
        alloc::realloc(ptr, old_layout, ns)
    }
}

// ---------------------------------------------------------------------------
// `can_try_again` predicate
// ---------------------------------------------------------------------------

/// C: `#define cantryagain(g)  (completestate(g) && !g->gcstopem)`
///
/// True when an emergency GC collection is safe: the interpreter has been
/// fully initialized (`completestate`) and is not already mid-collection
/// (`gcstopem`). Only in this state may a failed allocation trigger a retry.
#[inline]
fn can_try_again(is_complete: bool, gcstopem: bool) -> bool {
    // C: completestate(g) → g.is_complete()
    // C: !g->gcstopem    → !g.gcstopem
    is_complete && !gcstopem
}

// ---------------------------------------------------------------------------
// `first_try`: first allocation attempt (hooks into EMERGENCYGCTESTS)
// ---------------------------------------------------------------------------

/// C: `firsttry(g, block, os, ns)` macro / conditional function.
///
/// In the normal build, identical to `call_realloc` (the macro definition).
/// With the `emergency-gc-tests` Cargo feature enabled, deliberately returns
/// null on every allocation that *could* retry — forcing a full GC cycle on
/// each allocation to stress-test the emergency path.
///
/// # Safety
/// Same as `call_realloc`.
#[cfg(feature = "emergency-gc-tests")]
unsafe fn first_try(
    is_complete: bool,
    gcstopem: bool,
    ptr: *mut u8,
    os: usize,
    ns: usize,
) -> *mut u8 {
    // C: if (ns > 0 && cantryagain(g)) return NULL;  /* fail */
    if ns > 0 && can_try_again(is_complete, gcstopem) {
        return std::ptr::null_mut();
    }
    // C: else return callfrealloc(g, block, os, ns);
    // SAFETY: forwarding caller's ptr/os/ns contract.
    call_realloc(ptr, os, ns)
}

#[cfg(not(feature = "emergency-gc-tests"))]
#[inline]
unsafe fn first_try(
    _is_complete: bool,
    _gcstopem: bool,
    ptr: *mut u8,
    os: usize,
    ns: usize,
) -> *mut u8 {
    // C: #define firsttry(g,block,os,ns)  callfrealloc(g, block, os, ns)
    // SAFETY: forwarding caller's ptr/os/ns contract.
    call_realloc(ptr, os, ns)
}

// ---------------------------------------------------------------------------
// `try_again`: emergency-GC retry (static in C)
// ---------------------------------------------------------------------------

/// C: `static void *tryagain (lua_State *L, void *block, size_t osize, size_t nsize)`
///
/// Called after a first allocation attempt fails. If the GC is in a safe
/// state to run (`can_try_again`), triggers a full emergency collection then
/// retries. Otherwise returns null immediately.
///
/// C: `luaC_fullgc(L, 1)` → `crate::gc::full_collect(state)` (intra-crate).
///
/// # Safety
/// - `ptr` and `osize` obey the same invariant as `call_realloc`.
///
/// TODO(port): `LuaState` — see module-level note on circular dependency.
/// The `is_complete`/`gcstopem` parameters are extracted from
/// `state.global()` at each call site and passed in to avoid holding
/// `&GlobalState` across the mutable `full_collect` call.
unsafe fn try_again(
    is_complete: bool,
    gcstopem: bool,
    // TODO(port): replace with state: &mut LuaState once crate graph is resolved.
    full_collect: &mut dyn FnMut(),
    ptr: *mut u8,
    osize: usize,
    nsize: usize,
) -> *mut u8 {
    // C: if (cantryagain(g)) {
    if can_try_again(is_complete, gcstopem) {
        // C: luaC_fullgc(L, 1);  /* try to free some memory... */
        full_collect();
        // C: return callfrealloc(g, block, osize, nsize);  /* try again */
        // SAFETY: ptr/osize contract forwarded from caller; nsize unchanged.
        call_realloc(ptr, osize, nsize)
    } else {
        // C: else return NULL;  /* cannot run an emergency collection */
        std::ptr::null_mut()
    }
}

// ---------------------------------------------------------------------------
// Public / crate-internal allocator functions
// ---------------------------------------------------------------------------

/// C: `l_noret luaM_toobig (lua_State *L)`
///
/// Raises a memory error when an allocation request exceeds the maximum
/// representable block size. In C this calls `luaG_runerror(L, "memory
/// allocation error: block too big")` which diverges via longjmp. In Rust
/// we return `LuaError::Memory` for the caller to propagate with `?`.
///
/// error_sites.tsv: `luaM_toobig(L)` → `return Err(LuaError::Memory)`.
///
/// PORT NOTE: The C error message goes through `luaG_runerror` (LUA_ERRRUN).
/// The TSV maps this to `LuaError::Memory` (LUA_ERRMEM); we follow the TSV.
pub(crate) fn too_big() -> LuaError {
    // C: luaG_runerror(L, "memory allocation error: block too big");
    LuaError::Memory
}

/// C: `void luaM_free_ (lua_State *L, void *block, size_t osize)`
///
/// Frees a raw GC block and decrements `GlobalState.GCdebt` by `osize`.
///
/// macros.tsv: `luaM_free` / `luaM_freemem` → "Rust's Drop handles
/// deallocation; drop the call". This is the *underlying implementation*
/// for those macros; Phase D must retain it for objects not managed by
/// Rust `Drop`.
///
/// # Safety
/// - `block` was allocated by `malloc_` or `realloc_` in this module with
///   byte count `osize` and alignment `GC_ALIGN`.
/// - After this call, `block` must not be dereferenced.
///
/// TODO(port): Takes `gc_debt: &mut isize` extracted from `state.global_mut()`
/// because `LuaState` is not directly available here (circular dep). Phase B
/// replaces with `state: &mut LuaState`.
pub(crate) unsafe fn free_(gc_debt: &mut isize, block: *mut u8, osize: usize) {
    // C: global_State *g = G(L);   [G(L) → state.global()]
    // C: lua_assert((osize == 0) == (block == NULL));
    debug_assert!((osize == 0) == block.is_null());
    // C: callfrealloc(g, block, osize, 0);
    // SAFETY: `block` was allocated by this module with `osize` and `GC_ALIGN` — caller guarantee.
    call_realloc(block, osize, 0);
    // C: g->GCdebt -= osize;
    *gc_debt -= osize as isize;
}

/// C: `void *luaM_realloc_ (lua_State *L, void *block, size_t osize, size_t nsize)`
///
/// Generic allocation / reallocation routine. Updates `GCdebt` on success.
/// On first-attempt failure, tries an emergency GC cycle (via `try_again`).
/// Returns null if both attempts fail; callers that cannot tolerate failure
/// must use `safe_realloc_` instead.
///
/// C: `lua_assert((osize == 0) == (block == NULL))` — enforced as `debug_assert!`.
/// C: `lua_assert((nsize == 0) == (newblock == NULL))` — enforced as `debug_assert!`.
///
/// # Safety
/// - `block` is either null (new allocation) or was returned by this module
///   with byte count `osize` and alignment `GC_ALIGN`.
/// - If `nsize == 0`, the returned pointer is null and `block` is freed.
///
/// TODO(port): Caller-facing API; see module-level circular-dependency note.
pub(crate) unsafe fn realloc_(
    gc_debt: &mut isize,
    is_complete: bool,
    gcstopem: bool,
    full_collect: &mut dyn FnMut(),
    ptr: *mut u8,
    osize: usize,
    nsize: usize,
) -> *mut u8 {
    // C: lua_assert((osize == 0) == (block == NULL));
    debug_assert!((osize == 0) == ptr.is_null());
    // C: newblock = firsttry(g, block, osize, nsize);
    // SAFETY: forwarding ptr/osize/nsize contract.
    let mut newblock = first_try(is_complete, gcstopem, ptr, osize, nsize);
    // C: if (l_unlikely(newblock == NULL && nsize > 0)) {
    if newblock.is_null() && nsize > 0 {
        // C: newblock = tryagain(L, block, osize, nsize);
        // SAFETY: forwarding ptr/osize/nsize contract; try_again runs GC then retries.
        newblock = try_again(is_complete, gcstopem, full_collect, ptr, osize, nsize);
        // C: if (newblock == NULL) return NULL;  /* do not update 'GCdebt' */
        if newblock.is_null() {
            return std::ptr::null_mut();
        }
    }
    // C: lua_assert((nsize == 0) == (newblock == NULL));
    debug_assert!((nsize == 0) == newblock.is_null());
    // C: g->GCdebt = (g->GCdebt + nsize) - osize;
    *gc_debt = (*gc_debt + nsize as isize) - osize as isize;
    newblock
}

/// C: `void *luaM_saferealloc_ (lua_State *L, void *block, size_t osize, size_t nsize)`
///
/// Like `realloc_` but propagates `LuaError::Memory` if allocation fails after
/// the emergency GC retry. Used in contexts where failure is unrecoverable
/// (e.g. shrinking parser arrays to their exact final size).
///
/// error_sites.tsv: `luaM_saferealloc_ returning NULL` → `Err(LuaError::Memory)`.
///
/// # Safety
/// Same as `realloc_`.
///
/// TODO(port): Caller-facing API; see module-level circular-dependency note.
pub(crate) unsafe fn safe_realloc_(
    gc_debt: &mut isize,
    is_complete: bool,
    gcstopem: bool,
    full_collect: &mut dyn FnMut(),
    ptr: *mut u8,
    osize: usize,
    nsize: usize,
) -> Result<*mut u8, LuaError> {
    // C: void *newblock = luaM_realloc_(L, block, osize, nsize);
    // SAFETY: forwarding ptr/osize/nsize contract.
    let newblock = realloc_(gc_debt, is_complete, gcstopem, full_collect, ptr, osize, nsize);
    // C: if (l_unlikely(newblock == NULL && nsize > 0)) luaM_error(L);
    // C: luaM_error(L) → luaD_throw(L, LUA_ERRMEM)
    if newblock.is_null() && nsize > 0 {
        return Err(LuaError::Memory);
    }
    Ok(newblock)
}

/// C: `void *luaM_malloc_ (lua_State *L, size_t size, int tag)`
///
/// Allocates `size` bytes of new (untracked) memory for a GC object of type
/// `tag`. The `tag` is passed as `osize` to the allocator hook per Lua
/// convention; our `std::alloc` shim ignores it.
///
/// Returns `Err(LuaError::Memory)` if neither the first attempt nor an
/// emergency GC cycle can satisfy the request.
///
/// # Safety
/// The caller must eventually release the returned pointer via `free_` or
/// `realloc_` with the original `size` as `osize`.
///
/// TODO(port): Caller-facing API; see module-level circular-dependency note.
pub(crate) unsafe fn malloc_(
    gc_debt: &mut isize,
    is_complete: bool,
    gcstopem: bool,
    full_collect: &mut dyn FnMut(),
    size: usize,
    tag: usize,
) -> Result<*mut u8, LuaError> {
    // C: if (size == 0) return NULL;  /* that's all */
    if size == 0 {
        return Ok(std::ptr::null_mut());
    }
    // C: global_State *g = G(L);
    // C: void *newblock = firsttry(g, NULL, tag, size);
    // SAFETY: null pointer with `tag` as osize per Lua allocator convention.
    let mut newblock = first_try(is_complete, gcstopem, std::ptr::null_mut(), tag, size);
    // C: if (l_unlikely(newblock == NULL)) {
    if newblock.is_null() {
        // C: newblock = tryagain(L, NULL, tag, size);
        // SAFETY: null pointer; try_again passes it through to call_realloc.
        newblock = try_again(
            is_complete,
            gcstopem,
            full_collect,
            std::ptr::null_mut(),
            tag,
            size,
        );
        // C: if (newblock == NULL) luaM_error(L);
        if newblock.is_null() {
            return Err(LuaError::Memory);
        }
    }
    // C: g->GCdebt += size;
    *gc_debt += size as isize;
    Ok(newblock)
}

// ---------------------------------------------------------------------------
// Parser array helpers
// ---------------------------------------------------------------------------

/// C: `void *luaM_growaux_ (lua_State *L, void *block, int nelems, int *psize,
///                           int size_elems, int limit, const char *what)`
///
/// Grows a parser dynamic array to fit at least `nelems + 1` elements.
/// Returns the (possibly-moved) block; updates `*psize` only on success.
///
/// Growth strategy:
/// - `nelems + 1 <= *psize` → nothing to do, return block unchanged.
/// - `*psize >= limit / 2` but `*psize < limit` → cap at `limit`.
/// - `*psize >= limit` → raise a runtime error via `LuaError::runtime`.
/// - Otherwise double `*psize`, clamping up to `MINSIZEARRAY`.
///
/// `what` is a Rust-side identifier (e.g. `"constants"`, `"locals"`) used
/// only in error messages; `&str` is appropriate per PORTING.md §1.
///
/// `size_elems` is the byte size of one array element.
///
/// # Safety
/// - `block` is either null (array not yet allocated) or was returned by
///   this module with byte count `(*psize as usize) * (size_elems as usize)`
///   and alignment `GC_ALIGN`.
/// - `nelems >= 0` and `limit > 0` and `size_elems > 0`.
///
/// TODO(port): Phase B should replace the `void *`-style `*mut u8` block with
/// a typed `Vec<T>` per array kind; the raw interface is preserved here to
/// match C structure faithfully.
/// TODO(port): Caller-facing API; see module-level circular-dependency note.
pub(crate) unsafe fn grow_aux_(
    gc_debt: &mut isize,
    is_complete: bool,
    gcstopem: bool,
    full_collect: &mut dyn FnMut(),
    block: *mut u8,
    nelems: i32,
    psize: &mut i32,
    size_elems: i32,
    limit: i32,
    what: &str,
) -> Result<*mut u8, LuaError> {
    // C: int size = *psize;
    let mut size = *psize;
    // C: if (nelems + 1 <= size) return block;  /* nothing to be done */
    if nelems + 1 <= size {
        return Ok(block);
    }
    // C: if (size >= limit / 2) {  /* cannot double it? */
    if size >= limit / 2 {
        // C: if (l_unlikely(size >= limit))  /* cannot grow even a little? */
        if size >= limit {
            // C: luaG_runerror(L, "too many %s (limit is %d)", what, limit);
            return Err(LuaError::runtime(format_args!(
                "too many {} (limit is {})",
                what, limit
            )));
        }
        // C: size = limit;  /* still have at least one free place */
        size = limit;
    } else {
        // C: size *= 2;
        size *= 2;
        // C: if (size < MINSIZEARRAY) size = MINSIZEARRAY;  /* minimum size */
        if size < MINSIZEARRAY {
            size = MINSIZEARRAY;
        }
    }
    // C: lua_assert(nelems + 1 <= size && size <= limit);
    debug_assert!(nelems + 1 <= size && size <= limit);
    // C: newblock = luaM_saferealloc_(L, block,
    //                  cast_sizet(*psize) * size_elems,
    //                  cast_sizet(size)  * size_elems);
    // cast_sizet(x) → x as usize  (macros.tsv)
    let old_byte_len = (*psize as usize) * (size_elems as usize);
    let new_byte_len = (size as usize) * (size_elems as usize);
    // SAFETY: block + old_byte_len consistent with prior allocation (caller invariant).
    let newblock = safe_realloc_(
        gc_debt,
        is_complete,
        gcstopem,
        full_collect,
        block,
        old_byte_len,
        new_byte_len,
    )?;
    // C: *psize = size;  /* update only when everything else is OK */
    *psize = size;
    Ok(newblock)
}

/// C: `void *luaM_shrinkvector_ (lua_State *L, void *block, int *size,
///                                int final_n, int size_elem)`
///
/// Shrinks a parser array to exactly `final_n` elements. Prototype arrays
/// carry no slack capacity (size == count), so failure here is unrecoverable;
/// `safe_realloc_` raises `LuaError::Memory` on OOM.
///
/// `size_elem` is the byte size of one element.
///
/// # Safety
/// - `block` was returned by this module with byte count
///   `(*size as usize) * (size_elem as usize)` and alignment `GC_ALIGN`.
/// - `final_n <= *size`.
///
/// TODO(port): Same `void *` caveat as `grow_aux_`; Phase B replaces with `Vec<T>`.
/// TODO(port): Caller-facing API; see module-level circular-dependency note.
pub(crate) unsafe fn shrink_vector_(
    gc_debt: &mut isize,
    is_complete: bool,
    gcstopem: bool,
    full_collect: &mut dyn FnMut(),
    block: *mut u8,
    size: &mut i32,
    final_n: i32,
    size_elem: i32,
) -> Result<*mut u8, LuaError> {
    // C: size_t oldsize = cast_sizet((*size) * size_elem);
    let oldsize = (*size as usize) * (size_elem as usize);
    // C: size_t newsize = cast_sizet(final_n * size_elem);
    let newsize = (final_n as usize) * (size_elem as usize);
    // C: lua_assert(newsize <= oldsize);
    debug_assert!(newsize <= oldsize);
    // C: newblock = luaM_saferealloc_(L, block, oldsize, newsize);
    // SAFETY: block + oldsize consistent with prior allocation (caller invariant).
    let newblock = safe_realloc_(
        gc_debt,
        is_complete,
        gcstopem,
        full_collect,
        block,
        oldsize,
        newsize,
    )?;
    // C: *size = final_n;
    *size = final_n;
    Ok(newblock)
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lmem.c  (216 lines, 8 functions)
//   target_crate:  lua-gc
//   confidence:    medium
//   todos:         13
//   port_notes:    1
//   unsafe_blocks: 10
//   notes:         Logic and GC-debt accounting ported faithfully. The primary
//                  Phase B blocker is the circular crate dependency: lmem.c
//                  takes `lua_State *L` but LuaState lives in lua-vm (above
//                  lua-gc). Function signatures decouple the GC-state fields
//                  into plain parameters (`gc_debt`, `is_complete`, `gcstopem`,
//                  `full_collect`) as a workaround; Phase B should replace
//                  these with a `GcHost` trait or by moving mem.rs into lua-vm.
//                  The raw `std::alloc` shim in `call_realloc` uses a fixed
//                  GC_ALIGN=8 which Phase D must replace with per-object Layout.
//                  `unsafe_blocks` counts 10 `unsafe fn` declarations (no bare
//                  `unsafe { }` blocks); all carry `// SAFETY:` annotations.
// ──────────────────────────────────────────────────────────────────────────
