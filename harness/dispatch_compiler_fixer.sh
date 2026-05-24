#!/usr/bin/env bash
# dispatch_compiler_fixer.sh — fire one Compiler-fixer agent at one crate.
#
# Usage:
#   ./harness/dispatch_compiler_fixer.sh <crate> <budget_usd> [scope_hint]
#
# Example:
#   ./harness/dispatch_compiler_fixer.sh lua-vm 5.00 "focus on api.rs and debug.rs"
#
# Writes:
#   harness/oracle/results/fix-<crate>-<ts>.transcript.jsonl
#   harness/oracle/results/fix-<crate>-<ts>.translator.json
# Echoes a JSON line to stdout:
#   {"crate":"...","status":"ok|error","cost_usd":N,"duration_s":N,"residual_errors":N}

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CRATE="${1:?usage: dispatch_compiler_fixer.sh <crate> <budget_usd> [scope_hint]}"
BUDGET="${2:?budget required}"
SCOPE_HINT="${3:-}"

TS=$(date +%s)
RESULTS_DIR="harness/oracle/results"
mkdir -p "$RESULTS_DIR"
BASE="fix-$CRATE-$TS"
TRANSCRIPT="$RESULTS_DIR/$BASE.transcript.jsonl"
OUT_JSON="$RESULTS_DIR/$BASE.translator.json"

# Auth: subscription, never API. Same as fanout.sh.
export CLAUDE_CONFIG_DIR="$HOME/.claude-personal"
unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN
export CLAUDE_CODE_MAX_OUTPUT_TOKENS="${CLAUDE_CODE_MAX_OUTPUT_TOKENS:-64000}"

# Snapshot starting error count so we can measure forward progress.
START_ERRORS=$(cargo check -p "$CRATE" 2>&1 | awk '/^error\[/ {c++} END {print c+0}')

START_TS=$(date +%s)

PROMPT="You are a Compiler-fixer agent in a multi-crate Rust workspace. Scope: ONE crate, '$CRATE'.

Repo: $ROOT. Read CLAUDE.md and PORTING.md for conventions (no inline comments, no fallback patterns, no &str/String for Lua data, no unsafe outside explicit unsafe-budget crates).

Your task: reduce 'cargo check -p $CRATE' error count toward 0.

Starting baseline: $START_ERRORS errors.

$([ -n \"$SCOPE_HINT\" ] && echo \"Scope hint from orchestrator: $SCOPE_HINT\")

Hard constraints:
1. Edit ONLY files under crates/$CRATE/. Do NOT touch crates/lua-types/ or other crates.
2. If an error truly requires a lua-types change, leave a 'TODO(phase-b): needs lua_types::X' comment with a local todo!() stub and move on.
3. Do not invent business logic. todo!(\"phase-b: <description>\") is correct for unimplemented bodies.
4. Do not remove or rewrite existing real logic. Only fix what rustc complains about.
5. After each batch of edits, run 'cargo check -p $CRATE 2>&1 | grep -cE \"^error\\[\"' to see error count drop.
6. Do not commit; the orchestrator handles commits.
7. Stop when error count stops dropping (3 consecutive checks with no improvement) or hits 0.

Report at the end (under 150 words):
- Starting / ending error count
- Top 2 remaining error categories and why they're hard
- One sentence of architectural feedback for the next pass"

# Single invocation. budget cap is the hard stop.
claude -p \
    --append-system-prompt "$(cat PORTING.md)" \
    --allowedTools "Read,Write,Edit,Glob,Grep,Bash(cargo check*),Bash(cargo *),Bash(rustc *),Bash(grep *),Bash(rg *)" \
    --permission-mode dontAsk \
    --output-format stream-json \
    --include-partial-messages \
    --verbose \
    --max-budget-usd "$BUDGET" \
    "$PROMPT" \
    2>>"$RESULTS_DIR/$BASE.stderr" \
    | tee "$TRANSCRIPT" >/dev/null

# Extract the final result event.
jq -s 'map(select(.type == "result")) | .[-1] // {}' "$TRANSCRIPT" > "$OUT_JSON" 2>/dev/null || echo '{}' > "$OUT_JSON"

END_TS=$(date +%s)
DURATION=$((END_TS - START_TS))

COST=$(jq -r '.total_cost_usd // 0' "$OUT_JSON")
IS_ERROR=$(jq -r '.is_error // false' "$OUT_JSON")

# Measure residual.
END_ERRORS=$(cargo check -p "$CRATE" 2>&1 | awk '/^error\[/ {c++} END {print c+0}')

# Status: ok if errors dropped or hit 0, else "stalled" or "error"
if [ "$IS_ERROR" = "true" ]; then
    STATUS="error"
elif [ "$END_ERRORS" -lt "$START_ERRORS" ]; then
    STATUS="ok"
elif [ "$END_ERRORS" = "0" ]; then
    STATUS="ok"
else
    STATUS="stalled"
fi

# Single-line JSON summary on stdout. Orchestrator parses this.
jq -c -n \
    --arg crate "$CRATE" \
    --arg status "$STATUS" \
    --argjson cost "${COST:-0}" \
    --argjson dur "$DURATION" \
    --argjson start_err "$START_ERRORS" \
    --argjson end_err "$END_ERRORS" \
    '{crate: $crate, status: $status, cost_usd: $cost, duration_s: $dur, start_errors: $start_err, end_errors: $end_err}'
