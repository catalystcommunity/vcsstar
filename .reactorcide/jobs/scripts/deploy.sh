#!/usr/bin/env bash
# Build and push the vcsstar images, then helm upgrade the instance.
# Secrets arrive in the environment; this script never prints them.
set -euo pipefail
cd "${REACTORCIDE_REPOROOT:-${REACTORCIDE_CODE_DIR:-/job/src}}"

SHA="${REACTORCIDE_SHA:-$(git rev-parse HEAD)}"
BASE_VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
VERSION="${BASE_VERSION}-${SHA:0:12}"
echo "version ${VERSION}"

export HOME="${HOME:-/home/runner}"
LOCAL_BIN="$HOME/.local/bin"
mkdir -p "$HOME/.docker" "$LOCAL_BIN"
export PATH="$LOCAL_BIN:$PATH"

if ! command -v buildctl >/dev/null 2>&1; then
    BUILDKIT_VERSION=0.17.3
    curl -fsSL "https://github.com/moby/buildkit/releases/download/v${BUILDKIT_VERSION}/buildkit-v${BUILDKIT_VERSION}.linux-amd64.tar.gz" -o /tmp/buildkit.tar.gz
    tar -xzf /tmp/buildkit.tar.gz -C "$LOCAL_BIN" --strip-components=1 bin/buildctl
    rm /tmp/buildkit.tar.gz
fi
if ! command -v helm >/dev/null 2>&1; then
    curl -fsSL https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | USE_SUDO=false HELM_INSTALL_DIR="$LOCAL_BIN" bash
fi
if ! command -v kubectl >/dev/null 2>&1; then
    KUBECTL_VERSION=$(curl -fsSL https://dl.k8s.io/release/stable.txt)
    curl -fsSL "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/amd64/kubectl" -o "$LOCAL_BIN/kubectl"
    chmod +x "$LOCAL_BIN/kubectl"
fi

for i in $(seq 1 30); do
    if buildctl debug info >/dev/null 2>&1; then break; fi
    if [ "$i" -eq 30 ]; then echo "builder sidecar not ready" >&2; exit 1; fi
    sleep 1
done

AUTH=$(printf "%s:%s" "$REGISTRY_USER" "$REGISTRY_PASSWORD" | base64 -w 0)
cat > "$HOME/.docker/config.json" <<JSON
{ "auths": { "${REGISTRY}": { "auth": "${AUTH}" } } }
JSON
export DOCKER_CONFIG="$HOME/.docker"

IMAGE="${REGISTRY}/${IMAGE_PATH}"
CLI_IMAGE="${REGISTRY}/${CLI_IMAGE_PATH}"
echo "building ${IMAGE}:${VERSION}"
buildctl build \
    --frontend dockerfile.v0 \
    --opt filename=Containerfile \
    --opt target=runtime \
    --local context=. \
    --local dockerfile=. \
    --output "type=image,\"name=${IMAGE}:${VERSION},${IMAGE}:latest\",push=true"
echo "building ${CLI_IMAGE}:${VERSION}"
buildctl build \
    --frontend dockerfile.v0 \
    --opt filename=Containerfile \
    --opt target=cli \
    --local context=. \
    --local dockerfile=. \
    --output "type=image,\"name=${CLI_IMAGE}:${VERSION},${CLI_IMAGE}:latest\",push=true"

mkdir -p ~/.kube
printf '%s' "${KUBECONFIG_CONTENT}" > ~/.kube/config
chmod 600 ~/.kube/config

kubectl create namespace "${K8S_NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f -
kubectl create secret docker-registry regcred \
    --namespace "${K8S_NAMESPACE}" --save-config --dry-run=client \
    --docker-server="${REGISTRY}" --docker-username="${REGISTRY_USER}" --docker-password="${REGISTRY_PASSWORD}" \
    -o yaml | kubectl apply -f -
kubectl create secret generic vcsstar-linkkeys \
    --namespace "${K8S_NAMESPACE}" --save-config --dry-run=client \
    --from-literal=SECRET_VCSSTAR__LINKKEYS_API_KEY="${LINKKEYS_API_KEY}" \
    -o yaml | kubectl apply -f -
# The instance's GitHub App, when the store has one (set githubApp.existingSecret
# to vcsstar-github-app in the overlay).
if [ -n "${GITHUB_APP_ID:-}" ]; then
    kubectl create secret generic vcsstar-github-app \
        --namespace "${K8S_NAMESPACE}" --save-config --dry-run=client \
        --from-literal=SECRET_VCSSTAR__GITHUB_APP_ID="${GITHUB_APP_ID}" \
        --from-literal=SECRET_VCSSTAR__GITHUB_APP_CLIENT_ID="${GITHUB_APP_CLIENT_ID}" \
        --from-literal=SECRET_VCSSTAR__GITHUB_APP_CLIENT_SECRET="${GITHUB_APP_CLIENT_SECRET}" \
        --from-literal=SECRET_VCSSTAR__GITHUB_APP_PRIVATE_KEY="${GITHUB_APP_PRIVATE_KEY}" \
        -o yaml | kubectl apply -f -
fi

# Per-instance secrets that must survive redeploys: made once, never rotated
# by this job. The credential key seals stored mirror credentials; the SSH
# host key is what clients pin in known_hosts.
if ! kubectl -n "${K8S_NAMESPACE}" get secret vcsstar-credential-key >/dev/null 2>&1; then
    kubectl -n "${K8S_NAMESPACE}" create secret generic vcsstar-credential-key \
        --from-literal=SECRET_VCSSTAR__CREDENTIAL_KEY="$(head -c 32 /dev/urandom | base64 -w0)"
fi
if ! kubectl -n "${K8S_NAMESPACE}" get secret vcsstar-ssh-host-key >/dev/null 2>&1; then
    KEYDIR="$(mktemp -d)"
    ssh-keygen -q -t ed25519 -N "" -C "${HELM_RELEASE} host key" -f "${KEYDIR}/ssh_host_ed25519_key"
    kubectl -n "${K8S_NAMESPACE}" create secret generic vcsstar-ssh-host-key \
        --from-file=ssh_host_ed25519_key="${KEYDIR}/ssh_host_ed25519_key" \
        --from-file=ssh_host_ed25519_key.pub="${KEYDIR}/ssh_host_ed25519_key.pub"
    rm -rf "${KEYDIR}"
fi

# Dedicated corndogs (work capture) for this instance: file backend, required
# resilience policies, pinned chart release.
CORNDOGS_CHART_URL="https://github.com/catalystcommunity/corndogs/releases/download/helm_chart/v0.6.0/corndogs-0.6.0.tgz"
CORNDOGS_VALUES="$(mktemp)"
cat > "${CORNDOGS_VALUES}" <<VALS
fullnameOverride: corndogs
image:
  repository: ${REGISTRY}/public/catalystcommunity/corndogs
  tag: "0.8.0"
imagePullSecrets:
  - name: regcred
podSecurityContext:
  fsGroup: 65532
  fsGroupChangePolicy: OnRootMismatch
nodeSelector:
  kubernetes.io/arch: amd64
postgresql:
  enabled: false
zalando_postgres:
  enabled: false
storage:
  backend: file
  file:
    persistence:
      enabled: true
      size: 5Gi
resilience:
  submissionKeyPolicy: required
  taskGuardPolicy: required
VALS
helm upgrade --install corndogs "${CORNDOGS_CHART_URL}" --namespace "${K8S_NAMESPACE}" \
    -f "${CORNDOGS_VALUES}" --wait --timeout 5m
rm -f "${CORNDOGS_VALUES}"

# The instance overlay is not in the repository. It arrives as the content
# of the DEPLOY_VALUES secret, or as a path in VALUES_FILE for local runs.
if [ -n "${DEPLOY_VALUES:-}" ]; then
    VALUES_FILE="$(mktemp)"
    printf '%s\n' "${DEPLOY_VALUES}" > "${VALUES_FILE}"
fi
if [ -z "${VALUES_FILE:-}" ]; then
    echo "no instance values: set DEPLOY_VALUES (secret) or VALUES_FILE (path)" >&2
    exit 1
fi
RUNTIME_VALUES="$(mktemp)"
cat > "${RUNTIME_VALUES}" <<VALS
image:
  repository: ${IMAGE}
  tag: "${VERSION}"
VALS
helm upgrade --install --create-namespace --namespace "${K8S_NAMESPACE}" \
    "${HELM_RELEASE}" ./helm/vcsstar \
    -f "${VALUES_FILE}" -f "${RUNTIME_VALUES}" \
    --wait --timeout 10m
rm -f "${RUNTIME_VALUES}"
if [ -n "${DEPLOY_VALUES:-}" ]; then rm -f "${VALUES_FILE}"; fi
echo "deployed ${HELM_RELEASE} ${VERSION} to ${K8S_NAMESPACE}"
