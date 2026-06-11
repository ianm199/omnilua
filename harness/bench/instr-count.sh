#!/usr/bin/env bash
# instr-count.sh — deterministic instruction counts via cachegrind in a Linux
# container (PERF_PUSH_SPEC.md P2.1). The arbiter for small wall-clock deltas:
# wall_ratio = Ir_ratio x CPI_ratio, and this measures the Ir factor with
# ~0.1% stability, immune to thermal/scheduler/layout noise.
#
# Builds lua-rs (current tree) and the reference C interpreter inside the
# container (cargo/reference caches persist in a named volume), then runs
# cachegrind per workload per binary. Emits a TSV under results/ plus, when
# the workload manifest has an iteration count, per-iteration budgets
# (Ir minus the startup_empty constant, divided by iterations).
#
# --branch-sim: the CPI arbiter. macOS/arm64 wall time is layout-entangled —
# a call-free control once moved ~12% on pure code layout while Ir moved only
# +0.55%, so a same-Ir wall swing cannot be attributed from wall time alone.
# This flag adds valgrind --branch-sim=yes and reports deterministic simulated
# conditional-branch counts (Bc) and mispredicts (Bcm), plus indirect-branch
# (Bi) and indirect-mispredict (Bim), per workload alongside Ir. Bcm is the
# only LOCAL, reproducible way to settle "was the wall swing the branch
# predictor?" — the question the 2026-06-11 T2-C2 packet had no tool for.
# The branch model is valgrind's deterministic 2-bit saturating-counter
# simulation, not the host CPU's predictor, so counts reproduce across rigs.
# WITHOUT the flag, output (stdout table + TSV columns) is byte-identical to
# before; --branch-sim only adds columns, it never changes the Ir column.
#
# Usage:
#   bash harness/bench/instr-count.sh --workloads concat_chain,fibonacci
#   bash harness/bench/instr-count.sh --workloads table_seti_same --label myexp
#   bash harness/bench/instr-count.sh --branch-sim --workloads fibonacci
#   bash harness/bench/instr-count.sh --dir /tmp/myprobes --workloads short_gc
#     (--dir mounts a host directory of ad-hoc .lua files — e.g. iteration-
#      scaled copies of slow workloads, so a relative-delta recount takes
#      seconds instead of minutes; budgets need manifest rows, deltas don't)
#
# Expect ~20-100x slowdown under cachegrind (a bit more with --branch-sim);
# target packet-sized workload lists, not the full matrix. First run builds
# the container toolchain (several minutes); later runs are incremental.
#
# Note: this script and run-inside.sh use cachegrind (--tool=cachegrind
# --cache-sim=no), not callgrind — callgrind's call-graph tracking OOMs the
# ~8GB docker VM (exit 137). Cachegrind gives the same deterministic Ir total
# and supports --branch-sim. A prior agent noted that invoking run-inside.sh
# directly inside the container works when the docker wrapper here misbehaves.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOADS="startup_empty"
LABEL="instr"
EXTRA_DIR=""
BRANCH_SIM="no"
# `--features` is a NEVER-MERGES addition from the T4 safety-tax ablation branch
# (ablation/unchecked-stack). It forwards a cargo feature list into the
# in-container `cargo build -p lua-cli` so the ablated builds can be Ir-measured
# against the default build on the same toolchain. On main this flag does not
# exist; the default value "" reproduces byte-identical behavior to before.
FEATURES=""
while [ $# -gt 0 ]; do
    case "$1" in
        --workloads)  WORKLOADS="startup_empty,$2"; shift 2 ;;
        --label)      LABEL="$2";                   shift 2 ;;
        --dir)        EXTRA_DIR="$2";               shift 2 ;;
        --branch-sim) BRANCH_SIM="yes";             shift 1 ;;
        --features)   FEATURES="$2";                shift 2 ;;
        -h|--help)
            sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# //; s/^#//'
            exit 0 ;;
        *) echo "unknown flag: $1" >&2; exit 1 ;;
    esac
done

command -v docker >/dev/null || { echo "[err] docker required" >&2; exit 2; }

# Hold the perf-experiment marker (ownership-aware) so a Stop event during a
# long container run cannot auto-commit a dirty experiment tree — the exact
# incident that swept the gate-blocked intern fix on 2026-06-09 (73ef169).
PERF_MARKER="$ROOT/harness/.perf-experiment"
if [ -f "$PERF_MARKER" ]; then
    PERF_MARKER_OWNED=0
else
    PERF_MARKER_OWNED=1
    touch "$PERF_MARKER"
fi
trap '[ "$PERF_MARKER_OWNED" = "1" ] && rm -f "$PERF_MARKER"' EXIT

IMG=lua-rs-instr
VOL=lua-rs-instr-cache
OUT_DIR="$ROOT/harness/bench/results"
mkdir -p "$OUT_DIR"
TS=$(date -u +%Y%m%dT%H%M%SZ)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
TSV="$OUT_DIR/${TS}-${COMMIT}-${LABEL}.tsv"

docker build -q -t "$IMG" "$ROOT/harness/bench/instr" >/dev/null
docker volume create "$VOL" >/dev/null

DIRTY="no"
[ -n "$(git status --porcelain 2>/dev/null)" ] && DIRTY="yes"

{
    printf '# lua-rs instruction counts (callgrind, linux container)\n'
    printf '# timestamp_utc: %s\n# commit: %s\n# dirty: %s\n# workloads: %s\n' \
        "$TS" "$COMMIT" "$DIRTY" "$WORKLOADS"
    if [ "$BRANCH_SIM" = "yes" ]; then
        printf '# branch_sim: yes\n'
        printf 'workload\tbinary\tIr\tBc\tBcm\tBi\tBim\n'
    else
        printf 'workload\tbinary\tIr\n'
    fi
} > "$TSV"

EXTRA_MOUNT=()
if [ -n "$EXTRA_DIR" ]; then
    EXTRA_MOUNT=(-v "$EXTRA_DIR":/extra:ro)
fi
docker run --rm \
    -v "$ROOT":/src:ro \
    -v "$VOL":/cache \
    ${EXTRA_MOUNT[@]+"${EXTRA_MOUNT[@]}"} \
    "$IMG" bash /src/harness/bench/instr/run-inside.sh "$WORKLOADS" "$BRANCH_SIM" "$FEATURES" >> "$TSV"

echo "==> $TSV" >&2
column -t -s$'\t' "$TSV"

python3 - "$TSV" "$ROOT/harness/bench/workloads/manifest.tsv" <<'PY'
import sys

tsv, manifest = sys.argv[1], sys.argv[2]
iters = {}
for line in open(manifest):
    parts = line.rstrip("\n").split("\t")
    if len(parts) >= 3 and parts[2].isdigit():
        iters[parts[0]] = int(parts[2])

# Parse the TSV column-by-column. The value columns are everything after
# workload+binary: in default mode that is just [Ir]; in --branch-sim mode it
# is [Ir, Bc, Bcm, Bi, Bim]. Reading the header keeps the budget code one path.
header = []
counts = {}
for line in open(tsv):
    if line.startswith("#"):
        continue
    fields = line.rstrip("\n").split("\t")
    if line.startswith("workload"):
        header = fields[2:]
        continue
    w, b = fields[0], fields[1]
    vals = fields[2:]
    counts[(w, b)] = {name: int(v) for name, v in zip(header, vals) if v != ""}


def budget_rows(metric):
    """Per-iteration (value minus startup, / iterations) for each rs workload
    that has a manifest iteration count and a matching ref measurement."""
    base = {b: counts.get(("startup_empty", b), {}).get(metric) for b in ("ref", "rs")}
    out = []
    for (w, b), vals in sorted(counts.items()):
        if w == "startup_empty" or b != "rs" or metric not in vals:
            continue
        ref_vals = counts.get((w, "ref"), {})
        if w in iters and base["rs"] is not None and base["ref"] is not None and metric in ref_vals:
            n = iters[w]
            rs_per = (vals[metric] - base["rs"]) / n
            ref_per = (ref_vals[metric] - base["ref"]) / n
            out.append((w, ref_per, rs_per, rs_per / ref_per if ref_per else 0))
    return out


rows = budget_rows("Ir")
if rows:
    print("\nper-iteration instruction budgets (Ir minus startup, / iterations):")
    print(f"{'workload':<28}{'C Ir/iter':>12}{'rs Ir/iter':>12}{'Ir ratio':>10}")
    for w, a, b, r in rows:
        print(f"{w:<28}{a:>12.1f}{b:>12.1f}{r:>10.2f}")

for metric, label in (("Bc", "conditional branches"), ("Bcm", "branch mispredicts")):
    if metric not in header:
        continue
    rows = budget_rows(metric)
    if rows:
        print(f"\nper-iteration {metric} budgets ({label}, {metric} minus startup, / iterations):")
        print(f"{'workload':<28}{'C '+metric+'/iter':>12}{'rs '+metric+'/iter':>12}{metric+' ratio':>10}")
        for w, a, b, r in rows:
            print(f"{w:<28}{a:>12.1f}{b:>12.1f}{r:>10.2f}")
PY
