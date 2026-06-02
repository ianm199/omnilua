# lua-stdlib — the standard library

The base/string/table/math/os/io/coroutine/utf8/bit32 libraries. Second-largest
crate. Read the root `../../CLAUDE.md` first.

## Version rosters are data-driven — gate entries, never fork modules

Which functions exist differs by version, and the registration is a **per-version
table** (`src/init.rs` + per-lib bodies). Gate **individual entries**, not whole
modules:

- 5.1: global `unpack` (no `table.unpack`/`pack`/`move`); `loadstring`;
  `table.getn`/`foreach`/`foreachi`/`maxn`; `string.gfind`; `math.log` 1-arg +
  `log10`/`atan2`/`pow`/`mod` (no `math.type`); `gcinfo`; `newproxy`; `module`/
  `package.seeall`/`package.loaders`.
- `bit32` present in 5.2/5.3, absent in 5.1/5.4/5.5. `utf8` from 5.3.
  `string.pack`/`unpack`/`packsize` from 5.3. `math.type`/`math.tointeger` from
  5.3 (and they return a `fail` = `nil`, not `false`).
- Compat-math (`math.atan2`/`cosh`/`sinh`/`tanh`/`pow`/`log10`, `frexp`/`ldexp`)
  follows the reference's default `LUA_COMPAT_*` build flags — these are part of
  the contract, not optional. The playbook §1 spells out which flags are ON where.

## Error-message fidelity is in scope

`bad argument #N to '<fn>'` wording, the `got no value` / type qualifiers, and
location prefixes are oracle-checked. Many are version-specific. When you change a
message, capture the expected value from the reference binary
(`specs/oracle/diff_one.sh <ver> '...'`) and add it to `multiversion_oracle.rs` —
do not hand-write the expected string.

## Lua strings are bytes

Everything here handles `&[u8]`/`LuaString`, never `String`/`&str` (enforced).
`string.*` operates on byte strings; pattern matching is byte-wise.

## Test
`cargo test -p lua-stdlib`; behavior in `multiversion_oracle.rs`; full programs
via `harness/run_official_test.sh reference/lua-c/testes/{strings,errors,math}.lua`.
