# VM-layer cross-version findings (surfaced by the stdlib net-strengthening)

Date opened: 2026-06-14. Source: Stage-2 idiomatization Sprint 2 (the "crush the
stdlib" pass). Each stdlib module's net-strengthening pinned its behavior to the
five reference binaries; **31 cross-version bugs were caught and fixed in the
stdlib layer.** The findings below are the ones that could NOT be fixed in
`lua-stdlib` because their single source of truth lives in `lua-vm` — they were
deliberately **documented, not force-fixed across the crate boundary** (per the
sprint's STOP-and-report rule). They are real, reference-confirmed correctness
divergences; this is the prioritized backlog for a future `lua-vm` pass.

Each finding lists: the divergence, the reference truth, the owning location, and
the modules whose tests/behavior it touches.

---

## F1 — Argument-error function-name resolution (SYSTEMIC, highest impact)

**Divergence.** For a `bad argument #N to 'NAME' (...)` error, the resolved
`NAME` is wrong in two version-specific ways:
- **5.1**: when the offending call can't be name-resolved (e.g. a function passed
  where a level was expected), the reference says `'?'`; we say the full name.
- **5.1/5.2**: the reference resolves a global function as `'_G.fn'` / a bare
  short name in cases where we emit a different qualification.
- A **related** always-present gap: some errors omit the function name entirely
  where the reference includes it (the long-known `math.fmod(x,0)` →
  `bad argument #2 (zero)` vs reference `... to 'fmod' (zero)`).

**Reference truth.** Per-version name resolution in `luaL_argerror` /
`getfuncname` / the `funcnamefromcode` path.

**Owner.** `lua-vm` — `arg_error_impl` / `find_func_name_in_loaded` (the
function-name resolver), version-gated.

**Touched by (where stdlib tests pin only the stable substring to stay robust):**
`debug.getlocal` (5.1 `'?'`), `os.date` (invalid-specifier funcname), `io.read`
(5.1 `'?'`), `base` (`type`/`tonumber`/`select`/`rawlen`), `math.fmod`.

**Why deferred.** A single systemic resolver bug; fixing it per-callsite in
stdlib would be N partial fixes of one root cause. Fix once in the VM resolver.

---

## F2 — `__name` metamethod honored before 5.3

**Divergence.** The `__name` metafield (custom type name in `tostring` and in
type-mismatch error messages) is honored on all versions; it should be **5.3+
only** (5.1/5.2 ignore it).

**Reference truth.** `__name` was added in 5.3 (`luaL_tolstring` / `luaL_typeerror`).

**Owner.** `lua-vm` — `tagmethods.rs::obj_type_name_cow` (currently
version-less + allocation-free, used by **23 VM error/display sites** spanning
both `tostring` and error type-naming). Gating only `base.tostring`'s
`to_display_string` would fix display but leave error messages wrong — a partial
fix. Thread the version into `obj_type_name_cow`.

**Touched by.** `base.tostring`, and every VM type-mismatch error message.

---

## F3 — 5.1 yield-from-outside-a-coroutine wording

**Divergence.** Yielding from the main coroutine / across a C-call boundary: 5.1
reference says `attempt to yield across metamethod/C-call boundary`; we say
`attempt to yield from outside a coroutine` on all versions.

**Reference truth.** 5.1 `lua_yield`'s guard wording differs from 5.2+.

**Owner.** `lua-vm` — `do_.rs::lua_yieldk` (a shared yield guard used by
`coroutine.yield`, hooks, and the gsub/pcall callback paths — so a stdlib-side
rewrite would only fix the direct-call path, not be single-source). Add a
version gate in `lua_yieldk`.

**Touched by.** `coroutine.yield` (and any yield across a C boundary).

---

## F4 — resume of a normal/parent coroutine: registry resolution

**Divergence.** `coroutine.resume(parent_or_main)`:
- 5.1: reference `bad argument #1 ... coroutine expected`; we say
  `bad argument #1 (thread expected, got nil)`.
- 5.2+: reference `cannot resume non-suspended coroutine`; we say
  `cannot resume dead coroutine`.
Root cause: the parent/main thread isn't resolving in the registry, so the guard
sees `nil`/`dead` instead of a live non-suspended thread.

**Owner.** `lua-vm` — the thread/registry resolution feeding `coro_lib`'s resume
guard (beyond the cold message layer that `coro_lib` owns).

**Touched by.** `coroutine.resume` of a non-child coroutine.

---

## F5 — io default/standard output wording (5.1 vs 5.4)

**Divergence.** Closed default output: 5.1 reference says
`standard output file is closed`; 5.4 says `default output file is closed`. We
emit one form on all versions.

**Owner.** Likely stdlib-side (`io_lib::get_io_file_rc`) — a candidate for a
*future io packet* rather than a VM change, but it crosses into the default-file
registry handling; recorded here with the family. Lower priority (cosmetic).

**Touched by.** `io.write`/`io.read` on a closed default file.

---

## Priority

1. **F1** (systemic arg-name resolver) — touches the most surfaces; one fix.
2. **F2** (`__name` pre-5.3) — touches 23 sites incl. all type errors; one fix.
3. **F3 / F4** (coroutine wording + registry) — coroutine-scoped.
4. **F5** — cosmetic, stdlib-side, low priority.

None of these block correctness on the dominant versions (5.4 default); they are
pre-5.4 fidelity gaps that the stdlib's per-version net made visible. Fixing F1
and F2 would likely flip several currently-stable-substring stdlib tests to
exact-string pins.
