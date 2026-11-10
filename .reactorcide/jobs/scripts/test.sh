#!/usr/bin/env bash
# Build and test the workspace. Database tests need Postgres; the job starts
# one when docker is available and skips those tests otherwise.
set -euo pipefail
. "$(dirname "$0")/common.sh"
ensure_rust
# The runner image has no libpq; link the bundled copy (CI only).
FEATURES=(--features vcsstar-db/bundled-pq)
cargo fmt --all -- --check || { echo "run cargo fmt --all"; exit 1; }
cargo build --workspace --locked "${FEATURES[@]}"
cargo test --workspace --locked "${FEATURES[@]}"
cargo clippy --workspace --all-targets --locked "${FEATURES[@]}" -- -D warnings
