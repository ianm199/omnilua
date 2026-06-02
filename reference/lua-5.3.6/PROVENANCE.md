# Lua 5.3.6 reference (secondary behavioral oracle)

This is a pinned, vendored copy of upstream Lua 5.3.6, kept as a **reference
oracle** for version-gated 5.1/5.2/5.3 behavior. It is *not* the port source —
that is Lua 5.4.7 (`reference/lua-5.4.7`, see `harness/source.toml`). 5.3.6 is
here so multi-version behavior (line-hook traces, error wording, bytecode shape)
can be diffed against a real binary instead of hand-recorded constants.

## Pins

| artifact | url | sha256 |
|---|---|---|
| source | https://www.lua.org/ftp/lua-5.3.6.tar.gz | `fc5fd69bb8736323f026672b1b7235da613d7177e72558893a0bdcd320466d60` |
| tests  | https://www.lua.org/tests/lua-5.3.4-tests.tar.gz | `b80771238271c72565e5a1183292ef31bd7166414cd0d43a8eb79845fa7f599f` |

The test suite is the **5.3.4** bundle (the latest Lua published for the 5.3
line; it runs against 5.3.6) extracted into `reference/lua-5.3.6-tests/`.

## Build

```bash
make macosx -C reference/lua-5.3.6      # -> reference/lua-5.3.6/src/lua (gitignored)
```

The built `lua`/`luac`/`*.o`/`liblua.a` are gitignored (rebuild locally), the
same convention as the 5.4.7 reference.

## Why it exists (issue #92, cause 1)

Validated the documented ≤5.3 line-hook divergence and narrowed it to
numeric-`for` only:

| trace (`debug.sethook(f,"l")`) | 5.3.6 | 5.4.7 |
|---|---|---|
| `for i=1,4 do a=1 end` | `1,1,1,1,1` | `1,1,1,1` |
| `for i=1,3 do a=1 end` | `1,1,1,1` | `1,1,1` |
| while / repeat / generic-for | identical | identical |
| multi-line `if/<cond>/then/.../end` | `2,3,4,7` | `2,3,4,7` |

i.e. only numeric-`for` differs (5.3 fires one extra event because `FORPREP`
jumps forward to a bottom test, so iteration 1 also enters the body via a
backward jump). `while`/`repeat`/generic-`for` are byte-identical across
versions.
