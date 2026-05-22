//! Buffered streams — Rust port of `lzio.c` + `lzio.h`.
//!
//! Provides two public types:
//! - [`ZIO`]: a read cursor wrapping an external chunk-supplier callback.
//! - [`LexBuffer`]: a growable `Vec<u8>` byte buffer with the named interface
//!   that C code accessed through the `luaZ_*buffer*` macro family.
//!
//! The lzio header is merged here per PORTING.md §1 ("Headers merge into the
//! consuming `.rs`").  All macros defined in `lzio.h` are translated at their
//! call sites and collected as methods or constants in this module.
//!
//! # C source files
//! - `reference/lua-5.4.7/src/lzio.c`  (68 lines, 3 functions)
//! - `reference/lua-5.4.7/src/lzio.h`  (66 lines, struct + macros; merged)

// TODO(port): import path for LuaState will need adjustment once the
// crate-internal module graph is settled in Phase B.  Using a local path
// for now; may become `use lua_types::state::LuaState` or similar.
use crate::state::LuaState;
use lua_types::error::LuaError;

// ── Constants ──────────────────────────────────────────────────────────────────

// C: #define EOZ  (-1)  /* end of stream */
// macros.tsv: EOZ → const EOZ: i32 = -1
/// End-of-stream sentinel returned by [`ZIO::getc`] and [`ZIO::fill`].
pub(crate) const EOZ: i32 = -1;

// ── LexBuffer (was Mbuffer in C) ───────────────────────────────────────────────

/// Growable byte buffer used by the lexer for token text accumulation.
///
/// Corresponds to `Mbuffer` in `lzio.h`.  The C struct tracked `buffer`,
/// `n` (used length), and `buffsize` (allocated capacity) as three separate
/// fields with manual realloc.  In Rust all three are implicit in `Vec<u8>`.
///
/// # C mapping (types.tsv)
/// ```text
/// Mbuffer     → LexBuffer
/// .buffer     → Vec<u8>   (heap storage)
/// .n          → Vec::len()
/// .buffsize   → Vec::capacity()
/// ```
pub struct LexBuffer {
    // C: char *buffer;  size_t n;  size_t buffsize;
    buffer: Vec<u8>,
}

impl LexBuffer {
    // C: #define luaZ_initbuffer(L, buff) ((buff)->buffer = NULL, (buff)->buffsize = 0)
    // macros.tsv: luaZ_initbuffer → buf.init()  (most call sites just construct)
    /// Construct an empty `LexBuffer`.  Corresponds to the `luaZ_initbuffer` macro.
    pub fn new() -> Self {
        LexBuffer { buffer: Vec::new() }
    }

    // C: #define luaZ_buffer(buff)  ((buff)->buffer)
    // macros.tsv: luaZ_buffer → buf.as_mut_slice()
    /// Return the buffer contents as a mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    // C: #define luaZ_sizebuffer(buff)  ((buff)->buffsize)
    // macros.tsv: luaZ_sizebuffer → buf.capacity()
    /// Return the buffer's current allocation capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    // C: #define luaZ_bufflen(buff)  ((buff)->n)
    // macros.tsv: luaZ_bufflen → buf.len()
    /// Return the number of valid bytes currently stored in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    // C: #define luaZ_buffremove(buff, i)  ((buff)->n -= (i))
    // macros.tsv: luaZ_buffremove → buf.truncate_by(i)
    /// Shorten the live contents by `i` bytes without releasing capacity.
    pub fn truncate_by(&mut self, i: usize) {
        let new_len = self.buffer.len().saturating_sub(i);
        self.buffer.truncate(new_len);
    }

    // C: #define luaZ_resetbuffer(buff)  ((buff)->n = 0)
    // macros.tsv: luaZ_resetbuffer → buf.clear()
    /// Reset the live length to zero without releasing capacity.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    // C: #define luaZ_resizebuffer(L, buff, size) \
    //      ((buff)->buffer = luaM_reallocvchar(L, (buff)->buffer, \
    //                          (buff)->buffsize, size), \
    //       (buff)->buffsize = size)
    // macros.tsv: luaZ_resizebuffer → buf.resize(state, size)?
    /// Resize the buffer to exactly `size` bytes, filling new bytes with `0`.
    ///
    /// Returns `Err(LuaError::Memory)` on allocation failure.
    ///
    /// PORT NOTE: the C macro routes through `luaM_reallocvchar` and Lua's
    /// custom allocator.  Phase A uses `Vec::resize` with Rust's global
    /// allocator; OOM propagation via the custom allocator is a Phase D concern.
    // PERF(port): luaM_reallocvchar — Vec::resize may over-allocate relative
    // to the exact-fit C behaviour; profile in Phase B.
    pub fn resize(&mut self, _state: &mut LuaState, size: usize) -> Result<(), LuaError> {
        self.buffer.resize(size, 0u8);
        Ok(())
    }

    // C: #define luaZ_freebuffer(L, buff)  luaZ_resizebuffer(L, buff, 0)
    // macros.tsv: luaZ_freebuffer → (Rust Drop handles deallocation; drop the call)
    // PORT NOTE: `Drop for Vec` releases the heap allocation automatically.
    // Call sites that use `luaZ_freebuffer` can simply let the `LexBuffer` drop.
}

impl Default for LexBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ── ZIO (buffered input stream) ────────────────────────────────────────────────

/// Buffered input stream wrapping an external chunk-reader callback.
///
/// Corresponds to `struct Zio` / `ZIO` in `lzio.h`.  The C struct stored a
/// `lua_State *L` back-pointer and a `void *data` opaque pointer alongside a
/// raw `lua_Reader` function pointer.  In Rust:
///
/// - `lua_State *L` is removed from the struct; callers hold `&mut LuaState`
///   directly and pass it to fallible methods (per types.tsv).
/// - `void *data` is folded into the reader closure (per types.tsv).
/// - `const char *p` (raw pointer into the reader's internal buffer) becomes a
///   `usize` index into the owned `current_chunk` field.
///
/// # C mapping (types.tsv)
/// ```text
/// Zio           → ZIO
/// .n            → usize         (bytes still unread in current_chunk)
/// .p            → usize         (cursor index; was const char *)
/// .reader+.data → Box<dyn FnMut() -> Option<Vec<u8>>>  (combined)
/// .L            → removed; callers pass &mut LuaState to methods
/// ```
///
/// PORT NOTE: The types.tsv entry for `Zio.reader` lists
/// `Box<dyn FnMut() -> Option<&[u8]>>`, but `&[u8]` cannot name a lifetime
/// in a `dyn Fn` trait object without HRTB and a pinned source.  Phase A uses
/// `Option<Vec<u8>>` instead; the reader returns an owned chunk.  Phase B
/// should evaluate whether a zero-copy `&[u8]` path is achievable (e.g. by
/// making the reader hold a pinned internal buffer and returning a slice into
/// it via HRTB).
pub struct ZIO {
    // C: size_t n;  /* bytes still unread */
    n: usize,
    // C: const char *p;  /* current position in buffer */
    // PORT NOTE: raw pointer replaced by index into `current_chunk`.
    p: usize,
    // C: lua_Reader reader;  void *data;
    // PORT NOTE: C reader function pointer + void *data collapsed into one
    // closure; lua_State *L removed from the struct per types.tsv.
    // TODO(port): decide in Phase B whether concrete readers need
    // `&mut LuaState` as a parameter (e.g. for lapi's load callbacks that
    // may call into Lua).  If so, the signature becomes
    // `Box<dyn FnMut(&mut LuaState) -> Option<Vec<u8>>>` and fill/read must
    // thread state through.
    reader: Box<dyn FnMut() -> Option<Vec<u8>>>,
    // Owned current chunk returned by the reader.  Not present as a separate
    // field in C (C held a raw pointer into the reader's own internal buffer).
    current_chunk: Vec<u8>,
}

impl ZIO {
    // C: LUAI_FUNC void luaZ_init(lua_State *L, ZIO *z, lua_Reader reader, void *data)
    // macros.tsv: LUAI_FUNC → pub(crate)
    /// Initialise a `ZIO` with the given reader callback.
    ///
    /// Corresponds to `luaZ_init` in `lzio.c`.  The C parameters `reader` and
    /// `data` are combined into a single closure; `L` is no longer stored.
    ///
    /// # C source
    /// ```c
    /// // C: void luaZ_init(lua_State *L, ZIO *z, lua_Reader reader, void *data) {
    /// //   z->L = L;
    /// //   z->reader = reader;
    /// //   z->data = data;
    /// //   z->n = 0;
    /// //   z->p = NULL;
    /// // }
    /// ```
    pub(crate) fn new(reader: Box<dyn FnMut() -> Option<Vec<u8>>>) -> Self {
        ZIO {
            n: 0,
            p: 0,
            current_chunk: Vec::new(),
            reader,
        }
    }

    // C: LUAI_FUNC int luaZ_fill(ZIO *z)
    // macros.tsv: LUAI_FUNC → pub(crate)
    /// Refill the internal buffer by invoking the reader callback; return the
    /// first byte of the new chunk as an `i32`, or [`EOZ`] if no more data is
    /// available.
    ///
    /// # C source
    /// ```c
    /// // C: int luaZ_fill(ZIO *z) {
    /// //   size_t size;
    /// //   lua_State *L = z->L;
    /// //   const char *buff;
    /// //   lua_unlock(L);
    /// //   buff = z->reader(L, z->data, &size);
    /// //   lua_lock(L);
    /// //   if (buff == NULL || size == 0)
    /// //     return EOZ;
    /// //   z->n = size - 1;  /* discount char being returned */
    /// //   z->p = buff;
    /// //   return cast_uchar(*(z->p++));
    /// // }
    /// ```
    ///
    /// PORT NOTE: `lua_unlock`/`lua_lock` are no-ops in the default build and
    /// are dropped per macros.tsv.  `cast_uchar` → `as u8` per macros.tsv.
    pub(crate) fn fill(&mut self) -> i32 {
        // C: lua_unlock(L);  -- no-op per macros.tsv; dropped
        // C: buff = z->reader(L, z->data, &size);
        let chunk_opt = (self.reader)();
        // C: lua_lock(L);  -- no-op per macros.tsv; dropped

        match chunk_opt {
            // C: if (buff == NULL || size == 0) return EOZ;
            None => EOZ,
            Some(chunk) if chunk.is_empty() => EOZ,
            Some(chunk) => {
                // C: z->n = size - 1;  /* discount char being returned */
                self.n = chunk.len() - 1;
                // C: z->p = buff;   (reset cursor to start of new chunk)
                self.current_chunk = chunk;
                self.p = 0;
                // C: return cast_uchar(*(z->p++));
                // cast_uchar → as u8  per macros.tsv
                let byte = self.current_chunk[self.p] as u8;
                self.p += 1;
                byte as i32
            }
        }
    }

    // C: #define zgetc(z)  (((z)->n--)>0 ?  cast_uchar(*(z)->p++) : luaZ_fill(z))
    // macros.tsv: zgetc → z.getc()  returning i32 (next byte or EOZ)
    /// Return the next byte from the stream as an `i32`, or [`EOZ`] at
    /// end-of-stream.
    ///
    /// This is the hot-path inline method corresponding to the `zgetc` macro.
    /// When bytes remain in the current chunk no allocation occurs.
    ///
    /// # C source (macro)
    /// ```c
    /// // C: #define zgetc(z)  (((z)->n--)>0 ?  cast_uchar(*(z)->p++) : luaZ_fill(z))
    /// ```
    ///
    /// PORT NOTE: The C macro uses `(z)->n-- > 0` which reads n, tests it, then
    /// decrements.  When n == 0 the test is false (0 > 0) so fill is called
    /// without decrementing.  The Rust translation preserves this: `if self.n > 0`
    /// followed by an explicit `self.n -= 1`.
    #[inline]
    pub(crate) fn getc(&mut self) -> i32 {
        // C: ((z)->n--)>0  — if n is non-zero consume one byte from the chunk
        if self.n > 0 {
            self.n -= 1;
            // C: cast_uchar(*(z)->p++)  — read the byte then advance the cursor
            let byte = self.current_chunk[self.p] as u8;
            self.p += 1;
            byte as i32
        } else {
            // C: luaZ_fill(z)  — buffer exhausted; fetch the next chunk
            self.fill()
        }
    }

    // C: LUAI_FUNC size_t luaZ_read(ZIO *z, void *b, size_t n)
    // macros.tsv: LUAI_FUNC → pub(crate)
    /// Read exactly `buf.len()` bytes into `buf`.
    ///
    /// Returns the number of bytes that could **not** be read: `0` means
    /// complete success; a non-zero value means end-of-stream was reached with
    /// that many bytes still outstanding.
    ///
    /// # C source
    /// ```c
    /// // C: size_t luaZ_read(ZIO *z, void *b, size_t n) {
    /// //   while (n) {
    /// //     size_t m;
    /// //     if (z->n == 0) {  /* no bytes in buffer? */
    /// //       if (luaZ_fill(z) == EOZ)  /* try to read more */
    /// //         return n;  /* no more input; return number of missing bytes */
    /// //       else {
    /// //         z->n++;  /* luaZ_fill consumed first byte; put it back */
    /// //         z->p--;
    /// //       }
    /// //     }
    /// //     m = (n <= z->n) ? n : z->n;  /* min. between n and z->n */
    /// //     memcpy(b, z->p, m);
    /// //     z->n -= m;
    /// //     z->p += m;
    /// //     b = (char *)b + m;
    /// //     n -= m;
    /// //   }
    /// //   return 0;
    /// // }
    /// ```
    ///
    /// PORT NOTE: C's `void *b` + explicit `n` become Rust's `&mut [u8]`, whose
    /// length encodes the requested byte count.  `memcpy` becomes
    /// `copy_from_slice`.  The advancing pointer `b = (char *)b + m` is
    /// replaced by a `dst` index into `buf`.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> usize {
        // C: n (number of bytes still needed)
        let mut remaining = buf.len();
        // C: b (advancing output pointer) — represented as an index
        let mut dst: usize = 0;

        while remaining > 0 {
            // C: if (z->n == 0) {  /* no bytes in buffer? */
            if self.n == 0 {
                // C: if (luaZ_fill(z) == EOZ)
                if self.fill() == EOZ {
                    // C: return n;  /* no more input; return number of missing bytes */
                    return remaining;
                } else {
                    // C: z->n++;  /* luaZ_fill consumed first byte; put it back */
                    // C: z->p--;
                    // fill() advanced p by 1 and set n = chunk.len() - 1.
                    // Undoing that makes the whole chunk available to the
                    // copy loop below.
                    self.n += 1;
                    self.p -= 1;
                }
            }

            // C: m = (n <= z->n) ? n : z->n;  /* min. between n and z->n */
            let m = remaining.min(self.n);

            // C: memcpy(b, z->p, m);
            buf[dst..dst + m]
                .copy_from_slice(&self.current_chunk[self.p..self.p + m]);

            // C: z->n -= m;  z->p += m;
            self.n -= m;
            self.p += m;

            // C: b = (char *)b + m;  n -= m;
            dst += m;
            remaining -= m;
        }

        // C: return 0;
        0
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lzio.c  (68 lines, 3 functions)
//                  src/lzio.h  (66 lines, merged)
//   target_crate:  lua-vm
//   confidence:    medium
//   todos:         1
//   port_notes:    4
//   unsafe_blocks: 0   (must be 0 outside lua-gc/lua-coro)
//   notes:         Logic is faithful.  The one open question (TODO) is whether
//                  concrete reader callbacks will need `&mut LuaState` as a
//                  parameter when load/dofile lands in Phase B.  If so,
//                  `ZIO::reader`, `fill`, `getc`, and `read` all need a
//                  threading change.  `LexBuffer::resize` stubs OOM handling
//                  (real allocator wiring is Phase D).  Import paths for
//                  `LuaState` and `LuaError` will require crate-graph fixes
//                  in Phase B.
// ──────────────────────────────────────────────────────────────────────────────
