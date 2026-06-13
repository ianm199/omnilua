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
#     cargo build --release -p omnilua-cli
#
# Usage:
#   bash harness/bench/profile-hotspots.sh fibonacci
#   bash harness/bench/profile-hotspots.sh fibonacci 8   # sample 8s
#   SAMPLE_SECONDS=12 bash harness/bench/profile-hotspots.sh string_ops
#   PROFILE_REPEAT=30 bash harness/bench/profile-hotspots.sh closure_ops 8
#   PROFILE_LUA_EVAL='for i=1,100 do dofile("...") end' \
#     bash harness/bench/profile-hotspots.sh gc_pressure_x100 6

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

WORKLOAD="${1:?usage: $0 <workload-name> [seconds]}"
SAMPLE_SECONDS="${2:-${SAMPLE_SECONDS:-6}}"
RS_BIN="$ROOT/target/release/omnilua"
WORKLOAD_FILE="$ROOT/harness/bench/workloads/${WORKLOAD}.lua"
PROFILE_LUA_EVAL="${PROFILE_LUA_EVAL:-}"
PROFILE_REPEAT="${PROFILE_REPEAT:-1}"

case "$PROFILE_REPEAT" in
    ''|*[!0-9]*) echo "[err] PROFILE_REPEAT must be a positive integer" >&2; exit 2 ;;
    0)           echo "[err] PROFILE_REPEAT must be >= 1" >&2; exit 2 ;;
esac

[ -x "$RS_BIN" ] || { echo "[err] release binary missing: $RS_BIN — run cargo build --release -p omnilua-cli with frame pointers" >&2; exit 2; }
if [ -z "$PROFILE_LUA_EVAL" ]; then
    [ -f "$WORKLOAD_FILE" ] || { echo "[err] workload not found: $WORKLOAD_FILE" >&2; exit 2; }
fi
SAMPLE_BIN="/usr/bin/sample"
[ -x "$SAMPLE_BIN" ] || { echo "[err] $SAMPLE_BIN not found (macOS-only)" >&2; exit 2; }

TS=$(date -u +%Y%m%dT%H%M%SZ)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
WORKLOAD_LABEL="$WORKLOAD"
if [ -z "$PROFILE_LUA_EVAL" ] && [ "$PROFILE_REPEAT" -gt 1 ]; then
    escaped_workload=${WORKLOAD_FILE//\\/\\\\}
    escaped_workload=${escaped_workload//\"/\\\"}
    PROFILE_LUA_EVAL="for _profile_i = 1, ${PROFILE_REPEAT} do dofile(\"${escaped_workload}\") end"
    WORKLOAD_LABEL="${WORKLOAD}_x${PROFILE_REPEAT}"
fi

OUT_DIR="$ROOT/harness/bench/profiles/${TS}-${COMMIT}-${WORKLOAD_LABEL}"
mkdir -p "$OUT_DIR"
SAMPLE_OUT="$OUT_DIR/sample.txt"
SUMMARY="$OUT_DIR/summary.txt"
VM_EXECUTE="$OUT_DIR/vm-execute.txt"

# The workload is spawned DIRECTLY (not via with-timeout.sh) because
# /usr/bin/sample needs the interpreter's own PID — a wrapper pid samples as
# 100% __wait4. Its three std fds go to /dev/null (also < /dev/null) so it can
# never inherit and hold open this script's stdout/stderr pipe.
#
# AGENT-SAFETY (2026-06-11): the previous version armed a detached
#   ( sleep "$PROFILE_MAX_S" && kill ... ) &
# watchdog. That subshell INHERITED the script's stdout/stderr, so any reader
# of this script's output (a `| tail` pipe, or the agent harness's process-
# group drain) blocked on EOF until the 600s `sleep` finished — the script's
# real work completed and wrote sample.txt/summary.txt, but the caller never
# saw the script "return" for up to 10 minutes (observed: 0-byte/no-artifact
# stalls under the harness). The EXIT trap's `kill "$KILLER"` did not help:
# killing the subshell orphans its already-running `sleep` child.
#
# The bound is `sample`'s own duration argument: it self-terminates after
# $SAMPLE_SECONDS regardless of the workload, and the workload is killed the
# instant sampling returns. PROFILE_MAX_S stays as a hard ceiling on a runaway
# workload, but the watchdog is now (a) detached from this script's std fds
# — `< /dev/null > /dev/null 2>&1` — so it can NEVER hold the script's
# stdout/stderr pipe open (the actual stall cause), and (b) reaped together
# with its `sleep` child (via `pkill -P`) on every exit path. macOS has no
# `setsid`, so the process-group trick is unavailable; the fd detach is what
# fixes the harness stall, and -P reaping keeps no orphan `sleep` behind.
PROFILE_MAX_S="${PROFILE_MAX_S:-600}"
if [ -n "$PROFILE_LUA_EVAL" ]; then
    echo "==> spawning $RS_BIN -e <PROFILE_LUA_EVAL> ($WORKLOAD_LABEL)" >&2
    "$RS_BIN" -e "$PROFILE_LUA_EVAL" </dev/null >/dev/null 2>&1 &
else
    echo "==> spawning $RS_BIN $WORKLOAD_FILE" >&2
    "$RS_BIN" "$WORKLOAD_FILE" </dev/null >/dev/null 2>&1 &
fi
PID=$!

WATCHDOG=""
reap() {
    kill "$PID" 2>/dev/null || true
    if [ -n "$WATCHDOG" ]; then
        pkill -P "$WATCHDOG" 2>/dev/null || true
        kill "$WATCHDOG" 2>/dev/null || true
    fi
}
trap reap EXIT

( sleep "$PROFILE_MAX_S" && kill -KILL "$PID" 2>/dev/null ) </dev/null >/dev/null 2>&1 &
WATCHDOG=$!

# Give the workload a moment to leave the startup phase.
sleep 0.3
if ! kill -0 "$PID" 2>/dev/null; then
    echo "[err] workload exited before sampling could begin (workload too short? use PROFILE_REPEAT)" >&2
    exit 3
fi

echo "==> sampling PID $PID for ${SAMPLE_SECONDS}s" >&2
"$SAMPLE_BIN" "$PID" "$SAMPLE_SECONDS" -file "$SAMPLE_OUT" -mayDie 2>/dev/null
echo "==> sample written: $SAMPLE_OUT ($(wc -l < "$SAMPLE_OUT") lines)" >&2

# Sampling done: kill the workload and the watchdog (with its sleep child)
# immediately so neither outlives this script or holds any fd open.
reap
WATCHDOG=""

# `sample` output has a "Sort by top of stack, same collapsed" section with
# lines of the form:
#   "   2113  start  (in dyld) + ..."
#   "     2113  main  ..."
# We aggregate the leaf frames (after the colon "Call graph:" header, top-of-stack
# section) into a flat top-N list.

python3 - "$SAMPLE_OUT" "$SUMMARY" "$WORKLOAD_LABEL" "$COMMIT" "$TS" "$SAMPLE_SECONDS" <<'PY'
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

if grep -q "lua_vm::vm::execute" "$SAMPLE_OUT"; then
    python3 "$ROOT/harness/bench/vm-execute-attribution.py" \
        "$SAMPLE_OUT" \
        --source "$ROOT/crates/lua-vm/src/vm.rs" \
        --output "$VM_EXECUTE"
    if grep -q '^warning:' "$VM_EXECUTE"; then
        sed -n 's/^warning: /[warn] /p' "$VM_EXECUTE" >&2
    fi
    echo "==> vm execute attribution: $VM_EXECUTE" >&2
fi
