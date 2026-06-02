#!/usr/bin/env bash
# harness/bench/profile-hotspots.sh — wall-clock stack sampler for a single
# lua-rs workload. Spawns the workload as a child, runs /usr/bin/sample
# against the PID for SAMPLE_SECONDS, then writes the aggregated top-frames
# text to harness/bench/profiles/<UTC>-<sha>-<workload>/.
#
# Wall-clock sampling, NOT pure CPU. Time in syscalls / sleep / I/O shows up
# too. For a pure CPU trace, use xctrace Time Profiler. We use sample
# because it's universally available, fast, and good enough to find the
# first 80% of hotspots in interpreter work.
#
# Prereq: build with frame pointers + debug info, or symbols will be wrong:
#   CARGO_PROFILE_RELEASE_DEBUG=true \
#   RUSTFLAGS="-C force-frame-pointers=yes" \
#     cargo build --release -p lua-cli
#
# Usage:
#   bash harness/bench/profile-hotspots.sh fibonacci
#   bash harness/bench/profile-hotspots.sh fibonacci 8   # sample 8s
#   SAMPLE_SECONDS=12 bash harness/bench/profile-hotspots.sh string_ops
#   PROFILE_LUA_EVAL='for i=1,100 do dofile("...") end' \
#     bash harness/bench/profile-hotspots.sh gc_pressure_x100 6

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOAD="${1:?usage: $0 <workload-name> [seconds]}"
SAMPLE_SECONDS="${2:-${SAMPLE_SECONDS:-6}}"
RS_BIN="$ROOT/target/release/lua-rs"
WORKLOAD_FILE="$ROOT/harness/bench/workloads/${WORKLOAD}.lua"
PROFILE_LUA_EVAL="${PROFILE_LUA_EVAL:-}"

[ -x "$RS_BIN" ] || { echo "[err] release binary missing: $RS_BIN — run cargo build --release -p lua-cli with frame pointers" >&2; exit 2; }
if [ -z "$PROFILE_LUA_EVAL" ]; then
    [ -f "$WORKLOAD_FILE" ] || { echo "[err] workload not found: $WORKLOAD_FILE" >&2; exit 2; }
fi
SAMPLE_BIN="/usr/bin/sample"
[ -x "$SAMPLE_BIN" ] || { echo "[err] $SAMPLE_BIN not found (macOS-only)" >&2; exit 2; }

TS=$(date -u +%Y%m%dT%H%M%SZ)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
OUT_DIR="$ROOT/harness/bench/profiles/${TS}-${COMMIT}-${WORKLOAD}"
mkdir -p "$OUT_DIR"
SAMPLE_OUT="$OUT_DIR/sample.txt"
SUMMARY="$OUT_DIR/summary.txt"

if [ -n "$PROFILE_LUA_EVAL" ]; then
    echo "==> spawning $RS_BIN -e <PROFILE_LUA_EVAL>" >&2
    "$RS_BIN" -e "$PROFILE_LUA_EVAL" >/dev/null 2>&1 &
else
    echo "==> spawning $RS_BIN $WORKLOAD_FILE" >&2
    "$RS_BIN" "$WORKLOAD_FILE" >/dev/null 2>&1 &
fi
PID=$!
trap 'kill "$PID" 2>/dev/null || true' EXIT

# Give the workload a moment to leave the startup phase.
sleep 0.3
if ! kill -0 "$PID" 2>/dev/null; then
    echo "[err] workload exited before sampling could begin (workload too short?)" >&2
    exit 3
fi

echo "==> sampling PID $PID for ${SAMPLE_SECONDS}s" >&2
"$SAMPLE_BIN" "$PID" "$SAMPLE_SECONDS" -file "$SAMPLE_OUT" -mayDie 2>/dev/null
echo "==> sample written: $SAMPLE_OUT ($(wc -l < "$SAMPLE_OUT") lines)" >&2

# `sample` output has a "Sort by top of stack, same collapsed" section with
# lines of the form:
#   "   2113  start  (in dyld) + ..."
#   "     2113  main  ..."
# We aggregate the leaf frames (after the colon "Call graph:" header, top-of-stack
# section) into a flat top-N list.

python3 - "$SAMPLE_OUT" "$SUMMARY" "$WORKLOAD" "$COMMIT" "$TS" "$SAMPLE_SECONDS" <<'PY'
import re, sys, pathlib

src, dst, workload, commit, ts, secs = sys.argv[1:7]
text = pathlib.Path(src).read_text(errors="replace")

# `sample` output ends with a "Sort by top of stack, same collapsed (when >= 5):"
# section. Lines are: "        <frame name>  (in <lib>)        <count>"
# (count at END of line, not start).
m = re.search(r"Sort by top of stack[^:\n]*:\s*\n(.+?)(\n\s*\n|\nBinary Images|\Z)", text, re.S)
top_section = m.group(1) if m else ""

top_lines = []
for line in top_section.splitlines():
    line = line.rstrip()
    if not line.strip():
        continue
    m = re.match(r"\s*(.+?)\s+\(in\s+(\S+?)\)\s+(\d+)\s*$", line)
    if m:
        frame = m.group(1).strip()
        lib = m.group(2)
        count = int(m.group(3))
        top_lines.append((count, frame, lib))

top_lines.sort(reverse=True)
total = sum(c for c, _, _ in top_lines) or 1

lines = []
lines.append(f"workload:        {workload}")
lines.append(f"commit:          {commit}")
lines.append(f"timestamp_utc:   {ts}")
lines.append(f"sample_seconds:  {secs}")
lines.append(f"total_samples:   {total}")
lines.append("")
lines.append("Top 25 leaf frames (wall-clock, %):")
lines.append(f"  {'count':>8}  {'pct':>6}  frame  (lib)")
for c, frame, lib in top_lines[:25]:
    pct = 100.0 * c / total
    lib_str = f"  ({lib})" if lib else ""
    lines.append(f"  {c:>8}  {pct:>5.1f}%  {frame}{lib_str}")

pathlib.Path(dst).write_text("\n".join(lines) + "\n")
print("\n".join(lines))
PY

echo "" >&2
echo "==> summary: $SUMMARY" >&2
