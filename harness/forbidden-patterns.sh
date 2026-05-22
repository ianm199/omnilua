# Project-specific forbidden patterns for lua-rs-port.
# Sourced by port-harness/hooks/forbidden-pattern.sh.

FORBIDDEN_PATTERNS=(
    '\b(use tokio|async fn |use futures|use rayon)'
    '(use std::process::Command|std::process::Command::|std::process::Command\s*\{|:\s*std::process::Command\b)'
)

PATH_EXCEPTIONS=(
    ''
    'crates/lua-cli/'
)
