#!/usr/bin/env bash
# compare_bins.sh — direct A/B timing for two Lua interpreter binaries.
#
# This is for packet validation when the question is "did this Rust change move
# the workload?" rather than "how far are we from reference C?". It runs the
# same harness workloads through both binaries, asserts byte-identical output,
# and reports paired statistics. It intentionally does not append ledger rows;
# use compare.sh for dashboard/history evidence.
#
# Measurement discipline (PERF_PUSH_SPEC.md P1.1-P1.4):
#   - Runs are INTERLEAVED pairs (A then B, N times) so thermal/clock drift
#     hits both binaries symmetrically.
#   - repeat-each defaults to AUTO: a calibration run sizes the repeat factor
#     so every measured sample is >= MIN_SAMPLE_S (default 0.5 s). Rows whose
#     repeated sample still lands under 0.2 s get verdict "short".
#   - Per workload we report best-of-N (headline, back-compat), the median of
#     per-pair ratios, the fraction of pairs where B beat A, and a machine
#     verdict: improved (median<=0.99 and frac>=0.7); regressed-minor
#     (consistent slowdown under the tolerance, default <1.03 — lands, but
#     must be tracked); regressed (median >= tolerance — fails --gate); else
#     inconclusive. --tolerance N tunes the band; --strict makes
#     regressed-minor fail too (use for release gates). Rationale: until the
#     instruction-count rig can arbitrate, 1-3% single-row blips are usually
#     code-layout displacement, and blocking a broad win on one is worse than
#     landing it with a tracked line item.
#   - Headers carry provenance: dirty-tree flag, diff sha256, binary sha256s.
#   - Raw per-pair samples land in a .raw.tsv sidecar for re-analysis.
#
# Usage:
#   bash harness/bench/compare_bins.sh --a /tmp/lua-rs-base --b target/release/lua-rs \
#     --label-a base --label-b candidate --runs 10 --workloads gc_pressure,binarytrees
#   bash harness/bench/compare_bins.sh --a ... --b ... --gate     # fail on material regression
#   bash harness/bench/compare_bins.sh --a ... --b ... --quick    # exploratory: 5 runs, 0.3s floor
#   --repeat-each N overrides auto calibration; --min-sample S tunes the floor;
#   --skip w1,w2 excludes workloads; --no-match skips the output-equality check.
#
# Latency notes: the output-match check and the repeat calibration are cached
# in results/.match-cache.tsv / .calib-cache.tsv keyed on binary sha256 +
# workload mtime, so repeat invocations of the same binary pair skip both.
# Workloads whose manifest.tsv notes contain LIVELOCK are auto-skipped unless
# explicitly named via --workloads.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOAD_DIR="$ROOT/harness/bench/workloads"
OUT_DIR="$ROOT/harness/bench/results"
mkdir -p "$OUT_DIR"

A_BIN=""
B_BIN=""
LABEL_A="a"
LABEL_B="b"
RUNS=10
WORKLOAD_FILTER=""
REPEAT_EACH="auto"
MIN_SAMPLE_S="${MIN_SAMPLE_S:-0.5}"
GATE=0

MEDIAN_IMPROVE=0.99
MEDIAN_REGRESS=1.01
FRAC_HI=0.7
FRAC_LO=0.3
SHORT_FLOOR_S=0.2
MAX_SAMPLE_S="${MAX_SAMPLE_S:-120}"
GATE_TOLERANCE="${GATE_TOLERANCE:-1.03}"
STRICT=0
NO_MATCH=0
SKIP_LIST=""
CALIB_CACHE="$OUT_DIR/.calib-cache.tsv"
MATCH_CACHE="$OUT_DIR/.match-cache.tsv"

while [ $# -gt 0 ]; do
    case "$1" in
        --a)           A_BIN="$2";           shift 2 ;;
        --b)           B_BIN="$2";           shift 2 ;;
        --label-a)     LABEL_A="$2";         shift 2 ;;
        --label-b)     LABEL_B="$2";         shift 2 ;;
        --runs)        RUNS="$2";            shift 2 ;;
        --workloads)   WORKLOAD_FILTER="$2"; shift 2 ;;
        --repeat-each) REPEAT_EACH="$2";     shift 2 ;;
        --min-sample)  MIN_SAMPLE_S="$2";    shift 2 ;;
        --tolerance)   GATE_TOLERANCE="$2";  shift 2 ;;
        --gate)        GATE=1;               shift ;;
        --strict)      GATE=1; STRICT=1;     shift ;;
        --quick)       RUNS=5; MIN_SAMPLE_S=0.3; shift ;;
        --no-match)    NO_MATCH=1;           shift ;;
        --skip)        SKIP_LIST="$2";       shift 2 ;;
        -h|--help)
            sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# //; s/^#//'
            exit 0 ;;
        *) echo "unknown flag: $1" >&2; exit 1 ;;
    esac
done

[ -n "$A_BIN" ] || { echo "[err] missing --a binary" >&2; exit 2; }
[ -n "$B_BIN" ] || { echo "[err] missing --b binary" >&2; exit 2; }
[ -x "$A_BIN" ] || { echo "[err] --a binary not executable: $A_BIN" >&2; exit 2; }
[ -x "$B_BIN" ] || { echo "[err] --b binary not executable: $B_BIN" >&2; exit 2; }
case "$RUNS" in
    ''|*[!0-9]*) echo "[err] --runs must be a positive integer" >&2; exit 2 ;;
    0)           echo "[err] --runs must be >= 1" >&2; exit 2 ;;
esac
if [ "$REPEAT_EACH" != "auto" ]; then
    case "$REPEAT_EACH" in
        ''|*[!0-9]*) echo "[err] --repeat-each must be 'auto' or a positive integer" >&2; exit 2 ;;
        0)           echo "[err] --repeat-each must be >= 1" >&2; exit 2 ;;
    esac
fi

# Self-guard: refuse to measure while another measurement process is live
# (PERF_PUSH_SPEC P1.7). BENCH_IGNORE_RUNNING=1 overrides.
if [ "${BENCH_IGNORE_RUNNING:-0}" != "1" ]; then
    others=$(pgrep -f 'compare_bins|harness/bench/compare\.sh|profile-hotspots|callgrind' 2>/dev/null | grep -v -e "^$$\$" -e "^$PPID\$" || true)
    if [ -n "$others" ]; then
        echo "[err] another measurement process appears to be running (pids: $(echo "$others" | tr '\n' ' '))." >&2
        echo "[err] one measurement process at a time; set BENCH_IGNORE_RUNNING=1 to override." >&2
        exit 2
    fi
fi

# Hold the perf-experiment marker for the lifetime of the run so a Stop-hook
# firing mid-measurement neither contends for CPU nor auto-commits
# (PERF_PUSH_SPEC.md P7.4).
PERF_MARKER="$ROOT/harness/.perf-experiment"
touch "$PERF_MARKER"
trap 'rm -f "$PERF_MARKER"' EXIT

TS=$(date -u +%Y%m%dT%H%M%SZ)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
TSV="$OUT_DIR/${TS}-${COMMIT}-bin-ab.tsv"
JSON="$OUT_DIR/${TS}-${COMMIT}-bin-ab.json"
RAW="$OUT_DIR/${TS}-${COMMIT}-bin-ab.raw.tsv"

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
A_SHA=$(sha12 "$A_BIN")
B_SHA=$(sha12 "$B_BIN")

run_out() {
    local bin="$1"
    local workload="$2"
    perl -e 'alarm shift @ARGV; exec @ARGV' "$MAX_SAMPLE_S" "$bin" "$workload" 2>&1
}

# Every sample runs under a hard watchdog (perl alarm — portable on macOS,
# which has no `timeout`). A workload that hangs gets verdict "hang" instead
# of wedging the bench host (the 2026-06-09 table_ops repeated-dofile
# livelock burned 27 minutes at 100% CPU before this existed).
measure_one() {
    local bin="$1"
    local workload="$2"
    local repeat="$3"
    local eval_src=""
    local tmp real rss parsed rss_kb
    tmp=$(mktemp)
    case "$(uname -s)" in
        Darwin)
            if [ "$repeat" -le 1 ]; then
                perl -e 'alarm shift @ARGV; exec @ARGV' "$MAX_SAMPLE_S" \
                    /usr/bin/time -lp "$bin" "$workload" >/dev/null 2>"$tmp"
            else
                eval_src="for __bench_i = 1, $repeat do dofile([[$workload]]) end"
                perl -e 'alarm shift @ARGV; exec @ARGV' "$MAX_SAMPLE_S" \
                    /usr/bin/time -lp "$bin" -e "$eval_src" >/dev/null 2>"$tmp"
            fi
            real=$(awk '$1=="real" {print $2; exit}' "$tmp")
            rss=$(awk '/maximum resident set size/ {print $1; exit}' "$tmp")
            ;;
        *)
            if [ "$repeat" -le 1 ]; then
                perl -e 'alarm shift @ARGV; exec @ARGV' "$MAX_SAMPLE_S" \
                    /usr/bin/time -f '%e %M' "$bin" "$workload" >/dev/null 2>"$tmp"
            else
                eval_src="for __bench_i = 1, $repeat do dofile([[$workload]]) end"
                perl -e 'alarm shift @ARGV; exec @ARGV' "$MAX_SAMPLE_S" \
                    /usr/bin/time -f '%e %M' "$bin" -e "$eval_src" >/dev/null 2>"$tmp"
            fi
            parsed=$(awk '/^[0-9.]+ [0-9]+$/ {r=$1; k=$2} END {if (r != "") print r, k}' "$tmp")
            real=$(printf '%s' "$parsed" | awk '{print $1}')
            rss_kb=$(printf '%s' "$parsed" | awk '{print $2}')
            [ -n "$rss_kb" ] && rss=$((rss_kb * 1024))
            ;;
    esac
    rm -f "$tmp"
    if [ -z "${real:-}" ] || [ -z "${rss:-}" ]; then
        echo "[err] sample failed or timed out (>${MAX_SAMPLE_S}s) for $bin $workload" >&2
        return 1
    fi
    printf "%s %s\n" "$real" "$rss"
}

{
    printf '# lua-rs binary A/B compare\n'
    printf '# timestamp_utc: %s\n' "$TS"
    printf '# commit:        %s\n' "$COMMIT"
    printf '# dirty:         %s\n' "$DIRTY"
    printf '# diff_sha256:   %s\n' "$DIFF_SHA"
    printf '# os:            %s\n' "$OS_NAME"
    printf '# arch:          %s\n' "$ARCH"
    printf '# cpu:           %s\n' "$CPU"
    printf '# runs:          %d interleaved A/B pairs per workload\n' "$RUNS"
    printf '# repeat_each:   %s (min_sample_s %s)\n' "$REPEAT_EACH" "$MIN_SAMPLE_S"
    printf '# %s: %s (sha256 %s)\n' "$LABEL_A" "$A_BIN" "$A_SHA"
    printf '# %s: %s (sha256 %s)\n' "$LABEL_B" "$B_BIN" "$B_SHA"
    printf '# verdict rule:  improved = median<=%s and frac>=%s; regressed-minor = median in [%s,%s) and frac<=%s; regressed = median>=%s and frac<=%s\n' \
        "$MEDIAN_IMPROVE" "$FRAC_HI" "$MEDIAN_REGRESS" "$GATE_TOLERANCE" "$FRAC_LO" "$GATE_TOLERANCE" "$FRAC_LO"
    printf '# gate policy:   fail on regressed%s; regressed-minor lands but must be tracked\n' \
        "$([ "$STRICT" = "1" ] && echo ' or regressed-minor (--strict)')"
    printf '#\n'
    printf 'workload\t%s_wall_s\t%s_wall_s\t%s_over_%s_wall_ratio\tmedian_pair_ratio\tfrac_%s_faster\trepeat\t%s_rss_kb\t%s_rss_kb\t%s_over_%s_rss_ratio\tmatch\tverdict\n' \
        "$LABEL_A" "$LABEL_B" "$LABEL_B" "$LABEL_A" "$LABEL_B" "$LABEL_A" "$LABEL_B" "$LABEL_B" "$LABEL_A"
} > "$TSV"

printf 'workload\tpair\t%s_wall_s\t%s_wall_s\tpair_ratio\n' "$LABEL_A" "$LABEL_B" > "$RAW"

JSON_ROWS=""
TOTAL_A=0
TOTAL_B=0
ANY_REGRESSED=0
ANY_MINOR=0

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
    if [ -n "$SKIP_LIST" ] && echo ",$SKIP_LIST," | grep -q ",$wname,"; then
        echo "==> $wname SKIPPED (--skip)" >&2
        continue
    fi

    echo "==> $wname" >&2
    wl_mtime=$(stat -f %m "$wpath" 2>/dev/null || stat -c %Y "$wpath" 2>/dev/null)
    match="ok"
    if [ "$NO_MATCH" = "1" ]; then
        match="skipped"
    elif grep -q "^${A_SHA}	${B_SHA}	${wname}	${wl_mtime}\$" "$MATCH_CACHE" 2>/dev/null; then
        match="cached"
    else
        out_a=$(run_out "$A_BIN" "$wpath") || out_a="__RUN_A_FAILED__"
        out_b=$(run_out "$B_BIN" "$wpath") || out_b="__RUN_B_FAILED__"
        if [ "$out_a" = "__RUN_A_FAILED__" ] || [ "$out_b" = "__RUN_B_FAILED__" ]; then
            match="hang"
        elif [ "$out_a" = "$out_b" ]; then
            printf '%s\t%s\t%s\t%s\n' "$A_SHA" "$B_SHA" "$wname" "$wl_mtime" >> "$MATCH_CACHE"
        else
            match="diff"
        fi
    fi

    hang=0
    repeat=1
    if [ "$REPEAT_EACH" = "auto" ]; then
        cached_repeat=$(awk -F'\t' -v k1="$A_SHA" -v k2="$wname" -v k3="$wl_mtime" -v k4="$MIN_SAMPLE_S" \
            '$1==k1 && $2==k2 && $3==k3 && $4==k4 {print $5; exit}' "$CALIB_CACHE" 2>/dev/null || true)
        if [ -n "$cached_repeat" ]; then
            repeat="$cached_repeat"
        elif cal=$(measure_one "$A_BIN" "$wpath" 1); then
            cal_wall=$(echo "$cal" | awk '{print $1}')
            repeat=$(awk -v w="$cal_wall" -v m="$MIN_SAMPLE_S" 'BEGIN{
                if (w < 0.001) w = 0.001
                r = (w >= m) ? 1 : int(m / w) + 1
                if (r > 2000) r = 2000
                print r
            }')
            [ "$repeat" -gt 1 ] && echo "    calibrated repeat_each=$repeat (single run ${cal_wall}s)" >&2
            printf '%s\t%s\t%s\t%s\t%s\n' "$A_SHA" "$wname" "$wl_mtime" "$MIN_SAMPLE_S" "$repeat" >> "$CALIB_CACHE"
        else
            hang=1
        fi
    else
        repeat="$REPEAT_EACH"
    fi

    samples=$(mktemp)
    if [ "$hang" = "0" ]; then
        for i in $(seq 1 "$RUNS"); do
            pa=$(measure_one "$A_BIN" "$wpath" "$repeat") || { hang=1; break; }
            pb=$(measure_one "$B_BIN" "$wpath" "$repeat") || { hang=1; break; }
            aw=$(echo "$pa" | awk '{print $1}'); ar=$(echo "$pa" | awk '{print $2}')
            bw=$(echo "$pb" | awk '{print $1}'); br=$(echo "$pb" | awk '{print $2}')
            printf '%s %s %s %s\n' "$aw" "$bw" "$ar" "$br" >> "$samples"
            pr=$(awk -v a="$aw" -v b="$bw" 'BEGIN{if (a>0) printf "%.4f", b/a; else print "NaN"}')
            printf '%s\t%d\t%s\t%s\t%s\n' "$wname" "$i" "$aw" "$bw" "$pr" >> "$RAW"
        done
    fi

    if [ "$hang" = "1" ]; then
        rm -f "$samples"
        echo "    HANG: sample exceeded ${MAX_SAMPLE_S}s; recording verdict=hang" >&2
        printf '%s\tNaN\tNaN\tNaN\tNaN\tNaN\t%d\t0\t0\tNaN\thang\thang\n' "$wname" "$repeat" >> "$TSV"
        if [ -n "$JSON_ROWS" ]; then JSON_ROWS="$JSON_ROWS,"; fi
        JSON_ROWS="$JSON_ROWS{\"workload\":\"$wname\",\"verdict\":\"hang\",\"repeat_each\":$repeat,\"max_sample_s\":$MAX_SAMPLE_S}"
        continue
    fi

    stats=$(awk '
        { aw[NR]=$1; bw[NR]=$2; ar[NR]=$3; br[NR]=$4
          r[NR] = (aw[NR] > 0) ? bw[NR]/aw[NR] : 1e9
          if (r[NR] < 1.0) faster++
          if (best_a == "" || aw[NR] < best_a) best_a = aw[NR]
          if (best_b == "" || bw[NR] < best_b) best_b = bw[NR]
          if (max_ar == "" || ar[NR] > max_ar) max_ar = ar[NR]
          if (max_br == "" || br[NR] > max_br) max_br = br[NR]
        }
        END {
          n = NR
          for (i = 1; i <= n; i++)
            for (j = i + 1; j <= n; j++)
              if (r[j] < r[i]) { t = r[i]; r[i] = r[j]; r[j] = t }
          if (n % 2) median = r[(n + 1) / 2]
          else median = (r[n / 2] + r[n / 2 + 1]) / 2
          printf "%s %s %.3f %.3f %.2f %.0f %.0f\n", best_a, best_b,
            (best_a > 0) ? best_b / best_a : 0, median,
            (n > 0) ? faster / n : 0, max_ar / 1024, max_br / 1024
        }' "$samples")
    rm -f "$samples"

    a_wall=$(echo "$stats" | awk '{print $1}')
    b_wall=$(echo "$stats" | awk '{print $2}')
    wall_ratio=$(echo "$stats" | awk '{print $3}')
    median_ratio=$(echo "$stats" | awk '{print $4}')
    frac_faster=$(echo "$stats" | awk '{print $5}')
    a_rss_kb=$(echo "$stats" | awk '{print $6}')
    b_rss_kb=$(echo "$stats" | awk '{print $7}')
    rss_ratio=$(awk -v a="$b_rss_kb" -v b="$a_rss_kb" 'BEGIN{if (b>0) printf "%.3f", a/b; else print "NaN"}')

    verdict=$(awk -v m="$median_ratio" -v f="$frac_faster" -v aw="$a_wall" \
                  -v mi="$MEDIAN_IMPROVE" -v mr="$MEDIAN_REGRESS" \
                  -v fh="$FRAC_HI" -v fl="$FRAC_LO" -v sf="$SHORT_FLOOR_S" \
                  -v tol="$GATE_TOLERANCE" 'BEGIN{
        if (aw < sf)              { print "short";        exit }
        if (m <= mi && f >= fh)   { print "improved";     exit }
        if (m >= mr && f <= fl) {
            if (m >= tol)         { print "regressed";    exit }
            print "regressed-minor"; exit
        }
        print "inconclusive"
    }')
    [ "$verdict" = "regressed" ] && ANY_REGRESSED=1
    [ "$verdict" = "regressed-minor" ] && ANY_MINOR=1

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%d\t%s\t%s\t%s\t%s\t%s\n' \
        "$wname" "$a_wall" "$b_wall" "$wall_ratio" "$median_ratio" "$frac_faster" \
        "$repeat" "$a_rss_kb" "$b_rss_kb" "$rss_ratio" "$match" "$verdict" >> "$TSV"

    if [ -n "$JSON_ROWS" ]; then JSON_ROWS="$JSON_ROWS,"; fi
    JSON_ROWS="$JSON_ROWS{\"workload\":\"$wname\",\"${LABEL_A}_wall_s\":$a_wall,\"${LABEL_B}_wall_s\":$b_wall,\"${LABEL_B}_over_${LABEL_A}_wall_ratio\":$wall_ratio,\"median_pair_ratio\":$median_ratio,\"frac_${LABEL_B}_faster\":$frac_faster,\"repeat_each\":$repeat,\"${LABEL_A}_rss_kb\":$a_rss_kb,\"${LABEL_B}_rss_kb\":$b_rss_kb,\"${LABEL_B}_over_${LABEL_A}_rss_ratio\":$rss_ratio,\"match\":\"$match\",\"verdict\":\"$verdict\"}"

    TOTAL_A=$(awk -v t="$TOTAL_A" -v a="$a_wall" 'BEGIN{printf "%.4f", t+a}')
    TOTAL_B=$(awk -v t="$TOTAL_B" -v a="$b_wall" 'BEGIN{printf "%.4f", t+a}')
done

OVERALL_RATIO=$(awk -v a="$TOTAL_B" -v b="$TOTAL_A" 'BEGIN{if (b>0) printf "%.3f", a/b; else print "NaN"}')

{
    printf '{\n'
    printf '  "timestamp_utc": "%s",\n' "$TS"
    printf '  "commit": "%s",\n' "$COMMIT"
    printf '  "dirty": "%s", "diff_sha256": "%s",\n' "$DIRTY" "$DIFF_SHA"
    printf '  "os": "%s", "arch": "%s", "cpu": "%s",\n' "$OS_NAME" "$ARCH" "$CPU"
    printf '  "runs_per_workload": %d,\n' "$RUNS"
    printf '  "repeat_each": "%s", "min_sample_s": %s,\n' "$REPEAT_EACH" "$MIN_SAMPLE_S"
    printf '  "labels": {"a": "%s", "b": "%s"},\n' "$LABEL_A" "$LABEL_B"
    printf '  "binaries": {"%s": {"path": "%s", "sha256": "%s"}, "%s": {"path": "%s", "sha256": "%s"}},\n' \
        "$LABEL_A" "$A_BIN" "$A_SHA" "$LABEL_B" "$B_BIN" "$B_SHA"
    printf '  "totals": {"%s_wall_s": %s, "%s_wall_s": %s, "%s_over_%s_wall_ratio": %s},\n' \
        "$LABEL_A" "$TOTAL_A" "$LABEL_B" "$TOTAL_B" "$LABEL_B" "$LABEL_A" "$OVERALL_RATIO"
    printf '  "any_regressed": %s, "any_regressed_minor": %s, "gate_tolerance": %s, "strict": %s,\n' \
        "$ANY_REGRESSED" "$ANY_MINOR" "$GATE_TOLERANCE" "$STRICT"
    printf '  "rows": [%s]\n' "$JSON_ROWS"
    printf '}\n'
} > "$JSON"

echo >&2
echo "==> results:" >&2
echo "    tsv:  $TSV" >&2
echo "    raw:  $RAW" >&2
echo "    json: $JSON" >&2
echo >&2
cat "$TSV"

if [ "$ANY_MINOR" = "1" ]; then
    echo >&2
    echo "[gate] note: regressed-minor row(s) within tolerance (<${GATE_TOLERANCE}x);" >&2
    echo "[gate] these land but must be tracked (task or registry note), and arbitrated" >&2
    echo "[gate] by instruction counts once the instr-count rig exists." >&2
fi
if [ "$GATE" = "1" ] && [ "$ANY_REGRESSED" = "1" ]; then
    echo >&2
    echo "[gate] FAIL: at least one workload materially regressed (median >= ${GATE_TOLERANCE}x)" >&2
    exit 3
fi
if [ "$GATE" = "1" ] && [ "$STRICT" = "1" ] && [ "$ANY_MINOR" = "1" ]; then
    echo >&2
    echo "[gate] FAIL (--strict): regressed-minor row(s) present" >&2
    exit 3
fi
