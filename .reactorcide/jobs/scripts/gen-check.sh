#!/usr/bin/env bash
# Fail when crates/vcsstar-types or ui/src/generated differ from what the
# pinned csilgen produces from csil/vcsstar.csil.
set -euo pipefail
. "$(dirname "$0")/common.sh"
ensure_rust rustfmt
tools/csilgen.sh check
