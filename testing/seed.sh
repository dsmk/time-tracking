#!/usr/bin/env bash
#
# Seed a test OpenTodo instance with sample tasks and time entries, then write
# testing/.live-env describing exactly what was created so the opt-in live
# tests can assert against known numbers.
#
# Every OpenTodo operation here uses a documented `ot` command:
#   ot auth status / ot project add / ot add / ot time log / ot time list
#
# Requires: ot (0.9+), jq, GNU date.
#
#   ./seed.sh                 # seed the last complete Mon-Sun week
#   ./seed.sh --yes           # skip the confirmation prompt
#   ./seed.sh --force         # seed even if that week already has entries
#   ./seed.sh --timezone America/New_York

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
env_file="$here/.live-env"
ot="${OT:-ot}"
assume_yes=0
force=0
timezone=""

while [ $# -gt 0 ]; do
  case "$1" in
    --yes|-y) assume_yes=1 ;;
    --force) force=1 ;;
    --timezone) timezone="${2:?--timezone needs a value}"; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "seed.sh: unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

die() { echo "seed.sh: $*" >&2; exit 1; }

command -v "$ot" >/dev/null || die "the OpenTodo client '$ot' is not on PATH (set OT=/path/to/ot)"
command -v jq >/dev/null || die "jq is required"
date -d "2026-01-01 +1 day" >/dev/null 2>&1 || die "GNU date is required (on macOS: brew install coreutils, then PATH=\$(brew --prefix)/opt/coreutils/libexec/gnubin:\$PATH)"

# Detect the IANA zone that `ot` will use for --from/--to times, so the live
# tests can configure tt with the same one.
if [ -z "$timezone" ]; then
  if [ -n "${TZ:-}" ]; then
    timezone="$TZ"
  elif [ -L /etc/localtime ]; then
    timezone="$(readlink -f /etc/localtime | sed 's|.*/zoneinfo/||')"
  fi
fi
[ -n "$timezone" ] || die "could not detect a timezone; pass --timezone <IANA name>"

# --- confirm the target ------------------------------------------------------

auth="$("$ot" auth status --json)" || die "\`ot auth status\` failed; sign in first (see testing/README.md)"
server="$(printf '%s' "$auth" | jq -r '.server // .url // "unknown server"')"
account="$(printf '%s' "$auth" | jq -r '.account // .email // .user // "unknown account"')"
scope="$(printf '%s' "$auth" | jq -r '.scope // "unknown"')"

echo "About to create sample tasks and time entries:"
echo "  server:   $server"
echo "  account:  $account"
echo "  scope:    $scope"
echo "  timezone: $timezone"
echo
echo "This WRITES DATA. Use a throwaway instance or profile, never a real account."
if [ "$assume_yes" -ne 1 ]; then
  read -r -p "Continue? [y/N] " reply
  case "$reply" in y|Y|yes|YES) ;; *) echo "aborted"; exit 1 ;; esac
fi

# --- pick the reference week -------------------------------------------------

today="$(TZ="$timezone" date +%F)"
dow="$(TZ="$timezone" date +%u)"                                  # 1=Mon .. 7=Sun
this_monday="$(date -d "$today -$((dow - 1)) days" +%F)"
week_from="$(date -d "$this_monday -7 days" +%F)"                 # last complete week
week_to="$(date -d "$week_from +6 days" +%F)"
mon="$week_from"
tue="$(date -d "$week_from +1 day" +%F)"
wed="$(date -d "$week_from +2 days" +%F)"
thu="$(date -d "$week_from +3 days" +%F)"

echo
echo "Reference week: $week_from to $week_to"

existing="$("$ot" time list --from "$week_from" --to "$week_to" --json | jq '(.entries // .) | length')"
if [ "$existing" -ne 0 ] && [ "$force" -ne 1 ]; then
  die "that week already has $existing time entries; the live tests assert exact totals.
    Use a clean account, pick another week, or re-run with --force."
fi

# --- tasks -------------------------------------------------------------------

project="TT Test"
"$ot" project add "$project" >/dev/null 2>&1 || echo "note: project '$project' may already exist, continuing"

add_task() {
  local title="$1" out id
  out="$("$ot" add "$title" --project "$project" --json)" || die "\`ot add\` failed for '$title'"
  id="$(printf '%s' "$out" | jq -r '.id // .task.id // empty')"
  [ -n "$id" ] || die "could not read a task id from \`ot add --json\`: $out"
  printf '%s' "$id"
}

echo "Creating tasks..."
design_id="$(add_task "Design review")"
planning_id="$(add_task "Planning")"
report_id="$(add_task "Write report")"
echo "  Design review  $design_id"
echo "  Planning       $planning_id"
echo "  Write report   $report_id"

# --- time entries ------------------------------------------------------------
#
# Durations are chosen so every assertion is exact:
#   Mon 09:00-10:00  Design review  3600s
#   Tue 14:00-15:30  Planning       5400s
#   Wed 23:00-23:45  Design review  2700s   (near local midnight, on purpose)
#   Thu 30m          Write report   1800s   (documented duration form)
#                                  ------
#                                  13500s = 3h 45m
#
# No entry crosses midnight: OpenTodo rejects an end time before the start, and
# `ot time log --from/--to` are times on a single day.

echo "Logging time entries..."
"$ot" time log "$design_id"   --from 09:00 --to 10:00 --on "$mon" --note "seeded: morning review" >/dev/null
"$ot" time log "$planning_id" --from 14:00 --to 15:30 --on "$tue" --note "seeded: planning" >/dev/null
"$ot" time log "$design_id"   --from 23:00 --to 23:45 --on "$wed" --note "seeded: late session" >/dev/null
"$ot" time log "$report_id"   30m --on "$thu" --note "seeded: duration form" >/dev/null

logged="$("$ot" time list --from "$week_from" --to "$week_to" --json | jq '(.entries // .) | length')"
echo "  $logged entries now in $week_from..$week_to"

# --- record what the live tests should expect --------------------------------

cat > "$env_file" <<ENV
# Written by testing/seed.sh on $(date -Is). Source this before running the
# opt-in live tests:  set -a; . testing/.live-env; set +a
TT_LIVE=1
TT_LIVE_TZ=$timezone

TT_LIVE_WEEK_FROM=$week_from
TT_LIVE_WEEK_TO=$week_to
TT_LIVE_TOTAL_SECONDS=13500

TT_LIVE_MON=$mon
TT_LIVE_MON_SECONDS=3600
TT_LIVE_TUE=$tue
TT_LIVE_TUE_SECONDS=5400
TT_LIVE_WED=$wed
TT_LIVE_WED_SECONDS=2700
TT_LIVE_THU=$thu
TT_LIVE_THU_SECONDS=1800

# "Design review" holds the Monday and Wednesday entries.
TT_LIVE_TASK=$design_id
TT_LIVE_TASK_TITLE=Design review
TT_LIVE_TASK_SECONDS=6300
TT_LIVE_PLANNING_TASK=$planning_id
TT_LIVE_REPORT_TASK=$report_id
ENV

echo
echo "Wrote $env_file"
echo
echo "Run the live tests with:"
echo "  ./testing/run-live-tests.sh"
