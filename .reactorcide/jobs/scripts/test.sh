#!/usr/bin/env bash
# Build and test the workspace. Database tests need Postgres; the job starts
# one when docker is available and skips those tests otherwise.
set -euo pipefail
. "$(dirname "$0")/common.sh"
ensure_rust
cargo fmt --all -- --check || { echo "run cargo fmt --all"; exit 1; }
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
