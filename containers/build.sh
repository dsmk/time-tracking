#!/usr/bin/env bash
#
# Build the images in Containerfile. Run from anywhere; the build context is
# always the repository root, because the build needs tt's source.
#
#   ./build.sh                 # both
#   ./build.sh toolbox
#   ./build.sh test
#   ./build.sh toolbox --build-arg TOOLBOX_EXTRA_PACKAGES=""   # no Go toolchain
#
# Environment:
#   ENGINE        podman (default) or docker
#   IMAGE_PREFIX  image name prefix, default "tt"
#   OT_VERSION    the ot release to build in, default from the Containerfile
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
engine="${ENGINE:-podman}"
prefix="${IMAGE_PREFIX:-tt}"

command -v "$engine" >/dev/null || { echo "$engine is not installed" >&2; exit 1; }

targets=()
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    toolbox|test) targets+=("$1"); shift ;;
    -h|--help) sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) args+=("$1"); shift ;;
  esac
done
[ ${#targets[@]} -gt 0 ] || targets=(toolbox test)

[ -n "${OT_VERSION:-}" ] && args+=(--build-arg "OT_VERSION=$OT_VERSION")

for target in "${targets[@]}"; do
  echo "==> building $prefix-$target"
  "$engine" build \
    -f "$here/Containerfile" \
    --target "$target" \
    -t "$prefix-$target:latest" \
    ${args[@]+"${args[@]}"} \
    "$root"
done

echo
echo "Built: ${targets[*]/#/$prefix-}"
echo
echo "  toolbox create --image localhost/$prefix-toolbox:latest tt"
echo "  ./run-tests.sh unit"
