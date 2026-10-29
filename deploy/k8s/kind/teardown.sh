#!/usr/bin/env bash
#
# Delete the kind cluster and the generated secrets. Everything in it goes,
# including the OpenTodo database.
#
#   ./teardown.sh            # ask first
#   ./teardown.sh --yes      # do not ask
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cluster=opentodo

if [ "${1:-}" != "--yes" ]; then
  read -r -p "Delete kind cluster '$cluster' and all its data? [y/N] " reply
  case "$reply" in [yY]*) ;; *) echo "nothing done"; exit 0 ;; esac
fi

kind delete cluster --name "$cluster"
rm -f "$here/kind-ca.crt" "$here/../overlays/kind/secrets.env"

cat <<EOF
Cluster deleted. If you added them, remove the /etc/hosts line for
todo.kind.test / keycloak.kind.test and the imported CA from your browser.
EOF
