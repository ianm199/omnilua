# Lua 5.5.0 reference (secondary behavioral oracle)

This is a pinned, vendored copy of upstream Lua 5.5.0, kept as a reference
oracle for version-gated 5.5 behavior. It is not the port source; the top-level
port contract remains Lua 5.4.7 unless a harness runner explicitly selects 5.5.

## Pins

| artifact | url | sha256 |
|---|---|---|
| source | https://www.lua.org/ftp/lua-5.5.0.tar.gz | `57ccc32bbbd005cab75bcc52444052535af691789dba2b9016d5c50640d68b3d` |
| tests | https://github.com/lua/lua/archive/refs/tags/v5.5.0.tar.gz (`testes/`) | `a33484f7ce4c14e12ea4d51cc5a7353bff2796a8074004b96ae2dc246f33f16e` |

The test suite is the `testes/` tree from the `lua/lua` GitHub tag `v5.5.0`,
extracted into `reference/lua-5.5.0-tests/`.

## Build

```bash
make -C reference/lua-5.5.0 guess
bash harness/build_lua55_compat_off.sh
```

The helper temporarily undefines `LUA_COMPAT_GLOBAL` in `luaconf.h`, builds
`src/lua-compat-off`, restores the header, then rebuilds the stock compat-on
`src/lua`. It selects `macosx`, `linux`, or `posix` from `uname`; override with
`LUA_REF_TARGET=<target>` if needed. The built `lua`/`luac`/`*.o`/`liblua.a`/
`lua-compat-off` are gitignored and should be rebuilt locally, following the
same convention as the 5.3.6 and 5.4.7 references.
