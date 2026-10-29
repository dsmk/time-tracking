#!/usr/bin/env bash
#
# Point the `ot` client at an OpenTodo server with an API token, on a named
# profile, so `tt` can use it.
#
# `tt` stores no credentials and no server URL of its own — it runs `ot` and
# uses whatever `ot` is configured with. So this is the one setup step a
# `tt` user needs after a server is deployed.
#
# Why a token rather than `ot login`: an account created through single
# sign-on has no password, so there is nothing for `ot login` to ask for.
# Tokens work regardless, and keep working when the instance is SSO-only.
#
#   ./ot-credentials.sh --url https://todo.example.com
#   ./ot-credentials.sh --url https://todo.example.com --profile work
#   ./ot-credentials.sh --url https://todo.example.com --token-file ./token
#   OPENTODO_TOKEN=... ./ot-credentials.sh --url https://todo.example.com
#
# The token is read from --token-file, then $OPENTODO_TOKEN, then a hidden
# prompt. It is never passed as a command-line argument, where `ps` would
# show it to every user on the machine.
#
# Where to get one:
#
#   Web interface   Sign in (through GitHub, if that is how this instance is
#                   set up), then Settings -> Account -> API tokens.
#   ot              On an account that still has a password — typically the
#                   owner on a fresh instance:
#                       ot login <url>
#                       ot token create tt --scope read
#                   `--scope read` is enough for `tt status`, `tt log` and
#                   `tt report`. `tt start` and `tt stop` are writes and need
#                   a token without that restriction.
set -euo pipefail

url=""
profile=""
token_file=""
ot="${OT:-ot}"

usage() { sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    --url) url="${2:?--url needs a value}"; shift 2 ;;
    --profile) profile="${2:?--profile needs a value}"; shift 2 ;;
    --token-file) token_file="${2:?--token-file needs a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[ -n "$url" ] || { echo "--url is required" >&2; exit 2; }
url=${url%/}

case "$url" in
  https://*) ;;
  http://localhost*|http://127.0.0.1*)
    echo "note: plain http, so OpenTodo will not set Secure cookies. Fine for" >&2
    echo "      a local test server, not for anything else." >&2
    ;;
  *)
    echo "refusing a plain-http URL that is not loopback: $url" >&2
    echo "A token sent over http is readable by anything on the path." >&2
    exit 2
    ;;
esac

# --- the token ------------------------------------------------------------
token=""
if [ -n "$token_file" ]; then
  [ -r "$token_file" ] || { echo "cannot read $token_file" >&2; exit 2; }
  token=$(tr -d '\r\n' < "$token_file")
elif [ -n "${OPENTODO_TOKEN:-}" ]; then
  token=$OPENTODO_TOKEN
else
  if [ -t 0 ]; then
    printf 'API token for %s (input hidden): ' "$url" >&2
    read -rs token
    printf '\n' >&2
  else
    echo "no token: pass --token-file, set OPENTODO_TOKEN, or run this on a terminal" >&2
    exit 2
  fi
fi
[ -n "$token" ] || { echo "the token is empty" >&2; exit 2; }

# --- configure ot ---------------------------------------------------------
profile_args=()
[ -n "$profile" ] && profile_args=(--profile "$profile")

if ! command -v "$ot" >/dev/null; then
  # Quoted delimiter: this block has no variables, and backticks in an
  # unquoted heredoc would run the command rather than print its name.
  cat >&2 <<'EOF'
`ot` is not on PATH. Install it:

    go install github.com/OpenTodoApp/opentodo/cmd/ot@latest

(The module moved from github.com/pixari/opentodo; the old path no longer
resolves. See containers/README.md for a prebuilt alternative.)

Or set OT=/path/to/ot and run this again.
EOF
  exit 7
fi

configured=0
echo "configuring $ot${profile:+ (profile $profile)} for $url" >&2
# The token goes in on stdin rather than argv. `ot auth set-token` is the
# documented command for this; its exact argument form is not spelled out in
# the documentation, so a failure here falls back to the environment
# variables, which are documented precisely.
if printf '%s' "$token" | "$ot" auth set-token "${profile_args[@]}" "$url" >/dev/null 2>&1; then
  configured=1
fi

if [ "$configured" -eq 1 ] && "$ot" auth status "${profile_args[@]}" --json >/dev/null 2>&1; then
  echo "ok: ot is configured and the server accepted the token." >&2
  if [ -n "$profile" ]; then
    cat <<EOF

Use it by exporting the profile:

    export OPENTODO_PROFILE=$profile
    tt status

tt reads nothing of its own — it runs ot, and ot follows OPENTODO_PROFILE.
EOF
  else
    printf '\n    tt status\n\n'
  fi
  exit 0
fi

# --- fallback: an environment file ----------------------------------------
#
# OPENTODO_URL and OPENTODO_TOKEN are documented and need no help from `ot`'s
# own configuration file.
env_dir="${XDG_CONFIG_HOME:-$HOME/.config}/opentodo"
env_file="$env_dir/${profile:-default}.env"
mkdir -p "$env_dir"
( umask 077; cat > "$env_file" <<EOF
# Written by deploy/bin/ot-credentials.sh. Holds an API token: keep it 0600
# and out of version control.
export OPENTODO_URL=$url
export OPENTODO_TOKEN=$token
EOF
)
chmod 600 "$env_file"

cat >&2 <<EOF

Could not configure ot's own profile store, so the token was written to an
environment file instead. Both are documented ways to point ot at a server;
this one needs you to source it.

    source $env_file
    tt status

If you would rather use a profile, check what your ot expects:

    $ot auth set-token --help
EOF
