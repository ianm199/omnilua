#!/usr/bin/env bash
# run-inside.sh — container half of the P2.1 instruction-count rig.
# Mounted at /src (read-only repo) with /cache (persistent volume) for cargo
# and the reference build. Emits TSV rows on stdout:
#   workload<TAB>binary<TAB>Ir
# callgrind is simulation-based: counts are deterministic (~0.1%), immune to
# thermal/scheduler noise, and work fine inside VMs — no PMU needed.
set -euo pipefail

WORKLOADS="$1"

export CARGO_TARGET_DIR=/cache/target
export CARGO_HOME=/cache/cargo

echo "[inside] building lua-rs (release, linux)" >&2
cargo build --release --manifest-path /src/Cargo.toml -p lua-cli -q
RS_BIN="$CARGO_TARGET_DIR/release/lua-rs"

if ! /cache/luaref/src/lua -v >/dev/null 2>&1; then
    echo "[inside] building reference C lua (posix)" >&2
    rm -rf /cache/luaref
    cp -r /src/reference/lua-5.4.7 /cache/luaref
    make -C /cache/luaref clean >/dev/null 2>&1 || true
    make -C /cache/luaref posix -j4 >/dev/null 2>&1 || make -C /cache/luaref posix
fi
REF_BIN=/cache/luaref/src/lua
"$REF_BIN" -v >/dev/null 2>&1 || { echo "[inside] reference binary unusable after build" >&2; exit 2; }

# cachegrind with --cache-sim=no, not callgrind: callgrind's call-graph
# tracking OOMs the ~8GB docker VM on these workloads (exit 137). Cachegrind
# gives the same deterministic Ir total; use callgrind manually for
# per-function drill-downs on a bigger box.
count_ir() {
    local bin="$1" wpath="$2" out
    out=$(mktemp)
    valgrind --tool=cachegrind --cache-sim=no --cachegrind-out-file="$out" --quiet \
        "$bin" "$wpath" >/dev/null 2>&1 || true
    awk '/^summary:/ {print $2; exit}' "$out"
    rm -f "$out"
}

IFS=',' read -ra WLIST <<< "$WORKLOADS"
for wname in "${WLIST[@]}"; do
    wpath="/src/harness/bench/workloads/${wname}.lua"
    [ -f "$wpath" ] || { echo "[inside] missing workload $wname" >&2; continue; }
    echo "[inside] callgrind: $wname (ref)" >&2
    printf '%s\tref\t%s\n' "$wname" "$(count_ir "$REF_BIN" "$wpath")"
    echo "[inside] callgrind: $wname (rs)" >&2
    printf '%s\trs\t%s\n' "$wname" "$(count_ir "$RS_BIN" "$wpath")"
done
