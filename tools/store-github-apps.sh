#!/usr/bin/env bash
# Build the GITHUB_APPS JSON list from per-App files and write it to the
# local and production reactorcide stores as
# catalystcommunity/vcsstar:<instance>-github-apps. Prints no secret.
#
#   tools/store-github-apps.sh <instance> <prefix> <n> [<n>...]
#
# For each <n> it reads <prefix>app_id<n>, <prefix>client_id<n>,
# <prefix>client_secret<n>, and <prefix>private_key<n> (the PEM). Each App
# is checked with GitHub (`GET /app` with an App JWT), which also gives
# its slug. The App's webhook secret comes from <prefix>webhook_secret<n>,
# or from the file named by WEBHOOK_SECRET_FILE for every App without its
# own; vcsstar then accepts the App's deliveries for any repository.
# Example:
#   WEBHOOK_SECRET_FILE=~/.config/vcsstar/drill-state/gh-webhook.secret \
#     tools/store-github-apps.sh catalystsquad ./temp_ 1 2
set -euo pipefail
INSTANCE="${1:?instance}"; PREFIX="${2:?file prefix}"; shift 2
[[ $# -ge 1 ]] || { echo "name at least one App number" >&2; exit 1; }
API="${GITHUB_API:-https://api.github.com}"
umask 077
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }
for n in "$@"; do
    for f in app_id client_id client_secret private_key; do
        [[ -s "${PREFIX}${f}${n}" ]] || { echo "missing or empty ${PREFIX}${f}${n}" >&2; exit 1; }
    done
    app_id="$(tr -d '[:space:]' < "${PREFIX}app_id${n}")"
    now=$(date +%s)
    header=$(printf '{"alg":"RS256","typ":"JWT"}' | b64url)
    payload=$(printf '{"iat":%d,"exp":%d,"iss":"%s"}' $((now-60)) $((now+540)) "$app_id" | b64url)
    sig=$(printf '%s.%s' "$header" "$payload" | openssl dgst -sha256 -sign "${PREFIX}private_key${n}" -binary | b64url)
    curl -fsS -H "Authorization: Bearer ${header}.${payload}.${sig}" -H "Accept: application/vnd.github+json" \
        "$API/app" > "$WORK/app$n.json" || { echo "GitHub refused App $n (id $app_id): check the id and key" >&2; exit 1; }
    python3 -I - "$WORK/app$n.json" "$app_id" <<'PY'
import json, sys
a = json.load(open(sys.argv[1]))
assert str(a.get("id")) == sys.argv[2], "GitHub answered for a different App id"
print(f"App {a['id']} slug {a.get('slug')} owner {a.get('owner', {}).get('login')} events {','.join(a.get('events', []))}")
print("  permissions " + ",".join(f"{k}:{v}" for k, v in sorted(a.get("permissions", {}).items())))
PY
done
python3 -I - "$PREFIX" "$WORK" "${WEBHOOK_SECRET_FILE:-}" "$@" > "$WORK/apps.json" <<'PY'
import json, os, sys
prefix, work, shared, nums = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
out = []
for n in nums:
    r = lambda f: open(f"{prefix}{f}{n}").read()
    own = f"{prefix}webhook_secret{n}"
    src = own if os.path.exists(own) else (shared or None)
    hook = open(os.path.expanduser(src)).read().strip() if src else None
    print(f"App {n}: webhook secret {'from ' + ('its own file' if src == own else 'WEBHOOK_SECRET_FILE') if hook else 'not set'}", file=sys.stderr)
    out.append({
        "webhook_secret": hook,
        "app_id": r("app_id").strip(),
        "client_id": r("client_id").strip(),
        "client_secret": r("client_secret").strip(),
        "private_key": r("private_key").strip() + "\n",
        "slug": json.load(open(f"{work}/app{n}.json")).get("slug"),
    })
json.dump(out, sys.stdout)
PY
unset REACTORCIDE_API_URL
export REACTORCIDE_SECRETS_PASSWORD="${REACTORCIDE_SECRETS_PASSWORD:-$(cat "$HOME/.reactorcide-pass")}"
token="$(reactorcide secrets get reactorcide/k8s api_token)"
KEY="${INSTANCE}-github-apps"
reactorcide secrets set catalystcommunity/vcsstar "$KEY" --stdin < "$WORK/apps.json" >/dev/null
reactorcide secrets --api-url https://reactorcide.catalystsquad.com --token "$token" \
    set catalystcommunity/vcsstar "$KEY" --stdin < "$WORK/apps.json" >/dev/null
a=$(reactorcide secrets get catalystcommunity/vcsstar "$KEY" | sha256sum | cut -c1-12)
b=$(reactorcide secrets --api-url https://reactorcide.catalystsquad.com --token "$token" get catalystcommunity/vcsstar "$KEY" | sha256sum | cut -c1-12)
[[ "$a" == "$b" ]] || { echo "the two stores differ" >&2; exit 1; }
echo "stored catalystcommunity/vcsstar:$KEY ($# Apps) in the local and production stores"
echo "next: enable GITHUB_APPS in .reactorcide/overlays/deploy-${INSTANCE}.yaml and the workflow, then deploy"
