#!/usr/bin/env bash
# Mount the integration bench (RFC 0009 §6.2): a run CA, the k3d cluster,
# the images built from the tree, Redis, Dex and the chart. Run through
# `task it:up`, from the repository root.
set -euo pipefail

RUN=${IT_DIR:-.it/run}
CLUSTER=${IT_CLUSTER:-proxyauth-it}
PORT=${IT_HTTPS_PORT:-8443}
NS=proxyauth
ADMIN="$RUN/admin.kubeconfig"
STEPS="$RUN/steps.log"

step() { printf '%s %s\n' "$(date +%s)" "$1" >>"$STEPS"; echo "==> $1"; }

# One image reference from .it/images.lock, refused without a digest: a
# floating tag makes a failure impossible to reproduce.
image() {
  local ref
  ref=$(awk -v k="$1" '$1 == k { print $2 }' .it/images.lock)
  if [[ "$ref" != *@sha256:* ]]; then
    echo "it:up: image '$1' has no digest in .it/images.lock: '$ref'" >&2
    exit 1
  fi
  echo "$ref"
}
K3S=$(image k3s)
DEX=$(image dex)
REDIS=$(image redis)

# The engine: CONTAINER_HOST as given, else rootless podman's socket, else
# Docker (CI).
if [[ -z "${CONTAINER_HOST:-}" && -S "${XDG_RUNTIME_DIR:-/nonexistent}/podman/podman.sock" ]]; then
  export DOCKER_HOST="unix://$XDG_RUNTIME_DIR/podman/podman.sock"
  export DOCKER_SOCK="$XDG_RUNTIME_DIR/podman/podman.sock"
  ENGINE=podman
elif [[ -n "${CONTAINER_HOST:-}" ]]; then
  export DOCKER_HOST="$CONTAINER_HOST"
  ENGINE=${IT_ENGINE:-docker}
else
  ENGINE=docker
fi

# Rootless podman: k3s needs the cpuset controller delegated to the user
# session, or it exits at start and `k3d cluster create --wait` hangs.
if [[ "$ENGINE" == podman ]]; then
  delegated="/sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service/cgroup.controllers"
  if [[ -r "$delegated" ]] && ! grep -qw cpuset "$delegated"; then
    cat >&2 <<'MSG'
it:up: the cpuset cgroup controller is not delegated to your user, k3s cannot
start under rootless podman. Once, as root (then log out and back in):
  mkdir -p /etc/systemd/system/user@.service.d
  printf '[Service]\nDelegate=cpu cpuset io memory pids\n' > /etc/systemd/system/user@.service.d/delegate.conf
  systemctl daemon-reload
MSG
    exit 1
  fi
fi

# The kubelet's `logs -f` needs inotify instances; rootless podman shares the
# host's, and 128 (a common default) runs out with the suite's pods.
instances=$(cat /proc/sys/fs/inotify/max_user_instances 2>/dev/null || echo 0)
if [[ "$instances" -lt 512 ]]; then
  echo "it:up: warning: fs.inotify.max_user_instances is $instances; \`kubectl logs -f\` in the suite needs more:" >&2
  echo "  sysctl -w fs.inotify.max_user_instances=512   (as root)" >&2
fi

# Every persona's tokens go to one session keyring (`it:test` runs the
# suite under `keyctl session -`); the default 20000-byte quota fills up
# after about twenty logins and the CLI then fails to store its token.
keybytes=$(cat /proc/sys/kernel/keys/maxbytes 2>/dev/null || echo 0)
if [[ "$keybytes" -lt 1000000 ]]; then
  echo "it:up: warning: kernel.keys.maxbytes is $keybytes; the suite's logins need more:" >&2
  echo "  sysctl -w kernel.keys.maxbytes=2000000 kernel.keys.maxkeys=2000   (as root)" >&2
fi

if ss -Hltn "sport = :$PORT" | grep -q .; then
  echo "it:up: port $PORT is already in use (IT_HTTPS_PORT)" >&2
  exit 1
fi

mkdir -p "$RUN/ca" "$RUN/logs"
: >"$STEPS"

step "ca"
# CAROOT inside the run directory and never `mkcert -install`: the test CA
# must not enter the workstation's trust store (RFC 0009 T3).
CAROOT="$RUN/ca" mkcert -cert-file "$RUN/tls.crt" -key-file "$RUN/tls.key" \
  '*.k8s.localhost' proxyauth.k8s.localhost dex.k8s.localhost 2>/dev/null

step "cluster"
awk -v image="$K3S" -v ca="$RUN/ca/rootCA.pem" -v traefik="$PWD/.it/traefik-config.yaml" '
  /^ *__CA__$/ { while ((getline line < ca) > 0) print "              " line; next }
  { gsub(/__IMAGE__/, image); gsub(/__TRAEFIK_CONFIG__/, traefik); print }' .it/k3d.yaml.tpl >"$RUN/k3d.yaml"
sed -i "s/127.0.0.1:8443:8443/127.0.0.1:$PORT:8443/" "$RUN/k3d.yaml"
k3d cluster create "$CLUSTER" --config "$RUN/k3d.yaml" --wait
k3d kubeconfig get "$CLUSTER" >"$ADMIN"
chmod 600 "$ADMIN"
k() { kubectl --kubeconfig "$ADMIN" "$@"; }

step "images"
if [[ -z "${IT_SKIP_BUILD:-}" ]]; then
  "$ENGINE" build -f deploy/build/back/Containerfile -t localhost/proxyauthk8s/server:it .
  "$ENGINE" build -f deploy/build/front/Containerfile -t localhost/proxyauthk8s/front:it .
  "$ENGINE" build -f deploy/build/agent/Containerfile -t localhost/proxyauthk8s/agent:it .
fi
# Through a tarball: under podman, k3d's own `docker save` in its tools
# container fails.
images=(localhost/proxyauthk8s/server:it localhost/proxyauthk8s/front:it localhost/proxyauthk8s/agent:it)
if [[ "$ENGINE" == podman ]]; then
  podman save -m -o "$RUN/images.tar" "${images[@]}"
else
  "$ENGINE" save -o "$RUN/images.tar" "${images[@]}"
fi
k3d image import -c "$CLUSTER" "$RUN/images.tar"
rm -f "$RUN/images.tar"

step "stack"
k apply -f deploy/crds.yaml
k create namespace "$NS"
k -n "$NS" create secret tls proxyauthk8s-tls --cert "$RUN/tls.crt" --key "$RUN/tls.key"
k -n "$NS" create secret generic it-ca --from-file=rootCA.pem="$RUN/ca/rootCA.pem"
k -n "$NS" create secret generic proxyauthk8s-back-oidc \
  --from-literal=OIDC_ISSUER_URL=https://dex.k8s.localhost:8443 \
  --from-literal=OIDC_CLIENT_ID=proxyauthk8s \
  --from-literal=OIDC_AUDIENCE=proxyauthk8s \
  --from-literal="OIDC_SCOPES=openid profile email groups"
yq '.staticPasswords = (load(".it/personas.yaml").personas | to_entries | map({
      "email": .value.email, "username": .key, "hash": .value.hash,
      "userID": .value.userID, "groups": .value.groups}))' \
  .it/dex/config.yaml >"$RUN/dex.yaml"
k -n "$NS" create secret generic dex-config --from-file=config.yaml="$RUN/dex.yaml"
sed -e "s|__REDIS__|$REDIS|" -e "s|__DEX__|$DEX|" .it/stack.yaml | k apply -f -
helm --kubeconfig "$ADMIN" upgrade --install proxyauth deploy/chart -n "$NS" -f .it/values.yaml

step "rollout"
for d in redis dex proxyauth-proxyauthk8s-back proxyauth-proxyauthk8s-front proxyauth-proxyauthk8s-gateway; do
  k -n "$NS" rollout status "deployment/$d" --timeout=180s
done
k -n kube-system rollout status deployment/traefik --timeout=180s

step "ready"
# Traefik programs the routes a moment after the rollouts: retry briefly.
curl -fsS -o /dev/null --retry 15 --retry-all-errors --retry-delay 2 --cacert "$RUN/ca/rootCA.pem" \
  --resolve "proxyauth.k8s.localhost:$PORT:127.0.0.1" "https://proxyauth.k8s.localhost:$PORT/"
echo "bench up: https://proxyauth.k8s.localhost:$PORT (admin kubeconfig: $ADMIN)"
