#!/usr/bin/env bash
# table-bytes.sh — per-shape live heap bytes per table, lua-rs vs reference C.
#
# The W2.3 decomposition (PERFORMANCE_MODEL.md "RSS decomposition") showed the
# RSS gap is OBJECT SIZE at allocation-count parity. This is the before/after
# instrument for representation-diet packets: for each canonical table shape
# it allocates N tables, holds them live, and reports (peak - baseline)/N
# bytes per table on both implementations.
#
# Sides:
#   - lua-rs: a `--features dhat-heap` build (pass with --rs-bin, or built
#     fresh); per-shape bytes come from dhat's t-gmax line.
#   - C: a counting l_alloc patched into a COPY of the reference at
#     /tmp/lua-counting (never reference/ itself). Rebuilt here if missing;
#     per-shape bytes come from its ALLOCSTATS peak.
#
# Usage:
#   bash harness/bench/table-bytes.sh                # all shapes
#   bash harness/bench/table-bytes.sh --rs-bin /tmp/lua-rs-dhat
#
# Caveat: dhat reports allocator-request bytes (like the C side), not
# malloc-bucket-rounded RSS; both sides are measured the same way, so the
# ratio is fair, but absolute RSS will round up to malloc size classes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

RS_BIN=""
while [ $# -gt 0 ]; do
    case "$1" in
        --rs-bin) RS_BIN="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# //; s/^#//'
            exit 0 ;;
        *) echo "unknown flag: $1" >&2; exit 1 ;;
    esac
done

C_COUNT_DIR=/tmp/lua-counting
if [ ! -x "$C_COUNT_DIR/src/lua" ]; then
    echo "[setup] building counting-allocator reference copy in $C_COUNT_DIR" >&2
    rm -rf "$C_COUNT_DIR"
    cp -R "$ROOT/reference/lua-5.4.7" "$C_COUNT_DIR"
    make -C "$C_COUNT_DIR" clean -s >/dev/null 2>&1 || true
    python3 - "$C_COUNT_DIR/src/lauxlib.c" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
old = """static void *l_alloc (void *ud, void *ptr, size_t osize, size_t nsize) {
  (void)ud; (void)osize;  /* not used */
  if (nsize == 0) {
    free(ptr);
    return NULL;
  }
  else
    return realloc(ptr, nsize);
}"""
new = """static unsigned long long g_nalloc, g_nrealloc, g_nfree;
static unsigned long long g_total, g_live, g_peak;

static void print_alloc_stats (void) {
  fprintf(stderr,
    "ALLOCSTATS nalloc=%llu nrealloc=%llu nfree=%llu total=%llu peak=%llu live=%llu\\n",
    g_nalloc, g_nrealloc, g_nfree, g_total, g_peak, g_live);
}

static int g_stats_registered = 0;

static void *l_alloc (void *ud, void *ptr, size_t osize, size_t nsize) {
  size_t old = (ptr == NULL) ? 0 : osize;
  (void)ud;
  if (!g_stats_registered) { g_stats_registered = 1; atexit(print_alloc_stats); }
  if (nsize == 0) {
    if (ptr) { g_nfree++; g_live -= old; }
    free(ptr);
    return NULL;
  }
  if (ptr == NULL) g_nalloc++; else g_nrealloc++;
  if (nsize > old) g_total += nsize - old;
  g_live += nsize; g_live -= old;
  if (g_live > g_peak) g_peak = g_live;
  return realloc(ptr, nsize);
}"""
assert old in s, "l_alloc anchor not found (already patched?)"
open(p, "w").write(s.replace(old, new))
PY
    make -C "$C_COUNT_DIR" macosx -s -j4 >/dev/null
fi
C_BIN="$C_COUNT_DIR/src/lua"

if [ -z "$RS_BIN" ]; then
    echo "[setup] building lua-rs with --features dhat-heap" >&2
    cargo build --release -p lua-cli --features dhat-heap -q
    RS_BIN="$ROOT/target/release/lua-rs"
fi

N=200000
SNIP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/table-bytes-XXXXXX")
trap 'rm -rf "$SNIP_DIR"' EXIT

write_snip() {
    local name="$1" expr="$2"
    cat > "$SNIP_DIR/$name.lua" <<LUA
local N = $N
local keep = {}
for i = 1, N do
    keep[i] = $expr
end
collectgarbage("collect")
LUA
}

write_snip baseline "false"
write_snip empty "{}"
write_snip record3 "{item = i, left = false, right = false}"
write_snip record8 "{a=1,b=2,c=3,d=4,e=5,f=6,g=7,h=8}"
write_snip array3 "{i, i + 1, i + 2}"
write_snip array8 "{1, 2, 3, 4, 5, 6, 7, 8, i}"
write_snip mixed "{i, i + 1, x = i}"

c_peak() {
    "$C_BIN" "$1" 2>&1 >/dev/null | awk -F'peak=' '/ALLOCSTATS/ {print $2}' | awk '{print $1}'
}
rs_peak() {
    (cd "$SNIP_DIR" && "$RS_BIN" "$1" >/dev/null 2>gmax.txt)
    awk '/t-gmax/ {gsub(",", "", $4); print $4}' "$SNIP_DIR/gmax.txt"
}

c_base=$(c_peak "$SNIP_DIR/baseline.lua")
rs_base=$(rs_peak "$SNIP_DIR/baseline.lua")

printf '%-10s %14s %14s %8s\n' shape c_bytes/table rs_bytes/table ratio
for shape in empty record3 record8 array3 array8 mixed; do
    cp=$(c_peak "$SNIP_DIR/$shape.lua")
    rp=$(rs_peak "$SNIP_DIR/$shape.lua")
    cb=$(( (cp - c_base) / N ))
    rb=$(( (rp - rs_base) / N ))
    ratio=$(python3 -c "print(f'{$rb/$cb:.2f}')" 2>/dev/null || echo "-")
    printf '%-10s %14s %14s %8s\n' "$shape" "$cb" "$rb" "$ratio"
done
