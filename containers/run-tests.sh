#!/usr/bin/env bash
#
# Run tt's tests inside the `test` image, against the working tree.
#
#   ./run-tests.sh                                   # cargo test only
#   ./run-tests.sh unit
#   ./run-tests.sh seed                              # needs a server
#   ./run-tests.sh live                              # needs a server
#   ./run-tests.sh shell
#
# A live run needs a server and a token in the environment:
#
#   export OPENTODO_URL=https://todo.example.com
#   export OPENTODO_TOKEN=...          # see deploy/bin/ot-credentials.sh
#   ./run-tests.sh live
#
# THESE WRITE DATA. Use a throwaway instance.
#
# Networking: --network=host, so a server on the host's own localhost (such as
# testing/docker-compose.yml on :8080) is reachable at the same URL you would
# use outside. For a server elsewhere, host networking is harmless.
#
# Environment:
#   ENGINE        podman (default) or docker
#   IMAGE         image to run, default tt-test:latest
#   CARGO_VOLUME  named volume for the Rust build cache, default tt-cargo
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
engine="${ENGINE:-podman}"
image="${IMAGE:-tt-test:latest}"
cargo_volume="${CARGO_VOLUME:-tt-cargo}"

command -v "$engine" >/dev/null || { echo "$engine is not installed" >&2; exit 1; }

if ! "$engine" image exists "$image" 2>/dev/null && \
   ! "$engine" image inspect "$image" >/dev/null 2>&1; then
  echo "$image is not built yet. Run ./build.sh test" >&2
  exit 1
fi

opts=(
  --rm
  --network=host
  # The working tree, writable: seed.sh writes testing/.live-env and
  # capture-findings.sh writes testing/findings/.
  --volume "$root:/work:z"
  # Build artefacts and the cargo registry stay in volumes, so a container
  # run never touches the host's ./target and a second run is fast.
  --volume "$cargo_volume-target:/cargo-target"
  --volume "$cargo_volume-home:/cargo-home"
)

# Rootless podman: map the container's user to yours so nothing in the
# working tree ends up owned by a subuid.
if [ "$engine" = podman ]; then
  opts+=(--userns=keep-id)
fi

[ -t 0 ] && opts+=(--interactive --tty)

for var in OPENTODO_URL OPENTODO_TOKEN OPENTODO_PROFILE TT_LIVE_READ_TOKEN TZ; do
  [ -n "${!var:-}" ] && opts+=(--env "$var")
done

exec "$engine" run "${opts[@]}" "$image" "$@"
