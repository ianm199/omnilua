#!/usr/bin/env bash
# Stop hook: every .rs file under crates/ must end with a PORT STATUS trailer.
# Format defined in PORTING.md §12.
#
# Under parallel fanout (workers >1), the older "scan all crates/*.rs"
# approach caused false positives — worker A's hook would see worker B's
# in-flight file and report a missing trailer that belongs to neither.
# If CLAUDE_TARGET_RS_FILE is set (fanout.sh exports it per worker), check
# only that file. Otherwise fall back to the tree scan (sequential mode).

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

if [ -n "${CLAUDE_TARGET_RS_FILE:-}" ] && [ -f "${CLAUDE_TARGET_RS_FILE}" ]; then
    files=("${CLAUDE_TARGET_RS_FILE}")
else
    files=()
    while IFS= read -r f; do files+=("$f"); done < <(find crates -name '*.rs' 2>/dev/null)
fi

fail=0
# Look further back for the trailer — verbose `notes:` continuation lines
# can push PORT STATUS more than 25 lines from EOF in real ports. 60 is
# generous without being unbounded.
TAIL=60
for f in "${files[@]}"; do
    if [ "$(wc -c < "$f")" -lt 50 ]; then continue; fi

    if ! tail -"$TAIL" "$f" | grep -q "PORT STATUS"; then
        echo "[trailer-required] FAIL: $f missing PORT STATUS trailer" >&2
        fail=1
        continue
    fi

    for field in source target_crate confidence todos port_notes unsafe_blocks notes; do
        if ! tail -"$TAIL" "$f" | grep -q "$field:"; then
            echo "[trailer-required] FAIL: $f trailer missing field '$field'" >&2
            fail=1
        fi
    done
done

exit "$fail"
