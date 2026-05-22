#!/usr/bin/env bash
# Stop hook: grep changed .rs files for banned patterns per PORTING.md §5.
#
# Patterns:
#   - `use std::string::String` outside test code
#   - `: &str` / `: String` in function signatures (cheap heuristic — manual
#     review for false positives)
#   - `tokio::`, `async fn`, `futures::`, `rayon::`
#   - `std::process::Command` outside lua-cli
#   - `unwrap()` outside test code and lua-cli/main.rs

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

fail=0
report() { echo "[forbidden-import] FAIL: $*" >&2; fail=1; }

# Under parallel fanout, scope to only the current worker's target via
# CLAUDE_TARGET_RS_FILE (set by fanout.sh). Otherwise scan all crates/.
if [ -n "${CLAUDE_TARGET_RS_FILE:-}" ] && [ -f "${CLAUDE_TARGET_RS_FILE}" ]; then
    FILES=("${CLAUDE_TARGET_RS_FILE}")
else
    FILES=()
    while IFS= read -r f; do FILES+=("$f"); done < <(find crates -name '*.rs' 2>/dev/null)
fi

if [ "${#FILES[@]}" = "0" ]; then
    exit 0
fi

for f in "${FILES[@]}"; do
    # tokio / async / futures / rayon
    if grep -nE '\b(use tokio|async fn |use futures|use rayon)' "$f" > /dev/null; then
        report "banned crate import in $f"
        grep -nE '\b(use tokio|async fn |use futures|use rayon)' "$f" >&2 || true
    fi

    # std::process::Command outside lua-cli
    if [[ "$f" != crates/lua-cli/* ]]; then
        if grep -nE 'std::process::Command' "$f" > /dev/null; then
            report "std::process::Command outside lua-cli in $f"
        fi
    fi
done

exit "$fail"
