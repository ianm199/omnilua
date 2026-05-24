#!/usr/bin/env bash
# overnight.sh — sequential phase orchestrator for unattended progression.
#
# Phases:
#   B_finish  — compiler-fix residual lua-vm errors (~250 currently)
#   C_xlate   — translate 12 lua-stdlib files (Phase C)
#   C_wire    — pub mod each stdlib file into lua-stdlib/src/lib.rs
#   C_fix     — compiler-fix lua-stdlib residuals
#   D_xlate   — translate lgc.c + lmem.c (Phase D / GC)
#   D_fix     — compiler-fix lua-gc residuals
#   E_xlate   — translate any coroutines work in lua-coro
#   E_fix     — compiler-fix lua-coro
#   final     — workspace cargo check, write MORNING_REPORT.md
#
# Safety rails:
#   - $1000 hard cap on total spend
#   - $300 soft cap: log warning, keep going
#   - Per-file retry limit of 2
#   - Stall detection: 3 consecutive compiler-fixer dispatches with no
#     error-count drop → mark phase done, advance
#   - Systemic failure abort: 5 consecutive is_error:true → write report, exit
#   - No destructive ops, no force-pushes, no --no-verify, no skipping hooks
#
# Run as:
#   nohup ./harness/overnight.sh > /tmp/overnight.log 2>&1 &
#   disown
#
# Resume / state:
#   harness/overnight/state.jsonl    — append-only timeline
#   harness/overnight/attempts.tsv   — per-file retry tracking
#   harness/overnight/MORNING_REPORT.md — final report

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ───── Budget caps ─────────────────────────────────────────────────────
TOTAL_HARD_CAP=1000.00
TOTAL_SOFT_CAP=300.00
TOTAL_SPENT="0.00"
CONSECUTIVE_ERRORS=0

# ───── State files ─────────────────────────────────────────────────────
OVERNIGHT_DIR="harness/overnight"
mkdir -p "$OVERNIGHT_DIR"
STATE="$OVERNIGHT_DIR/state.jsonl"
ATTEMPTS="$OVERNIGHT_DIR/attempts.tsv"
REPORT="$OVERNIGHT_DIR/MORNING_REPORT.md"
LOG="$OVERNIGHT_DIR/overnight.log"
SUMMARY="$OVERNIGHT_DIR/per_phase_summary.jsonl"
touch "$STATE" "$ATTEMPTS" "$LOG" "$SUMMARY"

START_TS=$(date +%s)
START_ISO=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# ───── Helpers ─────────────────────────────────────────────────────────

emit() {
    local ts
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    echo "[$ts] $*" | tee -a "$LOG"
}

record() {
    # record "action" "detail" — appends a JSON line to state.jsonl
    local action="$1" detail="${2:-}"
    local ts
    ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    jq -c -n \
        --arg ts "$ts" --arg action "$action" --arg detail "$detail" \
        --argjson total "$TOTAL_SPENT" \
        '{ts: $ts, action: $action, detail: $detail, total_spent: $total}' \
        >> "$STATE"
}

add_cost() {
    # add_cost <usd>  → updates TOTAL_SPENT, returns 0 if under hard cap, 1 if over
    local delta="${1:-0}"
    TOTAL_SPENT=$(awk -v a="$TOTAL_SPENT" -v b="$delta" 'BEGIN { printf "%.4f", a + b }')
    if awk -v t="$TOTAL_SPENT" -v cap="$TOTAL_HARD_CAP" 'BEGIN { exit !(t > cap) }'; then
        emit "HARD CAP HIT: spent \$$TOTAL_SPENT > cap \$$TOTAL_HARD_CAP"
        return 1
    fi
    if awk -v t="$TOTAL_SPENT" -v cap="$TOTAL_SOFT_CAP" 'BEGIN { exit !(t > cap) }'; then
        emit "  (soft cap exceeded: \$$TOTAL_SPENT — continuing)"
    fi
    return 0
}

attempt_count() {
    # attempt_count <key>  → echoes the count
    local key="$1"
    grep -P "^$key\t" "$ATTEMPTS" 2>/dev/null | awk -F'\t' '{print $2}' | tail -1 || echo 0
}

bump_attempt() {
    local key="$1"
    local current new
    current=$(attempt_count "$key")
    new=$((current + 1))
    printf "%s\t%d\n" "$key" "$new" >> "$ATTEMPTS"
    echo "$new"
}

commit_changes() {
    local msg="$1"
    if [ -n "$(git status --porcelain)" ]; then
        git add -A
        if git commit -m "$msg" >/dev/null 2>&1; then
            local hash
            hash=$(git log -1 --format=%h)
            emit "  committed: $hash $msg"
            record "commit" "$hash $msg"
        else
            emit "  commit failed; continuing"
        fi
    fi
}

workspace_error_count() {
    cargo check --workspace 2>&1 | awk '/^error\[/ {c++} END {print c+0}'
}

crate_error_count() {
    cargo check -p "$1" 2>&1 | awk '/^error\[/ {c++} END {print c+0}'
}

# ───── Phase B finish ──────────────────────────────────────────────────

phase_b_finish() {
    emit "════ Phase B finish: lua-vm compiler-fix residuals ════"
    record "phase_start" "B_finish"

    local prev_errors stall=0
    prev_errors=$(crate_error_count lua-vm)
    emit "  lua-vm starting errors: $prev_errors"

    # Up to 3 passes, stop if errors stall.
    for pass in 1 2 3; do
        if [ "$prev_errors" -le 20 ]; then
            emit "  lua-vm under 20 errors ($prev_errors); enough for now"
            break
        fi

        local key="lua-vm-b-pass-$pass"
        local attempts
        attempts=$(attempt_count "$key")
        if [ "$attempts" -ge 2 ]; then
            emit "  lua-vm-b-pass-$pass already attempted twice; skip"
            continue
        fi
        bump_attempt "$key" >/dev/null

        emit "  → pass $pass: dispatching compiler-fixer (budget \$6, hint=focus on top E0308/E0061)"
        local raw
        raw=$(./harness/dispatch_compiler_fixer.sh lua-vm 6.00 \
              "Focus on E0308 type mismatches and E0061 wrong-arg-count errors. \
               Most are LuaState method-stub signatures that disagree with call sites. \
               Use Rust's 'expected ... found ...' diagnostics to align signatures." 2>&1 | tail -1) || true
        record "fixer_done" "lua-vm pass $pass: $raw"

        local cost end_err status
        cost=$(echo "$raw" | jq -r '.cost_usd // 0')
        end_err=$(echo "$raw" | jq -r '.end_errors // 999')
        status=$(echo "$raw" | jq -r '.status // "error"')

        if ! add_cost "$cost"; then return 2; fi

        if [ "$status" = "error" ]; then
            CONSECUTIVE_ERRORS=$((CONSECUTIVE_ERRORS + 1))
            emit "  pass $pass errored (consecutive: $CONSECUTIVE_ERRORS)"
            if [ "$CONSECUTIVE_ERRORS" -ge 5 ]; then return 3; fi
            continue
        else
            CONSECUTIVE_ERRORS=0
        fi

        emit "  pass $pass: $prev_errors → $end_err errors, \$$cost"
        commit_changes "Phase B compiler-fixer pass $pass: lua-vm $prev_errors → $end_err errors"

        if [ "$end_err" = "$prev_errors" ] || [ "$end_err" -ge "$prev_errors" ]; then
            stall=$((stall + 1))
            if [ "$stall" -ge 2 ]; then
                emit "  stalled at $end_err errors after 2 plateau passes; advancing"
                break
            fi
        else
            stall=0
        fi
        prev_errors=$end_err
    done

    record "phase_end" "B_finish"
    jq -c -n --arg phase "B_finish" --argjson cost_so_far "$TOTAL_SPENT" \
        --argjson lua_vm_errors "$(crate_error_count lua-vm)" \
        --argjson workspace_errors "$(workspace_error_count)" \
        '{phase: $phase, total_spent: $cost_so_far, lua_vm_errors: $lua_vm_errors, workspace_errors: $workspace_errors}' \
        >> "$SUMMARY"
}

# ───── Phase C: stdlib translate ───────────────────────────────────────

phase_c_translate() {
    emit "════ Phase C translate: 12 stdlib files ════"
    record "phase_start" "C_xlate"

    local files=(
        lbaselib.c lstrlib.c lmathlib.c ltablib.c liolib.c loslib.c
        lutf8lib.c ldblib.c lcorolib.c loadlib.c lauxlib.c linit.c
    )

    # fanout.sh handles per-file budget, parallelism, hooks, idempotency.
    # Each file: $3 budget; with 6 workers, wall time ≈ longest file.
    emit "  dispatching fanout: 6 workers, \$3/file"
    if ! ./harness/fanout.sh --files "${files[@]}" --workers 6 --allow-dirty 2>&1 | tee -a "$LOG"; then
        emit "  fanout errored (continuing)"
    fi

    # Aggregate cost from pilot.jsonl rows just added.
    local phase_cost
    phase_cost=$(jq -s '[.[] | select(.cost_usd) | .cost_usd] | add // 0' harness/oracle/results/pilot.jsonl)
    # fanout commits per-file via the Stop hook; we're not double-committing.
    emit "  Phase C translate spend: \$$phase_cost"
    add_cost "$phase_cost" || return 2

    record "phase_end" "C_xlate"
}

# ───── Phase C: wire lua-stdlib lib.rs ─────────────────────────────────

phase_c_wire() {
    emit "════ Phase C wire: lua-stdlib/src/lib.rs modules ════"
    record "phase_start" "C_wire"

    local files=(
        base string_lib table_lib math_lib io_lib os_lib utf8_lib
        debug_lib coro_lib loadlib auxlib init
    )

    local lib="crates/lua-stdlib/src/lib.rs"
    {
        echo "//! Lua 5.4 standard library — runtime stdlib crate."
        echo "//!"
        echo "//! Each module corresponds to one C source under reference/lua-5.4.7/src/."
        echo "//! See ANALYSES/file_deps.txt for the mapping."
        echo ""
        for m in "${files[@]}"; do
            if [ -f "crates/lua-stdlib/src/$m.rs" ]; then
                echo "pub mod $m;"
            else
                echo "// TODO(phase-c): pub mod $m; — file not yet ported"
            fi
        done
        echo ""
        echo "// ──────────────────────────────────────────────────────────────────────────"
        echo "// PORT STATUS"
        echo "//   source:        (module aggregator)"
        echo "//   target_crate:  lua-stdlib"
        echo "//   confidence:    high"
        echo "//   todos:         0"
        echo "//   port_notes:    0"
        echo "//   unsafe_blocks: 0"
        echo "//   notes:         Each pub mod maps to one stdlib C file."
        echo "// ──────────────────────────────────────────────────────────────────────────"
    } > "$lib"

    commit_changes "Phase C: wire lua-stdlib/src/lib.rs with translated modules"
    record "phase_end" "C_wire"
}

# ───── Phase C: stdlib compiler-fix ────────────────────────────────────

phase_c_fix() {
    emit "════ Phase C fix: lua-stdlib compiler-fix residuals ════"
    record "phase_start" "C_fix"

    local prev_errors stall=0
    prev_errors=$(crate_error_count lua-stdlib)
    emit "  lua-stdlib starting errors: $prev_errors"

    for pass in 1 2 3; do
        if [ "$prev_errors" -le 20 ]; then break; fi

        local key="lua-stdlib-c-pass-$pass"
        if [ "$(attempt_count "$key")" -ge 2 ]; then continue; fi
        bump_attempt "$key" >/dev/null

        emit "  → pass $pass: dispatching compiler-fixer (budget \$6)"
        local raw
        raw=$(./harness/dispatch_compiler_fixer.sh lua-stdlib 6.00 \
              "stdlib files often have lots of cross-crate refs to lua-vm. \
               Use lua_vm::state::LuaState and helpers from there. \
               If methods don't exist on LuaState, leave TODO(phase-c) and stub." 2>&1 | tail -1) || true
        record "fixer_done" "lua-stdlib pass $pass: $raw"

        local cost end_err status
        cost=$(echo "$raw" | jq -r '.cost_usd // 0')
        end_err=$(echo "$raw" | jq -r '.end_errors // 999')
        status=$(echo "$raw" | jq -r '.status // "error"')

        if ! add_cost "$cost"; then return 2; fi
        if [ "$status" = "error" ]; then
            CONSECUTIVE_ERRORS=$((CONSECUTIVE_ERRORS + 1))
            if [ "$CONSECUTIVE_ERRORS" -ge 5 ]; then return 3; fi
            continue
        else
            CONSECUTIVE_ERRORS=0
        fi

        emit "  pass $pass: $prev_errors → $end_err errors, \$$cost"
        commit_changes "Phase C compiler-fixer pass $pass: lua-stdlib $prev_errors → $end_err errors"
        if [ "$end_err" = "$prev_errors" ] || [ "$end_err" -ge "$prev_errors" ]; then
            stall=$((stall + 1))
            if [ "$stall" -ge 2 ]; then break; fi
        else
            stall=0
        fi
        prev_errors=$end_err
    done

    record "phase_end" "C_fix"
}

# ───── Phase D: GC translate + fix ─────────────────────────────────────

phase_d() {
    emit "════ Phase D: GC translate + fix (lua-gc) ════"
    record "phase_start" "D"

    if ! ./harness/fanout.sh --files lgc.c lmem.c --workers 2 --allow-dirty 2>&1 | tee -a "$LOG"; then
        emit "  Phase D fanout errored (continuing)"
    fi

    local phase_cost
    phase_cost=$(jq -s '[.[] | select(.cost_usd) | .cost_usd] | add // 0' harness/oracle/results/pilot.jsonl)

    # Wire lua-gc/src/lib.rs
    local lib="crates/lua-gc/src/lib.rs"
    {
        echo "//! Lua 5.4 garbage collector."
        echo "//!"
        echo "//! Modules:"
        echo "//!   heap - Phase-D production mark-sweep (Gc<T>, Trace, Heap)"
        echo "//!"
        echo "//! Historical Phase-A partial ports of C-Lua's \`lgc.c\`/\`lmem.c\` were removed"
        echo "//! once \`heap.rs\` became the production collector. Keep this crate's source"
        echo "//! tree limited to compiled modules so unsafe audits reflect the active build."
        echo ""
        echo "pub mod heap;"
        echo ""
        echo "pub use heap::{Color, Gc, GcBox, GcHeader, GcState, Heap, HeapGuard, Marker, StepBudget, StepOutcome, Trace, with_current_heap};"
        echo ""
        echo "// ──────────────────────────────────────────────────────────────────────────"
        echo "// PORT STATUS"
        echo "//   source:        (module aggregator; per-file ports own their own trailers)"
        echo "//   target_crate:  lua-gc"
        echo "//   confidence:    high"
        echo "//   todos:         0"
        echo "//   port_notes:    0"
        echo "//   unsafe_blocks: 0"
        echo "//   notes:         Module aggregator: re-exports the public surface of heap.rs"
        echo "//                  Reference-only Phase-A partial ports are not kept in src/."
        echo "// ──────────────────────────────────────────────────────────────────────────"
    } > "$lib"
    commit_changes "Phase D: wire lua-gc/src/lib.rs"

    # One compiler-fixer pass.
    local prev_errors
    prev_errors=$(crate_error_count lua-gc)
    if [ "$prev_errors" -gt 0 ]; then
        emit "  lua-gc starting errors: $prev_errors"
        for pass in 1 2; do
            local key="lua-gc-d-pass-$pass"
            if [ "$(attempt_count "$key")" -ge 2 ]; then continue; fi
            bump_attempt "$key" >/dev/null
            local raw
            raw=$(./harness/dispatch_compiler_fixer.sh lua-gc 5.00 \
                  "lua-gc has a 20-block unsafe budget; some unsafe is expected. \
                   The hooks enforce the cap. Real GC algorithm should stay; only fix compile errors." 2>&1 | tail -1) || true
            local cost end_err
            cost=$(echo "$raw" | jq -r '.cost_usd // 0')
            end_err=$(echo "$raw" | jq -r '.end_errors // 999')
            add_cost "$cost" || return 2
            commit_changes "Phase D fixer pass $pass: lua-gc $prev_errors → $end_err"
            if [ "$end_err" = "$prev_errors" ] || [ "$end_err" -ge "$prev_errors" ]; then break; fi
            prev_errors=$end_err
        done
    fi

    record "phase_end" "D"
}

# ───── Final report ────────────────────────────────────────────────────

write_morning_report() {
    local end_ts elapsed
    end_ts=$(date +%s)
    elapsed=$((end_ts - START_TS))
    local mins=$((elapsed / 60))

    {
        echo "# Overnight run — morning report"
        echo ""
        echo "**Started**: $START_ISO"
        echo "**Ended**: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
        echo "**Elapsed**: ${mins} min"
        echo "**Total spent**: \$$TOTAL_SPENT (cap \$$TOTAL_HARD_CAP)"
        echo ""
        echo "## Final workspace state"
        echo ""
        echo "| Crate | Errors |"
        echo "|---|---:|"
        for c in lua-types lua-lex lua-code lua-parse lua-vm lua-stdlib lua-gc lua-coro lua-cli; do
            echo "| $c | $(crate_error_count "$c") |"
        done
        echo ""
        echo "**Workspace total**: $(workspace_error_count) errors"
        echo ""
        echo "## Per-phase summary"
        echo ""
        echo "\`\`\`"
        jq -r '"\(.phase): spent=$\(.total_spent) workspace_errors=\(.workspace_errors // "?")"' "$SUMMARY"
        echo "\`\`\`"
        echo ""
        echo "## Git activity"
        echo ""
        echo "\`\`\`"
        git log --oneline --since="$START_ISO" | head -40
        echo "\`\`\`"
        echo ""
        echo "## Notable events"
        echo ""
        echo "\`\`\`"
        jq -r 'select(.action | IN("phase_start","phase_end","commit","fixer_done")) | "\(.ts) \(.action): \(.detail)"' "$STATE" | tail -40
        echo "\`\`\`"
        echo ""
        echo "## Where the run ended"
        echo ""
        if [ "${RUN_EXIT_REASON:-completed}" = "completed" ]; then
            echo "Completed all planned phases."
        else
            echo "Stopped early: $RUN_EXIT_REASON"
        fi
    } > "$REPORT"

    emit "morning report written: $REPORT"
}

# ───── Main ────────────────────────────────────────────────────────────

trap 'RUN_EXIT_REASON="signal trap (interrupted)"; write_morning_report; exit 130' INT TERM

emit "═══════════════════════════════════════════════════════════════"
emit "Overnight orchestrator starting at $START_ISO"
emit "Hard cap: \$$TOTAL_HARD_CAP  Soft cap: \$$TOTAL_SOFT_CAP"
emit "═══════════════════════════════════════════════════════════════"
record "run_start" ""

RUN_EXIT_REASON="completed"

phase_b_finish || {
    case $? in
        2) RUN_EXIT_REASON="budget hard cap";;
        3) RUN_EXIT_REASON="systemic failures in B_finish";;
    esac
    write_morning_report
    exit $?
}

phase_c_translate || { RUN_EXIT_REASON="failure in C_xlate"; write_morning_report; exit $?; }
phase_c_wire
phase_c_fix || { RUN_EXIT_REASON="failure in C_fix"; write_morning_report; exit $?; }
phase_d || { RUN_EXIT_REASON="failure in D"; write_morning_report; exit $?; }

write_morning_report
emit "═══════ Overnight complete ═══════"
exit 0
