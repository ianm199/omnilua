#!/usr/bin/env bash
# harness/stop-hook.sh — v5 Stop-hook test gate.
#
# Replaces the bare "git add . && git commit" Stop hook with a gate that
# refuses to land work which regresses a small smoke set of official Lua
# tests. The build-only gate the previous hook ran was satisfied by every
# agent commit that compiled cleanly, even when the change broke an
# already-passing test — those regressions only surfaced 1-5 commits later
# during the next mega_loop scan.
#
# Pipeline (each stage rejects the commit on failure, leaving the working
# tree dirty for human review):
#   1. Empty-tree fast path. Nothing to commit → exit 0 silently.
#   2. Gating hooks rerun (unsafe-budget, forbidden-import,
#      type-vocabulary, trailer-required) on the .rs files this agent
#      modified — the existing commit-on-stop.sh behaviour we preserve.
#   3. cargo build -p omnilua-cli -q. Build broken → reject.
#   4. Smoke set of 6 fixed tests + the optional AGENT_TARGET_PROG. Each
#      compared against harness/baseline-smoke.tsv. Previously-PASS test
#      now non-PASS → reject. New PASS for a previously-non-PASS test →
#      update baseline with current HEAD SHA.
#   5. git add -A && git commit.
#
# Honours AGENT_TARGET_PROG (path to a .lua test): when set, that test is
# appended to the smoke set so a debug-agent targeting a specific failure
# can never commit a change that regresses the very test it was dispatched
# against. mega_loop.sh's dispatch_debug forwards this var.

set -uo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)"
if [ -z "$ROOT" ]; then
    exit 0
fi
cd "$ROOT"

if [ -z "$(git status --porcelain 2>/dev/null)" ]; then
    exit 0
fi

# PERF_PUSH_SPEC.md P7.4: while a benchmark or perf experiment is live, do
# not build, smoke, or auto-commit — a Stop event mid-measurement must not
# contend with the bench for CPU or sweep unvalidated experiment diffs into
# a commit (both happened on 2026-06-09: d0dc949 swept an unvalidated vm.rs
# experiment; the 18:36:48Z hook run contended with a running A/B bench).
# compare_bins.sh maintains the marker for its lifetime; experiment chains
# may hold it across builds. Markers older than 2 hours are stale.
PERF_MARKER="$ROOT/harness/.perf-experiment"
if [ -f "$PERF_MARKER" ]; then
    if [ -n "$(find "$PERF_MARKER" -mmin -120 2>/dev/null)" ]; then
        echo "[stop-hook] perf experiment marker present; skipping build/smoke/auto-commit. Tree left as-is." >&2
        exit 0
    fi
    echo "[stop-hook] stale perf marker (>2h old) ignored: $PERF_MARKER" >&2
fi

GATING_HOOKS=(
    "$ROOT/.claude/hooks/unsafe-budget.sh"
    "$ROOT/.claude/hooks/forbidden-import.sh"
    "$ROOT/.claude/hooks/type-vocabulary.sh"
    "$ROOT/.claude/hooks/trailer-required.sh"
)

modified_rs=()
while IFS= read -r f; do
    [ -z "$f" ] && continue
    [[ "$f" == crates/*.rs ]] || [[ "$f" == crates/*/*.rs ]] || [[ "$f" == crates/*/src/*.rs ]] || [[ "$f" == crates/*/src/**/*.rs ]] || continue
    [ -f "$f" ] && modified_rs+=("$f")
done < <( (git diff --name-only HEAD -- 'crates/**/*.rs'; git ls-files --others --exclude-standard -- 'crates/**/*.rs') 2>/dev/null | sort -u )

failed_pairs=()
if [ "${#modified_rs[@]}" -gt 0 ]; then
    for f in "${modified_rs[@]}"; do
        for hook in "${GATING_HOOKS[@]}"; do
            [ -x "$hook" ] || continue
            if ! CLAUDE_TARGET_RS_FILE="$f" "$hook" >/dev/null 2>&1; then
                failed_pairs+=("$(basename "$hook"):$f")
            fi
        done
    done
fi

if [ "${#failed_pairs[@]}" -gt 0 ]; then
    echo "[stop-hook] BLOCKED: ${#failed_pairs[@]} gating-hook violation(s) on agent-modified files:" >&2
    for pair in "${failed_pairs[@]}"; do
        echo "  - $pair" >&2
    done
    echo "[stop-hook] Working tree left dirty. Fix violations or commit by hand." >&2
    exit 1
fi

mkdir -p "$ROOT/harness/impl"
BUILD_ERR="$ROOT/harness/impl/stop-build.err"
if ! cargo build -p omnilua-cli -q 2>"$BUILD_ERR"; then
    echo "[stop-hook] BUILD BROKEN — rejecting commit. See $BUILD_ERR" >&2
    tail -n 10 "$BUILD_ERR" >&2 || true
    exit 1
fi

SMOKE_TESTS=(strings.lua closure.lua tracegc.lua big.lua sort.lua math.lua)
if [ -n "${AGENT_TARGET_PROG:-}" ]; then
    extra=$(basename "$AGENT_TARGET_PROG")
    case "$extra" in
        *.lua)
            in_set=0
            for t in "${SMOKE_TESTS[@]}"; do
                [ "$t" = "$extra" ] && in_set=1 && break
            done
            [ "$in_set" = "0" ] && SMOKE_TESTS+=("$extra")
            ;;
    esac
fi

BASELINE="$ROOT/harness/baseline-smoke.tsv"
TESTES_DIR="$ROOT/reference/lua-c/testes"
if [ ! -d "$TESTES_DIR" ]; then
    echo "[stop-hook] missing $TESTES_DIR — cannot run smoke set. Refusing commit." >&2
    exit 1
fi

TMP_TSV=$(mktemp)
trap 'rm -f "$TMP_TSV" "$TMP_TSV.up"' EXIT

regressed=()
for t in "${SMOKE_TESTS[@]}"; do
    test_path="$TESTES_DIR/$t"
    if [ ! -f "$test_path" ]; then
        echo -e "$t\tMISSING" >> "$TMP_TSV"
        continue
    fi
    status=$(TEST_TIMEOUT_S="${TEST_TIMEOUT_S:-20}" "$ROOT/harness/run_one_test.sh" "$test_path" 2>/dev/null || echo "FAIL")
    echo -e "$t\t$status" >> "$TMP_TSV"
    base=""
    if [ -f "$BASELINE" ]; then
        base=$(awk -F'\t' -v name="$t" '$1==name {print $2; exit}' "$BASELINE")
    fi
    if [ "$base" = "PASS" ] && [ "$status" != "PASS" ]; then
        regressed+=("$t:$base->$status")
    fi
done

if [ "${#regressed[@]}" -gt 0 ]; then
    echo "[stop-hook] REGRESSION on smoke set — rejecting commit:" >&2
    for r in "${regressed[@]}"; do
        echo "  - $r" >&2
    done
    echo "[stop-hook] Working tree left dirty. Investigate or stash." >&2
    echo "[stop-hook] Smoke-set status this run:" >&2
    sed 's/^/  /' "$TMP_TSV" >&2
    exit 1
fi

if [ -f "$BASELINE" ]; then
    cp "$BASELINE" "$TMP_TSV.up"
else
    : > "$TMP_TSV.up"
fi
sha=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
baseline_changed=0
while IFS=$'\t' read -r t status; do
    [ -z "$t" ] && continue
    [ "$status" = "PASS" ] || continue
    prev=$(awk -F'\t' -v name="$t" '$1==name {print $2; exit}' "$TMP_TSV.up")
    if [ "$prev" != "PASS" ]; then
        grep -v "^$t	" "$TMP_TSV.up" > "$TMP_TSV.up.next" 2>/dev/null || true
        mv "$TMP_TSV.up.next" "$TMP_TSV.up"
        printf '%s\tPASS\t%s\n' "$t" "$sha" >> "$TMP_TSV.up"
        baseline_changed=1
    fi
done < "$TMP_TSV"

if [ "$baseline_changed" = "1" ]; then
    mv "$TMP_TSV.up" "$BASELINE"
fi

git add -A 2>/dev/null
git commit -q -m "agent: auto-commit at stop ($(date -u +%Y-%m-%dT%H:%M:%SZ))" 2>/dev/null || true
exit 0
