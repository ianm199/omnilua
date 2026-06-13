#!/usr/bin/env bash
# harness/bench/compare.sh — side-by-side throughput benchmark between
# lua-rs (Rust impl) and the pinned upstream Lua 5.4.7 reference binary.
#
# For each workload in harness/bench/workloads/, both binaries run N times
# (default 5) as INTERLEAVED ref/rs pairs so thermal and clock drift hit both
# sides symmetrically. We record min wall-clock + max RSS for each plus the
# median of per-pair ratios, then emit a TSV with the ratio (lua-rs /
# reference). Min wall-clock stays the headline (dashboard continuity);
# median_ratio is the robustness check.
#
# Short workloads are repeat-calibrated (PERF_PUSH_SPEC P1.3): a calibration
# run sizes a dofile-repeat factor so every measured sample is >= MIN_SAMPLE_S
# (default 0.5s), killing centisecond quantization. Calibration is cached in
# results/.calib-cache.tsv keyed on binary sha256 + workload mtime.
# Workloads whose manifest notes contain LIVELOCK are skipped unless named
# via --workloads; a hung sample records status=hang instead of wedging.
#
# Usage:
#   bash harness/bench/compare.sh                       # all workloads, N=5
#   bash harness/bench/compare.sh --runs 3              # fewer repetitions
#   bash harness/bench/compare.sh --workloads fib,table # subset
#   bash harness/bench/compare.sh --min-sample 0.3      # lower repeat floor
#
# Output:
#   harness/bench/results/<UTC-timestamp>-<short-sha>-compare.tsv
#   harness/bench/results/<UTC-timestamp>-<short-sha>-compare.json
#
# Reproducibility: hardware + OS + commit fingerprint in the TSV header so
# results from different machines do not get silently merged.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

REF_BIN="$ROOT/reference/lua-5.4.7/src/lua"
RS_BIN="$ROOT/target/release/omnilua"
WORKLOAD_DIR="$ROOT/harness/bench/workloads"
OUT_DIR="$ROOT/harness/bench/results"
mkdir -p "$OUT_DIR"

RUNS=5
WORKLOAD_FILTER=""
MIN_SAMPLE_S="${MIN_SAMPLE_S:-0.5}"
CALIB_CACHE="$OUT_DIR/.calib-cache.tsv"
BENCH_VARIANT="${BENCH_VARIANT:-stock}"

# Hold the perf-experiment marker so a Stop-hook firing mid-run neither
# contends for CPU nor auto-commits (PERF_PUSH_SPEC P7.4).
PERF_MARKER="$ROOT/harness/.perf-experiment"
if [ -f "$PERF_MARKER" ]; then
    PERF_MARKER_OWNED=0
else
    PERF_MARKER_OWNED=1
    touch "$PERF_MARKER"
fi
trap '[ "$PERF_MARKER_OWNED" = "1" ] && rm -f "$PERF_MARKER"' EXIT

while [ $# -gt 0 ]; do
    case "$1" in
        --runs)       RUNS="$2";             shift 2 ;;
        --workloads)  WORKLOAD_FILTER="$2";  shift 2 ;;
        --min-sample) MIN_SAMPLE_S="$2";     shift 2 ;;
        -h|--help)
            sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# //; s/^#//'
            exit 0 ;;
        *) echo "unknown flag: $1" >&2; exit 1 ;;
    esac
done

[ -x "$REF_BIN" ] || { echo "[err] reference binary missing: $REF_BIN — run 'make macosx -C reference/lua-5.4.7' first" >&2; exit 2; }
[ -x "$RS_BIN" ]  || { echo "[err] lua-rs release binary missing: $RS_BIN — run 'cargo build --release -p omnilua-cli'" >&2; exit 2; }

TS=$(date -u +%Y%m%dT%H%M%SZ)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
TSV="$OUT_DIR/${TS}-${COMMIT}-compare.tsv"
JSON="$OUT_DIR/${TS}-${COMMIT}-compare.json"

OS_NAME="$(uname -sr)"
ARCH="$(uname -m)"
CPU="$(sysctl -n machdep.cpu.brand_string 2>/dev/null || grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- | sed 's/^ *//' || echo 'unknown')"

sha12() {
    shasum -a 256 "$1" 2>/dev/null | awk '{print substr($1, 1, 12)}'
}
DIRTY="no"
DIFF_SHA="-"
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
    DIRTY="yes"
    DIFF_SHA=$(git diff HEAD 2>/dev/null | shasum -a 256 | awk '{print substr($1, 1, 12)}')
fi
REF_SHA=$(sha12 "$REF_BIN")
RS_SHA=$(sha12 "$RS_BIN")

# macOS (BSD time) and Linux (GNU time) have incompatible flags and output
# formats, so measure_one branches on the OS:
#
#   macOS `/usr/bin/time -lp` prints (RSS in BYTES):
#     real         0.12
#     user         0.10
#          16777216  maximum resident set size
#
#   Linux `/usr/bin/time -f '%e %M'` prints one line to stderr (RSS in KB):
#     0.12 16384
#   which we normalize to bytes (KB * 1024) so the ledger stays byte-keyed
#   regardless of which runner produced the row.

# Per-sample watchdog (COMPARE_MAX_S, default 120s; perl alarm — macOS has no
# timeout(1)) so a hung workload cannot wedge the ledgered run.
COMPARE_MAX_S="${COMPARE_MAX_S:-120}"
measure_one() {
    local bin="$1"
    local workload="$2"
    local repeat="${3:-1}"
    local tmp eval_src=""
    tmp=$(mktemp)
    local real rss rss_kb parsed
    case "$(uname -s)" in
        Darwin)
            if [ "$repeat" -le 1 ]; then
                "$ROOT/harness/bench/with-timeout.sh" "$COMPARE_MAX_S" \
                    /usr/bin/time -lp "$bin" "$workload" >/dev/null 2>"$tmp"
            else
                eval_src="for __bench_i = 1, $repeat do dofile([[$workload]]) end"
                "$ROOT/harness/bench/with-timeout.sh" "$COMPARE_MAX_S" \
                    /usr/bin/time -lp "$bin" -e "$eval_src" >/dev/null 2>"$tmp"
            fi
            real=$(awk '$1=="real" {print $2}' "$tmp" | head -1)
            rss=$(awk '/maximum resident set size/ {print $1}' "$tmp" | head -1)
            ;;
        *)
            if [ "$repeat" -le 1 ]; then
                "$ROOT/harness/bench/with-timeout.sh" "$COMPARE_MAX_S" \
                    /usr/bin/time -f '%e %M' "$bin" "$workload" >/dev/null 2>"$tmp"
            else
                eval_src="for __bench_i = 1, $repeat do dofile([[$workload]]) end"
                "$ROOT/harness/bench/with-timeout.sh" "$COMPARE_MAX_S" \
                    /usr/bin/time -f '%e %M' "$bin" -e "$eval_src" >/dev/null 2>"$tmp"
            fi
            parsed=$(awk '/^[0-9.]+ [0-9]+$/ {r=$1; k=$2} END {if (r != "") print r, k}' "$tmp")
            real=$(printf '%s' "$parsed" | awk '{print $1}')
            rss_kb=$(printf '%s' "$parsed" | awk '{print $2}')
            [ -n "$rss_kb" ] && rss=$((rss_kb * 1024))
            ;;
    esac
    rm -f "$tmp"
    if [ -z "$real" ] || [ -z "$rss" ]; then
        echo "[err] sample failed or timed out (>${COMPARE_MAX_S}s) for $bin $workload" >&2
        return 1
    fi
    printf "%s %s\n" "$real" "$rss"
}

{
    printf '# lua-rs bench compare\n'
    printf '# timestamp_utc: %s\n' "$TS"
    printf '# commit:        %s\n' "$COMMIT"
    printf '# os:            %s\n' "$OS_NAME"
    printf '# arch:          %s\n' "$ARCH"
    printf '# cpu:           %s\n' "$CPU"
    printf '# runs:          %d interleaved ref/rs pairs (reporting best wall-clock, max RSS, median pair ratio)\n' "$RUNS"
    printf '# min_sample_s:  %s (auto repeat calibration)\n' "$MIN_SAMPLE_S"
    printf '# dirty:         %s\n' "$DIRTY"
    printf '# diff_sha256:   %s\n' "$DIFF_SHA"
    printf '# variant:       %s (non-stock ledger rows are excluded from the dashboard trend)\n' "$BENCH_VARIANT"
    printf '# reference:     %s (Lua 5.4.7, sha256 %s)\n' "$REF_BIN" "$REF_SHA"
    printf '# lua-rs:        %s (sha256 %s)\n' "$RS_BIN" "$RS_SHA"
    printf '#\n'
    printf 'workload\tref_wall_s\trs_wall_s\twall_ratio\tref_rss_kb\trs_rss_kb\trss_ratio\tmedian_ratio\trepeat\tstatus\n'
} > "$TSV"

JSON_ROWS=""
TOTAL_REF=0
TOTAL_RS=0

for wpath in "$WORKLOAD_DIR"/*.lua; do
    wname=$(basename "$wpath" .lua)
    if [ -n "$WORKLOAD_FILTER" ]; then
        echo ",$WORKLOAD_FILTER," | grep -q ",$wname," || continue
    else
        if grep -q "^${wname}	.*LIVELOCK" "$WORKLOAD_DIR/manifest.tsv" 2>/dev/null; then
            echo "==> $wname SKIPPED: manifest marks LIVELOCK (run via --workloads to retest)" >&2
            continue
        fi
    fi

    echo "==> $wname" >&2
    wl_mtime=$(stat -f %m "$wpath" 2>/dev/null || stat -c %Y "$wpath" 2>/dev/null)

    hang=0
    repeat=1
    cached_repeat=$(awk -F'\t' -v k1="$REF_SHA" -v k2="$wname" -v k3="$wl_mtime" -v k4="$MIN_SAMPLE_S" \
        '$1==k1 && $2==k2 && $3==k3 && $4==k4 {print $5; exit}' "$CALIB_CACHE" 2>/dev/null || true)
    if [ -n "$cached_repeat" ]; then
        repeat="$cached_repeat"
    elif cal=$(measure_one "$REF_BIN" "$wpath" 1); then
        cal_wall=$(echo "$cal" | awk '{print $1}')
        repeat=$(awk -v w="$cal_wall" -v m="$MIN_SAMPLE_S" 'BEGIN{
            if (w < 0.001) w = 0.001
            r = (w >= m) ? 1 : int(m / w) + 1
            if (r > 2000) r = 2000
            print r
        }')
        [ "$repeat" -gt 1 ] && echo "    calibrated repeat_each=$repeat (single run ${cal_wall}s)" >&2
        printf '%s\t%s\t%s\t%s\t%s\n' "$REF_SHA" "$wname" "$wl_mtime" "$MIN_SAMPLE_S" "$repeat" >> "$CALIB_CACHE"
    else
        hang=1
    fi

    samples=$(mktemp)
    if [ "$hang" = "0" ]; then
        for _ in $(seq 1 "$RUNS"); do
            pr=$(measure_one "$REF_BIN" "$wpath" "$repeat") || { hang=1; break; }
            ps=$(measure_one "$RS_BIN" "$wpath" "$repeat") || { hang=1; break; }
            printf '%s %s %s %s\n' \
                "$(echo "$pr" | awk '{print $1}')" "$(echo "$ps" | awk '{print $1}')" \
                "$(echo "$pr" | awk '{print $2}')" "$(echo "$ps" | awk '{print $2}')" >> "$samples"
        done
    fi

    if [ "$hang" = "1" ]; then
        rm -f "$samples"
        echo "    HANG: sample exceeded ${COMPARE_MAX_S}s; recording status=hang" >&2
        printf '%s\tNaN\tNaN\tNaN\t0\t0\tNaN\tNaN\t%d\thang\n' "$wname" "$repeat" >> "$TSV"
        if [ -n "$JSON_ROWS" ]; then JSON_ROWS="$JSON_ROWS,"; fi
        JSON_ROWS="$JSON_ROWS{\"workload\":\"$wname\",\"status\":\"hang\",\"repeat_each\":$repeat}"
        continue
    fi

    stats=$(awk '
        { rw[NR]=$1; sw[NR]=$2; rr[NR]=$3; sr[NR]=$4
          r[NR] = (rw[NR] > 0) ? sw[NR]/rw[NR] : 1e9
          if (best_r == "" || rw[NR] < best_r) best_r = rw[NR]
          if (best_s == "" || sw[NR] < best_s) best_s = sw[NR]
          if (max_rr == "" || rr[NR] > max_rr) max_rr = rr[NR]
          if (max_sr == "" || sr[NR] > max_sr) max_sr = sr[NR]
        }
        END {
          n = NR
          for (i = 1; i <= n; i++)
            for (j = i + 1; j <= n; j++)
              if (r[j] < r[i]) { t = r[i]; r[i] = r[j]; r[j] = t }
          if (n % 2) median = r[(n + 1) / 2]
          else median = (r[n / 2] + r[n / 2 + 1]) / 2
          printf "%s %s %.2f %.0f %.0f %.3f\n", best_r, best_s,
            (best_r > 0) ? best_s / best_r : 0, max_rr / 1024, max_sr / 1024, median
        }' "$samples")
    rm -f "$samples"

    ref_wall=$(echo "$stats" | awk '{print $1}')
    rs_wall=$(echo "$stats" | awk '{print $2}')
    wall_ratio=$(echo "$stats" | awk '{print $3}')
    ref_rss_kb=$(echo "$stats" | awk '{print $4}')
    rs_rss_kb=$(echo "$stats" | awk '{print $5}')
    median_ratio=$(echo "$stats" | awk '{print $6}')
    rss_ratio=$(awk -v a="$rs_rss_kb" -v b="$ref_rss_kb" 'BEGIN{if (b>0) printf "%.2f", a/b; else print "NaN"}')

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%d\tok\n' \
        "$wname" "$ref_wall" "$rs_wall" "$wall_ratio" "$ref_rss_kb" "$rs_rss_kb" "$rss_ratio" "$median_ratio" "$repeat" >> "$TSV"

    if [ -n "$JSON_ROWS" ]; then JSON_ROWS="$JSON_ROWS,"; fi
    JSON_ROWS="$JSON_ROWS{\"workload\":\"$wname\",\"ref_wall_s\":$ref_wall,\"rs_wall_s\":$rs_wall,\"wall_ratio\":$wall_ratio,\"ref_rss_kb\":$ref_rss_kb,\"rs_rss_kb\":$rs_rss_kb,\"rss_ratio\":$rss_ratio,\"median_wall_ratio\":$median_ratio,\"repeat_each\":$repeat,\"status\":\"ok\"}"

    TOTAL_REF=$(awk -v t="$TOTAL_REF" -v a="$ref_wall" 'BEGIN{printf "%.4f", t+a}')
    TOTAL_RS=$(awk  -v t="$TOTAL_RS"  -v a="$rs_wall"  'BEGIN{printf "%.4f", t+a}')
done

OVERALL_RATIO=$(awk -v a="$TOTAL_RS" -v b="$TOTAL_REF" 'BEGIN{if (b>0) printf "%.2f", a/b; else print "NaN"}')

{
    printf '{\n'
    printf '  "timestamp_utc": "%s",\n' "$TS"
    printf '  "commit": "%s",\n' "$COMMIT"
    printf '  "os": "%s", "arch": "%s", "cpu": "%s",\n' "$OS_NAME" "$ARCH" "$CPU"
    printf '  "runs_per_workload": %d,\n' "$RUNS"
    printf '  "totals": {"ref_wall_s": %s, "rs_wall_s": %s, "overall_wall_ratio": %s},\n' "$TOTAL_REF" "$TOTAL_RS" "$OVERALL_RATIO"
    printf '  "rows": [%s]\n' "$JSON_ROWS"
    printf '}\n'
} > "$JSON"

# Append rows to the chassis evidence ledger so the dashboard can plot perf
# over commits. One row per (workload, metric) pair. Schema matches the
# kind=bench/target=rust-vs-reference convention used in redis-rs-port's
# history.py.
LEDGER="$ROOT/harness/evidence/ledger.jsonl"
mkdir -p "$(dirname "$LEDGER")"
EVIDENCE_REL="harness/bench/results/$(basename "$JSON")"

# Use Python (already available, no jq dependency) to emit well-formed JSON
# Lines from the same JSON_ROWS string we just built.
python3 - "$JSON" "$COMMIT" "$TS" "$OS_NAME" "$ARCH" "$CPU" "$EVIDENCE_REL" "$RUNS" "$LEDGER" "$BENCH_VARIANT" <<'PY'
import json, sys
json_path, commit, ts, os_name, arch, cpu, evidence_rel, runs, ledger, variant = sys.argv[1:]
with open(json_path) as f:
    data = json.load(f)
# Parity threshold: workloads above this wall_ratio are considered "failing
# perf parity" and the chassis will dispatch test-fixer packets against them.
PARITY_THRESHOLD = 1.5
with open(ledger, "a") as out:
    for row in data["rows"]:
        if row.get("status", "ok") != "ok":
            continue
        if row["workload"] == "startup_empty":
            continue
        for metric in ("wall_ratio", "rss_ratio"):
            entry = {
                "schema_version": 1,
                "ts": ts,
                "commit": commit,
                "kind": "bench",
                "target": "rust-vs-reference",
                "metric": metric,
                "workload": row["workload"],
                "value": row[metric],
                "unit": "ratio",
                "evidence": evidence_rel,
                "runner": "bench-vs-reference",
                "runs": int(runs),
                "repeat_each": int(row.get("repeat_each", 1)),
                "variant": variant,
                "os": os_name,
                "arch": arch,
                "cpu": cpu,
            }
            out.write(json.dumps(entry, sort_keys=True) + "\n")
        # Oracle-style row so the chassis's failing_rust_vs_ref_fixtures
        # filter sees this workload as a failing target. Format mirrors
        # the test-suite oracle rows: kind=oracle, target=rust-vs-reference,
        # numerator < denominator when failing parity. The parity gate is
        # defined on stock builds only — variant runs skip oracle rows.
        if variant != "stock":
            continue
        passing = row["wall_ratio"] <= PARITY_THRESHOLD
        oracle = {
            "schema_version": 1,
            "ts": ts,
            "commit": commit,
            "kind": "oracle",
            "target": "rust-vs-reference",
            "metric": "perf_parity",
            "fixture": row["workload"],
            "capability": "lua-perf-" + row["workload"].replace("_", "-"),
            "numerator": 1 if passing else 0,
            "denominator": 1,
            "wall_ratio": row["wall_ratio"],
            "threshold": PARITY_THRESHOLD,
            "evidence": evidence_rel,
            "runner": "bench-vs-reference",
        }
        out.write(json.dumps(oracle, sort_keys=True) + "\n")
PY

echo "" >&2
echo "==> results:" >&2
echo "    tsv:    $TSV" >&2
echo "    json:   $JSON" >&2
echo "    ledger: $LEDGER (appended $(grep -c '"kind": "bench"' "$LEDGER") total bench rows)" >&2
echo "" >&2
column -t -s $'\t' "$TSV"
echo ""
printf "overall wall-clock ratio (lua-rs / reference): %s\n" "$OVERALL_RATIO"
