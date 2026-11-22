#!/usr/bin/env bash
. "$(dirname "$0")/release-common.sh"
. "$(dirname "$0")/builder.sh"
tag=${REACTORCIDE_BRANCH:?}
version=$(release_version "$tag")
install_gh
draft=$(release_is_draft)
if [[ $draft == false ]]; then exit 0; fi

ensure_builder "$work"
buildctl build --frontend dockerfile.v0 --local context=. --local dockerfile=.reactorcide \
    --opt filename=Containerfile.cli --opt "build-arg:TARGET=$TARGET" \
    --opt "build-arg:VCSSTAR_VERSION=$version" --output "type=local,dest=$work/bin"

exe=vcsstar
if [[ $TARGET == *-windows-* ]]; then exe=vcsstar.exe; fi
if [[ $TARGET == x86_64-unknown-linux-gnu ]]; then
    [[ $("$work/bin/$exe" --version) == "vcsstar $version" ]]
fi
archive="$work/vcsstar-$version-$TARGET.tar.gz"
tar -czf "$archive" -C "$work/bin" "$exe" -C "$PWD" LICENSE README.md
gh release upload "$tag" "$archive" --clobber
