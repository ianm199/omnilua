#!/usr/bin/env bash
# Dispatch one Phase D-1a migration agent on a given scope.
# Usage: ./harness/d1a_dispatch.sh <scope> <tag>
#   e.g. ./harness/d1a_dispatch.sh crates/lua-vm/src/ lua-vm

set -uo pipefail

SCOPE="${1:?scope required}"
TAG="${2:?tag required}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROMPT_TEMPLATE="$(cat "$ROOT/harness/d1a_agent_prompt.txt")"
PROMPT="${PROMPT_TEMPLATE//__SCOPE__/$SCOPE}"

OUT="harness/impl/d1a-$TAG.translator.json"
TRANSCRIPT="harness/impl/d1a-$TAG.transcript.jsonl"
STDERR="harness/impl/d1a-$TAG.stderr"

export CLAUDE_CONFIG_DIR="$HOME/.claude-personal"
unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN
export CLAUDE_CODE_MAX_OUTPUT_TOKENS="${CLAUDE_CODE_MAX_OUTPUT_TOKENS:-64000}"

claude -p \
    --append-system-prompt "$(cat PORTING.md)" \
    --allowedTools "Read,Write,Edit,Glob,Grep,Bash(cargo build*),Bash(cargo check*),Bash(grep *),Bash(rg *),Bash(cat *),Bash(head *),Bash(tail *),Bash(wc *),Bash(find *),Bash(target/debug/lua-rs *)" \
    --permission-mode dontAsk \
    --output-format stream-json \
    --include-partial-messages \
    --verbose \
    --max-budget-usd 15 \
    "$PROMPT" \
    2>>"$STDERR" \
    | tee "$TRANSCRIPT" >/dev/null

jq -s 'map(select(.type == "result")) | .[-1] // {}' "$TRANSCRIPT" > "$OUT" 2>/dev/null || echo '{}' > "$OUT"
cost=$(jq -r '.total_cost_usd // 0' "$OUT")
echo "[d1a/$TAG] done. cost=\$$cost"
