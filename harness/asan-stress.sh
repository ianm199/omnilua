#!/usr/bin/env bash
# asan-stress.sh — the exact-rooting battery (issue #140, docs/EXACT_ROOTING_SPEC.md P0).
#
# Hunts the use-after-sweep bug class: objects the VM still uses that the GC's
# root trace does not cover. Three instruments, layered cheap-to-expensive:
#
#   1. LUA_RS_GC_QUARANTINE=1 (debug build) — sweep parks dead boxes instead of
#      freeing; any later dereference panics with a backtrace. Cadence is
#      IDENTICAL to a normal run, so behavioral failures count as findings too.
#   2. + LUA_RS_GC_STRESS=1 — collect at every checkpoint. Cadence-dependent
#      assertions in tests legitimately fail under stress, so in stress configs
#      ONLY panic signatures count as findings, not ordinary test failures.
#   3. --asan — nightly AddressSanitizer build (cached per commit, bincache
#      pattern). The truth-teller for reads that bypass the poisoned headers.
#
# Usage:
#   harness/asan-stress.sh            # quick: canaries + repro set, configs 1+2
#   harness/asan-stress.sh --full     # + full official suite under quarantine
#   harness/asan-stress.sh --asan     # + ASAN build over the repro set
#
# Exit codes: 0 clean, 1 findings (evidence saved), 2 setup error.
#
# Known stress-expected failures (NOT findings): canaries f/l/m/n
# (testc_gengc_age, testc_minor_stats, testc_finalizer_cohorts,
# testc_weak_registry) assert exact generational bookkeeping that
# collect-at-every-checkpoint legitimately changes.

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

FULL=0
ASAN=0
QUAR_ONLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --full) FULL=1; shift ;;
        --asan) ASAN=1; shift ;;
        --quarantine-only) QUAR_ONLY=1; shift ;;
        -h|--help)
            sed -n '2,26p' "$0"; exit 0 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

PANIC_RE='use-after-sweep|GC root loss|panicked at|attempt to .* freed'
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
EVIDENCE="$ROOT/harness/evidence/rooting-battery/$STAMP"
mkdir -p "$EVIDENCE"
findings=0
warns=0

note()    { printf '%s\n' "$*"; }
finding() { findings=$((findings+1)); printf 'FINDING: %s\n' "$*"; }

REPRO_SET=(db gc gengc coroutine locals calls closure errors)
STRESS_REPRO_SET=(db coroutine locals)

cargo build -p lua-cli -q || { echo "debug build failed" >&2; exit 2; }
BIN="$ROOT/target/debug/lua-rs"

run_capture() {
    local outfile="$1"; shift
    timeout "${BATTERY_TIMEOUT_S:-240}" "$@" > "$outfile" 2>&1
}

scan_panics() {
    local outfile="$1" label="$2"
    if grep -qE "$PANIC_RE" "$outfile"; then
        case "$outfile" in
            "$EVIDENCE"/*) ;;
            *) cp "$outfile" "$EVIDENCE/" ;;
        esac
        finding "$label: panic signature (evidence: $EVIDENCE/$(basename "$outfile"))"
        return 1
    fi
    return 0
}

note "== config 1: quarantine only (cadence-identical; ALL failures count) =="
LUA_RS_GC_QUARANTINE=1 harness/canaries/gc/run_canaries.sh > "$EVIDENCE/quar-canaries.log" 2>&1
if awk -F'\t' '$3=="FAIL"{exit 1}' harness/canaries/gc/results.tsv; then
    note "  canaries: all PASS (both GC modes)"
else
    cp harness/canaries/gc/results.tsv "$EVIDENCE/quar-canaries.tsv"
    finding "quarantine canaries failed (see $EVIDENCE/quar-canaries.tsv)"
fi

for t in "${REPRO_SET[@]}"; do
    out="$EVIDENCE/quar-$t.out"
    if LUA_RS_GC_QUARANTINE=1 run_capture "$out" \
        harness/run_official_test.sh "reference/lua-c/testes/$t.lua" \
        && grep -q "^PASS" "$out"; then
        note "  $t.lua: PASS"
        rm -f "$out"
    else
        finding "quarantine $t.lua failed (evidence: $out)"
    fi
done

out="$EVIDENCE/quar-db.wrap.out"
if LUA_RS_GC_QUARANTINE=1 run_capture "$out" "$BIN" harness/impl/official/db.wrap.lua; then
    note "  db.wrap.lua: PASS"
    rm -f "$out"
else
    finding "quarantine db.wrap.lua failed (evidence: $out)"
fi

if [ "$QUAR_ONLY" = "1" ]; then
    echo "---"
    if [ "$findings" -gt 0 ]; then
        echo "BATTERY (quarantine-only): $findings finding(s); evidence in $EVIDENCE"
        exit 1
    fi
    rm -rf "$EVIDENCE" 2>/dev/null || true
    echo "BATTERY (quarantine-only): clean"
    exit 0
fi

note "== config 2: stress + quarantine (only panic signatures count) =="
LUA_RS_GC_STRESS=1 LUA_RS_GC_QUARANTINE=1 harness/canaries/gc/run_canaries.sh \
    > "$EVIDENCE/stress-canaries.log" 2>&1
for o in harness/canaries/gc/canary_*.out; do
    scan_panics "$o" "stress canary $(basename "$o")" || true
done

for t in "${STRESS_REPRO_SET[@]}"; do
    out="$EVIDENCE/stress-$t.out"
    LUA_RS_GC_STRESS=1 LUA_RS_GC_QUARANTINE=1 run_capture "$out" \
        harness/run_official_test.sh "reference/lua-c/testes/$t.lua" || true
    if scan_panics "$out" "stress $t.lua"; then
        rm -f "$out"
        note "  $t.lua: no panic signatures"
    fi
done

out="$EVIDENCE/stress-db.wrap.out"
LUA_RS_GC_STRESS=1 LUA_RS_GC_QUARANTINE=1 run_capture "$out" "$BIN" \
    harness/impl/official/db.wrap.lua || true
if scan_panics "$out" "stress db.wrap.lua"; then
    rm -f "$out"
    note "  db.wrap.lua: no panic signatures"
fi

if [ "$FULL" = "1" ]; then
    note "== config 3 (--full): full official suite under quarantine =="
    out="$EVIDENCE/quar-full-suite.log"
    if LUA_RS_GC_QUARANTINE=1 TEST_TIMEOUT_S=240 harness/run_official_all.sh > "$out" 2>&1; then
        note "  full suite: PASS"
    else
        finding "full suite under quarantine failed (evidence: $out)"
    fi
fi

if [ "$ASAN" = "1" ]; then
    note "== config 4 (--asan): AddressSanitizer build over the repro set =="
    if ! rustup toolchain list 2>/dev/null | grep -q nightly; then
        echo "nightly toolchain required: rustup toolchain install nightly" >&2
        exit 2
    fi
    HOST_TRIPLE=$(rustc -vV | awk '/^host:/ {print $2}')
    SHA=$(git rev-parse --short=12 HEAD)
    CACHE_DIR=/tmp/lua-rs-bincache
    ASAN_BIN="$CACHE_DIR/$SHA-asan-lua-rs"
    if [ ! -x "$ASAN_BIN" ]; then
        mkdir -p "$CACHE_DIR"
        WT="$CACHE_DIR/wt-asan-$SHA"
        trap 'git worktree remove --force "$WT" >/dev/null 2>&1 || true' EXIT
        git worktree add --detach "$WT" "$SHA" >/dev/null 2>&1 || { echo "worktree add failed (dirty tree? commit first)" >&2; exit 2; }
        ( cd "$WT" && RUSTFLAGS=-Zsanitizer=address cargo +nightly build -p lua-cli --target "$HOST_TRIPLE" -q ) \
            || { echo "ASAN build failed" >&2; exit 2; }
        cp "$WT/target/$HOST_TRIPLE/debug/lua-rs" "$ASAN_BIN"
        git worktree remove --force "$WT" >/dev/null 2>&1 || true
    fi
    for stress in 0 1; do
        for t in "${STRESS_REPRO_SET[@]}"; do
            out="$EVIDENCE/asan-s$stress-$t.out"
            env LUA_RS_GC_STRESS=$stress LUA_RS_BIN="$ASAN_BIN" \
                timeout "${BATTERY_TIMEOUT_S:-300}" \
                harness/run_official_test.sh "reference/lua-c/testes/$t.lua" > "$out" 2>&1 || true
            if grep -q "AddressSanitizer" "$out"; then
                finding "ASAN report: $t.lua stress=$stress (evidence: $out)"
            else
                note "  $t.lua stress=$stress: no ASAN reports"
                rm -f "$out"
            fi
        done
        out="$EVIDENCE/asan-s$stress-db.wrap.out"
        env LUA_RS_GC_STRESS=$stress timeout "${BATTERY_TIMEOUT_S:-300}" \
            "$ASAN_BIN" harness/impl/official/db.wrap.lua > "$out" 2>&1 || true
        if grep -q "AddressSanitizer" "$out"; then
            finding "ASAN report: db.wrap.lua stress=$stress (evidence: $out)"
        else
            note "  db.wrap.lua stress=$stress: no ASAN reports"
            rm -f "$out"
        fi
    done
fi

echo "---"
if [ "$findings" -gt 0 ]; then
    echo "BATTERY: $findings finding(s); evidence in $EVIDENCE"
    exit 1
fi
rmdir "$EVIDENCE" 2>/dev/null || true
echo "BATTERY: clean"
exit 0
