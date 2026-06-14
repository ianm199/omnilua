//! Behavioral net for the `os` library's **deterministic** date/time surface.
//!
//! `os` is partly impure: `os.time`'s absolute value depends on the host
//! timezone, and `%z`/`%Z`/`%c`/`%x`/`%X`/`%r`/`%p`/`%P` route through the
//! platform `strftime` in the C reference (locale/zone-dependent). Those are
//! **not reference-pinnable** and are deliberately absent here (documented as
//! honest-negatives in `crates/lua-stdlib/GRADUATED.md`, the analogue of math's
//! platform-`rand()` negative). What IS deterministic — and what this file pins
//! against the version-suffixed reference binaries (`/tmp/lua-refs/bin/lua5.x`,
//! re-confirmed 2026-06-14 byte-identical across 5.1.5/5.2.4/5.3.6/5.4.7/5.5.0)
//! — is:
//!
//! * `os.date("!…", FIXED)` with UTC (`!`) and host-independent specifiers
//!   (numeric/ISO + C-locale English day/month names): identical on every
//!   version and every host.
//! * the `os.time(os.date("*t", t)) == t` LOCAL round-trip: host-independent
//!   because the decompose and recompose share one offset that cancels (the
//!   invariant `files.lua`'s `checkDateTable` checks, but which the harness
//!   SKIPS for fixed epochs under `_port=true`).
//! * the **version-gated** field-validation and specifier-validation seams of
//!   `os.time`/`os.date`. Pinning these across all five versions caught four
//!   real divergences (our impl applied the modern 5.3+/5.4+ rules to ALL
//!   versions); the asserts below are the reference behavior, so the four
//!   `*_crossversion` tests FAILED at baseline and PASS after the fix — the
//!   Phase-2 net-strengthening discipline (cf. `table.remove`, the string
//!   matcher).
//!
//! `omnilua` is a dev-dependency (it depends on `lua-stdlib`, so it can only be
//! a dev-dep — see `Cargo.toml`).

use omnilua::{Lua, LuaVersion, Value};

const ALL: [LuaVersion; 5] = [
    LuaVersion::V51,
    LuaVersion::V52,
    LuaVersion::V53,
    LuaVersion::V54,
    LuaVersion::V55,
];

/// Evaluate `code` under `version`, returning a string return value as bytes.
fn eval_str(version: LuaVersion, code: &str) -> Vec<u8> {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(Value::String(s)) => s
            .as_bytes()
            .unwrap_or_else(|e| panic!("string bytes under {version:?} for `{code}`: {e:?}"))
            .to_vec(),
        Ok(other) => panic!("`{code}` under {version:?} returned {other:?}, expected a string"),
        Err(e) => panic!("eval of `{code}` failed under {version:?}: {e:?}"),
    }
}

/// Evaluate `code` under `version`, returning an integer return value.
fn eval_int(version: LuaVersion, code: &str) -> i64 {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(Value::Integer(i)) => i,
        Ok(Value::Number(n)) if n.fract() == 0.0 => n as i64,
        Ok(other) => panic!("`{code}` under {version:?} returned {other:?}, expected an integer"),
        Err(e) => panic!("eval of `{code}` failed under {version:?}: {e:?}"),
    }
}

/// Evaluate `code`, expecting a boolean-`true` return (an invariant probe).
fn assert_true(version: LuaVersion, code: &str) {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(Value::Boolean(true)) => {}
        Ok(other) => panic!("`{code}` under {version:?} returned {other:?}, expected true"),
        Err(e) => panic!("eval of `{code}` failed under {version:?}: {e:?}"),
    }
}

/// Evaluate `code`, expecting it to raise; return the error message lossily.
fn eval_err(version: LuaVersion, code: &str) -> String {
    let lua = Lua::new_versioned(version);
    match lua.load(code).eval::<Value>() {
        Ok(v) => panic!("expected error under {version:?} for `{code}`, got {v:?}"),
        Err(e) => e.message_lossy(),
    }
}

// ── Deterministic UTC date formatting (host-independent, all versions) ────────

#[test]
fn date_utc_table_at_epoch_zero() {
    for v in ALL {
        assert_eq!(eval_int(v, "return os.date('!*t', 0).year"), 1970, "{v:?}");
        assert_eq!(eval_int(v, "return os.date('!*t', 0).month"), 1, "{v:?}");
        assert_eq!(eval_int(v, "return os.date('!*t', 0).day"), 1, "{v:?}");
        assert_eq!(eval_int(v, "return os.date('!*t', 0).hour"), 0, "{v:?}");
        assert_eq!(eval_int(v, "return os.date('!*t', 0).min"), 0, "{v:?}");
        assert_eq!(eval_int(v, "return os.date('!*t', 0).sec"), 0, "{v:?}");
        // wday is 1-based with Sunday=1; 1970-01-01 was a Thursday → 5.
        assert_eq!(eval_int(v, "return os.date('!*t', 0).wday"), 5, "{v:?}");
        assert_eq!(eval_int(v, "return os.date('!*t', 0).yday"), 1, "{v:?}");
    }
}

#[test]
fn date_utc_iso_strings_at_fixed_epochs() {
    for v in ALL {
        assert_eq!(
            eval_str(v, "return os.date('!%Y-%m-%dT%H:%M:%S', 0)"),
            b"1970-01-01T00:00:00",
            "{v:?}"
        );
        assert_eq!(
            eval_str(v, "return os.date('!%Y-%m-%dT%H:%M:%S', 1000000000)"),
            b"2001-09-09T01:46:40",
            "{v:?}"
        );
        assert_eq!(
            eval_str(v, "return os.date('!%Y-%m-%dT%H:%M:%S', 1700000000)"),
            b"2023-11-14T22:13:20",
            "{v:?}"
        );
        // Negative epoch (pre-1970) — exercises the Euclidean division path.
        assert_eq!(
            eval_str(v, "return os.date('!%Y-%m-%d %H:%M:%S', -1)"),
            b"1969-12-31 23:59:59",
            "{v:?}"
        );
        assert_eq!(
            eval_str(v, "return os.date('!%Y-%m-%d', -100000000)"),
            b"1966-10-31",
            "{v:?}"
        );
    }
}

#[test]
fn date_utc_english_names_are_c_locale() {
    // C-locale English day/month names: our impl hardcodes them, and the
    // reference (run under the C locale) matches byte-for-byte on every version.
    for v in ALL {
        assert_eq!(
            eval_str(v, "return os.date('!%a|%A|%b|%B', 1700000000)"),
            b"Tue|Tuesday|Nov|November",
            "{v:?}"
        );
    }
}

#[test]
fn date_utc_numeric_specifiers() {
    for v in ALL {
        assert_eq!(
            eval_str(v, "return os.date('!%y|%C|%m|%d|%e|%H|%I|%M|%S|%j|%w|%u', 1700000000)"),
            b"23|20|11|14|14|22|10|13|20|318|2|2",
            "{v:?}"
        );
        assert_eq!(
            eval_str(v, "return os.date('!%F|%T|%R|%D', 1700000000)"),
            b"2023-11-14|22:13:20|22:13|11/14/23",
            "{v:?}"
        );
    }
}

#[test]
fn date_literal_passthrough_and_percent_escape() {
    for v in ALL {
        assert_eq!(eval_str(v, "return os.date('!ab%Ycd', 0)"), b"ab1970cd", "{v:?}");
        assert_eq!(eval_str(v, "return os.date('!%%', 0)"), b"%", "{v:?}");
        // Empty format → empty string (files.lua: os.date("") == "").
        assert_eq!(eval_str(v, "return os.date('')"), b"", "{v:?}");
        assert_eq!(eval_str(v, "return os.date('!')"), b"", "{v:?}");
        // Embedded NULs pass through verbatim (files.lua: os.date("\0\0")).
        assert_eq!(eval_str(v, "return os.date('!\\0\\0')"), b"\0\0", "{v:?}");
    }
}

// ── The host-independent os.time round-trip invariant ─────────────────────────

#[test]
fn time_local_round_trip_is_host_independent() {
    // os.date("*t", t) decomposes with the host offset; os.time recomposes with
    // the same offset → they cancel, so the round trip equals `t` on every host
    // and every version. (The absolute value of os.time(table) is host-TZ
    // dependent and is deliberately NOT pinned — see the module doc.)
    let probe = "\
        local function rt(t) return os.time(os.date('*t', t)) == t end \
        return rt(0) and rt(1000) and rt(1000000000) and rt(1700000000) and rt(0x7fffffff)";
    for v in ALL {
        assert_true(v, probe);
    }
}

#[test]
fn time_default_fields_normalize_into_epoch() {
    // os.time fills hour=12, min=0, sec=0 when absent (all versions), and
    // normalizes month/day overflow. With no offset hook installed in the
    // embedding the absolute number is host-dependent, so we pin only the
    // host-independent *consistency*: explicit-noon equals default-noon, and a
    // 13th month equals next-January.
    let probe = "\
        local a = os.time{year=2000, month=1, day=1} \
        local b = os.time{year=2000, month=1, day=1, hour=12, min=0, sec=0} \
        local c = os.time{year=2023, month=13, day=1, hour=0, min=0, sec=0} \
        local d = os.time{year=2024, month=1, day=1, hour=0, min=0, sec=0} \
        return a == b and c == d";
    for v in ALL {
        assert_true(v, probe);
    }
}

// ── Version-gated field validation (the seams that caught four bugs) ──────────

#[test]
fn time_missing_field_names_first_unread_required_field_crossversion() {
    // The field-read ORDER differs: 5.1/5.2/5.3 read sec,min,hour,day,month,year
    // (first no-default field = `day`); 5.4/5.5 read year,month,day,…
    // (first no-default field = `year`). So `os.time{}` names a DIFFERENT field.
    for v in [LuaVersion::V51, LuaVersion::V52, LuaVersion::V53] {
        assert!(
            eval_err(v, "return os.time({})").contains("field 'day' missing in date table"),
            "{v:?}: {}",
            eval_err(v, "return os.time({})")
        );
    }
    for v in [LuaVersion::V54, LuaVersion::V55] {
        assert!(
            eval_err(v, "return os.time({})").contains("field 'year' missing in date table"),
            "{v:?}: {}",
            eval_err(v, "return os.time({})")
        );
    }
    // With year present (5.4/5.5) the next missing required field is `month`.
    for v in [LuaVersion::V54, LuaVersion::V55] {
        assert!(
            eval_err(v, "return os.time({year=2000})")
                .contains("field 'month' missing in date table"),
            "{v:?}"
        );
    }
}

#[test]
fn time_non_integer_field_is_unchecked_pre_5_3_crossversion() {
    // 5.1/5.2 `getfield` does NOT raise "is not an integer": a present-but-non-
    // integer field is treated as absent (→ its default, or "missing" if it has
    // none). So `os.time{year=1000, month=1, day=1, hour='x'}` succeeds (hour
    // falls back to its default 12) on 5.1/5.2, but raises on 5.3+.
    let code = "return os.time({year=1000, month=1, day=1, hour='x'})";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        // Must NOT raise; returns an integer (the host-dependent epoch).
        let lua = Lua::new_versioned(v);
        match lua.load(code).eval::<Value>() {
            Ok(Value::Integer(_)) | Ok(Value::Number(_)) => {}
            other => panic!("{v:?}: expected a number (no type-check), got {other:?}"),
        }
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert!(
            eval_err(v, code).contains("field 'hour' is not an integer"),
            "{v:?}: {}",
            eval_err(v, code)
        );
    }
    // A fractional float is likewise unchecked pre-5.3, an error 5.3+.
    let code2 = "return os.time({year=1000, month=1, day=1, hour=1.5})";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        let lua = Lua::new_versioned(v);
        assert!(
            matches!(
                lua.load(code2).eval::<Value>(),
                Ok(Value::Integer(_)) | Ok(Value::Number(_))
            ),
            "{v:?}: fractional hour must not raise pre-5.3"
        );
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert!(eval_err(v, code2).contains("field 'hour' is not an integer"), "{v:?}");
    }
}

#[test]
fn time_out_of_bound_field_is_unchecked_pre_5_3_crossversion() {
    // 5.1/5.2 do not bounds-check; 5.3+ raise "field '…' is out-of-bound".
    let code = "return os.time({year=0, month=1, day=2^32})";
    for v in [LuaVersion::V51, LuaVersion::V52] {
        let lua = Lua::new_versioned(v);
        assert!(
            matches!(
                lua.load(code).eval::<Value>(),
                Ok(Value::Integer(_)) | Ok(Value::Number(_))
            ),
            "{v:?}: huge day must not raise pre-5.3 (no bounds check)"
        );
    }
    for v in [LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert!(
            eval_err(v, code).contains("field 'day' is out-of-bound"),
            "{v:?}: {}",
            eval_err(v, code)
        );
    }
}

#[test]
fn date_invalid_specifier_is_unvalidated_on_5_1_crossversion() {
    // 5.1's `os_date` has NO `checkoption`: an unknown `%X` is passed straight to
    // the platform strftime, never raising a Lua error. 5.2+ validate against a
    // fixed option set and raise "invalid conversion specifier".
    for v in [LuaVersion::V51] {
        let lua = Lua::new_versioned(v);
        assert!(
            lua.load("return os.date('!%')").eval::<Value>().is_ok(),
            "5.1 must not raise on a bare trailing %"
        );
        let lua = Lua::new_versioned(v);
        assert!(
            lua.load("return os.date('!%9')").eval::<Value>().is_ok(),
            "5.1 must not raise on an unknown specifier"
        );
    }
    for v in [LuaVersion::V52, LuaVersion::V53, LuaVersion::V54, LuaVersion::V55] {
        assert!(
            eval_err(v, "return os.date('!%')").contains("invalid conversion specifier"),
            "{v:?}"
        );
        assert!(
            eval_err(v, "return os.date('!%9')").contains("invalid conversion specifier"),
            "{v:?}"
        );
    }
}
