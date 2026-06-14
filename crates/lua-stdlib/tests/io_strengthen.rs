//! Behavioral net for the deterministic, reference-pinnable surface of the
//! `io` library: read-format parsing/validation, the `*`-prefix version seam,
//! `read("n")` number parsing, and the closed-file error shape.
//!
//! `io` is IMPURE — its real work reaches the host filesystem through
//! `GlobalState::file_open_hook`, and the **default** `omnilua` embedding
//! (`Lua::new_versioned`) installs NO such hook, so `io.open` there returns a
//! clean `(nil, msg, errno)` failure rather than touching a disk. That makes
//! the host-specific I/O *results* (the bytes a particular OS returns) not
//! reproducible and out of scope.
//!
//! What IS deterministic — and what this file pins — is the format/validation
//! and error-shaping layer that runs *before* and *around* the host I/O:
//!   * which read formats are accepted, and the exact `bad argument` wording
//!     when they are not (`invalid option` vs `invalid format`);
//!   * the `*`-prefix version seam: 5.1/5.2 REQUIRE `*` (bare `l`/`n`/`a` is
//!     rejected) and have no `L` format; 5.3+ make `*` optional;
//!   * `read("n")` parsing of a *known* temp-file's content (integers, floats,
//!     hex floats) — the bytes are ones the test wrote, so the result is the
//!     same on every host;
//!   * `attempt to use a closed file`, including its source-location prefix.
//!
//! To exercise the read path deterministically these tests install their OWN
//! minimal `file_open_hook` over an in-test `/tmp` scratch file (unique name
//! per the repo's parallel-safety rule: pid + a per-process counter). This is
//! the harness "install the capability you need to make the net deterministic"
//! pattern — the embedding stays sandboxed by default; the test opts in.
//!
//! Every expected string here was captured from the version-suffixed reference
//! binaries (`/tmp/lua-refs/bin/lua5.x`), never from our own output, so the
//! assertions pin the REFERENCE, not the impl (non-tautological).
//!
//! `omnilua` is a dev-dependency here (it depends on `lua-stdlib`, so it can
//! only be a dev-dep — see `Cargo.toml`).

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use omnilua::{HostHooks, Lua, LuaError, LuaFileHandle, LuaVersion};

// ── A minimal real-filesystem file handle for the test embedding ─────────────

/// A scratch file backed by a real `std::fs::File`, just enough of
/// [`LuaFileHandle`] to drive `read`/`seek`/`close` over a temp file the test
/// itself created. Mirrors `lua-cli`'s `FsFile` read/readwrite arms but trimmed
/// to what the format-parsing net needs.
struct TestFsFile {
    inner: std::fs::File,
    pushback: Option<u8>,
    err: Option<(i32, String)>,
}

impl LuaFileHandle for TestFsFile {
    fn read_byte(&mut self) -> i32 {
        if let Some(b) = self.pushback.take() {
            return b as i32;
        }
        let mut buf = [0u8; 1];
        match self.inner.read(&mut buf) {
            Ok(1) => buf[0] as i32,
            Ok(_) => -1,
            Err(e) => {
                self.err = Some((e.raw_os_error().unwrap_or(0), e.to_string()));
                -1
            }
        }
    }

    fn unread_byte(&mut self, byte: i32) {
        if byte >= 0 {
            self.pushback = Some(byte as u8);
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        self.inner.write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pushback = None;
        self.inner.seek(pos)
    }

    fn tell(&mut self) -> io::Result<u64> {
        self.inner.seek(SeekFrom::Current(0))
    }

    fn clear_error(&mut self) {
        self.err = None;
    }

    fn has_error(&self) -> bool {
        self.err.is_some()
    }

    fn last_error_info(&self) -> Option<(i32, String)> {
        self.err.clone()
    }
}

thread_local! {
    /// The single scratch path the in-test hook reads from for the current
    /// test. `file_open_hook` is a bare `fn` pointer (not a closure), so it
    /// cannot capture the per-test path; this thread-local carries it. Each
    /// test runs on its own thread under the cargo test harness and sets this
    /// before constructing `Lua`, so there is no cross-test interference.
    static SCRATCH_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

fn test_file_open_hook(
    _filename: &[u8],
    mode: &[u8],
) -> Result<Box<dyn LuaFileHandle>, LuaError> {
    let path = SCRATCH_PATH
        .with(|p| p.borrow().clone())
        .expect("scratch path must be set before io.open in a test");
    let read = mode.first() == Some(&b'r');
    let file = if read {
        std::fs::File::open(&path)
    } else {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
    }
    .map_err(|e| LuaError::runtime(format_args!("open failed: {e}")))?;
    Ok(Box::new(TestFsFile {
        inner: file,
        pushback: None,
        err: None,
    }))
}

/// Process-unique scratch-path counter (combined with the pid) so parallel test
/// processes and parallel tests within one process never collide.
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a uniquely-named `/tmp` scratch file with `contents`, register it as
/// the active scratch path for the in-test file hook, and return the path so
/// the test can clean it up.
fn make_scratch(contents: &[u8]) -> std::path::PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!("omnilua_io_strengthen_{}_{}", std::process::id(), n));
    std::fs::write(&path, contents).expect("write scratch file");
    SCRATCH_PATH.with(|p| *p.borrow_mut() = Some(path.clone()));
    path
}

/// Build a `Lua` instance for `version` with the in-test file hook installed.
fn lua_with_fs(version: LuaVersion) -> Lua {
    let hooks = HostHooks::new().file_open(test_file_open_hook);
    Lua::with_hooks_versioned(hooks, version).expect("init lua with fs hook")
}

/// Run `code` under `version` (with fs hook + the named scratch file) and
/// return the single string it evaluates to. The Lua program is responsible
/// for turning whatever it observes (a read result, a `pcall` error message)
/// into a string, so the assertion pins an exact reference string.
fn eval_str(version: LuaVersion, scratch: &[u8], code: &str) -> String {
    let _path = make_scratch(scratch);
    let lua = lua_with_fs(version);
    let result: String = lua
        .load(code)
        .set_name(b"=io_test")
        .eval()
        .unwrap_or_else(|e| panic!("eval of `{code}` failed under {version:?}: {e:?}"));
    SCRATCH_PATH.with(|p| {
        if let Some(path) = p.borrow_mut().take() {
            let _ = std::fs::remove_file(path);
        }
    });
    result
}

const ALL: [LuaVersion; 5] = [
    LuaVersion::V51,
    LuaVersion::V52,
    LuaVersion::V53,
    LuaVersion::V54,
    LuaVersion::V55,
];

// ── Net 1: bad-format error wording is version-gated ─────────────────────────

/// An unrecognised read option is `invalid option` on 5.1/5.2 (no `*` prefix
/// present, so the leading char is rejected before the option switch) and
/// `invalid format` on 5.3+. Captured from the reference binaries:
///   5.1.5/5.2.4: `bad argument #1 to 'read' (invalid option)`
///   5.3.6/5.4.7/5.5.0: `bad argument #1 to 'read' (invalid format)`
#[test]
fn read_unknown_bare_format_wording_crossversion() {
    let code = "local f = io.open('x','r'); \
                local ok, err = pcall(function() return f:read('z') end); \
                f:close(); \
                return tostring(err)";
    for v in ALL {
        let got = eval_str(v, b"data\n", code);
        let expected_tail = match v {
            LuaVersion::V51 | LuaVersion::V52 => "(invalid option)",
            _ => "(invalid format)",
        };
        assert!(
            got.contains("bad argument #1 to 'read'") && got.ends_with(expected_tail),
            "{v:?}: read('z') error was `{got}`, expected to end with \
             `bad argument #1 to 'read' {expected_tail}`"
        );
    }
}

/// With a `*` prefix the leading char IS accepted, so an unknown option after
/// it is `invalid format` on EVERY version (the option switch's default).
/// Reference: all five binaries give `bad argument #1 to 'read' (invalid format)`
/// for `read('*z')`.
#[test]
fn read_unknown_starred_format_is_invalid_format_everywhere() {
    let code = "local f = io.open('x','r'); \
                local ok, err = pcall(function() return f:read('*z') end); \
                f:close(); \
                return tostring(err)";
    for v in ALL {
        let got = eval_str(v, b"data\n", code);
        assert!(
            got.ends_with("(invalid format)"),
            "{v:?}: read('*z') error was `{got}`, expected to end with `(invalid format)`"
        );
    }
}

// ── Net 2: the `*`-prefix version seam ───────────────────────────────────────

/// 5.1/5.2 REQUIRE the `*` prefix on read formats; a bare `l`/`n`/`a` is
/// rejected with `(invalid option)`. 5.3 made `*` optional, so bare `l` reads a
/// line. Reference-pinned: on 5.1/5.2 `read('l')` errors; on 5.3+ it returns
/// the first line. This pins the seam in BOTH directions (a contrast pair).
#[test]
fn bare_format_requires_star_on_51_52_crossversion() {
    for v in ALL {
        // The result string is either the line read, or the error message.
        let code = "local f = io.open('x','r'); \
                    local ok, res = pcall(function() return f:read('l') end); \
                    f:close(); \
                    if ok then return 'OK:' .. tostring(res) \
                    else return 'ERR:' .. tostring(res) end";
        let got = eval_str(v, b"firstline\nsecond\n", code);
        match v {
            LuaVersion::V51 | LuaVersion::V52 => assert!(
                got.starts_with("ERR:") && got.ends_with("(invalid option)"),
                "{v:?}: bare read('l') should error with (invalid option), got `{got}`"
            ),
            _ => assert_eq!(
                got, "OK:firstline",
                "{v:?}: bare read('l') should read the first line"
            ),
        }
    }
}

/// The `*l` form is accepted on EVERY version (the contrast to the bare form):
/// it is required on 5.1/5.2 and still accepted-though-deprecated on 5.3+.
/// Reference: all five binaries return the first line for `read('*l')`.
#[test]
fn starred_line_format_accepted_everywhere() {
    let code = "local f = io.open('x','r'); \
                local line = f:read('*l'); \
                f:close(); \
                return tostring(line)";
    for v in ALL {
        let got = eval_str(v, b"firstline\nsecond\n", code);
        assert_eq!(got, "firstline", "{v:?}: read('*l') should read the first line");
    }
}

/// The `L` (line-with-EOL) format did not exist in 5.1 — `*L` is rejected as
/// `(invalid format)` there — and was added in 5.2. Reference-pinned: 5.1
/// `read('*L')` errors `bad argument #1 to 'read' (invalid format)`; 5.2+
/// returns the line *including* its trailing `\n`.
#[test]
fn line_with_eol_format_is_52_plus_crossversion() {
    for v in ALL {
        let code = "local f = io.open('x','r'); \
                    local ok, res = pcall(function() return f:read('*L') end); \
                    f:close(); \
                    if ok then return 'OK:' .. tostring(res) \
                    else return 'ERR:' .. tostring(res) end";
        let got = eval_str(v, b"firstline\nsecond\n", code);
        match v {
            LuaVersion::V51 => assert!(
                got.starts_with("ERR:") && got.ends_with("(invalid format)"),
                "5.1: read('*L') should error (invalid format) — no L format, got `{got}`"
            ),
            _ => assert_eq!(
                got, "OK:firstline\n",
                "{v:?}: read('*L') should return the line including its newline"
            ),
        }
    }
}

// ── Net 3: read("n") number parsing of known content ─────────────────────────

/// `read("n")` parses a numeral from the file. The bytes are ones the test
/// wrote, so the parse result is host-independent. On 5.1/5.2 the `*` is
/// required (`*n`); on 5.3+ bare `n` works. Reference: `42` then `3.14` then the
/// hex float `0x1Ap2` → `104.0` (5.3+ has integer/float subtypes; the
/// `tostring` shape `104.0` is what 5.3+ prints).
#[test]
fn read_number_parses_known_content_53_plus() {
    let code = "local f = io.open('x','r'); \
                local a, b, c = f:read('n', 'n', 'n'); \
                f:close(); \
                return tostring(a) .. '|' .. tostring(b) .. '|' .. tostring(c)";
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        let got = eval_str(v, b"42 3.14 0x1Ap2\n", code);
        assert_eq!(
            got, "42|3.14|104.0",
            "{v:?}: read('n','n','n') over `42 3.14 0x1Ap2` should parse to 42, 3.14, 104.0"
        );
    }
}

/// On 5.1/5.2 the same parse uses the `*n` form (bare `n` is rejected, pinned
/// above). Those float-only versions print whole numerals without a `.0`
/// suffix, so `42` reads as `42`. Reference-pinned `*n` on 5.1/5.2.
#[test]
fn read_number_star_form_on_51_52() {
    let code = "local f = io.open('x','r'); \
                local a = f:read('*n'); \
                f:close(); \
                return tostring(a)";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        let got = eval_str(v, b"42 3.14\n", code);
        assert_eq!(got, "42", "{v:?}: read('*n') over `42 ...` should parse to 42");
    }
}

// ── Net 4: the closed-file error (shape + location prefix) ───────────────────

/// Reading from a closed handle raises `attempt to use a closed file`, and —
/// like the reference's `luaL_error` — the message carries the calling
/// source-location prefix (`<chunkname>:<line>:`). Reference-pinned across all
/// versions: `<src>:<line>: attempt to use a closed file`. The chunk is named
/// `=io_test`, which renders as `io_test` in the prefix.
#[test]
fn read_on_closed_file_errors_with_location_crossversion() {
    let code = "local f = io.open('x','r'); \
                f:close(); \
                local ok, err = pcall(function() return f:read('*l') end); \
                return tostring(err)";
    for v in ALL {
        let got = eval_str(v, b"data\n", code);
        assert!(
            got.ends_with("attempt to use a closed file"),
            "{v:?}: closed-file error was `{got}`"
        );
        assert!(
            got.contains("io_test:") && got != "attempt to use a closed file",
            "{v:?}: closed-file error should carry a source-location prefix \
             (like reference luaL_error), got `{got}`"
        );
    }
}

/// `io.type` reports `"closed file"` for a handle after `close`, `"file"` while
/// open, and the `fail` value (`nil`) for a non-handle — the C source pushes
/// `luaL_pushfail` (== `nil` on every version), NOT `false`. This is the
/// dispatch/type-shaping surface and is host-independent. Reference-pinned
/// across all versions: `file|closed file|nil`.
#[test]
fn io_type_reports_open_closed_and_non_handle_crossversion() {
    let code = "local f = io.open('x','r'); \
                local open = io.type(f); \
                f:close(); \
                local closed = io.type(f); \
                local notfile = io.type('hello'); \
                return tostring(open) .. '|' .. tostring(closed) .. '|' .. tostring(notfile)";
    for v in ALL {
        let got = eval_str(v, b"data\n", code);
        assert_eq!(
            got, "file|closed file|nil",
            "{v:?}: io.type should report file / closed file / nil (fail, not false)"
        );
    }
}
