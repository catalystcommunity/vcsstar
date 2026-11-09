#!/usr/bin/env bash
# Build and push the vscstar images, then helm upgrade the instance.
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
kubectl create secret generic vscstar-linkkeys \
    --namespace "${K8S_NAMESPACE}" --save-config --dry-run=client \
    --from-literal=SECRET_VSCSTAR__LINKKEYS_API_KEY="${LINKKEYS_API_KEY}" \
    -o yaml | kubectl apply -f -

RUNTIME_VALUES="$(mktemp)"
cat > "${RUNTIME_VALUES}" <<VALS
image:
  repository: ${IMAGE}
  tag: "${VERSION}"
VALS
helm upgrade --install --create-namespace --namespace "${K8S_NAMESPACE}" \
    "${HELM_RELEASE}" ./helm/vscstar \
    -f "${VALUES_FILE}" -f "${RUNTIME_VALUES}" \
    --wait --timeout 10m
rm -f "${RUNTIME_VALUES}"
echo "deployed ${HELM_RELEASE} ${VERSION} to ${K8S_NAMESPACE}"
