#!/usr/bin/env bash
# instr-count.sh — deterministic instruction counts via callgrind in a Linux
# container (PERF_PUSH_SPEC.md P2.1). The arbiter for small wall-clock deltas:
# wall_ratio = Ir_ratio x CPI_ratio, and this measures the Ir factor with
# ~0.1% stability, immune to thermal/scheduler/layout noise.
#
# Builds lua-rs (current tree) and the reference C interpreter inside the
# container (cargo/reference caches persist in a named volume), then runs
# callgrind per workload per binary. Emits a TSV under results/ plus, when
# the workload manifest has an iteration count, per-iteration budgets
# (Ir minus the startup_empty constant, divided by iterations).
#
# Usage:
#   bash harness/bench/instr-count.sh --workloads concat_chain,fibonacci
#   bash harness/bench/instr-count.sh --workloads table_seti_same --label myexp
#   bash harness/bench/instr-count.sh --dir /tmp/myprobes --workloads short_gc
#     (--dir mounts a host directory of ad-hoc .lua files — e.g. iteration-
#      scaled copies of slow workloads, so a relative-delta recount takes
#      seconds instead of minutes; budgets need manifest rows, deltas don't)
#
# Expect ~20-100x slowdown under callgrind; target packet-sized workload
# lists, not the full matrix. First run builds the container toolchain
# (several minutes); later runs are incremental.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOADS="startup_empty"
LABEL="instr"
EXTRA_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --workloads) WORKLOADS="startup_empty,$2"; shift 2 ;;
        --label)     LABEL="$2";                   shift 2 ;;
        --dir)       EXTRA_DIR="$2";               shift 2 ;;
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
    printf 'workload\tbinary\tIr\n'
} > "$TSV"

EXTRA_MOUNT=()
if [ -n "$EXTRA_DIR" ]; then
    EXTRA_MOUNT=(-v "$EXTRA_DIR":/extra:ro)
fi
docker run --rm \
    -v "$ROOT":/src:ro \
    -v "$VOL":/cache \
    "${EXTRA_MOUNT[@]}" \
    "$IMG" bash /src/harness/bench/instr/run-inside.sh "$WORKLOADS" >> "$TSV"

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

ir = {}
for line in open(tsv):
    if line.startswith("#") or line.startswith("workload"):
        continue
    w, b, v = line.rstrip("\n").split("\t")
    if v:
        ir[(w, b)] = int(v)

base = {b: ir.get(("startup_empty", b)) for b in ("ref", "rs")}
rows = []
for (w, b), v in sorted(ir.items()):
    if w == "startup_empty" or b != "rs":
        continue
    if w in iters and base["rs"] and base["ref"] and ("%s" % w, "ref") in [(k[0], k[1]) for k in ir]:
        n = iters[w]
        rs_per = (v - base["rs"]) / n
        ref_per = (ir[(w, "ref")] - base["ref"]) / n
        rows.append((w, ref_per, rs_per, rs_per / ref_per if ref_per else 0))

if rows:
    print("\nper-iteration instruction budgets (Ir minus startup, / iterations):")
    print(f"{'workload':<28}{'C Ir/iter':>12}{'rs Ir/iter':>12}{'Ir ratio':>10}")
    for w, a, b, r in rows:
        print(f"{w:<28}{a:>12.1f}{b:>12.1f}{r:>10.2f}")
PY
