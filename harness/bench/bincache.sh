#!/usr/bin/env bash
# bincache.sh — commit-keyed release-binary cache for A/B baselines.
#
# Prints the path to a cached release lua-rs built at <ref> (default HEAD),
# building it once into /tmp/lua-rs-bincache/<sha>-lua-rs if missing. Kills
# the stash->rebuild->swap dance every A/B used to need for its base binary
# (1-2 min + a stop-hook hazard each time; the 2026-06-10 audit found ~6
# repetitions in one afternoon).
#
# Builds happen in a detached worktree so the working tree (and any dirty
# experiment state) is never touched.
#
# Usage:
#   BASE=$(bash harness/bench/bincache.sh HEAD~1)
#   bash harness/bench/compare_bins.sh --a "$BASE" --b target/release/lua-rs ...
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
REF="${1:-HEAD}"
SHA=$(git rev-parse --short=12 "$REF")
CACHE_DIR=/tmp/lua-rs-bincache
BIN="$CACHE_DIR/$SHA-lua-rs"
if [ -x "$BIN" ]; then
    echo "$BIN"
    exit 0
fi
mkdir -p "$CACHE_DIR"
WT="$CACHE_DIR/wt-$SHA"
trap 'git worktree remove --force "$WT" >/dev/null 2>&1 || true' EXIT
git worktree add --detach "$WT" "$SHA" >/dev/null 2>&1
( cd "$WT" && cargo build --release -p lua-cli -q )
cp "$WT/target/release/lua-rs" "$BIN"
echo "$BIN"
