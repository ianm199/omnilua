# Confirmation: bug #77 — `string.find` spurious trailing empty value (R-A)

**Status: CONFIRMED, current, cross-version (5.1–5.5). CLEAR-CUT — safe to fix to match all references.**

## What the bug is

`string.find(s, pattern)` where `pattern` contains magic chars (so it takes the
pattern-matching path, not the plain-substring fast path) and has **no explicit
captures** returns a third, spurious, empty-string value after `start, end`. Real
Lua returns exactly two values in this case (and zero captures). The extra value
silently inflates return arity that callers branch on (`select('#', ...)`,
multiple-assignment), which is why R-A flagged it as the most dangerous 5.4
regression.

## Exact repros — ours vs each version reference

Reference binaries: `/tmp/lua-refs/bin/lua5.{1.5,2.4,3.6,4.7,5.0}` (unmodified
`make macosx`). Ours: `LUA_RS_VERSION=<v> target/debug/lua-rs`.

### Arity

```
print(select("#", string.find("hello","l+")))
```

| version | ours | reference |
|---|---|---|
| 5.3 | **3** | 2 |
| 5.4 | **3** | 2 |
| 5.5 | **3** | 2 |
| 5.1 (ref only) | — | 2 |
| 5.2 (ref only) | — | 2 |

### Values

```
print(string.find("hello","l+"))
```

| version | ours | reference |
|---|---|---|
| 5.3 / 5.4 / 5.5 | `3<TAB>4<TAB>` (trailing empty string) | `3<TAB>4` |

The spurious 3rd value is the empty byte slice `src[0..0]`.

### Boundary cases — all already MATCH (confirms the fix is narrowly scoped)

| case | result | why |
|---|---|---|
| `string.find("hello","(l+)")` (explicit capture) | MATCH (returns `3 4 ll`) | a real capture exists; arity already correct |
| `string.find("hello","ll")` (literal, no magic) | MATCH (arity 2) | takes the plain-substring fast path, never calls `push_captures` |
| `string.find("hello","l+",1,true)` (plain=true) | MATCH (arity 2) | plain path |
| `string.find("hello","z+")` (no match) | MATCH (returns nil) | match path returns nil |
| `string.match("hello","l+")` (match, not find) | MATCH (returns `ll`, arity 1) | match path *must* return the whole match; correct |
| `string.gmatch` / `string.gsub` with `l+` | MATCH | both call `push_captures` with a non-NULL `s`; correct |

So the divergence is exactly: **`find` path + pattern-matching branch (magic
chars or `^`) + zero explicit captures.**

## Root cause and impl location

`crates/lua-stdlib/src/string_lib.rs`

- **`str_find_aux`, line 1148** — the `find && matched` arm:
  ```rust
  let nc = push_captures(state, &ms, 0, 0)?;   // line 1148
  return Ok(nc + 2);                            // line 1149
  ```
  It pushes `start`,`end`, then calls `push_captures(.., s=0, e=0)`.
- **`push_captures`, line 1067**:
  ```rust
  let nlevels = if ms.level == 0 { 1 } else { ms.level as usize };
  ```
  When there are no captures (`ms.level == 0`) this unconditionally returns 1 and
  pushes one value.
- **`get_one_capture`, line 1037-1045** — with `i=0`, `s=0`, `e=0` returns
  `Bytes(&src[0..0])` = the empty string. That is the spurious 3rd value.

This is a faithful-port miss. In upstream `lstrlib.c`:

```c
static int push_captures (MatchState *ms, const char *s, const char *e) {
  int nlevels = (ms->level == 0 && s) ? 1 : ms->level;   /* note the `&& s` */
  ...
}
```

`str_find_aux` calls `push_captures(ms, NULL, 0)` on the **find** branch (s == NULL)
and `push_captures(ms, s1, res)` on the **match** branch (s != NULL). The
`(ms->level == 0 && s)` guard means: with no captures, *match* pushes the whole
match (s non-NULL ⇒ nlevels 1) but *find* pushes **nothing** (s NULL ⇒ nlevels 0).
Our port dropped the `&& s` condition, so the find path incorrectly synthesizes a
whole-match value.

## Intended fix (clear-cut)

Restore the `&& s` semantics. The port models C's `NULL s` with a sentinel, so the
minimal faithful change is to make `push_captures` take an `Option`/flag for "s is
present" (whole-match allowed), and have the `find` arm pass the absent form.

Concrete shape:

- Change `push_captures` signature to `push_captures(state, ms, s: Option<(usize, usize)>)`,
  computing `nlevels = if ms.level == 0 && s.is_some() { 1 } else { ms.level as usize }`,
  and when pushing index `i >= level`, use the `s` span (only reachable when
  `s.is_some()`).
- `find` arm (line 1148): `push_captures(state, &ms, None)` → returns 0 captures,
  so `Ok(0 + 2)` = arity 2.
- `match` arm (line 1151) and gsub/gmatch sites: pass `Some((s1, res))` /
  `Some((s, e))` — behavior unchanged.

Alternatively, a smaller local fix without touching the signature: in the `find`
arm, when `ms.level == 0`, push nothing and `return Ok(2)`; only call
`push_captures` when `ms.level > 0`. This is the lowest-risk edit and leaves
`match`/`gmatch`/`gsub` completely untouched.

Either way the fix is **shared-core** and matches **every** reference version
(5.1–5.5 all return arity 2), so it is safe under the "must match every version"
rule. Not contract-dependent.

## CI assertions to add (`crates/lua-rs-runtime/tests/multiversion_oracle.rs`)

Using `Lua::new_versioned` + the existing load+pcall wrapper, assert for each of
5.3, 5.4, 5.5:

1. `select("#", string.find("hello","l+"))` returns `2` (was 3).
2. `string.find("hello","l+")` returns exactly `3, 4` and the 3rd return is `nil`
   (e.g. `local a,b,c = string.find("hello","l+"); return tostring(a)..","..tostring(b)..","..tostring(c)` ⇒ `"3,4,nil"`).
3. Regression guards that must stay green (already correct — lock them in):
   - `select("#", string.find("hello","(l+)"))` == `3` and the capture is `"ll"`.
   - `string.match("hello","l+")` == `"ll"` (whole-match still returned).
   - `select("#", string.find("hello","ll"))` == `2` (plain path).
   - `({string.gsub("hello","l+","L")})[2]` (gsub count) == `1`.

## Gate to run after the fix

```
cargo build --workspace
cargo test --workspace --features lua-rs-runtime/derive
specs/oracle/check.sh 5.4 ; specs/oracle/check.sh 5.3 ; specs/oracle/check.sh 5.5
```
plus re-run the four `diff_one.sh` repros above (must all print MATCH).
