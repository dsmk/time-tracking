#!/usr/bin/env bash
#
# Capture what a real `ot` actually prints, so the UNVERIFIED items in
# docs/opentodo-compatibility.md §11 can be replaced with facts.
#
# Read-only by default. `--with-writes` additionally starts and stops a timer
# to capture the output of `ot time start` / `ot time stop`, which the docs do
# not describe.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$here/findings"
ot="${OT:-ot}"
with_writes=0
[ "${1:-}" = "--with-writes" ] && with_writes=1

command -v "$ot" >/dev/null || { echo "capture-findings.sh: '$ot' not on PATH" >&2; exit 1; }
mkdir -p "$out"

capture() {
  local name="$1"; shift
  echo "  $name  <-  ot $*"
  if "$ot" "$@" > "$out/$name" 2> "$out/$name.stderr"; then
    printf '0' > "$out/$name.exit"
  else
    printf '%s' "$?" > "$out/$name.exit"
  fi
}

echo "Capturing into $out"

# Needs no server and exits 0 offline: answers compat items 1-4 and 8 on its own.
capture schema.json schema
capture version.json version --json
capture auth-status.json auth status --json
capture tasks.json list --json

if [ -f "$here/.live-env" ]; then
  # shellcheck disable=SC1090
  set -a; . "$here/.live-env"; set +a
  capture time-list.json time list --from "$TT_LIVE_WEEK_FROM" --to "$TT_LIVE_WEEK_TO" --json
  capture time-list-running.json time list --running --json

  if [ "$with_writes" -eq 1 ] && [ -n "${TT_LIVE_TASK:-}" ]; then
    echo "  (writing: start then stop a timer on $TT_LIVE_TASK)"
    capture time-start.json time start "$TT_LIVE_TASK" --note "capture-findings" --json
    sleep 2
    capture time-stop.json time stop --json
  fi
fi

cat <<'NOTE'

Captured. These answer docs/opentodo-compatibility.md §11:

  schema.json            items 1-4 and 8: real flags, output schemas, limits
  version.json           item 3: which key holds the client version
  time-list.json         item 1: bare array vs {entries,...} envelope, serverTime
  time-start/stop.json   item 2: the undocumented start/stop output (--with-writes)
  tasks.json             item 8: task entity shape used by the tt picker

Check each file for credentials before pasting any of it into a document or
an issue — auth-status.json in particular names your account and token.
NOTE
