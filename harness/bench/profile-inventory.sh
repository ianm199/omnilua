#!/usr/bin/env bash
# Print profiling and telemetry tools available for this checkout/host.
#
# This is a discovery probe. It does not run profilers or write evidence.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

row() {
  local kind="$1"
  local name="$2"
  local status="$3"
  local path="$4"
  local note="$5"
  printf '%s\t%s\t%s\t%s\t%s\n' "$kind" "$name" "$status" "$path" "$note"
}

host_tool() {
  local name="$1"
  local note="$2"
  local path
  if path="$(command -v "$name" 2>/dev/null)"; then
    row "host" "$name" "available" "$path" "$note"
  else
    row "host" "$name" "missing" "-" "$note"
  fi
}

host_path() {
  local name="$1"
  local path="$2"
  local note="$3"
  if [[ -x "$path" ]]; then
    row "host" "$name" "available" "$path" "$note"
  else
    row "host" "$name" "missing" "$path" "$note"
  fi
}

repo_tool() {
  local rel="$1"
  local note="$2"
  local path="$ROOT/$rel"
  if [[ -x "$path" ]]; then
    row "repo" "$rel" "executable" "$rel" "$note"
  elif [[ -f "$path" ]]; then
    row "repo" "$rel" "present" "$rel" "$note"
  else
    row "repo" "$rel" "missing" "$rel" "$note"
  fi
}

printf 'kind\ttool\tstatus\tpath\tnote\n'

host_path "sample" "/usr/bin/sample" "macOS wall-clock stack sampler"
host_path "xctrace" "/usr/bin/xctrace" "macOS Time Profiler capture/export"
host_path "leaks" "/usr/bin/leaks" "macOS leak/memory inspection"
host_path "dtrace" "/usr/sbin/dtrace" "system tracing; often privilege-limited"
host_tool "cargo" "Rust build/test/feature probes"
host_tool "rustc" "Rust compiler version and codegen context"
host_tool "inferno-flamegraph" "flamegraph renderer for folded stacks"
host_tool "samply" "Firefox profiler capture for Rust programs"
host_tool "perf" "Linux CPU profiler"

repo_tool "harness/bench/compare.sh" "ledgered C-vs-Rust benchmark matrix"
repo_tool "harness/bench/profile-hotspots.sh" "sample wrapper plus VM source-region attribution"
repo_tool "harness/bench/vm-execute-attribution.py" "parse sample output into VM buckets"
repo_tool "harness/bench/opcode-profile.sh" "feature-gated opcode execution counts"
repo_tool "harness/bench/gc-profile.sh" "end-of-run collector counters"
repo_tool "harness/bench/value-layout.sh" "Rust-vs-C layout size probe"
repo_tool "harness/bench/history.py" "dashboard/history builder"
