#!/usr/bin/env bash
# Fail when crates/vscstar-types or ui/src/generated differ from what the
# pinned csilgen produces from csil/vscstar.csil.
set -euo pipefail
. "$(dirname "$0")/common.sh"
tools/csilgen.sh check
