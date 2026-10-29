#!/usr/bin/env bash
# RFC 0014, Still open 1: does the k3s of the bench accept a
# requestheader-client-ca-file that holds its own aggregation CA *and* a
# dedicated front-proxy CA, without breaking aggregation, and does it reload
# that file without a restart?
#
# A throwaway k3d cluster, nothing from the bench. Hard checks fail the run;
# everything else is recorded as an observation in $SUMMARY (markdown), for
# the RFC's §17 to quote.
set -euo pipefail

K3S_IMAGE=${K3S_IMAGE:?k3s image (from .it/images.lock)}
CLUSTER=rfc0014-probe
WORK=$(mktemp -d)
SUMMARY=${SUMMARY:-$WORK/summary.md}
NODE=k3d-$CLUSTER-server-0
TLS=/var/lib/rancher/k3s/server/tls
failed=0

note() { echo "$*" | tee -a "$SUMMARY"; }
check() { # check <name> <expected> <actual>
    if [ "$2" = "$3" ]; then note "| ✅ | $1 | \`$3\` |"; else note "| ❌ | $1 | expected \`$2\`, got \`$3\` |"; failed=1; fi
}
observe() { note "| 👁 | $1 | \`$2\` |"; }
cleanup() { k3d cluster delete "$CLUSTER" >/dev/null 2>&1 || true; }
trap cleanup EXIT

# A CA and a client certificate signed by it.
new_ca() { openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
    -subj "/CN=$1" -keyout "$WORK/$1.key" -out "$WORK/$1.crt" 2>/dev/null; }
new_client() { # new_client <name> <cn> <ca>
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=$2" \
        -keyout "$WORK/$1.key" -out "$WORK/$1.csr" 2>/dev/null
    printf 'extendedKeyUsage=clientAuth\n' >"$WORK/$1.ext"
    openssl x509 -req -in "$WORK/$1.csr" -CA "$WORK/$3.crt" -CAkey "$WORK/$3.key" -CAcreateserial \
        -days 1 -extfile "$WORK/$1.ext" -out "$WORK/$1.crt" 2>/dev/null
}
# Replace the bind-mounted CA file atomically (write, then rename).
set_ca_file() { cat "$@" >"$WORK/mnt/.ca.tmp" && mv "$WORK/mnt/.ca.tmp" "$WORK/mnt/requestheader-ca.crt"; }

# review <cert> [header...]: "status username groups" of a SelfSubjectReview.
review() {
    local cert=$1; shift
    local args=(-s -o "$WORK/out.json" -w '%{http_code}' --cacert "$WORK/server-ca.crt" -X POST
        -H 'Content-Type: application/json'
        -d '{"apiVersion":"authentication.k8s.io/v1","kind":"SelfSubjectReview"}')
    [ "$cert" != none ] && args+=(--cert "$WORK/$cert.crt" --key "$WORK/$cert.key")
    for h in "$@"; do args+=(-H "$h"); done
    local code; code=$(curl "${args[@]}" "$SERVER/apis/authentication.k8s.io/v1/selfsubjectreviews" || true)
    echo "$code $(jq -r '(.status.userInfo.username // "-") + " " + ((.status.userInfo.groups // []) | map(select(startswith("proxyauth:"))) | join(",")) + " " + ((.status.userInfo.extra // {}) | to_entries | map(.key + "=" + (.value|join("|"))) | join(","))' "$WORK/out.json" 2>/dev/null || echo '- -')"
}
# wait_for <seconds> <command...>: retry until it succeeds.
wait_for() { local t=$1; shift; for _ in $(seq "$t"); do "$@" && return 0; sleep 1; done; return 1; }
aggregation_ok() { kubectl get --raw /apis/metrics.k8s.io/v1beta1 >/dev/null 2>&1; }
honoured() { # honoured <cert> <expected-user>
    [ "$(review "$1" 'X-Remote-User: proxyauth:alice' | cut -d' ' -f2)" = "$2" ]
}
refused() { [ "$(review "$1" 'X-Remote-User: proxyauth:alice' | cut -d' ' -f1)" = 401 ]; }
# The k3s image has no openssl: its CA files are copied out and used here.
from_node() { docker cp -q "$NODE:$TLS/$1" "$WORK/$2"; }

mkdir -p "$WORK/mnt"
new_ca dedicated-ca && new_client proxy proxyauth-probe dedicated-ca
new_client wrong-cn someone-else dedicated-ca
new_ca second-ca && new_client proxy2 proxyauth-probe second-ca
# Start with the dedicated CA only: k3s' own request-header CA does not exist
# before the first start, which is what the bench has to work around too.
set_ca_file "$WORK/dedicated-ca.crt"

note "### RFC 0014 front-proxy probe on \`$K3S_IMAGE\`"
note ""
note "| | Check | Result |"
note "| --- | --- | --- |"

k3d cluster create "$CLUSTER" --image "$K3S_IMAGE" --wait --timeout 180s --no-lb \
    --volume "$WORK/mnt:/var/lib/probe@server:0" \
    --k3s-arg "--kube-apiserver-arg=requestheader-client-ca-file=/var/lib/probe/requestheader-ca.crt@server:0" \
    --k3s-arg "--kube-apiserver-arg=requestheader-allowed-names=system:auth-proxy,proxyauth-probe@server:0" \
    >/dev/null
export KUBECONFIG=$WORK/kubeconfig
k3d kubeconfig get "$CLUSTER" >"$KUBECONFIG"
SERVER=$(kubectl config view --raw -o jsonpath='{.clusters[0].cluster.server}')
kubectl config view --raw -o jsonpath='{.clusters[0].cluster.certificate-authority-data}' | base64 -d >"$WORK/server-ca.crt"
started=$(docker inspect -f '{{.State.StartedAt}}' "$NODE")
from_node client-auth-proxy.crt k3s-auth-proxy.crt \
    && observe "front-proxy client CN k3s itself uses" "$(openssl x509 -noout -subject -in "$WORK/k3s-auth-proxy.crt")" \
    || observe "front-proxy client CN k3s itself uses" "could not be read from the node"

# 1. The dedicated CA alone: the proxy's identity is honoured, aggregation is not.
wait_for 60 honoured proxy proxyauth:alice || true
check "dedicated CA alone: X-Remote-User honoured" "proxyauth:alice" "$(review proxy 'X-Remote-User: proxyauth:alice' | cut -d' ' -f2)"
sleep 20
observe "dedicated CA alone: aggregated API (metrics.k8s.io)" "$(aggregation_ok && echo reachable || echo unreachable)"

# 2. Concatenate k3s' request-header CA: aggregation comes back, without a restart.
from_node request-header-ca.crt k3s-request-header-ca.crt
set_ca_file "$WORK/k3s-request-header-ca.crt" "$WORK/dedicated-ca.crt"
t0=$(date +%s)
wait_for 300 aggregation_ok && agg=reachable || agg=unreachable
check "concatenated file: aggregated API reachable" reachable "$agg"
observe "seconds until aggregation came back" "$(( $(date +%s) - t0 ))"
check "concatenated file: X-Remote-User still honoured" "proxyauth:alice" "$(review proxy 'X-Remote-User: proxyauth:alice' | cut -d' ' -f2)"
check "no restart of the apiserver" "$started" "$(docker inspect -f '{{.State.StartedAt}}' "$NODE")"

# 3. What the RFC claims the apiserver does (§4.2, §5.4, §6.P point 8).
observe "two X-Remote-Group headers, one with a comma" \
    "$(review proxy 'X-Remote-User: proxyauth:alice' 'X-Remote-Group: proxyauth:a,b' 'X-Remote-Group: proxyauth:c' | cut -d' ' -f3)"
observe "X-Remote-Extra with a percent-encoded key" \
    "$(review proxy 'X-Remote-User: proxyauth:alice' 'X-Remote-Extra-example.com%2fteam: blue' | cut -d' ' -f4)"
observe "headers without any client certificate (§6.P 8)" "$(review none 'X-Remote-User: proxyauth:alice')"
observe "dedicated CA, CN not in allowed-names (§5.4)" "$(review wrong-cn 'X-Remote-User: proxyauth:alice')"
observe "trailing space in X-Remote-User (§4.2)" "$(review proxy 'X-Remote-User: proxyauth:bob ' | cut -d' ' -f2)|"
# A certificate of the regular client CA: the CN should win, the headers ignored.
if from_node client-ca.crt client-ca.crt && from_node client-ca.key client-ca.key; then
    new_client client-ca-cert probe-client-ca client-ca
    observe "certificate of --client-ca-file plus headers (UpstreamIdentityNotHonoured)" \
        "$(review client-ca-cert 'X-Remote-User: proxyauth:alice' | cut -d' ' -f1,2)"
else
    observe "certificate of --client-ca-file plus headers" "client CA could not be read from the node"
fi

# 4. CA rotation (§13): a second CA is refused, then accepted once added, then
# the first is refused once removed — all without a restart.
check "second CA before it is added: refused" "401" "$(review proxy2 'X-Remote-User: proxyauth:alice' | cut -d' ' -f1)"
set_ca_file "$WORK/k3s-request-header-ca.crt" "$WORK/dedicated-ca.crt" "$WORK/second-ca.crt"
t0=$(date +%s)
wait_for 180 honoured proxy2 proxyauth:alice && r=honoured || r=refused
check "second CA added: honoured without restart" honoured "$r"
observe "seconds until the added CA was honoured" "$(( $(date +%s) - t0 ))"
set_ca_file "$WORK/k3s-request-header-ca.crt" "$WORK/second-ca.crt"
wait_for 180 refused proxy && r=refused || r=honoured
check "first CA removed: refused without restart" refused "$r"
check "still no restart of the apiserver" "$started" "$(docker inspect -f '{{.State.StartedAt}}' "$NODE")"
wait_for 300 aggregation_ok && agg=reachable || agg=unreachable
check "aggregation after the rotation" reachable "$agg"

note ""
[ "$failed" = 0 ] && note "**All hard checks passed.**" || note "**Some hard checks failed.**"
exit "$failed"
