#!/usr/bin/env bash
# heap-diff.sh — dhat heap-allocation diff for one workload across two commits.
#
# RSS packets (UpVal diet, GC alloc work) need to show that ALLOC COUNT and
# BYTES-PER-BLOCK actually moved, not hand-wave a noisy RSS number. This script
# builds the dhat-instrumented lua-cli at commit A and commit B, runs the named
# workload under each, and prints total allocations, total bytes, bytes-per-
# block and peak-live bytes for A, B and the delta.
#
# It reuses the SAME instrument as table-bytes.sh: a `--features dhat-heap`
# release build (crates/lua-cli wires dhat::Alloc as the global allocator and
# dhat::Profiler::new_heap() at main()), whose stderr summary is:
#   dhat: Total:     34,507,978 bytes in 201,201 blocks
#   dhat: At t-gmax: 26,864,480 bytes in 200,639 blocks   <- peak live
#   dhat: At t-end:  ...
# Total gives alloc count + total bytes; t-gmax gives peak live bytes;
# bytes-per-block = total bytes / total blocks. dhat reports allocator-request
# bytes (not malloc-bucket-rounded RSS), same as table-bytes.sh — fair for
# deltas, absolute RSS rounds up to size classes.
#
# Usage:
#   bash harness/bench/heap-diff.sh closure_ops
#       # A = origin/main, B = current working tree (HEAD + any uncommitted)
#   bash harness/bench/heap-diff.sh closure_ops origin/main HEAD
#   bash harness/bench/heap-diff.sh gc_pressure abc123 def456
#
# commitA is always built in a throwaway `git worktree add` under a temp dir so
# the user's tree is never touched. commitB defaults to the WORKING TREE (built
# in place, in a scratch target dir) so you can diff uncommitted edits; pass an
# explicit ref for B to build it from a clean worktree too. Each build uses its
# own CARGO_TARGET_DIR so neither clobbers the user's target/.
#
# bash-3.2 / set -u clean (macOS default shell). Works from repo root or from
# harness/bench/.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOAD="${1:?usage: $0 <workload> [<commitA> <commitB>]}"
COMMIT_A="${2:-origin/main}"
COMMIT_B="${3:-__WORKTREE__}"

resolve_workload() {
    local base="$1" name="$2" p
    p="$base/harness/bench/workloads/${name}.lua"
    [ -f "$p" ] || p="$base/harness/bench/probes/${name}.lua"
    [ -f "$p" ] && { printf '%s\n' "$p"; return 0; }
    return 1
}

if ! resolve_workload "$ROOT" "$WORKLOAD" >/dev/null; then
    echo "[err] workload not found: $WORKLOAD (looked in workloads/ and probes/)" >&2
    exit 2
fi

SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/heap-diff-XXXXXX")"
WORKTREES=()
cleanup() {
    local wt
    if [ "${#WORKTREES[@]}" -gt 0 ]; then
        for wt in ${WORKTREES[@]+"${WORKTREES[@]}"}; do
            git worktree remove --force "$wt" 2>/dev/null || true
        done
    fi
    rm -rf "$SCRATCH"
}
trap cleanup EXIT

# build_and_run <label> <source_dir> <workload_name> -> echoes
#   "<total_blocks> <total_bytes> <peak_bytes>"
# parsed from the dhat stderr summary of the instrumented build.
build_and_run() {
    local label="$1" srcdir="$2" wname="$3"
    local tgt="$SCRATCH/target-$label"
    local wpath
    wpath="$(resolve_workload "$srcdir" "$wname")" || {
        echo "[err] workload $wname missing in $srcdir" >&2; return 3; }

    echo "[heap-diff] ($label) building --features dhat-heap in $srcdir" >&2
    ( cd "$srcdir" && CARGO_TARGET_DIR="$tgt" \
        cargo build --release -p omnilua-cli --features dhat-heap -q ) >&2

    local bin="$tgt/release/omnilua"
    [ -x "$bin" ] || { echo "[err] $label binary missing: $bin" >&2; return 3; }

    local rundir="$SCRATCH/run-$label"
    mkdir -p "$rundir"
    echo "[heap-diff] ($label) running $wname" >&2
    local stderr_out="$rundir/dhat.stderr"
    ( cd "$rundir" && "$bin" "$wpath" >/dev/null 2>"$stderr_out" ) || true

    # Parse "dhat: Total:  N bytes in M blocks" and "dhat: At t-gmax: P bytes ..".
    awk '
        /^dhat: Total:/    { gsub(/,/,""); total_b=$3; total_blk=$6 }
        /^dhat: At t-gmax:/{ gsub(/,/,""); peak_b=$4 }
        END {
            if (total_blk == "" || total_b == "" || peak_b == "")
                { print "PARSE_FAIL"; exit }
            printf "%s %s %s\n", total_blk, total_b, peak_b
        }
    ' "$stderr_out"
}

WT_SEQ=0
WT_PATH=""
# prepare_worktree <ref>: adds a detached worktree at <ref> and sets the global
# WT_PATH to its dir. It must NOT run in a $(...) command substitution — that
# would mutate WT_SEQ / WORKTREES in a throwaway subshell, so the counter would
# never advance (two refs would collide on the same path) and cleanup would
# leak the worktree. Returns the path via the global instead.
prepare_worktree() {
    local ref="$1"
    WT_SEQ=$((WT_SEQ + 1))
    WT_PATH="$SCRATCH/wt-${WT_SEQ}-$(echo "$ref" | tr '/:' '__')"
    git worktree add --detach "$WT_PATH" "$ref" >&2
    WORKTREES+=("$WT_PATH")
}

SHA_A="$(git rev-parse --short "$COMMIT_A")"
prepare_worktree "$COMMIT_A"
DIR_A="$WT_PATH"
RES_A="$(build_and_run A "$DIR_A" "$WORKLOAD")"

if [ "$COMMIT_B" = "__WORKTREE__" ]; then
    LABEL_B="working-tree"
    SHA_B="$(git rev-parse --short HEAD)+wt"
    DIR_B="$ROOT"
else
    LABEL_B="$COMMIT_B"
    SHA_B="$(git rev-parse --short "$COMMIT_B")"
    prepare_worktree "$COMMIT_B"
    DIR_B="$WT_PATH"
fi
RES_B="$(build_and_run B "$DIR_B" "$WORKLOAD")"

if [ "$RES_A" = "PARSE_FAIL" ] || [ "$RES_B" = "PARSE_FAIL" ]; then
    echo "[err] could not parse dhat summary (A='$RES_A' B='$RES_B')" >&2
    exit 4
fi

python3 - "$WORKLOAD" "$COMMIT_A" "$SHA_A" "$LABEL_B" "$SHA_B" "$RES_A" "$RES_B" <<'PY'
import sys

workload, ca, sha_a, cb, sha_b, res_a, res_b = sys.argv[1:8]
ablk, abytes, apeak = (int(x) for x in res_a.split())
bblk, bbytes, bpeak = (int(x) for x in res_b.split())

abpb = abytes / ablk if ablk else 0.0
bbpb = bbytes / bblk if bblk else 0.0


def fmt(v):
    return f"{v:,.2f}" if isinstance(v, float) else f"{v:,}"


def delta(a, b):
    d = b - a
    pct = (100.0 * d / a) if a else 0.0
    sign = "+" if d >= 0 else ""
    return f"{sign}{fmt(d)} ({sign}{pct:.2f}%)"


print(f"\nheap-diff: workload={workload}")
print(f"  A = {ca} ({sha_a})")
print(f"  B = {cb} ({sha_b})\n")

rows = [
    ("total allocations", ablk, bblk),
    ("total bytes", abytes, bbytes),
    ("bytes/block", abpb, bbpb),
    ("peak live bytes", apeak, bpeak),
]

w = 20
print(f"{'metric':<{w}}{'A':>18}{'B':>18}{'delta (B-A)':>26}")
print("-" * (w + 18 + 18 + 26))
for name, a, b in rows:
    print(f"{name:<{w}}{fmt(a):>18}{fmt(b):>18}{delta(a, b):>26}")
print()
PY
