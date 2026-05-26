#!/usr/bin/env bash
# run_canaries.sh — run each gc canary under both incremental and generational modes.
#
# Emits: harness/canaries/gc/results.tsv with one row per (canary, mode):
#   canary<TAB>mode<TAB>PASS|FAIL<TAB>summary
#
# Summary is the first non-blank line of stderr/stdout on FAIL, or "-" on PASS.

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
BIN="$ROOT/target/debug/lua-rs"
DIR="$ROOT/harness/canaries/gc"
TSV="$DIR/results.tsv"
: > "$TSV"

if [ ! -x "$BIN" ]; then
    echo "[err] lua-rs binary not built at $BIN" >&2; exit 2
fi

PREAMBLE='_soft = true
_port = true
_nomsg = true
arg = arg or {}
_G = _G or _ENV
if _VERSION == nil then _VERSION = "Lua 5.4" end
'

for f in "$DIR"/canary_*.lua; do
    base=$(basename "$f" .lua)
    for mode in incremental generational; do
        mode_setup=""
        if [ "$mode" = "generational" ]; then
            mode_setup='collectgarbage("generational")
'
        fi
        outfile="$DIR/${base}.${mode}.out"
        src_file=$(mktemp "${TMPDIR:-/tmp}/lua-rs-gc-canary.XXXXXX.lua")
        {
            printf '%s' "$PREAMBLE"
            printf '%s' "$mode_setup"
            cat "$f"
        } > "$src_file"
        "$BIN" "$src_file" > "$outfile" 2>&1 &
        _pid=$!
        ( sleep 30 && kill -KILL "$_pid" 2>/dev/null ) &
        _watcher=$!
        wait "$_pid"
        rc=$?
        kill "$_watcher" 2>/dev/null; wait "$_watcher" 2>/dev/null || true
        rm -f "$src_file"
        if [ "$rc" = "0" ] && grep -q "^PASS " "$outfile"; then
            printf '%s\t%s\tPASS\t-\n' "$base" "$mode" >> "$TSV"
            printf "  %-30s %-15s PASS\n" "$base" "$mode"
        else
            summary=$(head -3 "$outfile" | tr '\n' ' ' | head -c 120)
            printf '%s\t%s\tFAIL\t%s\n' "$base" "$mode" "$summary" >> "$TSV"
            printf "  %-30s %-15s FAIL  %s\n" "$base" "$mode" "$summary"
        fi
    done
done

echo "---"
echo "TSV: $TSV"
