#!/usr/bin/env bash
# run-inside.sh — container half of the P2.1 instruction-count rig.
# Mounted at /src (read-only repo) with /cache (persistent volume) for cargo
# and the reference build. Emits TSV rows on stdout.
#
# Two output schemas, selected by the second positional arg ($2 = branch_sim):
#   "no"  (default): workload<TAB>binary<TAB>Ir              — byte-identical to
#                    the original rig; nothing downstream needs to change.
#   "yes":           workload<TAB>binary<TAB>Ir<TAB>Bc<TAB>Bcm<TAB>Bi<TAB>Bim
#                    — adds simulated conditional-branch (Bc), conditional-
#                    mispredict (Bcm), indirect-branch (Bi) and indirect-
#                    mispredict (Bim) counts from valgrind --branch-sim=yes.
# callgrind is simulation-based: counts are deterministic (~0.1%), immune to
# thermal/scheduler noise, and work fine inside VMs — no PMU needed. The same
# is true of the branch predictor model: Bcm/Bim are a deterministic 2-bit
# saturating-counter simulation, not the host CPU's predictor, so they are
# reproducible across machines and the right arbiter for "was it the branch?".
set -euo pipefail

WORKLOADS="$1"
BRANCH_SIM="${2:-no}"
# $3 = cargo feature list (T4 ablation branch only; empty on main reproduces the
# original default build byte-for-byte).
FEATURES="${3:-}"

export CARGO_TARGET_DIR=/cache/target
export CARGO_HOME=/cache/cargo

FEATURE_ARGS=()
if [ -n "$FEATURES" ]; then
    FEATURE_ARGS=(--features "$FEATURES")
    echo "[inside] building lua-rs (release, linux) with features: $FEATURES" >&2
else
    echo "[inside] building lua-rs (release, linux)" >&2
fi
cargo build --release --manifest-path /src/Cargo.toml -p lua-cli -q "${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"}"
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
# per-function drill-downs on a bigger box. --branch-sim=yes adds the four
# branch events (Bc Bcm Bi Bim) to the same summary line at zero extra runs.
BRANCH_FLAG=""
[ "$BRANCH_SIM" = "yes" ] && BRANCH_FLAG="--branch-sim=yes"

# count_ir: emit the Ir total only (default schema).
count_ir() {
    local bin="$1" wpath="$2" out
    out=$(mktemp)
    valgrind --tool=cachegrind --cache-sim=no --cachegrind-out-file="$out" --quiet \
        "$bin" "$wpath" >/dev/null 2>&1 || true
    awk '/^summary:/ {print $2; exit}' "$out"
    rm -f "$out"
}

# count_branch: emit "Ir Bc Bcm Bi Bim" (space-separated) from the branch-sim
# summary line, which is exactly those five fields in that order.
count_branch() {
    local bin="$1" wpath="$2" out
    out=$(mktemp)
    valgrind --tool=cachegrind --cache-sim=no --branch-sim=yes \
        --cachegrind-out-file="$out" --quiet \
        "$bin" "$wpath" >/dev/null 2>&1 || true
    awk '/^summary:/ {print $2, $3, $4, $5, $6; exit}' "$out"
    rm -f "$out"
}

IFS=',' read -ra WLIST <<< "$WORKLOADS"
for wname in "${WLIST[@]}"; do
    wpath="/src/harness/bench/workloads/${wname}.lua"
    [ -f "$wpath" ] || wpath="/src/harness/bench/probes/${wname}.lua"
    [ -f "$wpath" ] || wpath="/extra/${wname}.lua"
    [ -f "$wpath" ] || { echo "[inside] missing workload $wname" >&2; continue; }
    if [ "$BRANCH_SIM" = "yes" ]; then
        echo "[inside] cachegrind --branch-sim: $wname (ref)" >&2
        printf '%s\tref\t%s\n' "$wname" "$(count_branch "$REF_BIN" "$wpath" | tr ' ' '\t')"
        echo "[inside] cachegrind --branch-sim: $wname (rs)" >&2
        printf '%s\trs\t%s\n' "$wname" "$(count_branch "$RS_BIN" "$wpath" | tr ' ' '\t')"
    else
        echo "[inside] cachegrind: $wname (ref)" >&2
        printf '%s\tref\t%s\n' "$wname" "$(count_ir "$REF_BIN" "$wpath")"
        echo "[inside] cachegrind: $wname (rs)" >&2
        printf '%s\trs\t%s\n' "$wname" "$(count_ir "$RS_BIN" "$wpath")"
    fi
done
