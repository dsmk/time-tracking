#!/usr/bin/env bash
#
# Run the opt-in live tests against the seeded test server.
#
# They sit behind the `live` build tag, so `go test ./...` does not even
# compile them; this script is the deliberate way in. They run one at a time
# because OpenTodo allows only one running timer per account, which is shared
# state.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname "$here")"
env_file="$here/.live-env"

if [ ! -f "$env_file" ]; then
  echo "run-live-tests.sh: $env_file not found." >&2
  echo "Start the server and seed it first — see testing/README.md:" >&2
  echo "  cd testing && docker compose up -d && ./seed.sh" >&2
  exit 1
fi

# shellcheck disable=SC1090
set -a; . "$env_file"; set +a

echo "Live tests against ${OPENTODO_URL:-the configured ot instance}"
echo "  week:     $TT_LIVE_WEEK_FROM .. $TT_LIVE_WEEK_TO"
echo "  timezone: $TT_LIVE_TZ"
echo

cd "$root"
exec go test -tags live -count=1 -v -timeout 10m \
  ./internal/integration -run 'TestLive' "$@"
