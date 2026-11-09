#!/usr/bin/env bash
# Type-check and build the web UI. Node comes from webi at a pinned version
# (the runner image has webi; packages land under $HOME/.local/opt).
set -euo pipefail
cd "${REACTORCIDE_CODE_DIR:-/job/src}/ui"
export HOME="${HOME:-/tmp/home}"
mkdir -p "$HOME"
if ! command -v node >/dev/null 2>&1 || [[ "$(node --version)" != v22.* ]]; then
    webi node@22.23.2 >/dev/null
    export PATH="$HOME/.local/opt/node/bin:$PATH"
fi
node --version
npm ci --ignore-scripts
npm rebuild esbuild
npm run build
test -f dist/index.html
