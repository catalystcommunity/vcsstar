#!/usr/bin/env bash
# Deploy an instance from this machine through reactorcide run-local, with
# secrets from the LOCAL reactorcide store. CI does the same job with the
# production store (.reactorcide/workflows/deploy-<instance>.yaml).
#
#   tools/deploy.sh dev <instance>       dev build of the working tree
#   tools/deploy.sh release <instance>   <version>-<sha> build, moves :latest
#
# <instance> names .reactorcide/overlays/deploy-<instance>.yaml. The store
# password comes from REACTORCIDE_SECRETS_PASSWORD or ~/.reactorcide-pass.
set -euo pipefail
MODE="${1:-}"; INSTANCE="${2:-}"
case "$MODE" in dev|release) ;; *) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;; esac
cd "$(dirname "$0")/.."
OVERLAY=".reactorcide/overlays/deploy-${INSTANCE}.yaml"
[[ -f "$OVERLAY" ]] || { echo "no instance overlay $OVERLAY" >&2; exit 1; }
if [[ -z "${REACTORCIDE_SECRETS_PASSWORD:-}" ]]; then
    REACTORCIDE_SECRETS_PASSWORD="$(cat "$HOME/.reactorcide-pass")"
fi
export REACTORCIDE_SECRETS_PASSWORD
# `secrets get` and run-local read the API instead of the local store when
# this is set.
unset REACTORCIDE_API_URL
ARGS=(-i "$OVERLAY")
[[ "$MODE" == dev ]] && ARGS=(-i .reactorcide/overlays/dev.yaml "${ARGS[@]}")
exec reactorcide run-local --source-dir . "${ARGS[@]}" .reactorcide/jobs/deploy.yaml
