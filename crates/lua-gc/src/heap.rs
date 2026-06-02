//! Phase D mark-and-sweep garbage collector.
//!
//! This module is the production GC for the Lua runtime, replacing the
//! `Rc<T>`-backed `GcRef<T>` placeholder used through Phase B/C. It is a
//! single-threaded precise tracing collector with incremental and
//! generational paths plus forward/backward write barriers.
//!
//! # Vocabulary
//!
//! - **Gc<T>**: a pointer-sized handle. `Copy + Clone`. Replaces `GcRef<T>`.
//! - **GcBox<T>**: the heap allocation; contains a header and the value.
//! - **GcHeader**: per-object metadata (color, age, finalized flag, intrusive
//!   `next` pointer for exactly one heap owner list, and grayagain revisit link).
//! - **Trace**: trait every GC-rooted type implements. The `trace` method
//!   walks all `Gc<_>` fields and calls `Marker::mark` on each.
//! - **Marker**: passed to `trace`; carries the gray queue.
//! - **Heap**: owns the allgc/finobj/tobefnz list heads, byte counters, and
//!   GC state machine.
//!
//! # Safety model
//!
//! All `unsafe` is confined to this crate (per `harness/unsafe-budgets.toml`).
//! The invariants are:
//!
//! 1. Every `Gc<T>` points to a valid, allocated, not-yet-swept `GcBox<T>`.
//! 2. The intrusive heap lists are consistent: traversing `header.next` from
//!    `Heap.head`, `Heap.finobj`, and `Heap.tobefnz` reaches every live
//!    heap-owned `GcBox` exactly once.
//! 3. After `Heap::full_collect(roots)`, every `Gc<T>` reachable from `roots`
//!    is still valid; unreachable boxes are dropped and deallocated.
//!
//! # Migration shape
//!
//! Existing code holds `GcRef<T>` (which after Phase D is a type alias for
//! `Gc<T>`). Legacy call sites like `GcRef::new(value)` route through
//! `Gc::new_uncollected` which allocates a `GcBox` but does NOT register it
//! in any heap. Phase D-1b agent work converts these to
//! `state.heap().allocate(value)` so the new box joins the heap owner lists.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::ptr::NonNull;

// ──────────────────────────────────────────────────────────────────────────
// Phase D-1c — scoped thread-local HeapGuard
//
// Lua's C API supports multiple `lua_State`s on one OS thread (sandbox-per-
// state is a real embedding pattern). We honor that by stacking the
// currently-active heap rather than holding a single slot. `HeapGuard::push`
// activates a heap; drop pops it.
//
// `with_current_heap(...)` exposes the top of the stack only for the dynamic
// extent of a closure — used by `GcRef::new` call sites that don't have
// `&mut LuaState` in scope.
// ──────────────────────────────────────────────────────────────────────────

thread_local! {
    static CURRENT_HEAP_STACK: RefCell<Vec<NonNull<Heap>>> = const { RefCell::new(Vec::new()) };
}

/// A scoped guard for the currently-active heap. Pushed at entry to
/// `state.run()` / `state.protected_call()` / `state.load()`; popped on
/// drop. Supports nesting (multiple LuaStates on one thread).
pub struct HeapGuard {
    // Anchor a NonNull so the user can't accidentally drop the guard while
    // an inner Lua state is still active. We rely on RAII.
    _private: (),
}

impl HeapGuard {
    /// Push `heap` onto the active stack. Returns a guard; dropping it pops.
    ///
    /// # Safety
    ///
    /// The pointer must remain valid for the lifetime of the guard. Callers
    /// typically pass `&state.global.heap`, which lives as long as the
    /// `GlobalState` (an `Rc<RefCell<_>>`); the guard must drop before the
    /// state is dropped.
    pub fn push(heap: &Heap) -> Self {
        let ptr = NonNull::from(heap);
        CURRENT_HEAP_STACK.with(|stack| stack.borrow_mut().push(ptr));
        HeapGuard { _private: () }
    }
}

impl Drop for HeapGuard {
    fn drop(&mut self) {
        CURRENT_HEAP_STACK.with(|stack| {
            let popped = stack.borrow_mut().pop();
            debug_assert!(popped.is_some(), "HeapGuard::drop with empty stack");
        });
    }
}

/// Runs `f` with a reference to the currently-active heap, or `None` if no
/// `HeapGuard` is in scope.
///
/// The heap reference is deliberately scoped to the closure. This avoids the
/// previous `current_heap() -> Option<&'static Heap>` lifetime lie while still
/// supporting legacy `GcRef::new` call sites that do not receive `&mut LuaState`.
pub fn with_current_heap<R>(f: impl for<'a> FnOnce(Option<&'a Heap>) -> R) -> R {
    CURRENT_HEAP_STACK.with(|stack| {
        let ptr = stack.borrow().last().copied();
        // SAFETY: the top NonNull was produced from a live `&Heap` whose
        // lifetime is bounded by the corresponding `HeapGuard`. The reference
        // is only handed to `f`, and cannot escape through the return type.
        let heap = ptr.map(|ptr| unsafe { &*ptr.as_ptr() });
        f(heap)
    })
}

#[derive(Copy, Clone, Debug)]
pub struct HeapRef {
    ptr: NonNull<Heap>,
}

impl HeapRef {
    pub fn from_heap(heap: &Heap) -> Self {
        HeapRef {
            ptr: NonNull::from(heap),
        }
    }

    pub fn contains_allocation(self, identity: usize, token: usize) -> bool {
        // SAFETY: `HeapRef` is created only from a live `&Heap`. Runtime-owned
        // weak handles store it inside `GlobalState`, whose heap field outlives
        // those handles. The method only traverses heap metadata and never
        // dereferences the weak target pointer.
        unsafe { self.ptr.as_ref() }.contains_allocation(identity, token)
    }
}

/// A traced color in the tri-color invariant.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Color {
    /// Not yet visited in the current cycle. The collector alternates between
    /// two white bits so allocations made during sweep are not collected by
    /// the cycle already in progress.
    White0,
    /// Alternate white bit.
    White1,
    /// Visited; outgoing references not yet traced.
    Gray,
    /// Fully traced; no outgoing pointers to white objects.
    Black,
}

impl Color {
    pub fn is_white(self) -> bool {
        matches!(self, Color::White0 | Color::White1)
    }

    fn other_white(self) -> Self {
        match self {
            Color::White0 => Color::White1,
            Color::White1 => Color::White0,
            Color::Gray | Color::Black => self,
        }
    }
}

/// Object age used by Lua's generational collector.
///
/// Mirrors `G_NEW` through `G_TOUCHED2` in `lgc.h`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum GcAge {
    New,
    Survival,
    Old0,
    Old1,
    Old,
    Touched1,
    Touched2,
}

impl GcAge {
    pub fn is_old(self) -> bool {
        !matches!(self, GcAge::New | GcAge::Survival)
    }

    fn next_after_minor(self) -> Self {
        match self {
            GcAge::New => GcAge::Survival,
            GcAge::Survival | GcAge::Old0 => GcAge::Old1,
            GcAge::Old1 | GcAge::Old | GcAge::Touched2 => GcAge::Old,
            GcAge::Touched1 => GcAge::Touched2,
        }
    }
}

/// Minimal metadata a finalizable VM object must expose for collector-owned
/// finalizer-list bookkeeping.
pub trait FinalizerEntry: Clone {
    fn identity(&self) -> usize;
    fn heap_ptr(&self) -> Option<NonNull<GcBox<dyn Trace>>> {
        None
    }
    fn age(&self) -> GcAge;
    fn is_finalized(&self) -> bool;
    fn set_finalized(&self, finalized: bool);
}

/// Minimal operations needed for collector-owned weak-registry bookkeeping.
pub trait WeakEntry: Clone {
    type Strong: Clone;

    fn identity(&self) -> usize;
    fn list_kind(&self) -> WeakListKind;
    fn upgrade(&self) -> Option<Self::Strong>;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WeakListKind {
    WeakValues,
    Ephemeron,
    AllWeak,
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct WeakRegistryStats {
    pub tracked: usize,
    pub snapshot_live: usize,
    pub snapshot_dead: usize,
    pub retained: usize,
    pub weak_values: usize,
    pub ephemeron: usize,
    pub all_weak: usize,
}

#[derive(Clone, Debug)]
pub struct WeakRegistry<T: WeakEntry> {
    weak_values: Vec<T>,
    ephemeron: Vec<T>,
    all_weak: Vec<T>,
    last_stats: WeakRegistryStats,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeakRegistrySnapshot<T> {
    pub weak_values: Vec<T>,
    pub ephemeron: Vec<T>,
    pub all_weak: Vec<T>,
}

impl<T> Default for WeakRegistrySnapshot<T> {
    fn default() -> Self {
        Self {
            weak_values: Vec::new(),
            ephemeron: Vec::new(),
            all_weak: Vec::new(),
        }
    }
}

impl<T> WeakRegistrySnapshot<T> {
    pub fn len(&self) -> usize {
        self.weak_values
            .len()
            .saturating_add(self.ephemeron.len())
            .saturating_add(self.all_weak.len())
    }

    pub fn into_flat(self) -> Vec<T> {
        self.weak_values
            .into_iter()
            .chain(self.ephemeron)
            .chain(self.all_weak)
            .collect()
    }
}

impl<T: WeakEntry> Default for WeakRegistry<T> {
    fn default() -> Self {
        Self {
            weak_values: Vec::new(),
            ephemeron: Vec::new(),
            all_weak: Vec::new(),
            last_stats: WeakRegistryStats::default(),
        }
    }
}

impl<T: WeakEntry> WeakRegistry<T> {
    pub fn len(&self) -> usize {
        self.weak_values
            .len()
            .saturating_add(self.ephemeron.len())
            .saturating_add(self.all_weak.len())
    }

    pub fn stats(&self) -> WeakRegistryStats {
        self.last_stats
    }

    fn list_mut(&mut self, kind: WeakListKind) -> &mut Vec<T> {
        match kind {
            WeakListKind::WeakValues => &mut self.weak_values,
            WeakListKind::Ephemeron => &mut self.ephemeron,
            WeakListKind::AllWeak => &mut self.all_weak,
        }
    }

    pub fn remove_identity(&mut self, id: usize) {
        self.weak_values.retain(|entry| entry.identity() != id);
        self.ephemeron.retain(|entry| entry.identity() != id);
        self.all_weak.retain(|entry| entry.identity() != id);
        self.last_stats.tracked = self.len();
        self.last_stats.retained = self.len();
        self.update_cohort_stats();
    }

    fn update_cohort_stats(&mut self) {
        self.last_stats.weak_values = self.weak_values.len();
        self.last_stats.ephemeron = self.ephemeron.len();
        self.last_stats.all_weak = self.all_weak.len();
    }

    pub fn push_unique(&mut self, entry: T) {
        let id = entry.identity();
        self.remove_identity(id);
        self.list_mut(entry.list_kind()).push(entry);
        self.last_stats.tracked = self.len();
        self.last_stats.retained = self.len();
        self.update_cohort_stats();
    }

    pub fn live_snapshot_by_kind(&mut self) -> WeakRegistrySnapshot<T::Strong> {
        let tracked_before = self.len();
        let mut seen = std::collections::HashSet::<usize>::new();
        let mut live = WeakRegistrySnapshot::default();
        let mut dead = 0usize;

        let entries = std::mem::take(&mut self.weak_values)
            .into_iter()
            .chain(std::mem::take(&mut self.ephemeron))
            .chain(std::mem::take(&mut self.all_weak));
        for entry in entries {
            if !seen.insert(entry.identity()) {
                continue;
            }
            match entry.upgrade() {
                Some(strong) => {
                    match entry.list_kind() {
                        WeakListKind::WeakValues => live.weak_values.push(strong),
                        WeakListKind::Ephemeron => live.ephemeron.push(strong),
                        WeakListKind::AllWeak => live.all_weak.push(strong),
                    }
                    self.list_mut(entry.list_kind()).push(entry);
                }
                None => dead += 1,
            }
        }

        self.last_stats = WeakRegistryStats {
            tracked: tracked_before,
            snapshot_live: live.len(),
            snapshot_dead: dead,
            retained: self.len(),
            weak_values: self.weak_values.len(),
            ephemeron: self.ephemeron.len(),
            all_weak: self.all_weak.len(),
        };
        live
    }

    pub fn live_snapshot(&mut self) -> Vec<T::Strong> {
        self.live_snapshot_by_kind().into_flat()
    }

    pub fn retain_identities(&mut self, ids: &std::collections::HashSet<usize>) {
        self.weak_values.retain(|entry| ids.contains(&entry.identity()));
        self.ephemeron.retain(|entry| ids.contains(&entry.identity()));
        self.all_weak.retain(|entry| ids.contains(&entry.identity()));
        self.last_stats.retained = self.len();
        self.last_stats.tracked = self.len();
        self.update_cohort_stats();
    }
}

#[derive(Clone, Debug)]
pub struct FinalizerRegistry<T: FinalizerEntry> {
    pending: Vec<T>,
    to_be_finalized: Vec<T>,
    pending_reallyold: usize,
    pending_old1: usize,
    pending_survival: usize,
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct FinalizerRegistryStats {
    pub pending_young: usize,
    pub pending_old: usize,
    pub to_be_finalized_young: usize,
    pub to_be_finalized_old: usize,
    pub finobj_new: usize,
    pub finobj_survival: usize,
    pub finobj_old1: usize,
    pub finobj_reallyold: usize,
    pub finobj_minor_scan: usize,
}

impl<T: FinalizerEntry> Default for FinalizerRegistry<T> {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            to_be_finalized: Vec::new(),
            pending_reallyold: 0,
            pending_old1: 0,
            pending_survival: 0,
        }
    }
}

impl<T: FinalizerEntry> FinalizerRegistry<T> {
    fn pending_new_len(&self) -> usize {
        self.pending.len().saturating_sub(
            self.pending_reallyold
                .saturating_add(self.pending_old1)
                .saturating_add(self.pending_survival),
        )
    }

    fn minor_scan_start(&self) -> usize {
        self.pending_reallyold.saturating_add(self.pending_old1)
    }

    fn debug_assert_pending_cohorts(&self) {
        debug_assert!(
            self.pending_reallyold
                .saturating_add(self.pending_old1)
                .saturating_add(self.pending_survival)
                <= self.pending.len()
        );
    }

    pub fn pending(&self) -> &[T] {
        &self.pending
    }

    pub fn pending_snapshot(&self) -> Vec<T> {
        self.pending.clone()
    }

    pub fn pending_minor_snapshot(&self) -> Vec<T> {
        self.pending[self.minor_scan_start().min(self.pending.len())..].to_vec()
    }

    pub fn to_be_finalized(&self) -> &[T] {
        &self.to_be_finalized
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn to_be_finalized_len(&self) -> usize {
        self.to_be_finalized.len()
    }

    pub fn has_to_be_finalized(&self) -> bool {
        !self.to_be_finalized.is_empty()
    }

    pub fn stats(&self) -> FinalizerRegistryStats {
        fn count_by_age<T: FinalizerEntry>(objects: &[T]) -> (usize, usize) {
            objects.iter().fold((0usize, 0usize), |(young, old), object| {
                if object.age().is_old() {
                    (young, old + 1)
                } else {
                    (young + 1, old)
                }
            })
        }
        let (pending_young, pending_old) = count_by_age(&self.pending);
        let (to_be_finalized_young, to_be_finalized_old) =
            count_by_age(&self.to_be_finalized);
        FinalizerRegistryStats {
            pending_young,
            pending_old,
            to_be_finalized_young,
            to_be_finalized_old,
            finobj_new: self.pending_new_len(),
            finobj_survival: self.pending_survival,
            finobj_old1: self.pending_old1,
            finobj_reallyold: self.pending_reallyold,
            finobj_minor_scan: self.pending.len().saturating_sub(self.minor_scan_start()),
        }
    }

    pub fn push_pending_unique(&mut self, object: T) -> bool {
        if object.is_finalized() {
            return false;
        }
        let id = object.identity();
        if !self.pending.iter().any(|o| o.identity() == id) {
            object.set_finalized(true);
            self.pending.push(object);
            self.debug_assert_pending_cohorts();
            true
        } else {
            false
        }
    }

    pub fn take_pending(&mut self) -> Vec<T> {
        self.pending_reallyold = 0;
        self.pending_old1 = 0;
        self.pending_survival = 0;
        std::mem::take(&mut self.pending)
    }

    fn retain_pending_not_in(&mut self, ids: &std::collections::HashSet<usize>) {
        if ids.is_empty() {
            return;
        }
        let original_reallyold = self.pending_reallyold;
        let original_old1 = self.pending_old1;
        let original_survival = self.pending_survival;
        let mut retained_reallyold = original_reallyold;
        let mut retained_old1 = original_old1;
        let mut retained_survival = original_survival;
        let mut retained = Vec::with_capacity(self.pending.len());
        for (index, object) in std::mem::take(&mut self.pending).into_iter().enumerate() {
            if ids.contains(&object.identity()) {
                if index < original_reallyold {
                    retained_reallyold -= 1;
                } else if index < original_reallyold + original_old1 {
                    retained_old1 -= 1;
                } else if index < original_reallyold + original_old1 + original_survival {
                    retained_survival -= 1;
                }
            } else {
                retained.push(object);
            }
        }
        self.pending = retained;
        self.pending_reallyold = retained_reallyold;
        self.pending_old1 = retained_old1;
        self.pending_survival = retained_survival;
        self.debug_assert_pending_cohorts();
    }

    pub fn push_to_be_finalized(&mut self, object: T) {
        object.set_finalized(true);
        self.to_be_finalized.push(object);
    }

    fn extend_to_be_finalized(&mut self, objects: Vec<T>) -> Vec<T> {
        let drain_order: Vec<T> = objects.into_iter().rev().collect();
        for object in drain_order.iter().cloned() {
            self.push_to_be_finalized(object);
        }
        drain_order
    }

    pub fn promote_pending_to_finalized(&mut self, objects: Vec<T>) -> Vec<T> {
        if objects.is_empty() {
            return Vec::new();
        }
        let ids: std::collections::HashSet<usize> =
            objects.iter().map(|object| object.identity()).collect();
        self.retain_pending_not_in(&ids);
        self.extend_to_be_finalized(objects)
    }

    pub fn promote_all_pending_to_old(&mut self) {
        self.pending_reallyold = self.pending.len();
        self.pending_old1 = 0;
        self.pending_survival = 0;
    }

    pub fn reset_generation_boundaries(&mut self) {
        self.pending_reallyold = 0;
        self.pending_old1 = 0;
        self.pending_survival = 0;
    }

    pub fn finish_minor_collection(&mut self) {
        let new = self.pending_new_len();
        self.pending_reallyold = self.pending_reallyold.saturating_add(self.pending_old1);
        self.pending_old1 = self.pending_survival;
        self.pending_survival = new;
        self.debug_assert_pending_cohorts();
    }

    pub fn pop_to_be_finalized(&mut self) -> Option<T> {
        let object = if self.to_be_finalized.is_empty() {
            None
        } else {
            Some(self.to_be_finalized.remove(0))
        };
        if let Some(ref object) = object {
            object.set_finalized(false);
        }
        object
    }
}

/// Per-object GC metadata. Lives at the start of every `GcBox`.
#[repr(C)]
pub struct GcHeader {
    color: Cell<Color>,
    age: Cell<GcAge>,
    /// Mirrors C-Lua's FINALIZEDBIT: true while the object is registered in a
    /// pending/to-be-finalized list. Cleared when the object is popped for its
    /// `__gc` call.
    finalized: Cell<bool>,
    /// True iff this box is linked into one of a heap's owner lists, so it will be
    /// swept and its `size` refunded. `new_uncollected` boxes leave this
    /// false: they never join a chain, are never swept, and so must never
    /// have buffer bytes charged against the pacer (the charge would never be
    /// refunded). [`Gc::account_buffer`] is a no-op when this is false.
    ///
    /// Kept separate from `size`: `collected` controls whether buffer charges
    /// are refundable; `size` remains the exact byte count refunded by sweep.
    collected: Cell<bool>,
    /// Intrusive link into exactly one heap owner list.
    next: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// Intrusive link into the collector's grayagain-style revisit list.
    gray_next: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// True while this object is linked into the grayagain revisit list.
    gray_listed: Cell<bool>,
    /// Rough byte size charged to the pacer for this object. Starts at the
    /// `GcBox<T>` size and is adjusted in place by [`Gc::account_buffer`] when
    /// the value's owned heap buffers (table array/node Vecs) grow or shrink.
    /// Invariant: this is always exactly the amount sweep will refund to the
    /// heap's byte counter when this object is freed.
    size: Cell<usize>,
    /// Concrete Rust type name captured at allocation for testC/diagnostic
    /// telemetry. Collector behavior must not branch on this field.
    type_name: &'static str,
}

impl GcHeader {
    fn new_white(size: usize, color: Color, type_name: &'static str) -> Self {
        Self {
            color: Cell::new(color),
            age: Cell::new(GcAge::New),
            finalized: Cell::new(false),
            collected: Cell::new(false),
            next: Cell::new(None),
            gray_next: Cell::new(None),
            gray_listed: Cell::new(false),
            size: Cell::new(size),
            type_name,
        }
    }
}

/// A heap-allocated, GC-tracked value plus its header.
#[repr(C)]
pub struct GcBox<T: ?Sized> {
    header: GcHeader,
    value: T,
}

impl<T: ?Sized> GcBox<T> {
    pub fn header(&self) -> &GcHeader {
        &self.header
    }
    pub fn value(&self) -> &T {
        &self.value
    }
}

/// A GC-managed pointer. Copy + Clone (one-machine-word). Replaces `GcRef<T>`.
pub struct Gc<T: ?Sized> {
    ptr: NonNull<GcBox<T>>,
    /// Marker so `Gc<T>` is treated as if it owns `T` for variance.
    _marker: PhantomData<T>,
}

// SAFETY: Gc is just a pointer. The Cell-based interior mutability of the
// header is single-threaded (the entire Lua runtime is single-threaded), so
// no Send/Sync impls are needed and we don't provide them.
impl<T: ?Sized> Copy for Gc<T> {}
impl<T: ?Sized> Clone for Gc<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> PartialEq for Gc<T> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(self.ptr.as_ptr(), other.ptr.as_ptr())
    }
}
impl<T: ?Sized> Eq for Gc<T> {}

impl<T: ?Sized> std::hash::Hash for Gc<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ptr.as_ptr().hash(state)
    }
}

impl<T: ?Sized> std::fmt::Debug for Gc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gc({:p})", self.ptr.as_ptr())
    }
}

impl<T: Trace + 'static> Gc<T> {
    /// Allocate a `GcBox<T>` outside any heap registry. Used by legacy
    /// `GcRef::new` call sites until Phase D-1b migrates them. The returned
    /// `Gc<T>` is reachable only through the caller's own retention path;
    /// without joining a heap owner list, it will never be swept (so
    /// effectively leaks until process exit — same as Rc behavior).
    pub fn new_uncollected(value: T) -> Self {
        let size = std::mem::size_of::<T>();
        let boxed = Box::new(GcBox {
            header: GcHeader::new_white(size, Color::White0, std::any::type_name::<T>()),
            value,
        });
        Gc {
            ptr: NonNull::new(Box::into_raw(boxed)).expect("Box::into_raw is non-null"),
            _marker: PhantomData,
        }
    }

    /// Erased heap-list pointer for collector-owned intrusive bookkeeping.
    pub fn as_trace_ptr(self) -> NonNull<GcBox<dyn Trace>> {
        self.ptr
    }
}

impl<T: ?Sized> Gc<T> {
    /// Two `Gc<T>`s are identity-equal iff they point at the same box.
    pub fn ptr_eq(a: Self, b: Self) -> bool {
        std::ptr::addr_eq(a.ptr.as_ptr(), b.ptr.as_ptr())
    }

    /// Identity as a usize — usable as a hash table key for "is the *same
    /// object*" lookups.
    pub fn identity(self) -> usize {
        self.ptr.as_ptr() as *const () as usize
    }

    /// Access the underlying value. Returns `&T` so callers can read fields
    /// without taking the `Gc` apart. Interior mutability lives inside T's
    /// own fields (Cell, RefCell, etc.).
    fn as_box(&self) -> &GcBox<T> {
        // SAFETY: A Gc<T> is constructed only by allocate() or
        // new_uncollected(), both of which produce a valid GcBox. The box
        // outlives the Gc until sweep, which only frees boxes not reachable
        // from any root — so as long as this Gc is on the stack or in a
        // traced field, the box is live.
        unsafe { self.ptr.as_ref() }
    }

    fn header(&self) -> &GcHeader {
        &self.as_box().header
    }

    pub fn color(self) -> Color {
        self.header().color.get()
    }

    pub fn set_color(self, color: Color) {
        self.header().color.set(color);
    }

    pub fn age(self) -> GcAge {
        self.header().age.get()
    }

    pub fn set_age(self, age: GcAge) {
        self.header().age.set(age);
    }

    pub fn is_finalized(self) -> bool {
        self.header().finalized.get()
    }

    pub fn set_finalized(self, finalized: bool) {
        self.header().finalized.set(finalized);
    }

    /// Charge (`delta > 0`) or refund (`delta < 0`) `delta` bytes of this
    /// object's owned heap buffers against the pacer, keeping `header.size`
    /// as the single source of truth for what sweep will refund.
    ///
    /// No-op when `delta == 0` or when this box is not on a heap owner list
    /// (`collected == false`): an uncollected box is never swept, so charging
    /// it would permanently inflate the byte counter. On the collected path,
    /// `header.size` and the heap's byte counter move together, so after sweep
    /// frees this box it refunds exactly the bytes that were charged here.
    pub fn account_buffer(&self, heap: &Heap, delta: isize) {
        if delta == 0 {
            return;
        }
        let header = self.header();
        if !header.collected.get() {
            return;
        }
        if delta >= 0 {
            header.size.set(header.size.get().saturating_add(delta as usize));
        } else {
            header
                .size
                .set(header.size.get().saturating_sub((-delta) as usize));
        }
        heap.adjust_bytes(delta);
    }
}

impl<T: ?Sized> std::ops::Deref for Gc<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.as_box().value
    }
}

impl<T: ?Sized> AsRef<T> for Gc<T> {
    fn as_ref(&self) -> &T {
        &self.as_box().value
    }
}

/// Every GC-rooted type implements `Trace` to expose its `Gc<_>` fields.
///
/// The `trace` method visits every reachable `Gc<_>` and calls
/// `Marker::mark` on it. For container fields (`Vec<LuaValue>`, etc.) call
/// `field.trace(m)` to delegate.
///
/// # Mechanical pattern
///
/// ```ignore
/// impl Trace for LuaTable {
///     fn trace(&self, m: &mut Marker) {
///         for v in self.array.iter() { v.trace(m); }
///         if let Some(mt) = self.metatable { m.mark(mt); }
///     }
/// }
/// ```
pub trait Trace {
    fn trace(&self, m: &mut Marker);
}

// Common blanket impls so most container types Just Work.
impl<T: Trace> Trace for Vec<T> {
    fn trace(&self, m: &mut Marker) {
        for item in self.iter() {
            item.trace(m);
        }
    }
}

impl<T: Trace> Trace for Option<T> {
    fn trace(&self, m: &mut Marker) {
        if let Some(v) = self {
            v.trace(m);
        }
    }
}

impl<T: Trace + ?Sized> Trace for Box<T> {
    fn trace(&self, m: &mut Marker) {
        (**self).trace(m);
    }
}

impl<T: Trace + ?Sized> Trace for std::rc::Rc<T> {
    fn trace(&self, m: &mut Marker) {
        (**self).trace(m);
    }
}

impl<T: Trace> Trace for RefCell<T> {
    fn trace(&self, m: &mut Marker) {
        self.borrow().trace(m);
    }
}

/// `Gc<T>` is itself traceable: marking it forwards to the contained `T`.
impl<T: Trace + 'static> Trace for Gc<T> {
    fn trace(&self, m: &mut Marker) {
        m.mark(*self);
    }
}

// Trivially-traceable primitive types: visiting does nothing.
macro_rules! trace_noop {
    ($($t:ty),*) => {
        $(impl Trace for $t {
            fn trace(&self, _m: &mut Marker) {}
        })*
    };
}
trace_noop!(
    bool, u8, u16, u32, u64, u128, usize,
    i8, i16, i32, i64, i128, isize,
    f32, f64, char, String, str
);

impl<T> Trace for std::marker::PhantomData<T> {
    fn trace(&self, _m: &mut Marker) {}
}

/// Diagnostic counters for the latest mark phase.
///
/// These are read-only telemetry for testC/canaries and unit tests. Collector
/// decisions must continue to use object color/age metadata, not these counts.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct MarkerStats {
    pub marked: usize,
    pub marked_young: usize,
    pub marked_old: usize,
    pub traced: usize,
    pub traced_young: usize,
    pub traced_old: usize,
}

/// Diagnostic counters for the latest sweep phase.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct SweepStats {
    pub visited: usize,
    pub visited_young: usize,
    pub visited_old: usize,
    pub revisit: usize,
    pub freed: usize,
    pub freed_bytes: usize,
}

impl SweepStats {
    fn record_visit(&mut self, age: GcAge) {
        self.visited += 1;
        if age.is_old() {
            self.visited_old += 1;
        } else {
            self.visited_young += 1;
        }
    }

    fn record_free(&mut self, bytes: usize) {
        self.freed += 1;
        self.freed_bytes += bytes;
    }

    fn add(&mut self, other: Self) {
        self.visited += other.visited;
        self.visited_young += other.visited_young;
        self.visited_old += other.visited_old;
        self.revisit += other.revisit;
        self.freed += other.freed;
        self.freed_bytes += other.freed_bytes;
    }
}

/// Diagnostic counts for the allgc list split by generational cursors.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct AllGcCohortStats {
    pub new: usize,
    pub survival: usize,
    pub old1: usize,
    pub old: usize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum MarkerMode {
    Full,
    Minor,
}

/// Holds the gray queue during a mark phase. Passed to `Trace::trace`.
pub struct Marker {
    gray_queue: Vec<NonNull<GcBox<dyn Trace>>>,
    visited: std::collections::HashSet<usize>,
    stats: MarkerStats,
    mode: MarkerMode,
}

impl Marker {
    fn new() -> Self {
        Self::new_with_mode(MarkerMode::Full)
    }

    fn new_minor() -> Self {
        Self::new_with_mode(MarkerMode::Minor)
    }

    fn new_with_mode(mode: MarkerMode) -> Self {
        Self {
            gray_queue: Vec::with_capacity(256),
            visited: std::collections::HashSet::new(),
            stats: MarkerStats::default(),
            mode,
        }
    }

    fn should_trace_age(&self, age: GcAge) -> bool {
        match self.mode {
            MarkerMode::Full => true,
            MarkerMode::Minor => !matches!(age, GcAge::Old),
        }
    }

    /// Mark a `Gc<T>` as gray (reachable, but its outgoing edges not yet
    /// traced). Called by `Trace::trace` implementations.
    ///
    /// Per-cycle dedup uses `visited` (a HashSet of box identities) rather
    /// than the color flag. Color-based dedup would silently skip
    /// `new_uncollected` boxes left Black by the previous cycle — those
    /// allocations are NOT on a heap owner list, so the start-of-mark
    /// "reset heap-owned objects to White" loop does not reach them, and a Black
    /// uncollected box would be skipped without re-tracing its children
    /// (causing reachable allgc descendants to be swept). The visited set
    /// is rebuilt every `full_collect` (Marker::new), so this dedup is
    /// always per-cycle.
    pub fn mark<T: Trace + 'static>(&mut self, gc: Gc<T>) {
        let ptr: NonNull<GcBox<dyn Trace>> = gc.ptr;
        self.mark_box(ptr, gc.header(), gc.identity());
    }

    fn mark_box(&mut self, ptr: NonNull<GcBox<dyn Trace>>, header: &GcHeader, id: usize) {
        if self.visited.insert(id) {
            let age = header.age.get();
            self.stats.marked += 1;
            if age.is_old() {
                self.stats.marked_old += 1;
            } else {
                self.stats.marked_young += 1;
            }
            if self.should_trace_age(age) {
                header.color.set(Color::Gray);
                self.gray_queue.push(ptr);
            }
        }
    }

    /// Record that an Rc-backed object (`GcRef<T>` in Phase A-D-0) has been
    /// visited and return whether this is the first visit. Used by recursive
    /// `Trace` impls to break cycles while the real `Gc<T>` gray-queue path is
    /// not yet wired (e.g. `_G._G == _G` would otherwise infinitely recurse).
    pub fn try_visit(&mut self, addr: usize) -> bool {
        self.visited.insert(addr)
    }

    /// True iff `id` was reached during the mark phase. Used by the
    /// post-mark hook (`Heap::full_collect_with_post_mark`) to decide whether
    /// a weak-table entry's target is still strongly reachable. Read-only —
    /// callers cannot add entries.
    pub fn is_visited(&self, id: usize) -> bool {
        self.visited.contains(&id)
    }

    /// True when the object was marked in this cycle, or when a minor cycle
    /// deliberately skipped an old object that the young sweep will not free.
    pub fn is_marked_or_old<T: Trace + 'static>(&self, gc: Gc<T>) -> bool {
        self.is_visited(gc.identity())
            || (matches!(self.mode, MarkerMode::Minor) && gc.age().is_old())
    }

    /// Number of objects marked so far. Used by the post-mark hook's
    /// ephemeron-convergence fixed-point loop to detect when an iteration
    /// added no new reachable objects and the loop can terminate.
    pub fn visited_count(&self) -> usize {
        self.visited.len()
    }

    /// Return diagnostic counters for the current mark phase.
    pub fn stats(&self) -> MarkerStats {
        self.stats
    }

    /// Drain the gray queue, transitively marking children. Each gray box
    /// becomes black; its `Trace::trace` is called so the children it points
    /// at get pushed onto the queue. Repeats until the queue is empty.
    ///
    /// Exposed for the post-mark hook so it can run ephemeron convergence:
    /// after marking new values via [`Marker::mark`] (or `value.trace(self)`),
    /// the hook calls `drain_gray_queue` to propagate the new reachability,
    /// then re-checks for fixed-point.
    pub fn drain_gray_queue(&mut self) {
        while let Some(gray_ptr) = self.gray_queue.pop() {
            unsafe {
                let bx = gray_ptr.as_ref();
                self.stats.traced += 1;
                if bx.header.age.get().is_old() {
                    self.stats.traced_old += 1;
                } else {
                    self.stats.traced_young += 1;
                }
                bx.header.color.set(Color::Black);
                bx.value.trace(self);
            }
        }
    }
}

/// Phases of the incremental collection cycle.
///
/// The state machine matches a simplified subset of C-Lua's `lgc.c` FSM and
/// is driven by [`Heap::incremental_step_with_post_mark`].
///
/// Transitions:
/// - `Pause` → `Propagate` (on first step: reset colors, trace roots).
/// - `Propagate` → `EnterAtomic` (when the gray queue empties).
/// - `EnterAtomic` → `Atomic` (atomic phase is about to run).
/// - `Atomic` → `SweepAllGc` (post-mark hook has run; sweep cursor is initialized).
/// - `SweepAllGc` → `SweepFinObj` (allgc sweep cursor reached the end).
/// - `SweepFinObj` → `SweepToBeFnz` (finobj sweep cursor reached the end).
/// - `SweepToBeFnz` → `SweepEnd` (tobefnz sweep cursor reached the end).
/// - `SweepEnd` → `CallFin` (finish sweep bookkeeping).
/// - `CallFin` → `Pause` (cycle is complete).
///
/// `Collecting` is kept as a compatibility alias for the old API (used by
/// `barrier`) — it means "anything but Pause."
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum GcState {
    Pause,
    Propagate,
    EnterAtomic,
    Atomic,
    SweepAllGc,
    SweepFinObj,
    SweepToBeFnz,
    SweepEnd,
    CallFin,
}

impl GcState {
    pub fn is_pause(self) -> bool {
        matches!(self, GcState::Pause)
    }
    pub fn is_propagate(self) -> bool {
        matches!(self, GcState::Propagate)
    }
    pub fn is_invariant(self) -> bool {
        matches!(self, GcState::Propagate | GcState::EnterAtomic | GcState::Atomic)
    }
    pub fn is_sweep(self) -> bool {
        matches!(
            self,
            GcState::SweepAllGc
                | GcState::SweepFinObj
                | GcState::SweepToBeFnz
                | GcState::SweepEnd
        )
    }
}

/// Outcome of one call to [`Heap::incremental_step_with_post_mark`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum StepOutcome {
    /// The step finished a cycle. The heap is back at `GcState::Pause`.
    Paused,
    /// The step performed work but the cycle is not finished. Caller may
    /// step again.
    InProgress,
    /// The heap is paused (via [`Heap::pause`]) or the caller asked for zero
    /// budget while no cycle was in progress and no work was needed.
    SkippedStopped,
}

/// Work budget for one incremental step.
///
/// `remaining_work` counts down by one for each unit of work performed (one
/// gray object traced, one swept node visited, one finalizer dispatched).
/// `max_credit` caps how negative `remaining_work` may be allowed to go — a
/// step that overshoots its budget rolls the overrun into the caller's debt
/// rather than letting unbounded work happen in one call.
#[derive(Copy, Clone, Debug)]
pub struct StepBudget {
    pub remaining_work: isize,
    pub max_credit: isize,
}

impl StepBudget {
    /// Build a budget from a number of allowed work units.
    pub fn from_work(work: isize) -> Self {
        Self { remaining_work: work.max(1), max_credit: work.max(1) }
    }
}

/// The heap. One per `GlobalState`. Owns the intrusive allgc list of every
/// allocated `GcBox`, tracks total bytes, and runs collections.
/// Floor for the post-collection threshold. Without it, a tight
/// allocation loop drives the live set near zero, so `bytes * pause/100`
/// collapses toward zero and a full stop-the-world collection fires every
/// few allocations, re-tracing all roots each time (issue #38, GC path).
/// The floor bounds the wasted work: the collector waits until at least
/// this many bytes of garbage accumulate before collecting a small heap.
///
/// Raised from 256 KB to 1 MB once table array/node buffer bytes became
/// honestly accounted (see [`Gc::account_buffer`]): with real buffer bytes
/// flowing into the pacer, a 256 KB floor fires too eagerly on table-heavy
/// workloads, re-tracing all roots each time. 1 MB keeps the small-heap
/// over-collection guard while letting honest pressure drive the threshold.
const GC_MIN_THRESHOLD: usize = 1024 * 1024;

pub struct Heap {
    /// Head of the singly-linked allgc list (heap-owned objects not currently
    /// registered for finalization).
    head: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// Head of the singly-linked finobj list (objects registered for `__gc`).
    finobj: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// Head of the singly-linked tobefnz list (objects awaiting `__gc`).
    tobefnz: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// First object that survived one minor collection. Objects before this
    /// cursor are the current nursery/new generation.
    survival: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// First object that survived two minor collections. Objects from
    /// `survival` up to this cursor are the survival generation.
    old1: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// First regular old object. Objects from `old1` up to this cursor became
    /// old in the previous young collection.
    reallyold: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// First OLD1 object when one may appear before the `old1` cursor due to
    /// barriers aging objects in younger list segments.
    firstold1: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// First survival object in the finobj list.
    finobjsur: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// First old1 object in the finobj list.
    finobjold1: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// First really-old object in the finobj list.
    finobjrold: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// Total bytes allocated (sum of header sizes; rough).
    bytes: Cell<usize>,
    /// White bit used for new allocations and for survivors after a sweep.
    current_white: Cell<Color>,
    /// Heap-owned allocation tokens keyed by box address. Weak handles store
    /// these tokens so address reuse after sweep cannot resurrect a stale weak
    /// target.
    allocation_tokens: RefCell<HashMap<usize, usize>>,
    /// Next non-zero token for a collected allocation.
    next_allocation_token: Cell<usize>,
    /// Threshold above which `step` triggers a collection.
    threshold: Cell<usize>,
    /// Multiplier on bytes_used to set next threshold after collection.
    pause_multiplier: Cell<usize>,
    /// State machine for the incremental collector.
    state: Cell<GcState>,
    /// If true, `step` and `barrier` are no-ops (for bootstrap before the
    /// world is consistent).
    paused: Cell<bool>,
    /// Counter of completed collections performed (for diagnostics).
    collections: Cell<usize>,
    /// Counter of completed young-generation collections.
    minor_collections: Cell<usize>,
    /// Counter of explicit stop-the-world full-collection requests.
    full_collections: Cell<usize>,
    /// Diagnostic counters from the most recently completed mark phase.
    last_mark_stats: Cell<MarkerStats>,
    /// Diagnostic counters from the most recent sweep phase.
    last_sweep_stats: Cell<SweepStats>,
    /// Intrusive grayagain-style list of objects that young collections must
    /// revisit even if they are not reached through normal roots: OLD0/OLD1
    /// and touched old objects.
    grayagain: Cell<Option<NonNull<GcBox<dyn Trace>>>>,
    /// In-progress marker state for incremental cycles. `Some` between
    /// `Propagate` start and `Sweep` start; `None` otherwise.
    marker: RefCell<Option<Marker>>,
    /// Sweep cursor. Points at the `Cell` whose `Option<NonNull>` is the
    /// "current" link being inspected during the sweep phase. Encoded as a
    /// raw pointer because the cell lives inside a `GcHeader` (Cell, not Cell<Cell>).
    /// `None` means: sweep starts from `self.head`.
    sweep_prev_next: Cell<Option<NonNull<Cell<Option<NonNull<GcBox<dyn Trace>>>>>>>,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    pub fn new() -> Self {
        Self {
            head: Cell::new(None),
            finobj: Cell::new(None),
            tobefnz: Cell::new(None),
            survival: Cell::new(None),
            old1: Cell::new(None),
            reallyold: Cell::new(None),
            firstold1: Cell::new(None),
            finobjsur: Cell::new(None),
            finobjold1: Cell::new(None),
            finobjrold: Cell::new(None),
            bytes: Cell::new(0),
            current_white: Cell::new(Color::White0),
            allocation_tokens: RefCell::new(HashMap::new()),
            next_allocation_token: Cell::new(1),
            threshold: Cell::new(64 * 1024), // initial threshold: 64 KB
            pause_multiplier: Cell::new(200), // 200% = collect when bytes 2x threshold
            state: Cell::new(GcState::Pause),
            paused: Cell::new(true), // start paused; caller enables when world is consistent
            collections: Cell::new(0),
            minor_collections: Cell::new(0),
            full_collections: Cell::new(0),
            last_mark_stats: Cell::new(MarkerStats::default()),
            last_sweep_stats: Cell::new(SweepStats::default()),
            grayagain: Cell::new(None),
            marker: RefCell::new(None),
            sweep_prev_next: Cell::new(None),
        }
    }

    /// Enable collection. Until this is called, `step` is a no-op (so the
    /// runtime can bootstrap without prematurely freeing objects).
    pub fn unpause(&self) {
        self.paused.set(false);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.get()
    }

    /// Allocate a new `GcBox<T>` and prepend it to the allgc chain.
    pub fn allocate<T: Trace + 'static>(&self, value: T) -> Gc<T> {
        let size = std::mem::size_of::<GcBox<T>>();
        let boxed = Box::new(GcBox {
            header: GcHeader::new_white(
                size,
                self.current_white.get(),
                std::any::type_name::<T>(),
            ),
            value,
        });
        boxed.header.next.set(self.head.get());
        boxed.header.collected.set(true);
        let raw: *mut GcBox<T> = Box::into_raw(boxed);
        let ptr: NonNull<GcBox<T>> =
            NonNull::new(raw).expect("Box::into_raw is non-null");
        let dyn_ptr: NonNull<GcBox<dyn Trace>> = ptr;
        let identity = ptr.as_ptr() as *const () as usize;
        let token = self.next_token();
        self.allocation_tokens
            .borrow_mut()
            .insert(identity, token);
        self.head.set(Some(dyn_ptr));
        self.bytes.set(self.bytes.get() + size);
        Gc {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Bytes currently retained by GC-tracked objects (rough estimate).
    pub fn bytes_used(&self) -> usize {
        self.bytes.get()
    }

    /// Adjust the heap's pacer byte counter by a signed delta, saturating at
    /// zero. Used by [`Gc::account_buffer`] to charge or refund the bytes of
    /// an object's owned heap buffers (table array/node Vecs) so collections
    /// fire at honest memory pressure rather than only on header sizes.
    pub fn adjust_bytes(&self, delta: isize) {
        if delta >= 0 {
            self.bytes.set(self.bytes.get().saturating_add(delta as usize));
        } else {
            self.bytes
                .set(self.bytes.get().saturating_sub((-delta) as usize));
        }
    }

    /// Current collection threshold in bytes. When `bytes_used() >= threshold_bytes()`,
    /// the next `step()` will run a full collection (unless paused). Used by
    /// callers that want to short-circuit expensive prep work (e.g. snapshotting
    /// weak tables / pending finalizers) when no collection will actually fire.
    pub fn threshold_bytes(&self) -> usize {
        self.threshold.get()
    }

    /// Override the next automatic collection threshold.
    ///
    /// The VM uses this when Lua-level GC pacing (`GCdebt`, minor-debt, and
    /// pause-debt calculations) has already computed a byte threshold from the
    /// collector-owned live-byte counter.
    pub fn set_threshold_bytes(&self, threshold: usize) {
        self.threshold.set(threshold.max(1));
    }

    /// Cheap predicate: would a `step()` actually do work? Equivalent to
    /// `!paused && bytes_used() >= threshold_bytes()`. Callers that build
    /// snapshot state before invoking the heap should gate on this.
    pub fn would_collect(&self) -> bool {
        !self.paused.get() && self.bytes.get() >= self.threshold.get()
    }

    pub fn collections(&self) -> usize {
        self.collections.get()
    }

    pub fn minor_collections(&self) -> usize {
        self.minor_collections.get()
    }

    pub fn full_collections(&self) -> usize {
        self.full_collections.get()
    }

    pub fn last_mark_stats(&self) -> MarkerStats {
        self.last_mark_stats.get()
    }

    pub fn last_sweep_stats(&self) -> SweepStats {
        self.last_sweep_stats.get()
    }

    pub fn allgc_cohort_stats(&self) -> AllGcCohortStats {
        let survival = self.survival.get();
        let old1 = self.old1.get();
        let reallyold = self.reallyold.get();
        let mut stats = AllGcCohortStats::default();
        let mut cursor = self.head.get();
        let mut seen = HashSet::new();
        let mut cohort = 0u8;
        while let Some(ptr) = cursor {
            let id = ptr.as_ptr() as *const () as usize;
            if !seen.insert(id) {
                break;
            }
            if Some(ptr) == reallyold {
                cohort = 3;
            } else if Some(ptr) == old1 {
                cohort = 2;
            } else if Some(ptr) == survival {
                cohort = 1;
            }
            match cohort {
                0 => stats.new += 1,
                1 => stats.survival += 1,
                2 => stats.old1 += 1,
                _ => stats.old += 1,
            }
            cursor = self.header_from_ptr(ptr).next.get();
        }
        stats
    }

    fn next_token(&self) -> usize {
        let token = self.next_allocation_token.get().max(1);
        let next = token.checked_add(1).unwrap_or(1).max(1);
        self.next_allocation_token.set(next);
        token
    }

    fn current_white(&self) -> Color {
        self.current_white.get()
    }

    fn other_white(&self) -> Color {
        self.current_white.get().other_white()
    }

    fn flip_current_white(&self) {
        self.current_white.set(self.other_white());
    }

    fn for_each_list_header(
        &self,
        head: Option<NonNull<GcBox<dyn Trace>>>,
        f: &mut impl FnMut(&GcHeader),
    ) {
        let mut cursor = head;
        while let Some(ptr) = cursor {
            let header = self.header_from_ptr(ptr);
            cursor = header.next.get();
            f(header);
        }
    }

    fn for_each_header(&self, mut f: impl FnMut(&GcHeader)) {
        self.for_each_list_header(self.head.get(), &mut f);
        self.for_each_list_header(self.finobj.get(), &mut f);
        self.for_each_list_header(self.tobefnz.get(), &mut f);
    }

    fn header_from_ptr<'a>(&'a self, ptr: NonNull<GcBox<dyn Trace>>) -> &'a GcHeader {
        unsafe { &(*ptr.as_ptr()).header }
    }

    fn clear_generation_cursors(&self) {
        self.survival.set(None);
        self.old1.set(None);
        self.reallyold.set(None);
        self.firstold1.set(None);
        self.finobjsur.set(None);
        self.finobjold1.set(None);
        self.finobjrold.set(None);
        self.clear_grayagain();
    }

    fn set_all_cursors_to_head(&self) {
        let head = self.head.get();
        self.survival.set(head);
        self.old1.set(head);
        self.reallyold.set(head);
        self.firstold1.set(None);
        let finobj = self.finobj.get();
        self.finobjsur.set(finobj);
        self.finobjold1.set(finobj);
        self.finobjrold.set(finobj);
        self.clear_grayagain();
    }

    fn correct_generation_pointers(
        &self,
        removed: NonNull<GcBox<dyn Trace>>,
        next: Option<NonNull<GcBox<dyn Trace>>>,
    ) {
        if self.survival.get() == Some(removed) {
            self.survival.set(next);
        }
        if self.old1.get() == Some(removed) {
            self.old1.set(next);
        }
        if self.reallyold.get() == Some(removed) {
            self.reallyold.set(next);
        }
        if self.firstold1.get() == Some(removed) {
            self.firstold1.set(next);
        }
        if self.finobjsur.get() == Some(removed) {
            self.finobjsur.set(next);
        }
        if self.finobjold1.get() == Some(removed) {
            self.finobjold1.set(next);
        }
        if self.finobjrold.get() == Some(removed) {
            self.finobjrold.set(next);
        }
        self.unlink_grayagain(removed);
    }

    fn unlink_from_list(
        &self,
        list: &Cell<Option<NonNull<GcBox<dyn Trace>>>>,
        ptr: NonNull<GcBox<dyn Trace>>,
    ) -> bool {
        let mut prev_cell = list;
        loop {
            let Some(current) = prev_cell.get() else {
                return false;
            };
            let header = self.header_from_ptr(current);
            let next = header.next.get();
            if std::ptr::addr_eq(current.as_ptr(), ptr.as_ptr()) {
                prev_cell.set(next);
                let prev_next_ptr = NonNull::from(prev_cell);
                let removed_next_ptr = NonNull::from(&self.header_from_ptr(ptr).next);
                if self.sweep_prev_next.get() == Some(removed_next_ptr) {
                    self.sweep_prev_next.set(Some(prev_next_ptr));
                }
                self.correct_generation_pointers(ptr, next);
                header.next.set(None);
                return true;
            }
            prev_cell = &header.next;
        }
    }

    fn link_to_head(
        &self,
        list: &Cell<Option<NonNull<GcBox<dyn Trace>>>>,
        ptr: NonNull<GcBox<dyn Trace>>,
    ) {
        let header = self.header_from_ptr(ptr);
        header.next.set(list.get());
        list.set(Some(ptr));
    }

    fn link_to_tail(
        &self,
        list: &Cell<Option<NonNull<GcBox<dyn Trace>>>>,
        ptr: NonNull<GcBox<dyn Trace>>,
    ) {
        let mut last_cell = list;
        loop {
            let Some(current) = last_cell.get() else {
                let header = self.header_from_ptr(ptr);
                header.next.set(None);
                last_cell.set(Some(ptr));
                return;
            };
            last_cell = &self.header_from_ptr(current).next;
        }
    }

    pub fn move_allgc_to_finobj(&self, ptr: NonNull<GcBox<dyn Trace>>) -> bool {
        let header = self.header_from_ptr(ptr);
        if !header.collected.get() {
            return false;
        }
        if !self.unlink_from_list(&self.head, ptr) {
            return false;
        }
        if self.state.get().is_sweep() {
            header.color.set(self.current_white());
        }
        self.link_to_head(&self.finobj, ptr);
        true
    }

    pub fn move_finobj_to_tobefnz(&self, ptr: NonNull<GcBox<dyn Trace>>) -> bool {
        if !self.unlink_from_list(&self.finobj, ptr) {
            return false;
        }
        self.link_to_tail(&self.tobefnz, ptr);
        true
    }

    pub fn move_tobefnz_to_allgc(&self, ptr: NonNull<GcBox<dyn Trace>>) -> bool {
        let header = self.header_from_ptr(ptr);
        if !self.unlink_from_list(&self.tobefnz, ptr) {
            return false;
        }
        if self.state.get().is_sweep() {
            header.color.set(self.current_white());
        }
        self.link_to_head(&self.head, ptr);
        if header.age.get() == GcAge::Old1 {
            self.firstold1.set(Some(ptr));
        }
        true
    }

    fn remember_minor_revisit(&self, ptr: NonNull<GcBox<dyn Trace>>) {
        let header = self.header_from_ptr(ptr);
        if header.gray_listed.get() {
            return;
        }
        header.gray_next.set(self.grayagain.get());
        header.gray_listed.set(true);
        self.grayagain.set(Some(ptr));
    }

    fn mark_minor_revisit_objects(&self, marker: &mut Marker) {
        let mut cursor = self.grayagain.get();
        while let Some(ptr) = cursor {
            let header = self.header_from_ptr(ptr);
            cursor = header.gray_next.get();
            let id = ptr.as_ptr() as *const () as usize;
            marker.mark_box(ptr, header, id);
        }
    }

    fn clear_grayagain(&self) {
        let mut cursor = self.grayagain.get();
        self.grayagain.set(None);
        while let Some(ptr) = cursor {
            let header = self.header_from_ptr(ptr);
            cursor = header.gray_next.get();
            header.gray_next.set(None);
            header.gray_listed.set(false);
        }
    }

    fn take_grayagain(&self) -> Vec<NonNull<GcBox<dyn Trace>>> {
        let mut objects = Vec::new();
        let mut cursor = self.grayagain.get();
        self.grayagain.set(None);
        while let Some(ptr) = cursor {
            let header = self.header_from_ptr(ptr);
            cursor = header.gray_next.get();
            header.gray_next.set(None);
            header.gray_listed.set(false);
            objects.push(ptr);
        }
        objects
    }

    fn replace_grayagain(&self, objects: Vec<NonNull<GcBox<dyn Trace>>>) {
        self.clear_grayagain();
        for ptr in objects.into_iter().rev() {
            self.remember_minor_revisit(ptr);
        }
    }

    fn unlink_grayagain(&self, removed: NonNull<GcBox<dyn Trace>>) {
        let keep = self
            .take_grayagain()
            .into_iter()
            .filter(|ptr| !std::ptr::addr_eq(ptr.as_ptr(), removed.as_ptr()))
            .collect();
        self.replace_grayagain(keep);
    }

    pub fn grayagain_count(&self) -> usize {
        let mut count = 0usize;
        let mut cursor = self.grayagain.get();
        while let Some(ptr) = cursor {
            count += 1;
            cursor = self.header_from_ptr(ptr).gray_next.get();
        }
        count
    }

    /// Return the current heap token for a live allocation identity.
    ///
    /// Weak handles use this before sweep to capture their target allocation
    /// without dereferencing stale pointers later.
    pub fn allocation_token(&self, identity: usize) -> Option<usize> {
        self.allocation_tokens.borrow().get(&identity).copied()
    }

    /// Return true when `identity` still names the same heap allocation.
    ///
    /// The token check prevents allocator address reuse from making a stale
    /// weak handle look live again.
    pub fn contains_allocation(&self, identity: usize, token: usize) -> bool {
        self.allocation_token(identity) == Some(token)
    }

    /// Forward write barrier: invoked when `parent` (already-traced black
    /// object) gains a new reference to `child`. To preserve the tri-color
    /// invariant ("no black points to white"), we mark the child gray
    /// immediately. Cheap: one branch + maybe one queue push.
    ///
    /// During incremental mode this prevents the marking phase from missing
    /// the new edge. In current stop-the-world mode it's still correct (a
    /// no-op when the collection is idle), so call sites can be wired now
    /// and the incremental upgrade is mechanical later.
    pub fn barrier<P, C>(&self, parent: Gc<P>, child: Gc<C>)
    where
        P: Trace + 'static,
        C: Trace + 'static,
    {
        if self.paused.get() || self.state.get().is_pause() {
            return;
        }
        if parent.header().color.get() != Color::Black {
            return;
        }
        if !child.header().color.get().is_white() {
            return;
        }
        child.header().color.set(Color::Gray);
        if let Ok(mut m_opt) = self.marker.try_borrow_mut() {
            if let Some(m) = m_opt.as_mut() {
                let ptr: NonNull<GcBox<dyn Trace>> = child.ptr;
                m.gray_queue.push(ptr);
                m.visited.insert(child.identity());
            }
        }
    }

    /// Backward barrier: if a black object receives a reference to a white
    /// child, gray the parent so the in-progress cycle will rescan it.
    pub fn barrier_back<P, C>(&self, parent: Gc<P>, child: Gc<C>)
    where
        P: Trace + 'static,
        C: Trace + 'static,
    {
        if self.paused.get() || self.state.get().is_pause() {
            return;
        }
        if parent.header().color.get() != Color::Black {
            return;
        }
        if !child.header().color.get().is_white() {
            return;
        }
        parent.header().color.set(Color::Gray);
        if let Ok(mut m_opt) = self.marker.try_borrow_mut() {
            if let Some(m) = m_opt.as_mut() {
                let ptr: NonNull<GcBox<dyn Trace>> = parent.ptr;
                m.gray_queue.push(ptr);
                m.visited.insert(parent.identity());
            }
        }
    }

    /// Generational forward barrier: if an old object receives a reference to a
    /// young object, the child cannot jump directly to OLD because it may still
    /// point at younger objects. Lua marks it OLD0 so later young collections
    /// advance it through OLD1 to OLD.
    pub fn generational_forward_barrier<P, C>(&self, parent: Gc<P>, child: Gc<C>)
    where
        P: Trace + 'static,
        C: Trace + 'static,
    {
        if parent.age().is_old() && !child.age().is_old() {
            child.set_age(GcAge::Old0);
            let ptr: NonNull<GcBox<dyn Trace>> = child.ptr;
            self.remember_minor_revisit(ptr);
        }
        self.barrier(parent, child);
    }

    /// Generational backward barrier: an old object that now points to a young
    /// object is revisited by the next young collection. This mirrors
    /// `luaC_barrierback_`'s age transition to TOUCHED1.
    pub fn generational_backward_barrier<P>(&self, parent: Gc<P>)
    where
        P: Trace + 'static,
    {
        if parent.age().is_old() {
            parent.set_color(Color::Gray);
            parent.set_age(GcAge::Touched1);
            let ptr: NonNull<GcBox<dyn Trace>> = parent.ptr;
            self.remember_minor_revisit(ptr);
        }
    }

    /// Possibly run a collection. Trigger: bytes_used > threshold.
    /// Caller passes the root set (the runtime — typically `GlobalState`
    /// implementing `Trace`).
    pub fn step(&self, roots: &dyn Trace) {
        self.step_with_post_mark(roots, |_: &mut Marker| {});
    }

    /// Like [`step`] but invokes a `post_mark` hook when a collection
    /// actually fires (threshold reached). Hook is a no-op on the
    /// short-circuit path. The runtime uses this to bridge weak-table
    /// pruning into implicit GC steps fired from inside the VM loop.
    pub fn step_with_post_mark<F: FnMut(&mut Marker)>(
        &self,
        roots: &dyn Trace,
        post_mark: F,
    ) {
        if self.paused.get() {
            return;
        }
        if self.bytes.get() < self.threshold.get() {
            return;
        }
        self.full_collect_with_post_mark(roots, post_mark);
    }

    /// Stop-the-world full collect. Marks every reachable object from
    /// `roots`, then sweeps white (unreachable) boxes from the heap owner lists.
    pub fn full_collect(&self, roots: &dyn Trace) {
        self.full_collect_with_post_mark(roots, |_: &mut Marker| {});
    }

    /// Run only the mark/atomic hook portion of a collection, without sweeping.
    ///
    /// This is used by runtimes that need an atomic reachability snapshot for
    /// weak-table cleanup while they are deliberately avoiding object freeing.
    pub fn mark_only_with_post_mark<F: FnMut(&mut Marker)>(
        &self,
        roots: &dyn Trace,
        mut post_mark: F,
    ) {
        if self.paused.get() {
            return;
        }
        let mut marker = Marker::new();
        roots.trace(&mut marker);
        marker.drain_gray_queue();
        post_mark(&mut marker);
        marker.drain_gray_queue();
        self.last_mark_stats.set(marker.stats());
    }

    /// Metadata transition used when entering generational mode after a full
    /// mark: all currently live objects become old.
    pub fn promote_all_to_old(&self) {
        self.for_each_header(|header| {
            header.age.set(GcAge::Old);
            header.color.set(Color::Black);
        });
        self.set_all_cursors_to_head();
    }

    /// Metadata transition used when returning to incremental mode: Lua clears
    /// age information and treats all objects as new again.
    pub fn reset_all_ages(&self) {
        let current_white = self.current_white();
        self.for_each_header(|header| {
            header.age.set(GcAge::New);
            header.color.set(current_white);
        });
        self.clear_generation_cursors();
    }

    /// Run a complete young-generation collection.
    ///
    /// This is the first generational path: it uses the normal root tracer for
    /// correctness, then limits sweep/freeing to young objects. Later work can
    /// replace the full root traversal with cohort-list traversal without
    /// changing the age/sweep contract introduced here.
    pub fn minor_collect_with_post_mark<F: FnMut(&mut Marker)>(
        &self,
        roots: &dyn Trace,
        mut post_mark: F,
    ) {
        if self.paused.get() {
            return;
        }
        if !self.state.get().is_pause() {
            self.abort_cycle();
        }

        self.state.set(GcState::Propagate);
        let mut marker = Marker::new_minor();
        self.last_sweep_stats.set(SweepStats::default());
        self.mark_minor_revisit_objects(&mut marker);
        roots.trace(&mut marker);
        marker.drain_gray_queue();

        self.state.set(GcState::EnterAtomic);
        self.state.set(GcState::Atomic);
        post_mark(&mut marker);
        marker.drain_gray_queue();
        self.last_mark_stats.set(marker.stats());

        self.state.set(GcState::SweepAllGc);
        self.sweep_young();
        *self.marker.borrow_mut() = None;
        self.sweep_prev_next.set(None);
        self.state.set(GcState::Pause);
        self.collections.set(self.collections.get() + 1);
        self.minor_collections.set(self.minor_collections.get() + 1);
    }

    /// Stop-the-world full collect with a post-mark hook.
    ///
    /// Internally drives the incremental state machine to completion with
    /// an unbounded budget — equivalent to repeatedly calling
    /// [`incremental_step_with_post_mark`] until it returns `Paused`. The
    /// post-mark hook is invoked exactly once, during the atomic transition.
    pub fn full_collect_with_post_mark<F: FnMut(&mut Marker)>(
        &self,
        roots: &dyn Trace,
        mut post_mark: F,
    ) {
        if self.paused.get() {
            return;
        }
        if !self.state.get().is_pause() {
            self.abort_cycle();
        }
        self.full_collections.set(self.full_collections.get() + 1);
        let unlimited = StepBudget { remaining_work: isize::MAX, max_credit: isize::MAX };
        loop {
            let outcome = self.incremental_step_with_post_mark(roots, unlimited, &mut post_mark);
            if matches!(outcome, StepOutcome::Paused | StepOutcome::SkippedStopped) {
                break;
            }
        }
    }

    /// Run one budgeted step of the incremental collector.
    ///
    /// The state machine advances `Pause → Propagate → EnterAtomic → Atomic →
    /// SweepAllGc → SweepFinObj → SweepToBeFnz → SweepEnd → CallFin → Pause`.
    /// Each phase consumes budget; the call returns when the budget runs out
    /// or the cycle reaches `Pause`. The `post_mark`
    /// hook is invoked exactly once per cycle, during the `Atomic`
    /// transition (after the initial gray-queue drain, before sweep starts).
    ///
    /// Returns:
    /// - [`StepOutcome::Paused`] — the cycle completed.
    /// - [`StepOutcome::InProgress`] — budget exhausted before the cycle
    ///   finished; caller may step again.
    /// - [`StepOutcome::SkippedStopped`] — heap is paused; nothing happened.
    pub fn incremental_step_with_post_mark<F: FnMut(&mut Marker)>(
        &self,
        roots: &dyn Trace,
        mut budget: StepBudget,
        mut post_mark: F,
    ) -> StepOutcome {
        if self.paused.get() {
            return StepOutcome::SkippedStopped;
        }
        self.run_budgeted(roots, &mut budget, &mut post_mark);
        if self.state.get().is_pause() {
            StepOutcome::Paused
        } else {
            StepOutcome::InProgress
        }
    }

    fn run_budgeted(
        &self,
        roots: &dyn Trace,
        budget: &mut StepBudget,
        post_mark: &mut dyn FnMut(&mut Marker),
    ) -> bool {
        self.run_budgeted_until(roots, budget, post_mark, None)
    }

    fn run_budgeted_until(
        &self,
        roots: &dyn Trace,
        budget: &mut StepBudget,
        post_mark: &mut dyn FnMut(&mut Marker),
        stop_at: Option<GcState>,
    ) -> bool {
        let mut did_work = false;
        loop {
            if stop_at == Some(self.state.get()) {
                return did_work;
            }
            if budget.remaining_work <= -budget.max_credit {
                return did_work;
            }
            match self.state.get() {
                GcState::Pause => {
                    self.start_cycle(roots);
                    self.state.set(GcState::Propagate);
                    budget.remaining_work -= 1;
                    did_work = true;
                    if stop_at == Some(GcState::Propagate) {
                        return did_work;
                    }
                }
                GcState::Propagate => {
                    let work = self.drain_gray_budgeted(budget.remaining_work.max(1));
                    budget.remaining_work -= work as isize;
                    did_work = did_work || work > 0;
                    let empty = {
                        let m = self.marker.borrow();
                        m.as_ref().map(|m| m.gray_queue.is_empty()).unwrap_or(true)
                    };
                    if empty {
                        self.state.set(GcState::EnterAtomic);
                        if stop_at == Some(GcState::EnterAtomic) {
                            return did_work;
                        }
                    } else if budget.remaining_work <= 0 {
                        return did_work;
                    }
                }
                GcState::EnterAtomic => {
                    self.state.set(GcState::Atomic);
                    budget.remaining_work -= 1;
                    did_work = true;
                    if stop_at == Some(GcState::Atomic) || budget.remaining_work <= 0 {
                        return did_work;
                    }
                }
                GcState::Atomic => {
                    self.run_atomic(post_mark);
                    self.state.set(GcState::SweepAllGc);
                    budget.remaining_work -= 1;
                    did_work = true;
                    if stop_at == Some(GcState::SweepAllGc) {
                        return did_work;
                    }
                }
                GcState::SweepAllGc => {
                    let work = self.sweep_budgeted(budget.remaining_work.max(1));
                    budget.remaining_work -= work as isize;
                    did_work = did_work || work > 0;
                    if self.sweep_prev_next.get().is_none() {
                        self.state.set(GcState::SweepFinObj);
                        self.sweep_prev_next.set(Some(NonNull::from(&self.finobj)));
                        if stop_at == Some(GcState::SweepFinObj) {
                            return did_work;
                        }
                    } else if budget.remaining_work <= 0 {
                        return did_work;
                    }
                }
                GcState::SweepFinObj => {
                    let work = self.sweep_budgeted(budget.remaining_work.max(1));
                    budget.remaining_work -= work as isize;
                    did_work = did_work || work > 0;
                    if self.sweep_prev_next.get().is_none() {
                        self.state.set(GcState::SweepToBeFnz);
                        self.sweep_prev_next.set(Some(NonNull::from(&self.tobefnz)));
                        if stop_at == Some(GcState::SweepToBeFnz) {
                            return did_work;
                        }
                    } else if budget.remaining_work <= 0 {
                        return did_work;
                    }
                }
                GcState::SweepToBeFnz => {
                    let work = self.sweep_budgeted(budget.remaining_work.max(1));
                    budget.remaining_work -= work as isize;
                    did_work = did_work || work > 0;
                    if self.sweep_prev_next.get().is_none() {
                        self.state.set(GcState::SweepEnd);
                        if stop_at == Some(GcState::SweepEnd) {
                            return did_work;
                        }
                    } else if budget.remaining_work <= 0 {
                        return did_work;
                    }
                }
                GcState::SweepEnd => {
                    self.state.set(GcState::CallFin);
                    budget.remaining_work -= 1;
                    did_work = true;
                    if stop_at == Some(GcState::CallFin) || budget.remaining_work <= 0 {
                        return did_work;
                    }
                }
                GcState::CallFin => {
                    self.finish_cycle();
                    self.state.set(GcState::Pause);
                    if stop_at == Some(GcState::Pause) {
                        return did_work;
                    }
                    return did_work;
                }
            }
        }
    }

    /// Drive an incremental cycle until `target` is entered, stopping before any
    /// subsequent phase work. Intended for testC-style inspection of mid-cycle
    /// color/barrier invariants; normal collector pacing uses
    /// [`Self::incremental_step_with_post_mark`].
    pub fn incremental_run_until_state_with_post_mark<F: FnMut(&mut Marker)>(
        &self,
        roots: &dyn Trace,
        target: GcState,
        max_work: isize,
        mut post_mark: F,
    ) -> StepOutcome {
        if self.paused.get() {
            return StepOutcome::SkippedStopped;
        }
        let work = max_work.max(1);
        let mut budget = StepBudget {
            remaining_work: work,
            max_credit: work,
        };
        self.run_budgeted_until(roots, &mut budget, &mut post_mark, Some(target));
        if self.state.get().is_pause() {
            StepOutcome::Paused
        } else {
            StepOutcome::InProgress
        }
    }

    fn start_cycle(&self, roots: &dyn Trace) {
        self.flip_current_white();
        let dead_white = self.other_white();
        self.for_each_header(|header| {
            header.color.set(dead_white);
        });
        let mut marker = Marker::new();
        roots.trace(&mut marker);
        *self.marker.borrow_mut() = Some(marker);
        self.sweep_prev_next.set(None);
    }

    fn drain_gray_budgeted(&self, max_units: isize) -> usize {
        let mut m_opt = self.marker.borrow_mut();
        let marker = match m_opt.as_mut() {
            Some(m) => m,
            None => return 0,
        };
        let mut work = 0usize;
        let mut budget = max_units;
        while budget > 0 {
            let next = match marker.gray_queue.pop() {
                Some(p) => p,
                None => break,
            };
            unsafe {
                let bx = next.as_ref();
                marker.stats.traced += 1;
                if bx.header.age.get().is_old() {
                    marker.stats.traced_old += 1;
                } else {
                    marker.stats.traced_young += 1;
                }
                bx.header.color.set(Color::Black);
                bx.value.trace(marker);
            }
            work += 1;
            budget -= 1;
        }
        work
    }

    fn run_atomic(&self, post_mark: &mut dyn FnMut(&mut Marker)) {
        let mut m_opt = self.marker.borrow_mut();
        if let Some(marker) = m_opt.as_mut() {
            marker.drain_gray_queue();
            post_mark(marker);
            marker.drain_gray_queue();
        }
        self.sweep_prev_next.set(Some(NonNull::from(&self.head)));
        self.last_sweep_stats.set(SweepStats::default());
    }

    fn sweep_budgeted(&self, max_units: isize) -> usize {
        let mut work = 0usize;
        let mut budget = max_units;
        let mut freed_bytes = 0usize;
        let mut stats = SweepStats::default();
        let current_white = self.current_white();
        let dead_white = self.other_white();
        let mut prev_next_ptr = match self.sweep_prev_next.get() {
            Some(p) => p,
            None => return 0,
        };
        while budget > 0 {
            let prev_cell = unsafe { prev_next_ptr.as_ref() };
            let cursor = prev_cell.get();
            let ptr = match cursor {
                Some(p) => p,
                None => {
                    self.sweep_prev_next.set(None);
                    break;
                }
            };
            let header = self.header_from_ptr(ptr);
            let next = header.next.get();
            let age = header.age.get();
            stats.record_visit(age);
            let color = header.color.get();
            if color == dead_white {
                prev_cell.set(next);
                let size = header.size.get();
                freed_bytes += size;
                stats.record_free(size);
                self.correct_generation_pointers(ptr, next);
                self.allocation_tokens
                    .borrow_mut()
                    .remove(&(ptr.as_ptr() as *const () as usize));
                unsafe {
                    let _ = Box::from_raw(ptr.as_ptr());
                }
            } else {
                if matches!(color, Color::Black | Color::Gray) {
                    header.color.set(current_white);
                }
                prev_next_ptr = unsafe { NonNull::from(&(*ptr.as_ptr()).header.next) };
                self.sweep_prev_next.set(Some(prev_next_ptr));
            }
            work += 1;
            budget -= 1;
        }
        if freed_bytes > 0 {
            self.bytes.set(self.bytes.get().saturating_sub(freed_bytes));
        }
        if stats.visited > 0 {
            let mut total = self.last_sweep_stats.get();
            total.add(stats);
            self.last_sweep_stats.set(total);
        }
        work
    }

    fn push_next_revisit(
        next_revisit: &mut Vec<NonNull<GcBox<dyn Trace>>>,
        seen: &mut HashSet<usize>,
        ptr: NonNull<GcBox<dyn Trace>>,
        age: GcAge,
    ) {
        if matches!(
            age,
            GcAge::Old0 | GcAge::Old1 | GcAge::Touched1 | GcAge::Touched2
        ) {
            let id = ptr.as_ptr() as *const () as usize;
            if seen.insert(id) {
                next_revisit.push(ptr);
            }
        }
    }

    fn sweep_young_range(
        &self,
        mut prev_next_ptr: NonNull<Cell<Option<NonNull<GcBox<dyn Trace>>>>>,
        limit: Option<NonNull<GcBox<dyn Trace>>>,
        next_revisit: &mut Vec<NonNull<GcBox<dyn Trace>>>,
        next_revisit_seen: &mut HashSet<usize>,
        processed: &mut Option<HashSet<usize>>,
        firstold1: &mut Option<NonNull<GcBox<dyn Trace>>>,
        freed_bytes: &mut usize,
        stats: &mut SweepStats,
    ) -> (
        NonNull<Cell<Option<NonNull<GcBox<dyn Trace>>>>>,
        Option<NonNull<GcBox<dyn Trace>>>,
    ) {
        let current_white = self.current_white();
        loop {
            let prev_cell = unsafe { prev_next_ptr.as_ref() };
            let Some(ptr) = prev_cell.get() else {
                return (prev_next_ptr, None);
            };
            if Some(ptr) == limit {
                return (prev_next_ptr, Some(ptr));
            }
            let header = self.header_from_ptr(ptr);
            let next = header.next.get();
            let age = header.age.get();
            stats.record_visit(age);
            if let Some(processed) = processed.as_mut() {
                processed.insert(ptr.as_ptr() as *const () as usize);
            }
            if header.color.get().is_white() && !age.is_old() {
                prev_cell.set(next);
                let size = header.size.get();
                *freed_bytes += size;
                stats.record_free(size);
                self.correct_generation_pointers(ptr, next);
                self.allocation_tokens
                    .borrow_mut()
                    .remove(&(ptr.as_ptr() as *const () as usize));
                unsafe {
                    let _ = Box::from_raw(ptr.as_ptr());
                }
                continue;
            }

            if !header.color.get().is_white() {
                let next_age = age.next_after_minor();
                header.age.set(next_age);
                if next_age == GcAge::Old1 && firstold1.is_none() {
                    *firstold1 = Some(ptr);
                }
                match age {
                    GcAge::New => header.color.set(current_white),
                    GcAge::Touched1 | GcAge::Touched2 => header.color.set(Color::Black),
                    _ => {}
                }
                Self::push_next_revisit(next_revisit, next_revisit_seen, ptr, next_age);
            }
            prev_next_ptr = unsafe { NonNull::from(&(*ptr.as_ptr()).header.next) };
        }
    }

    fn sweep_young(&self) {
        let mut freed_bytes = 0usize;
        let mut next_revisit = Vec::new();
        let mut next_revisit_seen = HashSet::new();
        let mut firstold1 = None;
        let mut stats = SweepStats::default();
        let old_revisit = self.take_grayagain();
        let mut processed = if old_revisit.is_empty() {
            None
        } else {
            Some(HashSet::new())
        };
        let survival = self.survival.get();
        let old1 = self.old1.get();

        let (psurvival, new_old1) = self.sweep_young_range(
            NonNull::from(&self.head),
            survival,
            &mut next_revisit,
            &mut next_revisit_seen,
            &mut processed,
            &mut firstold1,
            &mut freed_bytes,
            &mut stats,
        );
        self.sweep_young_range(
            psurvival,
            old1,
            &mut next_revisit,
            &mut next_revisit_seen,
            &mut processed,
            &mut firstold1,
            &mut freed_bytes,
            &mut stats,
        );

        let finobjsur = self.finobjsur.get();
        let finobjold1 = self.finobjold1.get();
        let mut dummy_firstold1 = None;
        let (pfinobjsur, new_finobjold1) = self.sweep_young_range(
            NonNull::from(&self.finobj),
            finobjsur,
            &mut next_revisit,
            &mut next_revisit_seen,
            &mut processed,
            &mut dummy_firstold1,
            &mut freed_bytes,
            &mut stats,
        );
        self.sweep_young_range(
            pfinobjsur,
            finobjold1,
            &mut next_revisit,
            &mut next_revisit_seen,
            &mut processed,
            &mut dummy_firstold1,
            &mut freed_bytes,
            &mut stats,
        );
        self.sweep_young_range(
            NonNull::from(&self.tobefnz),
            None,
            &mut next_revisit,
            &mut next_revisit_seen,
            &mut processed,
            &mut dummy_firstold1,
            &mut freed_bytes,
            &mut stats,
        );

        for ptr in old_revisit {
            if processed
                .as_ref()
                .is_some_and(|processed| processed.contains(&(ptr.as_ptr() as *const () as usize)))
            {
                continue;
            }
            stats.revisit += 1;
            let header = self.header_from_ptr(ptr);
            if header.color.get().is_white() {
                continue;
            }
            let age = header.age.get();
            let next_age = age.next_after_minor();
            header.age.set(next_age);
            if next_age == GcAge::Old1 && firstold1.is_none() {
                firstold1 = Some(ptr);
            }
            if matches!(age, GcAge::Touched1 | GcAge::Touched2) {
                header.color.set(Color::Black);
            }
            Self::push_next_revisit(&mut next_revisit, &mut next_revisit_seen, ptr, next_age);
        }

        if freed_bytes > 0 {
            self.bytes.set(self.bytes.get().saturating_sub(freed_bytes));
        }
        self.replace_grayagain(next_revisit);
        self.reallyold.set(old1);
        self.old1.set(new_old1);
        self.survival.set(self.head.get());
        self.firstold1.set(firstold1);
        self.finobjrold.set(finobjold1);
        self.finobjold1.set(new_finobjold1);
        self.finobjsur.set(self.finobj.get());
        self.last_sweep_stats.set(stats);
    }

    fn finish_cycle(&self) {
        let stats = self
            .marker
            .borrow()
            .as_ref()
            .map(|marker| marker.stats())
            .unwrap_or_default();
        self.last_mark_stats.set(stats);
        *self.marker.borrow_mut() = None;
        self.sweep_prev_next.set(None);
        let next = self
            .bytes
            .get()
            .saturating_mul(self.pause_multiplier.get())
            / 100;
        self.threshold.set(next.max(GC_MIN_THRESHOLD));
        self.collections.set(self.collections.get() + 1);
    }

    /// Finish an idle `CallFin` phase after the runtime has drained any
    /// pending to-be-finalized objects.
    pub fn finish_callfin_phase(&self) -> bool {
        if self.state.get() != GcState::CallFin {
            return false;
        }
        self.finish_cycle();
        self.state.set(GcState::Pause);
        true
    }

    fn abort_cycle(&self) {
        if !self.state.get().is_pause() {
            *self.marker.borrow_mut() = None;
            self.sweep_prev_next.set(None);
            let current_white = self.current_white();
            self.for_each_header(|header| {
                header.color.set(current_white);
            });
            self.state.set(GcState::Pause);
        }
    }

    /// Returns the current state of the incremental collector.
    pub fn gc_state(&self) -> GcState {
        self.state.get()
    }

    /// Approximate number of live GC boxes, computed by walking all heap owner
    /// lists. Linear in heap size; used for cycle-cost estimation and tests.
    pub fn allgc_count(&self) -> usize {
        let mut count = 0usize;
        self.for_each_header(|_| {
            count += 1;
        });
        count
    }

    /// Count live allgc objects whose concrete Rust type name matches
    /// `predicate`. This is diagnostic/testC telemetry only; collector logic
    /// must not depend on Rust type names.
    pub fn type_name_count(&self, mut predicate: impl FnMut(&'static str) -> bool) -> usize {
        let mut count = 0usize;
        self.for_each_header(|header| {
            if predicate(header.type_name) {
                count += 1;
            }
        });
        count
    }

    /// Drop every allocation, ignoring reachability. Called at shutdown.
    /// After this returns, every outstanding `Gc<T>` is dangling — callers
    /// must ensure no `Gc<T>` outlives the `Heap`.
    pub fn drop_all(&self) {
        *self.marker.borrow_mut() = None;
        self.sweep_prev_next.set(None);
        self.clear_generation_cursors();
        self.state.set(GcState::Pause);
        self.allocation_tokens.borrow_mut().clear();
        self.drop_list(&self.head);
        self.drop_list(&self.finobj);
        self.drop_list(&self.tobefnz);
        self.bytes.set(0);
    }

    fn drop_list(&self, list: &Cell<Option<NonNull<GcBox<dyn Trace>>>>) {
        let mut cursor = list.get();
        list.set(None);
        while let Some(ptr) = cursor {
            // SAFETY: same chain invariant as full_collect's sweep.
            let next = unsafe {
                let next = (*ptr.as_ptr()).header.next.get();
                let _ = Box::from_raw(ptr.as_ptr());
                next
            };
            cursor = next;
        }
    }
}

impl Drop for Heap {
    fn drop(&mut self) {
        self.drop_all();
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Tests — confirm the skeleton's invariants before agents ever touch it.
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny GC-tracked type for the smoke test.
    struct Cell0 {
        next: Cell<Option<Gc<Cell0>>>,
        marker_calls: Cell<usize>,
    }

    impl Trace for Cell0 {
        fn trace(&self, m: &mut Marker) {
            self.marker_calls.set(self.marker_calls.get() + 1);
            if let Some(n) = self.next.get() {
                m.mark(n);
            }
        }
    }

    /// Roots for tests: just a single Gc<Cell0>, or none.
    struct OneRoot(Option<Gc<Cell0>>);
    impl Trace for OneRoot {
        fn trace(&self, m: &mut Marker) {
            if let Some(g) = self.0 {
                m.mark(g);
            }
        }
    }

    struct TwoRoots {
        first: Option<Gc<Cell0>>,
        second: Option<Gc<Cell0>>,
    }

    impl Trace for TwoRoots {
        fn trace(&self, m: &mut Marker) {
            if let Some(g) = self.first {
                m.mark(g);
            }
            if let Some(g) = self.second {
                m.mark(g);
            }
        }
    }

    #[derive(Clone)]
    struct FinalizerCell {
        id: usize,
        age: GcAge,
        finalized: std::rc::Rc<Cell<bool>>,
    }

    impl FinalizerCell {
        fn new(id: usize) -> Self {
            Self {
                id,
                age: GcAge::New,
                finalized: std::rc::Rc::new(Cell::new(false)),
            }
        }
    }

    impl FinalizerEntry for FinalizerCell {
        fn identity(&self) -> usize {
            self.id
        }

        fn age(&self) -> GcAge {
            self.age
        }

        fn is_finalized(&self) -> bool {
            self.finalized.get()
        }

        fn set_finalized(&self, finalized: bool) {
            self.finalized.set(finalized);
        }
    }

    fn finalizer_ids(objects: &[FinalizerCell]) -> Vec<usize> {
        objects.iter().map(|object| object.id).collect()
    }

    #[derive(Clone)]
    struct WeakCell {
        id: usize,
        live: bool,
        kind: WeakListKind,
    }

    impl WeakEntry for WeakCell {
        type Strong = usize;

        fn identity(&self) -> usize {
            self.id
        }

        fn list_kind(&self) -> WeakListKind {
            self.kind
        }

        fn upgrade(&self) -> Option<Self::Strong> {
            self.live.then_some(self.id)
        }
    }

    #[test]
    fn weak_registry_dedups_snapshots_and_retains_live_ids() {
        let mut registry = WeakRegistry::default();
        registry.push_unique(WeakCell {
            id: 1,
            live: true,
            kind: WeakListKind::WeakValues,
        });
        registry.push_unique(WeakCell {
            id: 1,
            live: true,
            kind: WeakListKind::Ephemeron,
        });
        registry.push_unique(WeakCell {
            id: 2,
            live: false,
            kind: WeakListKind::AllWeak,
        });
        registry.push_unique(WeakCell {
            id: 3,
            live: true,
            kind: WeakListKind::WeakValues,
        });

        let stats = registry.stats();
        assert_eq!(stats.weak_values, 1);
        assert_eq!(stats.ephemeron, 1);
        assert_eq!(stats.all_weak, 1);

        let snapshot = registry.live_snapshot();
        assert_eq!(snapshot, vec![3, 1]);
        assert_eq!(
            registry.stats(),
            WeakRegistryStats {
                tracked: 3,
                snapshot_live: 2,
                snapshot_dead: 1,
                retained: 2,
                weak_values: 1,
                ephemeron: 1,
                all_weak: 0,
            }
        );

        let keep: std::collections::HashSet<usize> = [3usize].into_iter().collect();
        registry.retain_identities(&keep);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.stats().retained, 1);
        assert_eq!(registry.stats().weak_values, 1);
        registry.remove_identity(3);
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn finalizer_registry_tracks_generational_cohorts() {
        let mut registry = FinalizerRegistry::default();
        registry.push_pending_unique(FinalizerCell::new(1));
        registry.push_pending_unique(FinalizerCell::new(2));

        let stats = registry.stats();
        assert_eq!(stats.finobj_new, 2);
        assert_eq!(stats.finobj_survival, 0);
        assert_eq!(stats.finobj_old1, 0);
        assert_eq!(stats.finobj_reallyold, 0);
        assert_eq!(stats.finobj_minor_scan, 2);

        registry.finish_minor_collection();
        let stats = registry.stats();
        assert_eq!(stats.finobj_new, 0);
        assert_eq!(stats.finobj_survival, 2);
        assert_eq!(stats.finobj_old1, 0);
        assert_eq!(stats.finobj_reallyold, 0);
        assert_eq!(stats.finobj_minor_scan, 2);

        registry.push_pending_unique(FinalizerCell::new(3));
        registry.finish_minor_collection();
        let stats = registry.stats();
        assert_eq!(stats.finobj_new, 0);
        assert_eq!(stats.finobj_survival, 1);
        assert_eq!(stats.finobj_old1, 2);
        assert_eq!(stats.finobj_reallyold, 0);
        assert_eq!(stats.finobj_minor_scan, 1);

        registry.finish_minor_collection();
        let stats = registry.stats();
        assert_eq!(stats.finobj_new, 0);
        assert_eq!(stats.finobj_survival, 0);
        assert_eq!(stats.finobj_old1, 1);
        assert_eq!(stats.finobj_reallyold, 2);
        assert_eq!(stats.finobj_minor_scan, 0);
    }

    #[test]
    fn finalizer_registry_minor_snapshot_uses_cohort_boundaries() {
        let mut registry = FinalizerRegistry::default();
        registry.push_pending_unique(FinalizerCell::new(1));
        registry.push_pending_unique(FinalizerCell::new(2));
        registry.push_pending_unique(FinalizerCell::new(3));
        registry.finish_minor_collection();
        registry.finish_minor_collection();
        registry.push_pending_unique(FinalizerCell::new(4));
        registry.push_pending_unique(FinalizerCell::new(5));

        assert_eq!(
            finalizer_ids(&registry.pending_minor_snapshot()),
            vec![4, 5],
            "minor finalizer scan must skip the old1/reallyold prefix"
        );

        registry.push_to_be_finalized(FinalizerCell::new(99));
        registry.promote_pending_to_finalized(vec![
            FinalizerCell::new(1),
            FinalizerCell::new(2),
            FinalizerCell::new(4),
        ]);

        let stats = registry.stats();
        assert_eq!(stats.finobj_old1, 1);
        assert_eq!(stats.finobj_new, 1);
        assert_eq!(stats.finobj_minor_scan, 1);
        assert_eq!(finalizer_ids(registry.pending()), vec![3, 5]);
        assert_eq!(
            finalizer_ids(registry.to_be_finalized()),
            vec![99, 4, 2, 1],
            "new to-be-finalized batches append behind older queued finalizers"
        );
    }

    #[test]
    fn finalizer_registry_marks_and_clears_finalized_bit() {
        let mut registry = FinalizerRegistry::default();
        let object = FinalizerCell::new(1);

        assert!(!object.is_finalized());
        registry.push_pending_unique(object.clone());
        assert!(object.is_finalized());

        registry.push_pending_unique(object.clone());
        assert_eq!(registry.pending_len(), 1);

        registry.promote_pending_to_finalized(vec![object.clone()]);
        assert!(object.is_finalized());
        assert_eq!(registry.pending_len(), 0);
        assert_eq!(registry.to_be_finalized_len(), 1);

        let popped = registry.pop_to_be_finalized().unwrap();
        assert_eq!(popped.id, 1);
        assert!(!object.is_finalized());

        registry.push_pending_unique(object.clone());
        assert!(object.is_finalized());
        assert_eq!(registry.pending_len(), 1);
    }

    #[test]
    fn alloc_and_drop_all() {
        let heap = Heap::new();
        heap.unpause();
        let _a = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let _b = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        assert!(heap.bytes_used() > 0);
        heap.drop_all();
        assert_eq!(heap.bytes_used(), 0);
    }

    fn list_len(heap: &Heap, mut cursor: Option<NonNull<GcBox<dyn Trace>>>) -> usize {
        let mut count = 0usize;
        while let Some(ptr) = cursor {
            count += 1;
            cursor = heap.header_from_ptr(ptr).next.get();
        }
        count
    }

    #[test]
    fn finalizer_intrusive_lists_sweep_and_drop() {
        let heap = Heap::new();
        heap.unpause();
        let _normal = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let finobj = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let tobefnz = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });

        assert!(heap.move_allgc_to_finobj(finobj.as_trace_ptr()));
        assert!(heap.move_allgc_to_finobj(tobefnz.as_trace_ptr()));
        assert!(heap.move_finobj_to_tobefnz(tobefnz.as_trace_ptr()));
        assert_eq!(list_len(&heap, heap.head.get()), 1);
        assert_eq!(list_len(&heap, heap.finobj.get()), 1);
        assert_eq!(list_len(&heap, heap.tobefnz.get()), 1);
        assert_eq!(heap.allgc_count(), 3);

        heap.full_collect(&TwoRoots { first: Some(finobj), second: Some(tobefnz) });
        assert_eq!(list_len(&heap, heap.head.get()), 0);
        assert_eq!(list_len(&heap, heap.finobj.get()), 1);
        assert_eq!(list_len(&heap, heap.tobefnz.get()), 1);
        assert_eq!(heap.allgc_count(), 2);

        assert!(heap.move_tobefnz_to_allgc(tobefnz.as_trace_ptr()));
        heap.full_collect(&OneRoot(Some(tobefnz)));
        assert_eq!(list_len(&heap, heap.head.get()), 1);
        assert_eq!(list_len(&heap, heap.finobj.get()), 0);
        assert_eq!(list_len(&heap, heap.tobefnz.get()), 0);
        assert_eq!(heap.allgc_count(), 1);

        heap.drop_all();
        assert_eq!(heap.bytes_used(), 0);
    }

    #[test]
    fn account_buffer_refunded_on_sweep() {
        let heap = Heap::new();
        heap.unpause();
        let baseline = heap.bytes_used();
        {
            let a = heap.allocate(Cell0 {
                next: Cell::new(None),
                marker_calls: Cell::new(0),
            });
            assert!(heap.bytes_used() > baseline);
            a.account_buffer(&heap, 4096);
            assert_eq!(a.header().size.get(), std::mem::size_of::<GcBox<Cell0>>() + 4096);
        }
        // Drop the only root path (a is no longer Trace-visible). The +4096
        // must be refunded via header.size when the box is swept.
        heap.full_collect(&OneRoot(None));
        assert_eq!(
            heap.bytes_used(),
            baseline,
            "account_buffer charge must be fully refunded by sweep"
        );
    }

    #[test]
    fn account_buffer_noop_on_uncollected() {
        let heap = Heap::new();
        heap.unpause();
        let g = Gc::new_uncollected(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let before = heap.bytes_used();
        g.account_buffer(&heap, 8192);
        assert_eq!(heap.bytes_used(), before, "uncollected box must not charge the pacer");
    }

    #[test]
    fn collect_unreachable_frees_bytes() {
        let heap = Heap::new();
        heap.unpause();
        let _a = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let bytes_before = heap.bytes_used();
        assert!(bytes_before > 0);
        // No roots — everything should sweep.
        heap.full_collect(&OneRoot(None));
        assert_eq!(heap.bytes_used(), 0);
        assert_eq!(heap.collections(), 1);
    }

    #[test]
    fn collect_keeps_reachable() {
        let heap = Heap::new();
        heap.unpause();
        let root = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let bytes_before = heap.bytes_used();
        heap.full_collect(&OneRoot(Some(root)));
        assert_eq!(heap.bytes_used(), bytes_before);
        assert_eq!(root.marker_calls.get(), 1);
    }

    #[test]
    fn allocations_start_new_and_white() {
        let heap = Heap::new();
        heap.unpause();
        let obj = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        assert_eq!(obj.age(), GcAge::New);
        assert!(obj.color().is_white());
    }

    #[test]
    fn allocation_tokens_track_exact_live_box() {
        let heap = Heap::new();
        heap.unpause();
        let obj = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let id = obj.identity();
        let token = heap
            .allocation_token(id)
            .expect("heap allocation should get a token");

        assert!(heap.contains_allocation(id, token));
        assert!(!heap.contains_allocation(id, token + 1));

        heap.full_collect(&OneRoot(None));
        assert_eq!(heap.allocation_token(id), None);
        assert!(!heap.contains_allocation(id, token));
    }

    #[test]
    fn allocation_during_incremental_sweep_survives_current_cycle() {
        let heap = Heap::new();
        heap.unpause();
        let old_dead = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let old_id = old_dead.identity();

        let outcome = heap.incremental_run_until_state_with_post_mark(
            &OneRoot(None),
            GcState::SweepAllGc,
            1024,
            |_| {},
        );
        assert_eq!(outcome, StepOutcome::InProgress);
        assert_eq!(heap.gc_state(), GcState::SweepAllGc);

        let new_during_sweep = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let new_id = new_during_sweep.identity();

        loop {
            let outcome = heap.incremental_step_with_post_mark(
                &OneRoot(None),
                StepBudget::from_work(64),
                |_| {},
            );
            if matches!(outcome, StepOutcome::Paused) {
                break;
            }
        }

        assert_eq!(heap.allocation_token(old_id), None);
        assert!(heap.allocation_token(new_id).is_some());
        assert_eq!(heap.allgc_count(), 1);

        heap.full_collect(&OneRoot(None));
        assert_eq!(heap.allocation_token(new_id), None);
        assert_eq!(heap.allgc_count(), 0);
    }

    #[test]
    fn promote_and_reset_all_ages() {
        let heap = Heap::new();
        heap.unpause();
        let a = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let b = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });

        heap.promote_all_to_old();
        assert_eq!(a.age(), GcAge::Old);
        assert_eq!(b.age(), GcAge::Old);
        assert_eq!(a.color(), Color::Black);
        assert_eq!(b.color(), Color::Black);

        heap.reset_all_ages();
        assert_eq!(a.age(), GcAge::New);
        assert_eq!(b.age(), GcAge::New);
        assert!(a.color().is_white());
        assert!(b.color().is_white());
    }

    #[test]
    fn generational_barriers_update_ages() {
        let heap = Heap::new();
        heap.unpause();
        let parent = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let child = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });

        parent.set_age(GcAge::Old);
        parent.set_color(Color::Black);
        heap.generational_backward_barrier(parent);
        assert_eq!(parent.age(), GcAge::Touched1);
        assert_eq!(parent.color(), Color::Gray);

        heap.generational_forward_barrier(parent, child);
        assert_eq!(child.age(), GcAge::Old0);
    }

    #[test]
    fn minor_collect_frees_young_and_keeps_old() {
        let heap = Heap::new();
        heap.unpause();
        let old_unreachable = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        old_unreachable.set_age(GcAge::Old);
        old_unreachable.set_color(Color::Black);
        let _young_unreachable = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let young_survivor = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });

        heap.minor_collect_with_post_mark(&OneRoot(Some(young_survivor)), |_| {});

        assert_eq!(heap.allgc_count(), 2);
        assert_eq!(old_unreachable.age(), GcAge::Old);
        assert_eq!(young_survivor.age(), GcAge::Survival);
        assert!(young_survivor.color().is_white());
    }

    #[test]
    fn minor_collect_skips_untouched_old_root_scan_work() {
        let heap = Heap::new();
        heap.unpause();
        let old_root = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        old_root.set_age(GcAge::Old);
        old_root.set_color(Color::Black);
        let young_root = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });

        heap.minor_collect_with_post_mark(
            &TwoRoots {
                first: Some(old_root),
                second: Some(young_root),
            },
            |_| {},
        );

        let stats = heap.last_mark_stats();
        assert_eq!(stats.marked, 2);
        assert_eq!(stats.marked_old, 1);
        assert_eq!(stats.marked_young, 1);
        assert_eq!(stats.traced, 1);
        assert_eq!(stats.traced_old, 0);
        assert_eq!(stats.traced_young, 1);
        assert_eq!(old_root.marker_calls.get(), 0);
        assert_eq!(young_root.marker_calls.get(), 1);
        assert_eq!(old_root.age(), GcAge::Old);
        assert_eq!(young_root.age(), GcAge::Survival);
    }

    #[test]
    fn minor_collect_traces_touched_old_parent() {
        let heap = Heap::new();
        heap.unpause();
        let old_root = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        old_root.set_age(GcAge::Old);
        old_root.set_color(Color::Black);
        let young_child = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        old_root.next.set(Some(young_child));
        heap.generational_backward_barrier(old_root);

        heap.minor_collect_with_post_mark(&OneRoot(Some(old_root)), |_| {});

        let stats = heap.last_mark_stats();
        assert_eq!(stats.marked, 2);
        assert_eq!(stats.marked_old, 1);
        assert_eq!(stats.marked_young, 1);
        assert_eq!(stats.traced, 2);
        assert_eq!(stats.traced_old, 1);
        assert_eq!(stats.traced_young, 1);
        assert_eq!(old_root.marker_calls.get(), 1);
        assert_eq!(young_child.marker_calls.get(), 1);
        assert_eq!(old_root.age(), GcAge::Touched2);
        assert_eq!(young_child.age(), GcAge::Survival);
    }

    #[test]
    fn minor_sweep_uses_generation_cursors_to_skip_old_tail() {
        let heap = Heap::new();
        heap.unpause();
        let mut old_objects = Vec::new();
        for _ in 0..64 {
            old_objects.push(heap.allocate(Cell0 {
                next: Cell::new(None),
                marker_calls: Cell::new(0),
            }));
        }
        heap.promote_all_to_old();
        let young_root = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });

        heap.minor_collect_with_post_mark(&OneRoot(Some(young_root)), |_| {});

        let stats = heap.last_sweep_stats();
        assert_eq!(stats.visited, 1);
        assert_eq!(stats.visited_young, 1);
        assert_eq!(stats.visited_old, 0);
        assert_eq!(heap.allgc_count(), old_objects.len() + 1);
        assert_eq!(young_root.age(), GcAge::Survival);
    }

    #[test]
    fn full_sweep_corrects_generation_cursors_when_cursor_object_is_freed() {
        let heap = Heap::new();
        heap.unpause();
        let _old = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        heap.promote_all_to_old();
        assert!(heap.survival.get().is_some());
        assert!(heap.old1.get().is_some());
        assert!(heap.reallyold.get().is_some());

        heap.full_collect(&OneRoot(None));

        assert_eq!(heap.allgc_count(), 0);
        assert_eq!(heap.survival.get(), None);
        assert_eq!(heap.old1.get(), None);
        assert_eq!(heap.reallyold.get(), None);
        assert_eq!(heap.firstold1.get(), None);
        assert_eq!(heap.last_sweep_stats().freed, 1);
    }

    #[test]
    fn full_sweep_unlinks_freed_grayagain_entries() {
        let heap = Heap::new();
        heap.unpause();
        let parent = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        heap.promote_all_to_old();
        heap.generational_backward_barrier(parent);
        assert_eq!(heap.grayagain_count(), 1);

        heap.full_collect(&OneRoot(None));

        assert_eq!(heap.allgc_count(), 0);
        assert_eq!(heap.grayagain_count(), 0);

        let young = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        heap.minor_collect_with_post_mark(&OneRoot(Some(young)), |_| {});
        assert_eq!(young.age(), GcAge::Survival);
    }

    #[test]
    fn grayagain_links_object_once() {
        let heap = Heap::new();
        heap.unpause();
        let parent = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        parent.set_age(GcAge::Old);
        parent.set_color(Color::Black);

        heap.generational_backward_barrier(parent);
        heap.generational_backward_barrier(parent);

        assert_eq!(heap.grayagain_count(), 1);
    }

    #[test]
    fn grayagain_list_carries_old1_until_old() {
        let heap = Heap::new();
        heap.unpause();
        let survivor = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });

        heap.minor_collect_with_post_mark(&OneRoot(Some(survivor)), |_| {});
        assert_eq!(survivor.age(), GcAge::Survival);

        heap.minor_collect_with_post_mark(&OneRoot(Some(survivor)), |_| {});
        assert_eq!(survivor.age(), GcAge::Old1);

        heap.minor_collect_with_post_mark(&OneRoot(None), |_| {});
        assert_eq!(survivor.age(), GcAge::Old);
        assert_eq!(heap.allgc_count(), 1);
    }

    #[test]
    fn grayagain_list_carries_touched2_until_old() {
        let heap = Heap::new();
        heap.unpause();
        let parent = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        parent.set_age(GcAge::Old);
        parent.set_color(Color::Black);
        heap.generational_backward_barrier(parent);

        heap.minor_collect_with_post_mark(&OneRoot(None), |_| {});
        assert_eq!(parent.age(), GcAge::Touched2);

        heap.minor_collect_with_post_mark(&OneRoot(None), |_| {});
        assert_eq!(parent.age(), GcAge::Old);
        assert_eq!(parent.marker_calls.get(), 2);
    }

    #[test]
    fn collect_traverses_cycles() {
        let heap = Heap::new();
        heap.unpause();
        let a = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let b = heap.allocate(Cell0 {
            next: Cell::new(Some(a)),
            marker_calls: Cell::new(0),
        });
        a.next.set(Some(b)); // cycle
        // With a as root, both should survive.
        heap.full_collect(&OneRoot(Some(a)));
        assert_eq!(a.marker_calls.get(), 1);
        assert_eq!(b.marker_calls.get(), 1);
        // Drop the only root path; cycle should now be collected.
        // (Note: `a` and `b` are still on the stack as Gc<Cell0> handles, but
        // they're not in a Trace-visible position.)
        heap.full_collect(&OneRoot(None));
        assert_eq!(heap.bytes_used(), 0);
    }

    #[test]
    fn heap_guard_stacks() {
        assert!(with_current_heap(|heap| heap.is_none()), "no guard initially");
        let h1 = Heap::new();
        h1.unpause();
        {
            let _g1 = HeapGuard::push(&h1);
            assert!(with_current_heap(|heap| heap.is_some()));
            let h2 = Heap::new();
            h2.unpause();
            {
                let _g2 = HeapGuard::push(&h2);
                // top of stack is h2
                with_current_heap(|heap| {
                    assert!(std::ptr::addr_eq(
                        heap.unwrap() as *const _,
                        &h2 as *const _,
                    ));
                });
            }
            // _g2 dropped — top is back to h1
            with_current_heap(|heap| {
                assert!(std::ptr::addr_eq(
                    heap.unwrap() as *const _,
                    &h1 as *const _,
                ));
            });
        }
        assert!(
            with_current_heap(|heap| heap.is_none()),
            "stack empty after all guards drop"
        );
    }

    #[test]
    fn step_threshold_triggers_collect() {
        let heap = Heap::new();
        heap.unpause();
        // Allocate enough boxes to exceed the default 64KB threshold.
        let mut keeps = Vec::new();
        for _ in 0..64 {
            // ~1KB per box (Cell0 is small, but allocating many headers
            // accumulates). For the threshold test we'd typically allocate
            // larger objects; this is a smoke test.
            keeps.push(heap.allocate(Cell0 {
                next: Cell::new(None),
                marker_calls: Cell::new(0),
            }));
        }
        // Build roots that retain all of keeps via individual marks.
        struct ManyRoots<'a>(&'a [Gc<Cell0>]);
        impl<'a> Trace for ManyRoots<'a> {
            fn trace(&self, m: &mut Marker) {
                for g in self.0.iter() {
                    m.mark(*g);
                }
            }
        }
        heap.step(&ManyRoots(&keeps));
        // step is a no-op below threshold; all roots retained.
        assert!(heap.bytes_used() > 0);
    }

    #[test]
    fn threshold_floored_after_collecting_tiny_heap() {
        let heap = Heap::new();
        heap.unpause();
        struct NoRoots;
        impl Trace for NoRoots {
            fn trace(&self, _m: &mut Marker) {}
        }
        for _ in 0..200 {
            heap.allocate(Cell0 {
                next: Cell::new(None),
                marker_calls: Cell::new(0),
            });
        }
        heap.full_collect(&NoRoots);
        assert!(
            heap.threshold_bytes() >= GC_MIN_THRESHOLD,
            "threshold {} collapsed below floor {}; a churning program would full-collect per allocation",
            heap.threshold_bytes(),
            GC_MIN_THRESHOLD
        );
    }

    fn build_chain(heap: &Heap, len: usize) -> Gc<Cell0> {
        let head = heap.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let mut cur = head;
        for _ in 1..len {
            let n = heap.allocate(Cell0 {
                next: Cell::new(None),
                marker_calls: Cell::new(0),
            });
            cur.next.set(Some(n));
            cur = n;
        }
        head
    }

    #[test]
    fn budget_zero_does_some_work() {
        let heap = Heap::new();
        heap.unpause();
        let head = build_chain(&heap, 8);
        let roots = OneRoot(Some(head));
        let budget = StepBudget::from_work(0);
        let outcome = heap.incremental_step_with_post_mark(&roots, budget, |_| {});
        assert_ne!(outcome, StepOutcome::SkippedStopped);
        assert_ne!(heap.gc_state(), GcState::Pause);
    }

    #[test]
    fn run_until_state_stops_before_next_phase_work() {
        let heap = Heap::new();
        heap.unpause();
        let head = build_chain(&heap, 8);
        let roots = OneRoot(Some(head));
        let atomic_calls = Cell::new(0);

        let outcome = heap.incremental_run_until_state_with_post_mark(
            &roots,
            GcState::Atomic,
            1024,
            |_| atomic_calls.set(atomic_calls.get() + 1),
        );
        assert_eq!(outcome, StepOutcome::InProgress);
        assert_eq!(heap.gc_state(), GcState::Atomic);
        assert_eq!(atomic_calls.get(), 0, "atomic hook must not run before inspection");

        let outcome = heap.incremental_run_until_state_with_post_mark(
            &roots,
            GcState::SweepAllGc,
            1024,
            |_| atomic_calls.set(atomic_calls.get() + 1),
        );
        assert_eq!(outcome, StepOutcome::InProgress);
        assert_eq!(heap.gc_state(), GcState::SweepAllGc);
        assert_eq!(atomic_calls.get(), 1, "entering sweep must run the atomic hook once");
    }

    #[test]
    fn larger_budget_drains_more_gray_than_smaller() {
        let small_heap = Heap::new();
        small_heap.unpause();
        let h1 = build_chain(&small_heap, 64);
        let r1 = OneRoot(Some(h1));
        let mut small_calls = 0;
        loop {
            small_calls += 1;
            let outcome = small_heap.incremental_step_with_post_mark(
                &r1,
                StepBudget::from_work(2),
                |_| {},
            );
            if outcome == StepOutcome::Paused {
                break;
            }
            assert!(small_calls < 10_000, "did not converge");
        }

        let big_heap = Heap::new();
        big_heap.unpause();
        let h2 = build_chain(&big_heap, 64);
        let r2 = OneRoot(Some(h2));
        let mut big_calls = 0;
        loop {
            big_calls += 1;
            let outcome = big_heap.incremental_step_with_post_mark(
                &r2,
                StepBudget::from_work(64),
                |_| {},
            );
            if outcome == StepOutcome::Paused {
                break;
            }
            assert!(big_calls < 10_000, "did not converge");
        }

        assert!(
            big_calls < small_calls,
            "expected big_calls={} < small_calls={}",
            big_calls,
            small_calls
        );
    }

    #[test]
    fn sweep_can_pause_and_resume() {
        let heap = Heap::new();
        heap.unpause();
        for _ in 0..16 {
            let _ = heap.allocate(Cell0 {
                next: Cell::new(None),
                marker_calls: Cell::new(0),
            });
        }
        let roots = OneRoot(None);
        let bytes_before = heap.bytes_used();
        assert!(bytes_before > 0);
        let mut step_count = 0;
        let mut saw_in_progress_during_sweep = false;
        loop {
            step_count += 1;
            let outcome = heap.incremental_step_with_post_mark(
                &roots,
                StepBudget::from_work(2),
                |_| {},
            );
            if heap.gc_state().is_sweep() && outcome == StepOutcome::InProgress {
                saw_in_progress_during_sweep = true;
            }
            if outcome == StepOutcome::Paused {
                break;
            }
            assert!(step_count < 10_000, "did not converge");
        }
        assert!(saw_in_progress_during_sweep, "sweep never paused mid-list");
        assert_eq!(heap.bytes_used(), 0);
    }

    #[test]
    fn post_mark_runs_once_per_atomic() {
        let heap = Heap::new();
        heap.unpause();
        for _ in 0..32 {
            let _ = heap.allocate(Cell0 {
                next: Cell::new(None),
                marker_calls: Cell::new(0),
            });
        }
        let roots = OneRoot(None);
        let call_count = std::cell::Cell::new(0);
        let mut step_count = 0;
        loop {
            step_count += 1;
            let outcome = heap.incremental_step_with_post_mark(
                &roots,
                StepBudget::from_work(2),
                |_| {
                    call_count.set(call_count.get() + 1);
                },
            );
            if outcome == StepOutcome::Paused {
                break;
            }
            assert!(step_count < 10_000, "did not converge");
        }
        assert_eq!(call_count.get(), 1, "post_mark must run exactly once per cycle");
    }

    #[test]
    fn full_collect_equivalent_to_incremental_to_pause() {
        let h1 = Heap::new();
        h1.unpause();
        let head1 = h1.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let _orphan1 = h1.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let _orphan2 = h1.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let roots1 = OneRoot(Some(head1));
        h1.full_collect(&roots1);
        let bytes_full = h1.bytes_used();

        let h2 = Heap::new();
        h2.unpause();
        let head2 = h2.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let _orphan3 = h2.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let _orphan4 = h2.allocate(Cell0 {
            next: Cell::new(None),
            marker_calls: Cell::new(0),
        });
        let roots2 = OneRoot(Some(head2));
        loop {
            let outcome = h2.incremental_step_with_post_mark(
                &roots2,
                StepBudget::from_work(1),
                |_| {},
            );
            if outcome == StepOutcome::Paused {
                break;
            }
        }
        assert_eq!(h2.bytes_used(), bytes_full);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        production Rust heap/collector substrate
//   target_crate:  lua-gc
//   confidence:    medium
//   todos:         0
//   port_notes:    0
//   unsafe_blocks: 13
//   notes:         Mark-and-sweep collector heap + the Gc<T> raw-pointer substrate. Uses
//                  NonNull<GcBox<T>> with linked-list allgc walking; unsafe is
//                  required for raw pointer ops and Box::into_raw/from_raw. See
//                  unsafe-budgets.toml for the per-crate ceiling.
// ──────────────────────────────────────────────────────────────────────────────
