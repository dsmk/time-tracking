#!/usr/bin/env bash
#
# Remove the tasks seed.sh created, and with them their time entries.
# Deleting the tasks is enough: OpenTodo tombstones a task's entries with it.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
env_file="$here/.live-env"
ot="${OT:-ot}"

[ -f "$env_file" ] || { echo "reset.sh: no $env_file; nothing recorded to clean up" >&2; exit 1; }
# shellcheck disable=SC1090
set -a; . "$env_file"; set +a

for id in "${TT_LIVE_TASK:-}" "${TT_LIVE_PLANNING_TASK:-}" "${TT_LIVE_REPORT_TASK:-}"; do
  [ -n "$id" ] || continue
  echo "removing task $id"
  "$ot" rm "$id" --yes >/dev/null 2>&1 || "$ot" rm "$id" >/dev/null 2>&1 || echo "  (could not remove $id; remove it in the web app)"
done

rm -f "$env_file"
echo "done; removed $env_file"
echo "To throw the whole server away instead: cd testing && docker compose down -v"
