# Shared setup for tagged CLI and server releases.
set -euo pipefail
cd "${REACTORCIDE_CODE_DIR:-/job/src}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export PATH="$work:$PATH"
export GH_REPO=catalystcommunity/vcsstar

release_version() {
    local tag=$1
    if [[ $tag =~ ^vcsstar/v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
        echo "${tag#vcsstar/v}"
    else
        echo "Invalid release tag: $tag" >&2
        return 1
    fi
}

install_gh() {
    if ! command -v gh >/dev/null; then
        local archive=gh_2.63.2_linux_amd64
        curl -fsSL --retry 3 "https://github.com/cli/cli/releases/download/v2.63.2/$archive.tar.gz" |
            tar -xz -C "$work" --strip-components=2 "$archive/bin/gh"
    fi
}

release_is_draft() {
    gh release view "$tag" --json isDraft --jq .isDraft
}
