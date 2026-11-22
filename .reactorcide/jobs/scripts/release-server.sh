#!/usr/bin/env bash
. "$(dirname "$0")/release-common.sh"
. "$(dirname "$0")/builder.sh"
tag=${REACTORCIDE_BRANCH:?}
version=$(release_version "$tag")
# A completed release already has its immutable server image.
status=$(curl -sS --retry 3 -o /dev/null -w '%{http_code}' \
    "https://api.github.com/repos/$GH_REPO/releases/tags/vcsstar%2Fv$version")
case "$status" in
    200) exit 0 ;;
    404) ;;
    *) echo "GitHub release lookup returned HTTP $status" >&2; exit 1 ;;
esac
ensure_builder "$work"
export DOCKER_CONFIG="$work/docker"
registry_login
image="$REGISTRY/$IMAGE_PATH"
push_image() {
    buildctl build --frontend dockerfile.v0 --local context=. --local dockerfile=. \
        --opt filename=Containerfile --opt target=runtime --opt platform=linux/amd64 \
        --opt "build-arg:VCSSTAR_VERSION=$version" --output "type=image,name=$image:$1,push=true"
}

push_image "$version"
# Reuse the completed build for the floating tag, including on a retry.
git ls-remote --tags --refs "https://github.com/$GH_REPO.git" 'refs/tags/vcsstar/v*' |
    cut -f2 | sed 's|^refs/tags/||' | sort -V > "$work/tags"
if [[ $(tail -1 "$work/tags") == "$tag" ]]; then push_image stable; fi
