#!/usr/bin/env bash
#
# Entry point for the `test` image. Runs tt's test suites against a mounted
# copy of the repository, optionally talking to a real OpenTodo server.
#
#   unit       go test ./... — no server, no network, the suite CI runs
#   live       the opt-in tests behind the `live` build tag, against $OPENTODO_URL
#   seed       testing/seed.sh, to create the sample data live needs
#   findings   testing/capture-findings.sh, read-only
#   all        unit, then live when a server is configured
#   shell      a shell, for poking around
#
# Live runs need a reachable server and a token:
#
#   OPENTODO_URL    https://todo.example.com (or http://… for a local one)
#   OPENTODO_TOKEN  an API token for a throwaway account
#
# Both are what `ot` itself documents, so nothing here configures ot's own
# profile store.
#
# THESE WRITE DATA. Point them at a throwaway instance — testing/README.md
# explains why and how.
set -euo pipefail

cmd=${1:-all}
shift || true

cd /work

have_server() { [ -n "${OPENTODO_URL:-}" ] && [ -n "${OPENTODO_TOKEN:-}" ]; }

require_server() {
  if ! have_server; then
    cat >&2 <<'EOF'
OPENTODO_URL and OPENTODO_TOKEN are both required for this.

Get a token from the web interface (Settings -> Account -> API tokens), or
on an account that still has a password:

    ot login <url> && ot token create tt --scope read

../deploy/bin/ot-credentials.sh does the same thing for a host-side ot.
EOF
    exit 3
  fi
  echo "server: $OPENTODO_URL" >&2
  # Fail here, with a clear message, rather than inside a test.
  if ! ot auth status --json >/dev/null 2>&1; then
    echo "ot could not authenticate against $OPENTODO_URL" >&2
    ot auth status --json >&2 || true
    exit 3
  fi
}

case "$cmd" in
  unit)
    # Mirrors the CI job. The live tests sit behind the `live` build tag, so
    # this never reaches a server even if one is configured.
    exec go test ./... "$@"
    ;;

  live)
    require_server
    exec ./testing/run-live-tests.sh "$@"
    ;;

  seed)
    require_server
    exec ./testing/seed.sh "$@"
    ;;

  reset)
    require_server
    exec ./testing/reset.sh "$@"
    ;;

  findings)
    require_server
    exec ./testing/capture-findings.sh "$@"
    ;;

  all)
    echo "=== go test ./... ===" >&2
    go test ./...
    if have_server; then
      echo >&2
      echo "=== live tests ===" >&2
      require_server
      ./testing/run-live-tests.sh
    else
      echo >&2
      echo "No OPENTODO_URL/OPENTODO_TOKEN: skipping the live tests." >&2
      echo "That is the same thing CI does." >&2
    fi
    ;;

  shell)
    exec "${@:-bash}"
    ;;

  *)
    echo "unknown command: $cmd" >&2
    sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
    exit 2
    ;;
esac
