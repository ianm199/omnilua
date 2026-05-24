//! Lua ctype — character-classification table and predicates.
//!
//! Ported from `reference/lua-5.4.7/src/lctype.c` and `lctype.h`.
//!
//! Lua ships its own ctype replacements, optimised for its specific needs.
//! These do **not** match the standard C `<ctype.h>` semantics exactly; in
//! particular `lislalpha` / `lislalnum` treat `'_'` as alphabetic, and the
//! table is seeded for ASCII byte ranges only (with high bytes left at 0x00
//! unless `LUA_UCID` is enabled — see PORT NOTE below).
//!
//! On ASCII targets (`LUA_USE_CTYPE=0`, the default) the implementation is a
//! 257-entry byte lookup table.  Each entry is a bitfield:
//!
//! | bit | name      | meaning                                      |
//! |-----|-----------|----------------------------------------------|
//! |  0  | ALPHABIT  | Lua-alphabetic: ASCII letters plus `_`       |
//! |  1  | DIGITBIT  | decimal digit `0`-`9`                        |
//! |  2  | PRINTBIT  | printable (graph + space)                    |
//! |  3  | SPACEBIT  | whitespace (ASCII space, TAB, LF, VT, FF, CR)|
//! |  4  | XDIGITBIT | hex digit `0`-`9`, `A`-`F`, `a`-`f`         |
//!
//! `test_prop(c, mask)` indexes the table as `CTYPE_TABLE[(c + 1) as usize]`,
//! which allows `c = -1` (the `EOZ` end-of-stream sentinel) without underflow.
//!
//! PORT NOTE: The C code supports a compile-time `LUA_UCID` flag that sets all
//! non-ASCII bytes (0x80-0xFF, minus invalid UTF-8 sequences) to `ALPHABIT`
//! so that Unicode identifiers are recognised.  That path (`NONA = 0x01`) is
//! not translated here; only the default `NONA = 0x00` path is ported.
//! Enable it in Phase B by introducing a Cargo feature flag.

// C: #define ALPHABIT  0
const ALPHABIT: u32 = 0;

// C: #define DIGITBIT  1
const DIGITBIT: u32 = 1;

// C: #define PRINTBIT  2
const PRINTBIT: u32 = 2;

// C: #define SPACEBIT  3
const SPACEBIT: u32 = 3;

// C: #define XDIGITBIT 4
const XDIGITBIT: u32 = 4;

// C: #define MASK(B) (1 << (B))
// Inlined at each call site below as `1u8 << BIT`.

// C: #define NONA 0x00   /* non-ASCII bytes are not alphabetic by default */
// LUA_UCID disabled — all non-ASCII bytes remain 0x00.

// C: LUAI_DDEF const lu_byte luai_ctype_[UCHAR_MAX + 2] = { ... };
//
// UCHAR_MAX + 2 = 255 + 2 = 257 entries.
// Entry 0         → EOZ sentinel (c = -1; index = -1 + 1 = 0).
// Entries 1-256   → bytes 0x00-0xFF.
//
// Bit-flag legend (combined values seen in the table):
//   0x00 = no property (NUL, control chars, DEL, high bytes)
//   0x04 = PRINTBIT only (punctuation, symbols)
//   0x05 = ALPHABIT | PRINTBIT (non-hex letters + '_')
//   0x06 = DIGITBIT | PRINTBIT (this value does not appear alone; digits always have XDIGITBIT)
//   0x08 = SPACEBIT (TAB through CR)
//   0x0c = SPACEBIT | PRINTBIT (ASCII space 0x20)
//   0x15 = ALPHABIT | PRINTBIT | XDIGITBIT (A-F, a-f)
//   0x16 = DIGITBIT | PRINTBIT | XDIGITBIT (0-9)
pub(crate) static CTYPE_TABLE: [u8; 257] = [
    // C: 0x00,  /* EOZ */
    0x00,
    // C: 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  /* 0. bytes 0x00-0x07 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: 0x00, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00,  /* bytes 0x08-0x0F */
    //    BS    TAB   LF    VT    FF    CR
    0x00, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00,
    // C: 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  /* 1. bytes 0x10-0x17 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  /* bytes 0x18-0x1F */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: 0x0c, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,  /* 2. bytes 0x20-0x27 */
    //    SPC   !     "     #     $     %     &     '
    0x0c, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    // C: 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,  /* bytes 0x28-0x2F */
    //    (     )     *     +     ,     -     .     /
    0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    // C: 0x16, 0x16, 0x16, 0x16, 0x16, 0x16, 0x16, 0x16,  /* 3. bytes 0x30-0x37 */
    //    0     1     2     3     4     5     6     7
    0x16, 0x16, 0x16, 0x16, 0x16, 0x16, 0x16, 0x16,
    // C: 0x16, 0x16, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,  /* bytes 0x38-0x3F */
    //    8     9     :     ;     <     =     >     ?
    0x16, 0x16, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    // C: 0x04, 0x15, 0x15, 0x15, 0x15, 0x15, 0x15, 0x05,  /* 4. bytes 0x40-0x47 */
    //    @     A     B     C     D     E     F     G
    0x04, 0x15, 0x15, 0x15, 0x15, 0x15, 0x15, 0x05,
    // C: 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,  /* bytes 0x48-0x4F */
    //    H     I     J     K     L     M     N     O
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    // C: 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,  /* 5. bytes 0x50-0x57 */
    //    P     Q     R     S     T     U     V     W
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    // C: 0x05, 0x05, 0x05, 0x04, 0x04, 0x04, 0x04, 0x05,  /* bytes 0x58-0x5F */
    //    X     Y     Z     [     \     ]     ^     _
    0x05, 0x05, 0x05, 0x04, 0x04, 0x04, 0x04, 0x05,
    // C: 0x04, 0x15, 0x15, 0x15, 0x15, 0x15, 0x15, 0x05,  /* 6. bytes 0x60-0x67 */
    //    `     a     b     c     d     e     f     g
    0x04, 0x15, 0x15, 0x15, 0x15, 0x15, 0x15, 0x05,
    // C: 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,  /* bytes 0x68-0x6F */
    //    h     i     j     k     l     m     n     o
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    // C: 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,  /* 7. bytes 0x70-0x77 */
    //    p     q     r     s     t     u     v     w
    0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
    // C: 0x05, 0x05, 0x05, 0x04, 0x04, 0x04, 0x04, 0x00,  /* bytes 0x78-0x7F */
    //    x     y     z     {     |     }     ~     DEL
    0x05, 0x05, 0x05, 0x04, 0x04, 0x04, 0x04, 0x00,
    // C: NONA * 8, /* 8. bytes 0x80-0x87 */  (NONA = 0x00 in non-LUA_UCID build)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* bytes 0x88-0x8F */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* 9. bytes 0x90-0x97 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* bytes 0x98-0x9F */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* a. bytes 0xA0-0xA7 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* bytes 0xA8-0xAF */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* b. bytes 0xB0-0xB7 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* bytes 0xB8-0xBF */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: 0x00, 0x00, NONA, NONA, NONA, NONA, NONA, NONA,  /* c. bytes 0xC0-0xC7 */
    //    0xC0 and 0xC1 are invalid UTF-8 leading bytes → 0x00
    //    0xC2-0xC7 are valid UTF-8 two-byte sequence starters → NONA (0x00 here)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA, NONA, NONA, NONA, NONA, NONA, NONA, NONA,  /* bytes 0xC8-0xCF */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* d. bytes 0xD0-0xD7 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* bytes 0xD8-0xDF */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* e. bytes 0xE0-0xE7 */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA * 8, /* bytes 0xE8-0xEF */
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: NONA, NONA, NONA, NONA, NONA, 0x00, 0x00, 0x00,  /* f. bytes 0xF0-0xF7 */
    //    0xF0-0xF4 are valid UTF-8 four-byte starters → NONA (0x00 here)
    //    0xF5-0xF7 are invalid UTF-8 → 0x00
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // C: 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00   /* bytes 0xF8-0xFF */
    //    all invalid UTF-8 sequences → 0x00
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

// C: #define testprop(c,p) (luai_ctype_[(c)+1] & (p))
//
// `c` is an `i32` in Lua's internal representation: it is either a byte value
// 0-255, or -1 for EOZ.  Adding 1 shifts the range to 0-256, all valid indices
// into the 257-element table.
#[inline]
fn test_prop(c: i32, mask: u8) -> bool {
    debug_assert!(
        c >= -1 && c <= 255,
        "test_prop: c out of range: {}",
        c
    );
    CTYPE_TABLE[(c + 1) as usize] & mask != 0
}

// C: #define lislalpha(c) testprop(c, MASK(ALPHABIT))
//
// True for ASCII letters A-Z, a-z, and the underscore '_'.
// Includes non-ASCII bytes if LUA_UCID is enabled (not translated here).
#[inline]
pub(crate) fn lislalpha(c: i32) -> bool {
    test_prop(c, 1u8 << ALPHABIT)
}

// C: #define lislalnum(c) testprop(c, (MASK(ALPHABIT) | MASK(DIGITBIT)))
//
// True for ASCII letters, digits, and '_'.
#[inline]
pub(crate) fn lislalnum(c: i32) -> bool {
    test_prop(c, (1u8 << ALPHABIT) | (1u8 << DIGITBIT))
}

// C: #define lisdigit(c) testprop(c, MASK(DIGITBIT))
//
// True for ASCII decimal digits '0'-'9'.
#[inline]
pub(crate) fn lisdigit(c: i32) -> bool {
    test_prop(c, 1u8 << DIGITBIT)
}

// C: #define lisspace(c) testprop(c, MASK(SPACEBIT))
//
// True for ASCII whitespace: space (0x20), TAB (0x09), LF (0x0A),
// VT (0x0B), FF (0x0C), CR (0x0D).
#[inline]
pub(crate) fn lisspace(c: i32) -> bool {
    test_prop(c, 1u8 << SPACEBIT)
}

// C: #define lisprint(c) testprop(c, MASK(PRINTBIT))
//
// True for printable characters: ASCII space through '~' (0x20-0x7E).
#[inline]
pub(crate) fn lisprint(c: i32) -> bool {
    test_prop(c, 1u8 << PRINTBIT)
}

// C: #define lisxdigit(c) testprop(c, MASK(XDIGITBIT))
//
// True for hexadecimal digits: '0'-'9', 'A'-'F', 'a'-'f'.
#[inline]
pub(crate) fn lisxdigit(c: i32) -> bool {
    test_prop(c, 1u8 << XDIGITBIT)
}

// C: #define ltolower(c)  \
//      check_exp(('A' <= (c) && (c) <= 'Z') || (c) == ((c) | ('A' ^ 'a')), \
//                (c) | ('A' ^ 'a'))
//
// Converts an uppercase ASCII letter to its lowercase equivalent by setting
// bit 5 (0x20).  Only safe to call on uppercase letters A-Z, or on characters
// that already have bit 5 set (lowercase letters, '.', etc.).
//
// From macros.tsv: `check_exp(c, e)` → `{ debug_assert!(c); e }`.
// `'A' ^ 'a'` = 65 ^ 97 = 32 = 0x20.
#[inline]
pub(crate) fn ltolower(c: i32) -> i32 {
    debug_assert!(
        ('A' as i32 <= c && c <= 'Z' as i32) || c == (c | ('A' as i32 ^ 'a' as i32)),
        "ltolower: argument must be an uppercase letter or already lowercase/'.'"
    );
    c | ('A' as i32 ^ 'a' as i32)
}

// ──────────────────────────────────────────────────────────────────────────
// PORT STATUS
//   source:        src/lctype.c  (64 lines, 0 functions — only a table + header macros)
//   target_crate:  lua-vm
//   confidence:    high
//   todos:         0
//   port_notes:    1
//   unsafe_blocks: 0   (must be 0 outside explicit unsafe-budget crates)
//   notes:         Straightforward table + inline predicates; LUA_UCID path
//                  omitted (PORT NOTE in module doc). Phase B: add Cargo
//                  feature `lua-ucid` that substitutes NONA=0x01 for the
//                  non-ASCII rows.
// ──────────────────────────────────────────────────────────────────────────
