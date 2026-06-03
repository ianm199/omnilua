#!/usr/bin/env bash
# Build the Lua 5.5 reference twice: strict compat-off as lua-compat-off,
# then the stock compat-on binary as lua.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/reference/lua-5.5.0/src"
CONF="$SRC/luaconf.h"
BACKUP="$SRC/luaconf.h.compat-backup"

make_target() {
    if [ -n "${LUA_REF_TARGET:-}" ]; then
        echo "$LUA_REF_TARGET"
        return
    fi

    case "$(uname -s)" in
        Darwin) echo "macosx" ;;
        Linux) echo "linux" ;;
        *) echo "posix" ;;
    esac
}

[ -f "$CONF" ] || { echo "[err] missing $CONF" >&2; exit 2; }

restore_conf() {
    if [ -f "$BACKUP" ]; then
        mv "$BACKUP" "$CONF"
    fi
}
trap restore_conf EXIT

TARGET="$(make_target)"

make -C "$SRC" clean
cp "$CONF" "$BACKUP"
LC_ALL=C perl -0pi -e 's/\n#define LUA_COMPAT_GLOBAL\n/\n#undef LUA_COMPAT_GLOBAL\n/' "$CONF"

make -C "$SRC" "$TARGET"
cp "$SRC/lua" "$SRC/lua-compat-off"

restore_conf
trap - EXIT

make -C "$SRC" clean
make -C "$SRC" "$TARGET"
