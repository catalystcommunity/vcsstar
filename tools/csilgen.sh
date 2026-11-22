#!/usr/bin/env bash
# Pinned csilgen for this repository. Generators load from ./.generators
# (a link into .deps/), never from $HOME/.csilgen.
#
#   tools/csilgen.sh ensure     install the pinned CLI and generators under .deps/
#   tools/csilgen.sh generate   regenerate crates/vcsstar-types and ui/src/generated
#   tools/csilgen.sh check      fail when generated output differs from the tree
#   tools/csilgen.sh validate   validate csil/vcsstar.csil only
set -euo pipefail

CSILGEN_PIN="0.2.15"
# Commit of tag csilgen/v${CSILGEN_PIN}; Cargo.toml pins csilgen-transport
# to the same commit.
CSILGEN_COMMIT="9690ed8794454c2beb21571ce525ffd62a2f0701"
SHA_CLI_X86_64="a322246036d25ee013ad108ade04a4148fe0aab6cb5fd7522c757a4bb57817d8"
SHA_CLI_AARCH64="a35571539e2f451253639a574eb617e76c11675d21bbc2b7c330b0e189fb62a4"
SHA_GENERATORS="89c6d6ea88fc3ea5af7c9fb1fe30b87ddd166b2b8a6d6ce4c612791e97de9a75"
RELEASE_BASE="https://github.com/catalystcommunity/csilgen/releases/download/csilgen/v${CSILGEN_PIN}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPS="$ROOT/.deps"
BIN="$DEPS/bin/csilgen"
GEN_DIR="$DEPS/csilgen-generators/$CSILGEN_PIN"
SPEC="$ROOT/csil/vcsstar.csil"
RUST_OUT="$ROOT/crates/vcsstar-types"
TS_OUT="$ROOT/ui/src/generated"

sha_check() { echo "$2  $1" | sha256sum -c - >/dev/null; }

fetch() {
    local url="$1" dest="$2" sha="$3"
    if [ -f "$dest" ] && sha_check "$dest" "$sha"; then return 0; fi
    curl -fsSL "$url" -o "$dest.tmp"
    sha_check "$dest.tmp" "$sha" || { echo "checksum mismatch for $url" >&2; rm -f "$dest.tmp"; exit 1; }
    mv "$dest.tmp" "$dest"
}

ensure() {
    mkdir -p "$DEPS/bin" "$DEPS/dl" "$GEN_DIR"
    local arch sha
    case "$(uname -m)" in
        x86_64) arch=x86_64; sha="$SHA_CLI_X86_64" ;;
        aarch64|arm64) arch=aarch64; sha="$SHA_CLI_AARCH64" ;;
        *) echo "unsupported arch $(uname -m)" >&2; exit 1 ;;
    esac
    if ! { [ -x "$BIN" ] && [ "$("$BIN" --version 2>/dev/null)" = "csilgen $CSILGEN_PIN" ]; }; then
        local tgz="$DEPS/dl/csilgen-$CSILGEN_PIN-$arch-unknown-linux-gnu.tar.gz"
        fetch "$RELEASE_BASE/csilgen-$CSILGEN_PIN-$arch-unknown-linux-gnu.tar.gz" "$tgz" "$sha"
        tar -xzf "$tgz" -C "$DEPS/bin" --wildcards --no-anchored 'csilgen' --strip-components=0 2>/dev/null \
            || tar -xzf "$tgz" -C "$DEPS/bin"
        if [ ! -x "$BIN" ]; then
            found="$(find "$DEPS/bin" -type f -name csilgen | head -1)"
            [ -n "$found" ] && mv "$found" "$BIN"
        fi
        chmod +x "$BIN"
        [ "$("$BIN" --version)" = "csilgen $CSILGEN_PIN" ] || { echo "installed csilgen is not $CSILGEN_PIN" >&2; exit 1; }
    fi
    if [ ! -f "$GEN_DIR/.complete" ]; then
        local gtgz="$DEPS/dl/csilgen-generators-$CSILGEN_PIN.tar.gz"
        fetch "$RELEASE_BASE/csilgen-generators-$CSILGEN_PIN.tar.gz" "$gtgz" "$SHA_GENERATORS"
        rm -rf "$GEN_DIR"; mkdir -p "$GEN_DIR"
        tar -xzf "$gtgz" -C "$GEN_DIR"
        # Flatten if the tarball has a top-level directory.
        if ! ls "$GEN_DIR"/*.wasm >/dev/null 2>&1; then
            find "$GEN_DIR" -name '*.wasm' -exec mv {} "$GEN_DIR"/ \;
        fi
        touch "$GEN_DIR/.complete"
    fi
    ln -sfn "$GEN_DIR" "$ROOT/csil/.generators"
}

run_csilgen() {
    # csilgen looks for ./.generators relative to its working directory.
    (cd "$ROOT/csil" && "$BIN" "$@")
}

generate_into() {
    local rust_out="$1" ts_out="$2"
    rm -rf "$rust_out" "$ts_out"
    mkdir -p "$rust_out" "$ts_out"
    run_csilgen generate --quiet --input vcsstar.csil --target rust --output "$rust_out"
    (cd "$ROOT" && rustfmt --edition 2021 "$rust_out/src/lib.rs")
    run_csilgen generate --quiet --input vcsstar.csil --target typescript --output "$ts_out"
    # The Rust generator writes a crate manifest without lints; the workspace
    # supplies them. Keep the manifest as generated.
}

validate() { ensure; run_csilgen validate --input vcsstar.csil; }

generate() { ensure; validate; generate_into "$RUST_OUT" "$TS_OUT"; echo "generated into $RUST_OUT and $TS_OUT"; }

check() {
    ensure; validate
    CHECK_TMP="$(mktemp -d)"
    trap 'rm -rf "$CHECK_TMP"' EXIT
    local tmp="$CHECK_TMP"
    generate_into "$tmp/rust" "$tmp/ts"
    local rc=0
    diff -r "$tmp/rust" "$RUST_OUT" >/dev/null || rc=1
    diff -r "$tmp/ts" "$TS_OUT" >/dev/null || rc=1
    if [ $rc -ne 0 ]; then
        echo "generated code is stale. Run tools/csilgen.sh generate and commit the result." >&2
        diff -r "$tmp/rust" "$RUST_OUT" | head -20 >&2 || true
        exit 1
    fi
    echo "generated code is current"
}

case "${1:-}" in
    ensure) ensure ;;
    generate) generate ;;
    check) check ;;
    validate) validate ;;
    *) sed -n 2,9p "$0"; exit 2 ;;
esac
