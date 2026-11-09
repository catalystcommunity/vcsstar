#!/usr/bin/env bash
# Shared setup for vcsstar CI jobs. Non-root profiles cannot write to
# toolchain paths owned by root, so every cache lives under $HOME.
set -euo pipefail
cd "${REACTORCIDE_CODE_DIR:-/job/src}"
export HOME="${HOME:-/tmp/home}"
mkdir -p "$HOME"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"
export CARGO_TERM_COLOR=never
export RUSTFLAGS="-D warnings"

ensure_rust() {
    if ! command -v cargo >/dev/null 2>&1; then
        curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
    fi
    rustc --version
    cargo --version
}
