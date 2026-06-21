//! `dump_kit` — golden + roundtrip kit for the `string.dump` bytecode header
//! across all five versions (the deferred per-version-header architectural item).
//!
//! Flavor: golden constants (the reference header bytes, captured once into
//! `tests/golden/dump_headers.tsv` by `harness/gen_golden.sh`) + a roundtrip
//! invariant (`dump -> load -> call` must reproduce the value). Pure in-process:
//! no reference binary, no subprocess, no `/tmp` dependency — the rung-2 inner
//! loop the dump-header fix develops against, in milliseconds.
//!
//! COVERED: the header bytes a freshly-dumped function emits per version
//! (signature, version tag, format, size fields, sentinels) — exactly the prefix
//! `calls.lua` asserts — and that our own `dump`/`undump` round-trip.
//! NOT COVERED: full cross-binary BODY byte-fidelity vs the reference C dumper
//! (5.1/5.2 use a structurally different body our internal format does not emit);
//! the kit pins the header contract and self-consistency, not body interchange.

use omnilua::{Lua, LuaVersion};

const VERSIONS: &[(&str, LuaVersion)] = &[
    ("5.1", LuaVersion::V51),
    ("5.2", LuaVersion::V52),
    ("5.3", LuaVersion::V53),
    ("5.4", LuaVersion::V54),
    ("5.5", LuaVersion::V55),
];

/// Evaluate `code` under `version`, returning the string it produces. The
/// snippet is `load`/`loadstring`+`pcall`ed inside Lua so the running version's
/// own loader and renderer are exercised.
fn eval_str(version: LuaVersion, code: &str) -> String {
    let lua = Lua::new_versioned(version);
    let loader = if version == LuaVersion::V51 {
        "loadstring"
    } else {
        "load"
    };
    let wrapper = format!(
        "local f, e = {loader}([==[\n{code}\n]==])\n\
         if not f then error('load: ' .. tostring(e)) end\n\
         return f()"
    );
    lua.load(&wrapper)
        .eval()
        .unwrap_or_else(|e| panic!("dump_kit harness failure: {e:?}"))
}

/// Hex of the first `n` bytes of `string.dump(function() return 1 end)` under
/// `version`, computed inside Lua (avoids binary-in-`String` round-tripping).
fn our_header_hex(version: LuaVersion, n: usize) -> String {
    let code = format!(
        "local d = string.dump(function() return 1 end)\n\
         local t = {{}}\n\
         for i = 1, {n} do t[i] = string.format('%02x', string.byte(d, i)) end\n\
         return table.concat(t)"
    );
    eval_str(version, &code)
}

/// Parse the committed golden: version -> (header_len, hex_bytes).
fn golden() -> Vec<(String, usize, String)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/dump_headers.tsv");
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read golden {path}: {e} (run harness/gen_golden.sh)"));
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            let v = it.next().unwrap().to_string();
            let n: usize = it.next().unwrap().parse().unwrap();
            let hex = it.next().unwrap().to_string();
            (v, n, hex)
        })
        .collect()
}

fn version_of(tag: &str) -> LuaVersion {
    VERSIONS
        .iter()
        .find(|(s, _)| *s == tag)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| panic!("unknown version tag {tag}"))
}

#[test]
fn dump_header_matches_reference_golden() {
    let mut failures = Vec::new();
    for (tag, n, want_hex) in golden() {
        let version = version_of(&tag);
        let got_hex = our_header_hex(version, n);
        if got_hex != want_hex {
            failures.push(format!(
                "  {tag}: header mismatch (first {n} bytes)\n      ours: {got_hex}\n      ref : {want_hex}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "string.dump header diverges from reference:\n{}",
        failures.join("\n")
    );
}

#[test]
fn dump_load_roundtrip_reproduces_value() {
    for (tag, version) in VERSIONS {
        let got = eval_str(
            *version,
            "local d = string.dump(function() return 42 end)\n\
             local f = (loadstring or load)(d)\n\
             return tostring(f())",
        );
        assert_eq!(got, "42", "dump->load roundtrip failed on {tag}");
    }
}
