//! CLI-level oracle test for issue #79(d): an uncaught top-level error must
//! print a traceback whose deepest frame is the base C frame `\t[C]: in ?`,
//! matching the reference standalone interpreter.
//!
//! This is a SPAWN-THE-BINARY test on purpose. The `[C]: in ?` frame only
//! exists in the CLI traceback path — the standalone `pmain` C closure that the
//! CLI runs the whole program beneath. The in-process `load`+`pcall` wrapper in
//! `crates/lua-rs-runtime/tests/multiversion_oracle.rs` has no `pmain`, so it
//! never sees the frame and is unaffected by this fix.
//!
//! For each of 5.3 / 5.4 / 5.5 and each entry point (file, `-e`, piped stdin)
//! we assert the normalized stderr ends with `\t[C]: in ?` and that the line
//! directly above it is `... in main chunk`. When a matching reference binary
//! is present under `/tmp/lua-refs/bin/`, we additionally diff our normalized
//! stderr against the reference's.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const VERSIONS: &[&str] = &["5.3", "5.4", "5.5"];

/// A nested-local-fn script that raises an uncaught `error` at a known line.
const SCRIPT: &str = "local function inner()\n  error(\"boom\")\nend\ninner()\n";

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Unique temp path for a spawned process, including pid + counter so parallel
/// test threads never collide (per the harness temp-file rule).
fn temp_script() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lua_rs_tb_oracle_{}_{}.lua",
        std::process::id(),
        n
    ));
    std::fs::write(&p, SCRIPT).expect("write temp script");
    p
}

/// Normalize stderr the way `specs/oracle/diff_one.sh` does: collapse the
/// absolute script path to a stable token and scrub `0x…` addresses, so the
/// comparison is path- and address-independent.
fn normalize(stderr: &[u8], script_path: &str) -> String {
    let mut s = String::from_utf8_lossy(stderr).into_owned();
    if !script_path.is_empty() {
        s = s.replace(script_path, "<script>");
    }
    // Scrub hex addresses (e.g. `function: 0x55…`) to a stable token.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'0' && i + 1 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
        {
            out.push_str("0xADDR");
            i += 2;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Last non-empty line of `s`, and the line directly above it.
fn last_two_nonempty_lines(s: &str) -> (Option<&str>, Option<&str>) {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    let last = lines.last().copied();
    let above = if lines.len() >= 2 {
        Some(lines[lines.len() - 2])
    } else {
        None
    };
    (above, last)
}

fn lua_rs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lua-rs"))
}

fn reference_binary(version: &str) -> Option<PathBuf> {
    let p = PathBuf::from(format!("/tmp/lua-refs/bin/lua{}", full_version(version)));
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Map a short version (`5.4`) to the patch-level binary name (`5.4.7`).
fn full_version(version: &str) -> &'static str {
    match version {
        "5.3" => "5.3.6",
        "5.4" => "5.4.7",
        "5.5" => "5.5.0",
        other => panic!("unhandled version {other}"),
    }
}

/// Assert the traceback tail: deepest frame `\t[C]: in ?`, with the frame above
/// it ending in `in main chunk`.
fn assert_traceback_tail(normalized: &str, ctx: &str) {
    assert!(
        normalized.contains("stack traceback:"),
        "[{ctx}] missing 'stack traceback:' in stderr:\n{normalized}"
    );
    let (above, last) = last_two_nonempty_lines(normalized);
    assert_eq!(
        last,
        Some("\t[C]: in ?"),
        "[{ctx}] deepest traceback frame must be `\\t[C]: in ?`, got {last:?}\n{normalized}"
    );
    assert!(
        above.is_some_and(|l| l.ends_with("in main chunk")),
        "[{ctx}] frame above `[C]: in ?` must be the main chunk, got {above:?}\n{normalized}"
    );
}

#[test]
fn file_entry_point_has_base_c_frame() {
    for &v in VERSIONS {
        let script = temp_script();
        let script_str = script.to_string_lossy().into_owned();
        let out = lua_rs()
            .env("LUA_RS_VERSION", v)
            .arg(&script)
            .output()
            .expect("spawn lua-rs");
        let norm = normalize(&out.stderr, &script_str);
        assert_traceback_tail(&norm, &format!("file/{v}"));
        assert_eq!(
            out.status.code(),
            Some(1),
            "[file/{v}] uncaught file error must exit 1"
        );

        if let Some(refbin) = reference_binary(v) {
            let rout = Command::new(&refbin)
                .arg(&script)
                .output()
                .expect("spawn reference");
            let refnorm = normalize(&rout.stderr, &script_str)
                // Reference prefixes l_message with its own binary path; ours
                // uses the script-relative progname. Compare only the traceback
                // body (from `stack traceback:` onward), which is what #79d is
                // about.
                ;
            let our_tb = norm.split_once("stack traceback:").map(|x| x.1);
            let ref_tb = refnorm.split_once("stack traceback:").map(|x| x.1);
            // The 5.5 namewhat divergence (`in global 'error'` vs
            // `in function 'error'`) is now fixed in `push_func_name`, so the
            // traceback body matches the reference for every version.
            assert_eq!(
                our_tb, ref_tb,
                "[file/{v}] traceback body must match reference"
            );
        }

        let _ = std::fs::remove_file(&script);
    }
}

#[test]
fn dash_e_entry_point_has_base_c_frame() {
    for &v in VERSIONS {
        let out = lua_rs()
            .env("LUA_RS_VERSION", v)
            .arg("-e")
            .arg("error(\"boom\")")
            .output()
            .expect("spawn lua-rs -e");
        let norm = normalize(&out.stderr, "");
        assert_traceback_tail(&norm, &format!("-e/{v}"));
        assert_eq!(
            out.status.code(),
            Some(1),
            "[-e/{v}] uncaught -e error must exit 1"
        );
    }
}

#[test]
fn stdin_entry_point_has_base_c_frame() {
    for &v in VERSIONS {
        let mut child = lua_rs()
            .env("LUA_RS_VERSION", v)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn lua-rs (stdin)");
        child
            .stdin
            .take()
            .expect("stdin handle")
            .write_all(b"error(\"boom\")\n")
            .expect("write stdin");
        let out = child.wait_with_output().expect("wait lua-rs (stdin)");
        let norm = normalize(&out.stderr, "");
        assert_traceback_tail(&norm, &format!("stdin/{v}"));
        // Piped-stdin uncaught error exits 0 in the reference (no script>0 path,
        // dofile(stdin) does not set the failure flag) — preserved here.
        assert_eq!(
            out.status.code(),
            Some(0),
            "[stdin/{v}] piped-stdin error must exit 0 (matches reference)"
        );
    }
}

/// Lua 5.5 `global '<name>' already defined` guard (Div.1 of
/// `specs/followup/5.5-lang.md`). The error is reported at the line of the
/// offending initializer, exits 1, and prints a normal runtime traceback. This
/// asserts the full stderr (message + traceback body) matches lua5.5.0, and
/// that the reported line is the second `global x = ...` (line 3 here).
/// Lua 5.5 reorders `pushfuncname` (lauxlib.c) to prefer `namewhat` over the
/// global-name lookup, so a function reachable as a global renders
/// `in global '<name>'` rather than 5.3/5.4's `in function '<name>'`. This
/// covers both a global C function (`error`) and a global Lua function (`G`),
/// and asserts 5.3/5.4 keep the `in function` rendering (no regression).
#[test]
fn namewhat_global_rendering_is_version_gated() {
    // A global Lua fn G that calls the global C fn error — exercises both
    // frames. `-e` so there is no script path to normalize.
    const PROG: &str = "function G() error(\"x\") end G()";
    for &v in VERSIONS {
        let out = lua_rs()
            .env("LUA_RS_VERSION", v)
            .arg("-e")
            .arg(PROG)
            .output()
            .expect("spawn lua-rs -e");
        let norm = normalize(&out.stderr, "");
        if v == "5.5" {
            assert!(
                norm.contains("[C]: in global 'error'"),
                "[namewhat/5.5] expected `[C]: in global 'error'`:\n{norm}"
            );
            assert!(
                norm.contains("in global 'G'"),
                "[namewhat/5.5] expected `in global 'G'`:\n{norm}"
            );
            assert!(
                !norm.contains("in function 'error'") && !norm.contains("in function 'G'"),
                "[namewhat/5.5] must not use the 5.4 `in function` rendering:\n{norm}"
            );
        } else {
            assert!(
                norm.contains("[C]: in function 'error'"),
                "[namewhat/{v}] expected `[C]: in function 'error'`:\n{norm}"
            );
            assert!(
                norm.contains("in function 'G'"),
                "[namewhat/{v}] expected `in function 'G'`:\n{norm}"
            );
        }

        // When the reference binary is present, the whole traceback body must
        // match (this is the differential oracle, baked into the CLI test).
        if let Some(refbin) = reference_binary(v) {
            let rout = Command::new(&refbin)
                .arg("-e")
                .arg(PROG)
                .output()
                .expect("spawn reference");
            let refnorm = normalize(&rout.stderr, "");
            let our_tb = norm.split_once("stack traceback:").map(|x| x.1);
            let ref_tb = refnorm.split_once("stack traceback:").map(|x| x.1);
            assert_eq!(
                our_tb, ref_tb,
                "[namewhat/{v}] traceback body must match reference"
            );
        }
    }
}

/// Lua 5.5's `luaG_errormsg` converts a nil error object to
/// `"<no error object>"`, but the standalone CLI message handler (`lua.c`)
/// still renders the top-level message as `(error object is a nil value)`
/// for an uncaught `error(nil)`. Pin that the message-handler path is
/// unchanged by the nil-conversion fix, for every version.
#[test]
fn top_level_error_nil_message_unchanged() {
    for &v in VERSIONS {
        let out = lua_rs()
            .env("LUA_RS_VERSION", v)
            .arg("-e")
            .arg("error(nil)")
            .output()
            .expect("spawn lua-rs -e");
        let norm = normalize(&out.stderr, "");
        assert!(
            norm.contains("(error object is a nil value)"),
            "[error-nil/{v}] top-level error(nil) must print `(error object is a nil value)`:\n{norm}"
        );
        assert!(
            !norm.contains("<no error object>"),
            "[error-nil/{v}] CLI message handler path must not leak `<no error object>`:\n{norm}"
        );
        assert_traceback_tail(&norm, &format!("error-nil/{v}"));
    }
}

#[test]
fn v55_global_already_defined_traceback_matches_reference() {
    const GUARD_SCRIPT: &str = "global print\nglobal x = 1\nglobal x = 2\nprint(x)\n";

    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut script = std::env::temp_dir();
    script.push(format!(
        "lua_rs_global_guard_{}_{}.lua",
        std::process::id(),
        n
    ));
    std::fs::write(&script, GUARD_SCRIPT).expect("write guard script");
    let script_str = script.to_string_lossy().into_owned();

    let out = lua_rs()
        .env("LUA_RS_VERSION", "5.5")
        .arg(&script)
        .output()
        .expect("spawn lua-rs");
    let norm = normalize(&out.stderr, &script_str);

    assert_eq!(
        out.status.code(),
        Some(1),
        "global-guard error must exit 1\n{norm}"
    );
    assert!(
        norm.contains("<script>:3: global 'x' already defined"),
        "guard message must be at line 3 with the right text:\n{norm}"
    );
    assert_traceback_tail(&norm, "global-guard/5.5");

    if let Some(refbin) = reference_binary("5.5") {
        let rout = Command::new(&refbin)
            .arg(&script)
            .output()
            .expect("spawn reference 5.5");
        let refnorm = normalize(&rout.stderr, &script_str);
        // Compare from `stack traceback:` onward (the binary-path prefix on the
        // l_message line differs between our progname and the reference's, the
        // same convention used by the #79d tests above).
        let our_tb = norm.split_once("stack traceback:").map(|x| x.1);
        let ref_tb = refnorm.split_once("stack traceback:").map(|x| x.1);
        assert_eq!(
            our_tb, ref_tb,
            "global-guard traceback body must match reference 5.5"
        );
        assert_eq!(
            rout.status.code(),
            Some(1),
            "reference 5.5 must also exit 1 on the guard"
        );
    }

    let _ = std::fs::remove_file(&script);
}
