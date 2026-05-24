#!/usr/bin/env bash
# PreToolUse hook (Edit/Write): reject the tool call if the proposed file
# content introduces a new `GcRef::new(...)` call outside the whitelist.
#
# Phase D-1d: forces all allocation through `state.new_*` (D-1a) which at
# D-1e becomes `state.global.heap.allocate(...)`. Without this hook, agents
# tend to add `GcRef::new(...)` because it "works" today — but every such
# site is invisible to the future heap and re-creates the paper-GC bug.
#
# Whitelist (where GcRef::new IS legitimate):
#   crates/lua-types/src/gc.rs        — the wrapper definition itself
#   crates/lua-vm/src/state.rs        — allocation helper bodies
#   crates/lua-vm/src/string.rs       — string interning internals
#   crates/lua-vm/src/table.rs        — table-allocation internals
#   crates/lua-vm/src/func.rs         — closure/proto helpers
#   crates/lua-vm/src/api.rs          — placeholder bootstraps
#   crates/lua-vm/src/debug.rs        — error-message construction
#   crates/lua-vm/src/undump.rs       — bytecode-loader scaffolding (D-1c-bridge sites)
#   crates/lua-types/src/error.rs     — LuaError constructors
#   crates/lua-cli/src/main.rs        — bootstrap
#   tests/                            — explicit test setup
#
# Exits:
#   0  allow
#   2  block (stderr message visible to Claude)

set -uo pipefail

PAYLOAD="$(cat)"
TOOL_NAME=""
TARGET_PATH=""
PROPOSED_CONTENT=""
OLD_CONTENT=""

if command -v jq >/dev/null 2>&1; then
    TOOL_NAME=$(echo "$PAYLOAD" | jq -r '.tool_name // empty')
    TARGET_PATH=$(echo "$PAYLOAD" | jq -r '.tool_input.file_path // empty')
    case "$TOOL_NAME" in
        Write)
            PROPOSED_CONTENT=$(echo "$PAYLOAD" | jq -r '.tool_input.content // empty')
            ;;
        Edit)
            PROPOSED_CONTENT=$(echo "$PAYLOAD" | jq -r '.tool_input.new_string // empty')
            OLD_CONTENT=$(echo "$PAYLOAD" | jq -r '.tool_input.old_string // empty')
            ;;
        *) exit 0 ;;
    esac
fi

[ -z "$TARGET_PATH" ] && exit 0

# Whitelist (substring matches)
case "$TARGET_PATH" in
    */lua-types/src/gc.rs|*/lua-vm/src/state.rs|*/lua-vm/src/string.rs|*/lua-vm/src/table.rs|*/lua-vm/src/func.rs|*/lua-vm/src/api.rs|*/lua-vm/src/debug.rs|*/lua-vm/src/undump.rs|*/lua-types/src/error.rs|*/lua-cli/src/main.rs)
        exit 0 ;;
    */tests/*|*/test_*|*_test.rs)
        exit 0 ;;
esac

# Count new GcRef::new occurrences. For Edit, only count NEW occurrences
# (not present in old_string), so refactoring an existing one isn't blocked.
new_count=$(echo "$PROPOSED_CONTENT" | grep -c 'GcRef::new(' 2>/dev/null || echo 0)
old_count=0
if [ "$TOOL_NAME" = "Edit" ]; then
    old_count=$(echo "$OLD_CONTENT" | grep -c 'GcRef::new(' 2>/dev/null || echo 0)
fi

if [ "$new_count" -gt "$old_count" ]; then
    cat >&2 <<EOF
PRE-TOOLUSE BLOCKED: $TARGET_PATH introduces a new GcRef::new(...) call.

Phase D-1d forbids GcRef::new outside the allocator whitelist. Use the
state-owned helper instead:
  state.new_table()           — allocate a LuaTable
  state.new_string(bytes)     — allocate / intern a LuaString
  state.new_proto()           — allocate a LuaProto
  state.new_lclosure(p, n)    — allocate a Lua closure
  state.new_upval_closed(v)   — allocate a closed upvalue
  state.new_upval_open(t, l)  — allocate an open upvalue

If &mut LuaState is not in scope at this site, mark the site:
  // TODO(D-1c-bridge): allocation outside state context
  let x = GcRef::new(...);  // FIXME after D-1e: route via with_current_heap(...)
and surface it for the agent loop to migrate later.

File: $TARGET_PATH
new GcRef::new count: $new_count (was $old_count)
EOF
    exit 2
fi

exit 0
