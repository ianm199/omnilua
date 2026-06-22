//! Lexical analyzer for omniLua.
//!
//! Scans a [`ZIO`] byte stream into [`Token`] values one character at a time,
//! with one-token lookahead. One core lexes Lua 5.1 through 5.5; the lexical
//! differences between versions (which operators exist, escape handling,
//! reserved words, error wording) are gated on the active [`LuaVersion`] and are
//! the behavioural invariant this crate guarantees.
//!
//! This file was originally a line-by-line port of `llex.c` but has been
//! idiomatized and the C correspondence removed (see `GRADUATED.md`); behaviour
//! is held by bytecode parity plus the lexical-error / line-number oracle, not
//! by matching the C source.
//!
//! [`LuaVersion`]: lua_types::LuaVersion

use std::io::Write as IoWrite;

use lua_types::gc::GcRef;

pub use lua_types::LuaError;
pub use lua_types::LuaString;
pub use lua_vm::state::LuaState;
pub use lua_vm::table::LuaTable;

/// Growable token-text buffer for the lexer.
///
/// TODO: ZIO/LexBuffer will move to lua_vm::zio (separate refactor).
pub struct LexBuffer {
    buffer: Vec<u8>,
}

impl LexBuffer {
    pub fn new() -> Self {
        LexBuffer { buffer: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn truncate_by(&mut self, i: usize) {
        let new_len = self.buffer.len().saturating_sub(i);
        self.buffer.truncate(new_len);
    }

    /// Ensure the buffer can hold `size` bytes by adjusting allocated capacity,
    /// never the live byte count (otherwise `push_byte` would write past the live
    /// content and leave embedded zero padding inside the token text).
    pub fn resize(&mut self, _state: &mut LuaState, size: usize) -> Result<(), LuaError> {
        if size < self.buffer.len() {
            self.buffer.truncate(size);
        }
        if size > self.buffer.capacity() {
            let extra = size - self.buffer.capacity();
            self.buffer.reserve_exact(extra);
        }
        Ok(())
    }

    /// Append one byte to the live contents.  Panics if capacity exceeded
    /// (callers must pre-check via `save`).
    fn push_byte(&mut self, c: u8) {
        self.buffer.push(c);
    }

    /// The token text with `n` bytes removed from each end, copied to an owned
    /// `Vec`.
    ///
    /// This is the delimiter-stripping extraction used after scanning a quoted
    /// or long-bracket string: the buffer holds `<open-delim><content><close-delim>`
    /// where each delimiter is `n` bytes (`n = 1` for `"`/`'`, `n = sep` for
    /// `[=*[`). An empty `Vec` is returned when the buffer is shorter than the
    /// two delimiters (an empty string literal).
    ///
    /// The copy is an ownership requirement, not a leftover C idiom: the caller
    /// next passes these bytes to `new_string`, which needs `&mut LexState`, so
    /// the borrow of the buffer must end first.
    fn trim_ends(&self, n: usize) -> Vec<u8> {
        match self.buffer.len().checked_sub(2 * n) {
            Some(_) => self.buffer[n..self.buffer.len() - n].to_vec(),
            None => Vec::new(),
        }
    }

    /// The whole token text, copied to an owned `Vec`.
    ///
    /// Used for identifiers/keywords, where the entire buffer is the token. As
    /// with [`trim_ends`](Self::trim_ends) the copy releases the buffer borrow
    /// before `new_string` takes `&mut LexState`.
    fn to_owned_text(&self) -> Vec<u8> {
        self.buffer.clone()
    }

    /// The live contents with a single trailing NUL removed, if present.
    ///
    /// Numeral scanning and error-message formatting append a `\0` to mirror
    /// C's NUL-terminated buffer convention before reading the text back; this
    /// drops that one sentinel byte without copying. A buffer that does not end
    /// in NUL is returned whole.
    fn without_trailing_nul(&self) -> &[u8] {
        match self.buffer.last() {
            Some(&0) => &self.buffer[..self.buffer.len() - 1],
            _ => &self.buffer,
        }
    }
}

impl Default for LexBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Chunked byte source for the lexer.
///
/// Pulls successive byte chunks from a reader callback and hands them out one
/// byte at a time as `i32`, with [`EOZ`] (`-1`) signalling end of stream. A
/// reader is permitted to be re-polled after yielding an empty chunk: an empty
/// chunk reports `EOZ` without committing it, so a later [`getc`](ZIO::getc)
/// asks the reader again — that is how an interactive source distinguishes a
/// momentary stall from a true end.
///
/// The byte position is a single cursor into the current chunk; "remaining" is
/// derived as `chunk.len() - cursor`, not tracked as a separate counter.
///
/// TODO: ZIO/LexBuffer will move to lua_vm::zio (separate refactor).
pub struct ZIO {
    reader: Box<dyn FnMut() -> Option<Vec<u8>>>,
    chunk: Vec<u8>,
    cursor: usize,
}

impl ZIO {
    /// Construct a ZIO from a reader callback that yields successive chunks.
    pub fn new(reader: Box<dyn FnMut() -> Option<Vec<u8>>>) -> Self {
        ZIO {
            reader,
            chunk: Vec::new(),
            cursor: 0,
        }
    }

    /// Construct a ZIO that yields the supplied bytes once and then EOZ.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut once = Some(bytes);
        ZIO::new(Box::new(move || once.take()))
    }

    /// Return the next byte as `i32`, or [`EOZ`] at end of stream.
    pub fn getc(&mut self) -> i32 {
        match self.chunk.get(self.cursor) {
            Some(&b) => {
                self.cursor += 1;
                b as i32
            }
            None => self.fill(),
        }
    }

    /// Pull the next non-exhausted chunk and yield its first byte. An exhausted
    /// reader (`None`) or an empty chunk both report `EOZ` without advancing the
    /// cursor, so the reader may be asked again on the next [`getc`](ZIO::getc).
    fn fill(&mut self) -> i32 {
        match (self.reader)() {
            Some(chunk) if !chunk.is_empty() => {
                self.chunk = chunk;
                self.cursor = 1;
                self.chunk[0] as i32
            }
            _ => EOZ,
        }
    }
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// First token kind value that is not a single-byte character.
/// Single-byte tokens are represented by their ASCII value (0-255).
pub const FIRST_RESERVED: i32 = 257;

/// Name of the global environment upvalue.
pub const LUA_ENV: &[u8] = b"_ENV";

/// Number of reserved words (keywords).
pub const NUM_RESERVED: usize = (TK_WHILE - FIRST_RESERVED + 1) as usize;

/// End-of-stream sentinel returned by ZIO::getc.
pub const EOZ: i32 = -1;

const MAX_SIZE: usize = if std::mem::size_of::<usize>() < std::mem::size_of::<i64>() {
    usize::MAX
} else {
    i64::MAX as usize
};

const LUA_MIN_BUFFER: usize = 32;

// ── Token kind constants (ORDER RESERVED) ─────────────────────────────────────

/// `and`
pub const TK_AND: i32 = 257;
/// `break`
pub const TK_BREAK: i32 = 258;
/// `do`
pub const TK_DO: i32 = 259;
/// `else`
pub const TK_ELSE: i32 = 260;
/// `elseif`
pub const TK_ELSEIF: i32 = 261;
/// `end`
pub const TK_END: i32 = 262;
/// `false`
pub const TK_FALSE: i32 = 263;
/// `for`
pub const TK_FOR: i32 = 264;
/// `function`
pub const TK_FUNCTION: i32 = 265;
/// `goto`
pub const TK_GOTO: i32 = 266;
/// `if`
pub const TK_IF: i32 = 267;
/// `in`
pub const TK_IN: i32 = 268;
/// `local`
pub const TK_LOCAL: i32 = 269;
/// `nil`
pub const TK_NIL: i32 = 270;
/// `not`
pub const TK_NOT: i32 = 271;
/// `or`
pub const TK_OR: i32 = 272;
/// `repeat`
pub const TK_REPEAT: i32 = 273;
/// `return`
pub const TK_RETURN: i32 = 274;
/// `then`
pub const TK_THEN: i32 = 275;
/// `true`
pub const TK_TRUE: i32 = 276;
/// `until`
pub const TK_UNTIL: i32 = 277;
/// `while`  (last keyword; NUM_RESERVED = TK_WHILE - FIRST_RESERVED + 1 = 22)
pub const TK_WHILE: i32 = 278;
/// `//`  (floor division)
pub const TK_IDIV: i32 = 279;
/// `..`  (concatenation)
pub const TK_CONCAT: i32 = 280;
/// `...` (vararg)
pub const TK_DOTS: i32 = 281;
/// `==`
pub const TK_EQ: i32 = 282;
/// `>=`
pub const TK_GE: i32 = 283;
/// `<=`
pub const TK_LE: i32 = 284;
/// `~=`
pub const TK_NE: i32 = 285;
/// `<<`
pub const TK_SHL: i32 = 286;
/// `>>`
pub const TK_SHR: i32 = 287;
/// `::`
pub const TK_DBCOLON: i32 = 288;
/// `<eof>`
pub const TK_EOS: i32 = 289;
/// `<number>`  (float literal)
pub const TK_FLT: i32 = 290;
/// `<integer>` (integer literal)
pub const TK_INT: i32 = 291;
/// `<name>`    (identifier)
pub const TK_NAME: i32 = 292;
/// `<string>`  (string literal)
pub const TK_STRING: i32 = 293;

// Lua 5.5 `global`: with the upstream-default LUA_COMPAT_GLOBAL it is NOT a
// reserved word — it always lexes as TK_NAME (so it stays a valid identifier on
// every version), and the parser recognizes the `global` declaration statement
// contextually (see `globalstat`/`statement` in lua-parse). There is therefore
// no dedicated token id.

// ORDER RESERVED — index 0 = TK_AND - FIRST_RESERVED, etc.
/// Display strings for tokens, indexed by `token - FIRST_RESERVED`.
pub static LUAX_TOKENS: &[&[u8]] = &[
    // keywords (indices 0-21)
    b"and",
    b"break",
    b"do",
    b"else",
    b"elseif",
    b"end",
    b"false",
    b"for",
    b"function",
    b"goto",
    b"if",
    b"in",
    b"local",
    b"nil",
    b"not",
    b"or",
    b"repeat",
    b"return",
    b"then",
    b"true",
    b"until",
    b"while",
    // other terminal symbols (indices 22-35)
    b"//",
    b"..",
    b"...",
    b"==",
    b">=",
    b"<=",
    b"~=",
    b"<<",
    b">>",
    b"::",
    b"<eof>",
    b"<number>",
    b"<integer>",
    b"<name>",
    b"<string>",
];

// ── TokenValue ──────────────────────────────────────────────────────────────────

/// Semantic payload carried by a token.
#[derive(Clone)]
pub enum TokenValue {
    /// No semantic value (default; used for single-byte and most multi-char tokens).
    None,
    /// Float literal payload.
    Float(f64),
    /// Integer literal payload.
    Int(i64),
    /// String/name payload.
    Str(GcRef<LuaString>),
}

// ── Token ─────────────────────────────────────────────────────────────────────

/// A single lexed token with its semantic payload.
///
/// `kind` is an `i32` whose value is either an ASCII byte code (for single-byte
/// tokens like `+`, `-`, `[`) or one of the `TK_*` constants (for reserved
/// words, multi-char symbols, and literals).
///
/// The `i32` kind is the **cross-crate boundary**: `lua-parse` and the error
/// formatters consume these `TK_*` codes directly, so it stays an integer by
/// design rather than a typed enum. The lexer's own scan loop does its
/// dispatch on the internal [`Peek`] enum instead — the typed-match lives
/// inside the lexer, the integer codes cross the boundary.
#[derive(Clone)]
pub struct Token {
    pub kind: i32,
    pub value: TokenValue,
}

impl Token {
    /// Construct a token with no semantic value.
    pub fn new(kind: i32) -> Self {
        Token {
            kind,
            value: TokenValue::None,
        }
    }

    /// The end-of-stream sentinel token.
    pub fn eos() -> Self {
        Token::new(TK_EOS)
    }
}

// ── LexState ──────────────────────────────────────────────────────────────────

/// Per-chunk lexer (and shared parser) state.
///
/// Owns the input stream, token buffer, and current/lookahead tokens.
pub struct LexState {
    pub current: i32,
    pub linenumber: i32,
    pub lastline: i32,
    pub t: Token,
    pub lookahead: Token,
    pub fs: Option<()>,
    pub z: ZIO,
    pub buff: LexBuffer,
    pub h: Option<GcRef<LuaTable>>,
    /// Per-parse-session anchor for long strings, deduplicating all literal
    /// strings within a chunk so e.g. `local s1 <const>="..."` and
    /// `local s2 <const>="..."` with identical 50-byte payloads share one
    /// `LuaString` object — which is what makes
    /// `string.format("%p", s1) == string.format("%p", s2)` hold. Short strings
    /// already share identity via the global `interned_lt` pool, but long
    /// strings (>40 bytes) are not globally interned and need this session-level
    /// map. Keyed by the string bytes; populated lazily by `new_string`.
    pub long_str_anchor: std::collections::HashMap<Vec<u8>, GcRef<LuaString>>,
    pub dyd: Option<()>,
    pub source: GcRef<LuaString>,
    pub envn: GcRef<LuaString>,
    /// The active Lua version, snapshotted at lexer setup from
    /// `state.global().lua_version` (fixed for the lifetime of a parse). The
    /// error formatters (`lex_error`/`token2str`) take only `&LexState`, so they
    /// read the version here rather than threading a `&LuaState` through every
    /// syntax-error callsite. Lua 5.1 quotes the special multi-char token labels
    /// (`<eof>`, `<name>`, …) in error messages where 5.2+ leaves them bare.
    pub version: lua_types::LuaVersion,
}

// ── Character-classification helpers ─────────────────────────────────────────
//
// These take `i32` (a byte, or EOZ = -1) so EOZ returns false for all
// predicates.

#[inline]
fn is_digit(c: i32) -> bool {
    c >= b'0' as i32 && c <= b'9' as i32
}

#[inline]
fn is_xdigit(c: i32) -> bool {
    (c >= b'0' as i32 && c <= b'9' as i32)
        || (c >= b'a' as i32 && c <= b'f' as i32)
        || (c >= b'A' as i32 && c <= b'F' as i32)
}

// ASCII letters + '_'
#[inline]
fn is_lalpha(c: i32) -> bool {
    (c >= b'a' as i32 && c <= b'z' as i32)
        || (c >= b'A' as i32 && c <= b'Z' as i32)
        || c == b'_' as i32
}

#[inline]
fn is_lalnum(c: i32) -> bool {
    is_lalpha(c) || is_digit(c)
}

#[inline]
fn is_space(c: i32) -> bool {
    matches!(c, 9 | 10 | 11 | 12 | 13 | 32) // \t \n \v \f \r space
}

// printable ASCII (graph + space), i.e. 0x20-0x7E
#[inline]
fn is_print(c: i32) -> bool {
    c >= 0x20 && c <= 0x7E
}

#[inline]
fn curr_is_newline(ls: &LexState) -> bool {
    ls.current == b'\n' as i32 || ls.current == b'\r' as i32
}

// ── Low-level stream helpers ───────────────────────────────────────────────────

/// Advance the lexer by one character.
///
/// Named `advance` to avoid collision with Rust's iterator method.
#[inline]
fn advance(ls: &mut LexState) {
    ls.current = ls.z.getc();
}

/// Append character `c` to the token buffer, growing it if necessary.
///
/// On overflow calls [`lex_error`] which becomes `Err(LuaError::Syntax(...))`.
fn save(ls: &mut LexState, state: &mut LuaState, c: i32) -> Result<(), LuaError> {
    if ls.buff.len() + 1 > ls.buff.capacity() {
        if ls.buff.capacity() >= MAX_SIZE / 2 {
            return Err(lex_error(ls, b"lexical element too long", 0));
        }
        let newsize = ls.buff.capacity() * 2;
        ls.buff.resize(state, newsize)?;
    }
    ls.buff.push_byte(c as u8);
    Ok(())
}

/// Save the current character into the token buffer, then advance the stream.
///
/// Fallible because `save` may need to grow the buffer.
#[inline]
fn save_and_next(ls: &mut LexState, state: &mut LuaState) -> Result<(), LuaError> {
    let c = ls.current;
    save(ls, state, c)?;
    advance(ls);
    Ok(())
}

// ── Error helpers ─────────────────────────────────────────────────────────────

/// Build a syntax error, optionally annotated with the offending token text.
///
/// Returns the constructed [`LuaError`] by value rather than diverging. This is
/// the lexer's error-construction boundary: the lexer's own callsites wrap it as
/// `return Err(lex_error(...))`, and `lua-parse` does the same across the crate
/// boundary, so the by-value return is a deliberate public contract — not a
/// half-finished translation of a diverging C function. The error text format
/// (`source:line: message` plus an optional ` near <token>` suffix) is the
/// behavioural invariant the oracle checks.
pub fn lex_error(ls: &mut LexState, msg: &[u8], token: i32) -> LuaError {
    const LUA_IDSIZE: usize = 60;
    let mut buff = [0u8; LUA_IDSIZE];
    let n = lua_vm::object::chunk_id(&mut buff[..], ls.source.as_bytes());
    let src_part = &buff[..n];

    let mut full_msg: Vec<u8> = Vec::new();
    full_msg.extend_from_slice(src_part);
    let _ = write!(full_msg, ":{}: ", ls.linenumber);
    full_msg.extend_from_slice(msg);

    if token != 0 {
        let tok_text = txt_token(ls, token);
        full_msg.extend_from_slice(b" near ");
        full_msg.extend_from_slice(&tok_text);
    }

    LuaError::syntax_raw(&full_msg)
}

/// Report a syntax error at the current token.
pub fn syntax_error(ls: &mut LexState, msg: &[u8]) -> LuaError {
    let token = ls.t.kind;
    lex_error(ls, msg, token)
}

/// Report a semantic error at the current line WITHOUT the `near <token>`
/// suffix.
///
/// The `near` clause is suppressed by passing token `0`. Used for attribute
/// errors (`unknown attribute '<name>'`, `global variables cannot be
/// to-be-closed`) where the offending construct is the attribute itself, not
/// the current lookahead token.
pub fn sem_error(ls: &mut LexState, msg: &[u8]) -> LuaError {
    lex_error(ls, msg, 0)
}

/// Produce a human-readable representation of `token` for error messages.
///
/// For `TK_NAME`, `TK_STRING`, `TK_FLT`, `TK_INT`: formats the current
/// token buffer contents as `'<text>'`.  For everything else, delegates to
/// [`token2str`].
fn txt_token(ls: &mut LexState, token: i32) -> Vec<u8> {
    match token {
        t if t == TK_NAME || t == TK_STRING || t == TK_FLT || t == TK_INT => {
            let mut v: Vec<u8> = Vec::new();
            v.push(b'\'');
            v.extend_from_slice(ls.buff.without_trailing_nul());
            v.push(b'\'');
            v
        }
        _ => token2str_raw(token, ls.version),
    }
}

/// Produce a human-readable token description (for error messages and the parser).
///
/// Single-byte printable tokens are formatted as `'X'`; non-printable as
/// `'<\N>'`.  Reserved words and multi-char symbols are formatted as `'kw'`.
/// Literal tokens (`<name>`, `<string>`, etc.) return the bare label.
pub fn token2str(ls: &LexState, token: i32) -> Vec<u8> {
    token2str_raw(token, ls.version)
}

/// Inner implementation of [`token2str`] that does not need `LexState`.
///
/// `version` gates the 5.1 special-token quoting. Lua 5.1 wraps the whole
/// near/expected token in quotes, so the bare multi-char labels (`<eof>`,
/// `<name>`, …) returned for `token >= TK_EOS` end up quoted. 5.2 leaves those
/// bare and quotes only symbols/reserved/literals, so for 5.2+ the `>= TK_EOS`
/// arm stays unquoted. (Issue #105.)
///
/// `version` also gates the rendering of a non-printable single byte. Lua 5.2's
/// `luaX_token2str` formats such a byte as the bare label `char(%d)` (the
/// surrounding `near '...'` quoting is suppressed for tokens whose text starts
/// with `char(`), whereas 5.3+ render it as the quoted `'<\\%d>'`.
fn token2str_raw(token: i32, version: lua_types::LuaVersion) -> Vec<u8> {
    if token < FIRST_RESERVED {
        if is_print(token) {
            vec![b'\'', token as u8, b'\'']
        } else if version == lua_types::LuaVersion::V52 {
            let mut v: Vec<u8> = Vec::new();
            v.extend_from_slice(b"char(");
            let _ = write!(&mut v, "{}", token);
            v.push(b')');
            v
        } else {
            let mut v: Vec<u8> = Vec::new();
            v.extend_from_slice(b"'<\\");
            let _ = write!(&mut v, "{}", token);
            v.extend_from_slice(b">'");
            v
        }
    } else {
        let idx = (token - FIRST_RESERVED) as usize;
        let s = LUAX_TOKENS[idx];
        if token < TK_EOS || version == lua_types::LuaVersion::V51 {
            let mut v: Vec<u8> = Vec::with_capacity(s.len() + 2);
            v.push(b'\'');
            v.extend_from_slice(s);
            v.push(b'\'');
            v
        } else {
            s.to_vec()
        }
    }
}

// ── Public init / setup ───────────────────────────────────────────────────────

/// Initialise the lexer subsystem: intern all reserved words and fix them
/// in the GC so they are never collected.
///
/// Must be called exactly once during VM startup.
pub fn init(state: &mut LuaState) -> Result<(), LuaError> {
    let _e = intern_str_stub(state, LUA_ENV)?;

    for i in 0..NUM_RESERVED {
        let ts = intern_str_stub(state, LUAX_TOKENS[i])?;
        let _ = ts;
    }

    Ok(())
}

/// Initialise `ls` for lexing a new chunk from stream `z`.
pub fn set_input(
    state: &mut LuaState,
    ls: &mut LexState,
    z: ZIO,
    source: GcRef<LuaString>,
    firstchar: i32,
) -> Result<(), LuaError> {
    ls.t = Token::new(0);
    ls.current = firstchar;
    ls.lookahead = Token::eos();
    ls.z = z;
    ls.fs = None;
    ls.linenumber = 1;
    ls.lastline = 1;
    ls.source = source;
    ls.version = state.global().lua_version;
    ls.envn = intern_str_stub(state, LUA_ENV)?;
    ls.buff.resize(state, LUA_MIN_BUFFER)?;
    Ok(())
}

/// Create (or retrieve) a Lua string and anchor it in the parser's GC-protection
/// table `ls.h` so it cannot be collected before the end of compilation.
///
/// Also internalises long strings so that each unique content has exactly one
/// copy in memory.  The table `ls.h` is used as a set: the string is both the
/// key and the value.
pub(crate) fn new_string(
    state: &mut LuaState,
    ls: &mut LexState,
    bytes: &[u8],
) -> Result<GcRef<LuaString>, LuaError> {
    // The `long_str_anchor` map dedups so a second occurrence of the same
    // literal in the chunk returns the originally-created string; identity is
    // determined by the `Rc` allocation. Short strings already share identity
    // via the global pool; long strings (>40 bytes) need this session-level map.
    if let Some(existing) = ls.long_str_anchor.get(bytes) {
        return Ok(existing.clone());
    }
    let ts = intern_str_stub(state, bytes)?;
    ls.long_str_anchor.insert(bytes.to_vec(), ts.clone());
    Ok(ts)
}

// ── Public advance / lookahead ─────────────────────────────────────────────────

/// Consume the current token; load the next one from the stream.
///
/// If a lookahead token was set, it becomes the current token without re-reading
/// from the stream.
pub fn next(state: &mut LuaState, ls: &mut LexState) -> Result<(), LuaError> {
    ls.lastline = ls.linenumber;

    if ls.lookahead.kind != TK_EOS {
        // Clone to avoid borrow conflict; LuaString inside TokenValue is GcRef (Rc).
        ls.t = ls.lookahead.clone();
        ls.lookahead = Token::eos();
    } else {
        let mut val = TokenValue::None;
        let kind = llex(state, ls, &mut val)?;
        ls.t = Token { kind, value: val };
    }
    Ok(())
}

/// Peek at the next token without consuming the current one.
///
/// The lookahead token is cached in `ls.lookahead` and returned.  Only one
/// token of lookahead is supported; calling this twice without an intervening
/// [`next`] is a logic error (asserted in debug builds).
pub fn lookahead(state: &mut LuaState, ls: &mut LexState) -> Result<i32, LuaError> {
    debug_assert!(
        ls.lookahead.kind == TK_EOS,
        "luaX_lookahead: lookahead already set"
    );

    let mut val = TokenValue::None;
    let kind = llex(state, ls, &mut val)?;
    ls.lookahead = Token { kind, value: val };

    Ok(ls.lookahead.kind)
}

// ── Private lexer helpers ──────────────────────────────────────────────────────

/// If the current character equals `c`, advance and return `true`.
fn check_next1(ls: &mut LexState, c: i32) -> bool {
    if ls.current == c {
        advance(ls);
        true
    } else {
        false
    }
}

/// If the current character is either of the two bytes in `set`, save-and-advance
/// and return `true`.
fn check_next2(ls: &mut LexState, state: &mut LuaState, set: &[u8; 2]) -> Result<bool, LuaError> {
    if ls.current == set[0] as i32 || ls.current == set[1] as i32 {
        save_and_next(ls, state)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Increment the line counter and consume the newline sequence.
///
/// Handles `\n`, `\r`, `\n\r`, and `\r\n`: a second newline byte is only
/// consumed when it differs from the first, so a `\n\n` (two blank lines)
/// is not collapsed into one.
fn inc_line_number(ls: &mut LexState, _state: &mut LuaState) -> Result<(), LuaError> {
    debug_assert!(curr_is_newline(ls), "inc_line_number: not at a newline");

    let old = ls.current;
    advance(ls);

    if curr_is_newline(ls) && ls.current != old {
        advance(ls);
    }

    ls.linenumber += 1;
    if ls.linenumber >= i32::MAX {
        return Err(lex_error(ls, b"chunk has too many lines", 0));
    }
    Ok(())
}

/// Scan a numeric literal (integer or float, decimal or hex).
///
/// The caller may have already read an initial dot.  Accepts the pattern:
/// `%d(%x|%.|(Ee[+-]?))*` or `0[Xx](%x|%.|(Pp[+-]?))*`.
///
/// Returns `TK_INT` for integers, `TK_FLT` for floats.
fn read_numeral(
    state: &mut LuaState,
    ls: &mut LexState,
    seminfo: &mut TokenValue,
) -> Result<i32, LuaError> {
    let mut expo: &[u8; 2] = b"Ee";

    let first = ls.current;

    debug_assert!(is_digit(ls.current), "read_numeral: not at a digit");

    save_and_next(ls, state)?;

    let is_hex = first == b'0' as i32 && check_next2(ls, state, b"xX")?;

    // Lua 5.1 has no hexadecimal-aware number scanner: it never recognizes a
    // binary (`Pp`) exponent, a signed binary exponent, or a fractional `.`
    // inside a hex literal. Its scanner reads decimal digits/`.`, an optional
    // `Ee` exponent, then swallows the rest of the alphanumeric run (the hex
    // body) and hands the whole token to `strtod`/`strtoul`. So:
    //   * `0x1p4`  -> the `p4` rides the alphanumeric tail -> strtod -> 16
    //   * `0x1p-2` -> the tail stops at `-`, leaving `0x1p` -> malformed
    //   * `0x1.8p0`-> the main loop stops at `.`, leaving `0x1` (=1); `.8p0`
    //                 then lexes as a fresh, malformed decimal numeral.
    // 5.2+ added the hex-aware scanner (`Pp` exponent + signed exponent +
    // fractional `.`), so the hex-special path below is gated off for V51.
    let is_v51 = matches!(state.global().lua_version, lua_types::LuaVersion::V51);
    let hex_aware = is_hex && !is_v51;
    if hex_aware {
        expo = b"Pp";
    }

    // For a 5.1 hex literal the digit-scanning main loop is skipped entirely so
    // its `Ee`-exponent probe cannot mistake a hex `e`/`E` digit for a decimal
    // exponent (`0x1e2` is 482, not `0x1` × 10^2); the whole hex body rides the
    // alphanumeric tail below, exactly as 5.1's scanner does.
    let scan_digits = !(is_hex && is_v51);
    while scan_digits {
        if check_next2(ls, state, expo)? {
            check_next2(ls, state, b"-+")?;
        } else if is_xdigit(ls.current) || (ls.current == b'.' as i32 && (!is_hex || hex_aware)) {
            save_and_next(ls, state)?;
        } else {
            break;
        }
    }

    // Numeral "touching a letter" handling. 5.2+ append a single trailing letter
    // to force a malformed-number error (the alphanumeric run after it lexes
    // separately). Lua 5.1's scanner instead swallows the *entire* alphanumeric
    // (and `_`) run into the numeral, so a malformed 5.1 numeral reports the full
    // run in its `near '...'` snippet (`.8p0`, `3xyz`), not just its first letter.
    if matches!(state.global().lua_version, lua_types::LuaVersion::V51) {
        while is_lalnum(ls.current) {
            save_and_next(ls, state)?;
        }
    } else if is_lalpha(ls.current) {
        save_and_next(ls, state)?;
    }

    save(ls, state, 0)?;

    match parse_numeral(ls.buff.without_trailing_nul()) {
        None => Err(lex_error(ls, b"malformed number", TK_FLT)),
        Some(lua_types::LuaValue::Int(i)) => {
            if is_float_only(state) {
                // The float-only family (5.1/5.2) has no integer subtype: every
                // numeral becomes a float. A hex integer literal is read through
                // C's `strtoul(.., 16)`, an *unsigned* 64-bit conversion, so an
                // overflowing hex literal (top bit set) becomes a large positive
                // double — `0xFFFFFFFFFFFFFFFF` is `1.844674407371e+19`, not -1.
                // `str2int` wraps into `i64`, so reinterpret the bit pattern as
                // `u64` before widening; for any non-overflowing literal this is
                // identical to `i as f64`.
                let widened = if is_hex { (i as u64) as f64 } else { i as f64 };
                *seminfo = TokenValue::Float(widened);
                Ok(TK_FLT)
            } else {
                *seminfo = TokenValue::Int(i);
                Ok(TK_INT)
            }
        }
        Some(lua_types::LuaValue::Float(f)) => {
            *seminfo = TokenValue::Float(f);
            Ok(TK_FLT)
        }
        Some(other) => unreachable!("parse_numeral yielded non-numeric value: {other:?}"),
    }
}

/// Scan a `[=*[` or `]=*]` sequence; leave the last bracket as current char.
///
/// Returns:
/// - `count + 2` if well-formed (where `count` is the number of `=` signs),
/// - `1` if a single bracket with no `=`s and no second bracket,
/// - `0` if malformed (e.g. `[==` with no closing bracket).
fn skip_sep(state: &mut LuaState, ls: &mut LexState) -> Result<usize, LuaError> {
    let mut count: usize = 0;
    let s = ls.current;
    debug_assert!(
        s == b'[' as i32 || s == b']' as i32,
        "skip_sep: not at bracket"
    );

    save_and_next(ls, state)?;

    while ls.current == b'=' as i32 {
        save_and_next(ls, state)?;
        count += 1;
    }

    if ls.current == s {
        Ok(count + 2)
    } else if count == 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Scan a long string or long comment delimited by `[=*[` … `]=*]`.
///
/// `seminfo` is `Some` when reading a string literal; `None` when skipping a
/// long comment.  When `None`, buffer contents are discarded on each newline
/// to avoid wasting memory.
fn read_long_string(
    state: &mut LuaState,
    ls: &mut LexState,
    seminfo: Option<&mut TokenValue>,
    sep: usize,
) -> Result<(), LuaError> {
    let line = ls.linenumber;

    save_and_next(ls, state)?;

    if curr_is_newline(ls) {
        inc_line_number(ls, state)?;
    }

    // is_string: whether we are reading a string (true) or a comment (false)
    let is_string = seminfo.is_some();

    loop {
        match ls.current {
            c if c == EOZ => {
                let what: &[u8] = if is_string { b"string" } else { b"comment" };
                let mut msg: Vec<u8> = Vec::new();
                msg.extend_from_slice(b"unfinished long ");
                msg.extend_from_slice(what);
                // The "(starting at line N)" clause was added in 5.3. The
                // float-only family (5.1/5.2) reports the bare
                // "unfinished long string"/"unfinished long comment".
                if !is_float_only(state) {
                    msg.extend_from_slice(b" (starting at line ");
                    let _ = write!(&mut msg, "{}", line);
                    msg.push(b')');
                }
                return Err(lex_error(ls, &msg, TK_EOS));
            }
            c if c == b']' as i32 => {
                let s = skip_sep(state, ls)?;
                if s == sep {
                    save_and_next(ls, state)?;
                    break;
                }
                // else: the ']' sequence wasn't the closing delimiter; continue
            }
            c if c == b'\n' as i32 || c == b'\r' as i32 => {
                save(ls, state, b'\n' as i32)?;
                inc_line_number(ls, state)?;
                if !is_string {
                    ls.buff.clear();
                }
            }
            _ => {
                if is_string {
                    save_and_next(ls, state)?;
                } else {
                    advance(ls);
                }
            }
        }
    }

    if let Some(out) = seminfo {
        // The buffer holds `[=*[` + content + `]=*]`, each delimiter `sep` bytes
        // wide; strip both to recover the content.
        let content = ls.buff.trim_ends(sep);
        let ts = new_string(state, ls, &content)?;
        *out = TokenValue::Str(ts);
    }
    Ok(())
}

/// Check `c` is non-zero (truthy); if not, save the current char and raise a
/// string-escape error.
fn esc_check(
    state: &mut LuaState,
    ls: &mut LexState,
    ok: bool,
    msg: &[u8],
) -> Result<(), LuaError> {
    if !ok {
        if ls.current != EOZ {
            save_and_next(ls, state)?;
        }
        return Err(lex_error(ls, msg, TK_STRING));
    }
    Ok(())
}

/// Build a Lua 5.2-family escape-sequence error whose `near '...'` snippet is
/// the backslash plus the offending escape characters (`'\j'`, `'\256'`,
/// `'\xZ'`), not the buffer collected so far.
///
/// This mirrors 5.2's `escerror`, which resets the token buffer and saves `'\\'`
/// followed by the escape characters before raising. (5.3+ leaves the whole
/// buffer in place, so its snippet keeps the opening quote and prior content —
/// that path stays on [`esc_check`].) Trailing `EOZ` markers in `chars` are
/// dropped, matching `escerror`'s `c[i] != EOZ` guard.
fn esc_error_legacy(ls: &mut LexState, chars: &[i32], msg: &[u8]) -> LuaError {
    ls.buff.clear();
    ls.buff.push_byte(b'\\');
    for &c in chars {
        if c == EOZ {
            break;
        }
        ls.buff.push_byte(c as u8);
    }
    lex_error(ls, msg, TK_STRING)
}

/// Save-and-advance, then verify the new current char is a hex digit; return
/// its numeric value (0-15).
///
/// On failure the `near '...'` snippet is version-gated. 5.2 reports `\x` plus
/// the characters read so far (`'\xZ'`, `'\x1Z'`), built by resetting the buffer
/// (mirroring `escerror`); `chars` carries the hex characters already consumed by
/// the enclosing `\x` escape so the snippet can be rebuilt. 5.3+ report the whole
/// buffer-so-far via the save-and-next inside `esc_check`. (5.2 has no `\u`, so a
/// hex-digit error here always belongs to a `\x` escape.)
fn get_hexa(state: &mut LuaState, ls: &mut LexState, chars: &mut Vec<i32>) -> Result<u32, LuaError> {
    save_and_next(ls, state)?;
    if !is_xdigit(ls.current) {
        if matches!(state.global().lua_version, lua_types::LuaVersion::V52) {
            let mut esc: Vec<i32> = Vec::with_capacity(chars.len() + 2);
            esc.push(b'x' as i32);
            esc.extend_from_slice(chars);
            esc.push(ls.current);
            return Err(esc_error_legacy(ls, &esc, b"hexadecimal digit expected"));
        }
        esc_check(state, ls, false, b"hexadecimal digit expected")?;
    }
    chars.push(ls.current);
    Ok(hex_value_stub(ls.current))
}

/// Scan a `\xNN` hex escape; return the decoded byte value.
fn read_hex_esc(state: &mut LuaState, ls: &mut LexState) -> Result<u32, LuaError> {
    let mut chars: Vec<i32> = Vec::with_capacity(2);
    let r = get_hexa(state, ls, &mut chars)?;
    let r = (r << 4) + get_hexa(state, ls, &mut chars)?;
    ls.buff.truncate_by(2);
    Ok(r)
}

/// Scan a `\u{XXXXXX}` UTF-8 escape; return the Unicode codepoint.
fn read_utf8_esc(state: &mut LuaState, ls: &mut LexState) -> Result<u32, LuaError> {
    let mut i: usize = 4;

    save_and_next(ls, state)?;

    esc_check(state, ls, ls.current == b'{' as i32, b"missing '{'")?;

    // `\u{...}` exists only on 5.3+, so the 5.2 reset-buffer error branch in
    // `get_hexa` is never taken from here; the accumulator is unused.
    let mut hex_chars: Vec<i32> = Vec::new();
    let mut r = get_hexa(state, ls, &mut hex_chars)?;

    // The codepoint upper bound is version-gated, and the digit-accumulation
    // order differs between the families:
    //   * 5.3: accumulate the digit FIRST (`r = (r<<4)+digit`), THEN bound the
    //     running value at 0x10FFFF.
    //   * 5.4 / 5.5: bound BEFORE the shift (`r <= 0x7FFFFFFF >> 4`), then
    //     accumulate — allowing codepoints up to 0x7FFFFFFF.
    // The order (check-before-shift vs shift-before-check) is load-bearing: it
    // also determines how many digits land in the `near '...'` snippet of the
    // "UTF-8 value too large" error, so a too-large `\u{...}` reports the same
    // message and offset as the matching reference binary.
    let is_v53 = matches!(state.global().lua_version, lua_types::LuaVersion::V53);

    loop {
        save_and_next(ls, state)?;
        if !is_xdigit(ls.current) {
            break;
        }
        i += 1;
        if is_v53 {
            r = (r << 4) + hex_value_stub(ls.current);
            esc_check(state, ls, r <= 0x10_FFFF, b"UTF-8 value too large")?;
        } else {
            esc_check(
                state,
                ls,
                r <= (0x7FFF_FFFFu32 >> 4),
                b"UTF-8 value too large",
            )?;
            r = (r << 4) + hex_value_stub(ls.current);
        }
    }

    esc_check(state, ls, ls.current == b'}' as i32, b"missing '}'")?;

    advance(ls);

    ls.buff.truncate_by(i);

    Ok(r)
}

/// Scan `\u{...}` and append the UTF-8 encoding of the codepoint to the buffer.
fn utf8_esc(state: &mut LuaState, ls: &mut LexState) -> Result<(), LuaError> {
    let codepoint = read_utf8_esc(state, ls)?;

    let encoded = utf8_encode_stub(codepoint);

    for &b in &encoded {
        save(ls, state, b as i32)?;
    }
    Ok(())
}

/// Scan a decimal escape `\ddd` (up to 3 digits); return the byte value.
fn read_dec_esc(state: &mut LuaState, ls: &mut LexState) -> Result<u32, LuaError> {
    let mut i: usize = 0;
    let mut r: u32 = 0;
    let mut digits: Vec<i32> = Vec::with_capacity(3);

    while i < 3 && is_digit(ls.current) {
        r = 10 * r + (ls.current as u32 - b'0' as u32);
        digits.push(ls.current);
        save_and_next(ls, state)?;
        i += 1;
    }

    if r > u8::MAX as u32 {
        // The "too large" message and `near '...'` snippet are version-gated:
        //   * 5.1: spelled "escape sequence too large"; its scanner saves
        //     neither the backslash nor the digits, so the snippet is just the
        //     opening quote (`'"'`). The shared decode path *did* buffer them
        //     here, so trim the backslash + `i` digits back off before raising.
        //   * 5.2: "decimal escape too large", snippet is `\` + the digits
        //     (`'\256'`), built by resetting the buffer (see `esc_error_legacy`).
        //   * 5.3+: "decimal escape too large", snippet is the buffer collected
        //     so far (opening quote + content + `\` + digits + the trailing char
        //     `esc_check` save-and-nexts on).
        let version = state.global().lua_version;
        match version {
            lua_types::LuaVersion::V51 => {
                ls.buff.truncate_by(i + 1);
                return Err(lex_error(ls, b"escape sequence too large", TK_STRING));
            }
            lua_types::LuaVersion::V52 => {
                return Err(esc_error_legacy(ls, &digits, b"decimal escape too large"));
            }
            _ => {
                esc_check(state, ls, false, b"decimal escape too large")?;
            }
        }
    }

    ls.buff.truncate_by(i);
    Ok(r)
}

/// Scan a short (single/double-quoted) string literal.
///
/// Escape-sequence handling decodes a byte and then dispatches via the local
/// [`EscapeResult`] enum.
fn read_string(
    state: &mut LuaState,
    ls: &mut LexState,
    del: i32,
    seminfo: &mut TokenValue,
) -> Result<(), LuaError> {
    // What the escape-sequence handler does after decoding a byte.
    //
    // ReadSave:  advance, remove '\' from buffer, save decoded byte.
    // OnlySave:  remove '\' from buffer, save decoded byte (no advance).
    // NoSave:    nothing (just break from the escape case).
    enum EscapeResult {
        ReadSave(i32),
        OnlySave(i32),
        NoSave,
    }

    save_and_next(ls, state)?;

    while ls.current != del {
        match ls.current {
            c if c == EOZ => {
                return Err(lex_error(ls, b"unfinished string", TK_EOS));
            }
            c if c == b'\n' as i32 || c == b'\r' as i32 => {
                return Err(lex_error(ls, b"unfinished string", TK_STRING));
            }
            c if c == b'\\' as i32 => {
                save_and_next(ls, state)?;

                // Lua 5.1's lexer does NOT recognize `\x`, `\z`, or `\u`, and it
                // does NOT raise on an unknown escape. For any escape char outside
                // the known set, the 5.1 lexer silently drops the backslash and
                // keeps the next character verbatim (`"\x41"` → bytes `x41`,
                // `"\z"` → `z`, `"\q"` → `q`). Decimal escapes (`\ddd`) and the
                // standard letter/quote/newline escapes still work. Verified
                // against lua5.1.5; see specs/followup/5.1-roster-syntax.md §2.
                let is_v51 = matches!(state.global().lua_version, lua_types::LuaVersion::V51);
                // The `\u{...}` escape is a 5.3 addition. 5.1 has no `\u` at all
                // (silently drops the backslash); 5.2 added `\x` and `\z` but NOT
                // `\u`, so on the float-only family `\u` is an invalid escape.
                let has_u_escape = !is_float_only(state);

                // Inner switch on the escape character
                let esc = match ls.current {
                    c if c == b'a' as i32 => EscapeResult::ReadSave(b'\x07' as i32),
                    c if c == b'b' as i32 => EscapeResult::ReadSave(b'\x08' as i32),
                    c if c == b'f' as i32 => EscapeResult::ReadSave(b'\x0C' as i32),
                    c if c == b'n' as i32 => EscapeResult::ReadSave(b'\n' as i32),
                    c if c == b'r' as i32 => EscapeResult::ReadSave(b'\r' as i32),
                    c if c == b't' as i32 => EscapeResult::ReadSave(b'\t' as i32),
                    c if c == b'v' as i32 => EscapeResult::ReadSave(b'\x0B' as i32),
                    c if c == b'x' as i32 && !is_v51 => {
                        let decoded = read_hex_esc(state, ls)?;
                        EscapeResult::ReadSave(decoded as i32)
                    }
                    c if c == b'u' as i32 && has_u_escape => {
                        utf8_esc(state, ls)?;
                        EscapeResult::NoSave
                    }
                    c if c == b'\n' as i32 || c == b'\r' as i32 => {
                        inc_line_number(ls, state)?;
                        EscapeResult::OnlySave(b'\n' as i32)
                    }
                    c if c == b'\\' as i32 || c == b'"' as i32 || c == b'\'' as i32 => {
                        EscapeResult::ReadSave(c)
                    }
                    c if c == EOZ => EscapeResult::NoSave,
                    c if c == b'z' as i32 && !is_v51 => {
                        ls.buff.truncate_by(1);
                        advance(ls);
                        while is_space(ls.current) {
                            if curr_is_newline(ls) {
                                inc_line_number(ls, state)?;
                            } else {
                                advance(ls);
                            }
                        }
                        EscapeResult::NoSave
                    }
                    c if is_v51 && !is_digit(c) => {
                        // 5.1 unknown escape: drop the backslash, emit the char.
                        EscapeResult::ReadSave(c)
                    }
                    _ => {
                        if !is_digit(ls.current) {
                            // 5.2 reports the invalid escape as `\` + the offending
                            // char (`'\j'`, `'\u'`) by resetting the buffer; 5.3+
                            // report the whole buffer-so-far (`'"\j'`) via the
                            // save-and-next inside `esc_check`.
                            if matches!(state.global().lua_version, lua_types::LuaVersion::V52) {
                                return Err(esc_error_legacy(
                                    ls,
                                    &[ls.current],
                                    b"invalid escape sequence",
                                ));
                            }
                            esc_check(state, ls, false, b"invalid escape sequence")?;
                        }
                        let decoded = read_dec_esc(state, ls)?;
                        EscapeResult::OnlySave(decoded as i32)
                    }
                };

                match esc {
                    EscapeResult::ReadSave(c) => {
                        advance(ls);
                        ls.buff.truncate_by(1);
                        save(ls, state, c)?;
                    }
                    EscapeResult::OnlySave(c) => {
                        ls.buff.truncate_by(1);
                        save(ls, state, c)?;
                    }
                    EscapeResult::NoSave => {}
                }
            }
            _ => {
                save_and_next(ls, state)?;
            }
        }
    }

    save_and_next(ls, state)?;

    // The buffer holds the opening quote + content + closing quote; strip the
    // one-byte delimiter from each end.
    let content = ls.buff.trim_ends(1);
    let ts = new_string(state, ls, &content)?;
    *seminfo = TokenValue::Str(ts);
    Ok(())
}

/// Whether the active version is the float-only legacy family (5.1/5.2), which
/// lacks the 5.3 integer operators (`//`, `<<`, `>>`, and the bitwise binops).
fn is_float_only(state: &LuaState) -> bool {
    matches!(
        state.global().lua_version,
        lua_types::LuaVersion::V51 | lua_types::LuaVersion::V52
    )
}

/// The lexer's view of the current input character.
///
/// `ls.current` is stored as `i32` (a byte, or [`EOZ`] = `-1`) at the stream
/// boundary; this enum is the lexer's *internal* dispatch shape so the main
/// scan loop can `match` on byte literals (`Byte(b'=')`) and on end-of-stream
/// (`Eoz`) directly instead of an `i32` guard chain. It does not cross the
/// public boundary: scanned tokens are still reported as `TK_*` / ASCII `i32`.
enum Peek {
    Byte(u8),
    Eoz,
}

/// Classify [`LexState::current`] into the internal [`Peek`] dispatch shape.
fn peek(ls: &LexState) -> Peek {
    match u8::try_from(ls.current) {
        Ok(b) => Peek::Byte(b),
        Err(_) => Peek::Eoz,
    }
}

/// Core lexer dispatch: consume and return the next raw token kind.
///
/// The heart of the lexer: a scan loop that classifies the current character
/// (via [`Peek`]) and dispatches to the appropriate scanner. Single-character
/// tokens are returned as their ASCII `i32`; multi-character and reserved-word
/// tokens as the `TK_*` constants.
fn llex(
    state: &mut LuaState,
    ls: &mut LexState,
    seminfo: &mut TokenValue,
) -> Result<i32, LuaError> {
    ls.buff.clear();

    loop {
        match peek(ls) {
            Peek::Byte(b'\n' | b'\r') => {
                inc_line_number(ls, state)?;
                // Shebang handling: the reference loader strips a leading '#'
                // line (Unix shebang). Our test harness prepends a global-setup
                // preamble to every official test, so the script's '#' line is
                // not at byte zero. Apply the same rule at any token-scan line
                // start: treat a line whose first character is '#' as a
                // single-line comment. This sits in the dispatch loop (not
                // inc_line_number) so it does not affect newlines inside
                // long-bracket strings.
                if ls.current == b'#' as i32 {
                    while !curr_is_newline(ls) && ls.current != EOZ {
                        advance(ls);
                    }
                }
            }

            Peek::Byte(b' ' | b'\x0C' | b'\t' | b'\x0B') => {
                advance(ls);
            }

            Peek::Byte(b'-') => {
                advance(ls);
                if ls.current != b'-' as i32 {
                    return Ok(b'-' as i32);
                }
                advance(ls);

                if ls.current == b'[' as i32 {
                    let sep = skip_sep(state, ls)?;
                    ls.buff.clear();
                    if sep >= 2 {
                        read_long_string(state, ls, None, sep)?;
                        ls.buff.clear();
                        continue;
                    }
                }
                while !curr_is_newline(ls) && ls.current != EOZ {
                    advance(ls);
                }
                // loop continues (no token emitted for comments)
            }

            Peek::Byte(b'[') => {
                let sep = skip_sep(state, ls)?;
                if sep >= 2 {
                    read_long_string(state, ls, Some(seminfo), sep)?;
                    return Ok(TK_STRING);
                } else if sep == 0 {
                    return Err(lex_error(ls, b"invalid long string delimiter", TK_STRING));
                }
                return Ok(b'[' as i32);
            }

            Peek::Byte(b'=') => {
                advance(ls);
                if check_next1(ls, b'=' as i32) {
                    return Ok(TK_EQ);
                }
                return Ok(b'=' as i32);
            }

            Peek::Byte(b'<') => {
                advance(ls);
                if check_next1(ls, b'=' as i32) {
                    return Ok(TK_LE);
                } else if !is_float_only(state) && check_next1(ls, b'<' as i32) {
                    // The `<<` shift operator is a Lua 5.3 addition. Under the
                    // float-only legacy family (5.1/5.2) it does not exist: a
                    // bare `<` is returned, so a second `<` then surfaces
                    // upstream's "unexpected symbol near '<'".
                    return Ok(TK_SHL);
                }
                return Ok(b'<' as i32);
            }

            Peek::Byte(b'>') => {
                advance(ls);
                if check_next1(ls, b'=' as i32) {
                    return Ok(TK_GE);
                } else if !is_float_only(state) && check_next1(ls, b'>' as i32) {
                    // `>>` is a 5.3 addition; absent in 5.1/5.2.
                    return Ok(TK_SHR);
                }
                return Ok(b'>' as i32);
            }

            Peek::Byte(b'/') => {
                advance(ls);
                if !is_float_only(state) && check_next1(ls, b'/' as i32) {
                    // Floor division `//` is a 5.3 addition; absent in 5.1/5.2,
                    // where the second `/` becomes "unexpected symbol near '/'".
                    return Ok(TK_IDIV);
                }
                return Ok(b'/' as i32);
            }

            Peek::Byte(b'~') => {
                advance(ls);
                if check_next1(ls, b'=' as i32) {
                    return Ok(TK_NE);
                }
                return Ok(b'~' as i32);
            }

            Peek::Byte(b':') => {
                advance(ls);
                // Lua 5.1 has no `::label::` token; `::` was added with `goto` in
                // 5.2. Under V51 the second `:` is left for the parser, which
                // reports `unexpected symbol near ':'`. See
                // specs/followup/5.1-roster-syntax.md §2.
                let is_v51 = matches!(state.global().lua_version, lua_types::LuaVersion::V51);
                if !is_v51 && check_next1(ls, b':' as i32) {
                    return Ok(TK_DBCOLON);
                }
                return Ok(b':' as i32);
            }

            Peek::Byte(b'"' | b'\'') => {
                let del = ls.current;
                read_string(state, ls, del, seminfo)?;
                return Ok(TK_STRING);
            }

            Peek::Byte(b'.') => {
                save_and_next(ls, state)?;
                if check_next1(ls, b'.' as i32) {
                    if check_next1(ls, b'.' as i32) {
                        return Ok(TK_DOTS);
                    }
                    return Ok(TK_CONCAT);
                } else if !is_digit(ls.current) {
                    return Ok(b'.' as i32);
                } else {
                    return read_numeral(state, ls, seminfo);
                }
            }

            Peek::Byte(b'0'..=b'9') => {
                return read_numeral(state, ls, seminfo);
            }

            Peek::Eoz => {
                return Ok(TK_EOS);
            }

            Peek::Byte(c) => {
                if is_lalpha(c as i32) {
                    loop {
                        save_and_next(ls, state)?;
                        if !is_lalnum(ls.current) {
                            break;
                        }
                    }

                    let content = ls.buff.to_owned_text();
                    let ts = new_string(state, ls, &content)?;

                    // Recover the keyword index directly from the interned bytes
                    // via the `LUAX_TOKENS` table: the first `NUM_RESERVED`
                    // entries are the keywords in declaration order, so a match
                    // gives token id = `FIRST_RESERVED + index`.
                    let reserved_token: Option<i32> = LUAX_TOKENS[..NUM_RESERVED]
                        .iter()
                        .position(|kw| *kw == content.as_slice())
                        .map(|i| FIRST_RESERVED + i as i32);
                    *seminfo = TokenValue::Str(ts);

                    if let Some(tk) = reserved_token {
                        // Lua 5.1 has no `goto` keyword — `goto` is an ordinary
                        // identifier (`local goto = 5` is valid). The keyword and
                        // the `::label::` grammar were added in 5.2. So under V51
                        // `goto` lexes as a plain name; the parser then treats
                        // `goto done` as a name beginning an assignment, yielding
                        // the incidental `'=' expected near 'done'` the oracle
                        // reports. See specs/followup/5.1-roster-syntax.md §2.
                        if tk == TK_GOTO
                            && matches!(state.global().lua_version, lua_types::LuaVersion::V51)
                        {
                            return Ok(TK_NAME);
                        }
                        return Ok(tk);
                    }

                    // Lua 5.5: with the upstream-default `LUA_COMPAT_GLOBAL`, the
                    // `global` declaration word is NOT reserved — `global` stays a
                    // valid identifier, and the parser recognizes the declaration
                    // statement contextually (see `globalstat` in lua-parse). So
                    // `global` always lexes as a plain name, on every version.
                    return Ok(TK_NAME);
                } else {
                    let tok = ls.current;
                    advance(ls);
                    return Ok(tok);
                }
            }
        }
    }
}

// ── Cross-crate helpers ────────────────────────────────────────────────────────

fn intern_str_stub(state: &mut LuaState, bytes: &[u8]) -> Result<GcRef<LuaString>, LuaError> {
    state.intern_str(bytes)
}

/// Parse numeral bytes into an integer or float value, or `None` if malformed.
///
/// Wraps `lua_vm::object::str2num`, whose C-derived calling convention is an
/// out-parameter plus a `usize` status (`0` = no parse). The lexer never needs
/// the consumed-byte count — the whole token buffer is the numeral — so this
/// reduces the convention to an idiomatic `Option<LuaValue>`: `Some(value)` on
/// a successful parse, `None` on a malformed numeral.
fn parse_numeral(bytes: &[u8]) -> Option<lua_types::LuaValue> {
    let mut out = lua_types::LuaValue::Nil;
    if lua_vm::object::str2num(bytes, &mut out) == 0 {
        None
    } else {
        Some(out)
    }
}

fn hex_value_stub(c: i32) -> u32 {
    match c {
        c if c >= b'0' as i32 && c <= b'9' as i32 => (c - b'0' as i32) as u32,
        c if c >= b'a' as i32 && c <= b'f' as i32 => (c - b'a' as i32 + 10) as u32,
        c if c >= b'A' as i32 && c <= b'F' as i32 => (c - b'A' as i32 + 10) as u32,
        _ => 0,
    }
}

/// Encode a Unicode codepoint as a Lua-extended UTF-8 byte sequence (1 to 6 bytes).
///
/// Lua permits codepoints up to `0x7FFFFFFF` (5- and 6-byte sequences are
/// non-strict UTF-8 but accepted by `\u{...}` escapes per literals.lua test
/// cases).
fn utf8_encode_stub(codepoint: u32) -> Vec<u8> {
    debug_assert!(codepoint <= 0x7FFF_FFFF);
    if codepoint < 0x80 {
        return vec![codepoint as u8];
    }
    let mut x = codepoint;
    let mut mfb: u32 = 0x3f;
    let mut buf: Vec<u8> = Vec::with_capacity(8);
    loop {
        buf.push(0x80 | ((x & 0x3f) as u8));
        x >>= 6;
        mfb >>= 1;
        if x <= mfb {
            break;
        }
    }
    buf.push(((!mfb << 1) | x) as u8);
    buf.reverse();
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use lua_types::LuaVersion;
    use lua_vm::state::new_state;

    /// A `LexState` whose fields are about to be (re)initialised by
    /// [`set_input`]; only `source`/`envn`/`version` need to be real up front.
    fn empty_lexstate(source: GcRef<LuaString>, ver: LuaVersion) -> LexState {
        LexState {
            current: EOZ,
            linenumber: 1,
            lastline: 1,
            t: Token::eos(),
            lookahead: Token::eos(),
            fs: None,
            z: ZIO::from_bytes(Vec::new()),
            buff: LexBuffer::new(),
            h: None,
            long_str_anchor: std::collections::HashMap::new(),
            dyd: None,
            source: source.clone(),
            envn: source,
            version: ver,
        }
    }

    /// Build a `LexState` over `src` at version `ver` and drive [`next`] until
    /// [`TK_EOS`], collecting each token. The lexer is exercised end-to-end (the
    /// same path the parser takes), so these tests pin scanning behaviour without
    /// the full parser/oracle round-trip.
    fn lex(ver: LuaVersion, src: &[u8]) -> Vec<Token> {
        let mut state = new_state().expect("state init");
        state.global_mut().lua_version = ver;

        let source = state.intern_str(b"@test").expect("intern source");
        let firstchar = src.first().map_or(EOZ, |&b| b as i32);
        let rest: Vec<u8> = src.iter().skip(1).copied().collect();

        let mut ls = empty_lexstate(source.clone(), ver);
        set_input(&mut state, &mut ls, ZIO::from_bytes(rest), source, firstchar)
            .expect("set_input");

        let mut out = Vec::new();
        loop {
            next(&mut state, &mut ls).expect("next");
            let kind = ls.t.kind;
            out.push(ls.t.clone());
            if kind == TK_EOS {
                break;
            }
        }
        out
    }

    /// Just the token-kind sequence, dropping the trailing `TK_EOS`.
    fn kinds(ver: LuaVersion, src: &[u8]) -> Vec<i32> {
        let mut v = lex(ver, src);
        v.pop();
        v.into_iter().map(|t| t.kind).collect()
    }

    /// Lex `src` and return the bytes of its single string-literal token.
    fn one_string(ver: LuaVersion, src: &[u8]) -> Vec<u8> {
        let toks = lex(ver, src);
        let str_tok = toks
            .iter()
            .find(|t| t.kind == TK_STRING)
            .expect("a string token");
        match &str_tok.value {
            TokenValue::Str(s) => s.as_bytes().to_vec(),
            _ => panic!("string token without Str payload"),
        }
    }

    #[test]
    fn crlf_and_lfcr_pair_into_single_lines() {
        // Three logical lines joined by CRLF; the `+` lands on line 3, so a
        // lex error there must report line 3 (CRLF counts as ONE newline).
        let toks = lex(LuaVersion::V54, b"a\r\nb\r\n+");
        // a(name) b(name) +  EOS  -> the '+' is a single-byte token on line 3.
        assert_eq!(toks[0].kind, TK_NAME);
        assert_eq!(toks[1].kind, TK_NAME);
        assert_eq!(toks[2].kind, b'+' as i32);
        // LFCR also pairs.
        let toks2 = lex(LuaVersion::V54, b"a\n\rb");
        assert_eq!(toks2[0].kind, TK_NAME);
        assert_eq!(toks2[1].kind, TK_NAME);
        assert_eq!(toks2[2].kind, TK_EOS);
    }

    #[test]
    fn crlf_line_number_attribution() {
        // The error message must carry line 3 for a malformed numeral there.
        let mut state = new_state().expect("state init");
        state.global_mut().lua_version = LuaVersion::V54;
        let source = state.intern_str(b"@t").unwrap();
        let src = b"\r\n\r\n0x".to_vec();
        let firstchar = src[0] as i32;
        let rest: Vec<u8> = src.iter().skip(1).copied().collect();
        let mut ls = empty_lexstate(source.clone(), LuaVersion::V54);
        set_input(&mut state, &mut ls, ZIO::from_bytes(rest), source, firstchar).unwrap();
        // `0x` with no hex digits is a malformed numeral on line 3.
        let err = loop {
            match next(&mut state, &mut ls) {
                Ok(()) if ls.t.kind == TK_EOS => panic!("expected a lex error"),
                Ok(()) => continue,
                Err(e) => break e,
            }
        };
        let msg = err.message_lossy();
        assert!(msg.contains(":3:"), "error should be on line 3, got: {msg}");
        assert!(msg.contains("malformed number"), "got: {msg}");
    }

    #[test]
    fn hex_escape_decodes_to_bytes() {
        assert_eq!(one_string(LuaVersion::V54, br#""\x41\x42""#), b"AB");
        assert_eq!(one_string(LuaVersion::V54, br#""\x00\xff""#), b"\x00\xff");
    }

    #[test]
    fn utf8_escape_encodes_codepoint() {
        assert_eq!(one_string(LuaVersion::V54, br#""\u{48}\u{49}""#), b"HI");
        // U+00E9 (é) → two UTF-8 bytes 0xC3 0xA9.
        assert_eq!(one_string(LuaVersion::V54, br#""\u{e9}""#), b"\xc3\xa9");
        // A 6-byte (non-strict) sequence is accepted up to 0x7FFFFFFF in 5.4.
        assert_eq!(
            one_string(LuaVersion::V54, br#""\u{7FFFFFFF}""#),
            b"\xFD\xBF\xBF\xBF\xBF\xBF"
        );
    }

    #[test]
    fn decimal_escape_and_z_skip() {
        assert_eq!(one_string(LuaVersion::V54, br#""\65\66""#), b"AB");
        // \z swallows the following whitespace run (including newlines).
        assert_eq!(one_string(LuaVersion::V54, b"\"a\\z   \n\t b\""), b"ab");
    }

    #[test]
    fn long_brackets_with_levels() {
        assert_eq!(one_string(LuaVersion::V54, b"[[ hello ]]"), b" hello ");
        assert_eq!(one_string(LuaVersion::V54, b"[==[ hi ]==]"), b" hi ");
        // A leading newline immediately after the open bracket is dropped.
        assert_eq!(one_string(LuaVersion::V54, b"[[\nx]]"), b"x");
        // An inner `]=]` that is not the matching close is kept as content.
        assert_eq!(one_string(LuaVersion::V54, b"[==[a]=]b]==]"), b"a]=]b");
    }

    #[test]
    fn long_string_normalizes_newlines() {
        // CRLF inside a long string is normalized to a single \n.
        assert_eq!(one_string(LuaVersion::V54, b"[[a\r\nb]]"), b"a\nb");
    }

    #[test]
    fn numeral_int_float_boundary() {
        // Integer literals → TK_INT with the exact i64; floats → TK_FLT.
        let dec = lex(LuaVersion::V54, b"3");
        assert_eq!(dec[0].kind, TK_INT);
        assert!(matches!(dec[0].value, TokenValue::Int(3)));

        let hex = lex(LuaVersion::V54, b"0x10");
        assert_eq!(hex[0].kind, TK_INT);
        assert!(matches!(hex[0].value, TokenValue::Int(16)));

        let flt = lex(LuaVersion::V54, b"3.0");
        assert_eq!(flt[0].kind, TK_FLT);
        assert!(matches!(flt[0].value, TokenValue::Float(f) if f == 3.0));

        let expo = lex(LuaVersion::V54, b"1e2");
        assert_eq!(expo[0].kind, TK_FLT);
        assert!(matches!(expo[0].value, TokenValue::Float(f) if f == 100.0));

        // A bare dot followed by digits is a float; a lone dot is the '.' token.
        let dotnum = lex(LuaVersion::V54, b".5");
        assert_eq!(dotnum[0].kind, TK_FLT);
    }

    #[test]
    fn float_only_versions_widen_integer_literals() {
        // 5.1/5.2 have no integer subtype: every numeral lexes as a float.
        let v51 = lex(LuaVersion::V51, b"3");
        assert_eq!(v51[0].kind, TK_FLT);
        assert!(matches!(v51[0].value, TokenValue::Float(f) if f == 3.0));

        let v52 = lex(LuaVersion::V52, b"42");
        assert_eq!(v52[0].kind, TK_FLT);
    }

    #[test]
    fn version_gated_operators() {
        // `<<` is a 5.3+ token; on 5.4 it is one TK_SHL.
        assert_eq!(kinds(LuaVersion::V54, b"1 << 2"), vec![TK_INT, TK_SHL, TK_INT]);
        // On the float-only family it is NOT a token: two bare '<' bytes.
        assert_eq!(
            kinds(LuaVersion::V51, b"1 << 2"),
            vec![TK_FLT, b'<' as i32, b'<' as i32, TK_FLT]
        );
        // `//` floor-division: TK_IDIV on 5.4, two '/' on 5.1.
        assert_eq!(kinds(LuaVersion::V54, b"7 // 2"), vec![TK_INT, TK_IDIV, TK_INT]);
        assert_eq!(
            kinds(LuaVersion::V51, b"7 // 2"),
            vec![TK_FLT, b'/' as i32, b'/' as i32, TK_FLT]
        );
        // `::` label delimiter: TK_DBCOLON on 5.4, two ':' on 5.1.
        assert_eq!(kinds(LuaVersion::V54, b"::x::"), vec![TK_DBCOLON, TK_NAME, TK_DBCOLON]);
        assert_eq!(
            kinds(LuaVersion::V51, b"::x"),
            vec![b':' as i32, b':' as i32, TK_NAME]
        );
    }

    #[test]
    fn version_gated_reserved_words() {
        // `goto` is a keyword on 5.2+ but a plain name on 5.1.
        assert_eq!(kinds(LuaVersion::V54, b"goto"), vec![TK_GOTO]);
        assert_eq!(kinds(LuaVersion::V51, b"goto"), vec![TK_NAME]);
        // `global` is never reserved — always a name.
        assert_eq!(kinds(LuaVersion::V55, b"global"), vec![TK_NAME]);
        assert_eq!(kinds(LuaVersion::V54, b"global"), vec![TK_NAME]);
        // An ordinary keyword still maps to its TK_* on every version.
        assert_eq!(kinds(LuaVersion::V54, b"while"), vec![TK_WHILE]);
        assert_eq!(kinds(LuaVersion::V51, b"while"), vec![TK_WHILE]);
    }

    #[test]
    fn multichar_symbols_and_dots() {
        assert_eq!(kinds(LuaVersion::V54, b"=="), vec![TK_EQ]);
        assert_eq!(kinds(LuaVersion::V54, b"~="), vec![TK_NE]);
        assert_eq!(kinds(LuaVersion::V54, b"<="), vec![TK_LE]);
        assert_eq!(kinds(LuaVersion::V54, b">="), vec![TK_GE]);
        assert_eq!(kinds(LuaVersion::V54, b".."), vec![TK_CONCAT]);
        assert_eq!(kinds(LuaVersion::V54, b"..."), vec![TK_DOTS]);
        // A lone dot is its single-byte token.
        assert_eq!(kinds(LuaVersion::V54, b"."), vec![b'.' as i32]);
    }

    #[test]
    fn comments_emit_no_tokens() {
        // Short comment to end of line, then a name.
        assert_eq!(kinds(LuaVersion::V54, b"-- a comment\nx"), vec![TK_NAME]);
        // Long comment.
        assert_eq!(kinds(LuaVersion::V54, b"--[[ block\ncomment ]] y"), vec![TK_NAME]);
    }

    #[test]
    fn empty_input_is_just_eos() {
        let toks = lex(LuaVersion::V54, b"");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TK_EOS);
    }

    #[test]
    fn peek_maps_byte_and_eoz() {
        // The internal dispatch shape: a byte maps to Peek::Byte, EOZ to Peek::Eoz.
        let mut state = new_state().unwrap();
        let source = state.intern_str(b"@t").unwrap();
        let mut ls = empty_lexstate(source, LuaVersion::V54);
        ls.current = b'=' as i32;
        assert!(matches!(peek(&ls), Peek::Byte(b'=')));
        ls.current = EOZ;
        assert!(matches!(peek(&ls), Peek::Eoz));
    }

    #[test]
    fn zio_yields_bytes_then_eoz() {
        let mut z = ZIO::from_bytes(b"ab".to_vec());
        assert_eq!(z.getc(), b'a' as i32);
        assert_eq!(z.getc(), b'b' as i32);
        assert_eq!(z.getc(), EOZ);
        assert_eq!(z.getc(), EOZ);
        // An empty source is immediately EOZ.
        let mut empty = ZIO::from_bytes(Vec::new());
        assert_eq!(empty.getc(), EOZ);
    }

    #[test]
    fn lexbuffer_extraction_helpers() {
        let mut b = LexBuffer::new();
        for &c in b"\"hello\"" {
            b.push_byte(c);
        }
        assert_eq!(b.trim_ends(1), b"hello");
        assert_eq!(b.to_owned_text(), b"\"hello\"");
        // Too-short buffer for the requested delimiter width → empty content.
        let mut tiny = LexBuffer::new();
        tiny.push_byte(b'"');
        assert_eq!(tiny.trim_ends(1), b"");
        // Trailing-NUL trim drops exactly one sentinel.
        let mut num = LexBuffer::new();
        for &c in b"123\0" {
            num.push_byte(c);
        }
        assert_eq!(num.without_trailing_nul(), b"123");
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        originally ported from src/llex.c + src/llex.h; the C
//                  correspondence has since been GRADUATED away (see GRADUATED.md
//                  and the module-level doc). This file no longer tracks the C
//                  line-by-line — it is idiomatic Rust guarded by the oracle.
//   target_crate:  lua-lex
//   confidence:    high
//   todos:         1  (ZIO/LexBuffer move to lua_vm::zio — separate refactor)
//   port_notes:    0  (C-correspondence crutches removed in idiomatization S1)
//   unsafe_blocks: 0   (must be 0 outside explicit unsafe-budget crates)
//   notes:         Idiomatized (Sprint 1, P1a): byte cursor is a single-cursor
//                  chunk reader; numeral parsing returns Option not a C status +
//                  out-param; token text uses named LexBuffer extraction methods;
//                  the scan loop dispatches on an internal Peek enum while the
//                  emitted token kind stays i32 (TK_*) as the lua-parse boundary.
//                  The oracle that now guards behaviour: bytecode parity (token
//                  stream → identical luac -l -l), the multiversion_oracle, and
//                  the literals.lua / errors.lua line-and-message tests. Numeric
//                  literal parsing delegates to lua_vm::object::str2num. Do NOT
//                  reach for llex.c to debug this file — see GRADUATED.md.
// ──────────────────────────────────────────────────────────────────────────────
