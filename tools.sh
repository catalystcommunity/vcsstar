#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
bin_dir=${VCSSTAR_BIN_DIR:-"$HOME/.local/bin"}
exe=vcsstar
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) exe=vcsstar.exe ;; esac

release_target() {
    local arch
    case "$(uname -m)" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) echo "Unsupported architecture: $(uname -m)" >&2; return 1 ;;
    esac
    case "$(uname -s)" in
        Linux) echo "$arch-unknown-linux-gnu" ;;
        Darwin) echo "$arch-apple-darwin" ;;
        MINGW*|MSYS*|CYGWIN*)
            if [[ $arch == aarch64 ]]; then echo aarch64-pc-windows-gnullvm
            else echo x86_64-pc-windows-gnu; fi ;;
        *) echo "Unsupported system: $(uname -s)" >&2; return 1 ;;
    esac
}

install_release() {
    local target version archive url digest actual
    target=$(release_target)
    curl -fsSL https://api.github.com/repos/catalystcommunity/vcsstar/releases/latest -o "$work/release.json"
    version=$(jq -er '.tag_name | capture("^vcsstar/v(?<version>[0-9]+\\.[0-9]+\\.[0-9]+)$").version' "$work/release.json")
    archive="vcsstar-$version-$target.tar.gz"
    url=$(jq -er --arg name "$archive" '.assets[] | select(.name == $name) | .browser_download_url' "$work/release.json")
    digest=$(jq -er --arg name "$archive" '.assets[] | select(.name == $name) | .digest' "$work/release.json")
    curl -fsSL "$url" -o "$work/$archive"
    if command -v sha256sum >/dev/null; then
        actual=$(sha256sum < "$work/$archive")
    else
        actual=$(shasum -a 256 < "$work/$archive")
    fi
    [[ "sha256:${actual%% *}" == "$digest" ]] || { echo "Checksum mismatch for $archive" >&2; return 1; }
    tar -xzf "$work/$archive" -C "$work/bin" "$exe"
    [[ $("$work/bin/$exe" --version) == "vcsstar $version" ]]
}

case "${1:-}" in
    install-cli|build-install-cli) ;;
    *) echo "Usage: ./tools.sh {install-cli|build-install-cli}" >&2; exit 2 ;;
esac
[[ $# == 1 ]] || { echo "Expected a single command" >&2; exit 2; }
mkdir -p "$bin_dir"
work=$(mktemp -d "$bin_dir/.vcsstar-install.XXXXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin"
if [[ $1 == install-cli ]]; then
    install_release
else
    revision=$(git -C "$root" describe --tags --match 'vcsstar/v*' --always --dirty)
    VCSSTAR_VERSION="${revision#vcsstar/v}" cargo install --locked --path "$root/crates/vcsstar-cli" \
        --target-dir "${CARGO_TARGET_DIR:-$root/target}" --root "$work"
fi
mv -f "$work/bin/$exe" "$bin_dir/$exe"
"$bin_dir/$exe" --version
echo "Installed $bin_dir/$exe"
