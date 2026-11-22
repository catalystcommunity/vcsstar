#!/usr/bin/env bash
. "$(dirname "$0")/release-common.sh"
install_gh

if [[ $RELEASE_STEP == tag ]]; then
    # Calculate the version from the complete history of the tested commit.
    fetch=(--tags)
    if [[ $(git rev-parse --is-shallow-repository) == true ]]; then fetch+=(--unshallow); fi
    git fetch "${fetch[@]}" "https://github.com/$GH_REPO.git" refs/heads/main
    [[ $(git rev-parse HEAD) == "$REACTORCIDE_SHA" ]]
    if [[ $(git rev-parse FETCH_HEAD) != "$REACTORCIDE_SHA" ]]; then
        echo "A newer main commit has its own release check."
        exit 0
    fi
    curl -fsSL --retry 3 https://github.com/catalystcommunity/semver-tags/releases/download/v0.6.0/semver-tags.tar.gz |
        tar -xz -C "$work" semver-tags
    semver-tags run --dry_run --branch "" --output_json > "$work/plan"
    tag=$(python3 - "$work/plan" <<'PY'
import json, sys
with open(sys.argv[1]) as f:
    plan = json.load(f)
if plan["New_release_published"] == "true":
    print(plan["New_release_git_tag"])
PY
)
    if [[ -z $tag ]]; then echo "No release due."; exit 0; fi
    release_version "$tag" >/dev/null
    # A Git tag push sends the webhook that starts the release workflow.
    git -c credential.helper= -c 'credential.helper=!gh auth git-credential' \
        push "https://github.com/$GH_REPO.git" "$REACTORCIDE_SHA:refs/tags/$tag"
    exit 0
fi

tag=${REACTORCIDE_BRANCH:?}
version=$(release_version "$tag")
case "$RELEASE_STEP" in
    prepare)
        fetch=()
        if [[ $(git rev-parse --is-shallow-repository) == true ]]; then fetch+=(--unshallow); fi
        git fetch "${fetch[@]}" "https://github.com/$GH_REPO.git" \
            "refs/heads/main:refs/remotes/origin/main" "refs/tags/$tag:refs/tags/$tag"
        [[ $(git rev-parse "refs/tags/$tag^{commit}") == "$REACTORCIDE_SHA" ]]
        [[ $(git rev-parse HEAD) == "$REACTORCIDE_SHA" ]]
        git merge-base --is-ancestor HEAD refs/remotes/origin/main
        if ! gh release view "$tag" >/dev/null 2>&1; then
            gh release create "$tag" --verify-tag --draft --target "$REACTORCIDE_SHA" \
                --generate-notes --title "vcsstar $version"
        fi
        ;;
    publish)
        draft=$(release_is_draft)
        if [[ $draft == false ]]; then exit 0; fi
        gh release view "$tag" --json assets --jq '.assets[] | select(.state == "uploaded" and .size > 0) | .name' > "$work/assets"
        python3 - "$version" "$work/assets" <<'PY'
import sys, yaml
with open(".reactorcide/workflows/release.yaml") as f:
    targets = yaml.safe_load(f)["jobs"]["vcsstar-release-cli"]["for_each"]
with open(sys.argv[2]) as f:
    assets = set(f.read().splitlines())
missing = {f"vcsstar-{sys.argv[1]}-{target}.tar.gz" for target in targets} - assets
if missing:
    sys.exit(f"Missing release archives: {', '.join(sorted(missing))}")
PY
        # Pin the downloadable Compose file to the same image as the release.
        sed "s/VCSSTAR_TAG:-stable/VCSSTAR_TAG:-$version/" compose.yaml > "$work/compose.yaml"
        gh release upload "$tag" "$work/compose.yaml" --clobber
        gh release edit "$tag" --draft=false
        ;;
    *) echo "Unknown release step: $RELEASE_STEP" >&2; exit 1 ;;
esac
