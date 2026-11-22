# Use the BuildKit sidecar supplied by the builder capability.
ensure_builder() {
    local builder_bin=$1 buildkit_version=0.17.3 attempt
    if ! command -v buildctl >/dev/null 2>&1; then
        curl -fsSL "https://github.com/moby/buildkit/releases/download/v${buildkit_version}/buildkit-v${buildkit_version}.linux-amd64.tar.gz" -o "$builder_bin/buildkit.tar.gz"
        tar -xzf "$builder_bin/buildkit.tar.gz" -C "$builder_bin" --strip-components=1 bin/buildctl
        rm "$builder_bin/buildkit.tar.gz"
    fi
    for ((attempt=1; attempt<=30; attempt++)); do
        if buildctl debug info >/dev/null 2>&1; then break; fi
        if [ "$attempt" -eq 30 ]; then echo "builder sidecar not ready" >&2; exit 1; fi
        sleep 1
    done
}

registry_login() {
    mkdir -p "$DOCKER_CONFIG"
    local auth
    auth=$(printf '%s:%s' "$REGISTRY_USER" "$REGISTRY_PASSWORD" | base64 -w 0)
    (umask 077; printf '{"auths":{"%s":{"auth":"%s"}}}\n' "$REGISTRY" "$auth" > "$DOCKER_CONFIG/config.json")
}
