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

## F2 — `__name` metamethod honored before 5.3 — ✅ FIXED 2026-06-14

Fixed via a single-source `LuaVersion::honors_name_metafield()` (false for
5.1/5.2) gating every `__name` read site: `obj_type_name_cow` (+ the free
`obj_type_name` that supplies the version), `state.obj_type_name`, and the two
`auxlib` type-error / `tolstring` sites. Pinned by
`crates/lua-stdlib/tests/name_metafield_version.rs` (tostring + type-error, per
version), verified live against `lua5.{1.5,2.4,3.6,4.7,5.0}`. Original finding
below.

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

## F3 — 5.1 yield-from-outside-a-coroutine wording — ✅ FIXED 2026-06-14

Fixed by gating the `!is_yieldable()` block in `do_::lua_yieldk` so 5.1 returns
its single message (`attempt to yield across metamethod/C-call boundary`); 5.2+
unchanged. Reference-pinned by `v_yield_outside_coroutine_wording_crossversion`
in `multiversion_oracle` (PR #211). Original finding below.

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

## F6 — `load()` with a function reader is not lazy (eager drain) — CROSS-VERSION

Date opened: 2026-06-22. Source: arch7-reader wave.

**Divergence.** `load(reader_fn)` where the chunk has an early syntax error: the
reference stops pulling reader chunks the moment the lexer/parser detects the
error; we drain the reader to EOF first, then parse. Reproduces on **every**
version (5.1–5.5):

```
local i=0
local function r(x) return function() i=i+1; return string.sub(x,i,i) end end
local a,b = load(r("*a = 123")); print(not a, i)
-- reference: true  2     (reads '*' as firstchar, then 'a', errors)
-- ours:      true  9     (drains all 8 chars + EOF, then parses)
```

Valid chunks are unaffected — they read to EOF on both sides, so the count
matches (`diff_one 5.4 'load(r("return 1+2"))'` → MATCH). The bug is isolated to
the *early-error* reader-call count.

**Reference truth.** C's `lua_load` installs `lua_Reader` on the `ZIO`; the lexer
pulls chunks **on demand** via `zgetc`/`luaZ_fill`, which call back into the
reader (with `L`) only when a byte is needed. The first syntax error long-jumps
out of the parser before the rest of the stream is ever requested.

**Owner.** `lua-vm` — the reader plumbing, specifically the *type of the reader
threaded through the ZIO*. It must become reentrant: `FnMut(&mut LuaState) ->
Result<Option<Vec<u8>>, LuaError>` so a chunk can be fetched mid-parse (the
reader calls a Lua function, which needs `&mut LuaState`).

**Touched by.** `calls.lua@5.1` line 250 (`assert(not a and ... and i == 2)`),
and the reader-based `load` cases in `files.lua@5.1`/`files.lua@5.3`. The error
case is the only one that pins the *count*, so it is the gating assertion.

**Root cause — the exact chain (file:line).**
1. `crates/lua-stdlib/src/state_stub.rs` `load_with_reader` (~line 1787) drains
   the `F: FnMut(&mut LuaState) -> Result<Option<Vec<u8>>, …>` reader into a
   `Vec<u8>` **before** parsing — this is where all N reader calls happen (the 9
   in the repro), because it loops calling `generic_reader` (→ the Lua reader fn)
   until `None`. It then wraps the buffer in a once-`Box<dyn FnMut() ->
   Option<Vec<u8>>>` and hands it to `api::load`.
2. `crates/lua-vm/src/api.rs` `load` (~line 1982) takes
   `reader: Box<dyn FnMut() -> Option<Vec<u8>>>` (NON-reentrant — no `&mut
   LuaState`) and builds the vm `ZIO` via `crate::zio::ZIO::new(reader)`.
3. `crates/lua-vm/src/do_.rs` `parse_stub` (~line 37) then drains the vm `ZIO`
   (`z.getc()` loop) into another `Vec<u8>` `source` and calls the hook with
   `&[u8]`. (This second drain copies the already-materialized buffer; it does
   NOT call the Lua reader, so it does not add to the count — but it does make
   the chain eager-by-construction.)
4. `crates/lua-vm/src/state.rs` `ParserHook` (~line 1227) is typed
   `fn(&mut LuaState, source: &[u8], name, firstchar)` — a fully-materialized
   buffer, so the parser can be no lazier than its input.
5. `crates/lua-parse/src/lib.rs` `parse` (~line 6238) builds
   `lua_lex::ZIO::from_bytes(rest_bytes)` from the full buffer. The lua-lex
   `ZIO` (`crates/lua-lex/src/lib.rs` ~line 139) *already* pulls lazily from a
   `Box<dyn FnMut() -> Option<Vec<u8>>>` reader and stops at the first error —
   it is the only correct link; it is simply fed a once-buffer.

**Needed change (the real fix, end-to-end lazy).**
- Make the reader type **reentrant** through the whole chain:
  `FnMut(&mut LuaState) -> Result<Option<Vec<u8>>, LuaError>`, with the vm `ZIO`
  (`crates/lua-vm/src/zio.rs`) `getc`/`fill` taking `&mut LuaState`, and
  `api::load` (`crates/lua-vm/src/api.rs`) accepting that reader instead of the
  state-less `Box<dyn FnMut() -> Option<Vec<u8>>>`.
- Change `ParserHook` (`state.rs`) to carry the streaming source (the vm `ZIO`
  /reader) rather than `&[u8]`, and update `lua_parse::parse` to build its
  `lua_lex::ZIO` from a reader that pulls from the vm side on demand (the
  lua-lex `ZIO` already supports this).
- Delete the eager drains in `load_with_reader` (state_stub.rs) and `parse_stub`
  (do_.rs).

**Why deferred / not landed in this wave.** The fix is genuinely cross-crate but
its required edits fall **outside the arch7-reader edit boundary** (which was
`state_stub.rs`, `do_.rs`, `state.rs::ParserHook`, `lua-parse/lib.rs`,
`lua-lex/lib.rs`):
- The reentrant reader type must change `crates/lua-vm/src/api.rs` (`load`
  signature + `ZIO::new` call) and `crates/lua-vm/src/zio.rs` (the `ZIO` reader
  field type and `getc`/`fill` taking `&mut LuaState`). Both are **not** in the
  boundary, and `api::load` is the only `pub` parse entry reachable from
  lua-stdlib (`protected_parser` is `pub(crate)`), so there is no in-boundary
  path to a reentrant reader.
- Changing `ParserHook`'s fn-pointer signature forces edits to its **three
  installers** — `crates/lua-cli/src/main.rs` (~866), `crates/lua-rs-runtime/
  src/lib.rs` (~3498), `crates/lua-hlua-shim/src/lib.rs` (~163) — all outside
  the boundary.
- The "smaller correct step" (make the lexer stop pulling early) does **not**
  help here, because the offending reader calls happen in `load_with_reader`'s
  drain *before the lexer ever runs*; removing that drain is exactly what
  requires the reentrant-reader threading above.

The honest in-boundary outcome was therefore a precise documented diagnosis (this
finding) with no half-applied edits. A future wave should be scoped to include
`api.rs`, `zio.rs`, and the three ParserHook installers as one coordinated
change.

**Touched files for the future fix:** `state_stub.rs`, `api.rs`, `zio.rs`,
`do_.rs`, `state.rs` (ParserHook), `lua-parse/lib.rs`, `lua-lex/lib.rs`,
`lua-cli/main.rs`, `lua-rs-runtime/lib.rs`, `lua-hlua-shim/lib.rs`.

---

## Priority

1. **F1** (systemic arg-name resolver) — touches the most surfaces; one fix.
2. **F2** (`__name` pre-5.3) — touches 23 sites incl. all type errors; one fix.
3. **F3 / F4** (coroutine wording + registry) — coroutine-scoped.
4. **F6** (lazy `load` reader) — cross-version; gates `calls.lua@5.1`,
   `files.lua@5.1`, `files.lua@5.3`; needs a 10-file coordinated reentrant-reader
   change (api.rs + zio.rs + 3 ParserHook installers beyond the usual 5).
5. **F5** — cosmetic, stdlib-side, low priority.

None of these block correctness on the dominant versions (5.4 default); they are
pre-5.4 fidelity gaps that the stdlib's per-version net made visible. Fixing F1
and F2 would likely flip several currently-stable-substring stdlib tests to
exact-string pins.
