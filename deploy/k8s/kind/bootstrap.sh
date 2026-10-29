#!/usr/bin/env bash
#
# Stand up the whole stack — OpenTodo, Keycloak, PostgreSQL, an ingress with
# real TLS — in a throwaway kind cluster.
#
#   ./bootstrap.sh                 # create the cluster and deploy
#   ./bootstrap.sh --skip-cluster  # deploy into the existing kind cluster
#
# It is safe to run again: every step checks for what it would create.
#
# Afterwards you still have to do two things by hand, both printed at the end:
# add one line to /etc/hosts, and create the OpenTodo owner account in a
# browser (OpenTodo has no documented way to create it from the command line).
#
# Requires: kind, kubectl, openssl. Everything else is pulled from the
# cluster.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
overlay="$here/../overlays/kind"
cluster=opentodo
namespace=opentodo

INGRESS_NGINX_VERSION=controller-v1.15.1
CERT_MANAGER_VERSION=v1.21.3

TODO_HOST=todo.kind.test
KEYCLOAK_HOST=keycloak.kind.test
INGRESS_SVC=ingress-nginx-controller.ingress-nginx.svc.cluster.local

skip_cluster=0
for arg in "$@"; do
  case "$arg" in
    --skip-cluster) skip_cluster=1 ;;
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '\n==> %s\n' "$*"; }

# Retry a command that is expected to fail for a while. Admission webhooks go
# on refusing connections for a few seconds after the Deployment that serves
# them reports Available, so the first apply through one often loses.
#
#   retry <attempts> <seconds between> <what it is> -- cmd...
retry() {
  local attempts=$1 delay=$2 what=$3
  shift 4  # attempts, delay, what, --
  local attempt
  for attempt in $(seq 1 "$attempts"); do
    if "$@"; then
      return 0
    fi
    if [ "$attempt" -eq "$attempts" ]; then
      echo "$what did not succeed after $attempts attempts" >&2
      return 1
    fi
    echo "    $what not ready yet (attempt $attempt); retrying in ${delay}s"
    sleep "$delay"
  done
}

for tool in kind kubectl openssl; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done

# --- 1. the cluster -------------------------------------------------------
if [ "$skip_cluster" -eq 0 ]; then
  if kind get clusters 2>/dev/null | grep -qx "$cluster"; then
    say "kind cluster '$cluster' already exists"
  else
    say "creating kind cluster '$cluster'"
    kind create cluster --config "$here/cluster.yaml"
  fi
fi
kubectl cluster-info --context "kind-$cluster" >/dev/null
kubectl config use-context "kind-$cluster" >/dev/null

# --- 2. ingress-nginx -----------------------------------------------------
say "installing ingress-nginx $INGRESS_NGINX_VERSION"
kubectl apply -f \
  "https://raw.githubusercontent.com/kubernetes/ingress-nginx/${INGRESS_NGINX_VERSION}/deploy/static/provider/kind/deploy.yaml"
# `wait --selector` fails outright when nothing matches yet, and the
# controller's pod appears a moment after apply returns. The Deployment is
# created by the apply itself, so waiting on its rollout has no such race —
# and it reaches Available only once the pod is Ready, which is what the
# admission webhook needs.
kubectl -n ingress-nginx rollout status deployment/ingress-nginx-controller --timeout=300s

# --- 3. cert-manager ------------------------------------------------------
say "installing cert-manager $CERT_MANAGER_VERSION"
kubectl apply -f \
  "https://github.com/cert-manager/cert-manager/releases/download/${CERT_MANAGER_VERSION}/cert-manager.yaml"
kubectl -n cert-manager wait --for=condition=Available deployment --all --timeout=300s

say "creating the local certificate authority"
# cert-manager's validating webhook accepts traffic a little after its
# Deployment reports Available, and the first apply otherwise fails with
# "connection refused". Retry rather than make the operator do it.
retry 30 5 "cert-manager's webhook" -- kubectl apply -f "$here/ca-issuer.yaml"
# The CA key pair has to exist before the ClusterIssuer that signs with it
# can become ready.
kubectl -n cert-manager wait --for=condition=Ready certificate/kind-root-ca --timeout=120s
kubectl wait --for=condition=Ready clusterissuer/kind-ca --timeout=120s

# --- 4. in-cluster DNS ----------------------------------------------------
#
# The OIDC issuer URL has to mean the same thing to your browser and to the
# OpenTodo pod. Your browser gets there through /etc/hosts and kind's port
# mapping; a pod cannot, because 127.0.0.1 inside a pod is the pod. So teach
# CoreDNS to send these two names to the ingress controller, and the pod
# reaches Keycloak over exactly the URL in the discovery document.
say "pointing $KEYCLOAK_HOST and $TODO_HOST at the ingress controller inside the cluster"
corefile=$(kubectl -n kube-system get configmap coredns -o jsonpath='{.data.Corefile}')
if printf '%s' "$corefile" | grep -q 'kind\.test'; then
  echo "    CoreDNS already rewrites *.kind.test; leaving it alone"
else
  patched=$(printf '%s\n' "$corefile" | awk -v svc="$INGRESS_SVC" \
      -v a="$KEYCLOAK_HOST" -v b="$TODO_HOST" '
    { print }
    /^[[:space:]]*ready[[:space:]]*$/ && !done {
      printf "    rewrite name %s %s\n", a, svc
      printf "    rewrite name %s %s\n", b, svc
      done = 1
    }
    END {
      if (!done) {
        print "bootstrap: no `ready` line in the CoreDNS Corefile; " \
              "add the rewrite rules by hand" > "/dev/stderr"
        exit 1
      }
    }
  ')
  patch=$(mktemp)
  trap 'rm -f "$patch"' EXIT
  {
    echo "data:"
    echo "  Corefile: |"
    printf '%s\n' "$patched" | sed 's/^/    /'
  } > "$patch"
  kubectl -n kube-system patch configmap coredns --type merge --patch-file "$patch"
  kubectl -n kube-system rollout restart deployment coredns
  kubectl -n kube-system rollout status deployment coredns --timeout=120s
fi

# --- 5. secrets -----------------------------------------------------------
rand() { head -c 48 /dev/urandom | base64 | LC_ALL=C tr -dc 'A-Za-z0-9' | cut -c1-"${1:-32}"; }

if [ -f "$overlay/secrets.env" ]; then
  say "reusing $overlay/secrets.env"
else
  say "generating $overlay/secrets.env"
  cat > "$overlay/secrets.env" <<EOF
# Generated by kind/bootstrap.sh on $(date -Iseconds). Throwaway values for a
# throwaway cluster. Delete this file to have new ones generated.
OPENTODO_SETUP_TOKEN=$(rand 32)
OIDC_CLIENT_SECRET=$(rand 40)
KC_BOOTSTRAP_ADMIN_USERNAME=admin
KC_BOOTSTRAP_ADMIN_PASSWORD=$(rand 24)
POSTGRES_PASSWORD=$(rand 24)
DEV_USER_PASSWORD=$(rand 16)
# Fill these in and re-run to exercise the real GitHub bridge. Callback URL:
#   https://$KEYCLOAK_HOST/realms/opentodo/broker/github/endpoint
GITHUB_CLIENT_ID=${GITHUB_CLIENT_ID:-}
GITHUB_CLIENT_SECRET=${GITHUB_CLIENT_SECRET:-}
EOF
  chmod 600 "$overlay/secrets.env"
fi

# --- 6. the stack ---------------------------------------------------------
say "applying the kind overlay"
# Creating Ingresses goes through ingress-nginx's validating webhook.
retry 20 5 "ingress-nginx's admission webhook" -- kubectl apply -k "$overlay"

say "waiting for the certificates"
# `kubectl wait --all` errors out on an empty list, and cert-manager creates
# the Certificates from the Ingress annotations a moment after the apply.
for attempt in $(seq 1 60); do
  count=$(kubectl -n "$namespace" get certificate --no-headers 2>/dev/null | wc -l)
  [ "$count" -ge 2 ] && break
  if [ "$attempt" -eq 60 ]; then
    echo "cert-manager did not create Certificates from the Ingresses" >&2
    exit 1
  fi
  sleep 2
done
kubectl -n "$namespace" wait --for=condition=Ready certificate --all --timeout=180s

say "waiting for the workloads (Keycloak's first start builds, so this is slow)"
kubectl -n "$namespace" rollout status statefulset/keycloak-postgres --timeout=300s
kubectl -n "$namespace" rollout status deployment/keycloak --timeout=600s
kubectl -n "$namespace" rollout status deployment/opentodo --timeout=300s

# --- 7. what you do next --------------------------------------------------
kubectl -n cert-manager get secret kind-root-ca -o jsonpath='{.data.ca\.crt}' \
  | base64 -d > "$here/kind-ca.crt"

# kustomize appends a content hash to the generated Secret's name, so find
# it by prefix rather than guessing the suffix.
secret=$(kubectl -n "$namespace" get secret -o name | grep '^secret/stack-secrets-' | tail -1)
read_secret() {
  kubectl -n "$namespace" get "$secret" -o "jsonpath={.data.$1}" | base64 -d
}
setup_token=$(read_secret OPENTODO_SETUP_TOKEN)
admin_password=$(read_secret KC_BOOTSTRAP_ADMIN_PASSWORD)
dev_password=$(read_secret DEV_USER_PASSWORD)
dev_user=$(sed -n 's/^DEV_USER=//p' "$overlay/stack.env" | tail -1)

cat <<EOF

================================================================
The stack is up. Three things left, all on your machine.

1. Resolve the hostnames. Add this line to /etc/hosts:

     127.0.0.1  $TODO_HOST $KEYCLOAK_HOST

2. Trust the cluster CA, or accept the warning on both hosts.
   The root certificate is at:

     $here/kind-ca.crt

   Firefox:  Settings -> Privacy & Security -> Certificates ->
             View Certificates -> Authorities -> Import
   Chrome:   Settings -> Privacy and security -> Security ->
             Manage certificates -> Authorities -> Import

3. Create the OpenTodo owner account at

     https://$TODO_HOST

   The setup screen asks for a token, because the request arrives
   through the ingress rather than from loopback:

     $setup_token

Then, in OpenTodo as the owner:

   Settings -> People -> Single sign-on. Issuer, client id and
   secret are already filled in and locked (they come from the
   environment). Turn on "Allow new accounts via SSO" if you want
   brokered identities to get accounts without an invitation.

   Settings -> Account -> Security -> Linked accounts, to link the
   owner to the provider. Owners and admins are never linked by
   matching email; that is deliberate.

Signing in:

   Local Keycloak user      ${dev_user:-(none)} / $dev_password
   Keycloak admin console   https://$KEYCLOAK_HOST/admin/
                            admin / $admin_password

Tear it all down with ./teardown.sh
================================================================
EOF
